//! pikr — vim-modal picker / launcher.

#![forbid(unsafe_code)]
#![allow(dead_code)] // v0.1 scaffold — stubs land before consumers.

mod app;
mod cli;
mod config;
mod modes;
mod picker;
mod ui;

use anyhow::Result;
use clap::Parser;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let cli = cli::Cli::parse();
    app::run(cli)
}
