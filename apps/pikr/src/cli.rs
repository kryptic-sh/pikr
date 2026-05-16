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
    /// Available: drun, run, dmenu, ssh, emoji, clipboard, calc.
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

    /// Skip Wayland layer-shell, open as a regular window. Useful on compositors
    /// without wlr-layer-shell (Mutter/GNOME) or for X11.
    #[arg(long = "no-layer-shell")]
    pub no_layer_shell: bool,

    /// Smoked-glass background — paints the panel with reduced alpha so the
    /// compositor blurs / shows what's behind. Overrides `smoked` from
    /// config when set. Compositor must support alpha compositing.
    #[arg(long = "smoked")]
    pub smoked: bool,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Drun,
    Run,
    Dmenu,
    Ssh,
    Emoji,
    Clipboard,
    Calc,
}
