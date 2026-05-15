//! App wiring — turns CLI + config into a running picker.

use crate::cli::{Cli, Mode};
use crate::config::Config;
use anyhow::Result;

pub fn run(cli: Cli) -> Result<()> {
    let cfg = Config::load(cli.config.as_deref())?;
    tracing::debug!(?cfg, "config loaded");

    let mode = if cli.dmenu { Mode::Dmenu } else { cli.show };
    tracing::info!(?mode, "pikr starting");

    // TODO: build Mode impl, picker state, launch floem UI.
    eprintln!("pikr: v0.1 scaffold — UI not wired yet (mode = {mode:?})");
    Ok(())
}
