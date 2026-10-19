//! dmenu mode — stdin in, stdout out.

use super::{Entry, Mode};
use anyhow::Result;
use std::io::{self, BufRead, IsTerminal, Read};
use std::sync::{Arc, mpsc};

#[derive(Default)]
pub struct Dmenu;

impl Mode for Dmenu {
    fn collect(&mut self) -> Result<Vec<Entry>> {
        require_piped_stdin()?;
        Ok(read_entries(io::stdin().lock())?)
    }
}

fn require_piped_stdin() -> Result<()> {
    if io::stdin().is_terminal() {
        anyhow::bail!("dmenu mode requires entries on stdin");
    }
    Ok(())
}

/// The most stdin dmenu reads. A producer that writes more is an error, not
/// a picker that holds gigabytes in memory.
pub const MAX_STDIN_BYTES: u64 = 256 * 1024 * 1024;

/// One entry per non-empty line, from at most [`MAX_STDIN_BYTES`].
pub(crate) fn read_entries(reader: impl BufRead) -> io::Result<Vec<Entry>> {
    read_entries_capped(reader, MAX_STDIN_BYTES)
}

/// [`read_entries`] with the cap given: more than `max_bytes` of input is an
/// error. Reading stops one byte past the cap, so a single endless line is
/// bounded too.
fn read_entries_capped(reader: impl BufRead, max_bytes: u64) -> io::Result<Vec<Entry>> {
    let mut limited = reader.take(max_bytes + 1);
    let mut entries = Vec::new();
    for line in limited.by_ref().lines() {
        let line = line?;
        if !line.is_empty() {
            entries.push(Entry::stdout(line));
        }
    }
    if limited.limit() == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("more than {} MiB on stdin", max_bytes / (1024 * 1024)),
        ));
    }
    Ok(entries)
}

/// What the `--loading` reader sends when stdin closes: the entries, or the
/// read error as text (the UI's channel signal needs a `Clone` value, which
/// `io::Error` isn't).
pub type LoadedEntries = std::result::Result<Arc<Vec<Entry>>, String>;

/// `--loading`: read stdin on a background thread so the window can open
/// before the producer finishes. The result arrives once, when stdin closes.
pub fn read_stdin_in_background() -> Result<mpsc::Receiver<LoadedEntries>> {
    require_piped_stdin()?;
    Ok(read_in_background(|| io::stdin().lock()))
}

/// Read the lines of `open()` on a new thread and send the outcome, so a
/// read error reaches the UI instead of passing for an empty list.
fn read_in_background<R: BufRead>(
    open: impl FnOnce() -> R + Send + 'static,
) -> mpsc::Receiver<LoadedEntries> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let loaded = read_entries(open())
            .map(Arc::new)
            .map_err(|e| e.to_string());
        // A closed receiver means the window is already gone.
        let _ = tx.send(loaded);
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(input: &str) -> Vec<String> {
        read_entries(io::Cursor::new(input))
            .unwrap()
            .into_iter()
            .map(|e| e.label)
            .collect()
    }

    #[test]
    fn one_entry_per_line() {
        assert_eq!(labels("a\nb\nc\n"), ["a", "b", "c"]);
    }

    #[test]
    fn skips_empty_lines_and_handles_missing_final_newline() {
        assert_eq!(labels("a\n\n\nb"), ["a", "b"]);
    }

    #[test]
    fn empty_input_is_no_entries() {
        assert!(labels("").is_empty());
    }

    #[test]
    fn input_over_the_cap_is_an_error() {
        // At the cap exactly is fine; one byte past it is not, even when it
        // is a single line with no newline.
        assert_eq!(
            read_entries_capped(
                io::Cursor::new(
                    "ab
cd
"
                ),
                6
            )
            .unwrap()
            .len(),
            2
        );
        for over in [
            "ab
cd
e", "abcdefg",
        ] {
            let err = read_entries_capped(io::Cursor::new(over), 6).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidData, "{over:?}");
        }
    }

    #[test]
    fn invalid_utf8_is_an_error() {
        assert!(read_entries(io::Cursor::new(b"ok\n\xff\xfe\n".to_vec())).is_err());
    }

    #[test]
    fn background_read_sends_entries() {
        let rx = read_in_background(|| io::Cursor::new("a\n\nb\n"));
        let entries = rx.recv().unwrap().unwrap();
        let labels: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, ["a", "b"]);
    }

    #[test]
    fn background_read_error_is_sent_not_an_empty_list() {
        let rx = read_in_background(|| io::Cursor::new(b"ok\n\xff\xfe\n".to_vec()));
        let err = rx.recv().unwrap().unwrap_err();
        assert!(err.contains("UTF-8"), "{err}");
    }
}
