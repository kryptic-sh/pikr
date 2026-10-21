//! Per-mode usage tracking with half-life decay scoring.
//!
//! Each Accept bumps a counter and updates the last-used timestamp for the
//! payload's key. At rank time the matcher score is adjusted by a frecency
//! bonus computed as `count · exp(-Δt / HALF_LIFE)` — heavily-used recent
//! entries surface above rarely-used ones with equal text match.
//!
//! Stored as `usage.toml` in pikr's state dir (see
//! [`crate::picker::state_file_path`]). Per-mode tables so different pickers
//! don't cross-contaminate (drun won't bias ssh ordering).

use std::collections::HashMap;
use std::fmt::Write;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::cli::Mode as CliMode;
use crate::modes::{Entry, Payload};

/// Half-life for the recency decay. After this much time, a single use
/// counts for half its original weight. Set to 14 days so a launcher
/// favourite stays prominent for a couple of weeks before fading.
const HALF_LIFE: Duration = Duration::from_secs(14 * 24 * 60 * 60);

/// Multiplier applied to the decayed count to produce a u16 score bonus.
/// Sized so a small number of repeat accepts visibly bumps an entry past a
/// fresh nucleo hit (raw nucleo scores top out around the high hundreds for
/// strong matches).
const BONUS_SCALE: f64 = 80.0;

/// The most entries one mode keeps on disk. Entries that no longer move the
/// ranking are dropped first anyway (see [`Usage::prune`]); this bounds a
/// burst of distinct accepts within that window, such as a clipboard history
/// piped through dmenu.
const USAGE_CAP: usize = 1000;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageEntry {
    pub count: u32,
    /// Seconds since UNIX_EPOCH. Stored as i64 to survive system clocks set
    /// before 1970 (defensive — the toml serializer accepts negatives).
    pub last_used: i64,
}

/// In-memory frecency database. One `HashMap<key, UsageEntry>` per CLI mode.
/// `key` is the payload-derived string returned by [`payload_key`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    #[serde(flatten)]
    modes: HashMap<String, HashMap<String, UsageEntry>>,
}

impl Usage {
    /// Read the usage table from `usage.toml` in the state dir. Missing
    /// file / parse error / no state dir all collapse to an empty `Usage` —
    /// frecency is a soft optimisation, never a hard dependency.
    pub fn load() -> Self {
        let Some(path) = crate::picker::state_file_path("usage.toml") else {
            return Self::default();
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        Self::from_toml(&text)
    }

    /// `usage.toml`'s text parsed, without the clipboard section an older
    /// pikr wrote (see [`Usage::record`]), so the next save removes it.
    fn from_toml(text: &str) -> Self {
        let mut usage: Self = toml::from_str(text).unwrap_or_default();
        usage.modes.remove(&CliMode::Clipboard.key());
        usage
    }

    /// Persist to `usage.toml` in the state dir. Best-effort; errors are
    /// logged via tracing but never surfaced — losing frecency state on a
    /// disk-full / permission-denied is preferable to crashing the launcher.
    pub fn save(&self) {
        let Some(path) = crate::picker::state_file_path("usage.toml") else {
            return;
        };
        if let Some(parent) = path.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            tracing::warn!(error = %e, path = %parent.display(), "create state dir");
            return;
        }
        let mut pruned = self.clone();
        pruned.prune(unix_seconds(SystemTime::now()));
        let text = match toml::to_string(&pruned) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(error = %e, "serialize usage");
                return;
            }
        };
        if let Err(e) = crate::picker::write_private_state(&path, text.as_bytes()) {
            tracing::warn!(error = %e, path = %path.display(), "write usage");
        }
    }

    /// Drops the entries whose bonus at `now_secs` rounds to nothing, which
    /// can no longer change a ranking (a single accept fades out within a few
    /// [`HALF_LIFE`]s), then, in a mode with more than [`USAGE_CAP`] left, the
    /// lowest-scoring ones.
    fn prune(&mut self, now_secs: i64) {
        for entries in self.modes.values_mut() {
            entries.retain(|_, entry| score_bonus(entry, now_secs) > 0);
            if entries.len() > USAGE_CAP {
                let mut ranked: Vec<(u16, i64, String)> = entries
                    .iter()
                    .map(|(key, entry)| {
                        (score_bonus(entry, now_secs), entry.last_used, key.clone())
                    })
                    .collect();
                ranked.sort_unstable_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
                let kept: std::collections::HashSet<String> = ranked
                    .into_iter()
                    .take(USAGE_CAP)
                    .map(|(_, _, key)| key)
                    .collect();
                entries.retain(|key, _| kept.contains(key));
            }
        }
        self.modes.retain(|_, entries| !entries.is_empty());
    }

    /// Record an accept. `cli_mode` is the current picker mode; `payload` is
    /// what `modes::execute` would run. Stamps `last_used` with the current
    /// time. Saving is the caller's responsibility (we batch a single save
    /// at the call site so a Visual-mode multi-accept doesn't fsync N times).
    ///
    /// Clipboard mode is not recorded: its rows are passing clipboard
    /// contents, so a key would be the copied text itself (a password,
    /// written to disk) or a `cliphist` id that never comes back.
    pub fn record(&mut self, cli_mode: CliMode, payload: &Payload) {
        if cli_mode == CliMode::Clipboard {
            return;
        }
        let now = unix_seconds(SystemTime::now());
        let key = payload_key(payload);
        let entry = self
            .modes
            .entry(cli_mode.key())
            .or_default()
            .entry(key)
            .or_default();
        entry.count = entry.count.saturating_add(1);
        entry.last_used = now;
    }

    /// Compute the score bonus for a precomputed key under the mode `mode_key`
    /// names, as of `now`. Returns 0 for never-used keys.
    pub fn bonus_for_key(&self, mode_key: &str, key: &str, now: SystemTime) -> u16 {
        let Some(per_mode) = self.modes.get(mode_key) else {
            return 0;
        };
        let Some(entry) = per_mode.get(key) else {
            return 0;
        };
        score_bonus(entry, unix_seconds(now))
    }

    /// Compute the score bonus for `payload` under the mode `mode_key` names,
    /// as of `now`. Returns 0 for never-used payloads.
    #[cfg(test)]
    pub fn bonus(&self, mode_key: &str, payload: &Payload, now: SystemTime) -> u16 {
        let key = payload_key(payload);
        self.bonus_for_key(mode_key, &key, now)
    }
}

/// Precompute the usage key for every entry once at collect time, so the
/// per-keystroke rerank bonus loop looks up by key instead of rebuilding
/// (and hashing) a fresh String per entry.
pub(crate) fn entry_keys(entries: &[Arc<Entry>]) -> Vec<String> {
    entries.iter().map(|e| payload_key(&e.payload)).collect()
}

/// Stable key used to look up an entry's usage record. Two entries with the
/// same payload share a usage record; this is what the user expects (the
/// "same" Firefox shortcut whether it appeared in drun once or twice).
fn payload_key(payload: &Payload) -> String {
    match payload {
        Payload::Stdout(s) => s.clone(),
        Payload::Exec { program, args } | Payload::ExecWait { program, args } => {
            // Length-prefix every component so the key is injective within
            // the Exec/ExecWait variants: a program or arg containing U+001F
            // (legal in a Unix path, and shlex splits on whitespace only, so
            // it survives inside .desktop args too) can never collide with a
            // different program/arg split. The `{len}:` prefix fixes each
            // boundary unambiguously regardless of what the components
            // contain. NOT injective across Payload variants — e.g.
            // Stdout("2:ab") and Exec{program:"ab"} share the key — but each
            // mode emits a single variant today, so the cross-variant
            // collision is unreachable.
            let mut key = format!("{}:{program}", program.len());
            for arg in args {
                // write! into a String is infallible; `let _ =` consumes the
                // Result so the must-use lint stays quiet without a panic.
                let _ = write!(key, "\u{1f}{}:{arg}", arg.len());
            }
            key
        }
        Payload::SetClipboard(text) | Payload::ShellApp(text) => text.clone(),
    }
}

fn unix_seconds(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        // Pre-1970 clock — return 0 so frecency math still works.
        Err(_) => 0,
    }
}

/// `count · exp(-Δt / HALF_LIFE)` · `BONUS_SCALE`, saturated to `u16::MAX`.
fn score_bonus(entry: &UsageEntry, now_secs: i64) -> u16 {
    if entry.count == 0 {
        return 0;
    }
    // `last_used` comes from the state file: a corrupt one must not overflow.
    let dt = now_secs.saturating_sub(entry.last_used).max(0) as f64;
    let half_life_secs = HALF_LIFE.as_secs() as f64;
    // 2^(-dt/HALF_LIFE) — equivalent to 0.5^(dt/HALF_LIFE), cheaper than powf.
    let decay = (-dt / half_life_secs).exp2();
    let bonus = (entry.count as f64) * decay * BONUS_SCALE;
    bonus.clamp(0.0, u16::MAX as f64) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now_secs() -> i64 {
        unix_seconds(SystemTime::now())
    }

    #[test]
    fn unused_entry_returns_zero_bonus() {
        let usage = Usage::default();
        let payload = Payload::Stdout("never-seen".into());
        assert_eq!(
            usage.bonus(&CliMode::Drun.key(), &payload, SystemTime::now()),
            0
        );
    }

    #[test]
    fn fresh_accept_produces_nonzero_bonus() {
        let mut usage = Usage::default();
        let payload = Payload::Exec {
            program: "firefox".into(),
            args: vec!["%U".into()],
        };
        usage.record(CliMode::Drun, &payload);
        let bonus = usage.bonus(&CliMode::Drun.key(), &payload, SystemTime::now());
        assert!(bonus > 0, "freshly-accepted entry must get a bonus");
    }

    #[test]
    fn bonus_for_key_matches_payload_bonus() {
        // The key-taking variant must agree exactly with the payload-taking
        // one — the rerank loop uses it with precomputed keys.
        let mut usage = Usage::default();
        let payload = Payload::Exec {
            program: "firefox".into(),
            args: vec!["%U".into()],
        };
        usage.record(CliMode::Drun, &payload);
        let now = SystemTime::now();
        let key = payload_key(&payload);
        let via_payload = usage.bonus(&CliMode::Drun.key(), &payload, now);
        let via_key = usage.bonus_for_key(&CliMode::Drun.key(), &key, now);
        assert_eq!(via_payload, via_key);
        assert!(via_key > 0, "recorded payload must get a bonus");
    }

    #[test]
    fn repeated_accepts_increase_bonus() {
        let mut usage = Usage::default();
        let payload = Payload::Stdout("foo".into());
        usage.record(CliMode::Run, &payload);
        let one = usage.bonus(&CliMode::Run.key(), &payload, SystemTime::now());
        for _ in 0..4 {
            usage.record(CliMode::Run, &payload);
        }
        let five = usage.bonus(&CliMode::Run.key(), &payload, SystemTime::now());
        assert!(
            five > one,
            "5 accepts should outweigh 1 (got {one} vs {five})"
        );
    }

    #[test]
    fn decay_reduces_bonus_over_half_life() {
        // Same count, but one entry is half a life-time old. The older one
        // must score lower.
        let now = now_secs();
        let fresh = UsageEntry {
            count: 4,
            last_used: now,
        };
        let half_old = UsageEntry {
            count: 4,
            last_used: now - (HALF_LIFE.as_secs() as i64),
        };
        let fresh_bonus = score_bonus(&fresh, now);
        let old_bonus = score_bonus(&half_old, now);
        // 1 half-life back → ~half the weight.
        assert!(
            (old_bonus as f64) < (fresh_bonus as f64) * 0.6,
            "half-life decay should roughly halve the bonus (fresh={fresh_bonus}, old={old_bonus})"
        );
    }

    #[test]
    fn modes_are_independent() {
        // Recording in drun must not bias ssh.
        let mut usage = Usage::default();
        let payload = Payload::Stdout("foo".into());
        usage.record(CliMode::Drun, &payload);
        assert!(usage.bonus(&CliMode::Drun.key(), &payload, SystemTime::now()) > 0);
        assert_eq!(
            usage.bonus(&CliMode::Ssh.key(), &payload, SystemTime::now()),
            0
        );
    }

    #[test]
    fn exec_payload_with_args_keyed_uniquely() {
        // firefox alone and firefox-with-args produce different keys, so a
        // user's last-used-with-default-profile entry doesn't get bumped
        // every time they accept a `firefox --kiosk URL` shortcut.
        let p1 = Payload::Exec {
            program: "firefox".into(),
            args: vec![],
        };
        let p2 = Payload::Exec {
            program: "firefox".into(),
            args: vec!["--kiosk".into(), "https://x".into()],
        };
        assert_ne!(payload_key(&p1), payload_key(&p2));
    }

    #[test]
    fn stdout_payload_keyed_by_value() {
        let p = Payload::Stdout("😀".into());
        assert_eq!(payload_key(&p), "😀");
    }

    #[test]
    fn exec_program_with_space_distinct_from_program_plus_arg() {
        // Regression for the space-join ambiguity: `Exec{program:"a b"}`
        // and `Exec{program:"a", args:["b"]}` stringified to the same
        // "a b" key and shared one usage record. The unit-separator join
        // must keep them apart.
        let p1 = Payload::Exec {
            program: "a b".into(),
            args: vec![],
        };
        let p2 = Payload::Exec {
            program: "a".into(),
            args: vec!["b".into()],
        };
        assert_ne!(payload_key(&p1), payload_key(&p2));
    }

    #[test]
    fn exec_single_arg_with_space_distinct_from_two_args() {
        // Same program, different arg split: `["y z"]` vs `["y","z"]` both
        // joined to "x y z" under the old space-join. The unit-separator
        // join keeps them distinct — U+001F can't appear in shlex-split
        // args, so the join is unambiguous.
        let single = Payload::Exec {
            program: "x".into(),
            args: vec!["y z".into()],
        };
        let split = Payload::Exec {
            program: "x".into(),
            args: vec!["y".into(), "z".into()],
        };
        assert_ne!(payload_key(&single), payload_key(&split));
    }

    #[test]
    fn a_corrupt_last_used_gives_no_bonus_instead_of_overflowing() {
        let entry = UsageEntry {
            count: 5,
            last_used: i64::MIN,
        };
        assert_eq!(score_bonus(&entry, now_secs()), 0);
    }

    #[test]
    fn prune_drops_entries_that_no_longer_rank_and_keeps_the_rest() {
        let now = now_secs();
        let day = 24 * 60 * 60;
        let mut usage = Usage::default();
        let drun = usage.modes.entry("drun".into()).or_default();
        drun.insert(
            "fresh".into(),
            UsageEntry {
                count: 1,
                last_used: now,
            },
        );
        // One accept 100 days ago: 80 * 2^(-100/14) rounds to nothing.
        drun.insert(
            "faded".into(),
            UsageEntry {
                count: 1,
                last_used: now - 100 * day,
            },
        );
        // Many accepts as long ago still count.
        drun.insert(
            "staple".into(),
            UsageEntry {
                count: 400,
                last_used: now - 100 * day,
            },
        );
        usage.modes.entry("emoji".into()).or_default().insert(
            "gone".into(),
            UsageEntry {
                count: 1,
                last_used: now - 200 * day,
            },
        );
        usage.prune(now);
        let drun = &usage.modes["drun"];
        assert!(drun.contains_key("fresh") && drun.contains_key("staple"));
        assert!(!drun.contains_key("faded"));
        assert!(
            !usage.modes.contains_key("emoji"),
            "an emptied mode is dropped"
        );
    }

    #[test]
    fn prune_caps_a_mode_keeping_the_highest_scores() {
        let now = now_secs();
        let mut usage = Usage::default();
        let dmenu = usage.modes.entry("dmenu".into()).or_default();
        for i in 0..USAGE_CAP + 5 {
            // Older entries score lower.
            dmenu.insert(
                format!("k{i}"),
                UsageEntry {
                    count: 1,
                    last_used: now - i as i64,
                },
            );
        }
        usage.prune(now);
        let dmenu = &usage.modes["dmenu"];
        assert_eq!(dmenu.len(), USAGE_CAP);
        assert!(dmenu.contains_key("k0"));
        assert!(!dmenu.contains_key(&format!("k{}", USAGE_CAP + 4)));
    }

    #[test]
    fn clipboard_accepts_are_not_recorded() {
        let mut usage = Usage::default();
        usage.record(CliMode::Clipboard, &Payload::SetClipboard("hunter2".into()));
        assert!(usage.modes.is_empty());
        let text = toml::to_string(&usage).unwrap();
        assert!(!text.contains("hunter2"));
    }

    #[test]
    fn an_older_clipboard_section_is_dropped_on_load() {
        let text = "[clipboard.hunter2]\ncount = 3\nlast_used = 1\n\n[emoji.x]\ncount = 1\nlast_used = 1\n";
        let usage = Usage::from_toml(text);
        assert!(!usage.modes.contains_key("clipboard"));
        assert!(usage.modes.contains_key("emoji"));
    }

    #[test]
    fn serialize_roundtrip() {
        let mut usage = Usage::default();
        let payload = Payload::Stdout("hello".into());
        usage.record(CliMode::Emoji, &payload);
        let text = toml::to_string(&usage).unwrap();
        let back: Usage = toml::from_str(&text).unwrap();
        assert!(back.bonus(&CliMode::Emoji.key(), &payload, SystemTime::now()) > 0);
    }

    #[test]
    fn exec_program_with_unit_separator_distinct_from_program_plus_arg() {
        // The exact backlog collision: `a\u{1f}b` as a bare program vs
        // program `a` with arg `b` — the U+001F join stringified both to
        // "a\u{1f}b". Length-prefixing must keep them apart.
        let p1 = Payload::Exec {
            program: "a\u{1f}b".into(),
            args: vec![],
        };
        let p2 = Payload::Exec {
            program: "a".into(),
            args: vec!["b".into()],
        };
        assert_ne!(payload_key(&p1), payload_key(&p2));
    }

    #[test]
    fn exec_arg_with_unit_separator_distinct_from_two_args() {
        // shlex splits on whitespace only, so U+001F can survive inside a
        // single arg — `["a\u{1f}b"]` must not collide with `["a","b"]`.
        let single = Payload::Exec {
            program: "x".into(),
            args: vec!["a\u{1f}b".into()],
        };
        let split = Payload::Exec {
            program: "x".into(),
            args: vec!["a".into(), "b".into()],
        };
        assert_ne!(payload_key(&single), payload_key(&split));
    }

    #[test]
    fn exec_program_with_length_prefix_style_name_is_injective() {
        // A program that LOOKS like the new encoding ("1:a") must not
        // collide with the encoding of a different payload.
        let bare = Payload::Exec {
            program: "1:a".into(),
            args: vec![],
        };
        let arg = Payload::Exec {
            program: "1".into(),
            args: vec!["a".into()],
        };
        assert_ne!(payload_key(&bare), payload_key(&arg));
        let with_sep = Payload::Exec {
            program: "1:a\u{1f}1:b".into(),
            args: vec![],
        };
        let split = Payload::Exec {
            program: "a".into(),
            args: vec!["b".into()],
        };
        assert_ne!(payload_key(&with_sep), payload_key(&split));
    }
}
