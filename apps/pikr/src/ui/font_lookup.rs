//! The system fonts pikr last found for its theme's family, kept in the state
//! directory so the next launch reads the theme face straight from its file.
//! Asking the system is the slow part of loading fonts: fontconfig loads its
//! whole configuration and font cache on the first lookup. A launch that uses
//! the kept answer still asks, off the critical path, and keeps the new
//! answer when it differs, so a font installed or removed since shows from
//! the launch after.

use std::path::{Path, PathBuf};

use crcbl_ui::font::system::crcbl_font::{FontMatch, FontQuery, FontStyle, SystemFonts};
use serde::{Deserialize, Serialize};

/// The state file.
const FILE: &str = "fonts.toml";

/// Bumped when the file's meaning changes, so an older file reads as none.
const FORMAT: u32 = 1;

/// What the system answered for one family: the face the theme draws in and
/// the faces asked, in order, for the chars it lacks.
#[derive(Clone, Debug, PartialEq)]
pub struct Lookup {
    pub theme: FontMatch,
    pub fallbacks: Vec<FontMatch>,
}

/// Asks the system for `family`'s face, or the system sans-serif when the
/// family is not installed, and for `family`'s fallbacks.
pub fn find(family: &str) -> Result<Lookup, String> {
    let system = SystemFonts::new();
    let query = FontQuery::new(family);
    let theme = system
        .find(&query)
        .or_else(|| {
            tracing::warn!(
                family,
                "theme font not installed; using the system sans-serif"
            );
            system.find(&FontQuery::new("sans-serif"))
        })
        .ok_or_else(|| "no sans-serif font installed either".to_string())?;
    Ok(Lookup {
        theme,
        fallbacks: system.fallbacks(&query),
    })
}

/// The state file's path, `None` where the platform has no state directory.
pub fn path() -> Option<PathBuf> {
    crate::picker::state_file_path(FILE)
}

/// The lookup kept for `family` at `path`. A missing, unreadable or
/// malformed file, or one kept for another family, is none.
pub fn load(path: &Path, family: &str) -> Option<Lookup> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            tracing::warn!(error = %e, path = %path.display(), "cannot read the kept font lookup");
            return None;
        }
    };
    let kept: Kept = toml::from_str(&text)
        .inspect_err(|e| {
            tracing::warn!(error = %e, path = %path.display(), "ignoring the kept font lookup");
        })
        .ok()?;
    if kept.format != FORMAT || kept.family != family {
        return None;
    }
    Some(Lookup {
        theme: kept.theme.into(),
        fallbacks: kept.fallbacks.into_iter().map(Into::into).collect(),
    })
}

/// Keeps `lookup` as `family`'s at `path`. Best-effort; warnings via
/// `tracing` on failure.
pub fn save(path: &Path, family: &str, lookup: &Lookup) {
    let kept = Kept {
        format: FORMAT,
        family: family.to_owned(),
        theme: (&lookup.theme).into(),
        fallbacks: lookup.fallbacks.iter().map(Into::into).collect(),
    };
    let text = match toml::to_string(&kept) {
        Ok(text) => text,
        Err(e) => {
            tracing::warn!(error = %e, "cannot write the font lookup");
            return;
        }
    };
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        tracing::warn!(error = %e, path = %parent.display(), "create state dir");
        return;
    }
    match crate::picker::write_private_state(path, text.as_bytes()) {
        Ok(()) => tracing::debug!(path = %path.display(), "font lookup kept"),
        Err(e) => tracing::warn!(error = %e, path = %path.display(), "save the font lookup"),
    }
}

#[derive(Serialize, Deserialize)]
struct Kept {
    format: u32,
    /// The theme's family as configured, which the lookup answered.
    family: String,
    theme: KeptFace,
    fallbacks: Vec<KeptFace>,
}

/// A [`FontMatch`], which has no serde form of its own.
#[derive(Serialize, Deserialize)]
struct KeptFace {
    path: PathBuf,
    index: u32,
    family: String,
    weight: u16,
    style: KeptStyle,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum KeptStyle {
    Normal,
    Italic,
    Oblique,
}

impl From<&FontMatch> for KeptFace {
    fn from(face: &FontMatch) -> Self {
        Self {
            path: face.path.clone(),
            index: face.index,
            family: face.family.clone(),
            weight: face.weight,
            style: match face.style {
                FontStyle::Normal => KeptStyle::Normal,
                FontStyle::Italic => KeptStyle::Italic,
                FontStyle::Oblique => KeptStyle::Oblique,
            },
        }
    }
}

impl From<KeptFace> for FontMatch {
    fn from(face: KeptFace) -> Self {
        Self {
            path: face.path,
            index: face.index,
            family: face.family,
            weight: face.weight,
            style: match face.style {
                KeptStyle::Normal => FontStyle::Normal,
                KeptStyle::Italic => FontStyle::Italic,
                KeptStyle::Oblique => FontStyle::Oblique,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(path: &str, style: FontStyle) -> FontMatch {
        FontMatch {
            path: path.into(),
            index: 2,
            family: "Some Family".to_owned(),
            weight: 700,
            style,
        }
    }

    fn lookup() -> Lookup {
        Lookup {
            theme: face("/fonts/theme.ttf", FontStyle::Normal),
            fallbacks: vec![
                face("/fonts/cjk.ttc", FontStyle::Italic),
                face("/fonts/emoji.ttf", FontStyle::Oblique),
            ],
        }
    }

    #[test]
    fn a_kept_lookup_reads_back_for_its_family_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pikr").join(FILE);
        save(&path, "Hack", &lookup());
        assert_eq!(load(&path, "Hack"), Some(lookup()));
        assert_eq!(load(&path, "Inter"), None);
    }

    #[test]
    fn a_missing_malformed_or_older_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        assert_eq!(load(&path, "Hack"), None);
        std::fs::write(&path, "not toml [").unwrap();
        assert_eq!(load(&path, "Hack"), None);
        save(&path, "Hack", &lookup());
        let older = std::fs::read_to_string(&path)
            .unwrap()
            .replace(&format!("format = {FORMAT}"), "format = 0");
        std::fs::write(&path, older).unwrap();
        assert_eq!(load(&path, "Hack"), None);
    }
}
