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

    /// Preset: smoked-glass look — equivalent to `--blur 10 --opacity 0.8`.
    /// Explicit `--blur` / `--opacity` flags override the preset values.
    #[arg(long = "smoked")]
    pub smoked: bool,
}

impl Cli {
    /// Resolve the effective `blur` value after applying presets.
    /// Explicit `--blur` wins over `--smoked`.
    pub fn effective_blur(&self) -> Option<f32> {
        self.blur.or(if self.smoked { Some(10.0) } else { None })
    }

    /// Resolve the effective `opacity` value after applying presets.
    /// Explicit `--opacity` wins over `--smoked`.
    pub fn effective_opacity(&self) -> Option<f32> {
        self.opacity.or(if self.smoked { Some(0.8) } else { None })
    }
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

#[cfg(test)]
mod smoked_tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Cli {
        let mut v = vec!["pikr"];
        v.extend_from_slice(args);
        Cli::parse_from(v)
    }

    #[test]
    fn smoked_alone_uses_both_preset_values() {
        let c = parse(&["--smoked"]);
        assert_eq!(c.effective_blur(), Some(10.0));
        assert_eq!(c.effective_opacity(), Some(0.8));
    }

    #[test]
    fn smoked_with_explicit_opacity_keeps_preset_blur() {
        let c = parse(&["--smoked", "--opacity=0.3"]);
        assert_eq!(c.effective_blur(), Some(10.0));
        assert_eq!(c.effective_opacity(), Some(0.3));
    }

    #[test]
    fn smoked_with_explicit_blur_keeps_preset_opacity() {
        let c = parse(&["--smoked", "--blur=20"]);
        assert_eq!(c.effective_blur(), Some(20.0));
        assert_eq!(c.effective_opacity(), Some(0.8));
    }

    #[test]
    fn smoked_with_both_explicit_is_a_noop() {
        let c = parse(&["--smoked", "--blur=20", "--opacity=0.3"]);
        assert_eq!(c.effective_blur(), Some(20.0));
        assert_eq!(c.effective_opacity(), Some(0.3));
    }

    #[test]
    fn no_smoked_no_preset() {
        let c = parse(&[]);
        assert_eq!(c.effective_blur(), None);
        assert_eq!(c.effective_opacity(), None);
    }
}
