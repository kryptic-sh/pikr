//! ssh mode — SSH host picker from `~/.ssh/config` (and `/etc/ssh/ssh_config`
//! on Unix), following their `Include`s.

use super::{Entry, Mode};
use anyhow::Result;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct Ssh;

/// The resolved terminal to launch ssh inside. `$TERMINAL` is shell-split so
/// the common `TERMINAL="alacritty --class foo"` idiom works: the program is
/// what gets probed / spawned, the rest become prefix args to every ssh
/// launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Terminal {
    /// Binary name (or path) passed to `Command::new`.
    pub program: String,
    /// Extra args from the `$TERMINAL` value, prepended to the ssh argv.
    pub prefix_args: Vec<String>,
}

impl Terminal {
    fn new(program: String, prefix_args: Vec<String>) -> Self {
        Self {
            program,
            prefix_args,
        }
    }

    /// Plain binary name, no prefix args — what the candidate fallbacks and
    /// the tests use.
    fn simple(name: &str) -> Self {
        Self {
            program: name.to_string(),
            prefix_args: Vec::new(),
        }
    }
}

impl Mode for Ssh {
    fn collect(&mut self) -> Result<Vec<Entry>> {
        let terminal = resolve_terminal();
        let mut entries: Vec<Entry> = Vec::new();

        let home = home_dir();
        for (path, base) in config_paths() {
            entries.extend(read_config(&path, &base, home.as_deref(), &terminal, 0));
        }

        entries.sort_by_key(|a| a.label.to_lowercase());
        // Deduplicate: user config (~/.ssh/config) is parsed first; if the same
        // host appears in both user and system config, keep the first occurrence.
        entries.dedup_by(|a, b| a.label.eq_ignore_ascii_case(&b.label));
        Ok(entries)
    }
}

/// Each config file, with the directory its relative `Include`s resolve
/// against (ssh_config(5): `~/.ssh` for the user's, `/etc/ssh` for the
/// system's).
fn config_paths() -> Vec<(PathBuf, PathBuf)> {
    let mut paths = Vec::new();
    if let Some(home) = home_dir() {
        let dir = home.join(".ssh");
        paths.push((dir.join("config"), dir));
    }
    // System-wide ssh config only exists on Unix.
    #[cfg(unix)]
    paths.push((
        PathBuf::from("/etc/ssh/ssh_config"),
        PathBuf::from("/etc/ssh"),
    ));
    paths
}

/// How deep `Include`s may nest: OpenSSH's own limit (`READCONF_MAX_DEPTH`),
/// which also ends a file that includes itself.
const MAX_INCLUDE_DEPTH: usize = 16;

/// The entries of the config at `path` and of everything it includes, an
/// included file's entries where its `Include` stands. A file that cannot be
/// read gives none, as ssh skips an `Include` that matches nothing.
fn read_config(
    path: &Path,
    base: &Path,
    home: Option<&Path>,
    terminal: &Terminal,
    depth: usize,
) -> Vec<Entry> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    parse_config_with(&text, terminal, &mut |value| {
        if depth >= MAX_INCLUDE_DEPTH {
            tracing::warn!(path = %path.display(), "ssh config Include nested too deep; ignored");
            return Vec::new();
        }
        include_paths(value, base, home)
            .iter()
            .flat_map(|file| read_config(file, base, home, terminal, depth + 1))
            .collect()
    })
}

/// The files an `Include` line's `value` names, in the order ssh reads them:
/// each whitespace-separated path, `~/` taken from `home`, a relative one
/// resolved against `base`, and a last component with `*` or `?` expanded to
/// the matching files in its directory, sorted. Wildcards in a directory
/// component, `[...]` classes, `%` tokens and environment variables, which
/// ssh also accepts, are not expanded.
fn include_paths(value: &str, base: &Path, home: Option<&Path>) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for word in value.split_whitespace() {
        let path = match (word.strip_prefix("~/"), home) {
            (Some(rest), Some(home)) => home.join(rest),
            _ => base.join(word),
        };
        let pattern = path.file_name().and_then(|name| name.to_str());
        let Some(pattern) = pattern.filter(|name| name.contains(['*', '?'])) else {
            files.push(path);
            continue;
        };
        let dir = path.parent().unwrap_or(base);
        let Ok(listing) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut matched: Vec<PathBuf> = listing
            .filter_map(|item| item.ok())
            .filter(|item| {
                item.file_name()
                    .to_str()
                    .is_some_and(|name| wildcard_match(pattern, name))
            })
            .map(|item| item.path())
            .collect();
        matched.sort();
        files.extend(matched);
    }
    files
}

/// Whether `name` matches `pattern`, where `*` stands for any run of chars
/// and `?` for one, as glob(3) matches a single path component: a leading
/// `.` in `name` is matched only by a leading `.` in `pattern`.
fn wildcard_match(pattern: &str, name: &str) -> bool {
    if name.starts_with('.') && !pattern.starts_with('.') {
        return false;
    }
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    // Greedy match with backtracking to the last `*`.
    let (mut p, mut n) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == name[n]) {
            p += 1;
            n += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, n));
            p += 1;
        } else if let Some((star_p, star_n)) = star {
            p = star_p + 1;
            n = star_n + 1;
            star = Some((star_p, star_n + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
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

/// Walk the ssh config text, returning one Entry per non-wildcard Host,
/// with each `Include` line's value handed to `include`. The block the line
/// sits in carries on after it, as ssh restores its state after an included
/// file, and the included entries follow that block's: the order hosts were
/// first defined in, which decides which of two same-named ones is kept.
fn parse_config_with(
    text: &str,
    terminal: &Terminal,
    include: &mut dyn FnMut(&str) -> Vec<Entry>,
) -> Vec<Entry> {
    let mut entries = Vec::new();
    // Included entries, held until the block the `Include` sat in is out.
    let mut included = Vec::new();
    let mut current_hosts: Vec<String> = Vec::new();
    let mut hostname: Option<String> = None;
    let mut user: Option<String> = None;

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        // ssh_config(5): the keyword and its arguments are separated by
        // whitespace or by optional whitespace and exactly one `=`.
        let Some((key, rest)) = trimmed.split_once(|c: char| c.is_whitespace() || c == '=') else {
            continue;
        };
        let rest = rest.trim_start();
        let value = rest.strip_prefix('=').unwrap_or(rest).trim().to_string();
        let key = key.to_ascii_lowercase();

        if key == "include" {
            included.extend(include(&value));
            continue;
        }
        if key == "host" || key == "match" {
            // Flush the previous block before starting a new one.
            flush_hosts(
                &mut entries,
                &mut current_hosts,
                &mut hostname,
                &mut user,
                terminal,
            );
            entries.append(&mut included);
            if key == "match" {
                // A `Match` block names no hosts of its own; its settings
                // apply conditionally, so they belong to no listed entry.
                hostname = None;
                user = None;
                continue;
            }
            // ssh_config(5) allows `Host a b` — one entry per pattern.
            // Wildcard and negated (`!host`) patterns are not real hosts —
            // skip them.
            current_hosts = value
                .split_whitespace()
                .filter(|p| !p.contains('*') && !p.contains('?') && !p.starts_with('!'))
                .map(str::to_string)
                .collect();
            if current_hosts.is_empty() {
                // A wildcard-only block contributes no entries — its
                // HostName/User must not leak into the next block.
                hostname = None;
                user = None;
            }
        } else if !current_hosts.is_empty() {
            // ssh uses the first value it reads for each keyword.
            match key.as_str() {
                "hostname" => _ = hostname.get_or_insert(value),
                "user" => _ = user.get_or_insert(value),
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
    entries.append(&mut included);
    entries
}

/// Emit one entry per accumulated host, sharing the block's HostName/User,
/// then clear the block state. No-op when the block produced no hosts.
fn flush_hosts(
    entries: &mut Vec<Entry>,
    hosts: &mut Vec<String>,
    hostname: &mut Option<String>,
    user: &mut Option<String>,
    terminal: &Terminal,
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
    terminal: &Terminal,
) -> Entry {
    let description = match (hostname.as_deref(), user.as_deref()) {
        (Some(h), Some(u)) => Some(format!("{h} ({u})")),
        (Some(h), None) => Some(h.to_string()),
        (None, Some(u)) => Some(u.to_string()),
        (None, None) => None,
    };
    // Prefix args from `$TERMINAL` (e.g. `--class foo`) come before the
    // terminal's own launch convention.
    let mut args = terminal.prefix_args.clone();
    args.extend(build_terminal_args(&terminal.program, &host));
    Entry::exec_with(host, terminal.program.clone(), args, description, None)
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
/// - `cmd.exe`: `/K ssh <host>` for a host in the safe charset, which needs
///   no quoting; `/K` runs the command and keeps the prompt alive
///   afterwards. cmd has no reliable quoting, so any other host is refused
///   with a fixed message in the window rather than reaching its command
///   line.
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
            // and an embedded quote is escaped by doubling. PowerShell takes
            // the typographic single quotes as quotes too, so each is doubled.
            format!("ssh '{}'", pwsh_single_quoted(host)),
        ],
        "cmd.exe" | "cmd" => {
            // Separate arguments: one `ssh "host"` argument reaches cmd as
            // `"ssh \"host\""`, and ssh gets the quotes as part of the host.
            if is_safe_host(host) {
                vec!["/K".to_string(), "ssh".to_string(), host.to_string()]
            } else {
                vec![
                    "/K".to_string(),
                    "echo".to_string(),
                    CMD_UNSAFE_HOST.to_string(),
                ]
            }
        }
        // Unix terminals (alacritty, kitty, foot, xterm) and anything unknown.
        _ => vec!["-e".to_string(), "ssh".to_string(), host.to_string()],
    }
}

/// What cmd shows for a host it cannot be given safely. A fixed string: no
/// part of the host reaches cmd's command line.
const CMD_UNSAFE_HOST: &str =
    "pikr: this host has characters cmd.exe cannot be passed safely; use wt.exe or pwsh";

/// `text` for a PowerShell single-quoted string: every character PowerShell
/// reads as a single quote (`'`, `‘`, `’`, `‚`, `‛`) is doubled, which is how
/// it escapes one inside such a string.
fn pwsh_single_quoted(text: &str) -> String {
    let mut quoted = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            quoted.push(c);
        }
        quoted.push(c);
    }
    quoted
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
/// The `$TERMINAL` value is shell-split, so `TERMINAL="alacritty --class foo"`
/// resolves to program `alacritty` with prefix args `--class foo` — the old
/// whole-string probe silently fell back to a candidate for that common idiom.
fn resolve_terminal() -> Terminal {
    if let Ok(t) = std::env::var("TERMINAL")
        && !t.is_empty()
    {
        // The program (first word) is what gets probed; the rest become
        // prefix args. Unparseable or missing values fall through to the
        // candidates below.
        if let Some(mut split) = shlex::split(&t)
            && let Some(program) = split.first()
            && is_tool_installed(program)
        {
            let program = split.remove(0);
            return Terminal::new(program, split);
        }
    }
    for &name in terminal_candidates() {
        if is_tool_installed(name) {
            return Terminal::simple(name);
        }
    }
    // Last resort: first candidate even if not found; exec will fail with a
    // clear error rather than silently doing nothing.
    Terminal::simple(terminal_candidates().first().copied().unwrap_or("xterm"))
}

/// Probe whether a tool exists by walking `$PATH` and stat'ing the
/// binary — cheaper than spawning `--version` per candidate on the
/// launch path (a fork/exec/wait per miss).
fn is_tool_installed(name: &str) -> bool {
    if name.contains('/') || name.contains('\\') {
        // Path-like `$TERMINAL` value ("/usr/bin/alacritty"): probe the
        // path directly, not via PATH joins.
        return is_executable_file(Path::new(name));
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
            is_executable_file(&path)
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

/// A regular file that is executable — shared probe used by the
/// path-like `$TERMINAL` branch of `is_tool_installed` and by every
/// `probe_in_path` candidate.
fn is_executable_file(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|m| m.is_file() && is_executable(path, &m))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One config's text alone, its `Include` lines skipped.
    fn parse_config(text: &str, terminal: &Terminal) -> Vec<Entry> {
        parse_config_with(text, terminal, &mut |_| Vec::new())
    }

    #[test]
    fn parse_simple_host_block() {
        let input = "Host foo\n    HostName bar.example\n    User alice\n";
        let entries = parse_config(input, &Terminal::simple("xterm"));
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
    fn terminal_prefix_args_prepend_ssh_argv() {
        // `TERMINAL="alacritty --class foo"` resolves to program alacritty
        // with prefix args ["--class", "foo"]; every ssh launch must carry
        // them before the terminal's own `-e ssh <host>` convention.
        let terminal = Terminal::new(
            "alacritty".to_string(),
            vec!["--class".to_string(), "foo".to_string()],
        );
        let input = "Host dev\n";
        let entries = parse_config(input, &terminal);
        assert_eq!(entries.len(), 1);
        match &entries[0].payload {
            crate::modes::Payload::Exec { program, args } => {
                assert_eq!(program, "alacritty");
                assert_eq!(args, &["--class", "foo", "-e", "ssh", "dev"]);
            }
            other => panic!("expected Exec payload, got {other:?}"),
        }
    }

    #[test]
    fn wildcard_host_skipped() {
        let input = "Host *\n    ServerAliveInterval 60\n";
        let entries = parse_config(input, &Terminal::simple("xterm"));
        assert!(entries.is_empty(), "wildcard Host must be skipped");
    }

    #[test]
    fn host_with_only_hostname() {
        let input = "Host bastion\n    HostName 10.0.0.1\n";
        let entries = parse_config(input, &Terminal::simple("xterm"));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].description.as_deref(), Some("10.0.0.1"));
    }

    #[test]
    fn host_no_extras_has_no_description() {
        let input = "Host bare\n";
        let entries = parse_config(input, &Terminal::simple("xterm"));
        assert_eq!(entries.len(), 1);
        assert!(entries[0].description.is_none());
    }

    #[test]
    fn host_pattern_list_splits_into_entries() {
        // ssh_config(5) allows `Host dev prod`; each pattern is its own
        // entry, sharing the block's HostName.
        let input = "Host dev prod\n    HostName example.com\n";
        let entries = parse_config(input, &Terminal::simple("xterm"));
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
        let entries = parse_config(input, &Terminal::simple("xterm"));
        let labels: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, ["foo", "bar"]);
    }

    #[test]
    fn wildcard_block_settings_do_not_leak_into_next_block() {
        // A wildcard-only block contributes no entries, so its HostName must
        // not leak into the following block's description.
        let input = "Host *\n    HostName example.com\nHost foo\n";
        let entries = parse_config(input, &Terminal::simple("xterm"));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].label, "foo");
        assert!(entries[0].description.is_none());
    }

    #[test]
    fn match_block_settings_do_not_leak_into_previous_host() {
        // A `Match` block is its own conditional context, not part of the
        // `Host` block above it.
        let input = "Host github\n  HostName github.com\nMatch host prod\n  User deploy\nHost prod\n  HostName prod.example\n";
        let entries = parse_config(input, &Terminal::simple("xterm"));
        let descriptions: Vec<_> = entries
            .iter()
            .map(|e| (e.label.as_str(), e.description.as_deref()))
            .collect();
        assert_eq!(
            descriptions,
            [
                ("github", Some("github.com")),
                ("prod", Some("prod.example"))
            ]
        );
    }

    /// Labels and descriptions `input` parses to.
    fn parsed(input: &str) -> Vec<(String, Option<String>)> {
        parse_config(input, &Terminal::simple("xterm"))
            .into_iter()
            .map(|e| (e.label, e.description))
            .collect()
    }

    #[test]
    fn keyword_and_value_may_be_joined_by_equals() {
        // ssh_config(5): whitespace, or optional whitespace and one `=`.
        let input = "Host=alpha
  HostName=alpha.example
Host = beta
  HostName = beta.example
";
        assert_eq!(
            parsed(input),
            [
                ("alpha".to_string(), Some("alpha.example".to_string())),
                ("beta".to_string(), Some("beta.example".to_string())),
            ]
        );
    }

    #[test]
    fn negated_patterns_are_not_hosts() {
        let input = "Host !bastion web
  HostName web.example
";
        assert_eq!(
            parsed(input),
            [("web".to_string(), Some("web.example".to_string()))]
        );
    }

    #[test]
    fn the_first_value_of_a_keyword_wins_as_in_ssh() {
        let input = "Host web
  HostName first.example
  HostName second.example
";
        assert_eq!(
            parsed(input),
            [("web".to_string(), Some("first.example".to_string()))]
        );
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
        assert_eq!(args, vec!["/K", "ssh", "myserver"]);
    }

    /// The argv cmd builds its command line from must give ssh the bare
    /// host: run the same shape with `/C echo` and read what cmd ran.
    #[cfg(windows)]
    #[test]
    fn cmd_passes_the_host_without_quotes() {
        let mut args = build_terminal_args("cmd.exe", "my.server-1");
        args[0] = "/C".into();
        args[1] = "echo".into();
        let out = std::process::Command::new("cmd.exe")
            .args(&args)
            .output()
            .expect("run cmd.exe");
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "my.server-1");
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
    fn cmd_host_with_metacharacters_is_refused() {
        // cmd has no reliable quoting — an unsafe host must not reach its
        // command line at all; cmd shows a fixed message instead.
        let args = build_terminal_args("cmd.exe", "foo & notepad");
        assert_eq!(args, vec!["/K", "echo", CMD_UNSAFE_HOST]);
        assert!(!args.iter().any(|a| a.contains("notepad")));
    }

    #[test]
    fn pwsh_doubles_every_kind_of_single_quote() {
        let args = build_terminal_args("pwsh.exe", "foo\u{2019};calc;\u{2019}");
        assert_eq!(args[2], "ssh 'foo\u{2019}\u{2019};calc;\u{2019}\u{2019}'");
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

    #[test]
    fn wildcard_match_follows_glob_rules() {
        assert!(wildcard_match("*.conf", "a.conf"));
        assert!(!wildcard_match("*.conf", "a.conf.bak"));
        assert!(wildcard_match("a?c", "abc"));
        assert!(!wildcard_match("a?c", "ac"));
        assert!(wildcard_match("a*b*c", "axxbyyc"));
        assert!(wildcard_match("a*b", "ab"));
        assert!(!wildcard_match("a*b", "abc"));
        assert!(wildcard_match("**", "x"));
        // glob(3): a leading dot is matched only by a leading dot.
        assert!(!wildcard_match("*", ".hidden"));
        assert!(!wildcard_match("*.conf", ".hidden.conf"));
        assert!(wildcard_match(".*", ".hidden"));
    }

    fn labels(entries: &[Entry]) -> Vec<String> {
        entries.iter().map(|entry| entry.label.clone()).collect()
    }

    #[test]
    fn include_reads_matching_files_in_order_where_it_stands() {
        let dir = tempfile::tempdir().unwrap();
        let conf_d = dir.path().join("conf.d");
        std::fs::create_dir(&conf_d).unwrap();
        std::fs::write(conf_d.join("b.conf"), "Host bravo\n").unwrap();
        std::fs::write(conf_d.join("a.conf"), "Host alpha\n").unwrap();
        std::fs::write(conf_d.join("notes.txt"), "Host notes\n").unwrap();
        std::fs::write(conf_d.join(".hidden.conf"), "Host hidden\n").unwrap();
        let config = dir.path().join("config");
        std::fs::write(&config, "Host first\nInclude conf.d/*.conf\nHost last\n").unwrap();
        let entries = read_config(&config, dir.path(), None, &Terminal::simple("alacritty"), 0);
        assert_eq!(labels(&entries), ["first", "alpha", "bravo", "last"]);
    }

    #[test]
    fn include_resolves_home_and_absolute_paths() {
        let home = tempfile::tempdir().unwrap();
        let base = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join("hosts"), "Host from-home\n").unwrap();
        let absolute = base.path().join("abs");
        std::fs::write(&absolute, "Host from-absolute\n").unwrap();
        let config = base.path().join("config");
        std::fs::write(&config, format!("Include ~/hosts {}\n", absolute.display())).unwrap();
        let entries = read_config(
            &config,
            base.path(),
            Some(home.path()),
            &Terminal::simple("alacritty"),
            0,
        );
        assert_eq!(labels(&entries), ["from-home", "from-absolute"]);
    }

    #[test]
    fn include_inside_a_host_block_keeps_the_block_going() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("extra"), "Host child\n").unwrap();
        let config = dir.path().join("config");
        std::fs::write(
            &config,
            "Host parent\n    Include extra\n    HostName parent.example\n",
        )
        .unwrap();
        let entries = read_config(&config, dir.path(), None, &Terminal::simple("alacritty"), 0);
        assert_eq!(labels(&entries), ["parent", "child"]);
        assert_eq!(entries[0].description.as_deref(), Some("parent.example"));
    }

    #[test]
    fn a_config_including_itself_stops_at_the_depth_limit() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config");
        std::fs::write(&config, "Host loop\nInclude config\n").unwrap();
        let entries = read_config(&config, dir.path(), None, &Terminal::simple("alacritty"), 0);
        assert_eq!(entries.len(), MAX_INCLUDE_DEPTH + 1);
    }
}
