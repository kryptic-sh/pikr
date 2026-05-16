//! clipboard mode — cliphist picker.

use super::{Entry, Mode, Payload};
use anyhow::Result;

#[derive(Default)]
pub struct Clipboard;

impl Mode for Clipboard {
    fn name(&self) -> &'static str {
        "clipboard"
    }

    fn collect(&mut self) -> Result<Vec<Entry>> {
        let output = match std::process::Command::new("cliphist").arg("list").output() {
            Ok(o) if o.status.success() => o.stdout,
            // cliphist missing or non-zero exit → empty list, not an error.
            _ => return Ok(vec![]),
        };

        let text = String::from_utf8_lossy(&output);
        let entries = text
            .lines()
            .filter_map(|line| parse_line(line).map(|(id, preview)| make_entry(id, preview)))
            .collect();
        Ok(entries)
    }
}

/// Parse a single `cliphist list` output line.
///
/// Format: `<id>\t<preview>` where id is a decimal integer.
pub fn parse_line(line: &str) -> Option<(u64, String)> {
    let (id_str, preview) = line.split_once('\t')?;
    let id: u64 = id_str.trim().parse().ok()?;
    Some((id, preview.to_string()))
}

const PREVIEW_MAX: usize = 80;

fn make_entry(id: u64, preview: String) -> Entry {
    let label = if preview.len() > PREVIEW_MAX {
        format!("{}…", &preview[..PREVIEW_MAX])
    } else {
        preview
    };
    Entry {
        label,
        description: None,
        payload: Payload::Exec {
            program: "sh".to_string(),
            args: vec!["-c".to_string(), format!("cliphist decode {id} | wl-copy")],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
            Payload::Exec { program, args } => {
                assert_eq!(program, "sh");
                assert_eq!(args[0], "-c");
                assert!(args[1].contains("cliphist decode 42"));
                assert!(args[1].contains("wl-copy"));
            }
            _ => panic!("expected Exec payload"),
        }
    }

    #[test]
    fn long_preview_truncated() {
        let long: String = "x".repeat(100);
        let (id, preview) = parse_line(&format!("1\t{long}")).unwrap();
        let entry = make_entry(id, preview);
        assert!(
            entry.label.len() <= 81 + 3,
            "label too long: {}",
            entry.label.len()
        );
        assert!(entry.label.ends_with('…'));
    }

    #[test]
    fn invalid_line_returns_none() {
        assert!(parse_line("no_tab_here").is_none());
        assert!(parse_line("notanumber\tvalue").is_none());
    }
}
