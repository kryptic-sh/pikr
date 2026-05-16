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

fn parse_blur(s: &str) -> Result<f32, String> {
    let v: f32 = s
        .parse()
        .map_err(|_| format!("'{s}' is not a valid float"))?;
    if (0.0..=50.0).contains(&v) {
        Ok(v)
    } else {
        Err(format!("{v} is out of range 0.0..=50.0"))
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
    /// When unset, defaults to 1.0 (opaque) or 0.35 (tint over a blurred
    /// backdrop when `--blur` is also set).
    #[arg(long = "opacity", value_parser = parse_opacity)]
    pub opacity: Option<f32>,

    /// Blur strength (Gaussian sigma) for the backdrop glass effect. Range
    /// 0.0..=50.0. Setting any value enables backdrop capture + blur. Omit
    /// for no backdrop. Requires `grim` on Linux/Wayland.
    #[arg(long = "blur", value_parser = parse_blur)]
    pub blur: Option<f32>,
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
