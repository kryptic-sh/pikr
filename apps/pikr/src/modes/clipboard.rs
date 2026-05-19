//! clipboard mode — clipboard history picker.
//!
//! - Unix: shells out to `cliphist list` / `cliphist decode | wl-copy`.
//! - Windows: reads current clipboard text via `arboard`; single entry shown.
//!   Full per-item history requires Windows 10 1809+ WinRT (future work).
//! - Other targets: returns an empty list.

use super::{Entry, Mode, Payload};
use anyhow::Result;

#[derive(Default)]
pub struct Clipboard;

impl Mode for Clipboard {
    fn name(&self) -> &'static str {
        "clipboard"
    }

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
    use super::{Entry, Payload, Result};

    pub fn collect() -> Result<Vec<Entry>> {
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

    /// Parse a single `cliphist list` output line.
    ///
    /// Format: `<id>\t<preview>` where id is a decimal integer.
    pub fn parse_line(line: &str) -> Option<(u64, String)> {
        let (id_str, preview) = line.split_once('\t')?;
        let id: u64 = id_str.trim().parse().ok()?;
        Some((id, preview.to_string()))
    }

    pub const PREVIEW_MAX: usize = 80;

    pub fn make_entry(id: u64, preview: String) -> Entry {
        let label = if preview.len() > PREVIEW_MAX {
            format!("{}…", &preview[..PREVIEW_MAX])
        } else {
            preview
        };
        Entry {
            label,
            description: None,
            icon: None,
            payload: Payload::Exec {
                program: "sh".to_string(),
                args: vec!["-c".to_string(), format!("cliphist decode {id} | wl-copy")],
            },
        }
    }
}

// ── Windows — arboard (current clipboard text) ────────────────────────────────
//
// arboard exposes only the *current* clipboard item, not the full Windows
// clipboard history (Win32 ClipboardHistory WinRT API). This means the picker
// shows a single entry for the text currently on the clipboard. Selecting it
// is effectively a no-op (it re-sets the same text), but it keeps the
// interaction model consistent and confirms the clipboard is accessible.
// Full history support via WinRT is tracked in issue #37 as future work.

#[cfg(windows)]
mod windows_impl {
    use super::{Entry, Payload, Result};

    pub fn collect() -> Result<Vec<Entry>> {
        let mut cb = match arboard::Clipboard::new() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("clipboard: could not open arboard: {e}");
                return Ok(vec![]);
            }
        };

        match cb.get_text() {
            Ok(text) if !text.is_empty() => {
                tracing::info!(
                    "clipboard: Windows arboard — showing current clipboard text only \
                     (full history requires WinRT; see issue #37)"
                );
                Ok(vec![make_entry(text)])
            }
            Ok(_) => Ok(vec![]),
            Err(e) => {
                tracing::warn!("clipboard: could not read clipboard text: {e}");
                Ok(vec![])
            }
        }
    }

    const PREVIEW_MAX: usize = 80;

    fn make_entry(text: String) -> Entry {
        let label = if text.len() > PREVIEW_MAX {
            format!("{}…", &text[..PREVIEW_MAX])
        } else {
            text.clone()
        };
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
    use super::unix_impl::{PREVIEW_MAX, make_entry, parse_line};
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
            entry.label.len() <= PREVIEW_MAX + 3,
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

#[cfg(all(test, windows))]
mod windows_tests {
    use super::windows_impl::collect;

    #[test]
    fn collect_does_not_panic() {
        // clipboard state is non-deterministic; we only assert no panic and
        // that the return type is Ok.
        let result = collect();
        assert!(result.is_ok(), "windows collect must return Ok: {result:?}");
    }
}
