//! XDG icon-theme lookup with a per-process cache.
//!
//! drun entries carry an `Icon=` value that's either an absolute path or a
//! freedesktop name (e.g. `"firefox"`). The picker view calls into
//! `IconCache::resolve` lazily as virtual_stack builds visible rows, so
//! startup pays no resolution cost for entries the user never scrolls past.

use std::{collections::HashMap, path::PathBuf};

/// Per-process XDG icon-theme cache. Stores both hits and misses so that the
/// same name is never looked up twice.
pub struct IconCache {
    cache: HashMap<String, Option<PathBuf>>,
}

impl IconCache {
    /// Create an empty cache.
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
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

        let resolved = if name_or_path.starts_with('/') {
            Some(PathBuf::from(name_or_path))
        } else {
            freedesktop_icons::lookup(name_or_path)
                .with_size(32)
                .with_scale(1)
                .find()
        };

        self.cache
            .insert(name_or_path.to_string(), resolved.clone());
        resolved
    }

    /// Number of entries currently in the cache. Used in tests.
    #[cfg(test)]
    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }
}

impl Default for IconCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
