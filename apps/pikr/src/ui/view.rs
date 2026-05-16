//! Floem view tree for pikr.
//!
//! Layout: [prompt | query] / scrollable result list / status bar.

use std::sync::{Arc, Mutex};

use floem::{
    IntoView,
    event::{Event, EventListener, EventPropagation},
    ext_event::create_signal_from_channel,
    keyboard::{Key, NamedKey},
    kurbo::{Rect, Size as KurboSize},
    peniko::Color,
    reactive::{RwSignal, SignalGet, SignalUpdate, create_effect},
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
    entry: Arc<Entry>,
    positions: Vec<u32>,
    mi: usize,
    selected_sig: RwSignal<usize>,
    fg: Color,
    accent: Color,
    bg: Color,
) -> impl IntoView {
    // Selection-blended background: 55% accent over bg — visible enough
    // to spot at a glance against a busy dark theme.
    let selected_bg = Color::rgba8(
        ((accent.r as u16 * 55 + bg.r as u16 * 45) / 100) as u8,
        ((accent.g as u16 * 55 + bg.g as u16 * 45) / 100) as u8,
        ((accent.b as u16 * 55 + bg.b as u16 * 45) / 100) as u8,
        255,
    );

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

    h_stack((label_view.into_any(), desc_view)).style(move |s| {
        // Subscribe to selected_sig HERE so highlight tracks selection
        // without rebuilding the row (dyn_stack keys items by `mi`, so
        // a per-row bg baked in at construction would be stuck).
        let row_bg = if selected_sig.get() == mi {
            selected_bg
        } else {
            bg
        };
        // Fixed row height so the viewport math stays exact. items_center
        // vertically aligns the (label, description) pair inside the row
        // instead of stacking them to the top.
        s.width_full()
            .height(ROW_HEIGHT)
            .padding_horiz(10.0)
            .items_center()
            .background(row_bg)
    })
}

/// Pixel height per result row, in logical pixels. Used both by the row
/// itself and by the scroll viewport so the viewport stays an exact
/// multiple of row height.
pub(crate) const ROW_HEIGHT: f64 = 26.0;
/// Number of result rows the viewport shows. Window height is sized
/// from this plus the prompt + status chrome.
pub(crate) const VISIBLE_ROWS: usize = 10;
/// Prompt+query row height, matches ROW_HEIGHT for visual consistency.
pub(crate) const INPUT_ROW_HEIGHT: f64 = 30.0;
/// Status bar height.
pub(crate) const STATUS_HEIGHT: f64 = 24.0;

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
    /// Full entry list for the active mode. Wrapped in Arc so the
    /// dyn_stack items closure can hand each visible row a cheap
    /// refcount-clone instead of a deep clone of the underlying
    /// String/Vec data on every signal-triggered rerun.
    pub entries: Vec<Arc<Entry>>,
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
            CliMode::Clipboard => Box::new(crate::modes::clipboard::Clipboard),
            CliMode::Dmenu => Box::new(crate::modes::dmenu::Dmenu),
            CliMode::Drun => Box::new(crate::modes::drun::Drun),
            CliMode::Emoji => Box::new(crate::modes::emoji::Emoji),
            CliMode::Run => Box::new(crate::modes::run::Run),
            CliMode::Ssh => Box::new(crate::modes::ssh::Ssh),
        };
        self.entries = m
            .collect()
            .unwrap_or_default()
            .into_iter()
            .map(Arc::new)
            .collect();
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
    // Pull signals out of the shared state — they're `Copy`. The key
    // handler MUST update signals while NOT holding the AppState mutex,
    // because floem fires reactive subscribers (e.g. the dyn_stack items
    // closure) synchronously inside `set()` and those subscribers re-lock
    // the same mutex. Capturing the signals once up front lets the
    // handler drop the guard before any `set()` call.
    let (query_sig, selected_sig, vim_mode_sig, ex_buf_sig, count_sig) = {
        let s = state.lock().unwrap();
        (
            s.picker.query,
            s.picker.selected,
            s.picker.vim_mode,
            s.picker.ex_buf,
            s.picker.count,
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
        .style(move |s| s.color(accent).font_family(ff.clone()).font_size(font_size))
    };

    let ff_q = font_family.clone();
    // Cursor glyph follows vim mode: thin bar in Insert (caret between
    // chars), block in Normal (vim-style block cursor). Both are appended
    // to the query end since pikr does not track an in-string cursor
    // position. `blink_on` toggles every ~530ms via a background thread;
    // when off we render a space to keep the line width stable in the
    // monospace font.
    let blink_on: RwSignal<bool> = RwSignal::new(true);
    {
        let (tx, rx) = crossbeam_channel::unbounded::<()>();
        let tick_sig = create_signal_from_channel(rx);
        create_effect(move |_| {
            // Track every tick from the channel; flip the visibility bit.
            let _ = tick_sig.get();
            blink_on.update(|b| *b = !*b);
        });
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_millis(530));
                if tx.send(()).is_err() {
                    break;
                }
            }
        });
    }
    // Reset cursor to visible on every keystroke so the caret never
    // hides mid-type; the blink timer carries on toggling after.
    create_effect(move |_| {
        let _ = query_sig.get();
        blink_on.set(true);
    });
    let query_label = label(move || {
        let q = query_sig.get();
        let visible = blink_on.get();
        let cursor = match vim_mode_sig.get() {
            VimMode::Insert => '\u{258F}', // ▏ LEFT ONE EIGHTH BLOCK
            VimMode::Normal => '\u{2588}', // █ FULL BLOCK
        };
        let glyph = if visible { cursor } else { ' ' };
        format!("{}{}", q, glyph)
    })
    .style(move |s| {
        s.color(fg)
            .font_family(ff_q.clone())
            .font_size(font_size)
            .flex_grow(1.0)
            .margin_left(6.0)
    });

    let input_row = h_stack((prompt_label, query_label)).style(move |s| {
        s.width_full()
            .height(INPUT_ROW_HEIGHT)
            .padding_horiz(10.0)
            .items_center()
            .background(bg)
            .border_bottom(1.0)
            .border_color(accent)
    });

    // ── Result list ────────────────────────────────────────────────────────
    let state_list = Arc::clone(&state);
    let ff_list = font_family.clone();
    let result_list = dyn_stack(
        move || {
            // Re-build the row list only when entries/matches change. The
            // `rev` signal is bumped on query / mode-switch / rerank.
            // Selection highlight is reactive INSIDE each row via
            // selected_sig, not via rebuilding the list — see entry_row.
            let _r = rev.get();
            let s = state_list.lock().unwrap();
            s.matches
                .iter()
                .enumerate()
                .map(|(mi, m)| {
                    // Arc::clone — cheap refcount bump, not a String/Vec clone.
                    let entry = Arc::clone(&s.entries[m.index]);
                    (mi, m.index, entry, m.positions.clone())
                })
                .collect::<Vec<_>>()
        },
        // Key by (slot, entry-index). Slot alone is wrong: after the match
        // list shrinks, the kept slots still reference the old entry baked
        // in at the previous render. Including m.index forces a rebuild
        // whenever the slot now points at a different entry.
        |(mi, idx, _, _)| ((*mi as u64) << 32) | (*idx as u64),
        move |(mi, _idx, entry, positions)| {
            let ff = ff_list.clone();
            entry_row(entry, positions, mi, selected_sig, fg, accent, bg)
                .style(move |s| s.font_family(ff.clone()).font_size(font_size))
        },
    )
    .style(|s| s.width_full().flex_direction(FlexDirection::Column));

    // Viewport height is locked to an integer number of rows. Combined
    // with the fixed ROW_HEIGHT on each entry_row this guarantees the
    // scroll always shows whole rows — no half-clipped row at the
    // top/bottom edge as the user pages through.
    //
    // `ensure_visible` follows the cursor: every selection change emits
    // a Rect at the row's exact position and floem scrolls the viewport
    // just enough to keep it on-screen.
    let viewport_height = ROW_HEIGHT * VISIBLE_ROWS as f64;
    let scrollable = scroll(result_list)
        .ensure_visible(move || {
            let sel = selected_sig.get() as f64;
            Rect::from_origin_size((0.0, sel * ROW_HEIGHT), KurboSize::new(1.0, ROW_HEIGHT))
        })
        .style(move |s| s.width_full().height(viewport_height).background(bg));

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
            .height(STATUS_HEIGHT)
            .padding_horiz(10.0)
            .items_center()
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
    .style(move |s| {
        // Layer surfaces don't get compositor borders. Draw our own so
        // pikr looks like a window on both wlr-layer-shell and xdg
        // toplevels (where it's also fine — most tiling WMs strip
        // borders off floating popups anyway).
        s.width_full()
            .height_full()
            .background(bg)
            .border(1.5)
            .border_color(accent)
            .border_radius(8.0)
    })
    .keyboard_navigable()
    .on_event(EventListener::KeyDown, move |ev| {
        let Event::KeyDown(ke) = ev else {
            return EventPropagation::Continue;
        };

        let ctrl = ke.modifiers.control();
        let key = &ke.key.logical_key;
        tracing::debug!(?key, ctrl, "key down");

        // Snapshot everything we need from AppState BEFORE any signal.set().
        // Holding `state_key`'s guard across a `.set()` deadlocks because
        // floem fires reactive subscribers synchronously and the
        // result-list dyn_stack closure re-locks the same mutex.
        enum NavAction {
            None,
            Rerank,
            SwitchMode(CliMode),
        }
        let (vim_mode, ex_open, total, g_was_pending, action_opt) = {
            let s = state_key.lock().unwrap();
            let vim_mode = vim_mode_sig.get();
            let ex_open = ex_buf_sig.get();
            let total = s.matches.len();
            let g_pending = s.g_pending;
            let action = if ex_open.is_some() {
                None
            } else {
                key_to_action(&s.picker, key, ctrl)
            };
            (vim_mode, ex_open, total, g_pending, action)
        };

        // ── Ex mode ───────────────────────────────────────────────────────
        if let Some(mut buf) = ex_open {
            match key {
                Key::Named(NamedKey::Escape) => {
                    ex_buf_sig.set(None);
                }
                Key::Named(NamedKey::Enter) => {
                    let cmd = buf.trim().to_string();
                    ex_buf_sig.set(None);
                    let mode_switch = match cmd.as_str() {
                        "clipboard" => Some(CliMode::Clipboard),
                        "dmenu" => Some(CliMode::Dmenu),
                        "drun" => Some(CliMode::Drun),
                        "emoji" => Some(CliMode::Emoji),
                        "run" => Some(CliMode::Run),
                        "ssh" => Some(CliMode::Ssh),
                        "q" | "q!" => {
                            floem::quit_app();
                            return EventPropagation::Stop;
                        }
                        _ => None,
                    };
                    if let Some(m) = mode_switch {
                        state_key.lock().unwrap().switch_mode(m);
                        rev.update(|r| *r += 1);
                    }
                }
                Key::Named(NamedKey::Backspace) => {
                    buf.pop();
                    ex_buf_sig.set(Some(buf));
                }
                Key::Character(ch) => {
                    buf.push_str(ch);
                    ex_buf_sig.set(Some(buf));
                }
                _ => {}
            }
            return EventPropagation::Stop;
        }

        // ── `g` pending for `gg` ─────────────────────────────────────────
        if vim_mode == VimMode::Normal
            && g_was_pending
            && matches!(key, Key::Character(c) if c.as_str() == "g")
        {
            state_key.lock().unwrap().g_pending = false;
            count_sig.set(None);
            selected_sig.set(0);
            rev.update(|r| *r += 1);
            return EventPropagation::Stop;
        }
        // First `g` in normal mode.
        if vim_mode == VimMode::Normal && matches!(key, Key::Character(c) if c.as_str() == "g") {
            state_key.lock().unwrap().g_pending = true;
            return EventPropagation::Stop;
        }
        // Any key other than `g` clears the pending flag.
        if g_was_pending && !matches!(key, Key::Character(c) if c.as_str() == "g") {
            state_key.lock().unwrap().g_pending = false;
        }

        // ── Normal action dispatch ────────────────────────────────────────
        let Some(action) = action_opt else {
            return EventPropagation::Continue;
        };

        let mut after = NavAction::None;
        match action {
            Action::MoveDown(n) => {
                let cur = selected_sig.get();
                let next = (cur + n).min(total.saturating_sub(1));
                selected_sig.set(next);
            }
            Action::MoveUp(n) => {
                let cur = selected_sig.get();
                selected_sig.set(cur.saturating_sub(n));
            }
            Action::PageDown => {
                let cur = selected_sig.get();
                let next = (cur + 10).min(total.saturating_sub(1));
                selected_sig.set(next);
            }
            Action::PageUp => {
                let cur = selected_sig.get();
                selected_sig.set(cur.saturating_sub(10));
            }
            Action::Top => {
                selected_sig.set(0);
            }
            Action::Bottom => {
                if total > 0 {
                    selected_sig.set(total - 1);
                }
            }
            Action::EnterInsert => {
                vim_mode_sig.set(VimMode::Insert);
            }
            Action::EnterNormal => {
                vim_mode_sig.set(VimMode::Normal);
            }
            Action::StartSearch => {
                vim_mode_sig.set(VimMode::Insert);
            }
            Action::StartEx => {
                ex_buf_sig.set(Some(String::new()));
            }
            Action::Accept => {
                let sel = selected_sig.get();
                let payload = {
                    let s = state_key.lock().unwrap();
                    s.matches
                        .get(sel)
                        .map(|m| s.entries[m.index].payload.clone())
                };
                if let Some(payload) = payload {
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
                let mut q = query_sig.get();
                q.push(c);
                query_sig.set(q);
                after = NavAction::Rerank;
            }
            Action::Backspace => {
                let mut q = query_sig.get();
                q.pop();
                query_sig.set(q);
                after = NavAction::Rerank;
            }
        }

        // Mutations that need the AppState lock — perform them AFTER all
        // signal sets so subscribers don't re-enter while we hold it.
        match after {
            NavAction::Rerank => {
                state_key.lock().unwrap().rerank();
            }
            NavAction::SwitchMode(m) => {
                state_key.lock().unwrap().switch_mode(m);
            }
            NavAction::None => {}
        }

        // Bump rev for any navigation change (selection, mode).
        rev.update(|r| *r += 1);
        EventPropagation::Stop
    })
    .style(move |s| s.width_full().height_full().background(bg))
}
