//! Selection state machine with reactive floem signals.

use floem::reactive::{RwSignal, SignalGet, SignalUpdate};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VimMode {
    #[default]
    Normal,
    Insert,
}

/// Reactive state shared across the picker view tree.
///
/// All fields are `RwSignal`s so the floem view tree subscribes automatically.
#[derive(Clone)]
pub struct PickerState {
    pub query: RwSignal<String>,
    pub selected: RwSignal<usize>,
    pub vim_mode: RwSignal<VimMode>,
    /// Accumulated count prefix for motions like `5j`.
    pub count: RwSignal<Option<usize>>,
    /// Ex command buffer (`None` = not in ex mode).
    pub ex_buf: RwSignal<Option<String>>,
}

impl PickerState {
    pub fn new() -> Self {
        Self {
            query: RwSignal::new(String::new()),
            selected: RwSignal::new(0),
            vim_mode: RwSignal::new(VimMode::Normal),
            count: RwSignal::new(None),
            ex_buf: RwSignal::new(None),
        }
    }

    /// Clamp `selected` to `[0, max)`.
    pub fn clamp_selected(&self, max: usize) {
        if max == 0 {
            self.selected.set(0);
            return;
        }
        let cur = self.selected.get();
        if cur >= max {
            self.selected.set(max - 1);
        }
    }

    /// Consume and return the current count prefix (defaults to 1).
    pub fn take_count(&self) -> usize {
        let n = self.count.get().unwrap_or(1);
        self.count.set(None);
        n
    }

    /// Push a digit into the count prefix.
    pub fn push_count_digit(&self, d: u32) {
        let cur = self.count.get().unwrap_or(0);
        self.count.set(Some(cur * 10 + d as usize));
    }
}

impl Default for PickerState {
    fn default() -> Self {
        Self::new()
    }
}
