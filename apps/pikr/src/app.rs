//! App wiring — turns CLI + config into a running floem picker.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use floem::reactive::SignalUpdate;

use crate::cli::{Cli, Mode};
use crate::config::Config;
use crate::modes;
use crate::picker::state::PickerState;
use crate::ui::view::{AppState, message_view, picker_view};
use anyhow::Result;

pub fn run(cli: Cli, startup_started: Instant) -> Result<()> {
    let phase_started = Instant::now();
    let cfg = Config::load(cli.config.as_deref())?;
    tracing::debug!(
        phase_us = phase_started.elapsed().as_micros(),
        elapsed_us = startup_started.elapsed().as_micros(),
        "startup config loaded"
    );
    tracing::debug!(?cfg, "config loaded");

    // Wayland-capable unix: require a compositor at startup. Other OSes
    // (macOS / Windows once they're back in CI) skip the guard — they
    // open a regular OS-managed top-level window instead of layer-shell.
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            eprintln!("pikr: WAYLAND_DISPLAY is not set — pikr requires a Wayland compositor");
            std::process::exit(1);
        }
    }

    use floem::window::WindowConfig;

    // ── Message modal path ─────────────────────────────────────────────────
    // When --message is given we skip all picker logic and render a simple
    // non-interactive overlay. Esc dismisses via std::process::exit(0).
    if let Some(msg) = cli.message.clone() {
        let sheet = std::sync::Arc::new(crate::ui::css::build_stylesheet(&cfg.theme));
        let view = move || message_view(msg.clone(), std::sync::Arc::clone(&sheet));

        let width = cli.width.unwrap_or(720);
        let size = floem::kurbo::Size::new(width as f64, 120.0);

        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        {
            use floem::window::{Anchor, KeyboardInteractivity, LayerShellConfig};
            let layer_cfg = LayerShellConfig {
                namespace: "pikr".into(),
                anchor: Anchor::empty(),
                exclusive_zone: 0,
                keyboard_interactivity: KeyboardInteractivity::Exclusive,
                ..Default::default()
            };
            let window_config = WindowConfig::default()
                .size(size)
                .with_transparent(true)
                .with_layer_shell(layer_cfg);
            floem::Application::new_wayland()
                .window(move |_| view(), Some(window_config))
                .run();
        }

        #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
        {
            let window_config = WindowConfig::default().size(size).with_transparent(true);
            floem::Application::new()
                .window(move |_| view(), Some(window_config))
                .run();
        }
        return Ok(());
    }

    // ── Normal picker path ─────────────────────────────────────────────────

    let chosen_mode = if cli.dmenu { Mode::Dmenu } else { cli.show };
    tracing::info!(?chosen_mode, "pikr starting");

    let mut mode: Box<dyn modes::Mode> = match chosen_mode {
        Mode::Calc => Box::new(modes::calc::Calc),
        Mode::Clipboard => Box::new(modes::clipboard::Clipboard),
        Mode::Dmenu => Box::new(modes::dmenu::Dmenu),
        Mode::Drun => Box::new(modes::drun::Drun),
        Mode::Emoji => Box::new(modes::emoji::Emoji),
        Mode::Run => Box::new(modes::run::Run),
        Mode::Ssh => Box::new(modes::ssh::Ssh),
    };

    let phase_started = Instant::now();
    let entries: Vec<Arc<modes::Entry>> = mode.collect()?.into_iter().map(Arc::new).collect();
    tracing::debug!(
        phase_us = phase_started.elapsed().as_micros(),
        elapsed_us = startup_started.elapsed().as_micros(),
        count = entries.len(),
        "startup entries collected"
    );
    tracing::debug!(count = entries.len(), "entries collected");

    let picker = PickerState::new();
    picker.vim_mode.set(cli.mode);

    // --filter / --prefill / --query / --input-text: pre-fill the query and
    // position the cursor at the end before the first rerank.
    if let Some(ref text) = cli.filter {
        picker.query.set(text.clone());
        picker.query_cursor.set(text.chars().count());
    }

    let matcher = crate::picker::matcher::Matcher::with_case_sensitive(cfg.case_sensitive);
    let matches = Vec::new();

    let usage = crate::picker::frecency::Usage::load();
    let usage_keys = crate::picker::frecency::entry_keys(&entries);
    let history = crate::picker::history::History::load();
    let icons = Arc::new(Mutex::new(crate::picker::icons::IconCache::new()));
    let stylesheet = Arc::new(crate::ui::css::build_stylesheet(&cfg.theme));
    let app_state = Arc::new(Mutex::new(AppState {
        picker,
        entries,
        usage_keys,
        matches,
        g_pending: false,
        cli_mode: chosen_mode,
        prompt: cli.prompt.unwrap_or_default(),
        max_results: cfg.max_results,
        theme: cfg.theme,
        matcher,
        password: cli.password,
        usage,
        history,
        icons,
        stylesheet,
    }));

    let view = move || picker_view(Arc::clone(&app_state), startup_started);

    // Window height = scrollable viewport + chrome. chrome must precisely
    // sum every non-scrollable element in the v_stack so flex_grow on the
    // scroll doesn't squeeze (or expand) the visible row count.
    use crate::ui::view::{
        EX_BAR_TOTAL, INPUT_MARGIN_BOTTOM, INPUT_ROW_HEIGHT, PANEL_PAD, ROW_PITCH,
        STATUS_BAR_TOTAL, STATUS_HEIGHT, VISIBLE_ROWS,
    };
    let visible_rows = cli.lines.unwrap_or(VISIBLE_ROWS) as f64;
    let viewport_h = ROW_PITCH * visible_rows;
    let chrome_h = PANEL_PAD * 2.0       // panel top + bottom padding
        + INPUT_ROW_HEIGHT
        + INPUT_MARGIN_BOTTOM
        + EX_BAR_TOTAL
        + STATUS_HEIGHT
        + STATUS_BAR_TOTAL;
    let win_width = cli.width.unwrap_or(720) as f64;
    let size = floem::kurbo::Size::new(win_width, viewport_h + chrome_h);

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        use floem::window::{Anchor, KeyboardInteractivity, LayerShellConfig};

        // The window framebuffer is transparent so the compositor's window-corner
        // rounding (Hyprland / KWin / etc.) can clip cleanly without leaking the
        // opaque framebuffer fill into the rounded cutout.
        let layer_cfg = LayerShellConfig {
            namespace: "pikr".into(),
            // No edge anchors → compositor centers the surface on the output.
            anchor: Anchor::empty(),
            // Centered overlays should not reserve a strut.
            exclusive_zone: 0,
            // Exclusive keyboard focus: the compositor routes all key events to
            // pikr while open. Other apps can't take input until pikr is
            // dismissed (Esc x2 / Enter). Matches the rofi/wofi modal-launcher
            // convention.
            keyboard_interactivity: KeyboardInteractivity::Exclusive,
            ..Default::default()
        };
        let window_config = WindowConfig::default()
            .size(size)
            .with_transparent(true)
            .with_layer_shell(layer_cfg);
        floem::Application::new_wayland()
            .window(move |_| view(), Some(window_config))
            .run();
    }

    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    {
        // macOS / Windows: pikr opens a regular OS-managed top-level window.
        // No layer-shell, no exclusive keyboard grab — the host compositor's
        // window manager owns z-order and focus. Modal-launcher semantics
        // (Esc dismiss) still apply at the view layer.
        let window_config = WindowConfig::default().size(size).with_transparent(true);
        floem::Application::new()
            .window(move |_| view(), Some(window_config))
            .run();
    }
    Ok(())
}
