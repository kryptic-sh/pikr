//! TOML config loader. In-memory defaults; never auto-writes a default file.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub max_results: usize,
    pub case_sensitive: bool,
    pub theme: Theme,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Theme {
    pub bg: String,
    pub fg: String,
    pub accent: String,
    pub muted: String,
    pub selected_bg: String,
    pub font: String,
    pub font_size: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_results: 256,
            case_sensitive: false,
            theme: Theme::default(),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        // Tokyonight-night palette to match the user's rofi reference.
        Self {
            bg: "#292E42".into(),
            fg: "#C0CAF5".into(),
            accent: "#21D1D3".into(),
            muted: "#979FC2".into(),
            selected_bg: "#3E4153".into(),
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
