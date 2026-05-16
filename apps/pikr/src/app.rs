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
        Mode::Dmenu => Box::new(modes::dmenu::Dmenu),
        Mode::Drun => Box::new(modes::drun::Drun),
        Mode::Run => Box::new(modes::run::Run),
    };

    let entries: Vec<Arc<modes::Entry>> = mode
        .collect()?
        .into_iter()
        .map(Arc::new)
        .collect();
    tracing::debug!(count = entries.len(), "entries collected");

    let picker = PickerState::new();
    let mut matcher = crate::picker::matcher::Matcher::new();
    let labels: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
    let mut matches = matcher.rank(&labels, "");
    matches.truncate(cfg.max_results);

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
    }));

    let view = move || picker_view(Arc::clone(&app_state));

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        use floem::window::{Anchor, LayerShellConfig, WindowConfig};
        let layer_cfg = LayerShellConfig {
            namespace: "pikr".into(),
            // No edge anchors → compositor centers the surface on the output.
            anchor: Anchor::empty(),
            // Centered overlays should not reserve a strut.
            exclusive_zone: 0,
            ..Default::default()
        };
        // Height = input row + viewport (ROW_HEIGHT * VISIBLE_ROWS) +
        // status + ex_bar gutter + outer border slack. Keep in step with
        // ui::view::{ROW_HEIGHT, VISIBLE_ROWS, INPUT_ROW_HEIGHT,
        // STATUS_HEIGHT} so the window doesn't trim a half-row.
        use crate::ui::view::{INPUT_ROW_HEIGHT, ROW_HEIGHT, STATUS_HEIGHT, VISIBLE_ROWS};
        let viewport_h = ROW_HEIGHT * VISIBLE_ROWS as f64;
        // Ex gutter same height as status when shown, 0 otherwise; reserve it.
        let chrome_h = INPUT_ROW_HEIGHT + STATUS_HEIGHT + STATUS_HEIGHT + 6.0;
        let window_config = WindowConfig::default()
            .size(floem::kurbo::Size::new(640.0, viewport_h + chrome_h))
            .with_transparent(true)
            .with_layer_shell_config(layer_cfg);
        floem::Application::new_wayland()
            .window(move |_| view(), Some(window_config))
            .run();
    }
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    floem::launch(view);
    Ok(())
}
