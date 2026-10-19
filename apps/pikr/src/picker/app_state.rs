//! The picker session: entries, ranking, mode switching, and the
//! toolkit-independent rules the key handler applies (`--kb-custom`
//! matching, Visual-mode ranges, selection motion).

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use crate::cli::Mode as CliMode;
use crate::modes::Entry;
use crate::picker::input::{Key, Modifiers, NamedKey};
use crate::picker::keys::Action;
use crate::picker::matcher::{Match, Matcher};
use crate::picker::state::{PickerState, VimMode};

/// A pending `--kb-custom KEY=PROMPT` confirmation, shown as a card on the
/// highlighted row until Enter (accept) or Esc / Left (dismiss).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfirmCard {
    pub binding: usize,
    pub prompt: String,
}

/// True when `key` would move the query caret in `mode` from `cursor` (of
/// `len` chars): Left/Right in Insert and Normal, Home/End in Insert only
/// (Normal-mode Home/End jump the list). A binding on such a key yields to
/// the caret, so it fires only once the caret can't move that way.
fn caret_would_move(
    key: &crate::picker::keyspec::KeySpec,
    mode: VimMode,
    cursor: usize,
    len: usize,
) -> bool {
    let (insert, normal) = (mode == VimMode::Insert, mode == VimMode::Normal);
    let right = ((insert || normal) && key.is_named(NamedKey::ArrowRight))
        || (insert && key.is_named(NamedKey::End));
    let left = ((insert || normal) && key.is_named(NamedKey::ArrowLeft))
        || (insert && key.is_named(NamedKey::Home));
    (right && cursor < len) || (left && cursor > 0)
}

/// The `--kb-custom` action a key press triggers, if any. Bindings belong to
/// dmenu, so they stop applying after `:mode` switches away from it. An
/// unmodified binding on a caret key yields while the caret can still move
/// (see [`caret_would_move`]); a modified one such as `Alt+Right` always
/// fires. `caret` is the query cursor and the query length, in chars.
pub(crate) fn kb_custom_action(
    bindings: &[crate::picker::keyspec::KbCustom],
    cli_mode: CliMode,
    vim_mode: VimMode,
    key: &Key,
    mods: Modifiers,
    caret: (usize, usize),
) -> Option<Action> {
    if cli_mode != CliMode::Dmenu {
        return None;
    }
    let i = bindings.iter().position(|b| {
        b.key.matches(key, mods)
            && (b.key.has_modifiers() || !caret_would_move(&b.key, vim_mode, caret.0, caret.1))
    })?;
    Some(if bindings[i].confirm.is_some() {
        Action::ConfirmKbCustom(i)
    } else {
        Action::AcceptKbCustom(i)
    })
}

/// Actions ignored while `--loading` rows are still pending. With no rows
/// yet, dmenu's no-match fallthrough would print the typed query.
pub(crate) fn needs_loaded_rows(action: &Action) -> bool {
    matches!(
        action,
        Action::Accept
            | Action::AcceptCustom
            | Action::AcceptKbCustom(_)
            | Action::ConfirmKbCustom(_)
    )
}

/// Rows an accept acts on: the anchored range in Visual mode, otherwise just
/// the cursor row.
pub(crate) fn selection_range(mode: VimMode, anchor: Option<usize>, sel: usize) -> Vec<usize> {
    match (mode, anchor) {
        (VimMode::Visual, Some(a)) => (a.min(sel)..=a.max(sel)).collect(),
        _ => vec![sel],
    }
}

pub struct AppState {
    pub picker: PickerState,
    pub entries: Vec<Arc<Entry>>,
    /// Usage keys parallel to `entries` (one per entry, same order), so the
    /// per-keystroke bonus loop looks up frecency by key instead of rebuilding
    /// a String per entry. Rebuilt whenever `entries` is replaced.
    pub usage_keys: Vec<String>,
    /// Match keys parallel to `entries`, so a keystroke's rank does no
    /// Unicode segmentation. Rebuilt with `usage_keys`.
    pub match_keys: Vec<crate::picker::matcher::MatchKey>,
    pub matches: Vec<Match>,
    pub g_pending: bool,
    pub cli_mode: CliMode,
    pub prompt: String,
    pub max_results: usize,
    pub matcher: Matcher,
    /// When true, the query bar renders each character as ● (U+25CF) instead
    /// of the actual glyph. The underlying `query` signal still holds the
    /// real text — masking is display-only.
    pub password: bool,
    /// Per-mode frecency (count × half-life decay). Loaded from disk at
    /// startup; bumped on Accept and persisted then.
    pub usage: crate::picker::frecency::Usage,
    /// Per-mode query history (most-recent first). Pushed on Accept (with
    /// non-empty query), recalled via Ctrl-P/Ctrl-N in Insert mode.
    pub history: crate::picker::history::History,
    /// Precomputed expr → result for the calc-history entries, built once at
    /// startup (see modes::calc::precompute).
    pub calc_results: HashMap<String, String>,
    /// `--kb-custom` bindings, in flag order. Checked before the vim keymap;
    /// binding N accepts the selection and exits `KB_CUSTOM_EXIT_BASE + N`.
    pub kb_custom: Vec<crate::picker::keyspec::KbCustom>,
}

impl AppState {
    /// Replaces the entries and rebuilds everything kept parallel to them.
    pub fn set_entries(&mut self, entries: Vec<Arc<Entry>>) {
        self.usage_keys = crate::picker::frecency::entry_keys(&entries);
        self.match_keys = crate::picker::matcher::match_keys(&entries);
        self.entries = entries;
    }

    pub fn rerank(&mut self) {
        let query = self.picker.query.clone();
        tracing::debug!(
            mode = ?self.cli_mode,
            query_len = query.chars().count(),
            "picker query reranked"
        );
        // -P is a bare password prompt: rows ranked against the secret
        // would reveal which of them it matches.
        if self.password {
            self.matches.clear();
            return;
        }
        if matches!(self.cli_mode, CliMode::Calc) {
            self.rerank_calc(&query);
            return;
        }
        let mut ranked = self.matcher.rank(&self.entries, &self.match_keys, &query);
        // Frecency bonus: per-entry, derived from accept history. Added to
        // the nucleo score then re-sort, so heavily-used recent payloads
        // surface above one-off matches with equal text score. Empty query
        // also benefits — that's when you want the launcher to show the
        // app you actually launch every day first.
        let now = std::time::SystemTime::now();
        let mode_key = self.cli_mode.key();
        for m in &mut ranked {
            let bonus = self
                .usage
                .bonus_for_key(&mode_key, &self.usage_keys[m.index], now);
            m.score = m.score.saturating_add(bonus);
        }
        ranked.sort_by(|a, b| b.score.cmp(&a.score).then(a.index.cmp(&b.index)));
        self.matches = ranked;
        self.matches.truncate(self.max_results);
        self.picker.clamp_selected(self.matches.len());
    }

    /// Calc-mode rerank: build a row list from on-disk history plus the live
    /// query's eval result. The live entry sits on top (when the expression
    /// evaluates), historic entries below are fuzzy-filtered through the
    /// shared matcher. Pure-text queries that don't evaluate (e.g. `10+`)
    /// fall through to history-only ranking so the user can still scrub past
    /// expressions while typing a new one.
    fn rerank_calc(&mut self, query: &str) {
        let empty: Rc<Vec<u32>> = Rc::new(Vec::new());
        let trimmed = query.trim();
        let live_eval = if trimmed.is_empty() {
            None
        } else {
            crate::modes::calc::eval(query).map(|r| (trimmed.to_string(), r))
        };

        let history: Vec<(String, String)> = self
            .history
            .list(CliMode::Calc)
            .iter()
            .filter(|expr| {
                // Dedupe: when the live query exactly matches a stored entry,
                // skip the historic row so it doesn't render twice.
                live_eval
                    .as_ref()
                    .is_none_or(|(live_expr, _)| live_expr.as_str() != expr.as_str())
            })
            .filter_map(|expr| {
                self.calc_results
                    .get(expr)
                    .map(|result| (expr.clone(), result.clone()))
            })
            .collect();

        let mk_entry = |expr: &str, result: &str| {
            Arc::new(Entry {
                label: format!("{expr} = {result}"),
                description: None,
                icon: None,
                payload: crate::modes::Payload::Stdout(result.to_owned()),
            })
        };
        let mut entries: Vec<Arc<Entry>> = Vec::with_capacity(history.len() + 1);
        if let Some((expr, result)) = live_eval.as_ref() {
            entries.push(mk_entry(expr, result));
        }
        for (expr, result) in &history {
            entries.push(mk_entry(expr, result));
        }
        self.set_entries(entries);

        let live_offset = usize::from(live_eval.is_some());
        let mut matches: Vec<Match> = Vec::new();
        if live_eval.is_some() {
            // Live entry always sits at the top, unfiltered, with no
            // highlight spans (the whole label is the user's input).
            matches.push(Match {
                index: 0,
                score: u16::MAX,
                positions: empty.clone(),
                desc_positions: empty.clone(),
            });
        }

        if trimmed.is_empty() {
            // Empty query → show every historic entry verbatim, newest first.
            for i in 0..history.len() {
                matches.push(Match {
                    index: live_offset + i,
                    score: 0,
                    positions: empty.clone(),
                    desc_positions: empty.clone(),
                });
            }
        } else {
            // Non-empty query → fuzzy-rank history labels against the query
            // so users can recall past expressions while still typing.
            let ranked = self.matcher.rank(
                &self.entries[live_offset..],
                &self.match_keys[live_offset..],
                query,
            );
            for mut m in ranked {
                m.index += live_offset;
                matches.push(m);
            }
        }

        self.matches = matches;
        self.matches.truncate(self.max_results);
        self.picker.clamp_selected(self.matches.len());
    }

    pub fn switch_mode(&mut self, mode: CliMode) {
        self.cli_mode = mode;
        // Unsupported-on-this-platform modes resolve to `None`: the mode
        // label switches but the list is cleared rather than showing the
        // previous mode's entries under the new label. Unix behavior is
        // unchanged — every mode constructs its real Mode.
        let m: Option<Box<dyn crate::modes::Mode>> = match mode {
            CliMode::Calc => Some(Box::new(crate::modes::calc::Calc)),
            CliMode::Clipboard => Some(Box::new(crate::modes::clipboard::Clipboard)),
            CliMode::Dmenu => Some(Box::new(crate::modes::dmenu::Dmenu)),
            #[cfg(unix)]
            CliMode::Drun => Some(Box::new(crate::modes::drun::Drun)),
            #[cfg(not(unix))]
            CliMode::Drun => None,
            CliMode::Emoji => Some(Box::new(crate::modes::emoji::Emoji)),
            #[cfg(unix)]
            CliMode::Run => Some(Box::new(crate::modes::run::Run)),
            #[cfg(not(unix))]
            CliMode::Run => None,
            #[cfg(unix)]
            CliMode::Ssh => Some(Box::new(crate::modes::ssh::Ssh)),
            #[cfg(not(unix))]
            CliMode::Ssh => None,
        };
        let entries = match m {
            Some(mut m) => m
                .collect()
                .unwrap_or_default()
                .into_iter()
                .map(Arc::new)
                .collect(),
            None => Vec::new(),
        };
        self.set_entries(entries);
        self.picker.query.clear();
        self.picker.query_cursor = 0;
        self.picker.selected = 0;
        self.picker.reset_after_mode_switch();
        self.rerank();
    }
}

pub(crate) fn rerank_if_query_changed(
    previous: Option<&str>,
    current: &str,
    rerank: impl FnOnce(),
) -> bool {
    if previous == Some(current) {
        return false;
    }
    rerank();
    true
}

/// Selection after moving down by `n` rows, clamped to the last row.
/// `n` can be `usize::MAX` (a saturated count prefix); the addition must
/// saturate, not wrap — wrapping lands on `cur - 1` and moves the
/// selection up.
pub(crate) fn move_down_selection(cur: usize, n: usize, total: usize) -> usize {
    cur.saturating_add(n).min(total.saturating_sub(1))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::cell::Cell;

    /// An `AppState` with in-memory usage/history and no disk state, for
    /// driving `rerank` without a window.
    pub(crate) fn app_state(cli_mode: CliMode, labels: &[&str], password: bool) -> AppState {
        let entries: Vec<Arc<Entry>> = labels.iter().map(|l| Arc::new(Entry::stdout(*l))).collect();
        AppState {
            picker: crate::picker::state::PickerState::new(),
            usage_keys: crate::picker::frecency::entry_keys(&entries),
            match_keys: crate::picker::matcher::match_keys(&entries),
            entries,
            matches: Vec::new(),
            g_pending: false,
            cli_mode,
            prompt: String::new(),
            max_results: 256,
            matcher: crate::picker::matcher::Matcher::new(),
            password,
            usage: Default::default(),
            history: Default::default(),
            calc_results: Default::default(),
            kb_custom: Vec::new(),
        }
    }

    fn ranked_with_query(mut state: AppState, query: &str) -> AppState {
        state.picker.query = query.to_owned();
        state.rerank();
        state
    }

    // ── -P / --password ───────────────────────────────────────────────────

    #[test]
    fn password_ranks_no_rows() {
        let labels = ["hunter2", "hunt", "other"];
        let plain = ranked_with_query(app_state(CliMode::Dmenu, &labels, false), "hun");
        assert!(!plain.matches.is_empty());
        let masked = ranked_with_query(app_state(CliMode::Dmenu, &labels, true), "hun");
        assert!(masked.matches.is_empty());

        // Calc builds its own rows from the query; -P must stop that too.
        let calc = ranked_with_query(app_state(CliMode::Calc, &[], true), "6*7");
        assert!(calc.matches.is_empty());
    }

    #[test]
    fn initial_query_reranks_once() {
        let calls = Cell::new(0);

        assert!(rerank_if_query_changed(None, "", || {
            calls.set(calls.get() + 1);
        }));
        assert!(!rerank_if_query_changed(Some(""), "", || {
            calls.set(calls.get() + 1);
        }));

        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn changed_query_reranks_once() {
        let calls = Cell::new(0);

        assert!(rerank_if_query_changed(Some("old"), "new", || {
            calls.set(calls.get() + 1);
        }));

        assert_eq!(calls.get(), 1);
    }

    // ── caret_would_move tests ────────────────────────────────────────────

    fn spec(s: &str) -> crate::picker::keyspec::KeySpec {
        s.parse().unwrap()
    }

    #[test]
    fn right_binding_fires_only_at_query_end() {
        let right = spec("Right");
        let insert = VimMode::Insert;
        assert!(!caret_would_move(&right, insert, 0, 0), "empty query");
        assert!(!caret_would_move(&right, insert, 3, 3), "caret at end");
        assert!(
            caret_would_move(&right, insert, 1, 3),
            "caret mid-query moves instead"
        );
        assert!(caret_would_move(&spec("End"), insert, 0, 2));
    }

    #[test]
    fn left_binding_fires_only_at_query_start() {
        let insert = VimMode::Insert;
        assert!(!caret_would_move(&spec("Left"), insert, 0, 3));
        assert!(caret_would_move(&spec("Left"), insert, 2, 3));
        assert!(caret_would_move(&spec("Home"), insert, 1, 3));
    }

    #[test]
    fn other_bindings_ignore_the_caret() {
        assert!(!caret_would_move(
            &spec("Shift+Delete"),
            VimMode::Insert,
            1,
            3
        ));
    }

    #[test]
    fn normal_mode_home_end_move_the_list_not_the_caret() {
        let normal = VimMode::Normal;
        assert!(!caret_would_move(&spec("Home"), normal, 1, 3));
        assert!(!caret_would_move(&spec("End"), normal, 1, 3));
        // h/l and Left/Right still move the caret in Normal mode.
        assert!(caret_would_move(&spec("Right"), normal, 1, 3));
        assert!(caret_would_move(&spec("Left"), normal, 1, 3));
        // Visual mode has no caret motions at all.
        assert!(!caret_would_move(&spec("Right"), VimMode::Visual, 1, 3));
    }

    // ── kb_custom_action tests ────────────────────────────────────────────

    fn bindings(specs: &[&str]) -> Vec<crate::picker::keyspec::KbCustom> {
        specs.iter().map(|s| s.parse().unwrap()).collect()
    }

    fn named(k: NamedKey) -> Key {
        Key::Named(k)
    }

    #[test]
    fn kb_custom_picks_binding_and_confirm_variant() {
        let b = bindings(&["F2", "Shift+Delete=Sure?"]);
        let none = Modifiers::default();
        let act = |key: &Key, mods| {
            kb_custom_action(&b, CliMode::Dmenu, VimMode::Insert, key, mods, (0, 0))
        };
        assert_eq!(
            act(&named(NamedKey::F2), none),
            Some(Action::AcceptKbCustom(0))
        );
        assert_eq!(
            act(
                &named(NamedKey::Delete),
                Modifiers {
                    shift: true,
                    ..Default::default()
                }
            ),
            Some(Action::ConfirmKbCustom(1))
        );
        assert_eq!(act(&named(NamedKey::Delete), none), None);
    }

    #[test]
    fn kb_custom_applies_only_in_dmenu_mode() {
        let b = bindings(&["F2"]);
        let f2 = named(NamedKey::F2);
        let none = Modifiers::default();
        for mode in [CliMode::Drun, CliMode::Calc, CliMode::Emoji] {
            assert_eq!(
                kb_custom_action(&b, mode, VimMode::Insert, &f2, none, (0, 0)),
                None,
                "{mode:?}"
            );
        }
    }

    #[test]
    fn kb_custom_caret_guard_is_insert_home_end_only() {
        let b = bindings(&["Home"]);
        let home = named(NamedKey::Home);
        let none = Modifiers::default();
        // Mid-query in Insert, Home moves the caret: the binding yields.
        assert_eq!(
            kb_custom_action(&b, CliMode::Dmenu, VimMode::Insert, &home, none, (1, 3)),
            None
        );
        // In Normal, Home jumps the list, so the binding fires mid-query.
        assert_eq!(
            kb_custom_action(&b, CliMode::Dmenu, VimMode::Normal, &home, none, (1, 3)),
            Some(Action::AcceptKbCustom(0))
        );
    }

    #[test]
    fn kb_custom_modified_caret_key_always_fires() {
        let b = bindings(&["Alt+Right"]);
        let right = named(NamedKey::ArrowRight);
        assert_eq!(
            kb_custom_action(
                &b,
                CliMode::Dmenu,
                VimMode::Insert,
                &right,
                Modifiers {
                    alt: true,
                    ..Default::default()
                },
                (1, 3)
            ),
            Some(Action::AcceptKbCustom(0))
        );
    }

    // ── needs_loaded_rows / selection_range tests ─────────────────────────

    #[test]
    fn accepts_wait_for_loaded_rows() {
        for action in [
            Action::Accept,
            Action::AcceptCustom,
            Action::AcceptKbCustom(0),
            Action::ConfirmKbCustom(0),
        ] {
            assert!(needs_loaded_rows(&action), "{action:?}");
        }
        for action in [
            Action::InsertChar('a'),
            Action::Backspace,
            Action::MoveDown(1),
            Action::Cancel,
        ] {
            assert!(!needs_loaded_rows(&action), "{action:?}");
        }
    }

    #[test]
    fn selection_range_visual_spans_anchor_to_cursor() {
        assert_eq!(selection_range(VimMode::Visual, Some(1), 3), [1, 2, 3]);
        assert_eq!(selection_range(VimMode::Visual, Some(3), 1), [1, 2, 3]);
        assert_eq!(selection_range(VimMode::Visual, Some(2), 2), [2]);
    }

    #[test]
    fn selection_range_outside_visual_is_the_cursor_row() {
        assert_eq!(selection_range(VimMode::Visual, None, 4), [4]);
        assert_eq!(selection_range(VimMode::Normal, Some(1), 4), [4]);
        assert_eq!(selection_range(VimMode::Insert, None, 0), [0]);
    }

    #[test]
    fn move_down_saturates_on_max_count() {
        // n == usize::MAX (a saturated count prefix) must clamp to the last
        // row, not overflow: `cur + n` panics in debug and wraps to `cur - 1`
        // in release (moving the selection UP).
        assert_eq!(move_down_selection(1, usize::MAX, 10), 9);
        assert_eq!(move_down_selection(0, usize::MAX, 10), 9);
        assert_eq!(move_down_selection(5, usize::MAX, 3), 2);
        assert_eq!(move_down_selection(0, 2, 5), 2);
        assert_eq!(move_down_selection(4, 2, 5), 4);
        assert_eq!(move_down_selection(0, 2, 0), 0);
    }
}
