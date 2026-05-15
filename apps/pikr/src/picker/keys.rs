//! Vim keymap → picker action + dispatch.

use floem::keyboard::{Key, NamedKey};
use floem::reactive::{SignalGet, SignalUpdate};

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
    EnterNormal,
    StartSearch,
    StartEx,
    Accept,
    Cancel,
    InsertChar(char),
    Backspace,
}

/// Translate a floem key event into an `Action` given the current vim mode.
///
/// Returns `None` for unhandled keys. Count prefix digits in Normal mode are
/// accumulated on `state` before a motion is returned.
pub fn key_to_action(state: &PickerState, key: &Key, ctrl: bool) -> Option<Action> {
    let vim_mode = state.vim_mode.get();

    match vim_mode {
        VimMode::Insert => match key {
            Key::Named(NamedKey::Escape) => Some(Action::EnterNormal),
            Key::Named(NamedKey::Enter) => Some(Action::Accept),
            Key::Named(NamedKey::Backspace) => Some(Action::Backspace),
            Key::Character(s) => {
                let c = s.chars().next()?;
                Some(Action::InsertChar(c))
            }
            _ => None,
        },

        VimMode::Normal => {
            // Ex mode started by `:` — handled outside this function by the
            // view wiring, which intercepts the key before calling here.
            match key {
                Key::Named(NamedKey::Escape) => {
                    // Clear count on Escape.
                    state.count.set(None);
                    Some(Action::Cancel)
                }
                Key::Named(NamedKey::Enter) => Some(Action::Accept),
                Key::Character(s) => {
                    let c = s.chars().next()?;

                    // Count prefix: digits before a motion.
                    if c.is_ascii_digit() && c != '0' {
                        state.push_count_digit(c.to_digit(10).unwrap());
                        return None; // accumulating, no action yet
                    }
                    // '0' alone means Top (gg shorthand) only when count is None.
                    if c == '0' && state.count.get().is_none() {
                        // Vim `0` goes to line start; here treat as no-op
                        // (gg / G are top/bottom). Just clear.
                        return None;
                    }
                    // '0' as part of count prefix.
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
                        'g' => {
                            // Single `g` — the second `g` is handled on the
                            // next keystroke in the view layer via `g_pending`.
                            // Returning None here lets the view accumulate.
                            None
                        }
                        'i' => {
                            state.count.set(None);
                            Some(Action::EnterInsert)
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state() -> PickerState {
        PickerState::new()
    }

    #[test]
    fn insert_mode_char() {
        let s = make_state();
        s.vim_mode.set(VimMode::Insert);
        let a = key_to_action(&s, &Key::Character("a".into()), false);
        assert_eq!(a, Some(Action::InsertChar('a')));
    }

    #[test]
    fn insert_mode_escape_exits() {
        let s = make_state();
        s.vim_mode.set(VimMode::Insert);
        let a = key_to_action(&s, &Key::Named(NamedKey::Escape), false);
        assert_eq!(a, Some(Action::EnterNormal));
    }

    #[test]
    fn normal_j_k() {
        let s = make_state();
        let down = key_to_action(&s, &Key::Character("j".into()), false);
        assert_eq!(down, Some(Action::MoveDown(1)));
        let up = key_to_action(&s, &Key::Character("k".into()), false);
        assert_eq!(up, Some(Action::MoveUp(1)));
    }

    #[test]
    fn count_prefix_5j() {
        let s = make_state();
        // Press '5' — accumulates count, no action.
        let a1 = key_to_action(&s, &Key::Character("5".into()), false);
        assert_eq!(a1, None);
        assert_eq!(s.count.get(), Some(5));
        // Press 'j' — consume count → MoveDown(5).
        let a2 = key_to_action(&s, &Key::Character("j".into()), false);
        assert_eq!(a2, Some(Action::MoveDown(5)));
        assert_eq!(s.count.get(), None);
    }

    #[test]
    fn normal_g_is_bottom() {
        let s = make_state();
        let a = key_to_action(&s, &Key::Character("G".into()), false);
        assert_eq!(a, Some(Action::Bottom));
    }

    #[test]
    fn normal_enter_accepts() {
        let s = make_state();
        let a = key_to_action(&s, &Key::Named(NamedKey::Enter), false);
        assert_eq!(a, Some(Action::Accept));
    }

    #[test]
    fn normal_i_enters_insert() {
        let s = make_state();
        let a = key_to_action(&s, &Key::Character("i".into()), false);
        assert_eq!(a, Some(Action::EnterInsert));
    }
}
