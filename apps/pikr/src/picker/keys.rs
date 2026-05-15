//! Vim keymap → picker action.

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
