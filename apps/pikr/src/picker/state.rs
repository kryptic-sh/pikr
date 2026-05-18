//! Selection state machine with reactive floem signals.

use clap::ValueEnum;
use floem::reactive::{RwSignal, SignalGet, SignalUpdate};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, ValueEnum)]
pub enum VimMode {
    #[default]
    Normal,
    Insert,
    /// Range-select over the result list. `v` / `V` / `<C-v>` enter from
    /// Normal; the anchor lives on `PickerState::visual_anchor`. Esc → Normal.
    Visual,
}

/// Reactive state shared across the picker view tree.
///
/// All fields are `RwSignal`s so the floem view tree subscribes automatically.
#[derive(Clone)]
pub struct PickerState {
    pub query: RwSignal<String>,
    /// Char-index (codepoint count) into `query` where the editing caret
    /// sits. `0..=query.chars().count()`. Invariant: kept in range by every
    /// handler that mutates `query`.
    pub query_cursor: RwSignal<usize>,
    pub selected: RwSignal<usize>,
    pub vim_mode: RwSignal<VimMode>,
    /// Accumulated count prefix for motions like `5j`.
    pub count: RwSignal<Option<usize>>,
    /// Ex command buffer (`None` = not in ex mode).
    pub ex_buf: RwSignal<Option<String>>,
    /// Visual-mode anchor (the row where `v`/`V`/`<C-v>` was pressed). The
    /// selected range is `min(anchor, selected) ..= max(anchor, selected)`.
    /// `None` outside Visual mode.
    pub visual_anchor: RwSignal<Option<usize>>,
}

impl PickerState {
    pub fn new() -> Self {
        Self {
            query: RwSignal::new(String::new()),
            query_cursor: RwSignal::new(0),
            selected: RwSignal::new(0),
            // Start in Insert so the user can type a query immediately on launch.
            // Esc → Normal for j/k/G/etc. nav over the result list.
            vim_mode: RwSignal::new(VimMode::Insert),
            visual_anchor: RwSignal::new(None),
            count: RwSignal::new(None),
            ex_buf: RwSignal::new(None),
        }
    }

    /// Clamp `selected` to `[0, max)`. Skips `.set` when the value already
    /// matches so we don't fan out a spurious subscriber dispatch (floem's
    /// `set` is synchronous and triggers every subscriber even on no-op).
    pub fn clamp_selected(&self, max: usize) {
        let cur = self.selected.get_untracked();
        let next = if max == 0 { 0 } else { cur.min(max - 1) };
        if cur != next {
            self.selected.set(next);
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

#[cfg(test)]
mod tests {
    use super::*;
    use floem::reactive::Effect;
    use std::cell::Cell;
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};

    /// Regression for the no-results UI hang (2026-05-16). `rerank` was
    /// holding a `Mutex<AppState>` guard while calling `clamp_selected(0)`,
    /// which `selected.set(0)`. floem fires signal subscribers synchronously
    /// inside `set`; those subscribers re-locked the same mutex → permanent
    /// deadlock.
    ///
    /// We can't reproduce the exact UI subscriber graph in a unit test, but
    /// we can reproduce the same pattern: lock a mutex, set a signal whose
    /// subscriber locks the same mutex. WITHOUT `batch`, this would hang
    /// the test thread forever. WITH `batch`, the subscriber's effect runs
    /// AFTER the closure returns, by which time the guard is dropped.
    ///
    /// If a future refactor removes the `batch` wrap, this test deadlocks
    /// (test-runner timeout surfaces the regression).
    #[test]
    fn signal_set_inside_held_mutex_does_not_deadlock_when_batched() {
        let shared: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
        let observed: Rc<Cell<u32>> = Rc::new(Cell::new(0));
        let selected = RwSignal::new(0usize);

        // Subscriber: when `selected` changes, the effect locks `shared` and
        // bumps a counter. Floem fires this subscriber from inside the
        // call that triggers the signal change.
        {
            let shared = Arc::clone(&shared);
            let observed = observed.clone();
            Effect::new(move |_| {
                let _ = selected.get();
                let mut g = shared.lock().unwrap();
                *g += 1;
                observed.set(*g);
            });
        }
        // create_effect fires once at registration. With the lock free at
        // that point, the subscriber's counter is bumped to 1.
        assert_eq!(observed.get(), 1);

        // Production-shaped critical section: hold the mutex, set the signal.
        // Without `batch`, the subscriber would try to re-lock the held
        // mutex and the test thread would hang here (std::sync::Mutex is
        // non-reentrant).
        Effect::batch(|| {
            let g = shared.lock().unwrap();
            // With batch, signal dispatch is queued; the closure returns
            // and the guard drops before the subscriber actually runs.
            selected.set(1);
            assert_eq!(*g, 1);
        });

        // After batch returns, the subscriber has run with the lock free.
        assert_eq!(observed.get(), 2);
    }

    /// `clamp_selected(0)` when `selected` is already 0 must not fire the
    /// signal — otherwise every "no matches" rerank cascades a needless
    /// subscriber dispatch (status bar count, virtual_stack, etc).
    #[test]
    fn clamp_selected_zero_is_noop_when_already_zero() {
        let st = PickerState::new();
        st.selected.set(0);

        let count = Rc::new(Cell::new(0u32));
        {
            let count = count.clone();
            let sel = st.selected;
            Effect::new(move |_| {
                let _ = sel.get();
                count.set(count.get() + 1);
            });
        }
        // Initial registration fire.
        let baseline = count.get();
        st.clamp_selected(0);
        assert_eq!(
            count.get(),
            baseline,
            "clamp_selected(0) on already-0 selected must not refire subscribers"
        );
    }

    /// Clamping past-the-end pushes selected to max-1 and DOES fire (the
    /// value actually changed).
    #[test]
    fn clamp_selected_past_end_moves_to_last() {
        let st = PickerState::new();
        st.selected.set(10);
        st.clamp_selected(3);
        assert_eq!(st.selected.get_untracked(), 2);
    }

    /// Empty match list with non-zero selected must collapse to 0.
    #[test]
    fn clamp_selected_zero_collapses_nonzero() {
        let st = PickerState::new();
        st.selected.set(5);
        st.clamp_selected(0);
        assert_eq!(st.selected.get_untracked(), 0);
    }
}
