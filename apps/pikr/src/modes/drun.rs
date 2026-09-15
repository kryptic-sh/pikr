//! drun mode — application launcher.
//!
//! - Unix: parses XDG `.desktop` files via `freedesktop-desktop-entry`.
//! - Windows: enumerates `shell:AppsFolder`, the list behind Start's "All
//!   apps" (see `drun_windows.rs`). Icons are extracted through the shell,
//!   rasterised as PNG, and cached under `%LOCALAPPDATA%\pikr\icon-cache\`.
//! - Other targets: returns an empty list.

use super::{Entry, Mode};
use anyhow::Result;

// Win32 icon-extraction helper — only compiled and linked on Windows.
#[cfg(windows)]
#[path = "drun_icons_windows.rs"]
mod icons_windows;

#[cfg(windows)]
#[path = "drun_windows.rs"]
mod windows_impl;

#[cfg(windows)]
pub use windows_impl::launch;

#[derive(Default)]
pub struct Drun;

impl Mode for Drun {
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
            let entry = Entry::exec_with(label, program, args, description, icon);
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

    /// Resolve the locale used for localized names, POSIX-style:
    /// `LC_ALL` > `LC_*` > `LANG` — `LC_ALL` overrides the per-category
    /// variables, so it is consulted first. `C` / `POSIX` (and unset vars)
    /// fall through to the next variable.
    ///
    /// Returns the spec fallback chain for the winning variable, longest
    /// first: `lang_COUNTRY@MODIFIER`, `lang@MODIFIER`, `lang_COUNTRY`,
    /// `lang` (the codeset is dropped — `.desktop` keys never carry one).
    /// `freedesktop_desktop_entry` matches each item exactly and, on a miss,
    /// falls back by stripping at `_` only — so a lone `sr_RS@latin` would
    /// jump straight to `sr` and skip a `Name[sr_RS]` key. The explicit chain
    /// restores the territory-only step.
    pub(crate) fn pick_locale(get: impl Fn(&str) -> Option<String>) -> Vec<String> {
        for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
            if let Some(v) = get(var) {
                let no_codeset = v.split('.').next().unwrap_or(&v);
                if no_codeset.is_empty() || no_codeset == "C" || no_codeset == "POSIX" {
                    continue;
                }
                let (lang_country, modifier) = match no_codeset.split_once('@') {
                    Some((lc, m)) => (lc, Some(m.to_string())),
                    None => (no_codeset, None),
                };
                let lang = lang_country.split('_').next().unwrap_or(lang_country);
                if lang.is_empty() {
                    continue;
                }
                let mut chain = Vec::with_capacity(4);
                if let Some(m) = &modifier {
                    chain.push(format!("{lang_country}@{m}"));
                    if lang != lang_country {
                        chain.push(format!("{lang}@{m}"));
                    }
                }
                if lang_country != lang {
                    chain.push(lang_country.to_string());
                }
                if !chain.iter().any(|c| c == lang) {
                    chain.push(lang.to_string());
                }
                return chain;
            }
        }
        Vec::new()
    }

    fn current_locales() -> Vec<String> {
        pick_locale(|v| std::env::var(v).ok())
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

    /// Remove freedesktop field-code substitutions (`%f` / `%F` / `%u` / `%U` /
    /// `%i` / `%c` / `%k`) from a token. A `%%` literal collapses to `%`.
    /// Unknown `%X` pairs and a trailing `%` are NOT field codes — the spec
    /// does not define them to be deleted, and `Exec=notify-send "50%"` or
    /// `yt-dlp -o "%(title)s.%(ext)s"` legitimately carry them.
    pub fn strip_field_codes(tok: &str) -> String {
        let mut out = String::with_capacity(tok.len());
        let mut chars = tok.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '%' {
                match chars.next() {
                    Some('%') => out.push('%'),
                    Some('f' | 'F' | 'u' | 'U' | 'i' | 'c' | 'k') => {} // field code — drop
                    Some(other) => {
                        // Unknown code / literal `%X` — keep both characters.
                        out.push('%');
                        out.push(other);
                    }
                    None => out.push('%'), // trailing `%` — keep
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
        /// Max mtime (nanoseconds since UNIX_EPOCH) across every dir and file
        /// under `dirs`.
        max_mtime_unix_nanos: u64,
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
            Entry::exec_with(c.label, c.program, c.args, c.description, c.icon)
        }
    }

    impl From<&Entry> for CachedEntry {
        fn from(e: &Entry) -> Self {
            use super::super::Payload;
            let (program, args) = match &e.payload {
                Payload::Exec { program, args } => (program.clone(), args.clone()),
                // Unreachable today — the unix collector only builds Exec
                // entries.
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
            .place_state_file("drun-cache.toml")
            .ok()
    }

    /// Compute the max mtime (nanoseconds since UNIX_EPOCH) across every dir
    /// and file under the existing applications dirs.  A dir's own mtime only
    /// changes when its *direct* children change, so a `.desktop` file added
    /// or removed inside an existing subdirectory would be invisible to a
    /// roots-only key; taking the max over the whole tree covers both new
    /// files anywhere and structural changes.  Returns `None` if no dir exists
    /// or no entry's mtime is readable.
    pub(crate) fn tree_mtime(dirs: &[PathBuf]) -> Option<u64> {
        let mut mtimes: Vec<u64> = Vec::new();
        for dir in dirs.iter().filter(|d| d.exists()) {
            collect_tree_mtimes(dir, &mut mtimes);
        }
        mtimes.into_iter().max()
    }

    /// Walk `dir` and push the mtime of every dir and file under it,
    /// including `dir` itself.  Symlinked dirs are not descended into
    /// (`DirEntry::file_type()` does not follow links), which keeps the walk
    /// cycle-free.
    fn collect_tree_mtimes(dir: &Path, out: &mut Vec<u64>) {
        if let Some(nanos) = mtime_nanos(dir) {
            out.push(nanos);
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(nanos) = mtime_nanos(&path) {
                out.push(nanos);
            }
            if entry.file_type().is_ok_and(|ft| ft.is_dir()) {
                collect_tree_mtimes(&path, out);
            }
        }
    }

    /// Nanoseconds-since-UNIX_EPOCH mtime of `path`, or `None` if unreadable
    /// or pre-epoch.
    fn mtime_nanos(path: &Path) -> Option<u64> {
        std::fs::metadata(path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as u64) // lossless: fits u64 until year 2554
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
            || cached.max_mtime_unix_nanos != current_mtime
        {
            tracing::debug!(
                dirs_match = cached.dirs == dirs,
                locales_match = cached.locales == locales,
                mtime_match = cached.max_mtime_unix_nanos == current_mtime,
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
            max_mtime_unix_nanos: mtime,
            entries: entries.iter().map(CachedEntry::from).collect(),
        };
        let write = || -> std::io::Result<()> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let text = toml::to_string_pretty(&cached).map_err(std::io::Error::other)?;
            crate::picker::write_private_state(path, &text)
        };
        if let Err(e) = write() {
            tracing::warn!("drun cache write failed: {e}");
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, unix))]
mod tests {
    use super::super::{Entry, Payload};
    use super::unix_impl::{
        insert_first, load_cache, parse_exec, pick_locale, strip_field_codes, tree_mtime,
        write_cache,
    };
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
    fn unknown_percent_pair_preserved() {
        // Regression for the audit finding: stripping ANY `%X` mangled
        // legitimate Exec= args. `50%` and `%(title)s.%(ext)s` are not
        // field codes and must survive byte-for-byte.
        assert_eq!(strip_field_codes("50%"), "50%");
        assert_eq!(strip_field_codes("%(title)s.%(ext)s"), "%(title)s.%(ext)s");
        assert_eq!(strip_field_codes("%z"), "%z");
        // Trailing `%` is not a field code either.
        assert_eq!(strip_field_codes("tail%"), "tail%");
    }

    #[test]
    fn spec_field_codes_all_stripped() {
        // The freedesktop spec's codes, each standalone and inline.
        for code in ["f", "F", "u", "U", "i", "c", "k"] {
            assert_eq!(
                strip_field_codes(&format!("%{code}")),
                "",
                "%{code} is a field code and must be dropped"
            );
        }
        assert_eq!(
            strip_field_codes("prog --url=%u %f --flag"),
            // %f is its own token and becomes empty (parse_exec drops empty
            // tokens); the surrounding spaces remain.
            "prog --url=  --flag"
        );
        assert_eq!(
            parse_exec("prog --url=%u %f --flag").map(|(p, a)| (p, a.join("|"))),
            Some(("prog".to_string(), "--url=|--flag".to_string()))
        );
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
            Entry::exec_with(
                "Firefox",
                "firefox",
                vec!["-new-window".to_string()],
                None,
                None,
            ),
            Entry::exec_with(
                "Alacritty",
                "alacritty",
                Vec::new(),
                Some("Terminal emulator".to_string()),
                Some("utilities-terminal".to_string()),
            ),
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
    fn cache_written_0600_like_state_files() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("drun-cache.toml");
        // Pre-create at 0644 (as an older pikr under a default umask would) —
        // the write must re-chmod to 0600, matching history/usage state files.
        std::fs::write(&path, b"stale").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        let dirs = vec![std::path::PathBuf::from("/usr/share/applications")];
        let locales = vec!["en_US".to_string()];
        let entries = vec![Entry::exec("TestApp", "testapp")];

        write_cache(&dirs, &locales, 1, &entries, &path);

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "drun cache must follow the 0600 state-file policy"
        );
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

    #[test]
    fn tree_mtime_detects_same_second_add() {
        // Regression for the seconds-truncation finding: two files whose
        // mtimes fall within the same second truncated to an equal key, so a
        // shortcut added in the same second as the cached max was invisible
        // until a later change. With nanosecond keys the advance must move
        // the tree key. mtimes are pinned explicitly via `set_modified` so
        // the test is deterministic regardless of the filesystem's clock
        // tick granularity.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("applications");
        std::fs::create_dir_all(&root).unwrap();
        let a = root.join("a.desktop");
        let b = root.join("b.desktop");
        std::fs::write(&a, b"old").unwrap();
        std::fs::write(&b, b"new").unwrap();

        let base = std::time::SystemTime::now();
        let set_mtime = |path: &std::path::Path, t: std::time::SystemTime| {
            std::fs::File::options()
                .write(true)
                .open(path)
                .unwrap()
                .set_modified(t)
                .unwrap();
        };
        // Both mtimes in the same second, 1 ms apart.
        set_mtime(&a, base);
        set_mtime(&b, base + std::time::Duration::from_millis(1));

        let t1 = tree_mtime(std::slice::from_ref(&root)).expect("tree mtime must be readable");
        // Advance `a` by another 1 ms — still within the same second.
        set_mtime(&a, base + std::time::Duration::from_millis(2));
        let t2 = tree_mtime(std::slice::from_ref(&root)).expect("tree mtime must be readable");
        assert!(
            t2 > t1,
            "a same-second mtime advance must move the tree key (t1={t1}, t2={t2})"
        );
    }

    #[test]
    fn locale_precedence_puts_lc_all_first() {
        // POSIX: LC_ALL > LC_* > LANG. The old order consulted LC_MESSAGES
        // first, so a set LC_ALL was ignored whenever LC_MESSAGES was set.
        let lookup = |v: &str| match v {
            "LC_ALL" => Some("de_DE.UTF-8".to_string()),
            "LC_MESSAGES" => Some("en_US.UTF-8".to_string()),
            "LANG" => Some("fr_FR.UTF-8".to_string()),
            _ => None,
        };
        assert_eq!(
            pick_locale(lookup),
            vec!["de_DE".to_string(), "de".to_string()]
        );
    }

    #[test]
    fn locale_c_or_posix_falls_through_to_next_variable() {
        // C / POSIX are explicit "no localization" values — they must fall
        // through to the next variable, not win.
        let c = |v: &str| match v {
            "LC_ALL" => Some("C".to_string()),
            "LC_MESSAGES" => Some("en_US.UTF-8".to_string()),
            _ => None,
        };
        assert_eq!(pick_locale(c), vec!["en_US".to_string(), "en".to_string()]);
        let posix = |v: &str| match v {
            "LC_ALL" => Some("POSIX".to_string()),
            "LC_MESSAGES" => Some("en_US.UTF-8".to_string()),
            _ => None,
        };
        assert_eq!(
            pick_locale(posix),
            vec!["en_US".to_string(), "en".to_string()]
        );
    }

    #[test]
    fn locale_modifier_expands_to_full_fallback_chain() {
        // `sr_RS@latin.UTF-8` must produce the whole spec chain — without
        // the lang_COUNTRY step, freedesktop_desktop_entry's own `_`-stripping
        // fallback would jump from sr_RS@latin straight to sr and miss a
        // `Name[sr_RS]` key.
        let lookup = |v: &str| match v {
            "LANG" => Some("sr_RS@latin.UTF-8".to_string()),
            _ => None,
        };
        assert_eq!(
            pick_locale(lookup),
            vec![
                "sr_RS@latin".to_string(),
                "sr@latin".to_string(),
                "sr_RS".to_string(),
                "sr".to_string(),
            ]
        );
        // Language-only with a modifier: sr@latin → sr@latin, sr.
        let lang_only = |v: &str| match v {
            "LANG" => Some("sr@latin".to_string()),
            _ => None,
        };
        assert_eq!(
            pick_locale(lang_only),
            vec!["sr@latin".to_string(), "sr".to_string()]
        );
    }
}
