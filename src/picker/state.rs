//! Selection state machine.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VimMode {
    #[default]
    Normal,
    Insert,
}

#[derive(Debug, Default)]
pub struct PickerState {
    pub query: String,
    pub selected: usize,
    pub mode: VimMode,
    pub count: Option<usize>,
}
