//! Floem view tree for pikr — tokyonight/rofi-style layout.

use std::sync::{Arc, Mutex};

use floem::{
    IntoView,
    event::{Event, EventListener, EventPropagation},
    ext_event::create_signal_from_channel,
    keyboard::{Key, NamedKey},
    kurbo::{Rect, Size as KurboSize},
    peniko::Color,
    reactive::{RwSignal, SignalGet, SignalUpdate, batch, create_effect},
    style::FlexDirection,
    views::{
        Decorators, VirtualDirection, VirtualItemSize, container, h_stack, label, scroll,
        stack_from_iter, v_stack, virtual_stack,
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

/// Stable key for a result-list row, used by dyn_stack to decide whether to
/// reuse the cached child view. Mixes the match positions so a row rebuilds
/// when nucleo's highlight set changes — even when mi/idx stay the same.
///
/// FNV-1a with the proper non-zero offset basis. A 0-start would collide
/// across `positions=[]` and `positions=[0]` (the empty-query case vs. a
/// first-char match) and leave highlights stuck after clearing the query.
pub(crate) fn row_key(mi: usize, idx: usize, positions: &[u32], desc_positions: &[u32]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    h ^= mi as u64;
    h = h.wrapping_mul(0x100000001b3);
    h ^= idx as u64;
    h = h.wrapping_mul(0x100000001b3);
    h ^= positions.len() as u64;
    h = h.wrapping_mul(0x100000001b3);
    for p in positions {
        h ^= *p as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h ^= desc_positions.len() as u64;
    h = h.wrapping_mul(0x100000001b3);
    for p in desc_positions {
        h ^= *p as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Linearly blend `over` onto `under` at `t` (0..=1). Opaque result — used
/// for derived bg colors (hover, etc.) on a PreMultiplied-alpha surface
/// where partial-alpha would leak the framebuffer through.
fn blend(over: Color, under: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let r = (over.r as f32 * t + under.r as f32 * (1.0 - t)) as u8;
    let g = (over.g as f32 * t + under.g as f32 * (1.0 - t)) as u8;
    let b = (over.b as f32 * t + under.b as f32 * (1.0 - t)) as u8;
    Color::rgb8(r, g, b)
}

// ─── Match-highlight helpers ─────────────────────────────────────────────────

fn highlighted_label(
    label_str: String,
    positions: Vec<u32>,
    _mi: usize,
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
                    let _ = selected_sig.get(); // track for reactive style invalidation
                    // fzf pattern: matched chars always accent, unmatched fg.
                    // Selection visual lives on the row (bg + border), not text.
                    let color = if hl { accent } else { fg };
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
    desc_positions: Vec<u32>,
    mi: usize,
    selected_sig: RwSignal<usize>,
    visual_anchor_sig: RwSignal<Option<usize>>,
    state: Arc<Mutex<AppState>>,
    fg: Color,
    accent: Color,
    muted: Color,
    selected_bg: Color,
) -> impl IntoView {
    let label_view =
        highlighted_label(entry.label.clone(), positions, mi, selected_sig, fg, accent);

    let desc_view: Box<dyn floem::View> = match &entry.description {
        Some(d) => {
            // Wrap the description in parens, then re-emit char spans with
            // the matched indices in accent. desc_positions are indices into
            // the original `d` string — shift by 1 to account for the
            // leading "(" we prepend.
            let body: String = d.clone();
            let shifted: Vec<u32> = desc_positions.iter().map(|p| p + 1).collect();
            // Compose ( + body + ) and run through highlighted_label so it
            // emits the same per-char spans the title uses (matched = accent,
            // unmatched = muted via the default color we set on the wrapper).
            let wrapped = format!("({body})");
            highlighted_label(wrapped, shifted, mi, selected_sig, muted, accent)
                .style(move |s| s.color(muted).margin_left(DESC_GAP))
                .into_any()
        }
        None => floem::views::empty().into_any(),
    };

    let click_payload = entry.payload.clone();

    let hover_bg = blend(accent, selected_bg, 0.18);
    // Visual-range bg: tinted toward accent so it reads as "selected" but
    // stays distinct from the cursor row (which keeps the deeper selected_bg
    // and the accent border).
    let visual_bg = blend(accent, selected_bg, 0.35);
    h_stack((label_view.into_any(), desc_view))
        .style(move |s| {
            let cursor_row = selected_sig.get() == mi;
            let in_visual_range = match visual_anchor_sig.get() {
                Some(a) => {
                    let sel = selected_sig.get();
                    let (lo, hi) = (a.min(sel), a.max(sel));
                    mi >= lo && mi <= hi
                }
                None => false,
            };
            // Three-state bg: cursor > visual-range > none. Cursor wins when
            // both apply so the user always sees where j/k will go next.
            let bg = if cursor_row {
                selected_bg
            } else if in_visual_range {
                visual_bg
            } else {
                Color::TRANSPARENT
            };
            let border = if cursor_row {
                accent
            } else {
                Color::TRANSPARENT
            };
            let highlighted = cursor_row || in_visual_range;
            s.width_full()
                .height(ROW_HEIGHT)
                .padding_horiz(HORIZ_PAD)
                .items_center()
                .background(bg)
                .border(BORDER_W)
                .border_color(border)
                .border_radius(ROW_RADIUS)
                .cursor(floem::style::CursorStyle::Pointer)
                // Mouse hover: bg only, no border. The accent ring is reserved
                // for keyboard / programmatic selection (j/k/arrows).
                .apply_if(!highlighted, |s| s.hover(|s| s.background(hover_bg)))
        })
        .on_click_stop(move |_| {
            selected_sig.set(mi);
            // Mirror the keyboard Accept path: bump frecency + push history.
            {
                let mut s = state.lock().unwrap();
                let cli_mode = s.cli_mode;
                s.usage.record(cli_mode, &click_payload);
                s.usage.save();
                // We don't have the query signal here, so query_sig isn't
                // tracked. The query is on s.picker.query — fine to read.
                let q = s.picker.query.get_untracked();
                s.history.push(cli_mode, &q);
                s.history.save();
            }
            if let Err(e) = crate::modes::execute(&click_payload) {
                eprintln!("pikr: execute error: {e}");
            }
            std::process::exit(0);
        })
}

// ─── Query-text helpers ──────────────────────────────────────────────────────

/// Char-index → byte-offset into `s`. Clamps past-the-end to `s.len()`.
/// All caret math runs in char indices (codepoint-aware); String operations
/// need byte offsets.
pub(crate) fn char_idx_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

/// Ctrl-W word boundary: walk back from `cur` (char index), skip trailing
/// whitespace, then skip the run of non-whitespace before it. Returns the
/// new caret position.
pub(crate) fn word_boundary_back(s: &str, cur: usize) -> usize {
    let chars: Vec<char> = s.chars().collect();
    let mut i = cur.min(chars.len());
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    while i > 0 && !chars[i - 1].is_whitespace() {
        i -= 1;
    }
    i
}

// ─── Cursor helpers ──────────────────────────────────────────────────────────

/// Vim-style cursor glyph picker. `▏` (thin caret) in Insert mode, `█` (block)
/// in Normal. When `blink_on` is false we return a regular space so the line
/// width in a monospace font stays stable across blink phases.
fn cursor_glyph(mode: VimMode, blink_on: bool) -> char {
    if !blink_on {
        return ' ';
    }
    match mode {
        VimMode::Insert => '\u{258F}', // ▏
        // Normal + Visual both render the block cursor — Visual reuses Normal's
        // motion vocabulary, the mode difference is the list-range highlight.
        VimMode::Normal | VimMode::Visual => '\u{2588}', // █
    }
}

/// Insert a vim cursor glyph at codepoint position `cursor` in `text`. The
/// cursor is clamped to `0..=text.chars().count()`. Used by both the query
/// bar (cursor moves with Left/Right/Home/End) and the ex bar (always at end).
fn with_cursor(text: &str, cursor: usize, mode: VimMode, blink_on: bool) -> String {
    let glyph = cursor_glyph(mode, blink_on);
    let total = text.chars().count();
    let pos = cursor.min(total);
    let mut out = String::with_capacity(text.len() + 4);
    for (i, ch) in text.chars().enumerate() {
        if i == pos {
            out.push(glyph);
        }
        out.push(ch);
    }
    if pos >= total {
        out.push(glyph);
    }
    out
}

// ─── Ex command bar ──────────────────────────────────────────────────────────

fn ex_bar(
    ex_buf: RwSignal<Option<String>>,
    blink_on: RwSignal<bool>,
    fg: Color,
    bg: Color,
) -> impl IntoView {
    // Always render — even when no ex command is active. Toggling the bar
    // visibility would otherwise re-flow the v_stack and bump the status
    // bar up/down each time the user pressed `:` or Esc.
    //
    // Wrap the label in an h_stack so `items_center` actually centers the
    // glyphs vertically. A bare label doesn't have flex children, so
    // `items_center` on it would be a no-op and the text sticks to the top.
    h_stack((label(move || match ex_buf.get() {
        Some(s) => {
            // Ex caret always at end of buffer for now — ex command doesn't
            // support mid-buffer editing yet.
            let line = format!(":{s}");
            let end = line.chars().count();
            with_cursor(&line, end, VimMode::Insert, blink_on.get())
        }
        None => String::new(),
    })
    .style(move |s| s.color(fg)),))
    .style(move |s| {
        s.width_full()
            .height(22.0)
            .padding_horiz(HORIZ_PAD)
            .items_center()
            .background(bg)
            .border_radius(ROW_RADIUS)
            .margin_top(8.0)
            .margin_bottom(2.0)
    })
}

// ─── Status bar ──────────────────────────────────────────────────────────────

const STATUS_BAR_HEIGHT: f64 = 22.0;
const STATUS_BAR_VPAD: f64 = 3.0;
// 0 — the ex bar above already has `margin_bottom(2)` providing the gap.
const STATUS_BAR_MARGIN_TOP: f64 = 0.0;
// 0 — the panel's PANEL_PAD (10px) already gives the same gap below the
// status bar that PANEL_PAD provides above the input row. Any additional
// margin here doubled that gap.
const STATUS_BAR_MARGIN_BOTTOM: f64 = 0.0;
/// Total vertical space the status bar occupies in the panel — content
/// height + vertical padding (×2) + top margin + bottom margin. Used by
/// app.rs window sizing.
pub const STATUS_BAR_TOTAL: f64 =
    STATUS_BAR_HEIGHT + STATUS_BAR_VPAD * 2.0 + STATUS_BAR_MARGIN_TOP + STATUS_BAR_MARGIN_BOTTOM;

#[allow(clippy::too_many_arguments)]
fn status_bar(
    vim_mode_sig: RwSignal<VimMode>,
    selected_sig: RwSignal<usize>,
    rev: RwSignal<u64>,
    state: Arc<Mutex<AppState>>,
    fg: Color,
    accent: Color,
    muted: Color,
    selected_bg: Color,
    font_family: String,
    font_size: f32,
) -> impl IntoView {
    let _ = fg;
    let status_bg = blend(muted, selected_bg, 0.15);
    let count_bg = blend(accent, selected_bg, 0.25);

    let ff_mode = font_family.clone();
    let mode_label = label(move || match vim_mode_sig.get() {
        VimMode::Insert => "INSERT".to_string(),
        VimMode::Normal => "NORMAL".to_string(),
        VimMode::Visual => "VISUAL".to_string(),
    })
    .style(move |s| {
        let bg = match vim_mode_sig.get() {
            VimMode::Insert => accent,
            VimMode::Normal => muted,
            // Visual: distinct from both — blend accent into muted for a
            // pill that signals "selection active" without screaming insert.
            VimMode::Visual => blend(accent, muted, 0.5),
        };
        s.background(bg)
            .color(selected_bg)
            .font_family(ff_mode.clone())
            .font_size(font_size)
            .padding_horiz(8.0)
            .padding_vert(2.0)
            .border_radius(4.0)
    });

    let ff_mode_name = font_family.clone();
    let state_mode = Arc::clone(&state);
    let mode_name_label = label(move || {
        let _ = rev.get();
        let s = state_mode.lock().unwrap();
        format!("{:?}", s.cli_mode).to_lowercase()
    })
    .style(move |s| {
        s.color(muted)
            .font_family(ff_mode_name.clone())
            .font_size(font_size)
            .margin_left(10.0)
    });

    let ff_count = font_family.clone();
    let state_count = Arc::clone(&state);
    let count_label = label(move || {
        let _ = rev.get();
        let sel = selected_sig.get();
        let total = state_count.lock().unwrap().matches.len();
        if total == 0 {
            "0/0".to_string()
        } else {
            format!("{}/{}", sel + 1, total)
        }
    })
    .style(move |s| {
        s.color(accent)
            .background(count_bg)
            .font_family(ff_count.clone())
            .font_size(font_size)
            .padding_horiz(8.0)
            .padding_vert(2.0)
            .border_radius(4.0)
    });

    h_stack((
        mode_label,
        mode_name_label,
        container(floem::views::empty()).style(|s| s.flex_grow(1.0)),
        count_label,
    ))
    .style(move |s| {
        s.width_full()
            .height(STATUS_BAR_HEIGHT + STATUS_BAR_VPAD * 2.0)
            .min_height(STATUS_BAR_HEIGHT + STATUS_BAR_VPAD * 2.0)
            .flex_shrink(0.0)
            .padding_horiz(HORIZ_PAD)
            .padding_vert(STATUS_BAR_VPAD)
            .items_center()
            .margin_top(STATUS_BAR_MARGIN_TOP)
            .margin_bottom(STATUS_BAR_MARGIN_BOTTOM)
            .background(status_bg)
            .border_radius(ROW_RADIUS)
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
    /// True when running as a regular OS window (no wlr-layer-shell). The
    /// OS already paints a window border / shadow / corner rounding, so we
    /// drop our own panel border to avoid double-bordering.
    pub windowed: bool,
    /// Per-mode frecency (count × half-life decay). Loaded from disk at
    /// startup; bumped on Accept and persisted then.
    pub usage: crate::picker::frecency::Usage,
    /// Per-mode query history (most-recent first). Pushed on Accept (with
    /// non-empty query), recalled via Ctrl-P/Ctrl-N in Insert mode.
    pub history: crate::picker::history::History,
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
                    desc_positions: Vec::new(),
                }];
            } else {
                self.entries = Vec::new();
                self.matches = Vec::new();
            }
            self.picker.clamp_selected(self.matches.len());
            return;
        }
        let pairs: Vec<(&str, Option<&str>)> = self
            .entries
            .iter()
            .map(|e| (e.label.as_str(), e.description.as_deref()))
            .collect();
        let mut ranked = self.matcher.rank(&pairs, &query);
        // Frecency bonus: per-entry, derived from accept history. Added to
        // the nucleo score then re-sort, so heavily-used recent payloads
        // surface above one-off matches with equal text score. Empty query
        // also benefits — that's when you want the launcher to show the
        // app you actually launch every day first.
        let now = std::time::SystemTime::now();
        for m in &mut ranked {
            let bonus = self
                .usage
                .bonus(self.cli_mode, &self.entries[m.index].payload, now);
            m.score = m.score.saturating_add(bonus);
        }
        ranked.sort_by(|a, b| b.score.cmp(&a.score).then(a.index.cmp(&b.index)));
        self.matches = ranked;
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
        self.picker.query_cursor.set(0);
        self.picker.selected.set(0);
        self.rerank();
    }
}

// ─── Picker view ─────────────────────────────────────────────────────────────

pub fn picker_view(state: Arc<Mutex<AppState>>) -> impl IntoView {
    let (
        query_sig,
        query_cursor_sig,
        selected_sig,
        vim_mode_sig,
        ex_buf_sig,
        count_sig,
        visual_anchor_sig,
    ) = {
        let s = state.lock().unwrap();
        (
            s.picker.query,
            s.picker.query_cursor,
            s.picker.selected,
            s.picker.vim_mode,
            s.picker.ex_buf,
            s.picker.count,
            s.picker.visual_anchor,
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
    let windowed = state.lock().unwrap().windowed;

    let rev: RwSignal<u64> = RwSignal::new(0);

    // History-recall state: `history_cursor` is the index into the per-mode
    // history list (None = not recalling, just typing live). `history_draft`
    // stashes the live query when the user first hits Ctrl-P so Ctrl-N can
    // restore it once they walk back past the most-recent entry.
    let history_cursor: RwSignal<Option<usize>> = RwSignal::new(None);
    let history_draft: RwSignal<String> = RwSignal::new(String::new());

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

    // Hand-rolled input: a label that renders the query plus a vim-style
    // cursor glyph (thin in Insert, block in Normal). All editing flows
    // through the outer keydown handler — InsertChar / Backspace mutate
    // query_sig directly. Avoids floem text_input's focus-stealing Esc.
    let ff_q = font_family.clone();
    let blink_on: RwSignal<bool> = RwSignal::new(true);
    {
        let (tx, rx) = crossbeam_channel::unbounded::<()>();
        let tick_sig = create_signal_from_channel(rx);
        create_effect(move |_| {
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
    // Keep cursor visible on every keystroke so the caret never hides mid-type.
    // Both query, query_cursor, and ex_buf mutations should reset the blink phase.
    create_effect(move |_| {
        let _ = query_sig.get();
        let _ = query_cursor_sig.get();
        let _ = ex_buf_sig.get();
        blink_on.set(true);
    });
    let query_label = label(move || {
        with_cursor(
            &query_sig.get(),
            query_cursor_sig.get(),
            vim_mode_sig.get(),
            blink_on.get(),
        )
    })
    .style(move |s| {
        s.color(fg)
            .font_family(ff_q.clone())
            .font_size(input_font_size)
            .flex_grow(1.0)
            .margin_left(8.0)
    });

    // Rerank whenever the query mutates. Bump `rev` so the dyn_stack rebuilds
    // against the new match list.
    let state_rerank = Arc::clone(&state);
    create_effect(move |prev: Option<String>| {
        let cur = query_sig.get();
        if prev.as_deref() != Some(cur.as_str()) {
            // `rerank()` calls `picker.clamp_selected` which `selected.set(0)`s
            // when the match list is empty. floem fires subscribers (status
            // bar count, virtual_stack data fn, empty-state, …) synchronously
            // inside `set` — while we still hold the AppState mutex —
            // and those subscribers each `state.lock()` themselves. Std
            // Mutex isn't re-entrant, so the second lock hangs forever (the
            // "no-results hang"). `batch` queues subscriber effects until
            // the closure returns; by then the mutex is dropped.
            batch(|| {
                {
                    let mut s = state_rerank.lock().unwrap();
                    s.rerank();
                }
                rev.update(|r| *r += 1);
            });
        }
        cur
    });

    let hover_bg = blend(accent, selected_bg, 0.18);
    let input_row = h_stack((prompt_label, query_label)).style(move |s| {
        s.width_full()
            .height(INPUT_ROW_HEIGHT)
            .min_height(INPUT_ROW_HEIGHT)
            .flex_shrink(0.0)
            .padding_horiz(HORIZ_PAD)
            .items_center()
            .background(selected_bg)
            .border_radius(ROW_RADIUS)
            .margin_bottom(8.0)
            .hover(|s| s.background(hover_bg))
    });

    // ── Result list ────────────────────────────────────────────────────────
    // Virtualised: floem only builds row views inside the scroll viewport,
    // so a 1800-entry emoji list paints instantly instead of stalling vger
    // glyph-atlas uploads for hundreds of multi-byte unicode rows.
    let state_list = Arc::clone(&state);
    let state_row = Arc::clone(&state);
    let ff_list = font_family.clone();
    let result_list = virtual_stack(
        VirtualDirection::Vertical,
        VirtualItemSize::Fixed(Box::new(|| ROW_PITCH)),
        move || {
            let _r = rev.get();
            let s = state_list.lock().unwrap();
            // im::Vector clones in O(1) — virtual_stack's VirtualVector trait
            // is implemented for it; Vec is not.
            s.matches
                .iter()
                .enumerate()
                .map(|(mi, m)| {
                    let entry = Arc::clone(&s.entries[m.index]);
                    (
                        mi,
                        m.index,
                        entry,
                        m.positions.clone(),
                        m.desc_positions.clone(),
                    )
                })
                .collect::<im::Vector<_>>()
        },
        |(mi, idx, _, positions, desc_positions)| row_key(*mi, *idx, positions, desc_positions),
        move |(mi, _idx, entry, positions, desc_positions)| {
            let ff = ff_list.clone();
            entry_row(
                entry,
                positions,
                desc_positions,
                mi,
                selected_sig,
                visual_anchor_sig,
                Arc::clone(&state_row),
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

    // Empty-state hint shown inside the scroll viewport when there are zero
    // matches AND the user has typed something. Sits as a v_stack sibling
    // beneath the virtual_stack and toggles its display so it doesn't take
    // any space when results are present.
    let state_empty = Arc::clone(&state);
    let state_empty_style = Arc::clone(&state);
    let ff_empty = font_family.clone();
    let empty_msg = h_stack((label(move || {
        let _ = rev.get();
        let s = state_empty.lock().unwrap();
        if s.picker.query.get().is_empty() {
            "No entries.".to_string()
        } else {
            format!("No results for \u{201C}{}\u{201D}", s.picker.query.get())
        }
    })
    .style(move |s| {
        s.color(muted)
            .font_family(ff_empty.clone())
            .font_size(font_size)
    }),))
    .style(move |s| {
        let _ = rev.get();
        let visible = state_empty_style.lock().unwrap().matches.is_empty();
        s.width_full()
            .height(ROW_HEIGHT)
            .padding_horiz(HORIZ_PAD)
            .items_center()
            .apply_if(!visible, |s| s.display(floem::style::Display::None))
    });

    let result_area = v_stack((result_list, empty_msg))
        .style(|s| s.width_full().flex_direction(FlexDirection::Column));

    let state_ensure = Arc::clone(&state);
    let scrollable = scroll(result_area)
        .ensure_visible(move || {
            let _r = rev.get(); // re-evaluate whenever the match list churns
            // Empty list: returning a non-zero rect for a row that doesn't
            // exist made scroll re-adjust every frame and locked up the UI
            // thread on no-match queries (e.g. emoji + nonsense input).
            if state_ensure.lock().unwrap().matches.is_empty() {
                return Rect::ZERO;
            }
            let sel = selected_sig.get() as f64;
            let start_row = (sel - SCROLLOFF).max(0.0);
            let end_row = sel + 1.0 + SCROLLOFF;
            let top = start_row * ROW_PITCH;
            let height = (end_row - start_row) * ROW_PITCH;
            Rect::from_origin_size((0.0, top), KurboSize::new(1.0, height))
        })
        .style(move |s| {
            // min_height(0) lets the flex container shrink the scrollable
            // below its intrinsic content height. Without it, flex respects
            // the scroll's natural size (= every row stacked) and pushes the
            // ex / status bars off the bottom of the panel.
            s.width_full()
                .flex_grow(1.0)
                .flex_basis(0.0)
                .min_height(0.0)
                .class(floem::views::scroll::Handle, move |h| {
                    h.background(muted).border_radius(3.0)
                })
        });

    let ex_bg = blend(muted, selected_bg, 0.15);
    let ex = ex_bar(ex_buf_sig, blink_on, fg, ex_bg);
    let status = status_bar(
        vim_mode_sig,
        selected_sig,
        rev,
        Arc::clone(&state),
        fg,
        accent,
        muted,
        selected_bg,
        font_family.clone(),
        font_size,
    );

    // ── Outer panel ────────────────────────────────────────────────────────
    // Two layers: an opaque non-rounded outer fill (so the framebuffer
    // transparency doesn't leak outside the rounded corners or border ring),
    // and the rounded inner panel that holds the actual content.
    let state_key = Arc::clone(&state);
    container(
        container(
            v_stack((input_row, scrollable, ex.into_any(), status.into_any())).style(move |s| {
                s.width_full()
                    .height_full()
                    .padding_top(PANEL_PAD)
                    .padding_horiz(PANEL_PAD)
            }),
        )
        .style(move |s| {
            let s = s.width_full().height_full().background(bg);
            if windowed {
                // OS window paints its own border/shadow/rounding; skip ours.
                s
            } else {
                s.border(BORDER_W)
                    .border_color(accent)
                    .border_radius(PANEL_RADIUS)
            }
        }),
    )
    .style(move |s| s.width_full().height_full().background(bg))
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
                    if buf.is_empty() {
                        // Backspace on an empty ex prompt dismisses it (same
                        // affordance as readline / vim).
                        ex_buf_sig.set(None);
                    } else {
                        buf.pop();
                        ex_buf_sig.set(Some(buf));
                    }
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
            Action::EnterInsert => {
                visual_anchor_sig.set(None);
                vim_mode_sig.set(VimMode::Insert);
            }
            Action::EnterNormal => {
                visual_anchor_sig.set(None);
                vim_mode_sig.set(VimMode::Normal);
            }
            Action::EnterVisual => {
                // Anchor at current cursor; range = [anchor, selected]
                // computed at render time.
                visual_anchor_sig.set(Some(selected_sig.get()));
                vim_mode_sig.set(VimMode::Visual);
            }
            Action::StartSearch => vim_mode_sig.set(VimMode::Insert),
            Action::StartEx => ex_buf_sig.set(Some(String::new())),
            Action::Accept => {
                let sel = selected_sig.get();
                // Visual: execute every entry in the anchored range.
                // Normal/Insert: just the cursor row.
                let (payloads, cli_mode) = {
                    let mut s = state_key.lock().unwrap();
                    let anchor = visual_anchor_sig.get_untracked();
                    let range: Vec<usize> = match (vim_mode_sig.get_untracked(), anchor) {
                        (VimMode::Visual, Some(a)) => {
                            let (lo, hi) = (a.min(sel), a.max(sel));
                            (lo..=hi).collect()
                        }
                        _ => vec![sel],
                    };
                    let payloads: Vec<modes::Payload> = range
                        .into_iter()
                        .filter_map(|mi| s.matches.get(mi))
                        .map(|m| s.entries[m.index].payload.clone())
                        .collect();
                    // Frecency: bump count + last_used for every accepted
                    // payload, then persist once. Saving here (rather than
                    // in record() per entry) keeps a Visual-mode multi-launch
                    // to a single fsync.
                    let cli_mode = s.cli_mode;
                    for payload in &payloads {
                        s.usage.record(cli_mode, payload);
                    }
                    if !payloads.is_empty() {
                        s.usage.save();
                    }
                    // History: record the live query for Ctrl-P/Ctrl-N recall.
                    // Push the buffer the user actually accepted with — not
                    // whatever they might still be recalling — so reusing a
                    // recalled query just dedupes to the front of history.
                    let query_text = query_sig.get_untracked();
                    if !payloads.is_empty() {
                        s.history.push(cli_mode, &query_text);
                        s.history.save();
                    }
                    (payloads, cli_mode)
                };
                let _ = cli_mode;
                for payload in &payloads {
                    if let Err(e) = modes::execute(payload) {
                        eprintln!("pikr: execute error: {e}");
                    }
                }
                if !payloads.is_empty() {
                    std::process::exit(0);
                }
            }
            Action::Cancel => std::process::exit(0),
            Action::InsertChar(c) => {
                // Any user-initiated edit exits history recall — the buffer
                // is no longer a verbatim past query.
                history_cursor.set(None);
                let cur = query_cursor_sig.get();
                query_sig.update(|q| {
                    let byte_idx = char_idx_to_byte(q, cur);
                    q.insert(byte_idx, c);
                });
                query_cursor_sig.set(cur + 1);
            }
            Action::Backspace => {
                history_cursor.set(None);
                let cur = query_cursor_sig.get();
                if cur > 0 {
                    query_sig.update(|q| {
                        let start = char_idx_to_byte(q, cur - 1);
                        let end = char_idx_to_byte(q, cur);
                        q.replace_range(start..end, "");
                    });
                    query_cursor_sig.set(cur - 1);
                }
            }
            Action::DeleteForward => {
                history_cursor.set(None);
                let cur = query_cursor_sig.get();
                query_sig.update(|q| {
                    let total = q.chars().count();
                    if cur < total {
                        let start = char_idx_to_byte(q, cur);
                        let end = char_idx_to_byte(q, cur + 1);
                        q.replace_range(start..end, "");
                    }
                });
            }
            Action::CursorLeft => {
                let cur = query_cursor_sig.get();
                if cur > 0 {
                    query_cursor_sig.set(cur - 1);
                }
            }
            Action::CursorRight => {
                let cur = query_cursor_sig.get();
                let total = query_sig.get().chars().count();
                if cur < total {
                    query_cursor_sig.set(cur + 1);
                }
            }
            Action::CursorHome => query_cursor_sig.set(0),
            Action::CursorEnd => {
                let total = query_sig.get().chars().count();
                query_cursor_sig.set(total);
            }
            Action::DeleteWordBack => {
                history_cursor.set(None);
                let cur = query_cursor_sig.get();
                if cur > 0 {
                    query_sig.update(|q| {
                        let new_cur = word_boundary_back(q, cur);
                        let start = char_idx_to_byte(q, new_cur);
                        let end = char_idx_to_byte(q, cur);
                        q.replace_range(start..end, "");
                        query_cursor_sig.set(new_cur);
                    });
                }
            }
            Action::DeleteToLineStart => {
                history_cursor.set(None);
                let cur = query_cursor_sig.get();
                if cur > 0 {
                    query_sig.update(|q| {
                        let end = char_idx_to_byte(q, cur);
                        q.replace_range(0..end, "");
                    });
                    query_cursor_sig.set(0);
                }
            }
            Action::HistoryPrev => {
                let s = state_key.lock().unwrap();
                let cli_mode = s.cli_mode;
                let total = s.history.len(cli_mode);
                if total == 0 {
                    drop(s);
                } else {
                    let next_idx = match history_cursor.get_untracked() {
                        None => {
                            // First hop into history — stash the current
                            // draft so Ctrl-N can restore it later.
                            history_draft.set(query_sig.get_untracked());
                            0
                        }
                        Some(cur) => (cur + 1).min(total - 1),
                    };
                    if let Some(entry) = s.history.get(cli_mode, next_idx) {
                        let text = entry.to_string();
                        drop(s);
                        history_cursor.set(Some(next_idx));
                        let len = text.chars().count();
                        query_sig.set(text);
                        query_cursor_sig.set(len);
                    } else {
                        drop(s);
                    }
                }
            }
            Action::HistoryNext => {
                // Only meaningful while we're already recalling.
                if let Some(cur) = history_cursor.get_untracked() {
                    if cur == 0 {
                        // At the most-recent entry; the next step is back
                        // to the live draft.
                        let text = history_draft.get_untracked();
                        history_cursor.set(None);
                        let len = text.chars().count();
                        query_sig.set(text);
                        query_cursor_sig.set(len);
                    } else {
                        let next_idx = cur - 1;
                        let s = state_key.lock().unwrap();
                        let cli_mode = s.cli_mode;
                        if let Some(entry) = s.history.get(cli_mode, next_idx) {
                            let text = entry.to_string();
                            drop(s);
                            history_cursor.set(Some(next_idx));
                            let len = text.chars().count();
                            query_sig.set(text);
                            query_cursor_sig.set(len);
                        }
                    }
                }
            }
        }

        let _ = after;
        rev.update(|r| *r += 1);
        EventPropagation::Stop
    })
}

#[cfg(test)]
mod tests {
    use super::{char_idx_to_byte, row_key, word_boundary_back};

    /// Regression for the empty-query → first-char highlight stuck bug.
    /// Empty match `positions=[]` MUST hash differently from `positions=[0]`,
    /// otherwise dyn_stack reuses the cached row view and the old "first
    /// letter highlighted" rendering survives a query clear.
    #[test]
    fn row_key_distinguishes_empty_from_first_char_match() {
        let empty = row_key(0, 0, &[], &[]);
        let first = row_key(0, 0, &[0], &[]);
        assert_ne!(
            empty, first,
            "[] and [0] must hash differently or highlights stick after clear"
        );
    }

    /// Regression for the "typing au → auda keeps mi/idx but positions grow"
    /// case. Each prefix of a growing match must yield a fresh key so the
    /// span list rebuilds.
    #[test]
    fn row_key_changes_as_positions_grow() {
        let k1 = row_key(0, 5, &[0], &[]);
        let k2 = row_key(0, 5, &[0, 1], &[]);
        let k3 = row_key(0, 5, &[0, 1, 2], &[]);
        let k4 = row_key(0, 5, &[0, 1, 2, 3], &[]);
        assert_ne!(k1, k2);
        assert_ne!(k2, k3);
        assert_ne!(k3, k4);
        assert_ne!(k1, k4);
    }

    #[test]
    fn row_key_distinguishes_same_len_different_positions() {
        assert_ne!(row_key(0, 0, &[0, 3], &[]), row_key(0, 0, &[0, 4], &[]));
        assert_ne!(row_key(0, 0, &[1, 2], &[]), row_key(0, 0, &[2, 1], &[]));
    }

    #[test]
    fn row_key_distinguishes_mi_and_idx() {
        assert_ne!(row_key(0, 0, &[0], &[]), row_key(1, 0, &[0], &[]));
        assert_ne!(row_key(0, 0, &[0], &[]), row_key(0, 1, &[0], &[]));
    }

    #[test]
    fn row_key_is_deterministic() {
        assert_eq!(
            row_key(2, 7, &[0, 1, 2], &[3, 4]),
            row_key(2, 7, &[0, 1, 2], &[3, 4])
        );
    }

    /// Description positions must also invalidate the cache — typing across
    /// a description-only match has the same staleness risk as a label match.
    #[test]
    fn row_key_distinguishes_desc_positions() {
        assert_ne!(row_key(0, 0, &[], &[]), row_key(0, 0, &[], &[0]));
        assert_ne!(row_key(0, 0, &[0], &[0]), row_key(0, 0, &[0], &[1]));
    }

    #[test]
    fn char_idx_to_byte_ascii() {
        let s = "hello";
        assert_eq!(char_idx_to_byte(s, 0), 0);
        assert_eq!(char_idx_to_byte(s, 3), 3);
        assert_eq!(char_idx_to_byte(s, 5), 5); // past-the-end
        assert_eq!(char_idx_to_byte(s, 99), 5); // clamps
    }

    #[test]
    fn char_idx_to_byte_multibyte() {
        // "héllo" — é is 2 bytes in UTF-8.
        let s = "héllo";
        assert_eq!(char_idx_to_byte(s, 0), 0);
        assert_eq!(char_idx_to_byte(s, 1), 1); // before é
        assert_eq!(char_idx_to_byte(s, 2), 3); // after é (skips 2 bytes)
        assert_eq!(char_idx_to_byte(s, 5), 6);
    }

    #[test]
    fn word_boundary_back_skips_trailing_space() {
        // cursor at end of "foo bar  " — Ctrl-W should land at end of "foo "
        // (skip trailing ws, then skip "bar").
        let s = "foo bar  ";
        assert_eq!(word_boundary_back(s, 9), 4);
    }

    #[test]
    fn word_boundary_back_inside_word() {
        // cursor at "fo|o bar" → Ctrl-W deletes "fo", lands at 0.
        let s = "foo bar";
        assert_eq!(word_boundary_back(s, 2), 0);
    }

    #[test]
    fn word_boundary_back_at_start_noop() {
        let s = "foo";
        assert_eq!(word_boundary_back(s, 0), 0);
    }

    #[test]
    fn word_boundary_back_only_whitespace() {
        // " | " → walks back past all whitespace to 0.
        let s = "   ";
        assert_eq!(word_boundary_back(s, 3), 0);
    }
}
