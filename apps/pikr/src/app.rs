//! App wiring — turns CLI + config into a running picker window.

use std::sync::Arc;
use std::time::Instant;

use crate::cli::{Cli, Mode};
use crate::config::Config;
use crate::modes;
use crate::picker::app_state::AppState;
use crate::picker::session::Session;
use crate::picker::state::PickerState;
use crate::ui::run::{Content, MESSAGE_HEIGHT, WindowSpec};
use crate::ui::view::{VISIBLE_ROWS, password_window_height, window_height};
use anyhow::Result;

/// The window width when `--width` doesn't set one.
const DEFAULT_WIDTH: u32 = 720;

pub fn run(cli: Cli, startup_started: Instant) -> Result<()> {
    let phase_started = Instant::now();
    let cfg = Config::load(cli.config.as_deref())?;
    tracing::debug!(
        phase_us = phase_started.elapsed().as_micros(),
        elapsed_us = startup_started.elapsed().as_micros(),
        "startup config loaded"
    );
    tracing::debug!(?cfg, "config loaded");

    let width = f64::from(cli.width.unwrap_or(DEFAULT_WIDTH));
    let spec = |height| WindowSpec {
        width,
        height,
        stylesheet: crate::ui::theme_css::stylesheet_source(&cfg.theme),
        font_family: cfg.theme.font.clone(),
        font_size: cfg.theme.font_size,
        icon_themes: crate::picker::icon_theme::chosen(cfg.theme.icon_theme.as_deref()),
    };

    // ── Message modal path ─────────────────────────────────────────────────
    // When --message is given we skip all picker logic and render a simple
    // non-interactive overlay. Esc dismisses it.
    if let Some(msg) = cli.message.clone() {
        return crate::ui::run::run(spec(MESSAGE_HEIGHT), Content::Message(msg), startup_started);
    }

    // ── Normal picker path ─────────────────────────────────────────────────

    let chosen_mode = cli.chosen_mode();
    tracing::info!(?chosen_mode, "pikr starting");

    let mut mode: Box<dyn modes::Mode> = match chosen_mode {
        Mode::Calc => Box::new(modes::calc::Calc),
        Mode::Clipboard => Box::new(modes::clipboard::Clipboard),
        Mode::Dmenu => Box::new(modes::dmenu::Dmenu),
        Mode::Drun => Box::new(modes::drun::Drun::default()),
        Mode::Emoji => Box::new(modes::emoji::Emoji),
        Mode::Run => Box::new(modes::run::Run),
        Mode::Ssh => Box::new(modes::ssh::Ssh),
    };

    let phase_started = Instant::now();
    // --loading: open now, read stdin on a thread; rows arrive later.
    let pending = match (&cli.loading, chosen_mode) {
        // `Cli::validate` refuses --loading with -P.
        (Some(_), Mode::Dmenu) => Some(modes::dmenu::read_stdin_in_background()?),
        _ => None,
    };
    // -P and --lines 0 are bare prompts: no rows, and stdin is left unread.
    let entries: Vec<Arc<modes::Entry>> = if pending.is_some() || cli.bare_prompt() {
        Vec::new()
    } else {
        mode.collect()?.into_iter().map(Arc::new).collect()
    };
    let redraws = mode.redraws();
    tracing::debug!(
        phase_us = phase_started.elapsed().as_micros(),
        elapsed_us = startup_started.elapsed().as_micros(),
        count = entries.len(),
        "startup entries collected"
    );

    let mut picker = PickerState::new();
    picker.vim_mode = cli.mode;

    // --filter / --prefill / --query / --input-text: pre-fill the query and
    // position the cursor at the end before the first rerank.
    if let Some(ref text) = cli.filter {
        picker.query = text.clone();
        picker.query_cursor = text.chars().count();
    }

    let history = crate::picker::history::History::load();
    let calc_results = crate::modes::calc::precompute(history.list(crate::cli::Mode::Calc));
    let app = AppState {
        picker,
        usage_keys: crate::picker::frecency::entry_keys(&entries),
        match_keys: crate::picker::matcher::match_keys(&entries),
        entries,
        matches: Vec::new(),
        g_pending: false,
        cli_mode: chosen_mode,
        prompt: cli.prompt.unwrap_or_default(),
        max_results: cfg.max_results,
        matcher: crate::picker::matcher::Matcher::with_case_sensitive(cfg.case_sensitive),
        password: cli.password,
        rows: cli.lines.unwrap_or(VISIBLE_ROWS),
        usage: crate::picker::frecency::Usage::load(),
        history,
        calc_results,
        kb_custom: cli.kb_custom,
    };
    let height = if app.password {
        password_window_height()
    } else {
        window_height(app.rows)
    };
    let loading = pending.as_ref().and(cli.loading);
    let session = Session::new(app, loading);
    tracing::debug!(
        elapsed_us = startup_started.elapsed().as_micros(),
        "startup state ranked"
    );

    crate::ui::run::run(
        spec(height),
        Content::Picker {
            session: Box::new(session),
            pending,
            redraws,
        },
        startup_started,
    )
}
