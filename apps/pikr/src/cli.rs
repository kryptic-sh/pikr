//! CLI surface.

use clap::{Parser, ValueEnum};
use std::path::PathBuf;

fn parse_opacity(s: &str) -> Result<f32, String> {
    let v: f32 = s
        .parse()
        .map_err(|_| format!("'{s}' is not a valid float"))?;
    if (0.0..=1.0).contains(&v) {
        Ok(v)
    } else {
        Err(format!("{v} is out of range 0.0..=1.0"))
    }
}

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

    /// Panel background alpha, 0.0 (fully transparent) to 1.0 (opaque).
    /// Compositor must support alpha compositing for values < 1.0.
    /// Overrides `opacity` from config when set.
    #[arg(long = "opacity", value_parser = parse_opacity)]
    pub opacity: Option<f32>,

    /// Fake-glass overlay — draws procedural noise grain and a top-glow sheen
    /// on the panel surface. Independent of `--opacity`; combine both for a
    /// translucent frosted-glass look. Overrides `smoked` from config when set.
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
