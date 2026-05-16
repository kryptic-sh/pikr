//! App wiring — turns CLI + config into a running floem picker.

use std::sync::{Arc, Mutex};

use crate::cli::{Cli, Mode};
use crate::config::Config;
use crate::modes;
use crate::picker::state::PickerState;
use crate::ui::view::{AppState, picker_view};
use anyhow::Result;

pub fn run(cli: Cli) -> Result<()> {
    let cfg = Config::load(cli.config.as_deref())?;
    tracing::debug!(?cfg, "config loaded");

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
    let mut matcher = crate::picker::matcher::Matcher::new();
    let labels: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
    let mut matches = matcher.rank(&labels, "");
    matches.truncate(cfg.max_results);

    // Resolve --smoked preset BEFORE moving cli.prompt below (methods take &self).
    let opacity = cli.effective_opacity().or(cfg.opacity);
    let blur = cli.effective_blur().or(cfg.blur);
    let grayscale = cli.smoked;
    let app_state = Arc::new(Mutex::new(AppState {
        picker,
        entries,
        matches,
        g_pending: false,
        cli_mode: chosen_mode,
        prompt: cli.prompt.unwrap_or_default(),
        max_results: cfg.max_results,
        theme: cfg.theme,
        opacity,
        blur,
        grayscale,
        matcher,
    }));

    let view = move || picker_view(Arc::clone(&app_state));

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        use floem::window::WindowConfig;

        // Height = input row + viewport (ROW_HEIGHT * VISIBLE_ROWS) +
        // status + ex_bar gutter + outer border slack. Keep in step with
        // ui::view::{ROW_HEIGHT, VISIBLE_ROWS, INPUT_ROW_HEIGHT,
        // STATUS_HEIGHT} so the window doesn't trim a half-row.
        use crate::ui::view::{INPUT_ROW_HEIGHT, ROW_HEIGHT, STATUS_HEIGHT, VISIBLE_ROWS};
        let viewport_h = ROW_HEIGHT * VISIBLE_ROWS as f64;
        // Ex gutter same height as status when shown, 0 otherwise; reserve it.
        let chrome_h = INPUT_ROW_HEIGHT + STATUS_HEIGHT + STATUS_HEIGHT + 6.0;
        let size = floem::kurbo::Size::new(720.0, viewport_h + chrome_h);

        let use_layer_shell = !cli.no_layer_shell && std::env::var_os("WAYLAND_DISPLAY").is_some();

        // The window framebuffer is ALWAYS transparent so the compositor's
        // window-corner rounding (Hyprland / KWin / etc.) can clip cleanly
        // without leaking the opaque framebuffer fill into the rounded
        // cutout. The `transparent` config / CLI flag instead controls the
        // alpha of the VIEW's background fill — see view.rs.

        if use_layer_shell {
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
            // When falling back from wlr-layer-shell, distinguish two cases:
            //   * WAYLAND_DISPLAY unset → assume X11 → tag window as a Dock so
            //     the WM keeps it on top and out of the taskbar.
            //   * WAYLAND_DISPLAY set but --no-layer-shell → Mutter/GNOME
            //     style Wayland session → plain xdg_toplevel, no X11 attrs.
            let likely_x11 = std::env::var_os("WAYLAND_DISPLAY").is_none();
            tracing::info!(likely_x11, "using plain window path");
            let mut window_config = WindowConfig::default().size(size).with_transparent(true);
            if likely_x11 {
                use floem::window::{X11Config, X11WindowType};
                window_config = window_config.with_x11_config(X11Config {
                    window_types: vec![X11WindowType::Dock],
                    override_redirect: false,
                });
            }
            floem::Application::new()
                .window(move |_| view(), Some(window_config))
                .run();
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    floem::launch(view);
    Ok(())
}
