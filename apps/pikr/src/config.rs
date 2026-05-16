//! TOML config loader. In-memory defaults; never auto-writes a default file.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub max_results: usize,
    pub case_sensitive: bool,
    /// Panel background alpha. 1.0 = fully opaque (default). Values < 1.0
    /// require an alpha-compositing compositor (Hyprland / KWin / Sway on
    /// Wayland; picom etc on X11).
    pub opacity: f32,
    /// Fake-glass overlay — procedural noise grain + top-glow sheen painted
    /// over the panel surface. Independent of `opacity`; combine both for a
    /// frosted-glass look. Off by default.
    pub smoked: bool,
    pub theme: Theme,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Theme {
    pub bg: String,
    pub fg: String,
    pub accent: String,
    pub font: String,
    pub font_size: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_results: 256,
            case_sensitive: false,
            opacity: 1.0,
            smoked: false,
            theme: Theme::default(),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            bg: "#1e1e2e".into(),
            fg: "#cdd6f4".into(),
            accent: "#89b4fa".into(),
            font: "Hack Nerd Font Mono".into(),
            font_size: 14.0,
        }
    }
}

impl Config {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let path = match path {
            Some(p) => p.to_path_buf(),
            None => {
                let dirs = match xdg::BaseDirectories::with_prefix("pikr") {
                    Ok(d) => d,
                    Err(_) => return Ok(Self::default()),
                };
                match dirs.place_config_file("config.toml") {
                    Ok(p) => p,
                    Err(_) => return Ok(Self::default()),
                }
            }
        };
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("read config {}", path.display()))?;
        let cfg: Self =
            toml::from_str(&text).with_context(|| format!("parse config {}", path.display()))?;
        Ok(cfg)
    }
}
