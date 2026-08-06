//! Floem view tree for pikr — tokyonight/rofi-style layout.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use floem::ui_events::keyboard::{Key, KeyboardEvent, NamedKey};
use floem::{
    IntoView, ViewId,
    event::listener::{
        Click, KeyDown, UpdatePhasePaintPresent, WindowGainedFocus, WindowLostFocus,
    },
    event::{EventCx, EventPropagation},
    kurbo::{Rect, Size as KurboSize},
    peniko::Color,
    reactive::{Effect, RwSignal, SignalGet, SignalUpdate},
    receiver_signal::ChannelSignal,
    style::FlexDirection,
    text::{Attrs, AttrsList, FamilyOwned},
    views::{Container, Decorators, Empty, Label, Stack, dyn_view, img, rich_text, virtual_stack},
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
/// Margin between the input row and the result list. Drives chrome_h in app.rs.
pub const INPUT_MARGIN_BOTTOM: f64 = 8.0;
pub const STATUS_HEIGHT: f64 = 0.0;
const SCROLLOFF: f64 = 1.0;
pub const PANEL_PAD: f64 = 10.0;
const HORIZ_PAD: f64 = 14.0;
const DESC_GAP: f64 = 10.0;
const PANEL_RADIUS: f64 = 10.0;
const ROW_RADIUS: f64 = 6.0;
const BORDER_W: f64 = 1.5;
/// Rendered icon size (px). Source icons are resolved at 32 px so they
/// downscale cleanly to this render target.
const ICON_SIZE: f64 = 24.0;
/// Gap between the icon slot and the label text.
const ICON_GAP: f64 = 8.0;

// ─── Colour helpers ──────────────────────────────────────────────────────────

fn parse_color(hex: &str) -> Color {
    let s = hex.trim_start_matches('#');
    // Strict: exactly six hex digits after the optional `#`. Short hex,
    // 8-digit #AARRGGBB, or garbage all yield deterministic black — the
    // theme loader has no error channel, so a wrong config value must not
    // silently render some other colour.
    let rgb = match s.len() {
        6 => u32::from_str_radix(s, 16).unwrap_or(0x00_00_00),
        _ => 0x00_00_00,
    };
    let r = ((rgb >> 16) & 0xff) as u8;
    let g = ((rgb >> 8) & 0xff) as u8;
    let b = (rgb & 0xff) as u8;
    Color::from_rgb8(r, g, b)
}

/// Stable key for a result-list row, used by dyn_stack to decide whether to
/// reuse the cached child view. Mixes the match positions so a row rebuilds
/// when nucleo's highlight set changes — even when mi/idx stay the same.
///
/// `entry_id` is the Arc pointer of the row's `Entry`. Modes whose entry list
/// is stable across reranks (drun/emoji/…) keep the same Arc → same ptr →
/// cache stays warm. Calc replaces its single entry with a fresh Arc on every
/// rerank, so the ptr changes and the row view is rebuilt — otherwise the row
/// shows the previously-typed query's `"<expr> = <result>"` label.
///
/// FNV-1a with the proper non-zero offset basis. A 0-start would collide
/// across `positions=[]` and `positions=[0]` (the empty-query case vs. a
/// first-char match) and leave highlights stuck after clearing the query.
pub(crate) fn row_key(
    mi: usize,
    idx: usize,
    entry_id: u64,
    positions: &[u32],
    desc_positions: &[u32],
) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    h ^= mi as u64;
    h = h.wrapping_mul(0x100000001b3);
    h ^= idx as u64;
    h = h.wrapping_mul(0x100000001b3);
    h ^= entry_id;
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
    let o = over.to_rgba8();
    let u = under.to_rgba8();
    let r = (o.r as f32 * t + u.r as f32 * (1.0 - t)) as u8;
    let g = (o.g as f32 * t + u.g as f32 * (1.0 - t)) as u8;
    let b = (o.b as f32 * t + u.b as f32 * (1.0 - t)) as u8;
    Color::from_rgb8(r, g, b)
}

// ─── Match-highlight helpers ─────────────────────────────────────────────────

/// Render `label_str` with the `positions` codepoints painted in `accent`
/// and the rest in `fg`. The whole string lives in a single floem
/// [`rich_text`] so cosmic-text shapes the run once and per-glyph
/// advance widths stay stable as `positions` changes — previously each
/// run was a separate `label()` inside a flex row, and per-label
/// shaping introduced visible horizontal jitter when characters
/// switched between matched and unmatched as the user typed.
///
/// `families` (parsed once per session) and `font_size` are baked into the
/// `Attrs` because `rich_text` does not inherit them from the parent style
/// cascade (unlike `label`, which reads its font off `self.font.*`).
fn highlighted_label(
    label_str: String,
    positions: &[u32],
    fg: Color,
    accent: Color,
    families: std::sync::Arc<Vec<FamilyOwned>>,
    font_size: f32,
) -> impl IntoView {
    // Collapse adjacent same-color runs to keep the AttrsList compact.
    let pos_set: std::collections::HashSet<u32> = positions.iter().copied().collect();
    let mut runs: Vec<(std::ops::Range<usize>, Color)> = Vec::new();
    let mut byte = 0usize;
    for (i, c) in label_str.chars().enumerate() {
        let next = byte + c.len_utf8();
        let color = if pos_set.contains(&(i as u32)) {
            accent
        } else {
            fg
        };
        match runs.last_mut() {
            Some(last) if last.1 == color => last.0.end = next,
            _ => runs.push((byte..next, color)),
        }
        byte = next;
    }

    let compute = move || {
        let fam = families.clone();
        let default_attrs = Attrs::new().color(fg).family(&fam).font_size(font_size);
        let mut attrs_list = AttrsList::new(default_attrs);
        for (range, color) in &runs {
            let span_attrs = Attrs::new().color(*color).family(&fam).font_size(font_size);
            attrs_list.add_span(range.clone(), span_attrs);
        }
        (label_str.clone(), attrs_list)
    };
    let (initial_text, initial_attrs) = compute();
    rich_text(initial_text, initial_attrs, compute)
}

// ─── Entry-row view ──────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn entry_row(
    entry: Arc<Entry>,
    positions: Rc<Vec<u32>>,
    desc_positions: Rc<Vec<u32>>,
    mi: usize,
    selected_sig: RwSignal<usize>,
    visual_anchor_sig: RwSignal<Option<usize>>,
    state: Arc<Mutex<AppState>>,
    fg: Color,
    accent: Color,
    muted: Color,
    selected_bg: Color,
    families: std::sync::Arc<Vec<FamilyOwned>>,
    font_size: f32,
    sheet: Arc<hjkl_css::Stylesheet>,
) -> impl IntoView {
    // ── Icon view ─────────────────────────────────────────────────────────
    let sheet_icon = Arc::clone(&sheet);
    let icon_style = move |s: floem::style::Style| {
        // CSS owns width/height/margin. min_width/min_height/flex_shrink have
        // no CSS analogues in the pikr adapter yet — stay inline.
        crate::ui::css::apply(s, &sheet_icon, "div", &["icon"])
            .min_width(ICON_SIZE)
            .min_height(ICON_SIZE)
            .flex_shrink(0.0_f32)
    };
    // Resolve the entry's icon, falling back through a chain of
    // conventional generic-app icon names so drun entries with a missing
    // or unresolvable `Icon=` still render the slot. Picker rows always
    // reserve the slot whether or not anything ends up in it — labels
    // stay aligned across the column.
    //
    // For SVGs we go through `IconCache::rasterise_svg` (resvg → PNG)
    // because floem's `svg()` runs the file through vello, which silently
    // drops unsupported features (markers, complex clip-paths) and
    // commonly renders only the icon's solid background plate. resvg
    // handles full SVG 1.1, so the rasterised PNG fed through `img()`
    // produces a correct bitmap.
    let resolved = state
        .lock()
        .unwrap()
        .icons
        .lock()
        .unwrap()
        .resolve_or_fallback(
            entry.icon.as_deref(),
            crate::picker::icons::GENERIC_FALLBACK_NAMES,
        );
    let icon_view: Box<dyn floem::View> = match resolved {
        Some(ref path) if path.extension().and_then(|s| s.to_str()) == Some("svg") => {
            // Rasterise at 48 px (2× display) so downscaling stays sharp.
            // The cache holds the rendered PNG bytes per path.
            let bytes = state
                .lock()
                .unwrap()
                .icons
                .lock()
                .unwrap()
                .rasterise_svg(path, 48);
            match bytes {
                Some(arc) => img(move || (*arc).clone()).style(icon_style).into_any(),
                None => Empty::new().style(icon_style).into_any(),
            }
        }
        Some(ref path) => {
            // Raster: PNG / JPEG. Bytes are cached per path in IconCache so
            // the virtual_stack row rebuilds don't re-read the file from disk
            // every frame (floem's `img()` sniffs the format from magic
            // bytes).
            let bytes = state.lock().unwrap().icons.lock().unwrap().file_bytes(path);
            match bytes {
                Some(arc) => img(move || (*arc).clone()).style(icon_style).into_any(),
                None => Empty::new().style(icon_style).into_any(),
            }
        }
        None => Empty::new().style(icon_style).into_any(),
    };

    let label_view = highlighted_label(
        entry.label.clone(),
        &positions,
        fg,
        accent,
        families.clone(),
        font_size,
    );

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
            let sheet_desc = Arc::clone(&sheet);
            highlighted_label(
                wrapped,
                &shifted,
                muted,
                accent,
                families.clone(),
                font_size,
            )
            .style(move |s| crate::ui::css::apply(s, &sheet_desc, "div", &["desc"]))
            .into_any()
        }
        None => Empty::new().into_any(),
    };

    let click_payload = entry.payload.clone();

    let hover_bg = blend(accent, selected_bg, 0.18);
    // Visual-range bg: tinted toward accent so it reads as "selected" but
    // stays distinct from the cursor row (which keeps the deeper selected_bg
    // and the accent border).
    let visual_bg = blend(accent, selected_bg, 0.35);
    let sheet_row = Arc::clone(&sheet);
    Stack::horizontal((icon_view, label_view.into_any(), desc_view))
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
            // CSS owns width/height/padding/border-radius/align-items.
            // Reactive bg, border width+color, cursor, and hover stay inline.
            crate::ui::css::apply(s, &sheet_row, "stack", &["row"])
                .background(bg)
                .border(BORDER_W)
                .border_color(border)
                .cursor(floem::style::CursorStyle::Pointer)
                // Mouse hover: bg only, no border. The accent ring is reserved
                // for keyboard / programmatic selection (j/k/arrows).
                .apply_if(!highlighted, |s| s.hover(|s| s.background(hover_bg)))
        })
        .on_event_stop(Click, move |_cx: &mut EventCx, _ev: &()| {
            selected_sig.set(mi);
            // Mirror the keyboard Accept path: bump frecency + push history.
            {
                let mut s = state.lock().unwrap();
                // -P: sensitive input — never persisted.
                if !s.password {
                    let cli_mode = s.cli_mode;
                    s.usage.record(cli_mode, &click_payload);
                    s.usage.save();
                    // We don't have the query signal here, so query_sig isn't
                    // tracked. The query is on s.picker.query — fine to read.
                    let q = s.picker.query.get_untracked();
                    s.history.push(cli_mode, &q);
                    s.history.save();
                }
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

/// Render `text` with a vim cursor at codepoint position `cursor` (clamped to
/// `0..=text.chars().count()`).
///
/// - **Insert** (thin bar `▏`): the caret sits BETWEEN characters — it's
///   inserted at `pos` and does not cover a glyph.
/// - **Normal / Visual** (block `█`): the block sits ON the character at `pos`,
///   replacing it (vim-faithful) rather than being inserted before it. Inserting
///   before grew the line and pushed the covered char one cell right, so the
///   block rendered in the wrong spot. On the blink-off phase the underlying
///   character is revealed instead of the block, so it never vanishes or shifts.
///
/// A caret at or past the end of `text` (and the empty-query case) appends the
/// glyph. Used by both the query bar and the ex bar.
fn with_cursor(text: &str, cursor: usize, mode: VimMode, blink_on: bool) -> String {
    let glyph = cursor_glyph(mode, blink_on);
    let total = text.chars().count();
    let pos = cursor.min(total);
    let block = matches!(mode, VimMode::Normal | VimMode::Visual);
    let mut out = String::with_capacity(text.len() + 4);
    for (i, ch) in text.chars().enumerate() {
        if i == pos && block {
            // Block cursor covers this char: show the block (blink on) or the
            // char itself (blink off). Either way the following text is not
            // displaced.
            out.push(if blink_on { glyph } else { ch });
            continue;
        }
        if i == pos {
            // Insert caret: thin bar between chars (space keeps width on blink-off).
            out.push(glyph);
        }
        out.push(ch);
    }
    if pos >= total {
        out.push(glyph);
    }
    out
}

/// Replace every character in `text` with ● (U+25CF) when `enabled` is true.
/// The returned `String` has the same codepoint count as `text` so cursor
/// positioning stays correct.
pub(crate) fn mask_password(enabled: bool, text: &str) -> String {
    if enabled {
        "\u{25CF}".repeat(text.chars().count())
    } else {
        text.to_owned()
    }
}

// ─── Message modal view ───────────────────────────────────────────────────────

/// Non-interactive message overlay (rofi `--message` parity, issue #15).
///
/// Renders `text` inside the same panel chrome as `picker_view` — same bg,
/// border, radius, and padding. No input row, no result list, no status bar.
/// Pressing Escape dismisses via `std::process::exit(0)`.
pub fn message_view(text: String, sheet: Arc<hjkl_css::Stylesheet>) -> impl IntoView {
    // Same focus story as picker_view: floem routes key events only to the
    // focused view, and the Esc registry-fallback alone is racy — if the
    // compositor grants keyboard focus before the view tree settles, the
    // keystroke lands nowhere and the modal never dismisses. Claim view
    // focus the moment the window gains it.
    let root_id = ViewId::new();
    let sheet_text = Arc::clone(&sheet);
    let sheet_modal = Arc::clone(&sheet);
    let sheet_outer = Arc::clone(&sheet);

    let msg_label = Label::derived(move || text.clone())
        .style(move |s| crate::ui::css::apply(s, &sheet_text, "label", &["message-text"]));

    Container::with_id(
        root_id,
        Container::new(msg_label).style(move |s| {
            crate::ui::css::apply(s, &sheet_modal, "container", &["message-modal"])
        }),
    )
    .style(move |s| crate::ui::css::apply(s, &sheet_outer, "container", &["message-modal-outer"]))
    .style(|s| s.keyboard_navigable())
    .on_event(WindowGainedFocus, move |_cx: &mut EventCx, _ev: &()| {
        root_id.request_focus();
        EventPropagation::Continue
    })
    .on_event(
        KeyDown,
        move |_cx: &mut EventCx, kb_event: &KeyboardEvent| {
            if matches!(kb_event.key, Key::Named(NamedKey::Escape)) {
                std::process::exit(0);
            }
            EventPropagation::Stop
        },
    )
}

// ─── Ex command bar ──────────────────────────────────────────────────────────

fn ex_bar(
    ex_buf: RwSignal<Option<String>>,
    blink_on: RwSignal<bool>,
    fg: Color,
    sheet: Arc<hjkl_css::Stylesheet>,
) -> impl IntoView {
    // Always render — even when no ex command is active. Toggling the bar
    // visibility would otherwise re-flow the v_stack and bump the status
    // bar up/down each time the user pressed `:` or Esc.
    //
    // Wrap the label in an h_stack so `items_center` actually centers the
    // glyphs vertically. A bare label doesn't have flex children, so
    // `items_center` on it would be a no-op and the text sticks to the top.
    Stack::horizontal((Label::derived(move || match ex_buf.get() {
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
    .style(move |s| crate::ui::css::apply(s, &sheet, "stack", &["ex-bar"]))
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

/// Total vertical space the ex bar occupies. height(22) + margin_top(8) +
/// margin_bottom(2) — keep in sync with `fn ex_bar` if those change.
pub const EX_BAR_TOTAL: f64 = 22.0 + 8.0 + 2.0;

#[allow(clippy::too_many_arguments)]
fn status_bar(
    vim_mode_sig: RwSignal<VimMode>,
    selected_sig: RwSignal<usize>,
    rev: RwSignal<u64>,
    state: Arc<Mutex<AppState>>,
    fg: Color,
    accent: Color,
    muted: Color,
    _selected_bg: Color,
    _font_family: String,
    _font_size: f32,
    sheet: Arc<hjkl_css::Stylesheet>,
) -> impl IntoView {
    let _ = fg;

    let sheet_chip = Arc::clone(&sheet);
    let mode_label = Label::derived(move || match vim_mode_sig.get() {
        VimMode::Insert => "INSERT".to_string(),
        VimMode::Normal => "NORMAL".to_string(),
        VimMode::Visual => "VISUAL".to_string(),
    })
    .style(move |s| {
        // Mode chip's background is reactive on `vim_mode_sig` — stays inline.
        // CSS owns color, font, padding, border-radius.
        let bg = match vim_mode_sig.get() {
            VimMode::Insert => accent,
            VimMode::Normal => muted,
            // Visual: distinct from both — blend accent into muted for a
            // pill that signals "selection active" without screaming insert.
            VimMode::Visual => blend(accent, muted, 0.5),
        };
        crate::ui::css::apply(s, &sheet_chip, "label", &["mode-chip"]).background(bg)
    });

    let state_mode = Arc::clone(&state);
    let sheet_mode_name = Arc::clone(&sheet);
    let mode_name_label = Label::derived(move || {
        let _ = rev.get();
        let s = state_mode.lock().unwrap();
        format!("{:?}", s.cli_mode).to_lowercase()
    })
    .style(move |s| {
        crate::ui::css::apply(s, &sheet_mode_name, "label", &["mode-name"]).margin_left(10.0)
    });

    let state_count = Arc::clone(&state);
    let sheet_count = Arc::clone(&sheet);
    let count_label = Label::derived(move || {
        let _ = rev.get();
        let sel = selected_sig.get();
        let total = state_count.lock().unwrap().matches.len();
        if total == 0 {
            "0/0".to_string()
        } else {
            format!("{}/{}", sel + 1, total)
        }
    })
    .style(move |s| crate::ui::css::apply(s, &sheet_count, "label", &["count-chip"]));

    let sheet_bar = Arc::clone(&sheet);
    Stack::horizontal((
        mode_label,
        mode_name_label,
        Container::new(Empty::new()).style(|s| s.flex_grow(1.0_f32)),
        count_label,
    ))
    .style(move |s| {
        // Geometry / bg / radius from `.status-bar` in default.css.
        // Inline tail: `min_height` + `flex_shrink` (no CSS analogues)
        // and the const margins exposed for chrome math.
        crate::ui::css::apply(s, &sheet_bar, "stack", &["status-bar"])
            .min_height(STATUS_BAR_HEIGHT + STATUS_BAR_VPAD * 2.0)
            .flex_shrink(0.0_f32)
            .margin_top(STATUS_BAR_MARGIN_TOP)
            .margin_bottom(STATUS_BAR_MARGIN_BOTTOM)
    })
}

// ─── App-level reactive state ─────────────────────────────────────────────────

pub struct AppState {
    pub picker: PickerState,
    pub entries: Vec<Arc<Entry>>,
    /// Usage keys parallel to `entries` (one per entry, same order), so the
    /// per-keystroke bonus loop looks up frecency by key instead of rebuilding
    /// a String per entry. Rebuilt whenever `entries` is replaced.
    pub usage_keys: Vec<String>,
    pub matches: Vec<Match>,
    pub g_pending: bool,
    pub cli_mode: CliMode,
    pub prompt: String,
    pub max_results: usize,
    pub theme: Theme,
    pub matcher: Matcher,
    /// When true, the query bar renders each character as ● (U+25CF) instead
    /// of the actual glyph. The underlying `query` signal still holds the
    /// real text — masking is display-only.
    pub password: bool,
    /// Per-mode frecency (count × half-life decay). Loaded from disk at
    /// startup; bumped on Accept and persisted then.
    pub usage: crate::picker::frecency::Usage,
    /// Per-mode query history (most-recent first). Pushed on Accept (with
    /// non-empty query), recalled via Ctrl-P/Ctrl-N in Insert mode.
    pub history: crate::picker::history::History,
    /// XDG icon-theme lookup cache. Populated lazily as rows scroll into
    /// view; icons are picker-only (message_view doesn't use it).
    pub icons: Arc<Mutex<crate::picker::icons::IconCache>>,
    /// Parsed default stylesheet with theme colours substituted. Migrated
    /// `.style(...)` sites use `ui::css::apply` against this sheet to pick
    /// up declarative rules; reactive sites still chain inline.
    pub stylesheet: Arc<hjkl_css::Stylesheet>,
}

impl AppState {
    pub fn rerank(&mut self) {
        let query = self.picker.query.get();
        tracing::debug!(
            mode = ?self.cli_mode,
            query_len = query.chars().count(),
            "picker query reranked"
        );
        if matches!(self.cli_mode, CliMode::Calc) {
            self.rerank_calc(&query);
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
        let mode_key = crate::picker::frecency::mode_key(self.cli_mode);
        for m in &mut ranked {
            let bonus = self
                .usage
                .bonus_for_key(&mode_key, &self.usage_keys[m.index], now);
            m.score = m.score.saturating_add(bonus);
        }
        ranked.sort_by(|a, b| b.score.cmp(&a.score).then(a.index.cmp(&b.index)));
        self.matches = ranked;
        self.matches.truncate(self.max_results);
        self.picker.clamp_selected(self.matches.len());
    }

    /// Calc-mode rerank: build a row list from on-disk history plus the live
    /// query's eval result. The live entry sits on top (when the expression
    /// evaluates), historic entries below are fuzzy-filtered through the
    /// shared matcher. Pure-text queries that don't evaluate (e.g. `10+`)
    /// fall through to history-only ranking so the user can still scrub past
    /// expressions while typing a new one.
    fn rerank_calc(&mut self, query: &str) {
        let empty: Rc<Vec<u32>> = Rc::new(Vec::new());
        let trimmed = query.trim();
        let live_eval = if trimmed.is_empty() {
            None
        } else {
            crate::modes::calc::eval(query).map(|r| (trimmed.to_string(), r))
        };

        let history: Vec<(String, String)> = self
            .history
            .list(CliMode::Calc)
            .iter()
            .filter(|expr| {
                // Dedupe: when the live query exactly matches a stored entry,
                // skip the historic row so it doesn't render twice.
                live_eval
                    .as_ref()
                    .is_none_or(|(live_expr, _)| live_expr.as_str() != expr.as_str())
            })
            .filter_map(|expr| {
                let result = crate::modes::calc::eval(expr)?;
                Some((expr.clone(), result))
            })
            .collect();

        let mut entries: Vec<Arc<Entry>> = Vec::with_capacity(history.len() + 1);
        if let Some((expr, result)) = live_eval.as_ref() {
            entries.push(Arc::new(Entry {
                label: format!("{expr} = {result}"),
                description: None,
                icon: None,
                payload: crate::modes::Payload::Stdout(result.clone()),
            }));
        }
        for (expr, result) in &history {
            entries.push(Arc::new(Entry {
                label: format!("{expr} = {result}"),
                description: None,
                icon: None,
                payload: crate::modes::Payload::Stdout(result.clone()),
            }));
        }
        self.entries = entries;
        self.usage_keys = crate::picker::frecency::entry_keys(&self.entries);

        let live_offset = usize::from(live_eval.is_some());
        let mut matches: Vec<Match> = Vec::new();
        if live_eval.is_some() {
            // Live entry always sits at the top, unfiltered, with no
            // highlight spans (the whole label is the user's input).
            matches.push(Match {
                index: 0,
                score: u16::MAX,
                positions: empty.clone(),
                desc_positions: empty.clone(),
            });
        }

        if trimmed.is_empty() {
            // Empty query → show every historic entry verbatim, newest first.
            for i in 0..history.len() {
                matches.push(Match {
                    index: live_offset + i,
                    score: 0,
                    positions: empty.clone(),
                    desc_positions: empty.clone(),
                });
            }
        } else {
            // Non-empty query → fuzzy-rank history labels against the query
            // so users can recall past expressions while still typing.
            let pairs: Vec<(&str, Option<&str>)> = self
                .entries
                .iter()
                .skip(live_offset)
                .map(|e| (e.label.as_str(), e.description.as_deref()))
                .collect();
            let mut ranked = self.matcher.rank(&pairs, query);
            ranked.sort_by(|a, b| b.score.cmp(&a.score).then(a.index.cmp(&b.index)));
            for mut m in ranked {
                m.index += live_offset;
                matches.push(m);
            }
        }

        self.matches = matches;
        self.matches.truncate(self.max_results);
        self.picker.clamp_selected(self.matches.len());
    }

    pub fn switch_mode(&mut self, mode: CliMode) {
        self.cli_mode = mode;
        // Unsupported-on-this-platform modes resolve to `None`: the mode
        // label switches but the list is cleared rather than showing the
        // previous mode's entries under the new label. Unix behavior is
        // unchanged — every mode constructs its real Mode.
        let m: Option<Box<dyn crate::modes::Mode>> = match mode {
            CliMode::Calc => Some(Box::new(crate::modes::calc::Calc)),
            CliMode::Clipboard => Some(Box::new(crate::modes::clipboard::Clipboard)),
            CliMode::Dmenu => Some(Box::new(crate::modes::dmenu::Dmenu)),
            #[cfg(unix)]
            CliMode::Drun => Some(Box::new(crate::modes::drun::Drun)),
            #[cfg(not(unix))]
            CliMode::Drun => None,
            CliMode::Emoji => Some(Box::new(crate::modes::emoji::Emoji)),
            #[cfg(unix)]
            CliMode::Run => Some(Box::new(crate::modes::run::Run)),
            #[cfg(not(unix))]
            CliMode::Run => None,
            #[cfg(unix)]
            CliMode::Ssh => Some(Box::new(crate::modes::ssh::Ssh)),
            #[cfg(not(unix))]
            CliMode::Ssh => None,
        };
        self.entries = match m {
            Some(mut m) => m
                .collect()
                .unwrap_or_default()
                .into_iter()
                .map(Arc::new)
                .collect(),
            None => Vec::new(),
        };
        self.usage_keys = crate::picker::frecency::entry_keys(&self.entries);
        self.picker.query.set(String::new());
        self.picker.query_cursor.set(0);
        self.picker.selected.set(0);
        self.picker.reset_after_mode_switch();
        self.rerank();
    }
}

fn rerank_if_query_changed(previous: Option<&str>, current: &str, rerank: impl FnOnce()) -> bool {
    if previous == Some(current) {
        return false;
    }
    rerank();
    true
}

// ─── Picker view ─────────────────────────────────────────────────────────────

/// Selection after moving down by `n` rows, clamped to the last row.
/// `n` can be `usize::MAX` (a saturated count prefix); the addition must
/// saturate, not wrap — wrapping lands on `cur - 1` and moves the
/// selection up.
fn move_down_selection(cur: usize, n: usize, total: usize) -> usize {
    cur.saturating_add(n).min(total.saturating_sub(1))
}

pub fn picker_view(state: Arc<Mutex<AppState>>, startup_started: Instant) -> impl IntoView {
    // Typing-key events in floem main route only to the focused view. We
    // construct the outer container with a stable `root_id` so two
    // listeners below (WindowGainedFocus + per-KeyDown) can pin focus to
    // it — see the comments at the bottom of this function.
    let root_id = ViewId::new();
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

    let (_bg, fg, accent, muted, selected_bg, font_family, font_size) = {
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
    let password = state.lock().unwrap().password;
    let sheet = Arc::clone(&state.lock().unwrap().stylesheet);

    // Parse the font family list once per session and share it — the query
    // bar and every row's rich_text rebuild on each relayout/keystroke, and
    // re-parsing the same string there would be pure per-frame waste.
    let families: std::sync::Arc<Vec<FamilyOwned>> =
        std::sync::Arc::new(FamilyOwned::parse_list(&font_family).collect());

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
    let prompt_text = if prompt_str.is_empty() {
        ">".to_string()
    } else {
        format!("{}:", prompt_str)
    };
    let sheet_prompt = Arc::clone(&sheet);
    let prompt_label = Label::derived(move || prompt_text.clone())
        .style(move |s| crate::ui::css::apply(s, &sheet_prompt, "label", &["prompt"]));

    // Hand-rolled input: a label that renders the query plus a vim-style
    // cursor glyph (thin in Insert, block in Normal). All editing flows
    // through the outer keydown handler — InsertChar / Backspace mutate
    // query_sig directly. Avoids floem text_input's focus-stealing Esc.
    let sheet_query = Arc::clone(&sheet);
    let blink_on: RwSignal<bool> = RwSignal::new(true);
    // The blink thread keeps ticking, but the toggle is gated on window
    // focus: an unfocused surface would otherwise rebuild the query bar's
    // dyn_view (two TextLayout measurements) and repaint every 530 ms
    // forever, for no user-visible reason. WindowLostFocus leaves the cursor
    // visible-but-static; WindowGainedFocus restarts the blink from the
    // visible phase.
    let focused: RwSignal<bool> = RwSignal::new(false);
    {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let tick_sig = ChannelSignal::new(rx);
        Effect::new(move |_| {
            let _ = tick_sig.get(); // Option<()> — any value triggers the blink flip
            if focused.get_untracked() {
                blink_on.update(|b| *b = !*b);
            }
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
    Effect::new(move |_| {
        let _ = query_sig.get();
        let _ = query_cursor_sig.get();
        let _ = ex_buf_sig.get();
        blink_on.set(true);
    });
    // Query bar: the text is ONE label at a fixed position; the cursor is a
    // separate rect drawn on top, absolutely positioned at the caret's measured
    // x-offset. Only the cursor moves as the caret moves — the text never
    // relayouts (no reflow on caret move / blink / mode switch). The caret x and
    // the block width are measured with the SAME font the label renders, so the
    // overlay lines up exactly and stays font-independent.
    let qfont = font_family.clone();
    let families_query = families.clone();
    let query_view = dyn_view(move || {
        let displayed = mask_password(password, &query_sig.get());
        let mode = vim_mode_sig.get();
        let on = blink_on.get();
        let ff = qfont.clone();
        let families = families_query.clone();
        let measure = move |s: &str| -> f64 {
            if s.is_empty() {
                return 0.0;
            }
            let attrs = Attrs::new().family(&families).font_size(font_size);
            floem::text::TextLayout::new_with_text(s, AttrsList::new(attrs), None)
                .size()
                .width
        };

        let chars: Vec<char> = displayed.chars().collect();
        let n = chars.len();
        let cpos = query_cursor_sig.get().min(n);
        let before: String = chars[..cpos].iter().collect();
        let caret_x = measure(&before);
        let block = matches!(mode, VimMode::Normal | VimMode::Visual);
        // Block covers the caret cell; insert is a thin bar between cells.
        let cell_w = if block {
            let ch: String = chars.get(cpos).copied().unwrap_or(' ').to_string();
            measure(&ch).max(2.0)
        } else {
            2.0
        };
        let ff_t = ff.clone();
        let text_label = Label::new(displayed)
            .style(move |s| s.color(fg).font_family(ff_t.clone()).font_size(font_size));
        let cursor = Empty::new().style(move |s| {
            // Stretch vertically to the text's line box (inset top+bottom 0)
            // instead of guessing a pixel height, so the cursor aligns with the
            // text regardless of font metrics.
            let s = s
                .absolute()
                .inset_left(caret_x)
                .inset_top(0.0)
                .inset_bottom(0.0)
                .width(cell_w);
            // Hidden on the blink-off phase.
            if on { s.background(accent) } else { s }
        });

        if block {
            // Highlight rect behind the glyph so the character stays visible.
            Stack::horizontal((cursor, text_label)).into_any()
        } else {
            // Thin caret on top of the text.
            Stack::horizontal((text_label, cursor)).into_any()
        }
    })
    .style(move |s| {
        // CSS owns color, font, margin-left. flex_grow has no CSS analogue — stays inline.
        crate::ui::css::apply(s, &sheet_query, "label", &["query"]).flex_grow(1.0_f32)
    });

    // Rerank whenever the query mutates. Bump `rev` so the dyn_stack rebuilds
    // against the new match list.
    let state_rerank = Arc::clone(&state);
    Effect::new(move |prev: Option<String>| {
        let cur = query_sig.get();
        let reranked = rerank_if_query_changed(prev.as_deref(), &cur, || {
            // `rerank()` calls `picker.clamp_selected` which `selected.set(0)`s
            // when the match list is empty. floem fires subscribers (status
            // bar count, virtual_stack data fn, empty-state, …) synchronously
            // inside `set` — while we still hold the AppState mutex —
            // and those subscribers each `state.lock()` themselves. Std
            // Mutex isn't re-entrant, so the second lock hangs forever (the
            // "no-results hang"). `batch` queues subscriber effects until
            // the closure returns; by then the mutex is dropped.
            Effect::batch(|| {
                {
                    let mut s = state_rerank.lock().unwrap();
                    s.rerank();
                }
                rev.update(|r| *r += 1);
            });
        });
        if reranked && prev.is_none() {
            tracing::debug!(
                elapsed_us = startup_started.elapsed().as_micros(),
                "startup state ranked"
            );
        }
        cur
    });

    let hover_bg = blend(accent, selected_bg, 0.18);
    let sheet_input = Arc::clone(&sheet);
    let input_row = Stack::horizontal((prompt_label, query_view)).style(move |s| {
        // Geometry + bg + radius from `.input-row` in default.css. Inline
        // bits stay: `min_height` and `flex_shrink` (no CSS equivalents in
        // hjkl-css-gui), plus the reactive `:hover` blend.
        crate::ui::css::apply(s, &sheet_input, "stack", &["input-row"])
            .min_height(INPUT_ROW_HEIGHT)
            .flex_shrink(0.0_f32)
            .hover(|s| s.background(hover_bg))
    });

    // ── Result list ────────────────────────────────────────────────────────
    // Virtualised: floem only builds row views inside the scroll viewport,
    // so a 1800-entry emoji list paints instantly instead of stalling vger
    // glyph-atlas uploads for hundreds of multi-byte unicode rows.
    let state_list = Arc::clone(&state);
    let state_row = Arc::clone(&state);
    let sheet_row_list = Arc::clone(&sheet);
    let result_list = virtual_stack(
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
                .collect::<imbl::Vector<_>>()
        },
        |item| {
            let (mi, idx, entry, positions, desc_positions) = item;
            row_key(
                *mi,
                *idx,
                Arc::as_ptr(entry) as u64,
                positions,
                desc_positions,
            )
        },
        move |(mi, _idx, entry, positions, desc_positions)| {
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
                families.clone(),
                font_size,
                Arc::clone(&sheet_row_list),
            )
            .style(move |s| s.margin_top(ROW_GAP))
        },
    )
    .item_size_fixed(|| ROW_PITCH)
    .style(|s| s.width_full().flex_direction(FlexDirection::Column));

    // Empty-state hint shown inside the scroll viewport when there are zero
    // matches AND the user has typed something. Sits as a v_stack sibling
    // beneath the virtual_stack and toggles its display so it doesn't take
    // any space when results are present.
    let state_empty = Arc::clone(&state);
    let state_empty_style = Arc::clone(&state);
    let sheet_empty_text = Arc::clone(&sheet);
    let sheet_empty_row = Arc::clone(&sheet);
    let empty_msg = Stack::horizontal((Label::derived(move || {
        let _ = rev.get();
        let s = state_empty.lock().unwrap();
        if s.picker.query.get().is_empty() {
            "No entries.".to_string()
        } else {
            format!("No results for \u{201C}{}\u{201D}", s.picker.query.get())
        }
    })
    .style(move |s| crate::ui::css::apply(s, &sheet_empty_text, "label", &["empty-row-text"])),))
    .style(move |s| {
        let _ = rev.get();
        let visible = state_empty_style.lock().unwrap().matches.is_empty();
        // CSS owns width/height/padding/align-items. Visibility toggle stays inline.
        crate::ui::css::apply(s, &sheet_empty_row, "stack", &["empty-row"])
            .apply_if(!visible, |s| s.display(floem::style::Display::None))
    });

    let result_area = Stack::vertical((result_list, empty_msg))
        .style(|s| s.width_full().flex_direction(FlexDirection::Column));

    let state_ensure = Arc::clone(&state);
    let sheet_handle = Arc::clone(&sheet);
    let scrollable = floem::views::Scroll::new(result_area)
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
            let sh = Arc::clone(&sheet_handle);
            s.width_full()
                .flex_grow(1.0_f32)
                .flex_basis(0.0)
                .min_height(0.0)
                .class(floem::views::scroll::Handle, move |h| {
                    crate::ui::css::apply(h, &sh, "div", &["scroll-handle"])
                })
        });

    let ex = ex_bar(ex_buf_sig, blink_on, fg, Arc::clone(&sheet));
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
        Arc::clone(&sheet),
    );

    // ── Outer panel ────────────────────────────────────────────────────────
    // Two layers: an opaque non-rounded outer fill (so the framebuffer
    // transparency doesn't leak outside the rounded corners or border ring),
    // and the rounded inner panel that holds the actual content.
    let state_key = Arc::clone(&state);
    let sheet_stack = Arc::clone(&sheet);
    let sheet_panel = Arc::clone(&sheet);
    let sheet_outer = Arc::clone(&sheet);
    let first_focus_logged = Rc::new(Cell::new(false));
    let first_paint_logged = Rc::new(Cell::new(false));
    Container::with_id(
        root_id,
        Container::new(
            Stack::vertical((input_row, scrollable, ex.into_any(), status.into_any())).style(
                move |s| crate::ui::css::apply(s, &sheet_stack, "container", &["panel-stack"]),
            ),
        )
        .style(move |s| crate::ui::css::apply(s, &sheet_panel, "container", &["panel"])),
    )
    .style(move |s| crate::ui::css::apply(s, &sheet_outer, "container", &["panel-outer"]))
    .style(|s| s.keyboard_navigable())
    .on_event(
        UpdatePhasePaintPresent,
        move |_cx: &mut EventCx, _ev: &()| {
            if !first_paint_logged.replace(true) {
                tracing::debug!(
                    elapsed_us = startup_started.elapsed().as_micros(),
                    "startup first paint pass reached"
                );
            }
            EventPropagation::Continue
        },
    )
    .on_event(WindowGainedFocus, move |_cx: &mut EventCx, _ev: &()| {
        if !first_focus_logged.replace(true) {
            tracing::debug!(
                elapsed_us = startup_started.elapsed().as_micros(),
                "startup first focus received"
            );
        }
        // Compositor handed our surface keyboard focus — claim view focus
        // immediately so the very first keystroke is delivered. Without
        // this pikr drops keys until Esc→i: the Esc registry-fallback
        // is the only thing that nudges focus onto root.
        root_id.request_focus();
        // Restart the cursor blink from the visible phase on focus gain.
        focused.set(true);
        blink_on.set(true);
        EventPropagation::Continue
    })
    .on_event(WindowLostFocus, move |_cx: &mut EventCx, _ev: &()| {
        // Stop the blink loop (the tick effect checks this signal) and leave
        // the cursor visible-but-static so it doesn't vanish mid-wait.
        focused.set(false);
        blink_on.set(true);
        EventPropagation::Continue
    })
    .on_event(KeyDown, move |_cx: &mut EventCx, ke: &KeyboardEvent| {
        // Re-claim focus on every keystroke. New floem main routes typing
        // keys ONLY to the focused element; the reactive update triggered
        // by `picker.query.set(...)` below can drop focus on the next
        // render and strand subsequent keystrokes. TODO: investigate
        // upstream — figure out which sub-effect of the rerank chain
        // moves focus, and pin it so this per-key reclaim can go.
        root_id.request_focus();
        let ctrl = ke.modifiers.ctrl();
        let shift = ke.modifiers.shift();
        let key = &ke.key;

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
                key_to_action(&s.picker, key, ctrl, shift)
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
                        // `switch_mode` sets the `query`/`selected`/… signals.
                        // floem fires subscribers synchronously inside `set`
                        // unless batched; those subscribers (the rerank
                        // effect, status-bar count) re-lock the same AppState
                        // mutex this handler already holds, and std Mutex is
                        // non-reentrant → permanent deadlock. The guard must
                        // be created INSIDE the batch closure so it drops
                        // before floem runs the queued subscriber effects —
                        // see `signal_set_inside_held_mutex_does_not_deadlock_when_batched`
                        // in picker/state.rs. Batching only the sets inside
                        // `switch_mode` would not help: the queued effects
                        // would still run while this guard is held.
                        Effect::batch(|| {
                            state_key.lock().unwrap().switch_mode(m);
                            // History recall is picker_view-local: reset it so
                            // the new mode starts with a clean draft/cursor
                            // instead of the previous mode's recall state.
                            history_cursor.set(None);
                            history_draft.set(String::new());
                        });
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
                    buf.push_str(ch.as_str());
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
                selected_sig.set(move_down_selection(cur, n, total));
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
            Action::AppendAfter => {
                // vim `a`: caret one cell right (clamped), then Insert.
                let len = query_sig.get().chars().count();
                let cur = query_cursor_sig.get();
                query_cursor_sig.set((cur + 1).min(len));
                visual_anchor_sig.set(None);
                vim_mode_sig.set(VimMode::Insert);
            }
            Action::AppendEnd => {
                // vim `A`: caret to end, then Insert.
                query_cursor_sig.set(query_sig.get().chars().count());
                visual_anchor_sig.set(None);
                vim_mode_sig.set(VimMode::Insert);
            }
            Action::InsertStart => {
                // vim `I`: caret to start, then Insert.
                query_cursor_sig.set(0);
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
                    // -P: sensitive input — never persisted.
                    if !s.password {
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
                    }
                    (payloads, cli_mode)
                };
                // In dmenu mode with no matches, fall through to AcceptCustom
                // semantics so the typed query is returned as output — rofi
                // parity (rofi --dmenu returns the unmatched input).
                if payloads.is_empty() && cli_mode == CliMode::Dmenu {
                    let query_text = query_sig.get_untracked();
                    let payload = modes::Payload::Stdout(query_text.trim().to_string());
                    {
                        let mut s = state_key.lock().unwrap();
                        let q = query_text.trim();
                        // -P: sensitive input — never persisted.
                        if !s.password && !q.is_empty() {
                            s.history.push(cli_mode, q);
                            s.history.save();
                        }
                    }
                    if let Err(e) = modes::execute(&payload) {
                        eprintln!("pikr: execute error: {e}");
                    }
                    std::process::exit(0);
                }
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
            Action::AcceptCustom => {
                let query_text = query_sig.get_untracked();
                let payload = modes::Payload::Stdout(query_text.trim().to_string());
                {
                    let mut s = state_key.lock().unwrap();
                    let cli_mode = s.cli_mode;
                    let q = query_text.trim();
                    // -P: sensitive input — never persisted.
                    if !s.password && !q.is_empty() {
                        s.history.push(cli_mode, q);
                        s.history.save();
                    }
                }
                if let Err(e) = modes::execute(&payload) {
                    eprintln!("pikr: execute error: {e}");
                }
                std::process::exit(0);
            }
            Action::Cancel => std::process::exit(1),
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
    use super::{
        char_idx_to_byte, mask_password, move_down_selection, parse_color, rerank_if_query_changed,
        row_key, with_cursor, word_boundary_back,
    };
    use crate::picker::state::VimMode;
    use std::cell::Cell;

    #[test]
    fn initial_query_reranks_once() {
        let calls = Cell::new(0);

        assert!(rerank_if_query_changed(None, "", || {
            calls.set(calls.get() + 1);
        }));
        assert!(!rerank_if_query_changed(Some(""), "", || {
            calls.set(calls.get() + 1);
        }));

        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn changed_query_reranks_once() {
        let calls = Cell::new(0);

        assert!(rerank_if_query_changed(Some("old"), "new", || {
            calls.set(calls.get() + 1);
        }));

        assert_eq!(calls.get(), 1);
    }

    /// Regression for the empty-query → first-char highlight stuck bug.
    /// Empty match `positions=[]` MUST hash differently from `positions=[0]`,
    /// otherwise dyn_stack reuses the cached row view and the old "first
    /// letter highlighted" rendering survives a query clear.
    #[test]
    fn row_key_distinguishes_empty_from_first_char_match() {
        let empty = row_key(0, 0, 0, &[], &[]);
        let first = row_key(0, 0, 0, &[0], &[]);
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
        let k1 = row_key(0, 5, 0, &[0], &[]);
        let k2 = row_key(0, 5, 0, &[0, 1], &[]);
        let k3 = row_key(0, 5, 0, &[0, 1, 2], &[]);
        let k4 = row_key(0, 5, 0, &[0, 1, 2, 3], &[]);
        assert_ne!(k1, k2);
        assert_ne!(k2, k3);
        assert_ne!(k3, k4);
        assert_ne!(k1, k4);
    }

    #[test]
    fn row_key_distinguishes_same_len_different_positions() {
        assert_ne!(
            row_key(0, 0, 0, &[0, 3], &[]),
            row_key(0, 0, 0, &[0, 4], &[])
        );
        assert_ne!(
            row_key(0, 0, 0, &[1, 2], &[]),
            row_key(0, 0, 0, &[2, 1], &[])
        );
    }

    #[test]
    fn row_key_distinguishes_mi_and_idx() {
        assert_ne!(row_key(0, 0, 0, &[0], &[]), row_key(1, 0, 0, &[0], &[]));
        assert_ne!(row_key(0, 0, 0, &[0], &[]), row_key(0, 1, 0, &[0], &[]));
    }

    #[test]
    fn row_key_is_deterministic() {
        assert_eq!(
            row_key(2, 7, 42, &[0, 1, 2], &[3, 4]),
            row_key(2, 7, 42, &[0, 1, 2], &[3, 4])
        );
    }

    /// Description positions must also invalidate the cache — typing across
    /// a description-only match has the same staleness risk as a label match.
    #[test]
    fn row_key_distinguishes_desc_positions() {
        assert_ne!(row_key(0, 0, 0, &[], &[]), row_key(0, 0, 0, &[], &[0]));
        assert_ne!(row_key(0, 0, 0, &[0], &[0]), row_key(0, 0, 0, &[0], &[1]));
    }

    /// Regression for the calc-mode "10+10 = 11" stale-label bug. Calc emits
    /// a single synthetic entry at mi=0/idx=0/positions=[], so without the
    /// entry-id mix the key is byte-for-byte identical across reranks and
    /// dyn_stack keeps the cached row → user sees the prior expression's
    /// result. The fresh Arc per rerank changes `entry_id`; assert that
    /// different ids hash to different keys.
    #[test]
    fn row_key_distinguishes_entry_id() {
        assert_ne!(
            row_key(0, 0, 0xdead, &[], &[]),
            row_key(0, 0, 0xbeef, &[], &[])
        );
    }

    // ── with_cursor: ex-bar caret splice (still used for the `:` prompt) ──────

    /// Insert-mode caret is a thin bar sitting BETWEEN characters at the caret
    /// index — it does not cover a character. (Regression guard; this was already
    /// correct.)
    #[test]
    fn with_cursor_insert_bar_between_chars() {
        assert_eq!(
            with_cursor("hello", 0, VimMode::Insert, true),
            "\u{258F}hello"
        );
        assert_eq!(
            with_cursor("hello", 2, VimMode::Insert, true),
            "he\u{258F}llo"
        );
        assert_eq!(
            with_cursor("hello", 5, VimMode::Insert, true),
            "hello\u{258F}"
        );
        // All six char positions keep the text length at 5 + 1 caret cell.
        for c in 0..=5 {
            assert_eq!(
                with_cursor("hello", c, VimMode::Insert, true)
                    .chars()
                    .count(),
                6
            );
        }
    }

    /// BUG: the Normal-mode block cursor must sit ON the character at the caret
    /// (cover it), NOT be inserted before it. Inserting grew the line by a cell
    /// and pushed the covered char one position right, so the block rendered in
    /// the wrong spot.
    #[test]
    fn with_cursor_normal_block_overlays_char() {
        assert_eq!(
            with_cursor("hello", 0, VimMode::Normal, true),
            "\u{2588}ello"
        );
        assert_eq!(
            with_cursor("hello", 2, VimMode::Normal, true),
            "he\u{2588}lo"
        );
        assert_eq!(
            with_cursor("hello", 4, VimMode::Normal, true),
            "hell\u{2588}"
        );
        // Overlay keeps the visible length equal to the text length.
        assert_eq!(
            with_cursor("hello", 2, VimMode::Normal, true)
                .chars()
                .count(),
            5
        );
    }

    /// Caret past the last char (e.g. cursor at end) appends the block; an empty
    /// query renders just the block.
    #[test]
    fn with_cursor_normal_block_at_or_past_end() {
        assert_eq!(
            with_cursor("hello", 5, VimMode::Normal, true),
            "hello\u{2588}"
        );
        assert_eq!(with_cursor("", 0, VimMode::Normal, true), "\u{2588}");
    }

    /// Visual mode shares the Normal block-overlay behavior.
    #[test]
    fn with_cursor_visual_block_overlays_char() {
        assert_eq!(with_cursor("hi", 1, VimMode::Visual, true), "h\u{2588}");
    }

    /// On the blink-off phase the block reveals the character underneath (so the
    /// covered char doesn't vanish each blink) and never displaces the text.
    #[test]
    fn with_cursor_block_blink_off_reveals_char() {
        assert_eq!(with_cursor("hello", 2, VimMode::Normal, false), "hello");
        assert_eq!(
            with_cursor("hello", 2, VimMode::Normal, false)
                .chars()
                .count(),
            5
        );
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

    // ── mask_password tests ───────────────────────────────────────────────

    #[test]
    fn mask_password_disabled_returns_original() {
        assert_eq!(mask_password(false, "hello"), "hello");
        assert_eq!(mask_password(false, ""), "");
    }

    #[test]
    fn mask_password_enabled_replaces_with_bullets() {
        assert_eq!(mask_password(true, "abc"), "●●●");
        assert_eq!(mask_password(true, ""), "");
    }

    #[test]
    fn mask_password_preserves_char_count_for_multibyte() {
        // Each multibyte char must produce exactly one ● so cursor index
        // arithmetic in with_cursor stays correct.
        let s = "héllo"; // 5 codepoints, 6 bytes
        let masked = mask_password(true, s);
        assert_eq!(masked.chars().count(), s.chars().count());
        assert_eq!(masked, "●●●●●");
    }

    #[test]
    fn mask_password_single_char() {
        assert_eq!(mask_password(true, "x"), "●");
    }

    // ── parse_color tests ────────────────────────────────────────────────

    #[test]
    fn parse_color_accepts_6_digit_hex() {
        // #21D1D3 is the default accent (tokyonight cyan).
        let c = parse_color("#21D1D3");
        let rgba = c.to_rgba8();
        assert_eq!((rgba.r, rgba.g, rgba.b), (0x21, 0xD1, 0xD3));
    }

    #[test]
    fn parse_color_rejects_short_hex() {
        // 3-digit shorthand has no CSS-expansion semantics here — black.
        let c = parse_color("#abc");
        let rgba = c.to_rgba8();
        assert_eq!((rgba.r, rgba.g, rgba.b), (0, 0, 0));
    }

    #[test]
    fn parse_color_rejects_8_digit_hex() {
        // #AARRGGBB is out of scope — only 6-digit RGB is accepted.
        let c = parse_color("#aarrggbb");
        let rgba = c.to_rgba8();
        assert_eq!((rgba.r, rgba.g, rgba.b), (0, 0, 0));
    }

    #[test]
    fn parse_color_rejects_garbage() {
        let c = parse_color("zzz");
        let rgba = c.to_rgba8();
        assert_eq!((rgba.r, rgba.g, rgba.b), (0, 0, 0));
    }

    #[test]
    fn move_down_saturates_on_max_count() {
        // n == usize::MAX (a saturated count prefix) must clamp to the last
        // row, not overflow: `cur + n` panics in debug and wraps to `cur - 1`
        // in release (moving the selection UP).
        assert_eq!(move_down_selection(1, usize::MAX, 10), 9);
        assert_eq!(move_down_selection(0, usize::MAX, 10), 9);
        assert_eq!(move_down_selection(5, usize::MAX, 3), 2);
        assert_eq!(move_down_selection(0, 2, 5), 2);
        assert_eq!(move_down_selection(4, 2, 5), 4);
        assert_eq!(move_down_selection(0, 2, 0), 0);
    }
}
