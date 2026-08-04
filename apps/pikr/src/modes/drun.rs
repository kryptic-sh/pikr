//! drun mode — application launcher.
//!
//! - Unix: parses XDG `.desktop` files via `freedesktop-desktop-entry`.
//! - Windows: walks Start Menu `.lnk` shortcuts from both the per-user and
//!   all-users trees.  Icons are extracted via Win32 `SHGetFileInfoW` /
//!   `GetDIBits`, rasterised as PNG, and cached under
//!   `%LOCALAPPDATA%\pikr\icon-cache\`.
//! - Other targets: returns an empty list.

use super::{Entry, Mode};
use anyhow::Result;

// Win32 icon-extraction helper — only compiled and linked on Windows.
#[cfg(windows)]
#[path = "drun_icons_windows.rs"]
mod icons_windows;

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
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::time::SystemTime;

    pub fn collect() -> Result<Vec<Entry>> {
        let locales = current_locales();
        // Cache key inputs: the applications dirs that exist right now, in
        // `default_paths()` order (user-local first), plus the max mtime
        // across the whole tree.  A warm start with an unchanged tree skips
        // the parse entirely.
        let dirs: Vec<PathBuf> = default_paths().filter(|p| p.exists()).collect();
        let current_mtime = tree_mtime(&dirs);

        // --- Cache probe ---
        if let (Some(mtime), Some(path)) = (current_mtime, cache_path())
            && let Some(entries) = load_cache(&dirs, &locales, mtime, &path)
        {
            return Ok(entries);
        }

        // Map app-id → entry so the FIRST `.desktop` file (user-local,
        // iterated before system dirs) overrides later ones (system) per the
        // freedesktop search-order convention.
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
            insert_first(&mut by_id, id, entry);
        }

        let mut entries: Vec<Entry> = by_id.into_values().collect();
        entries.sort_by_key(|a| a.label.to_lowercase());

        // --- Cache write (best-effort) ---
        if let (Some(mtime), Some(path)) = (current_mtime, cache_path()) {
            write_cache(&dirs, &locales, mtime, &entries, &path);
        }

        Ok(entries)
    }

    /// Merge one parsed entry into the id → entry map so the FIRST occurrence
    /// per app id wins (user-local `.desktop` files are iterated first; a later
    /// system copy must not overwrite the override).
    pub fn insert_first(by_id: &mut HashMap<String, Entry>, id: String, entry: Entry) {
        by_id.entry(id).or_insert(entry);
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

    // ── Cache types ───────────────────────────────────────────────────────────

    /// On-disk cache for drun entries.
    ///
    /// Keyed by the applications dirs that existed at cache time (in
    /// `default_paths()` order), the locale list, and the max mtime across
    /// every dir and file under those dirs.  If any of the three changed since
    /// the cache was written the whole cache is stale and the parse re-runs.
    #[derive(Serialize, Deserialize)]
    struct CachedDrun {
        /// The applications dirs that existed at cache time, in
        /// `default_paths()` order.  Order matters: user-local dirs iterate
        /// first and their entries win the dedupe, so a reordered dirs list
        /// must invalidate.
        dirs: Vec<PathBuf>,
        /// current_locales() at cache time — localized names are baked into
        /// the entries.
        locales: Vec<String>,
        /// Max mtime (Unix seconds) across every dir and file under `dirs`.
        max_mtime_unix_secs: u64,
        /// Cached entries — labels, descriptions, icons, program + args.
        entries: Vec<CachedEntry>,
    }

    /// A single cached entry.  Mirrors the fields the parse loop populates:
    /// the unix collector only ever produces `Payload::Exec` entries.
    #[derive(Serialize, Deserialize)]
    struct CachedEntry {
        label: String,
        description: Option<String>,
        icon: Option<String>,
        program: String,
        args: Vec<String>,
    }

    impl From<CachedEntry> for Entry {
        fn from(c: CachedEntry) -> Self {
            let mut e = Entry::exec(c.label, c.program).with_args(c.args);
            if let Some(d) = c.description {
                e = e.with_description(d);
            }
            if let Some(i) = c.icon {
                e = e.with_icon(i);
            }
            e
        }
    }

    impl From<&Entry> for CachedEntry {
        fn from(e: &Entry) -> Self {
            use super::super::Payload;
            let (program, args) = match &e.payload {
                Payload::Exec { program, args } => (program.clone(), args.clone()),
                // Unreachable today — the unix collector only builds Exec
                // entries.  Mirrors the windows `_ =>` arm.
                _ => (String::new(), Vec::new()),
            };
            CachedEntry {
                label: e.label.clone(),
                description: e.description.clone(),
                icon: e.icon.clone(),
                program,
                args,
            }
        }
    }

    // ── Cache helpers ─────────────────────────────────────────────────────────

    /// Path to the drun cache file: `$XDG_STATE_HOME/pikr/drun-cache.toml`
    /// (falls back to `~/.local/state/pikr/` via `xdg`).  Creating the state
    /// dir here is a harmless side effect — `place_state_file` creates it if
    /// missing, and it is where the cache write lands on a miss anyway.
    fn cache_path() -> Option<PathBuf> {
        xdg::BaseDirectories::with_prefix("pikr")
            .ok()?
            .place_state_file("drun-cache.toml")
            .ok()
    }

    /// Compute the max mtime (Unix seconds) across every dir and file under
    /// the existing applications dirs.  A dir's own mtime only changes when
    /// its *direct* children change, so a `.desktop` file added or removed
    /// inside an existing subdirectory would be invisible to a roots-only
    /// key; taking the max over the whole tree covers both new files anywhere
    /// and structural changes.  Returns `None` if no dir exists or no entry's
    /// mtime is readable.
    fn tree_mtime(dirs: &[PathBuf]) -> Option<u64> {
        let mut mtimes: Vec<u64> = Vec::new();
        for dir in dirs.iter().filter(|d| d.exists()) {
            collect_tree_mtimes(dir, &mut mtimes);
        }
        mtimes.into_iter().max()
    }

    /// Walk `dir` and push the mtime of every dir and file under it,
    /// including `dir` itself.  Symlinked dirs are not descended into
    /// (`DirEntry::file_type()` does not follow links), which keeps the walk
    /// cycle-free and mirrors `WalkDir::follow_links(false)`.  Uses `std::fs`
    /// recursion because `walkdir` is a windows-only dependency in this crate.
    fn collect_tree_mtimes(dir: &Path, out: &mut Vec<u64>) {
        if let Some(secs) = mtime_secs(dir) {
            out.push(secs);
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(secs) = mtime_secs(&path) {
                out.push(secs);
            }
            if entry.file_type().is_ok_and(|ft| ft.is_dir()) {
                collect_tree_mtimes(&path, out);
            }
        }
    }

    /// Unix-seconds mtime of `path`, or `None` if unreadable or pre-epoch.
    fn mtime_secs(path: &Path) -> Option<u64> {
        std::fs::metadata(path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
    }

    /// Try to load a valid cache. Returns `Some(entries)` on hit, `None` on
    /// miss (missing file, parse error, or any key component changed — dirs,
    /// locales, or tree mtime).
    pub fn load_cache(
        dirs: &[PathBuf],
        locales: &[String],
        current_mtime: u64,
        path: &Path,
    ) -> Option<Vec<Entry>> {
        let _span = tracing::debug_span!("drun_cache_load", path = %path.display()).entered();
        let bytes = std::fs::read(path).ok()?;
        let text = std::str::from_utf8(&bytes).ok()?;
        let cached: CachedDrun = toml::from_str(text)
            .map_err(|e| tracing::warn!("drun cache parse error: {e}"))
            .ok()?;
        if cached.dirs != dirs
            || cached.locales != locales
            || cached.max_mtime_unix_secs != current_mtime
        {
            tracing::debug!(
                dirs_match = cached.dirs == dirs,
                locales_match = cached.locales == locales,
                mtime_match = cached.max_mtime_unix_secs == current_mtime,
                "drun cache key mismatch — invalidating"
            );
            return None;
        }
        tracing::debug!(entries = cached.entries.len(), "drun cache hit");
        Some(cached.entries.into_iter().map(Entry::from).collect())
    }

    /// Write a fresh cache to disk. Best-effort — logs a warning on any error.
    pub fn write_cache(
        dirs: &[PathBuf],
        locales: &[String],
        mtime: u64,
        entries: &[Entry],
        path: &Path,
    ) {
        let _span = tracing::debug_span!("drun_cache_write", path = %path.display()).entered();
        let cached = CachedDrun {
            dirs: dirs.to_vec(),
            locales: locales.to_vec(),
            max_mtime_unix_secs: mtime,
            entries: entries.iter().map(CachedEntry::from).collect(),
        };
        let write = || -> std::io::Result<()> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let text = toml::to_string_pretty(&cached).map_err(std::io::Error::other)?;
            std::fs::write(path, text)
        };
        if let Err(e) = write() {
            tracing::warn!("drun cache write failed: {e}");
        }
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
//   %LOCALAPPDATA%\pikr\drun-cache.json keyed by the max mtime across every
//   dir and file under both Start Menu roots. On the next run, if the mtime
//   hasn't changed the walk is skipped entirely and the cached entries are
//   returned directly. Keying on the whole tree rather than just the root
//   dirs means a shortcut added or removed anywhere — including inside an
//   existing subfolder, whose change never touches the root's own mtime —
//   invalidates the cache.
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
    /// Keyed by the max mtime (in Unix seconds) across every dir and file
    /// under both Start Menu roots.  If any entry's mtime has advanced since
    /// the cache was written the whole cache is considered stale and the walk
    /// is re-run.
    #[derive(Serialize, Deserialize)]
    struct CachedDrun {
        /// Max mtime across the whole Start Menu tree — every dir and file —
        /// at cache time.  Invalidate when any tree entry is newer than this
        /// value.  (Field name kept from the roots-only key for on-disk
        /// compatibility with existing cache files.)
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
        /// Absolute path to the cached PNG icon, or `None` if extraction
        /// failed or hasn't been attempted yet (old cache files).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        icon_path: Option<String>,
    }

    impl From<CachedEntry> for Entry {
        fn from(c: CachedEntry) -> Self {
            let mut e = Entry::exec(c.label, c.target).with_args(c.args);
            if let Some(d) = c.description {
                e = e.with_description(d);
            }
            if let Some(icon) = c.icon_path {
                e = e.with_icon(icon);
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
                icon_path: e.icon.clone(),
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

    /// Compute the max mtime (Unix seconds) across every dir and file under
    /// the existing Start Menu roots.  A root's own mtime only changes when
    /// its *direct* children change, so a shortcut added inside an existing
    /// subfolder would be invisible to a roots-only key; taking the max over
    /// the whole tree covers both new `.lnk` files anywhere and structural
    /// changes.  Returns `None` if no root exists or no entry's mtime is
    /// readable.
    pub fn tree_mtime(roots: &[PathBuf]) -> Option<u64> {
        roots
            .iter()
            .filter(|r| r.exists())
            .flat_map(|root| {
                WalkDir::new(root)
                    .follow_links(false)
                    .into_iter()
                    .filter_map(Result::ok)
            })
            .filter_map(|e| {
                e.metadata()
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
            let json = serde_json::to_vec_pretty(&cached).map_err(std::io::Error::other)?;
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
        let current_mtime = tree_mtime(&roots);

        // --- Cache probe ---
        {
            let _span = tracing::debug_span!("drun_cache_probe").entered();
            if let (Some(mtime), Some(path)) = (current_mtime, cache_path())
                && let Some(entries) = load_cache(mtime, &path)
            {
                return Ok(entries);
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
                        .is_some_and(|e| e.eq_ignore_ascii_case("lnk"))
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
        if let Some(ext) = target_path.extension().and_then(|e| e.to_str())
            && skip_exts.iter().any(|&skip| ext.eq_ignore_ascii_case(skip))
        {
            return None;
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
            .and_then(shlex::split)
            .unwrap_or_default();

        let mut entry = Entry::exec(label, target.clone()).with_args(args);

        // Parent folder name as description if non-empty and not the root
        // "Programs" folder itself.
        if !parent_name.is_empty() && !parent_name.eq_ignore_ascii_case("Programs") {
            entry = entry.with_description(parent_name);
        }

        // Resolve and cache the icon for the target executable.  The first
        // call per target does a Win32 SHGetFileInfoW + GetDIBits round-trip
        // and writes a PNG to %LOCALAPPDATA%\pikr\icon-cache\; subsequent
        // calls return the cached path directly.  Failure is non-fatal —
        // the entry is still shown without an icon.
        if let Some(icon_path) = super::icons_windows::icon_for(target_path) {
            entry = entry.with_icon(icon_path.to_string_lossy().into_owned());
        }

        Some(entry)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, unix))]
mod tests {
    use super::super::{Entry, Payload};
    use super::unix_impl::{insert_first, load_cache, parse_exec, strip_field_codes, write_cache};
    use std::collections::HashMap;

    #[test]
    fn user_local_entry_wins_over_system_copy() {
        let mut by_id = HashMap::new();
        insert_first(
            &mut by_id,
            "firefox".to_string(),
            Entry::exec("User Firefox", "firefox"),
        );
        insert_first(
            &mut by_id,
            "firefox".to_string(),
            Entry::exec("System Firefox", "firefox"),
        );
        assert_eq!(by_id.len(), 1);
        assert_eq!(by_id["firefox"].label, "User Firefox");
    }

    #[test]
    fn distinct_ids_both_kept() {
        let mut by_id = HashMap::new();
        insert_first(
            &mut by_id,
            "firefox".to_string(),
            Entry::exec("Firefox", "firefox"),
        );
        insert_first(
            &mut by_id,
            "alacritty".to_string(),
            Entry::exec("Alacritty", "alacritty"),
        );
        assert_eq!(by_id.len(), 2);
        assert_eq!(by_id["firefox"].label, "Firefox");
        assert_eq!(by_id["alacritty"].label, "Alacritty");
    }

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

    // ── Cache helpers ─────────────────────────────────────────────────────────

    #[test]
    fn write_then_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drun-cache.toml");

        let dirs = vec![std::path::PathBuf::from("/usr/share/applications")];
        let locales = vec!["en_US".to_string()];
        let mtime: u64 = 1_700_000_000;
        let entries = vec![
            Entry::exec("Firefox", "firefox").with_args(vec!["-new-window".to_string()]),
            Entry::exec("Alacritty", "alacritty")
                .with_description("Terminal emulator".to_string())
                .with_icon("utilities-terminal".to_string()),
        ];

        write_cache(&dirs, &locales, mtime, &entries, &path);
        assert!(path.exists(), "cache file must be written");

        let loaded = load_cache(&dirs, &locales, mtime, &path).expect("cache must hit on same key");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].label, "Firefox");
        assert_eq!(loaded[0].description, None);
        assert_eq!(loaded[0].icon, None);
        match &loaded[0].payload {
            Payload::Exec { program, args } => {
                assert_eq!(program, "firefox");
                assert_eq!(*args, vec!["-new-window".to_string()]);
            }
            _ => panic!("expected Exec payload"),
        }
        assert_eq!(loaded[1].label, "Alacritty");
        assert_eq!(loaded[1].description.as_deref(), Some("Terminal emulator"));
        assert_eq!(loaded[1].icon.as_deref(), Some("utilities-terminal"));
        match &loaded[1].payload {
            Payload::Exec { program, args } => {
                assert_eq!(program, "alacritty");
                assert!(args.is_empty());
            }
            _ => panic!("expected Exec payload"),
        }
    }

    #[test]
    fn load_cache_invalidates_on_mtime_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drun-cache.toml");

        let dirs = vec![std::path::PathBuf::from("/usr/share/applications")];
        let locales = vec!["en_US".to_string()];
        let entries = vec![Entry::exec("TestApp", "testapp")];

        write_cache(&dirs, &locales, 1, &entries, &path);
        assert!(path.exists(), "cache file must be written");

        let result = load_cache(&dirs, &locales, 2, &path);
        assert!(
            result.is_none(),
            "cache must be invalidated when mtime advances"
        );
    }

    #[test]
    fn load_cache_invalidates_on_dirs_or_locales_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drun-cache.toml");

        let dirs = vec![std::path::PathBuf::from("/usr/share/applications")];
        let locales = vec!["en_US".to_string()];
        let mtime: u64 = 42;
        let entries = vec![Entry::exec("TestApp", "testapp")];

        write_cache(&dirs, &locales, mtime, &entries, &path);

        // Same mtime and locales, different dirs → miss.
        let other_dirs = vec![std::path::PathBuf::from("/usr/local/share/applications")];
        assert!(
            load_cache(&other_dirs, &locales, mtime, &path).is_none(),
            "changed dirs must invalidate the cache"
        );

        // Same mtime and dirs, different locales → miss.
        let other_locales = vec!["de_DE".to_string()];
        assert!(
            load_cache(&dirs, &other_locales, mtime, &path).is_none(),
            "changed locales must invalidate the cache"
        );
    }

    #[test]
    fn load_cache_missing_or_corrupt_file_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.json");
        assert!(
            load_cache(&[], &[], 1, &missing).is_none(),
            "missing cache file must be a miss"
        );

        let corrupt = dir.path().join("corrupt.json");
        std::fs::write(&corrupt, b"this is not toml").unwrap();
        assert!(
            load_cache(&[], &[], 1, &corrupt).is_none(),
            "corrupt cache file must be a miss"
        );
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::windows_impl::{load_cache, start_menu_roots, tree_mtime, write_cache};

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

    /// The cache key must advance when a shortcut is added inside an EXISTING
    /// subfolder.  A root's own mtime only changes when its direct children
    /// change, so the old roots-only key would stay put here — this is the
    /// add-in-subfolder staleness finding.
    #[test]
    fn tree_mtime_advances_on_add_inside_existing_subfolder() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Programs");
        let vendor = root.join("Vendor");
        std::fs::create_dir_all(&vendor).unwrap();

        let root_mtime = |p: &std::path::Path| {
            p.metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
        };

        std::fs::write(vendor.join("App.lnk"), b"shortcut bytes").unwrap();
        let root_mtime_before = root_mtime(&root);
        let t1 = tree_mtime(&[root.clone()]).expect("tree mtime must be readable");

        // Coarse mtime granularity (FAT, 1s) can leave two writes within the
        // same second with equal mtimes, so wait past a full second before
        // adding the second file to make t2 > t1 robust.
        std::thread::sleep(std::time::Duration::from_millis(1100));

        // New shortcut inside the existing `Vendor` folder — the exact repro
        // from the finding.  `root`'s own mtime must NOT change here.
        std::fs::write(vendor.join("App2.lnk"), b"shortcut bytes").unwrap();

        assert_eq!(
            root_mtime_before,
            root_mtime(&root),
            "the root dir's mtime must be unchanged — the roots-only key would miss this add"
        );

        let t2 = tree_mtime(&[root]).expect("tree mtime must be readable");
        assert!(
            t2 > t1,
            "adding a shortcut inside an existing subfolder must advance the tree mtime (t1={t1}, t2={t2})"
        );
    }
}
