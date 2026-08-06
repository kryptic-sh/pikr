//! ssh mode — SSH host picker from `~/.ssh/config` (and `/etc/ssh/ssh_config`
//! on Unix).

use super::{Entry, Mode, Payload};
use anyhow::Result;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct Ssh;

impl Mode for Ssh {
    fn collect(&mut self) -> Result<Vec<Entry>> {
        let terminal = resolve_terminal();
        let mut entries: Vec<Entry> = Vec::new();

        for path in config_paths() {
            if let Ok(text) = std::fs::read_to_string(&path) {
                entries.extend(parse_config(&text, &terminal));
            }
        }

        entries.sort_by_key(|a| a.label.to_lowercase());
        // Deduplicate: user config (~/.ssh/config) is parsed first; if the same
        // host appears in both user and system config, keep the first occurrence.
        entries.dedup_by(|a, b| a.label.eq_ignore_ascii_case(&b.label));
        Ok(entries)
    }
}

fn config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(home) = home_dir() {
        paths.push(home.join(".ssh").join("config"));
    }
    // System-wide ssh config only exists on Unix.
    #[cfg(unix)]
    paths.push(PathBuf::from("/etc/ssh/ssh_config"));
    paths
}

/// Portable home-directory lookup.
/// - Unix: `$HOME`, falling back to `dirs::home_dir()`.
/// - Windows: `%USERPROFILE%`, falling back to `dirs::home_dir()`.
fn home_dir() -> Option<PathBuf> {
    #[cfg(unix)]
    {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .or_else(dirs::home_dir)
    }
    #[cfg(not(unix))]
    {
        std::env::var_os("USERPROFILE")
            .map(PathBuf::from)
            .or_else(dirs::home_dir)
    }
}

/// Walk the ssh config text, returning one Entry per non-wildcard Host.
pub fn parse_config(text: &str, terminal: &str) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut current_hosts: Vec<String> = Vec::new();
    let mut hostname: Option<String> = None;
    let mut user: Option<String> = None;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let (key, value) = match trimmed.split_once(|c: char| c.is_whitespace()) {
            Some((k, v)) => (k.to_ascii_lowercase(), v.trim().to_string()),
            None => continue,
        };

        if key == "host" {
            // Flush the previous block before starting a new one.
            flush_hosts(
                &mut entries,
                &mut current_hosts,
                &mut hostname,
                &mut user,
                terminal,
            );
            // ssh_config(5) allows `Host a b` — one entry per pattern.
            // Wildcard patterns are not real hosts — skip them.
            current_hosts = value
                .split_whitespace()
                .filter(|p| !p.contains('*') && !p.contains('?'))
                .map(str::to_string)
                .collect();
            if current_hosts.is_empty() {
                // A wildcard-only block contributes no entries — its
                // HostName/User must not leak into the next block.
                hostname = None;
                user = None;
            }
        } else if !current_hosts.is_empty() {
            match key.as_str() {
                "hostname" => hostname = Some(value),
                "user" => user = Some(value),
                _ => {}
            }
        }
    }
    // Flush the final block.
    flush_hosts(
        &mut entries,
        &mut current_hosts,
        &mut hostname,
        &mut user,
        terminal,
    );
    entries
}

/// Emit one entry per accumulated host, sharing the block's HostName/User,
/// then clear the block state. No-op when the block produced no hosts.
fn flush_hosts(
    entries: &mut Vec<Entry>,
    hosts: &mut Vec<String>,
    hostname: &mut Option<String>,
    user: &mut Option<String>,
    terminal: &str,
) {
    if hosts.is_empty() {
        return;
    }
    let hosts = std::mem::take(hosts);
    let hostname = hostname.take();
    let user = user.take();
    for host in hosts {
        entries.push(make_entry(host, hostname.clone(), user.clone(), terminal));
    }
}

fn make_entry(
    host: String,
    hostname: Option<String>,
    user: Option<String>,
    terminal: &str,
) -> Entry {
    let description = match (hostname.as_deref(), user.as_deref()) {
        (Some(h), Some(u)) => Some(format!("{h} ({u})")),
        (Some(h), None) => Some(h.to_string()),
        (None, Some(u)) => Some(u.to_string()),
        (None, None) => None,
    };
    let args = build_terminal_args(terminal, &host);
    Entry {
        label: host,
        description,
        icon: None,
        payload: Payload::Exec {
            program: terminal.to_string(),
            args,
        },
    }
}

/// Build the argv for the chosen terminal to run `ssh <host>`.
///
/// Each terminal has its own calling convention:
/// - Unix terminals (`alacritty`, `kitty`, `foot`, `xterm`): `-e ssh <host>`
/// - `wt.exe` (Windows Terminal): `-- ssh <host>`
///   Windows Terminal forwards everything after `--` to its shell/command.
/// - `pwsh.exe` / `powershell.exe`: `-NoExit -Command ssh '<host>'`
///   `-NoExit` keeps the window open after ssh exits so the user can see
///   error messages before the pane closes. The host is single-quoted —
///   pwsh `-Command` parses a script, and single quotes are literal there.
/// - `cmd.exe`: `/K ssh "<host>"` for a safe host, else the `-e` argv form.
///   `/K` runs the command and keeps the prompt alive afterwards; cmd has
///   no reliable quoting, so a host outside the safe charset never reaches
///   its script string.
fn build_terminal_args(terminal: &str, host: &str) -> Vec<String> {
    let binary = std::path::Path::new(terminal)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(terminal)
        .to_ascii_lowercase();

    match binary.as_str() {
        "wt.exe" | "wt" => vec!["--".to_string(), "ssh".to_string(), host.to_string()],
        "pwsh.exe" | "pwsh" | "powershell.exe" | "powershell" => vec![
            "-NoExit".to_string(),
            "-Command".to_string(),
            // pwsh -Command parses a script; single quotes are literal in pwsh
            // and an embedded quote is escaped by doubling, so the host can
            // never inject into the script.
            format!("ssh '{}'", host.replace('\'', "''")),
        ],
        "cmd.exe" | "cmd" => {
            if is_safe_host(host) {
                vec!["/K".to_string(), format!("ssh \"{host}\"")]
            } else {
                // cmd has no reliable quoting — fall back to the argv form so a
                // pathological host errors visibly in the terminal instead of
                // executing its metacharacters.
                vec!["-e".to_string(), "ssh".to_string(), host.to_string()]
            }
        }
        // Unix terminals (alacritty, kitty, foot, xterm) and anything unknown.
        _ => vec!["-e".to_string(), "ssh".to_string(), host.to_string()],
    }
}

/// Hosts are interpolated into a shell string for pwsh/cmd, so reject
/// anything outside the charset legit hostnames / IPv6 literals use
/// (alphanumerics plus `. - _ : [ ]`). cmd has no reliable quoting, so
/// an unsafe host must never reach its string arm.
fn is_safe_host(host: &str) -> bool {
    host.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '[' | ']'))
}

/// Ordered list of terminal-emulator candidates for the current OS.
#[cfg(unix)]
fn terminal_candidates() -> &'static [&'static str] {
    &["alacritty", "kitty", "foot", "xterm"]
}

#[cfg(windows)]
fn terminal_candidates() -> &'static [&'static str] {
    &["wt.exe", "pwsh.exe", "powershell.exe", "cmd.exe"]
}

// Fallback for non-unix, non-windows targets (e.g. cross-compilation checks).
#[cfg(not(any(unix, windows)))]
fn terminal_candidates() -> &'static [&'static str] {
    &["xterm"]
}

/// Find a usable terminal emulator: $TERMINAL env first, then well-known names.
fn resolve_terminal() -> String {
    if let Ok(t) = std::env::var("TERMINAL")
        && !t.is_empty()
        && is_tool_installed(&t)
    {
        return t;
    }
    for &name in terminal_candidates() {
        if is_tool_installed(name) {
            return name.to_string();
        }
    }
    // Last resort: first candidate even if not found; exec will fail with a
    // clear error rather than silently doing nothing.
    terminal_candidates()
        .first()
        .copied()
        .unwrap_or("xterm")
        .to_string()
}

/// Probe whether a tool exists by walking `$PATH` and stat'ing the
/// binary — cheaper than spawning `--version` per candidate on the
/// launch path (a fork/exec/wait per miss).
fn is_tool_installed(name: &str) -> bool {
    if name.contains('/') || name.contains('\\') {
        // Path-like `$TERMINAL` value ("/usr/bin/alacritty"): probe the
        // path directly, not via PATH joins.
        return std::fs::metadata(name)
            .map(|m| m.is_file() && is_executable(Path::new(name), &m))
            .unwrap_or(false);
    }
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    probe_in_path(&path, name)
}

/// Pure probe over an explicit PATH string — testable without env mutation.
fn probe_in_path(path_var: &std::ffi::OsStr, name: &str) -> bool {
    std::env::split_paths(path_var).any(|dir| {
        candidate_names(name).iter().any(|candidate| {
            let path = dir.join(candidate);
            std::fs::metadata(&path)
                .map(|m| m.is_file() && is_executable(&path, &m))
                .unwrap_or(false)
        })
    })
}

/// Names to try for a probe: the name as given, plus (Windows) the name
/// with each PATHEXT extension appended — mirroring what `Command::new`
/// resolves at spawn time, so `$TERMINAL=pwsh` finds `pwsh.exe`.
fn candidate_names(name: &str) -> Vec<String> {
    #[cfg(windows)]
    {
        let pathext =
            std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
        let mut names = vec![name.to_string()];
        for ext in pathext.split(';').filter(|e| !e.is_empty()) {
            names.push(format!("{name}{ext}"));
        }
        names
    }
    #[cfg(not(windows))]
    {
        vec![name.to_string()]
    }
}

/// Executable predicate: Unix checks the executable bit; Windows accepts
/// any file (PATHEXT extension resolution happens in `candidate_names`
/// and again at spawn time).
fn is_executable(path: &Path, meta: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = path;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = (path, meta);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_host_block() {
        let input = "Host foo\n    HostName bar.example\n    User alice\n";
        let entries = parse_config(input, "xterm");
        assert_eq!(entries.len(), 1);
        let e = &entries[0];
        assert_eq!(e.label, "foo");
        let desc = e.description.as_deref().unwrap_or("");
        assert!(
            desc.contains("bar.example"),
            "expected HostName in description: {desc}"
        );
        assert!(
            desc.contains("alice"),
            "expected User in description: {desc}"
        );
    }

    #[test]
    fn wildcard_host_skipped() {
        let input = "Host *\n    ServerAliveInterval 60\n";
        let entries = parse_config(input, "xterm");
        assert!(entries.is_empty(), "wildcard Host must be skipped");
    }

    #[test]
    fn host_with_only_hostname() {
        let input = "Host bastion\n    HostName 10.0.0.1\n";
        let entries = parse_config(input, "xterm");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].description.as_deref(), Some("10.0.0.1"));
    }

    #[test]
    fn host_no_extras_has_no_description() {
        let input = "Host bare\n";
        let entries = parse_config(input, "xterm");
        assert_eq!(entries.len(), 1);
        assert!(entries[0].description.is_none());
    }

    #[test]
    fn host_pattern_list_splits_into_entries() {
        // ssh_config(5) allows `Host dev prod`; each pattern is its own
        // entry, sharing the block's HostName.
        let input = "Host dev prod\n    HostName example.com\n";
        let entries = parse_config(input, "xterm");
        let labels: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, ["dev", "prod"]);
        assert!(
            entries
                .iter()
                .all(|e| e.description.as_deref() == Some("example.com"))
        );
    }

    #[test]
    fn wildcard_among_patterns_skipped_keeps_others() {
        // `Host foo * bar` — the wildcard pattern is dropped, the real ones
        // kept.
        let input = "Host foo * bar\n";
        let entries = parse_config(input, "xterm");
        let labels: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, ["foo", "bar"]);
    }

    #[test]
    fn wildcard_block_settings_do_not_leak_into_next_block() {
        // A wildcard-only block contributes no entries, so its HostName must
        // not leak into the following block's description.
        let input = "Host *\n    HostName example.com\nHost foo\n";
        let entries = parse_config(input, "xterm");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].label, "foo");
        assert!(entries[0].description.is_none());
    }

    // ── terminal arg-builder tests ─────────────────────────────────────────

    #[test]
    fn wt_args_use_double_dash() {
        let args = build_terminal_args("wt.exe", "myserver");
        assert_eq!(args, vec!["--", "ssh", "myserver"]);
    }

    #[test]
    fn pwsh_args_use_noexist_command() {
        let args = build_terminal_args("pwsh.exe", "myserver");
        assert_eq!(args, vec!["-NoExit", "-Command", "ssh 'myserver'"]);
    }

    #[test]
    fn powershell_args_use_noexist_command() {
        let args = build_terminal_args("powershell.exe", "myserver");
        assert_eq!(args, vec!["-NoExit", "-Command", "ssh 'myserver'"]);
    }

    #[test]
    fn cmd_args_use_slash_k() {
        let args = build_terminal_args("cmd.exe", "myserver");
        assert_eq!(args, vec!["/K", "ssh \"myserver\""]);
    }

    #[test]
    fn pwsh_host_with_metacharacters_is_quoted() {
        // `Host foo; notepad` must not become a script: pwsh -Command parses
        // the string, so the metacharacters must sit inside a literal quote.
        let args = build_terminal_args("pwsh.exe", "foo; notepad");
        assert_eq!(args, vec!["-NoExit", "-Command", "ssh 'foo; notepad'"]);
    }

    #[test]
    fn pwsh_host_with_single_quote_is_doubled() {
        // Embedded single quotes are escaped by doubling inside pwsh's
        // single-quoted literal.
        let args = build_terminal_args("pwsh.exe", "it's");
        assert_eq!(args, vec!["-NoExit", "-Command", "ssh 'it''s'"]);
    }

    #[test]
    fn cmd_host_with_metacharacters_falls_back_to_argv() {
        // cmd has no reliable quoting — an unsafe host must take the argv
        // path (visible error) rather than a /K script string.
        let args = build_terminal_args("cmd.exe", "foo; notepad");
        assert_eq!(args, vec!["-e", "ssh", "foo; notepad"]);
    }

    #[test]
    fn cmd_safe_host_quoted() {
        let args = build_terminal_args("cmd.exe", "myserver");
        assert_eq!(args, vec!["/K", "ssh \"myserver\""]);
    }

    #[test]
    fn unix_terminal_uses_dash_e() {
        for terminal in ["alacritty", "kitty", "foot", "xterm"] {
            let args = build_terminal_args(terminal, "myserver");
            assert_eq!(
                args,
                vec!["-e", "ssh", "myserver"],
                "expected -e argv for {terminal}"
            );
        }
    }

    #[test]
    #[cfg(windows)]
    fn windows_candidates_include_wt() {
        let candidates = terminal_candidates();
        assert!(
            candidates.contains(&"wt.exe"),
            "wt.exe must be a Windows terminal candidate"
        );
        assert!(
            candidates.contains(&"cmd.exe"),
            "cmd.exe must be a Windows terminal candidate"
        );
    }

    #[test]
    #[cfg(unix)]
    fn unix_candidates_include_alacritty() {
        let candidates = terminal_candidates();
        assert!(
            candidates.contains(&"alacritty"),
            "alacritty must be a unix terminal candidate"
        );
        assert!(
            !candidates.iter().any(|c| c.ends_with(".exe")),
            "unix candidates must not contain .exe names"
        );
    }

    #[test]
    fn home_dir_does_not_panic() {
        // On any platform, home_dir() must not panic — it may return None if
        // home is genuinely unset, but it must not abort the process.
        let _ = home_dir();
    }

    // ── tool-probe tests ───────────────────────────────────────────────────

    #[test]
    fn probe_finds_executable_in_path() {
        let dir = tempfile::tempdir().unwrap();
        let tool = dir.path().join("pikr-test-tool");
        std::fs::write(&tool, b"#!/bin/sh\nexit 0").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = std::env::join_paths([dir.path()]).unwrap();
        assert!(probe_in_path(&path, "pikr-test-tool"));
    }

    #[cfg(unix)]
    #[test]
    fn probe_ignores_non_executable_in_path() {
        // Same file WITHOUT the exec bit must not be found on Unix.
        let dir = tempfile::tempdir().unwrap();
        let tool = dir.path().join("pikr-test-tool");
        std::fs::write(&tool, b"#!/bin/sh\nexit 0").unwrap();
        let path = std::env::join_paths([dir.path()]).unwrap();
        assert!(!probe_in_path(&path, "pikr-test-tool"));
    }

    #[test]
    fn probe_misses_absent_tool() {
        let dir = tempfile::tempdir().unwrap();
        let path = std::env::join_paths([dir.path()]).unwrap();
        assert!(!probe_in_path(&path, "no-such-tool-xyz"));
    }

    #[test]
    fn probe_path_like_name_directly() {
        // An absolute $TERMINAL value is probed as a path, not via PATH.
        let dir = tempfile::tempdir().unwrap();
        let tool = dir.path().join("direct-tool");
        std::fs::write(&tool, b"#!/bin/sh\nexit 0").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        assert!(is_tool_installed(tool.to_str().unwrap()));
    }
}
