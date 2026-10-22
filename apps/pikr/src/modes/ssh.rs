//! ssh mode — SSH host picker from `~/.ssh/config` (and `/etc/ssh/ssh_config`
//! on Unix), following their `Include`s, and the hosts in `known_hosts`.

use super::{Entry, Mode};
use anyhow::Result;
use std::fmt::Write as _;
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
        let mut read = std::collections::HashSet::new();
        for (path, base) in config_paths() {
            entries.extend(read_config(
                &path,
                &base,
                home.as_deref(),
                &terminal,
                0,
                &mut read,
            ));
        }
        // After the configs, so a config `Host` of the same name, which can
        // carry a description, is the one kept.
        for path in known_hosts_paths() {
            if let Ok(text) = std::fs::read_to_string(&path) {
                for known in known_hosts(&text) {
                    entries.push(known_host_entry(known, &terminal));
                }
            }
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

/// The known-hosts files ssh reads by default: the user's
/// `UserKnownHostsFile` and, on Unix, the system's `GlobalKnownHostsFile`.
fn known_hosts_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(home) = home_dir() {
        paths.push(home.join(".ssh").join("known_hosts"));
    }
    #[cfg(unix)]
    paths.push(PathBuf::from("/etc/ssh/ssh_known_hosts"));
    paths
}

/// One host a known_hosts file names, with its port when not ssh's default.
#[derive(Debug, PartialEq, Eq)]
struct KnownHost {
    host: String,
    port: Option<u16>,
}

/// The hosts in a known_hosts file (sshd(8) "SSH_KNOWN_HOSTS FILE FORMAT"):
/// each line's comma-separated first field, `[host]:port` read as a host on
/// that port. Not listed: hashed entries (`|1|…`, which only ssh can match),
/// `@cert-authority` and `@revoked` lines, wildcard and negated patterns,
/// and a name starting with `-`, which ssh never writes and would read as an
/// option.
fn known_hosts(text: &str) -> Vec<KnownHost> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#') && !line.starts_with('@'))
        .filter_map(|line| line.split_whitespace().next())
        .filter(|field| !field.starts_with('|'))
        .flat_map(|field| field.split(','))
        .filter_map(|name| {
            let known = match name.strip_prefix('[') {
                Some(bracketed) => {
                    let (host, port) = bracketed.split_once("]:")?;
                    KnownHost {
                        host: host.to_owned(),
                        port: Some(port.parse().ok()?),
                    }
                }
                None => KnownHost {
                    host: name.to_owned(),
                    port: None,
                },
            };
            let bad = known.host.is_empty()
                || known.host.starts_with(['!', '-'])
                || known.host.contains(['*', '?']);
            (!bad).then_some(known)
        })
        .collect()
}

/// The row for a known host: `host`, or `host:port` run as `ssh -p PORT host`.
fn known_host_entry(known: KnownHost, terminal: &Terminal) -> Entry {
    match known.port {
        Some(port) => {
            let label = format!("{}:{port}", known.host);
            let ssh = ["-p".to_owned(), port.to_string(), known.host];
            ssh_entry(label, &ssh, None, terminal)
        }
        None => ssh_entry(known.host.clone(), &[known.host], None, terminal),
    }
}

/// How deep `Include`s may nest: OpenSSH's own limit (`READCONF_MAX_DEPTH`),
/// which also ends a file that includes itself.
const MAX_INCLUDE_DEPTH: usize = 16;

/// The entries of the config at `path` and of everything it includes, an
/// included file's entries where its `Include` stands. A file that cannot be
/// read gives none, as ssh skips an `Include` that matches nothing.
///
/// Each file is read once per walk (`read` holds the ones already read): a
/// second read could only repeat entries that are deduplicated anyway, and
/// files that include each other, or a glob matching its own file, would
/// otherwise be read a number of times exponential in [`MAX_INCLUDE_DEPTH`].
fn read_config(
    path: &Path,
    base: &Path,
    home: Option<&Path>,
    terminal: &Terminal,
    depth: usize,
    read: &mut std::collections::HashSet<PathBuf>,
) -> Vec<Entry> {
    let identity = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !read.insert(identity) {
        return Vec::new();
    }
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
            .flat_map(|file| read_config(file, base, home, terminal, depth + 1, read))
            .collect::<Vec<_>>()
    })
}

/// The files an `Include` line's `value` names, in the order ssh reads them:
/// each whitespace-separated path with its tokens and environment variables
/// expanded ([`expand_include_word`]), `~/` taken from `home`, a relative
/// one resolved against `base`, and wildcards expanded ([`glob_paths`]).
fn include_paths(value: &str, base: &Path, home: Option<&Path>) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for word in config_words(value) {
        let Some(word) = expand_include_word(&word, home, |name| std::env::var(name).ok()) else {
            tracing::debug!(%word, "ssh config Include path not expandable here; skipped");
            continue;
        };
        let path = match (word.strip_prefix("~/"), home) {
            (Some(rest), Some(home)) => home.join(rest),
            _ => base.join(&word),
        };
        files.extend(glob_paths(&path));
    }
    files
}

/// `word` from an `Include` line with its `%` tokens and `${NAME}`
/// environment variables (ssh_config(5) "TOKENS" and "ENVIRONMENT
/// VARIABLES") replaced, `env` looking the variables up. `None` when ssh
/// would ignore the path (an unset variable, an unknown token, an unclosed
/// `${`) or when it uses a token pikr cannot know while it lists hosts: the
/// destination's (`%h`, `%p`, `%r`, …) and the local user's and machine's
/// (`%u`, `%i`, `%L`, `%l`). Only `%%` and `%d` (`home`) are expanded.
fn expand_include_word(
    word: &str,
    home: Option<&Path>,
    env: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    let mut expanded = String::with_capacity(word.len());
    let mut rest = word;
    while let Some(at) = rest.find(['%', '$']) {
        expanded.push_str(&rest[..at]);
        let marker = &rest[at..];
        if let Some(var) = marker.strip_prefix("${") {
            let (name, after) = var.split_once('}')?;
            expanded.push_str(&env(name)?);
            rest = after;
        } else if let Some(after) = marker.strip_prefix("%%") {
            expanded.push('%');
            rest = after;
        } else if let Some(after) = marker.strip_prefix("%d") {
            expanded.push_str(home?.to_str()?);
            rest = after;
        } else if marker.starts_with('%') {
            return None;
        } else {
            expanded.push('$');
            rest = &marker[1..];
        }
    }
    expanded.push_str(rest);
    Some(expanded)
}

/// The paths `pattern` names, as glob(3) expands it: each component holding
/// a wildcard is matched ([`wildcard_match`]) against the listing of the
/// directories the components before it named, and the result is sorted by
/// bytes, as glob's `strcmp` sort is. A pattern without wildcards names
/// itself, whether or not it exists.
fn glob_paths(pattern: &Path) -> Vec<PathBuf> {
    let mut found = vec![PathBuf::new()];
    for component in pattern.components() {
        let wildcard = component
            .as_os_str()
            .to_str()
            .filter(|text| text.contains(['*', '?', '[']));
        let Some(wildcard) = wildcard else {
            for path in &mut found {
                path.push(component);
            }
            continue;
        };
        found = found
            .iter()
            .flat_map(|dir| {
                let listed = if dir.as_os_str().is_empty() {
                    Path::new(".")
                } else {
                    dir
                };
                std::fs::read_dir(listed)
                    .into_iter()
                    .flatten()
                    .filter_map(|item| item.ok())
                    .filter(|item| {
                        item.file_name()
                            .to_str()
                            .is_some_and(|name| wildcard_match(wildcard, name))
                    })
                    .map(|item| dir.join(item.file_name()))
                    .collect::<Vec<_>>()
            })
            .collect();
    }
    found.sort_by(|a, b| a.as_os_str().cmp(b.as_os_str()));
    found
}

/// A config value's words as ssh reads them: split at whitespace, with
/// double quotes keeping a word whole (`\"` and `\\` escape inside them),
/// and a `#` that starts a word, outside quotes, ending the line as a
/// comment, after OpenSSH's `argv_split`.
fn config_words(value: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut chars = value.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        match chars.peek() {
            None | Some('#') => break,
            Some(_) => {}
        }
        let mut word = String::new();
        let mut quoted = false;
        while let Some(c) = chars.next() {
            match c {
                '"' => quoted = !quoted,
                '\\' if quoted && matches!(chars.peek(), Some('"' | '\\')) => {
                    word.extend(chars.next());
                }
                c if c.is_whitespace() && !quoted => break,
                c => word.push(c),
            }
        }
        words.push(word);
    }
    words
}

/// One element of a glob(7) pattern.
enum Glob {
    Char(char),
    /// `?`
    Any,
    /// `*`
    Star,
    /// `[...]`: inclusive ranges, a lone character being a range of one.
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
}

impl Glob {
    fn matches(&self, c: char) -> bool {
        match self {
            Glob::Char(want) => *want == c,
            Glob::Any => true,
            Glob::Star => false,
            Glob::Class { negated, ranges } => {
                ranges.iter().any(|&(low, high)| (low..=high).contains(&c)) != *negated
            }
        }
    }
}

/// `pattern` read as glob(7) reads one path component: `*`, `?`, and
/// `[...]` classes with ranges, negated by a leading `!` (or `^`, which
/// glibc also takes), a `]` first in the class standing for itself; a `\`
/// makes the next character literal, and a `[` that is never closed is a
/// literal `[`.
fn glob_pattern(pattern: &str) -> Vec<Glob> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut parsed = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' => parsed.push(Glob::Star),
            '?' => parsed.push(Glob::Any),
            '\\' if i + 1 < chars.len() => {
                i += 1;
                parsed.push(Glob::Char(chars[i]));
            }
            '[' => match glob_class(&chars, i + 1) {
                Some((class, end)) => {
                    parsed.push(class);
                    i = end;
                    continue;
                }
                None => parsed.push(Glob::Char('[')),
            },
            c => parsed.push(Glob::Char(c)),
        }
        i += 1;
    }
    parsed
}

/// The class whose body starts at `chars[start]`, just past its `[`, and the
/// index just past its `]`; `None` if it is never closed.
fn glob_class(chars: &[char], start: usize) -> Option<(Glob, usize)> {
    let mut i = start;
    let negated = matches!(chars.get(i), Some('!' | '^'));
    if negated {
        i += 1;
    }
    let body = i;
    let mut ranges = Vec::new();
    loop {
        let low = *chars.get(i)?;
        if low == ']' && i > body {
            return Some((Glob::Class { negated, ranges }, i + 1));
        }
        match (chars.get(i + 1), chars.get(i + 2)) {
            (Some('-'), Some(&high)) if high != ']' => {
                ranges.push((low, high));
                i += 3;
            }
            _ => {
                ranges.push((low, low));
                i += 1;
            }
        }
    }
}

/// Whether `name` matches `pattern` as glob(3) matches a single path
/// component ([`glob_pattern`]): a leading `.` in `name` is matched only by
/// a literal leading `.` in `pattern`.
fn wildcard_match(pattern: &str, name: &str) -> bool {
    let pattern = glob_pattern(pattern);
    if name.starts_with('.') && !matches!(pattern.first(), Some(Glob::Char('.'))) {
        return false;
    }
    let name: Vec<char> = name.chars().collect();
    // Greedy match with backtracking to the last `*`.
    let (mut p, mut n) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        if p < pattern.len() && pattern[p].matches(name[n]) {
            p += 1;
            n += 1;
        } else if p < pattern.len() && matches!(pattern[p], Glob::Star) {
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
    pattern[p..].iter().all(|glob| matches!(glob, Glob::Star))
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
            current_hosts = config_words(&value)
                .into_iter()
                .filter(|p| !p.contains('*') && !p.contains('?') && !p.starts_with('!'))
                .collect();
            if current_hosts.is_empty() {
                // A wildcard-only block contributes no entries — its
                // HostName/User must not leak into the next block.
                hostname = None;
                user = None;
            }
        } else if !current_hosts.is_empty() {
            // ssh uses the first value it reads for each keyword.
            let first = config_words(&value).into_iter().next();
            match (key.as_str(), first) {
                ("hostname", Some(word)) => _ = hostname.get_or_insert(word),
                ("user", Some(word)) => _ = user.get_or_insert(word),
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
    ssh_entry(host.clone(), &[host], description, terminal)
}

/// A row labelled `label` that opens `terminal` running `ssh` with `ssh_args`.
fn ssh_entry(
    label: String,
    ssh_args: &[String],
    description: Option<String>,
    terminal: &Terminal,
) -> Entry {
    // Prefix args from `$TERMINAL` (e.g. `--class foo`) come before the
    // terminal's own launch convention.
    let mut args = terminal.prefix_args.clone();
    args.extend(build_terminal_args(&terminal.program, ssh_args));
    Entry::exec_with(label, terminal.program.clone(), args, description, None)
}

/// Build the argv for the chosen terminal to run `ssh` with `ssh_args` (the
/// host, after `-p PORT` for a port).
///
/// Each terminal has its own calling convention:
/// - Unix terminals (`alacritty`, `kitty`, `foot`, `xterm`): `-e ssh <args>`
/// - `wt.exe` (Windows Terminal): `-- ssh <args>`
///   Windows Terminal forwards everything after `--` to its shell/command.
/// - `pwsh.exe` / `powershell.exe`: `-NoExit -Command ssh '<arg>' …`
///   `-NoExit` keeps the window open after ssh exits so the user can see
///   error messages before the pane closes. Each argument is single-quoted —
///   pwsh `-Command` parses a script, and single quotes are literal there.
/// - `cmd.exe`: `/K ssh <args>` when every argument is in the safe charset,
///   which needs no quoting; `/K` runs the command and keeps the prompt alive
///   afterwards. cmd has no reliable quoting, so anything else is refused
///   with a fixed message in the window rather than reaching its command
///   line.
fn build_terminal_args(terminal: &str, ssh_args: &[String]) -> Vec<String> {
    let binary = std::path::Path::new(terminal)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(terminal)
        .to_ascii_lowercase();

    match binary.as_str() {
        "wt.exe" | "wt" => ["--", "ssh"]
            .into_iter()
            .map(str::to_owned)
            .chain(ssh_args.iter().cloned())
            .collect(),
        "pwsh.exe" | "pwsh" | "powershell.exe" | "powershell" => vec![
            "-NoExit".to_string(),
            "-Command".to_string(),
            // pwsh -Command parses a script; single quotes are literal in pwsh
            // and an embedded quote is escaped by doubling. PowerShell takes
            // the typographic single quotes as quotes too, so each is doubled.
            ssh_args.iter().fold("ssh".to_owned(), |mut script, arg| {
                let _ = write!(script, " '{}'", pwsh_single_quoted(arg));
                script
            }),
        ],
        "cmd.exe" | "cmd" => {
            // Separate arguments: one `ssh "host"` argument reaches cmd as
            // `"ssh \"host\""`, and ssh gets the quotes as part of the host.
            if ssh_args.iter().all(|arg| is_safe_host(arg)) {
                ["/K", "ssh"]
                    .into_iter()
                    .map(str::to_owned)
                    .chain(ssh_args.iter().cloned())
                    .collect()
            } else {
                vec![
                    "/K".to_string(),
                    "echo".to_string(),
                    CMD_UNSAFE_HOST.to_string(),
                ]
            }
        }
        // Unix terminals (alacritty, kitty, foot, xterm) and anything unknown.
        _ => ["-e", "ssh"]
            .into_iter()
            .map(str::to_owned)
            .chain(ssh_args.iter().cloned())
            .collect(),
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
    use crate::modes::Payload;

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
        let args = build_terminal_args("wt.exe", &["myserver".to_owned()]);
        assert_eq!(args, vec!["--", "ssh", "myserver"]);
    }

    #[test]
    fn pwsh_args_use_noexist_command() {
        let args = build_terminal_args("pwsh.exe", &["myserver".to_owned()]);
        assert_eq!(args, vec!["-NoExit", "-Command", "ssh 'myserver'"]);
    }

    #[test]
    fn powershell_args_use_noexist_command() {
        let args = build_terminal_args("powershell.exe", &["myserver".to_owned()]);
        assert_eq!(args, vec!["-NoExit", "-Command", "ssh 'myserver'"]);
    }

    #[test]
    fn cmd_args_use_slash_k() {
        let args = build_terminal_args("cmd.exe", &["myserver".to_owned()]);
        assert_eq!(args, vec!["/K", "ssh", "myserver"]);
    }

    /// The argv cmd builds its command line from must give ssh the bare
    /// host: run the same shape with `/C echo` and read what cmd ran.
    #[cfg(windows)]
    #[test]
    fn cmd_passes_the_host_without_quotes() {
        let mut args = build_terminal_args("cmd.exe", &["my.server-1".to_owned()]);
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
        let args = build_terminal_args("pwsh.exe", &["foo; notepad".to_owned()]);
        assert_eq!(args, vec!["-NoExit", "-Command", "ssh 'foo; notepad'"]);
    }

    #[test]
    fn pwsh_host_with_single_quote_is_doubled() {
        // Embedded single quotes are escaped by doubling inside pwsh's
        // single-quoted literal.
        let args = build_terminal_args("pwsh.exe", &["it's".to_owned()]);
        assert_eq!(args, vec!["-NoExit", "-Command", "ssh 'it''s'"]);
    }

    #[test]
    fn cmd_host_with_metacharacters_is_refused() {
        // cmd has no reliable quoting — an unsafe host must not reach its
        // command line at all; cmd shows a fixed message instead.
        let args = build_terminal_args("cmd.exe", &["foo & notepad".to_owned()]);
        assert_eq!(args, vec!["/K", "echo", CMD_UNSAFE_HOST]);
        assert!(!args.iter().any(|a| a.contains("notepad")));
    }

    #[test]
    fn pwsh_doubles_every_kind_of_single_quote() {
        let args = build_terminal_args("pwsh.exe", &["foo\u{2019};calc;\u{2019}".to_owned()]);
        assert_eq!(args[2], "ssh 'foo\u{2019}\u{2019};calc;\u{2019}\u{2019}'");
    }

    #[test]
    fn unix_terminal_uses_dash_e() {
        for terminal in ["alacritty", "kitty", "foot", "xterm"] {
            let args = build_terminal_args(terminal, &["myserver".to_owned()]);
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
    fn known_hosts_lists_plain_names_only() {
        let text = "\
# comment
github.com,140.82.121.4 ssh-ed25519 AAAAC3Nza
|1|JfKTdBh7rNbXkVAQCRp4OQoPfmI=|USECr3SWf1JUPsms5AqfD5QfxkM= ssh-rsa AAAAB3
[git.example.com]:2222 ssh-ed25519 AAAAC3Nza
@cert-authority *.example.com ssh-rsa AAAAB3
@revoked bad.example.com ssh-rsa AAAAB3
*.wild.example.com,!no.example.com,box.lan ssh-rsa AAAAB3

  server-01   ecdsa-sha2-nistp256 AAAAE2
";
        let named = |host: &str, port| KnownHost {
            host: host.to_owned(),
            port,
        };
        assert_eq!(
            known_hosts(text),
            [
                named("github.com", None),
                named("140.82.121.4", None),
                named("git.example.com", Some(2222)),
                named("box.lan", None),
                named("server-01", None),
            ]
        );
        assert_eq!(known_hosts("[a]:notaport k\n-oProxy k\n"), []);
    }

    #[test]
    fn a_known_host_on_a_port_runs_ssh_with_p() {
        let terminal = Terminal {
            program: "alacritty".to_owned(),
            prefix_args: Vec::new(),
        };
        let entry = known_host_entry(
            KnownHost {
                host: "git.example.com".to_owned(),
                port: Some(2222),
            },
            &terminal,
        );
        assert_eq!(entry.label, "git.example.com:2222");
        match entry.payload {
            Payload::Exec { program, args } => {
                assert_eq!(program, "alacritty");
                assert_eq!(args, ["-e", "ssh", "-p", "2222", "git.example.com"]);
            }
            other => panic!("expected Exec, got {other:?}"),
        }
        let ssh = ["-p".to_owned(), "2222".to_owned(), "h".to_owned()];
        assert_eq!(build_terminal_args("pwsh", &ssh)[2], "ssh '-p' '2222' 'h'");
        assert_eq!(
            build_terminal_args("cmd", &ssh),
            ["/K", "ssh", "-p", "2222", "h"]
        );
        assert_eq!(
            build_terminal_args("wt", &ssh),
            ["--", "ssh", "-p", "2222", "h"]
        );
    }

    #[test]
    fn config_words_follow_ssh_quoting_and_comments() {
        assert_eq!(config_words("web # prod box"), ["web"]);
        assert_eq!(config_words("a b\tc"), ["a", "b", "c"]);
        assert_eq!(
            config_words("\"My Configs/hosts\" other"),
            ["My Configs/hosts", "other"]
        );
        assert_eq!(config_words("\"has # inside\""), ["has # inside"]);
        assert_eq!(config_words("\"say \\\"hi\\\"\""), ["say \"hi\""]);
        assert_eq!(config_words("not#comment"), ["not#comment"]);
        assert!(config_words("# all comment").is_empty());
        assert!(config_words("").is_empty());
    }

    #[test]
    fn trailing_comments_are_not_hosts_or_values() {
        let input = "Host web # prod box\n    HostName example.com # eu\n    User bob # me\n";
        let entries = parse_config(input, &Terminal::simple("alacritty"));
        assert_eq!(labels(&entries), ["web"]);
        assert_eq!(entries[0].description.as_deref(), Some("example.com (bob)"));
    }

    #[test]
    fn a_quoted_include_path_may_hold_spaces() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("My Configs")).unwrap();
        std::fs::write(dir.path().join("My Configs/hosts"), "Host spaced\n").unwrap();
        let config = dir.path().join("config");
        std::fs::write(&config, "Include \"My Configs/hosts\"\n").unwrap();
        let entries = read_config(
            &config,
            dir.path(),
            None,
            &Terminal::simple("alacritty"),
            0,
            &mut Default::default(),
        );
        assert_eq!(labels(&entries), ["spaced"]);
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

    /// File names `pattern` expands to in `dir`.
    #[cfg(unix)]
    fn globbed(dir: &Path, pattern: &str) -> Vec<String> {
        glob_paths(&dir.join(pattern))
            .iter()
            .map(|path| path.file_name().unwrap().to_str().unwrap().to_owned())
            .collect()
    }

    // Unix only: Windows forbids `*` in a file name, and glibc is the
    // reference.
    #[cfg(unix)]
    #[test]
    fn glob_paths_agree_with_glibc() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "a.conf", "b.conf", "c.conf", "B.conf", "1.conf", ".h.conf", "]x", "-x", "!x", "a-b",
            "a[b", "x*y",
        ] {
            std::fs::write(dir.path().join(name), "").unwrap();
        }
        // What glibc's glob(3), which OpenSSH calls, returned for each
        // pattern over the same files (LC_ALL=C, `.` and `..` left out).
        let glibc = [
            ("[ab].conf", "a.conf b.conf"),
            ("[!a].conf", "1.conf B.conf b.conf c.conf"),
            ("[^a].conf", "1.conf B.conf b.conf c.conf"),
            ("[a-c].conf", "a.conf b.conf c.conf"),
            ("[!a-c].conf", "1.conf B.conf"),
            ("[]]x", "]x"),
            ("[!]]x", "!x -x"),
            ("a[-]b", "a-b"),
            ("[-]x", "-x"),
            ("[!x]x", "!x -x ]x"),
            (
                "*",
                "!x -x 1.conf B.conf ]x a-b a.conf a[b b.conf c.conf x*y",
            ),
            (".*", ".h.conf"),
            ("[.]h.conf", ""),
            ("a\\[b", "a[b"),
            ("x\\*y", "x*y"),
            ("[0-9]*", "1.conf"),
            ("[A-Z]*", "B.conf"),
            ("*[!f]", "!x -x ]x a-b a[b x*y"),
        ];
        for (pattern, want) in glibc {
            assert_eq!(globbed(dir.path(), pattern).join(" "), want, "{pattern}");
        }
        // No wildcard: the path itself, whether or not it exists.
        assert_eq!(globbed(dir.path(), "a[b"), ["a[b"]);
        assert_eq!(globbed(dir.path(), "missing"), ["missing"]);
    }

    #[test]
    fn glob_paths_expand_directory_components_in_byte_order() {
        let dir = tempfile::tempdir().unwrap();
        for sub in ["a", "a-b", "skip"] {
            std::fs::create_dir(dir.path().join(sub)).unwrap();
        }
        std::fs::write(dir.path().join("a/x.conf"), "").unwrap();
        std::fs::write(dir.path().join("a-b/x.conf"), "").unwrap();
        std::fs::write(dir.path().join("a-b/y.txt"), "").unwrap();
        std::fs::write(dir.path().join("skip/x.conf"), "").unwrap();
        std::fs::write(dir.path().join("afile"), "").unwrap();
        let found: Vec<PathBuf> = glob_paths(&dir.path().join("a*/*.conf"))
            .iter()
            .map(|path| path.strip_prefix(dir.path()).unwrap().to_path_buf())
            .collect();
        // strcmp order: `-` sorts before `/`, so a-b/… comes first.
        assert_eq!(
            found,
            [PathBuf::from("a-b/x.conf"), PathBuf::from("a/x.conf")]
        );
    }

    #[test]
    fn include_words_expand_tokens_and_variables_or_are_skipped() {
        let home = Path::new("/home/u");
        let env = |name: &str| (name == "CONF").then(|| "/etc/conf".to_owned());
        let expand = |word| expand_include_word(word, Some(home), env);
        assert_eq!(expand("plain").as_deref(), Some("plain"));
        assert_eq!(expand("%d/.ssh/x").as_deref(), Some("/home/u/.ssh/x"));
        assert_eq!(expand("100%%").as_deref(), Some("100%"));
        assert_eq!(expand("${CONF}/a").as_deref(), Some("/etc/conf/a"));
        assert_eq!(expand("$CONF").as_deref(), Some("$CONF"));
        assert_eq!(expand("${UNSET}/a"), None);
        assert_eq!(expand("${CONF"), None);
        assert_eq!(expand("hosts/%h"), None);
        assert_eq!(expand("%u.conf"), None);
        assert_eq!(expand("trailing%"), None);
        assert_eq!(expand_include_word("%d", None, env), None);
    }

    #[test]
    fn include_follows_a_home_token() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("conf.d")).unwrap();
        std::fs::write(dir.path().join("conf.d/a.conf"), "Host viatoken\n").unwrap();
        let config = dir.path().join("config");
        std::fs::write(&config, "Include %d/conf.d/[a-z].conf\n").unwrap();
        let entries = read_config(
            &config,
            Path::new("/nonexistent"),
            Some(dir.path()),
            &Terminal::simple("alacritty"),
            0,
            &mut Default::default(),
        );
        assert_eq!(labels(&entries), ["viatoken"]);
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
        let entries = read_config(
            &config,
            dir.path(),
            None,
            &Terminal::simple("alacritty"),
            0,
            &mut Default::default(),
        );
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
            &mut Default::default(),
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
        let entries = read_config(
            &config,
            dir.path(),
            None,
            &Terminal::simple("alacritty"),
            0,
            &mut Default::default(),
        );
        assert_eq!(labels(&entries), ["parent", "child"]);
        assert_eq!(entries[0].description.as_deref(), Some("parent.example"));
    }

    #[test]
    fn a_config_including_itself_is_read_once() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config");
        std::fs::write(&config, "Host loop\nInclude config\n").unwrap();
        let entries = read_config(
            &config,
            dir.path(),
            None,
            &Terminal::simple("alacritty"),
            0,
            &mut Default::default(),
        );
        assert_eq!(labels(&entries), ["loop"]);
    }

    #[test]
    fn files_including_each_other_are_each_read_once() {
        // Each file includes the whole directory, itself and the others:
        // without reading each file once this is 3^16 reads.
        let dir = tempfile::tempdir().unwrap();
        let conf_d = dir.path().join("config.d");
        std::fs::create_dir(&conf_d).unwrap();
        for name in ["a", "b", "c"] {
            std::fs::write(
                conf_d.join(name),
                format!("Host {name}\nInclude config.d/*\n"),
            )
            .unwrap();
        }
        let config = dir.path().join("config");
        std::fs::write(&config, "Include config.d/*\n").unwrap();
        let entries = read_config(
            &config,
            dir.path(),
            None,
            &Terminal::simple("alacritty"),
            0,
            &mut Default::default(),
        );
        assert_eq!(labels(&entries), ["a", "b", "c"]);
    }
}
