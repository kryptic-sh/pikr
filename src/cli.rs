//! CLI surface.

use clap::{Parser, ValueEnum};
use std::path::PathBuf;

#[derive(Parser, Debug, Clone)]
#[command(
    name = "pikr",
    version,
    about = "Vim-modal picker / launcher",
    long_about = None,
)]
pub struct Cli {
    /// Mode to launch in. With --dmenu this is ignored.
    #[arg(short = 's', long = "show", value_enum, default_value = "drun")]
    pub show: Mode,

    /// dmenu mode: read entries from stdin, print selection to stdout.
    #[arg(short = 'd', long = "dmenu")]
    pub dmenu: bool,

    /// Prompt text shown in the input row.
    #[arg(short = 'p', long = "prompt")]
    pub prompt: Option<String>,

    /// Override config file path.
    #[arg(long = "config")]
    pub config: Option<PathBuf>,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Drun,
    Run,
    Dmenu,
}
