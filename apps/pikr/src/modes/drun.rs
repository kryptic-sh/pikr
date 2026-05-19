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
// TODO: icon resolution via the `lnk` icon_location field is deferred to a
// follow-up issue (#38 phase 2). For now all entries have icon = None.

#[cfg(windows)]
mod windows_impl {
    use super::{Entry, Result};
    use std::path::{Path, PathBuf};
    use walkdir::WalkDir;

    pub fn collect() -> Result<Vec<Entry>> {
        let mut entries = Vec::new();
        for root in start_menu_roots() {
            if !root.exists() {
                continue;
            }
            for dir_entry in WalkDir::new(&root).follow_links(false) {
                let Ok(dir_entry) = dir_entry else {
                    continue;
                };
                let path = dir_entry.path();
                let is_lnk = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map_or(false, |e| e.eq_ignore_ascii_case("lnk"));
                if is_lnk {
                    if let Some(e) = parse_lnk(path) {
                        entries.push(e);
                    }
                }
            }
        }
        entries.sort_by_key(|e| e.label.to_lowercase());
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
    use super::windows_impl::start_menu_roots;

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
}
