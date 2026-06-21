//! Vim keymap → picker action + dispatch.

use floem::reactive::{SignalGet, SignalUpdate};
use floem::ui_events::keyboard::{Key, NamedKey};

use super::state::{PickerState, VimMode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    MoveDown(usize),
    MoveUp(usize),
    PageDown,
    PageUp,
    Top,
    Bottom,
    EnterInsert,
    /// `a` — move the caret one cell right, then enter Insert (vim append).
    AppendAfter,
    /// `A` — move the caret to end of the query, then enter Insert.
    AppendEnd,
    /// `I` — move the caret to the start of the query, then enter Insert.
    InsertStart,
    EnterNormal,
    EnterVisual,
    StartSearch,
    StartEx,
    Accept,
    AcceptCustom,
    Cancel,
    InsertChar(char),
    Backspace,
    // Query-bar caret editing (Insert mode).
    CursorLeft,
    CursorRight,
    CursorHome,
    CursorEnd,
    /// Delete the char to the right of the caret (Delete key).
    DeleteForward,
    /// Delete the word to the left of the caret (Ctrl-W, readline).
    DeleteWordBack,
    /// Delete from the caret to the start of the query (Ctrl-U, readline).
    DeleteToLineStart,
    /// Recall the previous query from history (Ctrl-P, fzf convention).
    HistoryPrev,
    /// Walk forward through history toward the live draft (Ctrl-N).
    HistoryNext,
}

/// Translate a floem key event into an `Action` given the current vim mode.
///
/// Returns `None` for unhandled keys. Count prefix digits in Normal mode are
/// accumulated on `state` before a motion is returned.
pub fn key_to_action(state: &PickerState, key: &Key, ctrl: bool, shift: bool) -> Option<Action> {
    let vim_mode = state.vim_mode.get();

    match vim_mode {
        // Visual mode shares the Normal motion vocabulary (j/k/G/gg/arrows/
        // pgup/pgdn/Home/End) — `v` extends the range instead of starting a
        // new one. Escape collapses back to Normal.
        VimMode::Visual => match key {
            Key::Named(NamedKey::Escape) => {
                state.count.set(None);
                Some(Action::EnterNormal)
            }
            // `v` / `V` / `<C-v>` in Visual collapses back to Normal (vim
            // semantics: same trigger toggles the mode off).
            Key::Character(s) if s.as_str() == "v" || s.as_str() == "V" => {
                Some(Action::EnterNormal)
            }
            Key::Character(s) if s.as_str() == "v" && ctrl => Some(Action::EnterNormal),
            _ => normal_or_visual_key(state, key, ctrl),
        },

        VimMode::Insert => match key {
            Key::Named(NamedKey::Escape) => Some(Action::EnterNormal),
            Key::Named(NamedKey::Enter) => {
                if shift {
                    Some(Action::AcceptCustom)
                } else {
                    Some(Action::Accept)
                }
            }
            Key::Named(NamedKey::Backspace) => Some(Action::Backspace),
            Key::Named(NamedKey::Delete) => Some(Action::DeleteForward),
            // Space arrives as Key::Character(" ") per keyboard-types spec.
            // Handled by the Key::Character arm below (InsertChar(' ')).
            // Caret navigation inside the query text.
            Key::Named(NamedKey::ArrowLeft) => Some(Action::CursorLeft),
            Key::Named(NamedKey::ArrowRight) => Some(Action::CursorRight),
            // Home / End in Insert: caret to start / end of query. The list
            // top / bottom motions live on Normal-mode keymap.
            Key::Named(NamedKey::Home) => Some(Action::CursorHome),
            Key::Named(NamedKey::End) => Some(Action::CursorEnd),
            // Arrow-up / down still steer the result list so the user can
            // type and pick without bouncing to Normal.
            Key::Named(NamedKey::ArrowDown) => Some(Action::MoveDown(1)),
            Key::Named(NamedKey::ArrowUp) => Some(Action::MoveUp(1)),
            Key::Named(NamedKey::PageDown) => Some(Action::PageDown),
            Key::Named(NamedKey::PageUp) => Some(Action::PageUp),
            Key::Character(s) => {
                let c = s.chars().next()?;
                // Readline-style Ctrl-shortcuts on the query bar.
                if ctrl {
                    return match c {
                        'a' => Some(Action::CursorHome),
                        'e' => Some(Action::CursorEnd),
                        'b' => Some(Action::CursorLeft),
                        'f' => Some(Action::CursorRight),
                        'w' => Some(Action::DeleteWordBack),
                        'u' => Some(Action::DeleteToLineStart),
                        'd' => Some(Action::DeleteForward),
                        'p' => Some(Action::HistoryPrev),
                        'n' => Some(Action::HistoryNext),
                        _ => None,
                    };
                }
                Some(Action::InsertChar(c))
            }
            _ => None,
        },

        VimMode::Normal => match key {
            Key::Named(NamedKey::Escape) => {
                state.count.set(None);
                Some(Action::Cancel)
            }
            // Query-caret movement in Normal mode (vim `h`/`l` + arrows). `j`/`k`
            // and Up/Down/Home/End stay list navigation in `normal_or_visual_key`.
            Key::Named(NamedKey::ArrowLeft) => {
                state.count.set(None);
                Some(Action::CursorLeft)
            }
            Key::Named(NamedKey::ArrowRight) => {
                state.count.set(None);
                Some(Action::CursorRight)
            }
            Key::Character(s) if s.as_str() == "h" => {
                state.count.set(None);
                Some(Action::CursorLeft)
            }
            Key::Character(s) if s.as_str() == "l" => {
                state.count.set(None);
                Some(Action::CursorRight)
            }
            // Insert-entry motions (vim): `a` append, `A` append-end, `I`
            // insert-start. `i` (insert before cursor) lives in the shared
            // motion table below.
            Key::Character(s) if s.as_str() == "a" => {
                state.count.set(None);
                Some(Action::AppendAfter)
            }
            Key::Character(s) if s.as_str() == "A" => {
                state.count.set(None);
                Some(Action::AppendEnd)
            }
            Key::Character(s) if s.as_str() == "I" => {
                state.count.set(None);
                Some(Action::InsertStart)
            }
            _ => normal_or_visual_key(state, key, ctrl),
        },
    }
}

/// Shared keymap for Normal and Visual modes. Both interpret the same motion
/// vocabulary (j/k/G/gg/arrows/count prefix); the mode-specific arms
/// (Escape, `v`-to-start, `v`-to-end) are handled in `key_to_action`.
fn normal_or_visual_key(state: &PickerState, key: &Key, ctrl: bool) -> Option<Action> {
    match key {
        Key::Named(NamedKey::Enter) => Some(Action::Accept),
        Key::Named(NamedKey::ArrowDown) => Some(Action::MoveDown(state.take_count())),
        Key::Named(NamedKey::ArrowUp) => Some(Action::MoveUp(state.take_count())),
        Key::Named(NamedKey::PageDown) => {
            state.count.set(None);
            Some(Action::PageDown)
        }
        Key::Named(NamedKey::PageUp) => {
            state.count.set(None);
            Some(Action::PageUp)
        }
        Key::Named(NamedKey::Home) => {
            state.count.set(None);
            Some(Action::Top)
        }
        Key::Named(NamedKey::End) => {
            state.count.set(None);
            Some(Action::Bottom)
        }
        Key::Character(s) => {
            let c = s.chars().next()?;

            if c.is_ascii_digit() && c != '0' {
                state.push_count_digit(c.to_digit(10).unwrap());
                return None;
            }
            if c == '0' && state.count.get().is_none() {
                return None;
            }
            if c == '0' {
                state.push_count_digit(0);
                return None;
            }

            match c {
                'j' => {
                    let n = state.take_count();
                    Some(Action::MoveDown(n))
                }
                'k' => {
                    let n = state.take_count();
                    Some(Action::MoveUp(n))
                }
                'G' => {
                    state.count.set(None);
                    Some(Action::Bottom)
                }
                'g' => None,
                'i' => {
                    state.count.set(None);
                    Some(Action::EnterInsert)
                }
                // `v` / `V` / `<C-v>` start Visual from Normal (and toggle
                // off when already in Visual — handled at the call site).
                'v' | 'V' => {
                    state.count.set(None);
                    Some(Action::EnterVisual)
                }
                '/' => {
                    state.count.set(None);
                    Some(Action::StartSearch)
                }
                ':' => {
                    state.count.set(None);
                    Some(Action::StartEx)
                }
                'd' if ctrl => {
                    state.count.set(None);
                    Some(Action::PageDown)
                }
                'u' if ctrl => {
                    state.count.set(None);
                    Some(Action::PageUp)
                }
                _ => {
                    state.count.set(None);
                    None
                }
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state() -> PickerState {
        let s = PickerState::new();
        // `PickerState::new()` defaults to Insert (so the user can type on
        // launch). The Normal-mode keymap tests below expect Normal — reset
        // here so they keep exercising the same dispatch path.
        s.vim_mode.set(VimMode::Normal);
        s
    }

    #[test]
    fn insert_mode_char() {
        let s = make_state();
        s.vim_mode.set(VimMode::Insert);
        let a = key_to_action(&s, &Key::Character("a".into()), false, false);
        assert_eq!(a, Some(Action::InsertChar('a')));
    }

    #[test]
    fn insert_left_right_arrow_move_caret() {
        let s = make_state();
        s.vim_mode.set(VimMode::Insert);
        let left = key_to_action(&s, &Key::Named(NamedKey::ArrowLeft), false, false);
        assert_eq!(left, Some(Action::CursorLeft));
        let right = key_to_action(&s, &Key::Named(NamedKey::ArrowRight), false, false);
        assert_eq!(right, Some(Action::CursorRight));
    }

    #[test]
    fn insert_home_end_caret() {
        let s = make_state();
        s.vim_mode.set(VimMode::Insert);
        let home = key_to_action(&s, &Key::Named(NamedKey::Home), false, false);
        assert_eq!(home, Some(Action::CursorHome));
        let end = key_to_action(&s, &Key::Named(NamedKey::End), false, false);
        assert_eq!(end, Some(Action::CursorEnd));
    }

    #[test]
    fn insert_ctrl_a_e_caret() {
        let s = make_state();
        s.vim_mode.set(VimMode::Insert);
        let ca = key_to_action(&s, &Key::Character("a".into()), true, false);
        assert_eq!(ca, Some(Action::CursorHome));
        let ce = key_to_action(&s, &Key::Character("e".into()), true, false);
        assert_eq!(ce, Some(Action::CursorEnd));
    }

    #[test]
    fn insert_ctrl_w_deletes_word() {
        let s = make_state();
        s.vim_mode.set(VimMode::Insert);
        let cw = key_to_action(&s, &Key::Character("w".into()), true, false);
        assert_eq!(cw, Some(Action::DeleteWordBack));
    }

    #[test]
    fn insert_ctrl_u_deletes_to_line_start() {
        let s = make_state();
        s.vim_mode.set(VimMode::Insert);
        let cu = key_to_action(&s, &Key::Character("u".into()), true, false);
        assert_eq!(cu, Some(Action::DeleteToLineStart));
    }

    #[test]
    fn insert_ctrl_p_recalls_history_prev() {
        let s = make_state();
        s.vim_mode.set(VimMode::Insert);
        let cp = key_to_action(&s, &Key::Character("p".into()), true, false);
        assert_eq!(cp, Some(Action::HistoryPrev));
    }

    #[test]
    fn insert_ctrl_n_recalls_history_next() {
        let s = make_state();
        s.vim_mode.set(VimMode::Insert);
        let cn = key_to_action(&s, &Key::Character("n".into()), true, false);
        assert_eq!(cn, Some(Action::HistoryNext));
    }

    #[test]
    fn insert_delete_key_forward() {
        let s = make_state();
        s.vim_mode.set(VimMode::Insert);
        let d = key_to_action(&s, &Key::Named(NamedKey::Delete), false, false);
        assert_eq!(d, Some(Action::DeleteForward));
    }

    #[test]
    fn insert_mode_escape_exits() {
        let s = make_state();
        s.vim_mode.set(VimMode::Insert);
        let a = key_to_action(&s, &Key::Named(NamedKey::Escape), false, false);
        assert_eq!(a, Some(Action::EnterNormal));
    }

    #[test]
    fn normal_j_k() {
        let s = make_state();
        let down = key_to_action(&s, &Key::Character("j".into()), false, false);
        assert_eq!(down, Some(Action::MoveDown(1)));
        let up = key_to_action(&s, &Key::Character("k".into()), false, false);
        assert_eq!(up, Some(Action::MoveUp(1)));
    }

    #[test]
    fn count_prefix_5j() {
        let s = make_state();
        // Press '5' — accumulates count, no action.
        let a1 = key_to_action(&s, &Key::Character("5".into()), false, false);
        assert_eq!(a1, None);
        assert_eq!(s.count.get(), Some(5));
        // Press 'j' — consume count → MoveDown(5).
        let a2 = key_to_action(&s, &Key::Character("j".into()), false, false);
        assert_eq!(a2, Some(Action::MoveDown(5)));
        assert_eq!(s.count.get(), None);
    }

    #[test]
    fn normal_arrow_keys() {
        let s = make_state();
        let down = key_to_action(&s, &Key::Named(NamedKey::ArrowDown), false, false);
        assert_eq!(down, Some(Action::MoveDown(1)));
        let up = key_to_action(&s, &Key::Named(NamedKey::ArrowUp), false, false);
        assert_eq!(up, Some(Action::MoveUp(1)));
        let pgdn = key_to_action(&s, &Key::Named(NamedKey::PageDown), false, false);
        assert_eq!(pgdn, Some(Action::PageDown));
        let home = key_to_action(&s, &Key::Named(NamedKey::Home), false, false);
        assert_eq!(home, Some(Action::Top));
        let end = key_to_action(&s, &Key::Named(NamedKey::End), false, false);
        assert_eq!(end, Some(Action::Bottom));
    }

    #[test]
    fn insert_space_inserts_space() {
        let s = make_state();
        s.vim_mode.set(VimMode::Insert);
        // keyboard-types 0.8.3: Space is Key::Character(" "), not NamedKey
        let a = key_to_action(&s, &Key::Character(" ".into()), false, false);
        assert_eq!(a, Some(Action::InsertChar(' ')));
    }

    #[test]
    fn insert_arrow_keys() {
        let s = make_state();
        s.vim_mode.set(VimMode::Insert);
        let down = key_to_action(&s, &Key::Named(NamedKey::ArrowDown), false, false);
        assert_eq!(down, Some(Action::MoveDown(1)));
        let up = key_to_action(&s, &Key::Named(NamedKey::ArrowUp), false, false);
        assert_eq!(up, Some(Action::MoveUp(1)));
        let pgup = key_to_action(&s, &Key::Named(NamedKey::PageUp), false, false);
        assert_eq!(pgup, Some(Action::PageUp));
    }

    #[test]
    fn normal_g_is_bottom() {
        let s = make_state();
        let a = key_to_action(&s, &Key::Character("G".into()), false, false);
        assert_eq!(a, Some(Action::Bottom));
    }

    #[test]
    fn normal_enter_accepts() {
        let s = make_state();
        let a = key_to_action(&s, &Key::Named(NamedKey::Enter), false, false);
        assert_eq!(a, Some(Action::Accept));
    }

    #[test]
    fn normal_v_enters_visual() {
        let s = make_state();
        let a = key_to_action(&s, &Key::Character("v".into()), false, false);
        assert_eq!(a, Some(Action::EnterVisual));
    }

    #[test]
    fn normal_caps_v_enters_visual() {
        let s = make_state();
        let a = key_to_action(&s, &Key::Character("V".into()), false, false);
        assert_eq!(a, Some(Action::EnterVisual));
    }

    #[test]
    fn visual_jk_moves_cursor() {
        let s = make_state();
        s.vim_mode.set(VimMode::Visual);
        let down = key_to_action(&s, &Key::Character("j".into()), false, false);
        assert_eq!(down, Some(Action::MoveDown(1)));
        let up = key_to_action(&s, &Key::Character("k".into()), false, false);
        assert_eq!(up, Some(Action::MoveUp(1)));
    }

    #[test]
    fn visual_v_toggles_back_to_normal() {
        let s = make_state();
        s.vim_mode.set(VimMode::Visual);
        let a = key_to_action(&s, &Key::Character("v".into()), false, false);
        assert_eq!(a, Some(Action::EnterNormal));
    }

    #[test]
    fn visual_escape_returns_to_normal() {
        let s = make_state();
        s.vim_mode.set(VimMode::Visual);
        let a = key_to_action(&s, &Key::Named(NamedKey::Escape), false, false);
        assert_eq!(a, Some(Action::EnterNormal));
    }

    // ── Normal-mode query-caret movement (#cursor-bugs) ──────────────────────
    // BUG: Normal mode bound no caret movement — h/l and Left/Right were
    // no-ops, so the cursor couldn't be moved over the query in Normal mode.

    #[test]
    fn normal_h_l_move_caret() {
        let s = make_state();
        let h = key_to_action(&s, &Key::Character("h".into()), false, false);
        assert_eq!(h, Some(Action::CursorLeft));
        let l = key_to_action(&s, &Key::Character("l".into()), false, false);
        assert_eq!(l, Some(Action::CursorRight));
    }

    #[test]
    fn normal_left_right_arrows_move_caret() {
        let s = make_state();
        let left = key_to_action(&s, &Key::Named(NamedKey::ArrowLeft), false, false);
        assert_eq!(left, Some(Action::CursorLeft));
        let right = key_to_action(&s, &Key::Named(NamedKey::ArrowRight), false, false);
        assert_eq!(right, Some(Action::CursorRight));
    }

    #[test]
    fn normal_a_appends() {
        let s = make_state();
        assert_eq!(
            key_to_action(&s, &Key::Character("a".into()), false, false),
            Some(Action::AppendAfter)
        );
    }

    #[test]
    fn normal_caps_a_appends_end() {
        let s = make_state();
        assert_eq!(
            key_to_action(&s, &Key::Character("A".into()), false, false),
            Some(Action::AppendEnd)
        );
    }

    #[test]
    fn normal_caps_i_inserts_start() {
        let s = make_state();
        assert_eq!(
            key_to_action(&s, &Key::Character("I".into()), false, false),
            Some(Action::InsertStart)
        );
    }

    #[test]
    fn normal_i_enters_insert() {
        let s = make_state();
        let a = key_to_action(&s, &Key::Character("i".into()), false, false);
        assert_eq!(a, Some(Action::EnterInsert));
    }

    #[test]
    fn insert_shift_enter_accept_custom() {
        let s = make_state();
        s.vim_mode.set(VimMode::Insert);
        let a = key_to_action(&s, &Key::Named(NamedKey::Enter), false, true);
        assert_eq!(a, Some(Action::AcceptCustom));
    }

    #[test]
    fn insert_plain_enter_still_accept() {
        let s = make_state();
        s.vim_mode.set(VimMode::Insert);
        let a = key_to_action(&s, &Key::Named(NamedKey::Enter), false, false);
        assert_eq!(a, Some(Action::Accept));
    }
}
