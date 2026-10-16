//! The picker's editing and selection state.

use clap::ValueEnum;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum VimMode {
    Normal,
    Insert,
    /// Range-select over the result list. `v` / `V` / `<C-v>` enter from
    /// Normal; the anchor lives on `PickerState::visual_anchor`. Esc → Normal.
    Visual,
}

/// What the user has typed and selected, independent of how it is drawn.
#[derive(Debug, Clone)]
pub struct PickerState {
    pub query: String,
    /// Char-index (codepoint count) into `query` where the editing caret
    /// sits. `0..=query.chars().count()`. Invariant: kept in range by every
    /// handler that mutates `query`.
    pub query_cursor: usize,
    pub selected: usize,
    pub vim_mode: VimMode,
    /// Accumulated count prefix for motions like `5j`.
    pub count: Option<usize>,
    /// Ex command buffer (`None` = not in ex mode).
    pub ex_buf: Option<String>,
    /// Visual-mode anchor (the row where `v`/`V`/`<C-v>` was pressed). The
    /// selected range is `min(anchor, selected) ..= max(anchor, selected)`.
    /// `None` outside Visual mode.
    pub visual_anchor: Option<usize>,
}

impl PickerState {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            query_cursor: 0,
            selected: 0,
            // Start in Insert so the user can type a query immediately on launch.
            // Esc → Normal for j/k/G/etc. nav over the result list.
            vim_mode: VimMode::Insert,
            visual_anchor: None,
            count: None,
            ex_buf: None,
        }
    }

    /// Clamp `selected` to `[0, max)`.
    pub fn clamp_selected(&mut self, max: usize) {
        self.selected = if max == 0 {
            0
        } else {
            self.selected.min(max - 1)
        };
    }

    /// Consume and return the current count prefix (defaults to 1).
    pub fn take_count(&mut self) -> usize {
        self.count.take().unwrap_or(1)
    }

    /// A mode switch rebuilds the entry list and drops selection state; it
    /// must also exit Visual and clear any count prefix. Leaving Visual armed
    /// with a stale anchor made the next Enter execute a range of the NEW
    /// mode's list (the `:mode`-from-Visual multi-launch bug).
    pub fn reset_after_mode_switch(&mut self) {
        self.vim_mode = VimMode::Normal;
        self.visual_anchor = None;
        self.count = None;
    }

    /// Push a digit into the count prefix.
    pub fn push_count_digit(&mut self, d: u32) {
        let cur = self.count.unwrap_or(0);
        // Saturate instead of wrapping: a pathological multi-digit count
        // prefix must not silently wrap back to a small number in release.
        let next = cur.saturating_mul(10).saturating_add(d as usize);
        self.count = Some(next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_selected_past_end_moves_to_last() {
        let mut st = PickerState::new();
        st.selected = 10;
        st.clamp_selected(3);
        assert_eq!(st.selected, 2);
    }

    /// Empty match list with non-zero selected must collapse to 0.
    #[test]
    fn clamp_selected_zero_collapses_nonzero() {
        let mut st = PickerState::new();
        st.selected = 5;
        st.clamp_selected(0);
        assert_eq!(st.selected, 0);
    }

    #[test]
    fn reset_after_mode_switch_exits_visual_and_clears_count() {
        // Regression for the `:mode`-from-Visual bug: a mode switch must
        // leave the picker in Normal with no anchor, or the next Enter
        // executes a range of the new mode's list.
        let mut st = PickerState::new();
        st.vim_mode = VimMode::Visual;
        st.visual_anchor = Some(5);
        st.count = Some(usize::MAX);
        st.reset_after_mode_switch();
        assert_eq!(st.vim_mode, VimMode::Normal);
        assert_eq!(st.visual_anchor, None);
        assert_eq!(st.count, None);
    }

    /// Pathological count prefixes (dozens of digits) saturate at
    /// `usize::MAX` instead of silently wrapping back to a small number.
    #[test]
    fn push_count_digit_saturates_on_overflow() {
        let mut st = PickerState::new();
        for _ in 0..25 {
            st.push_count_digit(9);
        }
        assert_eq!(st.count, Some(usize::MAX));
    }
}
