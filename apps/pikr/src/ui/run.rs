//! The window and its event loop: a crcbl panel, its GPU swapchain, and the
//! picker (or `--message` modal) drawn into it whenever something changed.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use crcbl_shell::{
    ButtonState, ClipboardContent, ClipboardRequestId, CloseReply, CursorIcon, LogicalSize,
    MimeType, PanelAnchor, PanelDesc, PanelKeyboard, PanelLayer, PhysicalRect, PointerButton,
    ScrollDelta, Shell, ShellCaps, ShellEvent, TextPurpose, WindowDesc, WindowId, WindowRole,
};
use crcbl_ui::draw_list::DrawList;
use crcbl_ui::font::Font;
use crcbl_ui::font::system::crcbl_font::{FontQuery, SystemFonts};
use crcbl_ui::text::FontAtlas;
use crcbl_ui::tree::{AvailableSpace, Ui};
use crcbl_ui::widget::PointerInput;
use glam::Vec2;

use crate::modes::{self, dmenu::LoadedEntries};
use crate::picker::session::{Outcome, Session};
use crate::picker::state::VimMode;
use crate::ui::gpu::Gpu;
use crate::ui::keyboard::{key_from_commit, key_from_press, modifiers_from_crcbl};
use crate::ui::view::{Icons, PANEL_RADIUS, Preedit, View};

/// The caret's blink half-period: crcbl's own, so pikr's query caret blinks
/// with every crcbl text field.
const BLINK: Duration = crcbl_ui::console::CARET_BLINK;
/// How long the window system gets to size a new window before pikr gives up.
const CONFIGURE_TIMEOUT: Duration = Duration::from_secs(5);
/// Pixels one wheel detent scrolls.
const WHEEL_LINE_PIXELS: f32 = 40.0;
/// How often `--loading` checks for its rows when the window system gave no
/// way to wake the loop.
const LOADING_POLL: Duration = Duration::from_millis(50);
/// How long to wait for the window system after an out-of-date swapchain
/// before drawing again: a new size is usually on its way.
const PRESENT_RETRY: Duration = Duration::from_millis(16);
/// The `--message` modal's height.
pub const MESSAGE_HEIGHT: f64 = 120.0;

/// What the window shows.
pub enum Content {
    Picker {
        session: Box<Session>,
        /// `--loading` rows still being read from stdin.
        pending: Option<mpsc::Receiver<LoadedEntries>>,
    },
    /// `--message`: text, dismissed with Escape.
    Message(String),
}

/// The window's setup, from the CLI and config.
pub struct WindowSpec {
    pub width: f64,
    pub height: f64,
    pub stylesheet: String,
    pub font_family: String,
}

/// Opens the window, runs until the picker finishes, then — window gone —
/// runs what it chose and exits the process.
pub fn run(spec: WindowSpec, content: Content, startup_started: Instant) -> Result<()> {
    // Finding and reading the fonts overlaps opening the window and the
    // GPU, the slow part of startup.
    let fonts = {
        let family = spec.font_family.clone();
        std::thread::spawn(move || load_fonts(&family))
    };
    let mut shell = crcbl_shell::open().context("opening the window system")?;
    let (window, transparent_window) = create_window(shell.as_mut(), &spec)?;
    let (extent, early) = wait_for_size(shell.as_mut(), window)?;
    let mut gpu = Gpu::open(&shell.surface_target(window)?, extent, transparent_window)?;
    let fonts = fonts.join().unwrap_or_else(|_| Fonts {
        theme: Err("the font loader panicked".into()),
        fallbacks: Vec::new(),
    });
    let outcome = Window::new(
        shell.as_mut(),
        window,
        &spec,
        fonts,
        &mut gpu,
        startup_started,
    )
    .and_then(|mut w| w.run(content, early));
    // The surface goes before the window it was made from.
    gpu.destroy();
    drop(shell);
    let (payloads, code) = outcome?;
    for payload in &payloads {
        if let Err(e) = modes::execute(payload) {
            eprintln!("pikr: execute error: {e}");
        }
    }
    std::process::exit(code);
}

/// A centred overlay panel taking the keyboard, where the window system has
/// panels (Wayland layer-shell, Win32 tool windows, AppKit panels);
/// otherwise an ordinary window. Transparent where the platform composites
/// window alpha, so the panel's rounded corners show the desktop.
fn create_window(shell: &mut dyn Shell, spec: &WindowSpec) -> Result<(WindowId, bool)> {
    let caps = shell.caps();
    let role = if caps.contains(ShellCaps::PANEL) {
        WindowRole::Panel(PanelDesc {
            namespace: "pikr",
            // No edge anchors: the panel is centred on the output.
            anchor: PanelAnchor::empty(),
            // Overlay rather than Top, so pikr shows above a fullscreen app
            // instead of opening hidden behind it.
            layer: PanelLayer::Overlay,
            // Every key comes to pikr while it is open, as rofi and wofi do.
            keyboard: PanelKeyboard::Exclusive,
            ..PanelDesc::default()
        })
    } else {
        tracing::debug!(backend = ?shell.backend(), "no panel windows; opening a toplevel");
        WindowRole::Toplevel
    };
    let transparent = caps.contains(ShellCaps::TRANSPARENT);
    if !transparent {
        tracing::warn!(
            backend = ?shell.backend(),
            "the window system cannot make pikr transparent; drawing it opaque, so the corners outside the rounded panel show black"
        );
    }
    let window = shell
        .create_window(&WindowDesc {
            title: "pikr",
            app_id: "pikr",
            size: LogicalSize::new(spec.width, spec.height),
            resizable: false,
            role,
            transparent,
            ..WindowDesc::default()
        })
        .context("creating the window")?;
    Ok((window, transparent))
}

/// Pumps until the window system gives the new window a size. Returns the
/// size and the events pumped on the way, which the window loop handles
/// first: a refused focus, the scale factor and an early focus all arrive
/// here.
fn wait_for_size(shell: &mut dyn Shell, window: WindowId) -> Result<((u32, u32), Vec<ShellEvent>)> {
    let started = Instant::now();
    let mut events = Vec::new();
    loop {
        shell.pump(&mut |event| events.push(event));
        if let Some(size) = shell.window_state(window)?.size() {
            return Ok(((size.width, size.height), events));
        }
        if started.elapsed() > CONFIGURE_TIMEOUT {
            return Err(anyhow!("the window system never sized pikr's window"));
        }
        shell.wait_events(Some(Duration::from_millis(8)));
    }
}

struct Window<'a> {
    shell: &'a mut dyn Shell,
    window: WindowId,
    gpu: &'a mut Gpu,
    ui: Ui,
    atlas: FontAtlas,
    list: DrawList,
    view: View,
    icons: Icons,
    /// Window size in device pixels, and device pixels per logical pixel.
    extent: (u32, u32),
    scale: f32,
    pointer: Vec2,
    pointer_down: bool,
    /// Wheel movement not yet applied, in logical pixels.
    wheel: f32,
    cursor: Option<CursorIcon>,
    /// The caret rectangle last handed to the input method.
    input_area: Option<PhysicalRect>,
    /// What the input method was last told is taking typing.
    text_input: Option<TextPurpose>,
    /// The clipboard read a paste is waiting on.
    paste_request: Option<ClipboardRequestId>,
    focused: bool,
    blink_at: Instant,
    /// Modifiers held at the last key press, for the text commit after it.
    mods: crate::picker::input::Modifiers,
    startup_started: Instant,
    painted: bool,
    focus_logged: bool,
}

impl<'a> Window<'a> {
    fn new(
        shell: &'a mut dyn Shell,
        window: WindowId,
        spec: &WindowSpec,
        fonts: Fonts,
        gpu: &'a mut Gpu,
        startup_started: Instant,
    ) -> Result<Self> {
        let state = shell.window_state(window)?;
        let size = state.size().unwrap_or_default();
        let mut ui = Ui::new();
        ui.add_stylesheet("pikr", &spec.stylesheet);
        // Registered under the theme's family, which the stylesheet names;
        // without any face, text is drawn in crcbl's built-in sans-serif.
        let family = &spec.font_family;
        if let Err(e) = fonts
            .theme
            .and_then(|font| ui.register_font(family, font).map_err(|e| e.to_string()))
        {
            tracing::warn!(family, "theme font not used: {e}");
        }
        if fonts.fallbacks.is_empty() {
            tracing::warn!("no fallback fonts: chars the theme font lacks draw as boxes");
        }
        ui.set_fallback_fonts(fonts.fallbacks);
        Ok(Self {
            shell,
            window,
            gpu,
            ui,
            atlas: FontAtlas::built_in(),
            list: DrawList::new(),
            view: View::new(),
            icons: Icons::new(),
            extent: (size.width, size.height),
            scale: 1.0,
            pointer: Vec2::ZERO,
            pointer_down: false,
            wheel: 0.0,
            cursor: Some(CursorIcon::Default),
            input_area: None,
            text_input: None,
            paste_request: None,
            focused: false,
            blink_at: Instant::now() + BLINK,
            mods: Default::default(),
            startup_started,
            painted: false,
            focus_logged: false,
        })
    }

    /// Runs until the content is done; returns what to run and the exit code.
    /// `early` are events pumped before the window loop started.
    fn run(
        &mut self,
        mut content: Content,
        mut early: Vec<ShellEvent>,
    ) -> Result<(Vec<modes::Payload>, i32)> {
        self.sync_text_input(&content)?;
        self.update_input_region()?;
        // `--loading` rows, and whether their arrival wakes the loop.
        let mut arrived = match &mut content {
            Content::Picker { pending, .. } => pending.take().map(|rx| self.forward(rx)),
            Content::Message(_) => None,
        };

        let mut dirty = true;
        loop {
            if let (Some((rx, _)), Content::Picker { session, .. }) = (&arrived, &mut content)
                && let Ok(loaded) = rx.try_recv()
            {
                match loaded {
                    Ok(entries) => session.entries_arrived(&entries),
                    // Same outcome as a read error without --loading.
                    Err(e) => return Err(anyhow!("reading stdin: {e}")),
                }
                arrived = None;
                dirty = true;
            }

            let mut events = std::mem::take(&mut early);
            self.shell.pump(&mut |event| events.push(event));
            for event in events {
                match self.handle(event, &mut content)? {
                    Step::Continue(changed) => dirty |= changed,
                    Step::Exit(payloads, code) => {
                        if let Content::Picker { session, .. } = &content {
                            session.save();
                        }
                        return Ok((payloads, code));
                    }
                }
            }

            self.sync_text_input(&content)?;

            if self.focused && Instant::now() >= self.blink_at {
                self.view.blink_on = !self.view.blink_on;
                self.blink_at = Instant::now() + BLINK;
                dirty = true;
            }

            if dirty {
                let pointer = PointerInput {
                    down: self.pointer_down,
                    ..PointerInput::hovering(self.pointer)
                };
                self.frame(&content, pointer)?;
                if self.gpu.present(&self.list, &self.atlas)? {
                    dirty = false;
                    self.log_first_paint();
                } else {
                    // Out of date and reconfigured. Retrying at once would
                    // spin while it stays out of date (a window with no size).
                    self.shell.wait_events(Some(PRESENT_RETRY));
                }
                continue;
            }
            let mut timeout = self
                .focused
                .then(|| self.blink_at.saturating_duration_since(Instant::now()));
            if matches!(arrived, Some((_, false))) {
                timeout = Some(timeout.map_or(LOADING_POLL, |t| t.min(LOADING_POLL)));
            }
            self.shell.wait_events(timeout);
        }
    }

    /// Where the window is transparent, takes the pointer only inside the
    /// rounded panel, so a click on a see-through corner reaches the window
    /// beneath. The region is the corners' outer bound: on Win32 it also
    /// clips drawing, and must not cut the antialiased edge.
    fn update_input_region(&mut self) -> Result<()> {
        let caps = self.shell.caps();
        if !caps.contains(ShellCaps::INPUT_REGION) || !caps.contains(ShellCaps::TRANSPARENT) {
            return Ok(());
        }
        let (width, height) = self.extent;
        let radius = (PANEL_RADIUS * f64::from(self.scale)).ceil() as u32;
        let region = PhysicalRect::new(0, 0, width, height).rounded(radius);
        self.shell.set_input_region(self.window, Some(&region))?;
        Ok(())
    }

    /// What the loop does after the session's `outcome`.
    fn step(&mut self, outcome: Outcome) -> Step {
        match outcome {
            Outcome::Continue => Step::Continue(true),
            Outcome::Exit { payloads, code } => Step::Exit(payloads, code),
            Outcome::Paste => {
                match self
                    .shell
                    .clipboard_request(self.window, MimeType::TextUtf8)
                {
                    Ok(request) => {
                        tracing::debug!(?request, "clipboard read requested");
                        self.paste_request = Some(request);
                    }
                    Err(e) => tracing::warn!("cannot read the clipboard: {e}"),
                }
                Step::Continue(false)
            }
        }
    }

    /// Tells the input method whether the query is taking typing, whenever
    /// that changes. Turning it off drops any composition in progress.
    fn sync_text_input(&mut self, content: &Content) -> Result<()> {
        let purpose = text_purpose(content);
        if purpose != self.text_input {
            self.text_input = purpose;
            self.shell.set_text_input(self.window, purpose)?;
            if purpose.is_none() {
                self.view.preedit = Preedit::default();
            }
        }
        Ok(())
    }

    /// Relays `--loading` rows from the reader thread, waking the loop when
    /// they arrive. The flag says whether it can: without a waker the loop
    /// has to poll for them.
    fn forward(
        &mut self,
        rx: mpsc::Receiver<LoadedEntries>,
    ) -> (mpsc::Receiver<LoadedEntries>, bool) {
        let (tx, forwarded) = mpsc::channel();
        let waker = self.shell.waker();
        let wakes = waker.is_some();
        if !wakes {
            tracing::warn!("the window system cannot wake pikr; polling for --loading rows");
        }
        std::thread::spawn(move || {
            if let Ok(loaded) = rx.recv() {
                let _ = tx.send(loaded);
                if let Some(waker) = waker {
                    waker.wake();
                }
            }
        });
        (forwarded, wakes)
    }

    fn handle(&mut self, event: ShellEvent, content: &mut Content) -> Result<Step> {
        match event {
            ShellEvent::Resized {
                size, scale_factor, ..
            }
            | ShellEvent::ScaleFactorChanged {
                size, scale_factor, ..
            } => {
                self.extent = (size.width, size.height);
                self.scale = scale_factor as f32;
                self.icons.scale = scale_factor.ceil().max(1.0) as u32;
                self.gpu.resize(self.extent)?;
                self.update_input_region()?;
                Ok(Step::Continue(true))
            }
            ShellEvent::CloseRequested { window } => {
                self.shell.reply_close_request(window, CloseReply::Keep)?;
                Ok(Step::Exit(Vec::new(), 1))
            }
            ShellEvent::Focus { focused, .. } => {
                self.focused = focused;
                // A composition does not survive losing the keyboard; a
                // stale one would keep every key blocked.
                if !focused {
                    self.view.preedit = Preedit::default();
                }
                if focused && !self.focus_logged {
                    self.focus_logged = true;
                    tracing::debug!(
                        elapsed_us = self.startup_started.elapsed().as_micros(),
                        "startup first focus received"
                    );
                }
                // Restart the blink from the visible phase; unfocused, the
                // caret stays visible and still.
                self.view.blink_on = true;
                self.blink_at = Instant::now() + BLINK;
                Ok(Step::Continue(true))
            }
            ShellEvent::FocusRefused { .. } => {
                tracing::warn!("the window system refused pikr the keyboard focus");
                Ok(Step::Continue(false))
            }
            ShellEvent::Key {
                keysym,
                state: ButtonState::Pressed,
                modifiers,
                ..
            } => {
                self.mods = modifiers_from_crcbl(modifiers);
                let key = key_from_press(keysym, self.mods);
                tracing::trace!(?keysym, ?modifiers, ?key, "key pressed");
                Ok(key.map_or(Step::Continue(false), |key| self.key(&key, content)))
            }
            ShellEvent::TextCommit { text, .. } => {
                let key = key_from_commit(&text, self.mods);
                Ok(key.map_or(Step::Continue(false), |key| self.key(&key, content)))
            }
            ShellEvent::TextPreedit { text, cursor, .. } => {
                // A composition is no chord: forget the modifiers of the
                // last key press, or its commit would be read as one and
                // dropped. Where the input method consumes the composing
                // keys (Wayland), no key press would refresh them.
                if !text.is_empty() {
                    self.mods = Default::default();
                }
                self.view.preedit = Preedit { text, cursor };
                Ok(Step::Continue(true))
            }
            ShellEvent::PointerMotion { abs: Some(at), .. } => {
                self.pointer = Vec2::new(at.x as f32, at.y as f32) / self.scale;
                Ok(Step::Continue(true))
            }
            ShellEvent::Button {
                button: PointerButton::Left,
                state,
                position,
                ..
            } => {
                if let Some(at) = position {
                    self.pointer = Vec2::new(at.x as f32, at.y as f32) / self.scale;
                }
                let down = state == ButtonState::Pressed;
                // Each edge is its own frame, so a press and its release in
                // one pump still make a click.
                let pointer = PointerInput {
                    pos: self.pointer,
                    down,
                    released: self.pointer_down && !down,
                    secondary_pressed: false,
                };
                let clicked = self.frame(content, pointer)?;
                self.pointer_down = down;
                match (clicked, content) {
                    (Some(row), Content::Picker { session, .. }) => {
                        let outcome = session.click_row(row);
                        Ok(self.step(outcome))
                    }
                    _ => Ok(Step::Continue(true)),
                }
            }
            ShellEvent::ClipboardData {
                request,
                content: clipboard,
                ..
            } if Some(request) == self.paste_request => {
                self.paste_request = None;
                match (clipboard, content) {
                    (ClipboardContent::Bytes(bytes), Content::Picker { session, .. }) => {
                        session.paste(&String::from_utf8_lossy(&bytes));
                        Ok(Step::Continue(true))
                    }
                    (other, _) => {
                        tracing::debug!(answer = ?other, "nothing to paste");
                        Ok(Step::Continue(false))
                    }
                }
            }
            ShellEvent::Wheel { delta, .. } => {
                self.wheel += wheel_scroll(delta, self.scale);
                Ok(Step::Continue(true))
            }
            _ => Ok(Step::Continue(false)),
        }
    }

    fn key(&mut self, key: &crate::picker::input::Key, content: &mut Content) -> Step {
        match content {
            Content::Picker { session, .. } => {
                let outcome = session.handle_key(key, self.mods);
                // A keystroke shows the caret, so it never hides mid-type.
                self.view.blink_on = true;
                self.blink_at = Instant::now() + BLINK;
                self.step(outcome)
            }
            Content::Message(_) => {
                use crate::picker::input::{Key, NamedKey};
                if *key == Key::Named(NamedKey::Escape) {
                    Step::Exit(Vec::new(), 0)
                } else {
                    Step::Continue(false)
                }
            }
        }
    }

    /// Builds, lays out and records one frame of UI for `pointer`. Returns
    /// the result row a click landed on.
    ///
    /// The tree is built twice. crcbl's list builds the rows its previous
    /// layout showed, so a list whose rows or scroll offset just changed
    /// would draw last frame's window of rows; the first pass, with the
    /// buttons as they were before this event, lays out the new window and
    /// the second draws it.
    fn frame(&mut self, content: &Content, pointer: PointerInput) -> Result<Option<usize>> {
        let wheel = std::mem::take(&mut self.wheel);
        let clicked = self.build(content, pointer, wheel);
        // Layout showed list rows the build left out (the first frame, or a
        // list that grew): build again, with this frame's click and wheel
        // already taken.
        if self.ui.needs_rebuild() {
            let at_rest = PointerInput {
                down: self.pointer_down,
                ..PointerInput::hovering(pointer.pos)
            };
            self.build(content, at_rest, 0.0);
        }
        self.list.clear();
        self.list.set_scale(self.scale);
        self.ui.emit(&mut self.list);

        let cursor = self.ui.cursor();
        if cursor != self.cursor {
            self.cursor = cursor;
            self.shell.set_cursor(self.window, cursor)?;
        }

        // The input method opens its composition and candidate windows
        // beside the caret, so it hears of every move.
        let area = self
            .view
            .caret_rect(&self.ui)
            .map(|(min, max)| device_rect(min, max, self.scale));
        if area != self.input_area {
            self.input_area = area;
            self.shell.set_text_input_area(self.window, area)?;
        }
        Ok(clicked)
    }

    /// One build and layout of the tree.
    fn build(&mut self, content: &Content, pointer: PointerInput, wheel: f32) -> Option<usize> {
        self.ui.begin_frame(pointer);
        if wheel != 0.0 {
            self.ui.scroll_wheel(Vec2::new(0.0, wheel));
        }
        self.icons.begin_frame();
        let clicked = match content {
            Content::Picker { session, .. } => self.view.build(
                &mut self.ui,
                session,
                &mut self.icons,
                self.gpu.images_mut(),
            ),
            Content::Message(text) => {
                crate::ui::view::build_message(&mut self.ui, text);
                None
            }
        };
        let logical = Vec2::new(self.extent.0 as f32, self.extent.1 as f32) / self.scale;
        self.ui
            .layout(Vec2::ZERO, AvailableSpace::definite(logical), &self.atlas);
        clicked
    }

    fn log_first_paint(&mut self) {
        if !self.painted {
            self.painted = true;
            tracing::debug!(
                elapsed_us = self.startup_started.elapsed().as_micros(),
                "startup first paint pass reached"
            );
        }
    }
}

/// What the loop does after an event.
enum Step {
    /// Keep going; `true` when something on screen changed.
    Continue(bool),
    /// Close the window, run these payloads, and exit with this code.
    Exit(Vec<modes::Payload>, i32),
}

/// The system fonts the picker draws in, each found, read and parsed.
struct Fonts {
    /// The face the theme names, or the system's sans-serif when the theme's
    /// family is not installed.
    theme: Result<Font, String>,
    /// The system's fallbacks for the theme's family, in order, for the chars
    /// its face has no glyph for: emoji, CJK, symbols.
    fallbacks: Vec<Font>,
}

fn load_fonts(family: &str) -> Fonts {
    let started = Instant::now();
    let system = SystemFonts::new();
    let query = FontQuery::new(family);
    let theme = system
        .find(&query)
        .or_else(|| {
            tracing::warn!(
                family,
                "theme font not installed; using the system sans-serif"
            );
            system.find(&FontQuery::new("sans-serif"))
        })
        .ok_or_else(|| "no sans-serif font installed either".to_string())
        .and_then(|face| Font::from_system(&face).map_err(|e| e.to_string()));
    let fallbacks = system
        .fallbacks(&query)
        .iter()
        .filter_map(|face| {
            Font::from_system(face)
                .inspect_err(
                    |e| tracing::debug!(path = %face.path.display(), "fallback font skipped: {e}"),
                )
                .ok()
        })
        .collect::<Vec<_>>();
    tracing::debug!(
        elapsed_us = started.elapsed().as_micros(),
        fallbacks = fallbacks.len(),
        "startup fonts loaded"
    );
    Fonts { theme, fallbacks }
}

/// What the input method should compose for: the query while it takes text
/// (Insert mode, or the `:` prompt), kept from learning under `-P`. In Normal
/// and Visual mode keys are commands, which an input method in its native
/// mode would otherwise turn into a composition; their characters still
/// arrive as commits without it.
fn text_purpose(content: &Content) -> Option<TextPurpose> {
    let Content::Picker { session, .. } = content else {
        return None;
    };
    let picker = &session.app.picker;
    let typing = picker.vim_mode == VimMode::Insert || picker.ex_buf.is_some();
    match (typing, session.app.password) {
        (false, _) => None,
        (true, true) => Some(TextPurpose::Password),
        (true, false) => Some(TextPurpose::Normal),
    }
}

/// How far a wheel event scrolls the list, in logical pixels (positive
/// moves down). A touchpad reports device pixels.
fn wheel_scroll(delta: ScrollDelta, scale: f32) -> f32 {
    match delta {
        ScrollDelta::Lines { y, .. } => -y * WHEEL_LINE_PIXELS,
        ScrollDelta::Pixels { y, .. } => -(y as f32) / scale,
    }
}

/// A rectangle from logical pixels to the window's device pixels, grown to
/// whole pixels so it still covers what it framed.
fn device_rect(min: Vec2, max: Vec2, scale: f32) -> PhysicalRect {
    let (min, max) = ((min * scale).floor(), (max * scale).ceil());
    PhysicalRect::new(
        min.x as i32,
        min.y as i32,
        (max.x - min.x).max(0.0) as u32,
        (max.y - min.y).max(0.0) as u32,
    )
}

#[cfg(test)]
mod headless;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_input_method_composes_only_while_the_query_takes_text() {
        use crate::cli::Mode as CliMode;
        use crate::picker::app_state::tests::app_state;
        use crate::picker::input::{Key, Modifiers, NamedKey};

        let picker = |password| Content::Picker {
            session: Box::new(Session::new(
                app_state(CliMode::Dmenu, &["alpha"], password),
                None,
            )),
            pending: None,
        };
        let press = |content: &mut Content, key: Key| {
            if let Content::Picker { session, .. } = content {
                session.handle_key(&key, Modifiers::default());
            }
        };

        let mut content = picker(false);
        assert_eq!(text_purpose(&content), Some(TextPurpose::Normal));
        press(&mut content, Key::Named(NamedKey::Escape));
        assert_eq!(text_purpose(&content), None, "Normal mode");
        press(&mut content, Key::Character(":".into()));
        assert_eq!(
            text_purpose(&content),
            Some(TextPurpose::Normal),
            ": prompt"
        );

        assert_eq!(text_purpose(&picker(true)), Some(TextPurpose::Password));
        assert_eq!(text_purpose(&Content::Message("hi".into())), None);
    }

    #[test]
    fn touchpad_scroll_is_converted_to_logical_pixels() {
        let touchpad = ScrollDelta::Pixels { x: 0.0, y: -30.0 };
        assert_eq!(wheel_scroll(touchpad, 2.0), 15.0);
        assert_eq!(wheel_scroll(touchpad, 1.0), 30.0);
        // Detents are already in logical units and ignore the scale.
        let wheel = ScrollDelta::Lines { x: 0.0, y: -1.0 };
        assert_eq!(wheel_scroll(wheel, 2.0), WHEEL_LINE_PIXELS);
    }

    #[test]
    fn device_rect_scales_and_covers_partial_pixels() {
        let rect = device_rect(Vec2::new(10.5, 4.0), Vec2::new(12.5, 18.0), 1.5);
        // 15.75..18.75 wide and 6..27 tall, grown to whole pixels.
        assert_eq!(rect, PhysicalRect::new(15, 6, 4, 21));
        assert_eq!(
            device_rect(Vec2::new(3.0, 5.0), Vec2::new(5.0, 19.0), 1.0),
            PhysicalRect::new(3, 5, 2, 14)
        );
    }
}
