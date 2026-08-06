//! XDG icon-theme lookup with a per-process cache.
//!
//! drun entries carry an `Icon=` value that's either an absolute path or a
//! freedesktop name (e.g. `"firefox"`). The picker view calls into
//! `IconCache::resolve` lazily as virtual_stack builds visible rows, so
//! startup pays no resolution cost for entries the user never scrolls past.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

/// Per-process XDG icon-theme cache. Stores both hits and misses so that the
/// same name is never looked up twice, and caches icon file bytes per file
/// version — SVG rasterised to PNG once so resvg's render pass doesn't
/// repeat, raw PNG / JPEG bytes read once so the disk isn't touched on every
/// virtual_stack frame. A `stat` per lookup checks the file's mtime/length
/// and re-reads (or re-rasterises) when the file changed on disk; a rewrite
/// that preserves both mtime and length on a coarse filesystem stays
/// undetectable, which is inherent to a stat key.
pub struct IconCache {
    /// name-or-path → resolved file path (or `None` for misses).
    cache: HashMap<String, Option<PathBuf>>,
    /// SVG file path → PNG bytes, rasterised once per file version.
    raster_cache: HashMap<PathBuf, CachedBytes>,
    /// Raw file path → file bytes, read once per file version. PNGs / JPEGs
    /// go straight to floem's `img()` from here instead of being re-read on
    /// each row rebuild.
    file_cache: HashMap<PathBuf, CachedBytes>,
}

/// Cached file bytes plus the mtime/length they were read at, so a file
/// replaced on disk while pikr runs is re-read instead of serving stale
/// bytes for the session.
struct CachedBytes {
    mtime: std::time::SystemTime,
    len: u64,
    bytes: Arc<Vec<u8>>,
}

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

/// Themes searched in order when the default-theme lookup fails. Some
/// minimal systems (headless sway, fresh installs) have no
/// `XDG_CURRENT_DESKTOP` / GTK theme set, so `freedesktop_icons::lookup`
/// falls back to hicolor — which doesn't ship the conventional
/// `application-x-executable` / `applications-other` generic icons.
/// Adwaita and Papirus do, and `breeze` is the KDE equivalent.
#[cfg(unix)]
const FALLBACK_THEMES: &[&str] = &["Adwaita", "Papirus", "breeze", "hicolor"];

/// A 32 px, scale-1 freedesktop icon lookup — the shared starting point for
/// the primary and fallback-theme lookups in [`IconCache::resolve`]. Takes
/// the name as a parameter (not a capture) so the returned builder can borrow
/// it for the `.with_theme(t).find()` chain.
#[cfg(unix)]
fn icon_lookup(name: &str) -> freedesktop_icons::LookupBuilder<'_> {
    freedesktop_icons::lookup(name).with_size(32).with_scale(1)
}

impl IconCache {
    /// Create an empty cache.
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
            raster_cache: HashMap::new(),
            file_cache: HashMap::new(),
        }
    }

    /// Resolve `name_or_path` to a usable file path.
    ///
    /// - Absolute paths (`/…`) pass through verbatim.
    /// - Bare freedesktop icon names go through `freedesktop_icons::lookup`
    ///   with a 32 px request size (we render at 24, so a 32 source
    ///   downscales cleanly).
    ///
    /// Results — including `None` for misses — are cached so we don't
    /// re-resolve the same name on every virtual_stack frame.
    pub fn resolve(&mut self, name_or_path: &str) -> Option<PathBuf> {
        if name_or_path.is_empty() {
            return None;
        }

        if let Some(cached) = self.cache.get(name_or_path) {
            return cached.clone();
        }

        let resolved = if Path::new(name_or_path).is_absolute() {
            // Absolute path on any OS (Unix: starts with `/`;
            // Windows: starts with a drive letter like `C:\`).
            // Pass through verbatim — no theme lookup needed.
            Some(PathBuf::from(name_or_path))
        } else {
            // freedesktop_icons is unix-only. On macOS / Windows bare
            // icon names don't resolve to anything useful — the picker
            // shows no icon and the cache stores the miss.
            #[cfg(unix)]
            {
                icon_lookup(name_or_path).find().or_else(|| {
                    FALLBACK_THEMES
                        .iter()
                        .find_map(|theme| icon_lookup(name_or_path).with_theme(theme).find())
                })
            }
            #[cfg(not(unix))]
            {
                None
            }
        };

        self.cache
            .insert(name_or_path.to_string(), resolved.clone());
        resolved
    }

    /// Resolve `primary` if set, otherwise walk `fallbacks` in order and
    /// return the first match. Used by the picker so drun entries with a
    /// missing or unresolvable `Icon=` still render an icon slot instead
    /// of dead space. Returns `None` only when every option misses —
    /// `application-x-executable` is the conventional generic-app icon but
    /// not every theme installs it, so we try a chain of common names.
    pub fn resolve_or_fallback(
        &mut self,
        primary: Option<&str>,
        fallbacks: &[&str],
    ) -> Option<PathBuf> {
        if let Some(name) = primary
            && let Some(p) = self.resolve(name)
        {
            return Some(p);
        }
        for fb in fallbacks {
            if let Some(p) = self.resolve(fb) {
                return Some(p);
            }
        }
        None
    }

    /// Rasterise `svg_path` to PNG bytes at `size_px` × `size_px`, cached per
    /// file version. Returns `None` if the file can't be read or parsed.
    ///
    /// A `stat` per lookup checks the file's mtime and length; when either
    /// differs from the cached entry the SVG is re-rasterised, so an icon
    /// replaced on disk while pikr runs picks up the new bytes. The stat is
    /// far cheaper than the render pass it lets us skip.
    ///
    /// Bypasses floem's `svg()` view, which goes through vello's SVG path
    /// and silently drops paths that use features it doesn't fully
    /// support (markers, complex clip-paths, …). Many app icons end up
    /// rendering only their solid background plate that way — looks like
    /// a black square. resvg handles the full SVG 1.1 surface, produces
    /// a correct raster, and we route the PNG through floem's `img()`.
    pub fn rasterise_svg(&mut self, svg_path: &Path, size_px: u32) -> Option<Arc<Vec<u8>>> {
        let meta = std::fs::metadata(svg_path).ok()?;
        let Some(mtime) = meta.modified().ok() else {
            // No mtime to key freshness on — rasterise without caching.
            return render_svg_to_png(svg_path, size_px).map(Arc::new);
        };
        if let Some(cached) = self.raster_cache.get(svg_path)
            && cached.mtime == mtime
            && cached.len == meta.len()
        {
            return Some(cached.bytes.clone());
        }
        let bytes = render_svg_to_png(svg_path, size_px)?;
        let arc = Arc::new(bytes);
        self.raster_cache.insert(
            svg_path.to_path_buf(),
            CachedBytes {
                mtime,
                len: meta.len(),
                bytes: arc.clone(),
            },
        );
        Some(arc)
    }

    /// Read `path`'s bytes, cached per file version for subsequent calls. PNG /
    /// JPEG files go straight to floem's `img()`; caching the bytes avoids a
    /// disk read on every row rebuild (the virtual_stack rebuilds visible rows
    /// on each rerank and scroll).
    ///
    /// A `stat` per lookup checks the file's mtime and length; when either
    /// differs from the cached entry the file is re-read, so an icon replaced
    /// on disk while pikr runs picks up the new bytes. The stat is far cheaper
    /// than the disk read it lets us skip.
    pub fn file_bytes(&mut self, path: &Path) -> Option<Arc<Vec<u8>>> {
        let meta = std::fs::metadata(path).ok()?;
        let Some(mtime) = meta.modified().ok() else {
            // No mtime to key freshness on — read without caching.
            return std::fs::read(path).ok().map(Arc::new);
        };
        if let Some(cached) = self.file_cache.get(path)
            && cached.mtime == mtime
            && cached.len == meta.len()
        {
            return Some(cached.bytes.clone());
        }
        let bytes = std::fs::read(path).ok()?;
        let arc = Arc::new(bytes);
        self.file_cache.insert(
            path.to_path_buf(),
            CachedBytes {
                mtime,
                len: meta.len(),
                bytes: arc.clone(),
            },
        );
        Some(arc)
    }

    /// Number of entries currently in the cache. Used in tests.
    #[cfg(test)]
    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }

    /// Number of rasterised SVGs cached. Used in tests.
    #[cfg(test)]
    pub fn raster_cache_len(&self) -> usize {
        self.raster_cache.len()
    }

    /// Number of raw file-byte entries cached. Used in tests.
    #[cfg(test)]
    pub fn file_cache_len(&self) -> usize {
        self.file_cache.len()
    }
}

/// Pure helper — SVG file → PNG bytes via resvg + tiny-skia. Scales the
/// rasterised image so the longer side hits `size_px`, then draws into a
/// square `size_px × size_px` pixmap (transparent fill).
fn render_svg_to_png(svg_path: &Path, size_px: u32) -> Option<Vec<u8>> {
    let data = std::fs::read(svg_path).ok()?;
    let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).ok()?;
    let svg_size = tree.size();
    let dim = svg_size.width().max(svg_size.height());
    // A crafted `<svg width="0" height="0">` yields a zero tree size; the
    // scale would be `size_px / 0` = inf and tiny-skia would rasterise
    // garbage. Bail out — the caller treats None as "no icon".
    if dim <= 0.0 || !dim.is_finite() {
        return None;
    }
    let scale = size_px as f32 / dim;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size_px, size_px)?;
    let transform = resvg::tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());
    pixmap.encode_png().ok()
}

impl Default for IconCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pin_mtime(path: &Path, t: std::time::SystemTime) {
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    #[test]
    fn absolute_path_passthrough() {
        let mut cache = IconCache::new();
        let path = "/usr/share/icons/hicolor/32x32/apps/firefox.png";
        let result = cache.resolve(path);
        assert_eq!(result, Some(PathBuf::from(path)));
    }

    #[test]
    fn empty_string_returns_none() {
        let mut cache = IconCache::new();
        assert_eq!(cache.resolve(""), None);
    }

    #[test]
    fn empty_string_not_cached() {
        // Empty string short-circuits before the cache insert, so cache stays empty.
        let mut cache = IconCache::new();
        let _ = cache.resolve("");
        let _ = cache.resolve("");
        assert_eq!(cache.cache_len(), 0);
    }

    #[test]
    fn cache_hit_does_not_re_lookup() {
        let mut cache = IconCache::new();
        // Use an absolute path so the result is deterministic regardless of
        // the system's installed icon themes.
        let path = "/tmp/some-icon.png";
        cache.resolve(path);
        cache.resolve(path);
        // Only one entry in cache regardless of how many times we ask.
        assert_eq!(cache.cache_len(), 1);
    }

    #[test]
    fn different_names_cached_separately() {
        let mut cache = IconCache::new();
        cache.resolve("/tmp/icon-a.png");
        cache.resolve("/tmp/icon-b.png");
        assert_eq!(cache.cache_len(), 2);
    }

    #[test]
    fn fallback_used_when_primary_is_none() {
        // Fallback path is absolute so the test is deterministic regardless
        // of the system icon theme.
        let mut cache = IconCache::new();
        let result = cache.resolve_or_fallback(None, &["/tmp/fallback.png"]);
        assert_eq!(result, Some(PathBuf::from("/tmp/fallback.png")));
    }

    #[test]
    fn fallback_used_when_primary_misses() {
        // Primary is a bare name that almost certainly isn't installed
        // (random suffix); fallback is an absolute path passthrough.
        let mut cache = IconCache::new();
        let result =
            cache.resolve_or_fallback(Some("zzz-no-such-icon-xyz"), &["/tmp/fallback.png"]);
        assert_eq!(result, Some(PathBuf::from("/tmp/fallback.png")));
    }

    #[test]
    fn fallback_walks_chain_until_a_hit() {
        // First entry is a bare name that won't resolve; second is an
        // absolute path that always does.
        let mut cache = IconCache::new();
        let result =
            cache.resolve_or_fallback(None, &["zzz-no-such-icon-xyz", "/tmp/late-fallback.png"]);
        assert_eq!(result, Some(PathBuf::from("/tmp/late-fallback.png")));
    }

    #[test]
    fn fallback_returns_none_when_chain_exhausted() {
        let mut cache = IconCache::new();
        let result = cache.resolve_or_fallback(None, &["zzz-nope-1", "zzz-nope-2"]);
        assert_eq!(result, None);
    }

    #[test]
    fn primary_wins_when_resolvable() {
        let mut cache = IconCache::new();
        let result = cache.resolve_or_fallback(Some("/tmp/primary.png"), &["/tmp/fallback.png"]);
        assert_eq!(result, Some(PathBuf::from("/tmp/primary.png")));
    }

    #[test]
    fn rasterise_missing_file_is_none() {
        let mut cache = IconCache::new();
        let result = cache.rasterise_svg(Path::new("/tmp/does-not-exist.svg"), 24);
        assert!(result.is_none());
        assert_eq!(cache.raster_cache_len(), 0);
    }

    #[test]
    fn rasterise_simple_svg_round_trip() {
        // Tiny inline SVG that resvg + tiny-skia can absolutely render.
        let dir = std::env::temp_dir();
        let path = dir.join("pikr-test-icon.svg");
        std::fs::write(
            &path,
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10" fill="#ff0000"/></svg>"##,
        )
        .unwrap();
        let mut cache = IconCache::new();
        let bytes = cache.rasterise_svg(&path, 24).expect("rasterise");
        // PNG magic header.
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        // Second call must hit the cache (no re-rasterise).
        let bytes2 = cache.rasterise_svg(&path, 24).expect("cache hit");
        assert!(Arc::ptr_eq(&bytes, &bytes2));
        assert_eq!(cache.raster_cache_len(), 1);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn rasterise_zero_size_svg_returns_none() {
        // A zero-sized SVG must not reach the scale division (`size_px / 0`
        // = inf → tiny-skia garbage). resvg may reject the tree outright or
        // parse it with a zero size — either way the result must be None.
        let dir = std::env::temp_dir();
        let path = dir.join("pikr-test-zero-size.svg");
        std::fs::write(
            &path,
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="0" height="0"/>"##,
        )
        .unwrap();
        let result = render_svg_to_png(&path, 24);
        assert!(result.is_none());
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn file_bytes_caches_and_dedupes() {
        // The cache doesn't parse the bytes, so any content works.
        let dir = std::env::temp_dir();
        let path = dir.join("pikr-test-icon.png");
        std::fs::write(&path, b"\x89PNG not-really-a-png").unwrap();
        let mut cache = IconCache::new();
        let bytes = cache.file_bytes(&path).expect("read");
        // Second call must hit the cache (no re-read) — same Arc.
        let bytes2 = cache.file_bytes(&path).expect("cache hit");
        assert!(Arc::ptr_eq(&bytes, &bytes2));
        assert_eq!(cache.file_cache_len(), 1);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn file_bytes_missing_file_is_none() {
        let mut cache = IconCache::new();
        let result = cache.file_bytes(Path::new("/tmp/does-not-exist.png"));
        assert!(result.is_none());
        assert_eq!(cache.file_cache_len(), 0);
    }

    #[test]
    fn file_bytes_invalidates_on_mtime_change() {
        // An icon replaced on disk while pikr runs must be re-read, not served
        // from the session cache. Content changes with the SAME byte length, so
        // only the pinned mtime distinguishes the versions.
        let dir = std::env::temp_dir();
        let path = dir.join("pikr-test-icon-replaced.png");
        std::fs::write(&path, b"AAAA").unwrap();
        let mut cache = IconCache::new();
        let first = cache.file_bytes(&path).expect("read");
        assert_eq!(&first[..], b"AAAA");

        let t0 = std::time::SystemTime::now();
        std::fs::write(&path, b"BBBB").unwrap();
        pin_mtime(&path, t0 + std::time::Duration::from_millis(1));

        let second = cache.file_bytes(&path).expect("re-read");
        assert_ne!(
            &second[..],
            &first[..],
            "changed mtime must re-read the file"
        );
        assert_eq!(&second[..], b"BBBB");
        assert_eq!(
            cache.file_cache_len(),
            1,
            "entry must be replaced, not duplicated"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn file_bytes_invalidates_on_len_change() {
        // Same pinned mtime, different length: the len half of the key must
        // invalidate on its own.
        let dir = std::env::temp_dir();
        let path = dir.join("pikr-test-icon-len.png");
        std::fs::write(&path, b"AAAA").unwrap();
        let mut cache = IconCache::new();
        let first = cache.file_bytes(&path).expect("read");

        let t0 = std::time::SystemTime::now();
        std::fs::write(&path, b"BB").unwrap();
        pin_mtime(&path, t0); // restore the original mtime — only len differs

        let second = cache.file_bytes(&path).expect("re-read");
        assert_ne!(
            &second[..],
            &first[..],
            "changed length must re-read the file"
        );
        assert_eq!(&second[..], b"BB");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn rasterise_svg_invalidates_on_change() {
        // A replaced SVG must re-rasterise: the old PNG bytes must not survive.
        let dir = std::env::temp_dir();
        let path = dir.join("pikr-test-icon-replaced.svg");
        std::fs::write(
            &path,
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10" fill="#ff0000"/></svg>"##,
        )
        .unwrap();
        let mut cache = IconCache::new();
        let first = cache.rasterise_svg(&path, 24).expect("rasterise");
        assert_eq!(&first[..8], b"\x89PNG\r\n\x1a\n");

        let t0 = std::time::SystemTime::now();
        std::fs::write(
            &path,
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10" fill="#00ff00"/></svg>"##,
        )
        .unwrap();
        pin_mtime(&path, t0 + std::time::Duration::from_millis(1));

        let second = cache.rasterise_svg(&path, 24).expect("re-rasterise");
        assert!(
            !Arc::ptr_eq(&first, &second),
            "a changed SVG must re-rasterise, not return the cached PNG"
        );
        assert_eq!(
            cache.raster_cache_len(),
            1,
            "entry must be replaced, not duplicated"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn generic_fallback_names_starts_with_application_x_executable() {
        // The conventional name comes first; later entries are fallbacks
        // for systems that don't ship the canonical one.
        assert_eq!(GENERIC_FALLBACK_NAMES[0], "application-x-executable");
        assert!(!GENERIC_FALLBACK_NAMES.is_empty());
    }

    #[test]
    #[ignore = "requires system icon theme; run with --ignored on a dev machine"]
    fn name_lookup_returns_some_for_common_icon() {
        let mut cache = IconCache::new();
        // "utilities-terminal" is present in virtually every hicolor theme.
        let result = cache.resolve("utilities-terminal");
        assert!(
            result.is_some(),
            "expected system icon lookup to succeed for 'utilities-terminal'"
        );
        // Second call must come from cache.
        let result2 = cache.resolve("utilities-terminal");
        assert_eq!(result, result2);
        assert_eq!(cache.cache_len(), 1);
    }
}
