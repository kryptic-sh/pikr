//! Entry icons: XDG icon-theme lookup and decoding, both cached.
//!
//! drun entries carry an `Icon=` value that's either an absolute path or a
//! freedesktop name (e.g. `"firefox"`). The view asks for an entry's icon only
//! as its row is built, so startup pays nothing for entries the user never
//! scrolls to.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use crcbl_image::theme::{FileVersion, IconThemes, SearchPath};
use crcbl_image::{ImageError, Limits, Rgba8Image};

/// Conventional generic-application icon names tried in order when an
/// entry's `Icon=` is missing or doesn't resolve. Hicolor + most theme
/// inheritances ship at least one of these.
pub const GENERIC_FALLBACK_NAMES: &[&str] = &[
    "application-x-executable",
    "applications-other",
    "application-x-generic",
    "applications-system",
    "exec",
];

/// Themes searched, in order, after `hicolor` misses. Some minimal systems
/// (headless sway, fresh installs) ship only hicolor, which doesn't carry
/// the conventional `application-x-executable` / `applications-other`
/// generic icons; Adwaita and Papirus do, and `breeze` is the KDE
/// equivalent.
const FALLBACK_THEMES: &[&str] = &["Adwaita", "Papirus", "breeze"];

/// How long a missed name stays cached before it is looked up again — so an
/// icon theme installed while pikr runs is picked up without waiting for a
/// restart, while a persistently-missing name isn't re-resolved on every row
/// rebuild. Hits are served for the session.
const MISS_RELOOKUP: Duration = Duration::from_secs(60);

/// Bounds on one decoded icon. `Icon=` paths come from user-writable
/// `.desktop` files, so the file is untrusted; no real icon is larger.
const ICON_LIMITS: Limits = Limits {
    max_width: 1024,
    max_height: 1024,
    max_bytes: 1024 * 1024 * 4,
};

/// One decoded file, at one size, as it was when read.
struct Decoded {
    version: FileVersion,
    size: u32,
    /// `None` when the file could not be decoded, so a broken icon is not
    /// re-read on every row rebuild either.
    image: Option<Arc<Rgba8Image>>,
}

/// Per-process icon cache: theme lookups through [`IconThemes`] (which caches
/// hits, and misses for [`MISS_RELOOKUP`]), and decoded images keyed by file
/// version, so a file replaced on disk while pikr runs is decoded again.
pub struct IconCache {
    themes: IconThemes,
    decoded: HashMap<PathBuf, Decoded>,
}

impl IconCache {
    pub fn new() -> Self {
        let themes = IconThemes::new(SearchPath::from_environment(), "hicolor", MISS_RELOOKUP)
            .with_fallback_themes(FALLBACK_THEMES.iter().map(|t| (*t).to_owned()))
            .with_generic_names(GENERIC_FALLBACK_NAMES.iter().map(|n| (*n).to_owned()));
        Self {
            themes,
            decoded: HashMap::new(),
        }
    }

    /// The file for an entry's `Icon=` value at `size` × `scale` pixels: an
    /// absolute path that exists, or a theme icon. A missing or unresolvable
    /// name falls back through [`GENERIC_FALLBACK_NAMES`], so the row still
    /// gets the generic application icon. Off Linux and the BSDs there are no
    /// theme directories, so only absolute paths resolve.
    pub fn resolve(&mut self, icon: Option<&str>, size: u32, scale: u32) -> Option<PathBuf> {
        let name = icon
            .filter(|name| !name.is_empty())
            .unwrap_or(GENERIC_FALLBACK_NAMES[0]);
        self.themes.lookup(name, size, scale)
    }

    /// `path` decoded to fit a `size` × `size` square, cached per file
    /// version. `None` when the file can't be read or decoded — including an
    /// SVG that uses something crcbl's renderer does not draw, which it
    /// refuses rather than drawing it partly.
    pub fn image(&mut self, path: &Path, size: u32) -> Option<Arc<Rgba8Image>> {
        let version = FileVersion::of(path)?;
        if let Some(cached) = self.decoded.get(path)
            && cached.version == version
            && cached.size == size
        {
            return cached.image.clone();
        }
        let image = std::fs::read(path)
            .map_err(|e| e.to_string())
            .and_then(|bytes| decode(&bytes, size).map_err(|e| e.to_string()))
            .inspect_err(|e| tracing::debug!(path = %path.display(), "icon not drawn: {e}"))
            .ok()
            .map(Arc::new);
        self.decoded.insert(
            path.to_path_buf(),
            Decoded {
                version,
                size,
                image: image.clone(),
            },
        );
        image
    }

    /// Number of decoded files cached. Used in tests.
    #[cfg(test)]
    fn decoded_len(&self) -> usize {
        self.decoded.len()
    }
}

impl Default for IconCache {
    fn default() -> Self {
        Self::new()
    }
}

/// `bytes` decoded by their format's signature, then scaled down to fit a
/// `size` × `size` square. SVG has no signature; anything unrecognised is
/// tried as SVG, which rejects what is not.
fn decode(bytes: &[u8], size: u32) -> Result<Rgba8Image, ImageError> {
    use crcbl_image::{bmp, ico, jpeg, png, resize, svg, webp};
    let image = if bytes.starts_with(b"\x89PNG") {
        png::decode(bytes, &ICON_LIMITS)?
    } else if bytes.starts_with(&[0xff, 0xd8]) {
        jpeg::decode(bytes, &ICON_LIMITS)?
    } else if bytes.starts_with(b"BM") {
        bmp::decode(bytes, &ICON_LIMITS)?
    } else if bytes.starts_with(&[0, 0, 1, 0]) {
        ico::decode(bytes, size, &ICON_LIMITS)?
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        webp::decode(bytes, &ICON_LIMITS)?
    } else {
        // Rendered straight at the target size: nothing to scale.
        return svg::decode(bytes, size, 1, &ICON_LIMITS);
    };
    let (width, height) = resize::shrink_to_fit((image.width(), image.height()), (size, size));
    resize::resize(&image, width, height, &ICON_LIMITS)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED_SQUARE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10" fill="#ff0000"/></svg>"##;
    const GREEN_SQUARE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10" fill="#00ff00"/></svg>"##;

    fn pin_mtime(path: &Path, t: std::time::SystemTime) {
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    /// The first pixel's RGBA.
    fn first_pixel(image: &Rgba8Image) -> [u8; 4] {
        image.pixels()[..4].try_into().unwrap()
    }

    #[test]
    fn absolute_path_that_exists_resolves_to_itself() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("icon.svg");
        std::fs::write(&path, RED_SQUARE).unwrap();
        let mut cache = IconCache::new();
        let name = path.to_string_lossy();
        assert_eq!(cache.resolve(Some(&name), 24, 1), Some(path.clone()));
    }

    #[test]
    fn svg_decodes_at_the_requested_size() {
        let image = decode(RED_SQUARE.as_bytes(), 24).expect("decode");
        assert_eq!((image.width(), image.height()), (24, 24));
        assert_eq!(first_pixel(&image), [0xff, 0, 0, 0xff]);
    }

    #[test]
    fn zero_size_svg_is_refused() {
        let zero = br#"<svg xmlns="http://www.w3.org/2000/svg" width="0" height="0"/>"#;
        assert!(decode(zero, 24).is_err());
    }

    #[test]
    fn garbage_is_refused() {
        assert!(decode(b"\x89PNG not-really-a-png", 24).is_err());
        assert!(decode(b"not an image at all", 24).is_err());
    }

    #[test]
    fn missing_file_is_none_and_not_cached() {
        let mut cache = IconCache::new();
        let dir = tempfile::tempdir().unwrap();
        assert!(cache.image(&dir.path().join("nope.png"), 24).is_none());
        assert_eq!(cache.decoded_len(), 0);
    }

    #[test]
    fn decode_is_cached_per_file_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("icon.svg");
        std::fs::write(&path, RED_SQUARE).unwrap();
        let mut cache = IconCache::new();
        let first = cache.image(&path, 24).expect("decode");
        let again = cache.image(&path, 24).expect("cache hit");
        assert!(
            Arc::ptr_eq(&first, &again),
            "same version must not re-decode"
        );

        // Same length, different content: only the pinned mtime moves.
        let t0 = std::time::SystemTime::now();
        std::fs::write(&path, GREEN_SQUARE).unwrap();
        pin_mtime(&path, t0 + Duration::from_millis(1));
        let second = cache.image(&path, 24).expect("re-decode");
        assert_eq!(first_pixel(&second), [0, 0xff, 0, 0xff]);
        assert_eq!(cache.decoded_len(), 1, "entry replaced, not duplicated");
    }

    #[test]
    fn a_new_size_decodes_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("icon.svg");
        std::fs::write(&path, RED_SQUARE).unwrap();
        let mut cache = IconCache::new();
        assert_eq!(cache.image(&path, 24).unwrap().width(), 24);
        assert_eq!(cache.image(&path, 48).unwrap().width(), 48);
    }

    #[test]
    fn generic_fallback_names_starts_with_application_x_executable() {
        assert_eq!(GENERIC_FALLBACK_NAMES[0], "application-x-executable");
    }

    #[test]
    #[ignore = "requires system icon theme; run with --ignored on a dev machine"]
    fn name_lookup_returns_some_for_common_icon() {
        let mut cache = IconCache::new();
        // "utilities-terminal" is present in virtually every hicolor theme.
        assert!(cache.resolve(Some("utilities-terminal"), 24, 1).is_some());
    }
}
