//! drun mode — XDG `.desktop` application launcher.

use super::{Entry, Mode};
use anyhow::Result;
use freedesktop_desktop_entry::{DesktopEntry, Iter, default_paths};
use std::collections::HashMap;

#[derive(Default)]
pub struct Drun;

impl Mode for Drun {
    fn name(&self) -> &'static str {
        "drun"
    }

    fn collect(&mut self) -> Result<Vec<Entry>> {
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
fn parse_exec(raw: &str) -> Option<(String, Vec<String>)> {
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
fn strip_field_codes(tok: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

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
