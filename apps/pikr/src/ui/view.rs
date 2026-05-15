//! Floem view tree for pikr.
//!
//! Layout: [prompt | query] / scrollable result list / status bar.

use std::sync::{Arc, Mutex};

use floem::{
    IntoView,
    event::{Event, EventListener, EventPropagation},
    keyboard::{Key, NamedKey},
    peniko::Color,
    reactive::{RwSignal, SignalGet, SignalUpdate},
    style::FlexDirection,
    views::{Decorators, container, dyn_stack, h_stack, label, scroll, stack_from_iter, v_stack},
};

use crate::cli::Mode as CliMode;
use crate::config::Theme;
use crate::modes::{self, Entry};
use crate::picker::{
    keys::{Action, key_to_action},
    matcher::{Match, Matcher},
    state::{PickerState, VimMode},
};

// ─── Colour helpers ──────────────────────────────────────────────────────────

fn parse_color(hex: &str) -> Color {
    let s = hex.trim_start_matches('#');
    let rgb = u32::from_str_radix(s, 16).unwrap_or(0x00_00_00);
    let r = ((rgb >> 16) & 0xff) as u8;
    let g = ((rgb >> 8) & 0xff) as u8;
    let b = (rgb & 0xff) as u8;
    Color::rgb8(r, g, b)
}

// ─── Match-highlight helpers ─────────────────────────────────────────────────

/// Build a horizontal stack of spans with matched chars highlighted in accent.
fn highlighted_label(
    label_str: String,
    positions: Vec<u32>,
    fg: Color,
    accent: Color,
) -> impl IntoView {
    // Walk codepoints, split into (normal, highlighted) runs.
    let mut spans: Vec<(String, bool)> = Vec::new();
    let pos_set: std::collections::HashSet<u32> = positions.into_iter().collect();
    for (i, c) in label_str.chars().enumerate() {
        let hl = pos_set.contains(&(i as u32));
        if let Some(last) = spans.last_mut()
            && last.1 == hl
        {
            last.0.push(c);
            continue;
        }
        spans.push((c.to_string(), hl));
    }
    let fg_clone = fg;
    let accent_clone = accent;
    let items: Vec<Box<dyn floem::View>> = spans
        .into_iter()
        .map(move |(text, hl)| {
            let color = if hl { accent_clone } else { fg_clone };
            label(move || text.clone())
                .style(move |s| s.color(color))
                .into_any()
        })
        .collect();
    h_stack_from_iter(items)
}

fn h_stack_from_iter(items: Vec<Box<dyn floem::View>>) -> impl IntoView {
    stack_from_iter(items).style(|s| s.flex_direction(FlexDirection::Row))
}

// ─── Entry-row view ──────────────────────────────────────────────────────────

fn entry_row(
    entry: Entry,
    positions: Vec<u32>,
    is_selected: bool,
    fg: Color,
    accent: Color,
    bg: Color,
) -> impl IntoView {
    let row_bg = if is_selected {
        // Blend bg toward accent (30% accent, 70% bg).
        Color::rgba8(
            ((accent.r as u16 * 30 + bg.r as u16 * 70) / 100) as u8,
            ((accent.g as u16 * 30 + bg.g as u16 * 70) / 100) as u8,
            ((accent.b as u16 * 30 + bg.b as u16 * 70) / 100) as u8,
            255,
        )
    } else {
        bg
    };

    let label_view = highlighted_label(entry.label.clone(), positions, fg, accent);

    let desc_view: Box<dyn floem::View> = match &entry.description {
        Some(d) => {
            let d = d.clone();
            let dim = fg.multiply_alpha(0.6);
            label(move || d.clone())
                .style(move |s| s.color(dim).margin_left(8.0))
                .into_any()
        }
        None => floem::views::empty().into_any(),
    };

    h_stack((label_view.into_any(), desc_view))
        .style(move |s| s.width_full().padding(4.0).background(row_bg))
}

// ─── Ex command bar ──────────────────────────────────────────────────────────

fn ex_bar(ex_buf: RwSignal<Option<String>>, fg: Color, bg: Color) -> impl IntoView {
    label(move || match ex_buf.get() {
        Some(s) => format!(":{s}"),
        None => String::new(),
    })
    .style(move |s| {
        s.width_full()
            .height(22.0)
            .padding_left(4.0)
            .color(fg)
            .background(bg)
    })
}

// ─── App-level reactive state ─────────────────────────────────────────────────

/// All mutable picker state that lives in `Arc<Mutex<_>>` so event closures
/// can share it without the `Send` restriction on `RwSignal`.
///
/// The entry list and matches are re-computed in the key-event handler
/// whenever the query or mode changes.
pub struct AppState {
    pub picker: PickerState,
    /// Full entry list for the active mode.
    pub entries: Vec<Entry>,
    /// Ranked matches against `picker.query`. Refreshed on query/mode change.
    pub matches: Vec<Match>,
    /// Pending `g` for `gg` motion.
    pub g_pending: bool,
    /// Active CLI mode (for ex mode switching).
    pub cli_mode: CliMode,
    /// Prompt string from CLI.
    pub prompt: String,
    /// Max results cap from config.
    pub max_results: usize,
    /// Theme.
    pub theme: Theme,
    /// Cached fuzzy matcher (nucleo allocates ~135KB per instance).
    pub matcher: Matcher,
}

impl AppState {
    /// Re-rank entries against the current query.
    pub fn rerank(&mut self) {
        let query = self.picker.query.get();
        let labels: Vec<&str> = self.entries.iter().map(|e| e.label.as_str()).collect();
        self.matches = self.matcher.rank(&labels, &query);
        self.matches.truncate(self.max_results);
        self.picker.clamp_selected(self.matches.len());
    }

    /// Switch to a new mode and reload entries.
    pub fn switch_mode(&mut self, mode: CliMode) {
        self.cli_mode = mode;
        let mut m: Box<dyn crate::modes::Mode> = match mode {
            CliMode::Dmenu => Box::new(crate::modes::dmenu::Dmenu),
            CliMode::Drun => Box::new(crate::modes::drun::Drun),
            CliMode::Run => Box::new(crate::modes::run::Run),
        };
        self.entries = m.collect().unwrap_or_default();
        self.picker.query.set(String::new());
        self.picker.selected.set(0);
        self.rerank();
    }
}

// ─── Picker view ─────────────────────────────────────────────────────────────

/// Build and launch the picker UI.
///
/// `state` is wrapped in `Arc<Mutex<>>` so the key-event handler closure
/// can mutate it while floem views read reactive signals independently.
pub fn picker_view(state: Arc<Mutex<AppState>>) -> impl IntoView {
    // Pull signals out of the shared state — they're `Copy`.
    let (query_sig, selected_sig, vim_mode_sig, ex_buf_sig) = {
        let s = state.lock().unwrap();
        (
            s.picker.query,
            s.picker.selected,
            s.picker.vim_mode,
            s.picker.ex_buf,
        )
    };

    // Derive theme colors upfront (read-only, no signal needed).
    let (bg, fg, accent, font_family, font_size) = {
        let s = state.lock().unwrap();
        let t = &s.theme;
        (
            parse_color(&t.bg),
            parse_color(&t.fg),
            parse_color(&t.accent),
            t.font.clone(),
            t.font_size,
        )
    };
    let prompt_str = {
        let s = state.lock().unwrap();
        s.prompt.clone()
    };

    // ── Reactive match list signal ─────────────────────────────────────────
    // We store a "revision" signal that bumps on any state change so the
    // dyn_stack rerenders. The actual matches come from state.matches.
    let rev: RwSignal<u64> = RwSignal::new(0);

    // ── Input row ─────────────────────────────────────────────────────────
    let prompt_label = {
        let p = prompt_str.clone();
        let ff = font_family.clone();
        label(move || {
            if p.is_empty() {
                "> ".to_string()
            } else {
                format!("{}: ", p)
            }
        })
        .style(move |s| {
            s.color(accent)
                .font_family(ff.clone())
                .font_size(font_size)
                .padding(4.0)
        })
    };

    let ff_q = font_family.clone();
    let query_label = label(move || query_sig.get()).style(move |s| {
        s.color(fg)
            .font_family(ff_q.clone())
            .font_size(font_size)
            .flex_grow(1.0)
            .padding(4.0)
    });

    let input_row = h_stack((prompt_label, query_label)).style(move |s| {
        s.width_full()
            .background(bg)
            .border_bottom(1.0)
            .border_color(accent)
    });

    // ── Result list ────────────────────────────────────────────────────────
    let state_list = Arc::clone(&state);
    let ff_list = font_family.clone();
    let result_list = dyn_stack(
        move || {
            let _r = rev.get(); // subscribe to rev signal
            let s = state_list.lock().unwrap();
            let sel = selected_sig.get();
            // Collect (index_in_matches, entry_clone, positions, is_selected).
            s.matches
                .iter()
                .enumerate()
                .map(|(mi, m)| {
                    let entry = s.entries[m.index].clone();
                    (mi, entry, m.positions.clone(), mi == sel)
                })
                .collect::<Vec<_>>()
        },
        |(mi, _, _, _)| *mi,
        move |(_, entry, positions, is_selected)| {
            let ff = ff_list.clone();
            entry_row(entry, positions, is_selected, fg, accent, bg)
                .style(move |s| s.font_family(ff.clone()).font_size(font_size))
        },
    )
    .style(|s| s.width_full().flex_direction(FlexDirection::Column));

    let scrollable =
        scroll(result_list).style(move |s| s.width_full().flex_grow(1.0).background(bg));

    // ── Status bar ─────────────────────────────────────────────────────────
    let ff_s = font_family.clone();
    let status = label(move || {
        let mode = match vim_mode_sig.get() {
            VimMode::Normal => "NORMAL",
            VimMode::Insert => "INSERT",
        };
        let sel = selected_sig.get();
        format!(" {mode}  {sel} ")
    })
    .style(move |s| {
        s.width_full()
            .height(22.0)
            .color(bg)
            .background(accent)
            .font_family(ff_s.clone())
            .font_size(font_size)
    });

    // ── Ex bar ─────────────────────────────────────────────────────────────
    let ex = ex_bar(ex_buf_sig, fg, bg);

    // ── Outer container with keyboard handler ──────────────────────────────
    let state_key = Arc::clone(&state);
    container(
        v_stack((input_row, scrollable, ex.into_any(), status))
            .style(|s| s.width_full().height_full()),
    )
    .keyboard_navigable()
    .on_event(EventListener::KeyDown, move |ev| {
        let Event::KeyDown(ke) = ev else {
            return EventPropagation::Continue;
        };

        let ctrl = ke.modifiers.control();
        let key = &ke.key.logical_key;

        let mut s = state_key.lock().unwrap();
        let vim_mode = s.picker.vim_mode.get();

        // ── Ex mode ───────────────────────────────────────────────────────
        if let Some(mut buf) = s.picker.ex_buf.get() {
            match key {
                Key::Named(NamedKey::Escape) => {
                    s.picker.ex_buf.set(None);
                }
                Key::Named(NamedKey::Enter) => {
                    let cmd = buf.trim().to_string();
                    s.picker.ex_buf.set(None);
                    // Dispatch ex command.
                    let mode_switch = match cmd.as_str() {
                        "drun" => Some(CliMode::Drun),
                        "run" => Some(CliMode::Run),
                        "dmenu" => Some(CliMode::Dmenu),
                        "q" | "q!" => {
                            floem::quit_app();
                            return EventPropagation::Stop;
                        }
                        _ => None,
                    };
                    if let Some(m) = mode_switch {
                        drop(s);
                        state_key.lock().unwrap().switch_mode(m);
                        rev.update(|r| *r += 1);
                    }
                }
                Key::Named(NamedKey::Backspace) => {
                    buf.pop();
                    s.picker.ex_buf.set(Some(buf));
                }
                Key::Character(ch) => {
                    buf.push_str(ch);
                    s.picker.ex_buf.set(Some(buf));
                }
                _ => {}
            }
            return EventPropagation::Stop;
        }

        // ── `g` pending for `gg` ─────────────────────────────────────────
        if vim_mode == VimMode::Normal
            && s.g_pending
            && matches!(key, Key::Character(c) if c.as_str() == "g")
        {
            s.g_pending = false;
            s.picker.count.set(None);
            s.picker.selected.set(0);
            rev.update(|r| *r += 1);
            return EventPropagation::Stop;
        }
        // First `g` in normal mode.
        if vim_mode == VimMode::Normal && matches!(key, Key::Character(c) if c.as_str() == "g") {
            s.g_pending = true;
            return EventPropagation::Stop;
        }
        // Any key other than `g` clears the pending flag.
        if s.g_pending && !matches!(key, Key::Character(c) if c.as_str() == "g") {
            s.g_pending = false;
        }

        // ── Normal action dispatch ────────────────────────────────────────
        let action = key_to_action(&s.picker, key, ctrl);
        let Some(action) = action else {
            return EventPropagation::Continue;
        };

        let total = s.matches.len();
        match action {
            Action::MoveDown(n) => {
                let cur = s.picker.selected.get();
                let next = (cur + n).min(total.saturating_sub(1));
                s.picker.selected.set(next);
            }
            Action::MoveUp(n) => {
                let cur = s.picker.selected.get();
                s.picker.selected.set(cur.saturating_sub(n));
            }
            Action::PageDown => {
                let cur = s.picker.selected.get();
                let next = (cur + 10).min(total.saturating_sub(1));
                s.picker.selected.set(next);
            }
            Action::PageUp => {
                let cur = s.picker.selected.get();
                s.picker.selected.set(cur.saturating_sub(10));
            }
            Action::Top => {
                s.picker.selected.set(0);
            }
            Action::Bottom => {
                if total > 0 {
                    s.picker.selected.set(total - 1);
                }
            }
            Action::EnterInsert => {
                s.picker.vim_mode.set(VimMode::Insert);
            }
            Action::EnterNormal => {
                s.picker.vim_mode.set(VimMode::Normal);
            }
            Action::StartSearch => {
                // `/` enters insert so the query bar becomes active.
                s.picker.vim_mode.set(VimMode::Insert);
            }
            Action::StartEx => {
                s.picker.ex_buf.set(Some(String::new()));
            }
            Action::Accept => {
                let sel = s.picker.selected.get();
                if let Some(m) = s.matches.get(sel) {
                    let payload = s.entries[m.index].payload.clone();
                    drop(s); // release lock before execute
                    if let Err(e) = modes::execute(&payload) {
                        eprintln!("pikr: execute error: {e}");
                    }
                    std::process::exit(0);
                }
            }
            Action::Cancel => {
                std::process::exit(0);
            }
            Action::InsertChar(c) => {
                let mut q = s.picker.query.get();
                q.push(c);
                s.picker.query.set(q.clone());
                s.rerank();
                rev.update(|r| *r += 1);
            }
            Action::Backspace => {
                let mut q = s.picker.query.get();
                q.pop();
                s.picker.query.set(q.clone());
                s.rerank();
                rev.update(|r| *r += 1);
            }
        }

        // Bump rev for any navigation change (selection, mode).
        rev.update(|r| *r += 1);
        EventPropagation::Stop
    })
    .style(move |s| s.width_full().height_full().background(bg))
}
