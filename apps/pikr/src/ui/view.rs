//! The picker's UI tree, rebuilt from the session on every frame —
//! tokyonight/rofi-style layout. Styling lives in `styles/default.css`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use crcbl_image::Rgba8Image;
use crcbl_ui::font::Font;
use crcbl_ui::font::layout::TextLayout;
use crcbl_ui::image::{AtlasImage, ImageAtlas};
use crcbl_ui::style::{Declaration, Sides};
use crcbl_ui::tree::{LengthAuto, NodeKey, Position, Ui};
use glam::Vec2;

use crate::picker::icons::IconCache;
use crate::picker::query_text::{empty_state_text, mask_password};
use crate::picker::session::Session;
use crate::picker::state::VimMode;

// ─── Layout constants ────────────────────────────────────────────────────────
// They mirror `default.css`, for sizing the window before anything is laid
// out; keep the two in step.

pub const ROW_HEIGHT: f64 = 36.0;
pub const ROW_GAP: f64 = 4.0;
pub const ROW_PITCH: f64 = ROW_HEIGHT + ROW_GAP;
pub const VISIBLE_ROWS: usize = 8;
pub const INPUT_ROW_HEIGHT: f64 = ROW_HEIGHT;
/// Margin between the input row and the result list.
pub const INPUT_MARGIN_BOTTOM: f64 = 8.0;
pub const PANEL_PAD: f64 = 10.0;
/// The panel's border width; the panel is border-box, so it eats into the
/// height too.
pub const PANEL_BORDER: f64 = 1.5;
/// The panel's corner radius.
pub const PANEL_RADIUS: f64 = 10.0;
/// The ex bar: height + top margin + bottom margin.
pub const EX_BAR_TOTAL: f64 = 22.0 + 8.0 + 2.0;
/// The status bar's height, padding included.
pub const STATUS_BAR_TOTAL: f64 = 28.0;
/// Rows kept visible past the cursor row when the list scrolls to it.
const SCROLLOFF: f64 = 1.0;
/// Rendered icon edge in logical pixels.
pub const ICON_SIZE: u32 = 24;
/// The Insert-mode caret's width (`.caret`).
const CARET_WIDTH: f32 = 2.0;

/// The window height for `rows` visible result rows.
pub fn window_height(rows: usize) -> f64 {
    ROW_PITCH * rows as f64
        + PANEL_PAD * 2.0
        + PANEL_BORDER * 2.0
        + INPUT_ROW_HEIGHT
        + INPUT_MARGIN_BOTTOM
        + EX_BAR_TOTAL
        + STATUS_BAR_TOTAL
}

/// The `-P` window height: the input row and status bar, with no list and
/// no ex bar.
pub fn password_window_height() -> f64 {
    window_height(0) - EX_BAR_TOTAL
}

// ─── Icons ───────────────────────────────────────────────────────────────────

/// Icons registered in the renderer's image atlas, by file.
pub struct Icons {
    cache: IconCache,
    /// The decoded image each file's registration was made from, so a file
    /// decoded afresh (changed on disk) is registered again.
    registered: HashMap<PathBuf, (Arc<Rgba8Image>, AtlasImage)>,
    /// Files drawn this frame, kept when the atlas has to make room.
    used: HashSet<PathBuf>,
    /// Device pixels per logical pixel, rounded up.
    pub scale: u32,
}

impl Icons {
    /// Icons looked up in `themes` first; see [`IconCache::new`].
    pub fn new(themes: &[String]) -> Self {
        Self {
            cache: IconCache::new(themes),
            registered: HashMap::new(),
            used: HashSet::new(),
            scale: 1,
        }
    }

    /// Starts a frame's bookkeeping of which icons are on screen.
    pub fn begin_frame(&mut self) {
        self.used.clear();
    }

    /// The atlas image for an entry's `Icon=` value, registering it on first
    /// use. When the atlas is full, icons not drawn this frame make room.
    fn get(&mut self, atlas: &mut ImageAtlas, icon: Option<&str>) -> Option<AtlasImage> {
        let px = ICON_SIZE * self.scale;
        let path = self.cache.resolve(icon, ICON_SIZE, self.scale)?;
        let image = self.cache.image(&path, px)?;
        self.used.insert(path.clone());
        if let Some((drawn, registered)) = self.registered.get(&path)
            && Arc::ptr_eq(drawn, &image)
        {
            return Some(*registered);
        }
        if let Some((_, stale)) = self.registered.remove(&path) {
            atlas.unregister(stale.id);
        }
        let register =
            |atlas: &mut ImageAtlas| atlas.register(image.width(), image.height(), image.pixels());
        let registered = match register(atlas) {
            Ok(registered) => registered,
            Err(_) => {
                let used = &self.used;
                self.registered.retain(|path, (_, registered)| {
                    let keep = used.contains(path);
                    if !keep {
                        atlas.unregister(registered.id);
                    }
                    keep
                });
                register(atlas)
                    .inspect_err(|e| tracing::debug!("icon atlas full: {e}"))
                    .ok()?
            }
        };
        self.registered.insert(path, (image, registered));
        Some(registered)
    }
}

// ─── The view ────────────────────────────────────────────────────────────────

/// What the view keeps between frames.
pub struct View {
    /// The result list, as last frame built it.
    list: Option<NodeKey>,
    /// The selection, and the query its rows were ranked for, that the list
    /// was last scrolled to: wheel scrolling is left alone until either
    /// changes, so a new ranking brings its best match back into view.
    scrolled_to: Option<(usize, String)>,
    /// The caret's blink phase.
    pub blink_on: bool,
    /// An input method's composition in progress, drawn at the caret.
    pub preedit: Preedit,
    /// The query caret, as last frame built it.
    caret: Option<NodeKey>,
    /// What the query is drawn in, to place the caret in the same layout.
    query_font: QueryFont,
}

/// The face and size the stylesheet gives the query's text: the theme font,
/// or crcbl's sans-serif without one, carrying the fallback faces, as
/// crcbl-ui draws the span.
pub struct QueryFont {
    pub font: Font,
    pub size: f32,
}

/// An input method's composition: its text, and where its own cursor sits in
/// it as a byte offset on a `char` boundary (`None`: at the end).
#[derive(Default)]
pub struct Preedit {
    pub text: String,
    pub cursor: Option<usize>,
}

impl View {
    /// Whether last frame built the result list.
    #[cfg(test)]
    pub fn built_a_list(&self) -> bool {
        self.list.is_some()
    }

    pub fn new(query_font: QueryFont) -> Self {
        Self {
            list: None,
            scrolled_to: None,
            blink_on: true,
            preedit: Preedit::default(),
            caret: None,
            query_font,
        }
    }

    /// Builds the picker for `session`. Returns the result row clicked this
    /// frame, if any.
    pub fn build(
        &mut self,
        ui: &mut Ui,
        session: &Session,
        icons: &mut Icons,
        atlas: &mut ImageAtlas,
    ) -> Option<usize> {
        let app = &session.app;
        let picker = &app.picker;
        self.scroll_to_selection(ui, picker.selected, &picker.query, app.matches.is_empty());
        let mut clicked = None;
        // Split before masking: the cursor is a byte offset into the real
        // text.
        let preedit = &self.preedit.text;
        let at = self
            .preedit
            .cursor
            .filter(|&i| preedit.is_char_boundary(i))
            .unwrap_or(preedit.len());
        let composing = [
            mask_password(app.password, &preedit[..at]),
            mask_password(app.password, &preedit[at..]),
        ];
        // The composition is drawn at the caret taking the keys: the `:`
        // prompt's while it is open, which its commit goes to.
        let (query_preedit, ex_preedit) = if picker.ex_buf.is_some() {
            (Default::default(), composing)
        } else {
            (composing, Default::default())
        };
        ui.block(".panel", &[], |ui| {
            ui.block(".input-row", &[], |ui| {
                let prompt = if app.prompt.is_empty() {
                    ">".to_string()
                } else {
                    format!("{}:", app.prompt)
                };
                ui.span(".prompt", prompt.as_str(), &[]);
                let shown = mask_password(app.password, &picker.query);
                self.caret = query(
                    ui,
                    &self.query_font,
                    &shown,
                    picker.query_cursor,
                    picker.vim_mode,
                    self.blink_on,
                    &query_preedit,
                );
            });

            // -P and --lines 0 are bare prompts: no list under them.
            if !app.lists_rows() {
                self.list = None;
            } else if let Some(text) = &session.loading {
                ui.block(".loading", &[], |ui| {
                    ui.span(".loading-text", text.as_str(), &[]);
                });
            } else {
                ui.block(".results", &[], |ui| {
                    if app.matches.is_empty() {
                        self.list = None;
                        ui.block(".empty-row", &[], |ui| {
                            let text = empty_state_text(&picker.query);
                            ui.span(".empty-row-text", text.as_str(), &[]);
                            ui.block(".spacer", &[], |_| {});
                            confirm_card(ui, session);
                        });
                        return;
                    }
                    // Selected for screen readers: the row Return accepts.
                    let list = ui.selectable_list(
                        "#results",
                        app.matches.len(),
                        ROW_PITCH as f32,
                        Some(picker.selected),
                        |ui, mi| {
                            let row = result_row(ui, session, mi, icons, atlas);
                            if row {
                                clicked = Some(mi);
                            }
                        },
                    );
                    self.list = Some(list.key);
                    ui.scrollbar("#results-bar", list.key);
                });
            }

            // -P never opens the `:` prompt, so its bar is left out too.
            if !app.password {
                ui.block(".ex-bar", &[], |ui| {
                    if let Some(buf) = &picker.ex_buf {
                        let line = format!(":{buf}");
                        let end = line.chars().count();
                        // The input method composes at whichever caret is
                        // taking keys, so it follows this one while open.
                        self.caret = query(
                            ui,
                            &self.query_font,
                            &line,
                            end,
                            VimMode::Insert,
                            self.blink_on,
                            &ex_preedit,
                        );
                    }
                });
            }

            ui.block(".status-bar", &[], |ui| {
                let (chip, label) = match picker.vim_mode {
                    VimMode::Insert => (".mode-chip.insert", "INSERT"),
                    VimMode::Normal => (".mode-chip.normal", "NORMAL"),
                    VimMode::Visual => (".mode-chip.visual", "VISUAL"),
                };
                ui.span(chip, label, &[]);
                let mode_name = if app.password {
                    "password".to_string()
                } else {
                    app.cli_mode.key()
                };
                ui.span(".mode-name", mode_name.as_str(), &[]);
                ui.block(".spacer", &[], |_| {});
                if !app.lists_rows() {
                    return;
                }
                let total = app.matches.len();
                let count = if total == 0 {
                    "0/0".to_string()
                } else {
                    format!("{}/{}", picker.selected + 1, total)
                };
                ui.span(".count-chip", count.as_str(), &[]);
            });
        });
        clicked
    }

    /// The caret's rectangle in logical pixels, as last frame laid it out.
    pub fn caret_rect(&self, ui: &Ui) -> Option<(Vec2, Vec2)> {
        ui.rect(self.caret?)
    }

    /// Scrolls the list so the selection, and [`SCROLLOFF`] rows around it,
    /// are in view — once per selection or query change, so the wheel can
    /// still move the list away from it.
    fn scroll_to_selection(&mut self, ui: &mut Ui, selected: usize, query: &str, empty: bool) {
        let Some(list) = self.list else {
            return;
        };
        if empty
            || self
                .scrolled_to
                .as_ref()
                .is_some_and(|(at, ranked)| *at == selected && ranked == query)
        {
            return;
        }
        let Some((min, max)) = ui.rect(list) else {
            return;
        };
        self.scrolled_to = Some((selected, query.to_owned()));
        let view = f64::from(max.y - min.y);
        let offset = f64::from(ui.scroll_offset_of(list).y);
        let top = (selected as f64 - SCROLLOFF).max(0.0) * ROW_PITCH;
        let bottom = (selected as f64 + 1.0 + SCROLLOFF) * ROW_PITCH;
        let next = if top < offset {
            top
        } else if bottom > offset + view {
            bottom - view
        } else {
            return;
        };
        ui.set_scroll_offset_of(list, Vec2::new(0.0, next as f32));
    }
}

/// The query text with its caret: a thin bar between characters in Insert,
/// a block over the character under it in Normal and Visual. The caret hides
/// on the blink-off phase without moving the text. In Insert mode an input
/// method's `preedit`, split at its own cursor, is drawn around the caret.
/// Returns the caret's node. Screen readers hear an editable field holding
/// `text`, with the caret at `cursor`.
///
/// The text is one span and the caret is laid over it where crcbl's layout
/// of the same text puts it, so right-to-left and mixed text, which bidi
/// reorders, get their caret in the right place. Only a composition splits
/// the text, to underline the pre-edit.
fn query(
    ui: &mut Ui,
    font: &QueryFont,
    text: &str,
    cursor: usize,
    mode: VimMode,
    blink_on: bool,
    preedit: &[String; 2],
) -> Option<NodeKey> {
    let chars: Vec<char> = text.chars().map(uncollapsed).collect();
    let cursor = cursor.min(chars.len());
    let blink = if blink_on { ".blink-on" } else { "" };
    if mode == VimMode::Insert && preedit.iter().any(|part| !part.is_empty()) {
        let (field, caret) = composing(ui, &chars, cursor, &format!(".caret{blink}"), preedit);
        accessible_field(ui, field, text, cursor);
        return caret;
    }
    let shown: String = chars.iter().collect();
    let byte = shown
        .char_indices()
        .nth(cursor)
        .map_or(shown.len(), |(byte, _)| byte);
    let line_height = font.font.metrics().normal_line_height(font.size);
    let x = TextLayout::new(&font.font, &shown, font.size, line_height, None)
        .caret(byte)
        .x;
    let mut caret = None;
    let field = ui.block(".query", &[], |ui| {
        ui.span(".query-text", shown.as_str(), &[]);
        match mode {
            VimMode::Insert => {
                let at = laid_over(x);
                caret = Some(ui.block(&format!(".caret{blink}"), &at, |_| {}).key);
            }
            VimMode::Normal | VimMode::Visual => {
                // Past the end the block sits on a no-break space.
                let under = chars.get(cursor).copied().unwrap_or(NBSP).to_string();
                let glyph = TextLayout::new(&font.font, &under, font.size, line_height, None);
                // A right-to-left char's caret stands at its right edge.
                let width = glyph.width();
                let left = if glyph.caret(0).x > 0.0 { x - width } else { x };
                let at = laid_over(left);
                let block = ui.span(&format!(".caret-block{blink}"), under.as_str(), &at);
                caret = Some(block.key);
            }
        }
    });
    accessible_field(ui, field.key, text, cursor);
    caret
}

/// Marks `field` for screen readers as an editable field holding `text`,
/// with the caret before char `cursor`. Visual mode selects rows, not text,
/// so the field never has a selection.
fn accessible_field(ui: &mut Ui, field: NodeKey, text: &str, cursor: usize) {
    let at = text
        .char_indices()
        .nth(cursor)
        .map_or(text.len(), |(byte, _)| byte);
    ui.set_accessible_text_input(field, text, at..at);
}

/// Inline style placing a caret `x` pixels from the query's left, over its
/// text rather than beside it.
fn laid_over(x: f32) -> [Declaration; 2] {
    [
        Declaration::Position(Position::Absolute),
        Declaration::Inset(Sides::Left, LengthAuto::Px(x)),
    ]
}

/// The query while an input method composes: the text split around the
/// caret, the pre-edit underlined at it. Bidi cannot reorder across the
/// split, so here a right-to-left query still puts the caret at the split.
/// Returns the query's node and the caret's.
fn composing(
    ui: &mut Ui,
    chars: &[char],
    cursor: usize,
    caret_class: &str,
    preedit: &[String; 2],
) -> (NodeKey, Option<NodeKey>) {
    let preedit = preedit
        .each_ref()
        .map(|part| part.chars().map(uncollapsed).collect::<String>());
    let before: String = chars[..cursor].iter().collect();
    let after: String = chars[cursor..].iter().collect();
    let mut caret = None;
    let field = ui.block(".query", &[], |ui| {
        ui.span(".query-text", before.as_str(), &[]);
        ui.span(".preedit", preedit[0].as_str(), &[]);
        // In the row here, not laid over it: taking back its own width keeps
        // the text from moving as it does.
        let width_taken_back = [Declaration::Margin(
            Sides::Right,
            LengthAuto::Px(-CARET_WIDTH),
        )];
        caret = Some(ui.block(caret_class, &width_taken_back, |_| {}).key);
        ui.span(".preedit", preedit[1].as_str(), &[]);
        ui.span(".query-text", after.as_str(), &[]);
    });
    (field.key, caret)
}

/// U+00A0, a space crcbl-ui never collapses.
const NBSP: char = '\u{A0}';

/// `c` as the query and the result rows draw it. crcbl-ui drops plain spaces
/// at the end of a span (it has no `white-space: pre`), so a query ending in
/// spaces drew the caret left of where typing goes (seen on Windows with
/// `--filter "a  b  "`), and a space next to a highlighted run vanished
/// ("client for" drew as "clientfor"). A no-break space is drawn the same
/// width and keeps the character count, so the caret's char index still
/// lines up.
fn uncollapsed(c: char) -> char {
    if c == ' ' { NBSP } else { c }
}

/// Result row `mi`. Returns whether it was clicked.
fn result_row(
    ui: &mut Ui,
    session: &Session,
    mi: usize,
    icons: &mut Icons,
    atlas: &mut ImageAtlas,
) -> bool {
    let app = &session.app;
    let picker = &app.picker;
    let m = &app.matches[mi];
    let entry = &app.entries[m.index];
    let in_visual_range = picker.visual_anchor.is_some_and(|a| {
        let (lo, hi) = (a.min(picker.selected), a.max(picker.selected));
        (lo..=hi).contains(&mi)
    });
    let class = if mi == picker.selected {
        ".row.selected"
    } else if in_visual_range {
        ".row.visual"
    } else {
        ".row"
    };
    ui.block(class, &[], |ui| {
        match icons.get(atlas, entry.icon.as_deref()) {
            Some(icon) => {
                ui.span(".icon", icon, &[]);
            }
            None => {
                ui.block(".icon-slot", &[], |_| {});
            }
        }
        highlighted(ui, ".label", &entry.label, &m.positions);
        if let Some(desc) = &entry.description {
            // The positions index the description; the parentheses wrap it.
            let shifted: Vec<u32> = m.desc_positions.iter().map(|p| p + 1).collect();
            highlighted(ui, ".desc", &format!("({desc})"), &shifted);
        }
        ui.block(".spacer", &[], |_| {});
        if mi == picker.selected {
            confirm_card(ui, session);
        }
    });
    ui.clicked()
}

/// `text` as a row of runs, the characters at `positions` in the accent.
/// Named `text` for screen readers, which would otherwise read it run by run.
fn highlighted(ui: &mut Ui, selector: &str, text: &str, positions: &[u32]) {
    let block = ui.block(selector, &[], |ui| {
        for (run, matched) in runs(text, positions) {
            let shown: String = run.chars().map(uncollapsed).collect();
            ui.span(if matched { ".hl" } else { ".tx" }, shown.as_str(), &[]);
        }
    });
    ui.set_accessible_label(block.key, text);
}

/// `text` split into maximal runs of matched and unmatched characters.
fn runs<'a>(text: &'a str, positions: &[u32]) -> Vec<(&'a str, bool)> {
    let matched: HashSet<u32> = positions.iter().copied().collect();
    let mut runs: Vec<(&str, bool)> = Vec::new();
    let mut start = 0;
    let mut current = None;
    for (i, (byte, _)) in text.char_indices().enumerate() {
        let is_match = matched.contains(&(i as u32));
        if current.is_some_and(|c| c != is_match) {
            runs.push((&text[start..byte], !is_match));
            start = byte;
        }
        current = Some(is_match);
    }
    if let Some(is_match) = current {
        runs.push((&text[start..], is_match));
    }
    runs
}

/// The `--kb-custom KEY=PROMPT` confirm card, while one is pending.
fn confirm_card(ui: &mut Ui, session: &Session) {
    if let Some(card) = &session.confirm {
        let text = format!("{}  \u{21B5}", card.prompt);
        ui.span(".confirm-card", text.as_str(), &[]);
    }
}

/// The `--message` modal: `text` centred in the panel chrome.
pub fn build_message(ui: &mut Ui, text: &str) {
    ui.block(".message", &[], |ui| {
        ui.span(".message-text", text, &[]);
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn query_spaces_are_drawn_as_no_break_spaces() {
        let shown: String = "a  b ".chars().map(super::uncollapsed).collect();
        assert_eq!(shown, "a\u{A0}\u{A0}b\u{A0}");
        assert_eq!(shown.chars().count(), "a  b ".chars().count());
    }

    use super::runs;

    #[test]
    fn runs_split_at_match_boundaries() {
        assert_eq!(
            runs("firefox", &[0, 1, 4]),
            [("fi", true), ("re", false), ("f", true), ("ox", false)]
        );
    }

    #[test]
    fn runs_cover_multibyte_text_by_char_index() {
        assert_eq!(
            runs("héllo", &[1]),
            [("h", false), ("é", true), ("llo", false)]
        );
    }

    #[test]
    fn runs_of_unmatched_and_empty_text() {
        assert_eq!(runs("abc", &[]), [("abc", false)]);
        assert!(runs("", &[0]).is_empty());
    }
}
