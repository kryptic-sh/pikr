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
    // Default filter quiets two chatty deps that emit benign WARNs on every
    // startup:
    //   - `usvg` complains about `clip-path: none` / `marker-*: none` and
    //     missing exotic font-families on icons that still render fine.
    //   - `wgpu_hal` notes missing Vulkan validation layers and an init-time
    //     GLES context re-init under Wayland; neither affects runtime.
    // Users can opt back in with `RUST_LOG=usvg=warn` etc.
    // Logs go to stderr so stdout stays exclusively the picker payload
    // (the dmenu-style emit). Without this, every wgpu / floem trace line
    // ends up in the consumer's stdin pipe alongside the actual selection.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("warn,usvg=error,wgpu_hal=error")
            }),
        )
        .init();

    let cli = cli::Cli::parse();
    app::run(cli)
}
