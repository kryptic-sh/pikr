//! Per-mode query history with most-recent-first ordering.
//!
//! `Ctrl-P` (HistoryPrev) walks backwards into the history; `Ctrl-N`
//! (HistoryNext) walks forward and eventually restores the live draft the
//! user was typing before they started recalling. Push happens on Accept;
//! duplicates are deduped (newer occurrence wins), capped at [`HISTORY_CAP`]
//! per mode.
//!
//! Stored at `$XDG_STATE_HOME/pikr/history.toml`. Per-mode lists so emoji
//! recall doesn't pollute drun.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::cli::Mode as CliMode;

/// Max entries kept per mode. Older entries fall off the bottom as new ones
/// are pushed. 100 is generous for a launcher (bash defaults to 500 in a
/// shell but launcher queries are shorter-lived).
const HISTORY_CAP: usize = 100;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct History {
    /// Per-mode entries, ordered most-recent first.
    #[serde(flatten)]
    modes: HashMap<String, Vec<String>>,
}

impl History {
    /// Read from `$XDG_STATE_HOME/pikr/history.toml`. Missing file / parse
    /// error / no state dir all collapse to an empty `History`.
    pub fn load() -> Self {
        let Some(path) = state_file_path() else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        toml::from_str(&text).unwrap_or_default()
    }

    /// Persist. Best-effort; warnings via `tracing` on failure.
    pub fn save(&self) {
        let Some(path) = state_file_path() else {
            return;
        };
        if let Some(parent) = path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            tracing::warn!(error = %e, path = %parent.display(), "create state dir");
            return;
        }
        let text = match toml::to_string(self) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(error = %e, "serialize history");
                return;
            }
        };
        if let Err(e) = std::fs::write(&path, text) {
            tracing::warn!(error = %e, path = %path.display(), "write history");
        }
    }

    /// Push `query` to the front of `mode`'s history. Empty / whitespace-only
    /// queries are ignored. Dedupes existing occurrences (the new push wins),
    /// truncates to [`HISTORY_CAP`].
    pub fn push(&mut self, mode: CliMode, query: &str) {
        let q = query.trim();
        if q.is_empty() {
            return;
        }
        let list = self.modes.entry(mode_key(mode)).or_default();
        list.retain(|h| h != q);
        list.insert(0, q.to_string());
        if list.len() > HISTORY_CAP {
            list.truncate(HISTORY_CAP);
        }
    }

    /// Lookup the `index`-th most recent entry for `mode`. `index = 0` →
    /// most recent. Returns `None` past the end.
    pub fn get(&self, mode: CliMode, index: usize) -> Option<&str> {
        self.modes
            .get(&mode_key(mode))?
            .get(index)
            .map(String::as_str)
    }

    /// Number of entries stored for `mode`.
    pub fn len(&self, mode: CliMode) -> usize {
        self.modes.get(&mode_key(mode)).map_or(0, Vec::len)
    }

    pub fn is_empty(&self, mode: CliMode) -> bool {
        self.len(mode) == 0
    }
}

fn mode_key(mode: CliMode) -> String {
    format!("{mode:?}").to_lowercase()
}

fn state_file_path() -> Option<PathBuf> {
    let dirs = xdg::BaseDirectories::with_prefix("pikr").ok()?;
    dirs.place_state_file("history.toml").ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_then_get_returns_most_recent_first() {
        let mut h = History::default();
        h.push(CliMode::Drun, "firefox");
        h.push(CliMode::Drun, "audacity");
        assert_eq!(h.get(CliMode::Drun, 0), Some("audacity"));
        assert_eq!(h.get(CliMode::Drun, 1), Some("firefox"));
    }

    #[test]
    fn push_dedupes_to_most_recent() {
        let mut h = History::default();
        h.push(CliMode::Drun, "firefox");
        h.push(CliMode::Drun, "audacity");
        h.push(CliMode::Drun, "firefox"); // dedupe — should move to front
        assert_eq!(h.len(CliMode::Drun), 2);
        assert_eq!(h.get(CliMode::Drun, 0), Some("firefox"));
        assert_eq!(h.get(CliMode::Drun, 1), Some("audacity"));
    }

    #[test]
    fn push_ignores_blank_and_whitespace() {
        let mut h = History::default();
        h.push(CliMode::Drun, "");
        h.push(CliMode::Drun, "   ");
        h.push(CliMode::Drun, "\t\n");
        assert!(h.is_empty(CliMode::Drun));
    }

    #[test]
    fn push_trims_surrounding_whitespace() {
        // Two pushes of the "same" trimmed query dedupe — saving twice with
        // trailing whitespace shouldn't bloat the history.
        let mut h = History::default();
        h.push(CliMode::Drun, "firefox");
        h.push(CliMode::Drun, "  firefox  ");
        assert_eq!(h.len(CliMode::Drun), 1);
    }

    #[test]
    fn push_caps_at_history_cap() {
        let mut h = History::default();
        for i in 0..(HISTORY_CAP + 25) {
            h.push(CliMode::Drun, &format!("q{i}"));
        }
        assert_eq!(h.len(CliMode::Drun), HISTORY_CAP);
        // The latest push must still be at the front.
        let expected = format!("q{}", HISTORY_CAP + 24);
        assert_eq!(h.get(CliMode::Drun, 0), Some(expected.as_str()));
    }

    #[test]
    fn modes_are_independent() {
        let mut h = History::default();
        h.push(CliMode::Drun, "firefox");
        assert_eq!(h.len(CliMode::Drun), 1);
        assert_eq!(h.len(CliMode::Ssh), 0);
        assert_eq!(h.get(CliMode::Ssh, 0), None);
    }

    #[test]
    fn get_past_end_is_none() {
        let mut h = History::default();
        h.push(CliMode::Drun, "firefox");
        assert_eq!(h.get(CliMode::Drun, 5), None);
    }

    #[test]
    fn serialize_roundtrip() {
        let mut h = History::default();
        h.push(CliMode::Emoji, "smile");
        h.push(CliMode::Emoji, "heart");
        let text = toml::to_string(&h).unwrap();
        let back: History = toml::from_str(&text).unwrap();
        assert_eq!(back.get(CliMode::Emoji, 0), Some("heart"));
        assert_eq!(back.get(CliMode::Emoji, 1), Some("smile"));
    }
}
