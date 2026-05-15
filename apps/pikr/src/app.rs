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

    let entries = mode.collect()?;
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

    floem::launch(move || picker_view(Arc::clone(&app_state)));
    Ok(())
}
