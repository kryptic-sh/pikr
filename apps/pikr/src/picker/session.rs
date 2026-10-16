//! One picker run: key presses and clicks in, what to draw and what to do
//! on exit out. No window, no toolkit: the frontend feeds events in and
//! draws whatever state this leaves behind.

use std::sync::Arc;

use crate::cli::Mode as CliMode;
use crate::modes::{Entry, Payload};
use crate::picker::app_state::{
    AppState, ConfirmCard, kb_custom_action, move_down_selection, needs_loaded_rows,
    rerank_if_query_changed, selection_range,
};
use crate::picker::input::{Key, Modifiers, NamedKey};
use crate::picker::keys::{Action, key_to_action};
use crate::picker::query_text::{char_idx_to_byte, word_boundary_back};
use crate::picker::state::VimMode;

/// Rows a PageUp / PageDown moves the selection by.
const PAGE_ROWS: usize = 10;

/// What the frontend does after an event.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    /// Keep running; redraw from the session's state.
    Continue,
    /// Save state with [`Session::save`], run `payloads` in order, then exit
    /// the process with `code`.
    Exit { payloads: Vec<Payload>, code: i32 },
}

impl Outcome {
    fn exit(payloads: Vec<Payload>, code: i32) -> Self {
        Self::Exit { payloads, code }
    }
}

pub struct Session {
    pub app: AppState,
    /// A pending `--kb-custom KEY=PROMPT` confirmation.
    pub confirm: Option<ConfirmCard>,
    /// `--loading` text, shown in place of the list until the rows arrive.
    pub loading: Option<String>,
    /// Index into the mode's query history while Ctrl-P / Ctrl-N recall is
    /// active, `None` while typing live.
    history_cursor: Option<usize>,
    /// The live query, stashed on the first Ctrl-P so Ctrl-N can restore it.
    history_draft: String,
    /// The query the match list was last ranked for.
    ranked_query: Option<String>,
    usage_dirty: bool,
    history_dirty: bool,
}

impl Session {
    /// Starts a session and ranks the initial query. `loading` is the
    /// `--loading` text when rows are still on their way.
    pub fn new(app: AppState, loading: Option<String>) -> Self {
        let mut session = Self {
            app,
            confirm: None,
            loading,
            history_cursor: None,
            history_draft: String::new(),
            ranked_query: None,
            usage_dirty: false,
            history_dirty: false,
        };
        session.rerank_if_query_changed();
        session
    }

    /// Swaps in the `--loading` rows once stdin has closed.
    pub fn entries_arrived(&mut self, entries: &[Entry]) {
        self.app.entries = entries.iter().cloned().map(Arc::new).collect();
        self.app.usage_keys = crate::picker::frecency::entry_keys(&self.app.entries);
        self.app.rerank();
        self.loading = None;
    }

    /// Writes frecency and history if this session changed them. Called by
    /// the frontend before it acts on [`Outcome::Exit`]; never by tests, so
    /// they don't touch the user's state files.
    pub fn save(&self) {
        if self.usage_dirty {
            self.app.usage.save();
        }
        if self.history_dirty {
            self.app.history.save();
        }
    }

    fn rerank_if_query_changed(&mut self) {
        let current = self.app.picker.query.clone();
        let app = &mut self.app;
        rerank_if_query_changed(self.ranked_query.as_deref(), &current, || app.rerank());
        self.ranked_query = Some(current);
    }

    /// Records an accept for frecency and query history, unless `-P` made
    /// the input sensitive.
    fn record_accept(&mut self, payloads: &[Payload], query: &str) {
        if self.app.password || payloads.is_empty() {
            return;
        }
        let cli_mode = self.app.cli_mode;
        for payload in payloads {
            self.app.usage.record(cli_mode, payload);
        }
        self.usage_dirty = true;
        self.push_history(query);
    }

    fn push_history(&mut self, query: &str) {
        if self.app.password || query.trim().is_empty() {
            return;
        }
        self.app.history.push(self.app.cli_mode, query);
        self.history_dirty = true;
    }

    /// The payloads of the rows an accept acts on.
    fn selected_payloads(&self) -> Vec<Payload> {
        let picker = &self.app.picker;
        selection_range(picker.vim_mode, picker.visual_anchor, picker.selected)
            .into_iter()
            .filter_map(|mi| self.app.matches.get(mi))
            .map(|m| self.app.entries[m.index].payload.clone())
            .collect()
    }

    /// The typed query as a dmenu answer.
    fn query_payload(&self) -> Payload {
        Payload::Stdout(self.app.picker.query.trim().to_string())
    }

    /// A click on result row `mi`: accept it, as Enter does on the cursor
    /// row. While a confirm card is open the click only dismisses it.
    pub fn click_row(&mut self, mi: usize) -> Outcome {
        if self.confirm.take().is_some() {
            return Outcome::Continue;
        }
        let Some(m) = self.app.matches.get(mi) else {
            return Outcome::Continue;
        };
        let payload = self.app.entries[m.index].payload.clone();
        self.app.picker.selected = mi;
        let query = self.app.picker.query.clone();
        self.record_accept(std::slice::from_ref(&payload), &query);
        Outcome::exit(vec![payload], 0)
    }

    pub fn handle_key(&mut self, key: &Key, mods: Modifiers) -> Outcome {
        let outcome = self.dispatch_key(key, mods);
        self.rerank_if_query_changed();
        outcome
    }

    fn dispatch_key(&mut self, key: &Key, mods: Modifiers) -> Outcome {
        // A confirm card owns the keyboard: Enter accepts its binding, Esc or
        // Left dismisses, everything else is swallowed so the selection and
        // query can't change under the card.
        if let Some(card) = &self.confirm {
            return match key {
                Key::Named(NamedKey::Enter) => self.apply(Action::AcceptKbCustom(card.binding)),
                Key::Named(NamedKey::Escape | NamedKey::ArrowLeft) => {
                    self.confirm = None;
                    Outcome::Continue
                }
                _ => Outcome::Continue,
            };
        }

        if let Some(buf) = self.app.picker.ex_buf.take() {
            return self.ex_key(buf, key);
        }

        let picker = &self.app.picker;
        let vim_mode = picker.vim_mode;
        let caret = (picker.query_cursor, picker.query.chars().count());
        let is_g = matches!(key, Key::Character(c) if c.as_str() == "g");

        // `gg` two-key motion.
        if vim_mode == VimMode::Normal && is_g {
            if std::mem::take(&mut self.app.g_pending) {
                self.app.picker.count = None;
                self.app.picker.selected = 0;
            } else {
                self.app.g_pending = true;
            }
            return Outcome::Continue;
        }
        self.app.g_pending = false;

        let action = kb_custom_action(
            &self.app.kb_custom,
            self.app.cli_mode,
            vim_mode,
            key,
            mods,
            caret,
        )
        .or_else(|| key_to_action(&mut self.app.picker, key, mods.ctrl, mods.shift));
        let Some(action) = action else {
            return Outcome::Continue;
        };
        // Rows haven't arrived yet (--loading): nothing to accept.
        if self.loading.is_some() && needs_loaded_rows(&action) {
            return Outcome::Continue;
        }
        self.apply(action)
    }

    /// A key while the `:` prompt is open; `buf` is its text, already taken
    /// out of the state and put back unless the key closes the prompt.
    fn ex_key(&mut self, mut buf: String, key: &Key) -> Outcome {
        match key {
            Key::Named(NamedKey::Escape) => {}
            Key::Named(NamedKey::Enter) => {
                let mode = match buf.trim() {
                    "calc" => CliMode::Calc,
                    "clipboard" => CliMode::Clipboard,
                    "dmenu" => CliMode::Dmenu,
                    "drun" => CliMode::Drun,
                    "emoji" => CliMode::Emoji,
                    "run" => CliMode::Run,
                    "ssh" => CliMode::Ssh,
                    "q" | "q!" => return Outcome::exit(Vec::new(), 0),
                    _ => return Outcome::Continue,
                };
                self.app.switch_mode(mode);
                // The new mode starts with a clean recall state.
                self.history_cursor = None;
                self.history_draft.clear();
                self.ranked_query = Some(self.app.picker.query.clone());
            }
            // Backspace on an empty prompt dismisses it, as in readline and vim.
            Key::Named(NamedKey::Backspace) if buf.is_empty() => {}
            Key::Named(NamedKey::Backspace) => {
                buf.pop();
                self.app.picker.ex_buf = Some(buf);
            }
            Key::Character(ch) => {
                buf.push_str(ch);
                self.app.picker.ex_buf = Some(buf);
            }
            _ => self.app.picker.ex_buf = Some(buf),
        }
        Outcome::Continue
    }

    fn apply(&mut self, action: Action) -> Outcome {
        let total = self.app.matches.len();
        let picker = &mut self.app.picker;
        match action {
            Action::MoveDown(n) => picker.selected = move_down_selection(picker.selected, n, total),
            Action::MoveUp(n) => picker.selected = picker.selected.saturating_sub(n),
            Action::PageDown => {
                picker.selected = (picker.selected + PAGE_ROWS).min(total.saturating_sub(1));
            }
            Action::PageUp => picker.selected = picker.selected.saturating_sub(PAGE_ROWS),
            Action::Top => picker.selected = 0,
            Action::Bottom => picker.selected = total.saturating_sub(1),
            Action::EnterInsert => {
                picker.visual_anchor = None;
                picker.vim_mode = VimMode::Insert;
            }
            Action::AppendAfter => {
                // vim `a`: caret one cell right (clamped), then Insert.
                let len = picker.query.chars().count();
                picker.query_cursor = (picker.query_cursor + 1).min(len);
                picker.visual_anchor = None;
                picker.vim_mode = VimMode::Insert;
            }
            Action::AppendEnd => {
                picker.query_cursor = picker.query.chars().count();
                picker.visual_anchor = None;
                picker.vim_mode = VimMode::Insert;
            }
            Action::InsertStart => {
                picker.query_cursor = 0;
                picker.visual_anchor = None;
                picker.vim_mode = VimMode::Insert;
            }
            Action::EnterNormal => {
                picker.visual_anchor = None;
                picker.vim_mode = VimMode::Normal;
            }
            Action::EnterVisual => {
                picker.visual_anchor = Some(picker.selected);
                picker.vim_mode = VimMode::Visual;
            }
            Action::StartSearch => picker.vim_mode = VimMode::Insert,
            Action::StartEx => picker.ex_buf = Some(String::new()),
            Action::Accept => return self.accept(),
            Action::AcceptCustom => {
                let query = self.app.picker.query.clone();
                self.push_history(query.trim());
                return Outcome::exit(vec![self.query_payload()], 0);
            }
            Action::ConfirmKbCustom(index) => {
                // With no matching row the card sits in the empty-state row,
                // and confirming prints the typed query like an unprompted
                // binding does.
                if let Some(prompt) = self.app.kb_custom[index].confirm.clone() {
                    self.confirm = Some(ConfirmCard {
                        binding: index,
                        prompt,
                    });
                }
            }
            Action::AcceptKbCustom(index) => {
                // Hand the selection back for a script-defined action (forget,
                // copy, delete, …). No frecency or history bump: an entry the
                // script is about to remove shouldn't be promoted for next time.
                let mut payloads = self.selected_payloads();
                // No match: return the typed query, as Enter does in dmenu.
                if payloads.is_empty() {
                    payloads.push(self.query_payload());
                }
                return Outcome::exit(payloads, crate::cli::KB_CUSTOM_EXIT_BASE + index as i32);
            }
            Action::Cancel => return Outcome::exit(Vec::new(), 1),
            Action::InsertChar(c) => {
                // Any user-initiated edit exits history recall — the buffer
                // is no longer a verbatim past query.
                self.history_cursor = None;
                let byte_idx = char_idx_to_byte(&picker.query, picker.query_cursor);
                picker.query.insert(byte_idx, c);
                picker.query_cursor += 1;
            }
            Action::Backspace => {
                self.history_cursor = None;
                let cur = picker.query_cursor;
                if cur > 0 {
                    let start = char_idx_to_byte(&picker.query, cur - 1);
                    let end = char_idx_to_byte(&picker.query, cur);
                    picker.query.replace_range(start..end, "");
                    picker.query_cursor = cur - 1;
                }
            }
            Action::DeleteForward => {
                self.history_cursor = None;
                let cur = picker.query_cursor;
                if cur < picker.query.chars().count() {
                    let start = char_idx_to_byte(&picker.query, cur);
                    let end = char_idx_to_byte(&picker.query, cur + 1);
                    picker.query.replace_range(start..end, "");
                }
            }
            Action::CursorLeft => picker.query_cursor = picker.query_cursor.saturating_sub(1),
            Action::CursorRight => {
                if picker.query_cursor < picker.query.chars().count() {
                    picker.query_cursor += 1;
                }
            }
            Action::CursorHome => picker.query_cursor = 0,
            Action::CursorEnd => picker.query_cursor = picker.query.chars().count(),
            Action::DeleteWordBack => {
                self.history_cursor = None;
                let cur = picker.query_cursor;
                if cur > 0 {
                    let new_cur = word_boundary_back(&picker.query, cur);
                    let start = char_idx_to_byte(&picker.query, new_cur);
                    let end = char_idx_to_byte(&picker.query, cur);
                    picker.query.replace_range(start..end, "");
                    picker.query_cursor = new_cur;
                }
            }
            Action::DeleteToLineStart => {
                self.history_cursor = None;
                let end = char_idx_to_byte(&picker.query, picker.query_cursor);
                picker.query.replace_range(0..end, "");
                picker.query_cursor = 0;
            }
            Action::HistoryPrev => self.history_prev(),
            Action::HistoryNext => self.history_next(),
        }
        Outcome::Continue
    }

    fn accept(&mut self) -> Outcome {
        let payloads = self.selected_payloads();
        let query = self.app.picker.query.clone();
        if payloads.is_empty() {
            // In dmenu mode with no matches, Enter returns the typed query —
            // rofi parity (rofi --dmenu returns the unmatched input).
            if self.app.cli_mode == CliMode::Dmenu {
                self.push_history(query.trim());
                return Outcome::exit(vec![self.query_payload()], 0);
            }
            return Outcome::Continue;
        }
        self.record_accept(&payloads, &query);
        Outcome::exit(payloads, 0)
    }

    /// Ctrl-P: step back through this mode's query history, stashing the
    /// live draft on the first step.
    fn history_prev(&mut self) {
        let cli_mode = self.app.cli_mode;
        let total = self.app.history.len(cli_mode);
        if total == 0 {
            return;
        }
        let next_idx = match self.history_cursor {
            None => {
                self.history_draft = self.app.picker.query.clone();
                0
            }
            Some(cur) => (cur + 1).min(total - 1),
        };
        if let Some(text) = self.app.history.get(cli_mode, next_idx).map(str::to_owned) {
            self.history_cursor = Some(next_idx);
            self.set_query(text);
        }
    }

    /// Ctrl-N: step forward through history, back to the live draft after the
    /// most recent entry. Does nothing unless recall is active.
    fn history_next(&mut self) {
        match self.history_cursor {
            None => {}
            Some(0) => {
                self.history_cursor = None;
                let draft = std::mem::take(&mut self.history_draft);
                self.set_query(draft);
            }
            Some(cur) => {
                let cli_mode = self.app.cli_mode;
                if let Some(text) = self.app.history.get(cli_mode, cur - 1).map(str::to_owned) {
                    self.history_cursor = Some(cur - 1);
                    self.set_query(text);
                }
            }
        }
    }

    /// Replaces the query and puts the caret at its end.
    fn set_query(&mut self, text: String) {
        self.app.picker.query_cursor = text.chars().count();
        self.app.picker.query = text;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::picker::app_state::tests::app_state;

    fn ch(c: &str) -> Key {
        Key::Character(c.into())
    }

    fn named(k: NamedKey) -> Key {
        Key::Named(k)
    }

    fn none() -> Modifiers {
        Modifiers::default()
    }

    fn session(cli_mode: CliMode, labels: &[&str]) -> Session {
        Session::new(app_state(cli_mode, labels, false), None)
    }

    fn type_text(s: &mut Session, text: &str) {
        for c in text.chars() {
            assert_eq!(s.handle_key(&ch(&c.to_string()), none()), Outcome::Continue);
        }
    }

    #[test]
    fn typing_reranks_the_matches() {
        let mut s = session(CliMode::Dmenu, &["alpha", "beta", "gamma"]);
        assert_eq!(s.app.matches.len(), 3);
        type_text(&mut s, "bet");
        assert_eq!(s.app.picker.query, "bet");
        assert_eq!(s.app.picker.query_cursor, 3);
        assert_eq!(s.app.matches.len(), 1);
        assert_eq!(s.app.entries[s.app.matches[0].index].label, "beta");
    }

    #[test]
    fn enter_accepts_the_selected_row_and_records_it() {
        let mut s = session(CliMode::Dmenu, &["alpha", "beta"]);
        s.handle_key(&named(NamedKey::ArrowDown), none());
        let out = s.handle_key(&named(NamedKey::Enter), none());
        assert_eq!(out, Outcome::exit(vec![Payload::Stdout("beta".into())], 0));
        assert!(s.usage_dirty, "the accept is recorded for frecency");
    }

    #[test]
    fn dmenu_enter_with_no_match_returns_the_query() {
        let mut s = session(CliMode::Dmenu, &["alpha"]);
        type_text(&mut s, "zzz");
        assert!(s.app.matches.is_empty());
        let out = s.handle_key(&named(NamedKey::Enter), none());
        assert_eq!(out, Outcome::exit(vec![Payload::Stdout("zzz".into())], 0));
        assert!(s.history_dirty);
    }

    #[test]
    fn password_accept_records_nothing() {
        let mut s = Session::new(app_state(CliMode::Dmenu, &["alpha"], true), None);
        type_text(&mut s, "al");
        assert!(matches!(
            s.handle_key(&named(NamedKey::Enter), none()),
            Outcome::Exit { code: 0, .. }
        ));
        assert!(!s.usage_dirty && !s.history_dirty);
    }

    #[test]
    fn escape_twice_cancels_with_exit_code_one() {
        let mut s = session(CliMode::Dmenu, &["alpha"]);
        assert_eq!(
            s.handle_key(&named(NamedKey::Escape), none()),
            Outcome::Continue
        );
        assert_eq!(s.app.picker.vim_mode, VimMode::Normal);
        assert_eq!(
            s.handle_key(&named(NamedKey::Escape), none()),
            Outcome::exit(Vec::new(), 1)
        );
    }

    #[test]
    fn gg_jumps_to_the_top() {
        let mut s = session(CliMode::Dmenu, &["a", "b", "c"]);
        s.handle_key(&named(NamedKey::Escape), none());
        s.handle_key(&ch("G"), none());
        assert_eq!(s.app.picker.selected, 2);
        s.handle_key(&ch("g"), none());
        assert_eq!(s.app.picker.selected, 2, "one g only arms the motion");
        s.handle_key(&ch("g"), none());
        assert_eq!(s.app.picker.selected, 0);
    }

    #[test]
    fn ex_prompt_edits_and_quits() {
        let mut s = session(CliMode::Dmenu, &["a"]);
        s.handle_key(&named(NamedKey::Escape), none());
        s.handle_key(&ch(":"), none());
        assert_eq!(s.app.picker.ex_buf.as_deref(), Some(""));
        type_text(&mut s, "qx");
        s.handle_key(&named(NamedKey::Backspace), none());
        assert_eq!(s.app.picker.ex_buf.as_deref(), Some("q"));
        assert_eq!(
            s.handle_key(&named(NamedKey::Enter), none()),
            Outcome::exit(Vec::new(), 0)
        );
    }

    #[test]
    fn ex_backspace_on_empty_prompt_closes_it() {
        let mut s = session(CliMode::Dmenu, &["a"]);
        s.handle_key(&named(NamedKey::Escape), none());
        s.handle_key(&ch(":"), none());
        s.handle_key(&named(NamedKey::Backspace), none());
        assert_eq!(s.app.picker.ex_buf, None);
    }

    #[test]
    fn confirm_card_swallows_keys_until_enter() {
        let mut app = app_state(CliMode::Dmenu, &["alpha", "beta"], false);
        app.kb_custom = vec!["F2=Sure?".parse().unwrap()];
        let mut s = Session::new(app, None);
        s.handle_key(&named(NamedKey::F2), none());
        assert!(s.confirm.is_some());
        s.handle_key(&ch("x"), none());
        assert_eq!(s.app.picker.query, "", "keys under the card are swallowed");
        assert_eq!(
            s.handle_key(&named(NamedKey::Enter), none()),
            Outcome::exit(
                vec![Payload::Stdout("alpha".into())],
                crate::cli::KB_CUSTOM_EXIT_BASE
            )
        );
    }

    #[test]
    fn loading_rows_block_accept() {
        let mut s = Session::new(
            app_state(CliMode::Dmenu, &[], false),
            Some("loading".into()),
        );
        type_text(&mut s, "x");
        assert_eq!(
            s.handle_key(&named(NamedKey::Enter), none()),
            Outcome::Continue
        );
        s.entries_arrived(&[Entry::stdout("x-ray")]);
        assert_eq!(s.loading, None);
        assert_eq!(s.app.matches.len(), 1);
    }

    #[test]
    fn history_recall_walks_back_and_restores_the_draft() {
        let mut app = app_state(CliMode::Dmenu, &[], false);
        app.history.push(CliMode::Dmenu, "older");
        app.history.push(CliMode::Dmenu, "newer");
        let mut s = Session::new(app, None);
        type_text(&mut s, "dr");
        let ctrl = Modifiers {
            ctrl: true,
            ..Default::default()
        };
        s.handle_key(&ch("p"), ctrl);
        assert_eq!(s.app.picker.query, "newer");
        s.handle_key(&ch("p"), ctrl);
        assert_eq!(s.app.picker.query, "older");
        s.handle_key(&ch("n"), ctrl);
        s.handle_key(&ch("n"), ctrl);
        assert_eq!(s.app.picker.query, "dr");
        assert_eq!(s.app.picker.query_cursor, 2);
    }

    #[test]
    fn click_accepts_the_row_but_first_dismisses_a_card() {
        let mut app = app_state(CliMode::Dmenu, &["alpha", "beta"], false);
        app.kb_custom = vec!["F2=Sure?".parse().unwrap()];
        let mut s = Session::new(app, None);
        s.handle_key(&named(NamedKey::F2), none());
        assert_eq!(s.click_row(1), Outcome::Continue);
        assert!(s.confirm.is_none());
        assert_eq!(
            s.click_row(1),
            Outcome::exit(vec![Payload::Stdout("beta".into())], 0)
        );
    }
}
