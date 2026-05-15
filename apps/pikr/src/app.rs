//! App wiring — turns CLI + config into a running picker.
//!
//! Today this runs headless: it builds the selected `Mode`, collects entries,
//! ranks them against an empty query, and prints a preview to stderr. Stdout
//! is reserved for the future dmenu accept path. The interactive floem UI
//! lands in Epic 3.

use crate::cli::{Cli, Mode};
use crate::config::Config;
use crate::modes::{self, Entry};
use crate::picker::matcher::Matcher;
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
    eprintln!(
        "pikr: mode={} · {} entries collected",
        mode.name(),
        entries.len()
    );

    preview(&entries, cfg.max_results.min(10));
    eprintln!("pikr: UI not wired yet (Epic 3). Selection / execute paths idle.");
    Ok(())
}

fn preview(entries: &[Entry], n: usize) {
    if entries.is_empty() {
        return;
    }
    let labels: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
    let mut matcher = Matcher::new();
    let ranked = matcher.rank(&labels, "");
    eprintln!("pikr: top {} preview:", n.min(ranked.len()));
    for m in ranked.iter().take(n) {
        let entry = &entries[m.index];
        match &entry.description {
            Some(d) => eprintln!("  · {} — {}", entry.label, d),
            None => eprintln!("  · {}", entry.label),
        }
    }
}
