//! run mode — `$PATH` executable launcher.
//!
//! On Unix, executability is determined by the executable bit on any of
//! user/group/other. On Windows, a file is considered executable when its
//! extension appears in `%PATHEXT%` (defaulting to `.COM;.EXE;.BAT;.CMD`
//! when the variable is absent). On other targets the filter always returns
//! false (no entries collected).
//!
//! The label stored in each `Entry` is the **stem** on Windows (e.g.
//! `firefox` rather than `firefox.exe`) so the user can type without the
//! extension. `Command::new` on Windows performs its own `PATHEXT` lookup
//! when the argument has no extension, so dropping the extension is safe.

use super::{Entry, Mode};
use anyhow::Result;
use std::collections::BTreeSet;
use std::path::Path;

#[derive(Default)]
pub struct Run;

impl Mode for Run {
    fn name(&self) -> &'static str {
        "run"
    }

    fn collect(&mut self) -> Result<Vec<Entry>> {
        let names = scan_path();
        Ok(names
            .into_iter()
            .map(|n| Entry::exec(n.clone(), n))
            .collect())
    }
}

// ── per-OS executable predicate ───────────────────────────────────────────────

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(windows)]
fn is_executable(path: &Path) -> bool {
    // PATHEXT-aware. Default to the canonical Windows extension set when the
    // env var is missing (some shells strip it).
    let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => pathext
            .split(';')
            .map(|p| p.trim_start_matches('.'))
            .any(|p| p.eq_ignore_ascii_case(ext)),
        None => false,
    }
}

#[cfg(not(any(unix, windows)))]
fn is_executable(_path: &Path) -> bool {
    false
}

// ── label helper ──────────────────────────────────────────────────────────────

/// Return the string that will be shown in the picker and passed to
/// `Command::new`. On Windows the extension is stripped so the user can type
/// `firefox` instead of `firefox.exe`; `Command::new` handles PATHEXT lookup
/// automatically when no extension is present. On Unix the full filename is
/// used unchanged.
fn entry_label(path: &Path) -> Option<String> {
    if cfg!(windows) {
        path.file_stem()
            .and_then(|s| s.to_str())
            .map(str::to_string)
    } else {
        path.file_name()
            .and_then(|s| s.to_str())
            .map(str::to_string)
    }
}

// ── PATH scanning ─────────────────────────────────────────────────────────────

fn scan_path() -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let Some(path) = std::env::var_os("PATH") else {
        return names;
    };
    for dir in std::env::split_paths(&path) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        scan_dir(&dir, &mut names);
    }
    names
}

fn scan_dir(dir: &Path, names: &mut BTreeSet<String>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in read.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let entry_path = entry.path();
        if !is_executable(&entry_path) {
            continue;
        }
        if let Some(label) = entry_label(&entry_path) {
            names.insert(label);
        }
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn is_executable_unix_bit() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let exec_path = dir.path().join("my_exec");
        let non_exec_path = dir.path().join("not_exec");

        std::fs::write(&exec_path, b"#!/bin/sh").unwrap();
        std::fs::write(&non_exec_path, b"data").unwrap();

        // chmod +x the first file only
        let mut perms = std::fs::metadata(&exec_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&exec_path, perms).unwrap();

        let mut perms2 = std::fs::metadata(&non_exec_path).unwrap().permissions();
        perms2.set_mode(0o644);
        std::fs::set_permissions(&non_exec_path, perms2).unwrap();

        assert!(
            is_executable(&exec_path),
            "chmod +x file must be considered executable"
        );
        assert!(
            !is_executable(&non_exec_path),
            "mode 0o644 file must not be considered executable"
        );
    }

    #[test]
    #[cfg(windows)]
    fn is_executable_windows_pathext() {
        // Override PATHEXT for this test so it is deterministic.
        std::env::set_var("PATHEXT", ".COM;.EXE;.BAT;.CMD");

        let exe_path = Path::new("some_tool.exe");
        let txt_path = Path::new("readme.txt");

        assert!(
            is_executable(exe_path),
            ".exe must match PATHEXT .COM;.EXE;.BAT;.CMD"
        );
        assert!(
            !is_executable(txt_path),
            ".txt must not match PATHEXT .COM;.EXE;.BAT;.CMD"
        );
    }

    #[test]
    #[cfg(windows)]
    fn pathext_case_insensitive() {
        std::env::set_var("PATHEXT", ".COM;.EXE;.BAT;.CMD");

        // Extension in upper-case must still match.
        let upper = Path::new("tool.EXE");
        let lower = Path::new("tool.exe");

        assert!(is_executable(upper), ".EXE (upper) must match");
        assert!(is_executable(lower), ".exe (lower) must match");
    }

    #[test]
    #[cfg(windows)]
    fn pathext_default_when_unset() {
        // Remove PATHEXT entirely so the default is used.
        std::env::remove_var("PATHEXT");

        assert!(
            is_executable(Path::new("thing.exe")),
            ".exe must match default PATHEXT"
        );
        assert!(
            is_executable(Path::new("thing.cmd")),
            ".cmd must match default PATHEXT"
        );
        assert!(
            is_executable(Path::new("thing.bat")),
            ".bat must match default PATHEXT"
        );
        assert!(
            !is_executable(Path::new("thing.dll")),
            ".dll must not match default PATHEXT"
        );
    }

    #[test]
    fn entry_label_unix_keeps_extension() {
        // entry_label on non-Windows returns the full filename.
        #[cfg(not(windows))]
        {
            let p = Path::new("/usr/bin/git");
            assert_eq!(entry_label(p).as_deref(), Some("git"));
            let p2 = Path::new("/usr/bin/python3.11");
            assert_eq!(entry_label(p2).as_deref(), Some("python3.11"));
        }
    }

    #[test]
    #[cfg(windows)]
    fn entry_label_windows_strips_extension() {
        let p = Path::new(r"C:\Windows\System32\notepad.exe");
        assert_eq!(
            entry_label(p).as_deref(),
            Some("notepad"),
            "Windows label must be the stem without .exe"
        );
    }
}
