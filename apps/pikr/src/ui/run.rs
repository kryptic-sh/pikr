//! The window and its event loop: a crcbl panel, its GPU swapchain, and the
//! picker (or `--message` modal) drawn into it whenever something changed.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use crcbl_shell::{
    ButtonState, CloseReply, CursorIcon, LogicalSize, PanelAnchor, PanelDesc, PanelKeyboard,
    PanelLayer, PointerButton, ScrollDelta, Shell, ShellCaps, ShellEvent, TextPurpose, WindowDesc,
    WindowId, WindowRole,
};
use crcbl_ui::draw_list::DrawList;
use crcbl_ui::font::Font;
use crcbl_ui::text::FontAtlas;
use crcbl_ui::tree::{AvailableSpace, Ui};
use crcbl_ui::widget::PointerInput;
use glam::Vec2;

use crate::modes::{self, dmenu::LoadedEntries};
use crate::picker::session::{Outcome, Session};
use crate::ui::gpu::Gpu;
use crate::ui::keyboard::{key_from_commit, key_from_press, modifiers_from_crcbl};
use crate::ui::view::{Icons, View};

/// The caret's blink half-period, as the floem picker and crcbl's own text
/// field blink.
const BLINK: Duration = Duration::from_millis(530);
/// How long the window system gets to size a new window before pikr gives up.
const CONFIGURE_TIMEOUT: Duration = Duration::from_secs(5);
/// Pixels one wheel detent scrolls.
const WHEEL_LINE_PIXELS: f32 = 40.0;
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
        std::thread::spawn(move || Fonts {
            theme: load_font(&family),
            emoji: load_font(crate::ui::fonts::EMOJI_FAMILY),
        })
    };
    let mut shell = crcbl_shell::open().context("opening the window system")?;
    let (window, transparent_window) = create_window(shell.as_mut(), &spec)?;
    let extent = wait_for_size(shell.as_mut(), window)?;
    let mut gpu = Gpu::open(&shell.surface_target(window)?, extent, transparent_window)?;
    let fonts = fonts.join().unwrap_or_else(|_| Fonts {
        theme: Err("the font loader panicked".into()),
        emoji: Err("the font loader panicked".into()),
    });
    let outcome = Window::new(
        shell.as_mut(),
        window,
        &spec,
        fonts,
        &mut gpu,
        startup_started,
    )
    .and_then(|mut w| w.run(content));
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

/// Pumps until the window system gives the new window a size.
fn wait_for_size(shell: &mut dyn Shell, window: WindowId) -> Result<(u32, u32)> {
    let started = Instant::now();
    loop {
        shell.pump(&mut |_| {});
        if let Some(size) = shell.window_state(window)?.size() {
            return Ok((size.width, size.height));
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
        // without it, text is drawn in crcbl's built-in sans-serif face.
        let family = &spec.font_family;
        if let Err(e) = fonts
            .theme
            .and_then(|font| ui.register_font(family, font).map_err(|e| e.to_string()))
        {
            tracing::warn!(family, "theme font not used: {e}");
        }
        // Emoji, and any other char the theme font lacks, are drawn from the
        // system's emoji font; without one they draw as missing-glyph boxes.
        match fonts.emoji {
            Ok(emoji) => ui.set_fallback_fonts([emoji]),
            Err(e) => tracing::warn!("emoji font not used: {e}"),
        }
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
            focused: false,
            blink_at: Instant::now() + BLINK,
            mods: Default::default(),
            startup_started,
            painted: false,
            focus_logged: false,
        })
    }

    /// Runs until the content is done; returns what to run and the exit code.
    fn run(&mut self, mut content: Content) -> Result<(Vec<modes::Payload>, i32)> {
        // The query is a text field: the input method composes into it, and
        // a password query is kept from learning and suggestions.
        let purpose = match &content {
            Content::Picker { session, .. } if session.app.password => Some(TextPurpose::Password),
            Content::Picker { .. } => Some(TextPurpose::Normal),
            Content::Message(_) => None,
        };
        self.shell.set_text_input(self.window, purpose)?;
        let arrived = match &mut content {
            Content::Picker { pending, .. } => pending.take().map(|rx| self.forward(rx)),
            Content::Message(_) => None,
        };

        let mut dirty = true;
        loop {
            if let (Some(rx), Content::Picker { session, .. }) = (&arrived, &mut content)
                && let Ok(loaded) = rx.try_recv()
            {
                match loaded {
                    Ok(entries) => session.entries_arrived(&entries),
                    // Same outcome as a read error without --loading.
                    Err(e) => return Err(anyhow!("reading stdin: {e}")),
                }
                dirty = true;
            }

            let mut events = Vec::new();
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
                }
                continue;
            }
            let timeout = self
                .focused
                .then(|| self.blink_at.saturating_duration_since(Instant::now()));
            self.shell.wait_events(timeout);
        }
    }

    /// Relays `--loading` rows from the reader thread, waking the loop when
    /// they arrive.
    fn forward(&mut self, rx: mpsc::Receiver<LoadedEntries>) -> mpsc::Receiver<LoadedEntries> {
        let (tx, forwarded) = mpsc::channel();
        let waker = self.shell.waker();
        std::thread::spawn(move || {
            if let Ok(loaded) = rx.recv() {
                let _ = tx.send(loaded);
                if let Some(waker) = waker {
                    waker.wake();
                }
            }
        });
        forwarded
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
                Ok(Step::Continue(true))
            }
            ShellEvent::CloseRequested { window } => {
                self.shell.reply_close_request(window, CloseReply::Keep)?;
                Ok(Step::Exit(Vec::new(), 1))
            }
            ShellEvent::Focus { focused, .. } => {
                self.focused = focused;
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
                key_code,
                keysym,
                state: ButtonState::Pressed,
                modifiers,
                ..
            } => {
                self.mods = modifiers_from_crcbl(modifiers);
                let key = key_from_press(key_code, keysym, self.mods);
                Ok(key.map_or(Step::Continue(false), |key| self.key(&key, content)))
            }
            ShellEvent::TextCommit { text, .. } => {
                let key = key_from_commit(&text, self.mods);
                Ok(key.map_or(Step::Continue(false), |key| self.key(&key, content)))
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
                        Ok(Step::from_outcome(session.click_row(row)))
                    }
                    _ => Ok(Step::Continue(true)),
                }
            }
            ShellEvent::Wheel { delta, .. } => {
                let y = match delta {
                    ScrollDelta::Lines { y, .. } => -y * WHEEL_LINE_PIXELS,
                    ScrollDelta::Pixels { y, .. } => -(y as f32),
                };
                self.wheel += y;
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
                Step::from_outcome(outcome)
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
        let at_rest = PointerInput {
            down: self.pointer_down,
            ..PointerInput::hovering(pointer.pos)
        };
        let wheel = std::mem::take(&mut self.wheel);
        self.build(content, at_rest, wheel);
        let clicked = self.build(content, pointer, 0.0);
        self.list.clear();
        self.list.set_scale(self.scale);
        self.ui.emit(&mut self.list);

        let cursor = self.ui.cursor();
        if cursor != self.cursor {
            self.cursor = cursor;
            self.shell.set_cursor(self.window, cursor)?;
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

impl Step {
    fn from_outcome(outcome: Outcome) -> Self {
        match outcome {
            Outcome::Continue => Self::Continue(true),
            Outcome::Exit { payloads, code } => Self::Exit(payloads, code),
        }
    }
}

/// The system fonts the picker draws in, each found, read and parsed.
struct Fonts {
    /// The face the theme names.
    theme: Result<Font, String>,
    /// The fallback for chars the theme's face has no glyph for.
    emoji: Result<Font, String>,
}

/// The installed font `family` names, found, read and parsed.
fn load_font(family: &str) -> Result<Font, String> {
    let path = crate::ui::fonts::find(family).ok_or("no such font installed")?;
    let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    Font::parse(bytes).map_err(|e| format!("{}: {e}", path.display()))
}
