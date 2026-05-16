//! ssh mode — SSH host picker from `~/.ssh/config` and `/etc/ssh/ssh_config`.

use super::{Entry, Mode, Payload};
use anyhow::Result;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct Ssh;

impl Mode for Ssh {
    fn name(&self) -> &'static str {
        "ssh"
    }

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
    if let Some(home) = dirs_home() {
        paths.push(home.join(".ssh").join("config"));
    }
    paths.push(PathBuf::from("/etc/ssh/ssh_config"));
    paths
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// Walk the ssh config text, returning one Entry per non-wildcard Host.
pub fn parse_config(text: &str, terminal: &str) -> Vec<Entry> {
    let mut entries = Vec::new();
    let mut current_host: Option<String> = None;
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
            if let Some(host) = current_host.take() {
                entries.push(make_entry(host, hostname.take(), user.take(), terminal));
            }
            // Wildcard patterns are not real hosts — skip them.
            if value.contains('*') || value.contains('?') {
                current_host = None;
            } else {
                current_host = Some(value);
            }
        } else if current_host.is_some() {
            match key.as_str() {
                "hostname" => hostname = Some(value),
                "user" => user = Some(value),
                _ => {}
            }
        }
    }
    // Flush the final block.
    if let Some(host) = current_host {
        entries.push(make_entry(host, hostname, user, terminal));
    }
    entries
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
    let args = vec!["-e".to_string(), "ssh".to_string(), host.clone()];
    Entry {
        label: host,
        description,
        payload: Payload::Exec {
            program: terminal.to_string(),
            args,
        },
    }
}

/// Find a usable terminal emulator: $TERMINAL env first, then well-known names.
fn resolve_terminal() -> String {
    if let Ok(t) = std::env::var("TERMINAL")
        && !t.is_empty()
        && which(&t).is_some()
    {
        return t;
    }
    for name in ["alacritty", "kitty", "foot", "xterm"] {
        if which(name).is_some() {
            return name.to_string();
        }
    }
    // Last resort: return "xterm" even if not found; the exec will fail with a
    // clear error rather than silently doing nothing.
    "xterm".to_string()
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn is_executable(path: &Path) -> bool {
    path.metadata()
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
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
}
