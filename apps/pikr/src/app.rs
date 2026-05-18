//! App wiring — turns CLI + config into a running floem picker.

use std::sync::{Arc, Mutex};

use floem::reactive::{SignalGet, SignalUpdate};

use crate::cli::{Cli, Mode};
use crate::config::Config;
use crate::modes;
use crate::picker::state::PickerState;
use crate::ui::view::{AppState, message_view, picker_view};
use anyhow::Result;

pub fn run(cli: Cli) -> Result<()> {
    let cfg = Config::load(cli.config.as_deref())?;
    tracing::debug!(?cfg, "config loaded");

    // ── Message modal path ─────────────────────────────────────────────────
    // When --message is given we skip all picker logic and render a simple
    // non-interactive overlay. Esc dismisses via std::process::exit(0).
    if let Some(msg) = cli.message.clone() {
        let theme = cfg.theme.clone();
        let windowed = cli.no_layer_shell;
        let view = move || message_view(msg.clone(), theme.clone(), windowed);

        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        {
            use floem::window::WindowConfig;

            if std::env::var_os("WAYLAND_DISPLAY").is_none() {
                eprintln!("pikr: WAYLAND_DISPLAY is not set — pikr requires a Wayland compositor");
                std::process::exit(1);
            }

            let width = cli.width.unwrap_or(720);
            let size = floem::kurbo::Size::new(width as f64, 120.0);
            let use_layer_shell = !cli.no_layer_shell;
            if use_layer_shell {
                use floem::window::{Anchor, LayerShellConfig};
                let layer_cfg = LayerShellConfig {
                    namespace: "pikr".into(),
                    anchor: Anchor::empty(),
                    exclusive_zone: 0,
                    ..Default::default()
                };
                let window_config = WindowConfig::default()
                    .size(size)
                    .with_transparent(true)
                    .with_layer_shell_config(layer_cfg);
                floem::Application::new_wayland()
                    .window(move |_| view(), Some(window_config))
                    .run();
            } else {
                let window_config = WindowConfig::default().size(size).with_transparent(false);
                floem::Application::new_wayland()
                    .window(move |_| view(), Some(window_config))
                    .run();
            }
        }
        #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
        floem::launch(view);
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

    let entries: Vec<Arc<modes::Entry>> = mode.collect()?.into_iter().map(Arc::new).collect();
    tracing::debug!(count = entries.len(), "entries collected");

    let picker = PickerState::new();
    picker.vim_mode.set(cli.mode);

    // --filter / --prefill / --query / --input-text: pre-fill the query and
    // position the cursor at the end before the first rerank.
    if let Some(ref text) = cli.filter {
        picker.query.set(text.clone());
        picker.query_cursor.set(text.chars().count());
    }

    let mut matcher = crate::picker::matcher::Matcher::new();
    // (label, description) — matcher ranks each field separately so a label
    // hit can outweigh a description hit even at equal nucleo score.
    let pairs: Vec<(&str, Option<&str>)> = entries
        .iter()
        .map(|e| (e.label.as_str(), e.description.as_deref()))
        .collect();
    // Use the prefill query (if any) for the initial rank so the list reflects
    // the filter text on first paint instead of being an empty-query rank.
    let initial_query = picker.query.get_untracked();
    let mut matches = matcher.rank(&pairs, &initial_query);
    matches.truncate(cfg.max_results);

    let usage = crate::picker::frecency::Usage::load();
    let history = crate::picker::history::History::load();
    let icons = Arc::new(Mutex::new(crate::picker::icons::IconCache::new()));
    let app_state = Arc::new(Mutex::new(AppState {
        picker,
        entries,
        matches,
        g_pending: false,
        cli_mode: chosen_mode,
        prompt: cli.prompt.unwrap_or_default(),
        max_results: cfg.max_results,
        theme: cfg.theme,
        matcher,
        windowed: cli.no_layer_shell,
        password: cli.password,
        usage,
        history,
        icons,
    }));
    // Apply frecency bonus to the initial empty-query rank so a fresh
    // launch already shows favourites at the top instead of XDG order.
    if let Ok(mut s) = app_state.lock() {
        s.rerank();
    }

    let view = move || picker_view(Arc::clone(&app_state));

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        use floem::window::WindowConfig;

        // Height = input row + viewport (ROW_HEIGHT * VISIBLE_ROWS) +
        // status + ex_bar gutter + outer border slack. Keep in step with
        // ui::view::{ROW_HEIGHT, VISIBLE_ROWS, INPUT_ROW_HEIGHT,
        // STATUS_HEIGHT} so the window doesn't trim a half-row.
        use crate::ui::view::{
            INPUT_ROW_HEIGHT, ROW_PITCH, STATUS_BAR_TOTAL, STATUS_HEIGHT, VISIBLE_ROWS,
        };
        let visible_rows = cli.lines.unwrap_or(VISIBLE_ROWS) as f64;
        let viewport_h = ROW_PITCH * visible_rows;
        // chrome = input row + input margin_bottom + ex gutter + status bar
        // (height + vert padding + top margin) + panel padding (both sides).
        let chrome_h = INPUT_ROW_HEIGHT + 8.0 + STATUS_HEIGHT + STATUS_BAR_TOTAL + 20.0;
        let win_width = cli.width.unwrap_or(720) as f64;
        let size = floem::kurbo::Size::new(win_width, viewport_h + chrome_h);

        if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            eprintln!("pikr: WAYLAND_DISPLAY is not set — pikr requires a Wayland compositor");
            std::process::exit(1);
        }

        // The window framebuffer is ALWAYS transparent so the compositor's
        // window-corner rounding (Hyprland / KWin / etc.) can clip cleanly
        // without leaking the opaque framebuffer fill into the rounded
        // cutout. The `transparent` config / CLI flag instead controls the
        // alpha of the VIEW's background fill — see view.rs.

        if !cli.no_layer_shell {
            tracing::info!("using wlr-layer-shell path");
            use floem::window::{Anchor, LayerShellConfig};
            let layer_cfg = LayerShellConfig {
                namespace: "pikr".into(),
                // No edge anchors → compositor centers the surface on the output.
                anchor: Anchor::empty(),
                // Centered overlays should not reserve a strut.
                exclusive_zone: 0,
                ..Default::default()
            };
            let window_config = WindowConfig::default()
                .size(size)
                .with_transparent(true)
                .with_layer_shell_config(layer_cfg);
            floem::Application::new_wayland()
                .window(move |_| view(), Some(window_config))
                .run();
        } else {
            tracing::info!("using plain Wayland window path (--no-layer-shell)");
            // Opaque framebuffer in --no-layer-shell mode: vger SDF anti-aliasing
            // along rounded edges mixes with the framebuffer alpha, not the
            // parent's bg. On a transparent framebuffer that produces dotted
            // / fuzzy corners on the status badges. Layer-shell path keeps
            // transparency for compositor-side corner clipping.
            let window_config = WindowConfig::default().size(size).with_transparent(false);
            floem::Application::new_wayland()
                .window(move |_| view(), Some(window_config))
                .run();
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    floem::launch(view);
    Ok(())
}
