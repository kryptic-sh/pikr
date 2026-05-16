//! Floem view tree for pikr — tokyonight/rofi-style layout.

use std::sync::{Arc, Mutex};

use floem::{
    IntoView, View,
    event::{Event, EventListener, EventPropagation},
    keyboard::{Key, NamedKey},
    kurbo::{Rect, Size as KurboSize},
    peniko::Color,
    reactive::{RwSignal, SignalGet, SignalUpdate, create_effect},
    style::FlexDirection,
    views::{
        Decorators, container, dyn_stack, h_stack, label, scroll, stack_from_iter, text_input,
        v_stack,
    },
};

use crate::cli::Mode as CliMode;
use crate::config::Theme;
use crate::modes::{self, Entry};
use crate::picker::{
    keys::{Action, key_to_action},
    matcher::{Match, Matcher},
    state::{PickerState, VimMode},
};

// ─── Layout constants ────────────────────────────────────────────────────────

pub const ROW_HEIGHT: f64 = 36.0;
pub const ROW_GAP: f64 = 4.0;
pub const ROW_PITCH: f64 = ROW_HEIGHT + ROW_GAP;
pub const VISIBLE_ROWS: usize = 8;
pub const INPUT_ROW_HEIGHT: f64 = ROW_HEIGHT;
pub const STATUS_HEIGHT: f64 = 0.0;
const SCROLLOFF: f64 = 1.0;
const PANEL_PAD: f64 = 10.0;
const HORIZ_PAD: f64 = 14.0;
const DESC_GAP: f64 = 10.0;
const PANEL_RADIUS: f64 = 10.0;
const ROW_RADIUS: f64 = 6.0;
const BORDER_W: f64 = 1.5;

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

fn highlighted_label(
    label_str: String,
    positions: Vec<u32>,
    mi: usize,
    selected_sig: RwSignal<usize>,
    fg: Color,
    accent: Color,
) -> impl IntoView {
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
    let items: Vec<Box<dyn floem::View>> = spans
        .into_iter()
        .map(move |(text, hl)| {
            label(move || text.clone())
                .style(move |s| {
                    let selected = selected_sig.get() == mi;
                    // Selected row: label colour = accent. Match highlights flip
                    // to fg so highlighted chars stay visible against accent.
                    let color = match (selected, hl) {
                        (true, true) => fg,
                        (true, false) => accent,
                        (false, true) => accent,
                        (false, false) => fg,
                    };
                    s.color(color)
                })
                .into_any()
        })
        .collect();
    stack_from_iter(items).style(|s| s.flex_direction(FlexDirection::Row))
}

// ─── Entry-row view ──────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn entry_row(
    entry: Arc<Entry>,
    positions: Vec<u32>,
    mi: usize,
    selected_sig: RwSignal<usize>,
    fg: Color,
    accent: Color,
    muted: Color,
    selected_bg: Color,
) -> impl IntoView {
    let label_view =
        highlighted_label(entry.label.clone(), positions, mi, selected_sig, fg, accent);

    let desc_view: Box<dyn floem::View> = match &entry.description {
        Some(d) => {
            let d = format!("({d})");
            label(move || d.clone())
                .style(move |s| s.color(muted).margin_left(DESC_GAP))
                .into_any()
        }
        None => floem::views::empty().into_any(),
    };

    let click_payload = entry.payload.clone();

    h_stack((label_view.into_any(), desc_view))
        .style(move |s| {
            let selected = selected_sig.get() == mi;
            let bg = if selected {
                selected_bg
            } else {
                Color::TRANSPARENT
            };
            let border = if selected { accent } else { Color::TRANSPARENT };
            s.width_full()
                .height(ROW_HEIGHT)
                .padding_horiz(HORIZ_PAD)
                .items_center()
                .background(bg)
                .border(BORDER_W)
                .border_color(border)
                .border_radius(ROW_RADIUS)
                .cursor(floem::style::CursorStyle::Pointer)
        })
        .on_click_stop(move |_| {
            selected_sig.set(mi);
            if let Err(e) = crate::modes::execute(&click_payload) {
                eprintln!("pikr: execute error: {e}");
            }
            std::process::exit(0);
        })
}

// ─── Ex command bar ──────────────────────────────────────────────────────────

fn ex_bar(ex_buf: RwSignal<Option<String>>, fg: Color) -> impl IntoView {
    label(move || match ex_buf.get() {
        Some(s) => format!(":{s}"),
        None => String::new(),
    })
    .style(move |s| {
        s.width_full()
            .height(22.0)
            .padding_horiz(HORIZ_PAD)
            .color(fg)
    })
}

// ─── App-level reactive state ─────────────────────────────────────────────────

pub struct AppState {
    pub picker: PickerState,
    pub entries: Vec<Arc<Entry>>,
    pub matches: Vec<Match>,
    pub g_pending: bool,
    pub cli_mode: CliMode,
    pub prompt: String,
    pub max_results: usize,
    pub theme: Theme,
    pub matcher: Matcher,
}

impl AppState {
    pub fn rerank(&mut self) {
        let query = self.picker.query.get();
        if matches!(self.cli_mode, CliMode::Calc) {
            if let Some(result) = crate::modes::calc::eval(&query) {
                let label = format!("{} = {}", query.trim(), result);
                let entry = Entry {
                    label,
                    description: None,
                    payload: crate::modes::Payload::Stdout(result),
                };
                self.entries = vec![Arc::new(entry)];
                self.matches = vec![Match {
                    index: 0,
                    score: 0,
                    positions: Vec::new(),
                }];
            } else {
                self.entries = Vec::new();
                self.matches = Vec::new();
            }
            self.picker.clamp_selected(self.matches.len());
            return;
        }
        let labels: Vec<&str> = self.entries.iter().map(|e| e.label.as_str()).collect();
        self.matches = self.matcher.rank(&labels, &query);
        self.matches.truncate(self.max_results);
        self.picker.clamp_selected(self.matches.len());
    }

    pub fn switch_mode(&mut self, mode: CliMode) {
        self.cli_mode = mode;
        let mut m: Box<dyn crate::modes::Mode> = match mode {
            CliMode::Calc => Box::new(crate::modes::calc::Calc),
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

pub fn picker_view(state: Arc<Mutex<AppState>>) -> impl IntoView {
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

    let (bg, fg, accent, muted, selected_bg, font_family, font_size) = {
        let s = state.lock().unwrap();
        let t = &s.theme;
        (
            parse_color(&t.bg),
            parse_color(&t.fg),
            parse_color(&t.accent),
            parse_color(&t.muted),
            parse_color(&t.selected_bg),
            t.font.clone(),
            t.font_size,
        )
    };
    let prompt_str = state.lock().unwrap().prompt.clone();

    let rev: RwSignal<u64> = RwSignal::new(0);

    // ── Prompt + query input ───────────────────────────────────────────────
    // Use floem's `text_input` widget so cursor / selection / editing all
    // come for free. Prompt sits inline on the left as a label, input fills
    // the remaining row width via flex_grow.
    let input_font_size = font_size;
    let prompt_text = if prompt_str.is_empty() {
        ">".to_string()
    } else {
        format!("{}:", prompt_str)
    };
    let ff_prompt = font_family.clone();
    let prompt_label = label(move || prompt_text.clone()).style(move |s| {
        s.color(accent)
            .font_family(ff_prompt.clone())
            .font_size(input_font_size)
    });

    // text_input is bound to query_sig (RwSignal<String>); typing mutates it,
    // and our `create_effect` below reranks on each mutation.
    let ff_q = font_family.clone();
    let query_input = text_input(query_sig)
        .style(move |s| {
            s.color(fg)
                .font_family(ff_q.clone())
                .font_size(input_font_size)
                .flex_grow(1.0)
                .margin_left(10.0)
                .background(Color::TRANSPARENT)
                .border(0.0)
                .padding(0.0)
        })
        .keyboard_navigable();
    let input_id = query_input.id();

    // vim_mode → text_input focus. Insert focuses the input so typing edits the
    // query; Normal clears focus so j/k/G/etc. reach the outer key handler.
    create_effect(move |_| match vim_mode_sig.get() {
        VimMode::Insert => input_id.request_focus(),
        VimMode::Normal => input_id.clear_focus(),
    });

    // Rerank whenever the query mutates (text_input drives the signal).
    // Bump `rev` so the dyn_stack rebuilds against the new match list.
    let state_rerank = Arc::clone(&state);
    create_effect(move |prev: Option<String>| {
        let cur = query_sig.get();
        if prev.as_deref() != Some(cur.as_str()) {
            state_rerank.lock().unwrap().rerank();
            rev.update(|r| *r += 1);
        }
        cur
    });

    let input_row = h_stack((prompt_label, query_input)).style(move |s| {
        s.width_full()
            .height(INPUT_ROW_HEIGHT)
            .padding_horiz(HORIZ_PAD)
            .items_center()
            .background(selected_bg)
            .border_radius(ROW_RADIUS)
            .margin_bottom(8.0)
    });

    // ── Result list ────────────────────────────────────────────────────────
    let state_list = Arc::clone(&state);
    let ff_list = font_family.clone();
    let result_list = dyn_stack(
        move || {
            let _r = rev.get();
            let s = state_list.lock().unwrap();
            s.matches
                .iter()
                .enumerate()
                .map(|(mi, m)| {
                    let entry = Arc::clone(&s.entries[m.index]);
                    (mi, m.index, entry, m.positions.clone())
                })
                .collect::<Vec<_>>()
        },
        |(mi, idx, _, _)| ((*mi as u64) << 32) | (*idx as u64),
        move |(mi, _idx, entry, positions)| {
            let ff = ff_list.clone();
            entry_row(
                entry,
                positions,
                mi,
                selected_sig,
                fg,
                accent,
                muted,
                selected_bg,
            )
            .style(move |s| {
                s.font_family(ff.clone())
                    .font_size(font_size)
                    .margin_top(ROW_GAP)
            })
        },
    )
    .style(|s| s.width_full().flex_direction(FlexDirection::Column));

    let viewport_height = ROW_PITCH * VISIBLE_ROWS as f64;
    let scrollable = scroll(result_list)
        .ensure_visible(move || {
            let sel = selected_sig.get() as f64;
            let start_row = (sel - SCROLLOFF).max(0.0);
            let end_row = sel + 1.0 + SCROLLOFF;
            let top = start_row * ROW_PITCH;
            let height = (end_row - start_row) * ROW_PITCH;
            Rect::from_origin_size((0.0, top), KurboSize::new(1.0, height))
        })
        .style(move |s| {
            s.width_full()
                .height(viewport_height)
                .class(floem::views::scroll::Handle, move |h| {
                    h.background(muted).border_radius(3.0)
                })
        });

    let ex = ex_bar(ex_buf_sig, fg);

    // ── Outer panel ────────────────────────────────────────────────────────
    let state_key = Arc::clone(&state);
    container(
        v_stack((input_row, scrollable, ex.into_any()))
            .style(move |s| s.width_full().height_full().padding(PANEL_PAD)),
    )
    .style(move |s| {
        s.width_full()
            .height_full()
            .background(bg)
            .border(BORDER_W)
            .border_color(accent)
            .border_radius(PANEL_RADIUS)
    })
    .keyboard_navigable()
    .on_event(EventListener::KeyDown, move |ev| {
        let Event::KeyDown(ke) = ev else {
            return EventPropagation::Continue;
        };
        let ctrl = ke.modifiers.control();
        let key = &ke.key.logical_key;

        enum NavAction {
            None,
            Rerank,
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

        if let Some(mut buf) = ex_open {
            match key {
                Key::Named(NamedKey::Escape) => {
                    ex_buf_sig.set(None);
                }
                Key::Named(NamedKey::Enter) => {
                    let cmd = buf.trim().to_string();
                    ex_buf_sig.set(None);
                    let mode_switch = match cmd.as_str() {
                        "calc" => Some(CliMode::Calc),
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

        // `gg` two-key motion.
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
        if vim_mode == VimMode::Normal && matches!(key, Key::Character(c) if c.as_str() == "g") {
            state_key.lock().unwrap().g_pending = true;
            return EventPropagation::Stop;
        }
        if g_was_pending && !matches!(key, Key::Character(c) if c.as_str() == "g") {
            state_key.lock().unwrap().g_pending = false;
        }

        let Some(action) = action_opt else {
            return EventPropagation::Continue;
        };

        let after = NavAction::None;
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
            Action::EnterInsert => vim_mode_sig.set(VimMode::Insert),
            Action::EnterNormal => vim_mode_sig.set(VimMode::Normal),
            Action::StartSearch => vim_mode_sig.set(VimMode::Insert),
            Action::StartEx => ex_buf_sig.set(Some(String::new())),
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
            Action::Cancel => std::process::exit(0),
            // text_input owns char/backspace input; outer handler ignores them.
            Action::InsertChar(_) | Action::Backspace => {
                return EventPropagation::Continue;
            }
        }

        let _ = after;
        rev.update(|r| *r += 1);
        EventPropagation::Stop
    })
}
