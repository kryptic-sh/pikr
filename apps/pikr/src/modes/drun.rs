//! drun mode — application launcher.
//!
//! - Unix: parses XDG `.desktop` files via `freedesktop-desktop-entry`.
//! - Windows: walks Start Menu `.lnk` shortcuts from both the per-user and
//!   all-users trees. Icon resolution is deferred to a follow-up issue.
//! - Other targets: returns an empty list.

use super::{Entry, Mode};
use anyhow::Result;

#[derive(Default)]
pub struct Drun;

impl Mode for Drun {
    fn name(&self) -> &'static str {
        "drun"
    }

    fn collect(&mut self) -> Result<Vec<Entry>> {
        #[cfg(unix)]
        return unix_impl::collect();
        #[cfg(windows)]
        return windows_impl::collect();
        #[cfg(not(any(unix, windows)))]
        return Ok(Vec::new());
    }
}

// ── Unix — XDG `.desktop` parser ─────────────────────────────────────────────

#[cfg(unix)]
mod unix_impl {
    use super::{Entry, Result};
    use freedesktop_desktop_entry::{DesktopEntry, Iter, default_paths};
    use std::collections::HashMap;

    pub fn collect() -> Result<Vec<Entry>> {
        let locales = current_locales();
        // Map app-id → entry so later `.desktop` files (user-local) override
        // earlier ones (system) per freedesktop spec.
        let mut by_id: HashMap<String, Entry> = HashMap::new();

        for path in Iter::new(default_paths()) {
            let Ok(de) = DesktopEntry::from_path(path, Some(&locales)) else {
                continue;
            };

            if de.no_display() || de.hidden() {
                continue;
            }
            if de.type_().is_some_and(|ty| ty != "Application") {
                continue;
            }

            let Some(exec_raw) = de.exec() else {
                continue;
            };
            let Some((program, args)) = parse_exec(exec_raw) else {
                continue;
            };

            let label = de
                .name(&locales)
                .map(|s| s.into_owned())
                .unwrap_or_else(|| program.clone());

            let description = de
                .generic_name(&locales)
                .or_else(|| de.comment(&locales))
                .map(|s| s.into_owned());

            let icon = de.icon().map(|s| s.to_string());
            let id = de.appid.clone();
            let mut entry = Entry::exec(label, program).with_args(args);
            if let Some(d) = description {
                entry = entry.with_description(d);
            }
            if let Some(i) = icon {
                entry = entry.with_icon(i);
            }
            by_id.insert(id, entry);
        }

        let mut entries: Vec<Entry> = by_id.into_values().collect();
        entries.sort_by_key(|a| a.label.to_lowercase());
        Ok(entries)
    }

    fn current_locales() -> Vec<String> {
        let mut out = Vec::new();
        for var in ["LC_MESSAGES", "LC_ALL", "LANG"] {
            if let Ok(v) = std::env::var(var) {
                let trimmed = v.split('.').next().unwrap_or(&v);
                if !trimmed.is_empty() && trimmed != "C" && trimmed != "POSIX" {
                    out.push(trimmed.to_string());
                    break;
                }
            }
        }
        out
    }

    /// Split an `Exec=` string per freedesktop spec, dropping field codes
    /// (`%f` / `%F` / `%u` / `%U` / `%i` / `%c` / `%k`). Returns
    /// `(program, args)` or `None` if the line is unusable.
    pub fn parse_exec(raw: &str) -> Option<(String, Vec<String>)> {
        let tokens = shlex::split(raw)?;
        let mut out = Vec::with_capacity(tokens.len());
        for tok in tokens {
            let cleaned = strip_field_codes(&tok);
            if cleaned.is_empty() {
                continue;
            }
            out.push(cleaned);
        }
        let mut iter = out.into_iter();
        let program = iter.next()?;
        Some((program, iter.collect()))
    }

    /// Remove field-code substitutions (`%X` for single-letter `X`) from a token.
    /// A `%%` literal collapses to `%`.
    pub fn strip_field_codes(tok: &str) -> String {
        let mut out = String::with_capacity(tok.len());
        let mut chars = tok.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '%' {
                match chars.next() {
                    Some('%') => out.push('%'),
                    Some(_) => {} // single-char field code — drop
                    None => {}
                }
            } else {
                out.push(c);
            }
        }
        out
    }
}

// ── Windows — Start Menu `.lnk` walker ───────────────────────────────────────
//
// Two Start Menu trees are scanned:
//   Per-user:  %APPDATA%\Microsoft\Windows\Start Menu\Programs\
//   All-users: %ProgramData%\Microsoft\Windows\Start Menu\Programs\
//
// Each `.lnk` is parsed with the `lnk` crate (pure Rust, no Win32 calls).
// The label is the shortcut's filename stem; the description is the parent
// folder name (mirrors XDG Categories). The payload is Exec { target, args }.
//
// Performance (#42):
// - Step 1: Walk is serial (cheap — just filesystem stat calls).
// - Step 2: Parse is parallel via rayon::par_iter (lnk open + parse +
//   Path::exists() are the expensive part; fans out across all hardware threads).
// - Cache: after a successful walk+parse, results are written to
//   %LOCALAPPDATA%\pikr\drun-cache.json keyed by the max mtime across both
//   Start Menu roots. On the next run, if the mtime hasn't changed the walk is
//   skipped entirely and the cached entries are returned directly.
//
// Note on cache staleness: the Path::exists() check happens at parse time and
// is not repeated on cache hit. An entry that existed at cache time but has
// since been uninstalled will show up in the list until the cache invalidates
// (i.e. until something else modifies the Start Menu dir). This is an
// intentional tradeoff — checking every target path on every startup defeats
// the purpose of caching.
//
// TODO: icon resolution via the `lnk` icon_location field is deferred to a
// follow-up issue (#38 phase 2). For now all entries have icon = None.

#[cfg(windows)]
mod windows_impl {
    use super::{Entry, Result};
    use rayon::prelude::*;
    use serde::{Deserialize, Serialize};
    use std::path::{Path, PathBuf};
    use std::time::SystemTime;
    use walkdir::WalkDir;

    // ── Cache types ───────────────────────────────────────────────────────────

    /// On-disk cache for drun entries.
    ///
    /// Keyed by the max mtime (in Unix seconds) across both Start Menu roots.
    /// If any root's directory mtime has advanced since the cache was written
    /// the whole cache is considered stale and the walk is re-run.
    #[derive(Serialize, Deserialize)]
    struct CachedDrun {
        /// Max of all roots' dir-mtimes at cache time.  Invalidate when any
        /// root mtime is newer than this value.
        roots_mtime_unix_secs: u64,
        /// Cached entries — labels, target paths, args.  Icons not yet (#40).
        entries: Vec<CachedEntry>,
    }

    /// A single cached entry.  Mirrors the fields we populate in [`parse_lnk`].
    #[derive(Serialize, Deserialize)]
    struct CachedEntry {
        label: String,
        description: Option<String>,
        target: String,
        args: Vec<String>,
    }

    impl From<CachedEntry> for Entry {
        fn from(c: CachedEntry) -> Self {
            let mut e = Entry::exec(c.label, c.target).with_args(c.args);
            if let Some(d) = c.description {
                e = e.with_description(d);
            }
            e
        }
    }

    impl From<&Entry> for CachedEntry {
        fn from(e: &Entry) -> Self {
            use super::super::Payload;
            let (target, args) = match &e.payload {
                Payload::Exec { program, args } => (program.clone(), args.clone()),
                _ => (String::new(), Vec::new()),
            };
            CachedEntry {
                label: e.label.clone(),
                description: e.description.clone(),
                target,
                args,
            }
        }
    }

    // ── Cache helpers ─────────────────────────────────────────────────────────

    /// Path to the drun cache file: `%LOCALAPPDATA%\pikr\drun-cache.json`.
    ///
    /// Uses `dirs::data_local_dir()` which resolves to `%LOCALAPPDATA%` on
    /// Windows (e.g. `C:\Users\<user>\AppData\Local`).
    fn cache_path() -> Option<PathBuf> {
        dirs::data_local_dir().map(|d| d.join("pikr").join("drun-cache.json"))
    }

    /// Compute the max mtime (Unix seconds) across all existing Start Menu roots.
    /// Returns `None` if no root exists or no mtime is readable.
    fn roots_mtime(roots: &[PathBuf]) -> Option<u64> {
        roots
            .iter()
            .filter(|r| r.exists())
            .filter_map(|r| {
                r.metadata()
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
            })
            .max()
    }

    /// Try to load a valid cache. Returns `Some(entries)` on hit, `None` on miss.
    pub fn load_cache(current_mtime: u64, path: &Path) -> Option<Vec<Entry>> {
        let _span = tracing::debug_span!("drun_cache_load", path = %path.display()).entered();
        let bytes = std::fs::read(path).ok()?;
        let cached: CachedDrun = serde_json::from_slice(&bytes)
            .map_err(|e| tracing::warn!("drun cache parse error: {e}"))
            .ok()?;
        if cached.roots_mtime_unix_secs != current_mtime {
            tracing::debug!(
                cached = cached.roots_mtime_unix_secs,
                current = current_mtime,
                "drun cache mtime mismatch — invalidating"
            );
            return None;
        }
        tracing::debug!(entries = cached.entries.len(), "drun cache hit");
        Some(cached.entries.into_iter().map(Entry::from).collect())
    }

    /// Write a fresh cache to disk. Best-effort — logs a warning on any error.
    pub fn write_cache(mtime: u64, entries: &[Entry], path: &Path) {
        let _span = tracing::debug_span!("drun_cache_write", path = %path.display()).entered();
        let cached = CachedDrun {
            roots_mtime_unix_secs: mtime,
            entries: entries.iter().map(CachedEntry::from).collect(),
        };
        let write = || -> std::io::Result<()> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let json = serde_json::to_vec_pretty(&cached)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
            std::fs::write(path, json)
        };
        if let Err(e) = write() {
            tracing::warn!("drun cache write failed: {e}");
        }
    }

    // ── Public collect ────────────────────────────────────────────────────────

    pub fn collect() -> Result<Vec<Entry>> {
        let _span = tracing::debug_span!("drun_collect_windows").entered();

        let roots = start_menu_roots();
        let current_mtime = roots_mtime(&roots);

        // --- Cache probe ---
        {
            let _span = tracing::debug_span!("drun_cache_probe").entered();
            if let (Some(mtime), Some(path)) = (current_mtime, cache_path()) {
                if let Some(entries) = load_cache(mtime, &path) {
                    return Ok(entries);
                }
            }
        }

        // --- Step 1: serial walk — cheap filesystem stat only ---
        let lnk_paths: Vec<PathBuf> = {
            let _span = tracing::debug_span!("drun_walk").entered();
            roots
                .iter()
                .filter(|r| r.exists())
                .flat_map(|root| {
                    WalkDir::new(root)
                        .follow_links(false)
                        .into_iter()
                        .filter_map(Result::ok)
                })
                .map(|e| e.into_path())
                .filter(|p| {
                    p.extension()
                        .and_then(|e| e.to_str())
                        .map_or(false, |e| e.eq_ignore_ascii_case("lnk"))
                })
                .collect()
        };

        // --- Step 2: parallel parse — lnk open + parse + Path::exists() ---
        let mut entries: Vec<Entry> = {
            let _span = tracing::debug_span!("drun_parse", lnk_count = lnk_paths.len()).entered();
            lnk_paths
                .par_iter()
                .filter_map(|path| parse_lnk(path))
                .collect()
        };

        entries.sort_by_key(|e| e.label.to_lowercase());

        // --- Cache write (best-effort) ---
        if let (Some(mtime), Some(path)) = (current_mtime, cache_path()) {
            write_cache(mtime, &entries, &path);
        }

        Ok(entries)
    }

    /// Return the Start Menu `Programs` roots to scan.
    ///
    /// Per-user path uses `%APPDATA%` directly rather than `dirs::data_dir()`
    /// because `dirs` maps `data_dir` to `%APPDATA%\Roaming` (same location),
    /// but using the env var keeps the intent explicit and avoids the `dirs`
    /// dependency on Windows.
    pub fn start_menu_roots() -> Vec<PathBuf> {
        let mut roots = Vec::new();
        if let Some(appdata) = std::env::var_os("APPDATA") {
            roots.push(
                PathBuf::from(appdata)
                    .join("Microsoft")
                    .join("Windows")
                    .join("Start Menu")
                    .join("Programs"),
            );
        }
        if let Some(programdata) = std::env::var_os("ProgramData") {
            roots.push(
                PathBuf::from(programdata)
                    .join("Microsoft")
                    .join("Windows")
                    .join("Start Menu")
                    .join("Programs"),
            );
        }
        roots
    }

    /// Parse a single `.lnk` shortcut into an [`Entry`], or return `None` if
    /// it should be skipped (unresolvable target, document shortcut, or
    /// uninstaller).
    ///
    /// Filter rules:
    /// - Skip if the lnk crate returns a parse error.
    /// - Skip if no target path is available (e.g. URL shortcut stored as .lnk).
    /// - Skip if the target extension is `.txt`, `.url`, `.pdf`, `.html`, or `.htm`.
    /// - Skip if the parent folder name contains "uninstall" (case-insensitive).
    /// - Skip if the resolved target path does not exist on disk (broken shortcut).
    pub fn parse_lnk(path: &Path) -> Option<Entry> {
        // Parent folder name is used as description (mirrors XDG Categories).
        let parent_name = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        // Skip uninstaller folders.
        if parent_name.to_lowercase().contains("uninstall") {
            return None;
        }

        let shortcut = lnk::ShellLink::open(path, lnk::encoding::WINDOWS_1252).ok()?;

        // link_target() constructs the full target path from LinkInfo.
        let target = shortcut.link_target()?;

        // Skip non-program shortcuts by target extension.
        let skip_exts = ["txt", "url", "pdf", "html", "htm"];
        let target_path = Path::new(&target);
        if let Some(ext) = target_path.extension().and_then(|e| e.to_str()) {
            if skip_exts.iter().any(|&skip| ext.eq_ignore_ascii_case(skip)) {
                return None;
            }
        }

        // Skip broken shortcuts — target must exist on disk.
        if !target_path.exists() {
            return None;
        }

        // Shortcut filename stem becomes the label.
        let label = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        if label.is_empty() {
            return None;
        }

        // Parse command-line arguments from StringData, splitting on shell
        // word boundaries (spaces, with basic quoting handled by shlex).
        let args: Vec<String> = shortcut
            .string_data()
            .command_line_arguments()
            .as_deref()
            .and_then(|s| shlex::split(s))
            .unwrap_or_default();

        let mut entry = Entry::exec(label, target).with_args(args);

        // Parent folder name as description if non-empty and not the root
        // "Programs" folder itself.
        if !parent_name.is_empty() && !parent_name.eq_ignore_ascii_case("Programs") {
            entry = entry.with_description(parent_name);
        }

        // TODO(#38 phase 2): resolve icon from shortcut.string_data().icon_location()
        // or shortcut.extra_data() via the freedesktop-icons equivalent on Windows.

        Some(entry)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, unix))]
mod tests {
    use super::unix_impl::{parse_exec, strip_field_codes};

    #[test]
    fn strip_field_code_alone() {
        assert_eq!(strip_field_codes("%U"), "");
        assert_eq!(strip_field_codes("%f"), "");
    }

    #[test]
    fn strip_field_code_inline() {
        assert_eq!(strip_field_codes("--url=%u"), "--url=");
    }

    #[test]
    fn strip_double_percent() {
        assert_eq!(strip_field_codes("100%%"), "100%");
    }

    #[test]
    fn parse_simple() {
        let (prog, args) = parse_exec("firefox %U").unwrap();
        assert_eq!(prog, "firefox");
        assert!(args.is_empty());
    }

    #[test]
    fn parse_with_arg() {
        let (prog, args) = parse_exec("alacritty -e tmux %f").unwrap();
        assert_eq!(prog, "alacritty");
        assert_eq!(args, vec!["-e".to_string(), "tmux".to_string()]);
    }

    #[test]
    fn parse_quoted() {
        let (prog, args) = parse_exec(r#"foo --opt "two words" %u"#).unwrap();
        assert_eq!(prog, "foo");
        assert_eq!(args, vec!["--opt".to_string(), "two words".to_string()]);
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::windows_impl::{load_cache, start_menu_roots, write_cache};

    /// start_menu_roots() must return at least one path when %APPDATA% or
    /// %ProgramData% env vars are set (as they always are on real Windows).
    /// Returns an empty vec on hosts without those env vars set (CI / cross-
    /// compile check hosts), which is also acceptable.
    #[test]
    fn start_menu_roots_returns_at_least_one_path() {
        // On a real Windows host both env vars are set; on a cross-compile CI
        // host neither may be set. Both outcomes are valid — we just assert
        // no panic and that the return type is correct.
        let roots = start_menu_roots();
        // If the env vars are set, expect ≥1 root.
        let has_appdata = std::env::var_os("APPDATA").is_some();
        let has_programdata = std::env::var_os("ProgramData").is_some();
        if has_appdata || has_programdata {
            assert!(
                !roots.is_empty(),
                "start_menu_roots must return ≥1 entry when env vars are set"
            );
        }
    }

    /// Serialize a `CachedDrun` with a couple of entries, write to a temp file,
    /// read back via `load_cache` with the same mtime, and assert equality.
    #[test]
    fn cache_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drun-cache.json");

        let mtime: u64 = 1_700_000_000;
        let entries = vec![
            super::super::Entry::exec("Firefox", "C:\\Program Files\\Firefox\\firefox.exe"),
            super::super::Entry::exec("Notepad", "C:\\Windows\\notepad.exe")
                .with_description("Windows Accessories".to_string()),
        ];

        write_cache(mtime, &entries, &path);
        assert!(path.exists(), "cache file must be written");

        let loaded = load_cache(mtime, &path).expect("cache must hit on same mtime");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].label, "Firefox");
        assert_eq!(loaded[1].label, "Notepad");
        assert_eq!(
            loaded[1].description.as_deref(),
            Some("Windows Accessories")
        );
    }

    /// Write a cache with mtime=100; query with current_mtime=200.
    /// `load_cache` must return `None` (cache miss / invalidated).
    #[test]
    fn cache_invalidates_on_mtime_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drun-cache.json");

        let cached_mtime: u64 = 100;
        let current_mtime: u64 = 200;

        let entries = vec![super::super::Entry::exec(
            "TestApp",
            "C:\\Program Files\\TestApp\\app.exe",
        )];

        write_cache(cached_mtime, &entries, &path);
        assert!(path.exists(), "cache file must be written");

        let result = load_cache(current_mtime, &path);
        assert!(
            result.is_none(),
            "cache must be invalidated when mtime advances"
        );
    }
}
