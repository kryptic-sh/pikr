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

fn default_font() -> String {
    #[cfg(target_os = "windows")]
    {
        // Cascadia Mono ships with Windows 10 2004+, Windows 11.
        // Consolas is the legacy fallback for older Windows builds.
        // Both render the picker text legibly even without Nerd Font
        // glyphs; the Scoop manifest depends on nerd-fonts/Hack-NF for
        // users who want the full glyph coverage.
        "Cascadia Mono".into()
    }
    #[cfg(not(target_os = "windows"))]
    {
        "Hack Nerd Font Mono".into()
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
            font: default_font(),
            font_size: 14.0,
        }
    }
}

impl Config {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let path = match path {
            Some(p) => p.to_path_buf(),
            None => {
                // XDG config lookup on unix; non-unix falls back to
                // in-memory defaults until we route through `dirs`.
                #[cfg(unix)]
                {
                    let dirs = xdg::BaseDirectories::with_prefix("pikr");
                    match dirs.place_config_file("config.toml") {
                        Ok(p) => p,
                        Err(_) => return Ok(Self::default()),
                    }
                }
                #[cfg(not(unix))]
                {
                    return Ok(Self::default());
                }
            }
        };
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("read config {}", path.display()))?;
        let mut cfg: Self =
            toml::from_str(&text).with_context(|| format!("parse config {}", path.display()))?;
        // TOML accepts `font_size = nan` / `= inf`; a non-finite value would
        // flow into every layout / text-measurement call unverified, so
        // reject it back to the default.
        if !cfg.theme.font_size.is_finite() {
            tracing::warn!(
                font_size = cfg.theme.font_size,
                "non-finite font_size in config — falling back to the default"
            );
            cfg.theme.font_size = Self::default().theme.font_size;
        }
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_with(config_toml: &str) -> Config {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, config_toml).unwrap();
        Config::load(Some(&path)).unwrap()
    }

    #[test]
    fn non_finite_font_size_falls_back_to_default() {
        // TOML parses `nan` / `inf` as floats; a non-finite font_size would
        // flow into every layout call — it must be rejected back to 14.0.
        for bad in ["nan", "inf", "-inf"] {
            let cfg = load_with(&format!("[theme]\nfont_size = {bad}\n"));
            assert_eq!(cfg.theme.font_size, 14.0, "font_size = {bad}");
        }
    }

    #[test]
    fn finite_font_size_respected() {
        let cfg = load_with("[theme]\nfont_size = 18.5\n");
        assert_eq!(cfg.theme.font_size, 18.5);
    }

    #[test]
    fn missing_config_returns_defaults() {
        let cfg = Config::load(Some(std::path::Path::new(
            "/tmp/definitely-not-a-pikr-config.toml",
        )))
        .unwrap();
        assert_eq!(cfg.theme.font_size, 14.0);
        assert_eq!(cfg.max_results, 256);
    }
}
