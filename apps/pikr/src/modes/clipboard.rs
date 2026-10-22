//! clipboard mode — clipboard history picker.
//!
//! - Unix: shells out to `cliphist list`, then `cliphist decode` into
//!   `wl-copy`.
//! - Windows: the text items of Windows' clipboard history (Win+V), newest
//!   first; with history off, the current clipboard text alone.
//! - Other targets: returns an empty list.

#[cfg(windows)]
use super::Payload;
use super::{Entry, Mode};
use anyhow::Result;

#[derive(Default)]
pub struct Clipboard;

const PREVIEW_MAX: usize = 80;

fn preview_label(text: &str) -> String {
    match text.char_indices().nth(PREVIEW_MAX) {
        Some((boundary, _)) => format!("{}…", &text[..boundary]),
        None => text.to_string(),
    }
}

impl Mode for Clipboard {
    fn collect(&mut self) -> Result<Vec<Entry>> {
        #[cfg(unix)]
        return unix_impl::collect();
        #[cfg(windows)]
        return windows_impl::collect();
        #[cfg(not(any(unix, windows)))]
        return Ok(Vec::new());
    }
}

// ── Unix — cliphist ───────────────────────────────────────────────────────────

#[cfg(unix)]
mod unix_impl {
    use super::{Entry, Result, preview_label};

    pub fn collect() -> Result<Vec<Entry>> {
        // cliphist missing or failing → empty list, not an error; a missing
        // one is warned about, or "No entries." would look like an empty
        // history.
        let output = match std::process::Command::new("cliphist").arg("list").output() {
            Ok(o) if o.status.success() => o.stdout,
            // Not a warning: a history nothing was ever stored in exits 1
            // ("please store something first").
            Ok(o) => {
                tracing::debug!(
                    status = %o.status,
                    stderr = %String::from_utf8_lossy(&o.stderr).trim(),
                    "cliphist list failed; no clipboard history"
                );
                return Ok(vec![]);
            }
            Err(e) => {
                tracing::warn!("cannot run cliphist, which clipboard mode reads: {e}");
                return Ok(vec![]);
            }
        };

        let text = String::from_utf8_lossy(&output);
        let entries = text
            .lines()
            .filter_map(|line| parse_line(line).map(|(id, preview)| make_entry(id, preview)))
            .collect();
        Ok(entries)
    }

    /// Parse a single `cliphist list` output line.
    ///
    /// Format: `<id>\t<preview>` where id is a decimal integer.
    pub fn parse_line(line: &str) -> Option<(u64, String)> {
        let (id_str, preview) = line.split_once('\t')?;
        let id: u64 = id_str.trim().parse().ok()?;
        Some((id, preview.to_string()))
    }

    pub fn make_entry(id: u64, preview: String) -> Entry {
        // ExecWait, not Exec: the accept path must observe the script's
        // exit — a missing wl-copy (or a failed decode) otherwise fails
        // silently and the user's selection never reaches the clipboard.
        Entry::exec_wait(preview_label(&preview), "sh")
            .with_args(vec!["-c".to_string(), copy_script(id)])
    }

    /// The `sh` script that puts history item `id` on the clipboard: decoded
    /// to a temporary file first, and handed to `wl-copy` only if that
    /// worked. Piping straight into `wl-copy` set an empty clipboard and
    /// reported success when the decode failed (an item rotated out of
    /// cliphist's history, a locked database). A file rather than a shell
    /// variable, since an image item is binary. `id` is a parsed integer, so
    /// nothing from the clipboard reaches the script's text.
    pub fn copy_script(id: u64) -> String {
        format!(
            "f=$(mktemp) || exit 1\ntrap 'rm -f \"$f\"' EXIT\ncliphist decode {id} >\"$f\" && wl-copy <\"$f\""
        )
    }
}

// ── Windows — clipboard history (WinRT), else arboard ─────────────────────────
//
// Windows keeps its own history (Settings › System › Clipboard, Win+V) and
// hands it out through WinRT's `Clipboard::GetHistoryItemsAsync`, newest
// first. Only items holding text are listed; images and files are skipped.
// Accepting one puts its text back on the clipboard through arboard, which
// Windows then records as the newest history item.

#[cfg(windows)]
mod windows_impl {
    use super::{Entry, Payload, Result, preview_label};
    use windows::ApplicationModel::DataTransfer::{
        Clipboard, ClipboardHistoryItemsResultStatus, StandardDataFormats,
    };

    pub fn collect() -> Result<Vec<Entry>> {
        match history_texts() {
            Ok(Some(texts)) => {
                tracing::debug!(
                    items = texts.len(),
                    "clipboard: Windows clipboard history read"
                );
                Ok(texts.into_iter().map(make_entry).collect())
            }
            Ok(None) => {
                tracing::info!(
                    "clipboard: Windows clipboard history is off (Settings › System › \
                     Clipboard); showing the current clipboard text only"
                );
                current_text()
            }
            Err(e) => {
                tracing::warn!(
                    "clipboard: cannot read Windows clipboard history, showing the \
                     current clipboard text only: {e:#}"
                );
                current_text()
            }
        }
    }

    /// The text of every history item that holds text, newest first, or
    /// `None` when the user has clipboard history turned off.
    ///
    /// WinRT's `Clipboard` only activates in a single-threaded apartment, and
    /// the calling thread may already be in a multithreaded one, so the read
    /// runs on a thread that joins an STA of its own.
    pub(super) fn history_texts() -> Result<Option<Vec<String>>> {
        std::thread::scope(|s| {
            s.spawn(|| {
                let _com = crate::modes::drun::ComApartment::enter();
                read_history()
            })
            .join()
            .map_err(|_| anyhow::anyhow!("the clipboard history thread panicked"))?
        })
    }

    fn read_history() -> Result<Option<Vec<String>>> {
        if !Clipboard::IsHistoryEnabled()? {
            return Ok(None);
        }
        let result = Clipboard::GetHistoryItemsAsync()?.join()?;
        let status = result.Status()?;
        if status == ClipboardHistoryItemsResultStatus::ClipboardHistoryDisabled {
            return Ok(None);
        }
        anyhow::ensure!(
            status == ClipboardHistoryItemsResultStatus::Success,
            "clipboard history answered with status {}",
            status.0
        );
        let text_format = StandardDataFormats::Text()?;
        let mut texts = Vec::new();
        for item in result.Items()? {
            let content = item.Content()?;
            if !content.Contains(&text_format)? {
                continue;
            }
            let text = content.GetTextAsync()?.join()?.to_string_lossy();
            if !text.is_empty() {
                texts.push(text);
            }
        }
        Ok(Some(texts))
    }

    /// The current clipboard text as the only entry, for when the history
    /// cannot be read.
    fn current_text() -> Result<Vec<Entry>> {
        let mut cb = match arboard::Clipboard::new() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("clipboard: could not open arboard: {e}");
                return Ok(vec![]);
            }
        };

        match cb.get_text() {
            Ok(text) if !text.is_empty() => Ok(vec![make_entry(text)]),
            Ok(_) => Ok(vec![]),
            Err(e) => {
                tracing::warn!("clipboard: could not read clipboard text: {e}");
                Ok(vec![])
            }
        }
    }

    fn make_entry(text: String) -> Entry {
        let label = preview_label(&text);
        Entry {
            label,
            description: None,
            icon: None,
            payload: Payload::SetClipboard(text),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, unix))]
mod tests {
    use super::PREVIEW_MAX;
    use super::unix_impl::{copy_script, make_entry, parse_line};
    use crate::modes::Payload;

    #[test]
    fn parse_cliphist_line() {
        let e1 = parse_line("42\thello world").unwrap();
        assert_eq!(e1.0, 42);
        assert_eq!(e1.1, "hello world");

        let e2 = parse_line("3\t<image>").unwrap();
        assert_eq!(e2.0, 3);
        assert_eq!(e2.1, "<image>");
    }

    #[test]
    fn entry_payload_args() {
        let (id, preview) = parse_line("42\thello world").unwrap();
        let entry = make_entry(id, preview);
        match &entry.payload {
            Payload::ExecWait { program, args } => {
                assert_eq!(program, "sh");
                assert_eq!(args[0], "-c");
                assert!(args[1].contains("cliphist decode 42"));
                assert!(args[1].contains("wl-copy"));
            }
            other => panic!("expected ExecWait payload, got {other:?}"),
        }
    }

    /// Runs [`copy_script`] under `sh` with stand-in `cliphist` and
    /// `wl-copy` first on `PATH`: `cliphist` runs `decode`, and `wl-copy`
    /// records what it was given. Returns the exit status and that record.
    fn run_copy_script(decode: &str) -> (bool, Option<Vec<u8>>) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let tool = |name: &str, body: &str| {
            let path = dir.path().join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        let got = dir.path().join("got");
        tool("cliphist", decode);
        tool("wl-copy", &format!("cat >'{}'", got.display()));
        let path = format!(
            "{}:{}",
            dir.path().display(),
            std::env::var("PATH").unwrap()
        );
        let status = std::process::Command::new("sh")
            .args(["-c", &copy_script(7)])
            .env("PATH", path)
            .status()
            .unwrap();
        (status.success(), std::fs::read(&got).ok())
    }

    #[test]
    fn a_failed_decode_leaves_the_clipboard_alone_and_fails() {
        let (ok, copied) = run_copy_script("exit 1");
        assert!(!ok, "a failed decode must fail the accept");
        assert_eq!(copied, None, "wl-copy must not run");
    }

    #[test]
    fn a_decoded_item_reaches_wl_copy_byte_for_byte() {
        let (ok, copied) = run_copy_script("printf 'pic\\000ture'");
        assert!(ok);
        assert_eq!(copied.as_deref(), Some(&b"pic\0ture"[..]));
    }

    #[test]
    fn long_preview_truncated() {
        let long: String = "x".repeat(100);
        let (id, preview) = parse_line(&format!("1\t{long}")).unwrap();
        let entry = make_entry(id, preview);
        assert!(
            entry.label.len() <= PREVIEW_MAX + 3,
            "label too long: {}",
            entry.label.len()
        );
        assert!(entry.label.ends_with('…'));
    }

    #[test]
    fn unicode_preview_truncates_on_character_boundary() {
        let preview = format!("{}éz", "a".repeat(PREVIEW_MAX - 1));
        let entry = make_entry(42, preview);

        assert_eq!(entry.label, format!("{}é…", "a".repeat(PREVIEW_MAX - 1)));
        assert_eq!(entry.label.chars().count(), PREVIEW_MAX + 1);
        match entry.payload {
            Payload::ExecWait { program, args } => {
                assert_eq!(program, "sh");
                assert_eq!(args, ["-c".to_string(), copy_script(42)]);
            }
            payload => panic!("expected ExecWait payload, got {payload:?}"),
        }
    }

    #[test]
    fn invalid_line_returns_none() {
        assert!(parse_line("no_tab_here").is_none());
        assert!(parse_line("notanumber\tvalue").is_none());
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::windows_impl::{collect, history_texts};

    /// Whether history is on or off, asking for it must succeed: WinRT's
    /// `Clipboard` refuses activation outside a single-threaded apartment,
    /// which a test thread is not in.
    #[test]
    fn clipboard_history_can_be_asked_for() {
        let result = history_texts();
        assert!(result.is_ok(), "history read failed: {result:?}");
    }

    #[test]
    fn collect_does_not_panic() {
        // clipboard state is non-deterministic; we only assert no panic and
        // that the return type is Ok.
        let result = collect();
        assert!(result.is_ok(), "windows collect must return Ok: {result:?}");
    }
}
