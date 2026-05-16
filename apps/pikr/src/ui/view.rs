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

use crate::ui::glass::{backdrop_overlay, glow_overlay};

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

/// Linearly blend `over` onto `under` at `t` (0..=1) and return an opaque
/// result. Used for borders / dividers that visually want partial-alpha
/// appearance — `multiply_alpha` on a border would leave the pixel
/// translucent, and on the PreMultiplied-alpha wgpu surface that means
/// the framebuffer (and thus the desktop) shows through.
fn blend(over: Color, under: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let r = (over.r as f32 * t + under.r as f32 * (1.0 - t)) as u8;
    let g = (over.g as f32 * t + under.g as f32 * (1.0 - t)) as u8;
    let b = (over.b as f32 * t + under.b as f32 * (1.0 - t)) as u8;
    Color::rgb8(r, g, b)
}

// ─── Match-highlight helpers ─────────────────────────────────────────────────

/// Build a horizontal stack of spans with matched chars highlighted in accent.
/// Colors flip when `selected_sig.get() == mi` so text stays readable on the
/// solid-accent selection background.
#[allow(clippy::too_many_arguments)]
fn highlighted_label(
    label_str: String,
    positions: Vec<u32>,
    mi: usize,
    selected_sig: RwSignal<usize>,
    fg: Color,
    accent: Color,
    selected_text: Color,
    selected_hl: Color,
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
                    let color = match (selected, hl) {
                        (true, true) => selected_hl,
                        (true, false) => selected_text,
                        (false, true) => accent,
                        (false, false) => fg,
                    };
                    s.color(color)
                })
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
    // Outline-style selection: text colour doesn't change between states
    // (only the row border lights up in accent). `bg` is used as the row's
    // fill — the caller passes `inner_bg` so the row is opaque when the
    // panel is opaque, transparent when the panel is translucent.
    let row_fill = bg;
    let desc_color = fg.multiply_alpha(0.55);

    let label_view = highlighted_label(
        entry.label.clone(),
        positions,
        mi,
        selected_sig,
        fg,
        accent,
        fg,
        accent,
    );

    let desc_view: Box<dyn floem::View> = match &entry.description {
        Some(d) => {
            // Wrap the description in parens + italic style for the
            // "Buffr (Web Browser)" look. floem's italic style isn't
            // exposed in 0.2; the parens carry most of the visual weight.
            let d = format!("({d})");
            label(move || d.clone())
                .style(move |s| s.color(desc_color).margin_left(DESC_GAP))
                .into_any()
        }
        None => floem::views::empty().into_any(),
    };

    // Single-click accepts the entry — pikr exits immediately. Captures the
    // payload at row construction so the handler doesn't need an AppState
    // reference. Selection mirrors the click target first so the highlight
    // briefly tracks the click before exit (cosmetic if the exit is instant).
    let click_payload = entry.payload.clone();

    // Single-layer row. Selection = solid accent fill (Raycast-style); no
    // floem `.border()` — center-aligned stroke leaks via PreMultiplied
    // alpha. Vertical spacing comes from `padding_vert(5)` so the 10px
    // gap between rows sits INSIDE this row's bbox and is filled by the
    // row's own bg paint, never by an unpainted parent margin.
    h_stack((label_view.into_any(), desc_view))
        .style(move |s| {
            let selected = selected_sig.get() == mi;
            let row_bg = if selected { accent } else { row_fill };
            s.width_full()
                .height(ROW_HEIGHT + 10.0)
                .padding_vert(5.0)
                .padding_horiz(HORIZ_PAD)
                .items_center()
                .background(row_bg)
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

/// Pixel height per result row, in logical pixels. Used both by the row
/// itself and by the scroll viewport so the viewport stays an exact
/// multiple of row height. Sized to give Raycast-like breathing room.
pub(crate) const ROW_HEIGHT: f64 = 40.0;
/// Vertical spacing added by `margin_vert(5)` on each row — 5px top and
/// bottom collapse to 10px total between adjacent rows.
pub(crate) const ROW_GAP: f64 = 10.0;
/// Effective layout height of one row including its top+bottom margin.
/// Used by viewport sizing AND scroll-follow math so the cursor rect
/// stays aligned with the row's actual y-position.
pub(crate) const ROW_PITCH: f64 = ROW_HEIGHT + ROW_GAP;
/// Number of result rows the viewport shows. Window height is sized
/// from this plus the prompt + status chrome.
pub(crate) const VISIBLE_ROWS: usize = 8;
/// Rows of buffer kept visible above/below the selected row before the
/// viewport scrolls (vim-style `scrolloff`).
pub(crate) const SCROLLOFF: f64 = 1.0;
/// Prompt+query row height — slightly taller than result rows so the
/// input gets visual prominence (Raycast pattern).
pub(crate) const INPUT_ROW_HEIGHT: f64 = 56.0;
/// Status bar height.
pub(crate) const STATUS_HEIGHT: f64 = 28.0;
/// Horizontal padding shared by input row, result rows, status bar.
pub(crate) const HORIZ_PAD: f64 = 18.0;
/// Spacing between a row's label and its description.
pub(crate) const DESC_GAP: f64 = 14.0;

// ─── Ex command bar ──────────────────────────────────────────────────────────

fn ex_bar(ex_buf: RwSignal<Option<String>>, fg: Color, bg: Color) -> impl IntoView {
    label(move || match ex_buf.get() {
        Some(s) => format!(":{s}"),
        None => String::new(),
    })
    .style(move |s| {
        s.width_full()
            .height(22.0)
            .padding_horiz(HORIZ_PAD)
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
    /// Panel background alpha. `None` = mode default (1.0 opaque, or 0.35
    /// tint when `blur` is enabled). `Some(v)` overrides.
    pub opacity: Option<f32>,
    /// Backdrop blur sigma. `Some(v)` enables capture + blur of the desktop
    /// behind the panel; `None` disables the effect.
    pub blur: Option<f32>,
    /// Cached fuzzy matcher (nucleo allocates ~135KB per instance).
    pub matcher: Matcher,
}

impl AppState {
    /// Re-rank entries against the current query.
    pub fn rerank(&mut self) {
        let query = self.picker.query.get();
        // Calc mode is reactive: the query IS the expression. We synthesize
        // a single entry per evaluation instead of fuzzy-ranking a static list.
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

    /// Switch to a new mode and reload entries.
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
    //
    // `bg` is always the opaque theme color — used wherever bg is treated as
    // a *color value* (selected-row text, status-bar fg, etc).
    //
    // `panel_bg` fills the outer container. `opacity` controls its alpha so
    // the compositor can show what's behind (alpha < 1.0). Every inner view
    // fills with `Color::TRANSPARENT` (see `inner_bg` below) to avoid
    // stacking fills that would cancel the transparency effect.
    let (bg, panel_bg, fg, accent, font_family, font_size, blur, opacity, panel_height) = {
        let s = state.lock().unwrap();
        let t = &s.theme;
        let bg = parse_color(&t.bg);
        // Mode-aware opacity default: 0.35 tint when blur is on (so the
        // backdrop shows through), 1.0 opaque otherwise. Explicit
        // `--opacity` always wins.
        let default_opacity = if s.blur.is_some() { 0.35 } else { 1.0 };
        let opacity = s.opacity.unwrap_or(default_opacity);
        let panel_bg = bg.multiply_alpha(opacity);
        let panel_h = crate::ui::view::INPUT_ROW_HEIGHT
            + ROW_HEIGHT * VISIBLE_ROWS as f64
            + STATUS_HEIGHT * 2.0
            + 6.0;
        (
            bg,
            panel_bg,
            parse_color(&t.fg),
            parse_color(&t.accent),
            t.font.clone(),
            t.font_size,
            s.blur,
            opacity,
            panel_h,
        )
    };
    // When the panel is opaque (no blur, no opacity override), inner views
    // fill with the same opaque bg so the wgpu surface — which has
    // PreMultiplied alpha — doesn't punch transparent holes through where
    // children paint over the parent's opaque background. When the panel
    // *is* translucent, inner_bg stays transparent so stacked fills don't
    // cancel the translucency.
    let inner_bg = if opacity >= 1.0 && blur.is_none() {
        panel_bg
    } else {
        Color::TRANSPARENT
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
    // Input row text is ~1.4× larger than result rows — Raycast pattern
    // for visual hierarchy between query and list.
    let input_font_size = font_size * 1.4;
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
                .font_size(input_font_size)
        })
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
            .font_size(input_font_size)
            .flex_grow(1.0)
            .margin_left(8.0)
    });

    // Input card: use a nested-container "border" so no center-aligned stroke
    // is needed. The outer container carries the border colour as its fill
    // (a 1px ring visible between it and the inner h_stack). The inner
    // h_stack has margin(1) to expose that ring. This avoids the corner-zone
    // stroke leak on a PreMultiplied-alpha surface.
    //
    // margin_bottom(10) lives on the OUTER container so the gap between the
    // card and the result list is covered by the parent v_stack's inner_bg.
    let input_border = blend(accent, bg, 0.35);
    let input_row = container(h_stack((prompt_label, query_label)).style(move |s| {
        s.width_full()
            .height(INPUT_ROW_HEIGHT - 2.0) // shrink by 2*margin(1)
            .padding_horiz(HORIZ_PAD)
            .items_center()
            .background(inner_bg)
            .border_radius(5.0) // outer radius(6) − margin(1)
            .margin(1.0)
    }))
    .style(move |s| {
        s.width_full()
            .height(INPUT_ROW_HEIGHT)
            .margin_bottom(10.0)
            .background(input_border)
            .border_radius(6.0)
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
            entry_row(entry, positions, mi, selected_sig, fg, accent, inner_bg)
                .style(move |s| s.font_family(ff.clone()).font_size(font_size))
        },
    )
    .style(move |s| {
        s.width_full()
            .flex_direction(FlexDirection::Column)
            .background(inner_bg)
    });

    // Viewport height is locked to an integer number of rows. Combined
    // with the fixed ROW_HEIGHT on each entry_row this guarantees the
    // scroll always shows whole rows — no half-clipped row at the
    // top/bottom edge as the user pages through.
    //
    // `ensure_visible` follows the cursor: every selection change emits
    // a Rect at the row's exact position and floem scrolls the viewport
    // just enough to keep it on-screen.
    let viewport_height = ROW_PITCH * VISIBLE_ROWS as f64;
    let scrollable = scroll(result_list)
        .ensure_visible(move || {
            // Vim-style scrolloff: the visibility rect spans the selected
            // row plus `SCROLLOFF` rows above and below it. floem scrolls
            // the moment any of that 3-row band would leave the viewport,
            // so the cursor never sits at the very edge before the list
            // moves.
            let sel = selected_sig.get() as f64;
            let start_row = (sel - SCROLLOFF).max(0.0);
            let end_row = sel + 1.0 + SCROLLOFF;
            let top = start_row * ROW_PITCH;
            let height = (end_row - start_row) * ROW_PITCH;
            Rect::from_origin_size((0.0, top), KurboSize::new(1.0, height))
        })
        .style(move |s| s.width_full().height(viewport_height).background(inner_bg));

    // ── Status bar ─────────────────────────────────────────────────────────
    // Hidden by default (height 0). The mode + selection text are still
    // computed so the bar can be flipped back on with a height bump.
    let _ = font_family.clone();
    let _ = vim_mode_sig;
    let _ = selected_sig;
    let status = floem::views::empty().style(|s| s.height(0.0).width_full());
    let _ = (bg, accent, font_size); // intentionally unused for the hidden status

    // ── Ex bar ─────────────────────────────────────────────────────────────
    let ex = ex_bar(ex_buf_sig, fg, inner_bg);

    // ── Glass overlays (smoked mode) ──────────────────────────────────────
    // When smoked, capture the desktop at startup, blur it heavily, and
    // render it behind the panel content. The glow overlay is always rendered
    // on top when smoked. If capture fails (grim absent, X11, etc.) the glow
    // still renders for a minimal glass look; if both are absent the panel
    // falls back to its flat-color background.
    //
    // Panel dimensions match app.rs: viewport_h + chrome_h. Recompute here
    // rather than threading an extra parameter so view.rs stays self-contained.
    let panel_w = 720_u32;
    let panel_h_u32 = panel_height as u32;

    // Panel corner radius — kept in step with the outer container's
    // `border_radius` below so the backdrop image clips to the same rounded
    // rect instead of leaking pixels into the corner cutouts.
    let panel_radius = 8.0_f64;
    let glass_overlays: Box<dyn floem::View> = if let Some(sigma) = blur {
        let backdrop: Box<dyn floem::View> =
            match crate::backdrop::capture_blurred(panel_w, panel_h_u32, sigma) {
                Some(bytes) => backdrop_overlay(bytes, panel_radius)
                    .style(|s| s.absolute().width_full().height_full())
                    .into_any(),
                None => floem::views::empty().into_any(),
            };
        let glow = glow_overlay(panel_height);
        floem::views::stack((backdrop, glow))
            .style(|s| s.absolute().width_full().height_full())
            .into_any()
    } else {
        floem::views::empty().into_any()
    };

    // panel_bg already bakes the mode-aware default opacity, so we use it
    // directly — no separate surface_bg path needed.
    let surface_bg = panel_bg;

    // Border ring thickness (logical px). Used to inset the inner container
    // so the outer accent fill shows as a ring of this width.
    const BORDER_W: f64 = 1.5;

    // ── Outer container with keyboard handler ──────────────────────────────
    // The "border" is rendered as a nested-container paint ring rather than
    // a stroked path. Floem's paint_border uses a center-aligned stroke: its
    // outer half falls outside the rounded-rect bg fill on a
    // PreMultiplied-alpha surface, and those pixels land on the transparent
    // framebuffer → visible corner halos. Using opaque fills eliminates that.
    //
    // Layout:
    //   outer container — background(accent), border_radius(panel_radius)
    //     inner container — background(surface_bg), border_radius(inner_r),
    //                       margin(BORDER_W), height/width_full
    //       stack(glass, v_stack) — the actual content
    let inner_r = (panel_radius - BORDER_W).max(0.0);
    let state_key = Arc::clone(&state);
    container(
        container(
            floem::views::stack((
                // glass FIRST so it paints below, content AFTER so it paints on
                // top. Stack order = paint order in floem.
                glass_overlays,
                v_stack((input_row, scrollable, ex.into_any(), status)).style(move |s| {
                    // 10px outer inset for all children. Paints inner_bg in
                    // its full bounds (including the padding region) so no
                    // unpainted strip can leak the framebuffer.
                    s.width_full()
                        .height_full()
                        .padding(10.0)
                        .background(inner_bg)
                }),
            ))
            .style(move |s| s.width_full().height_full().background(inner_bg)),
        )
        .style(move |s| {
            s.width_full()
                .height_full()
                .background(surface_bg)
                .border_radius(inner_r)
        }),
    )
    .style(move |s| {
        // Outer accent fill is the panel border. No stroke needed: the accent
        // background is visible as a BORDER_W-pixel ring between the outer
        // container's rounded edge and the inner container's inset edge.
        // `padding(BORDER_W)` creates the inset gap — more reliable than
        // `margin` on the child because `height_full()` inside a padded
        // container resolves against the content box height (after padding).
        s.width_full()
            .height_full()
            .background(accent)
            .border_radius(panel_radius)
            .padding(BORDER_W)
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
}
