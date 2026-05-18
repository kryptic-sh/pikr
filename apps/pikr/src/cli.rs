//! CLI surface.

use clap::{Parser, ValueEnum};
use std::path::PathBuf;

pub use crate::picker::state::VimMode;

#[derive(Parser, Debug, Clone)]
#[command(
    name = "pikr",
    version,
    about = "Vim-modal picker / launcher",
    long_about = None,
)]
pub struct Cli {
    /// Mode to launch in. With --dmenu this is ignored.
    /// Available: drun, run, dmenu, ssh, emoji, clipboard, calc.
    #[arg(short = 's', long = "show", value_enum, default_value = "drun")]
    pub show: Mode,

    /// dmenu mode: read entries from stdin, print selection to stdout.
    #[arg(short = 'd', long = "dmenu")]
    pub dmenu: bool,

    /// Prompt text shown in the input row.
    #[arg(short = 'p', long = "prompt")]
    pub prompt: Option<String>,

    /// Override config file path.
    #[arg(long = "config")]
    pub config: Option<PathBuf>,

    /// Vim mode to start in. Defaults to `insert` so the user can begin
    /// typing a query immediately on launch. Values derive from `VimMode`
    /// so new variants are exposed automatically.
    #[arg(long = "mode", value_enum, default_value_t = VimMode::Insert)]
    pub mode: VimMode,

    /// Mask the query bar with ● (U+25CF) per character. Real chars still
    /// drive the matcher; the payload on accept is still the typed text.
    #[arg(short = 'P', long = "password")]
    pub password: bool,

    /// Pre-fill the query string on launch. Sets the initial query and moves
    /// the cursor to the end of the prefill text before the first rerank.
    #[arg(long = "filter", aliases = ["query", "prefill", "input-text"])]
    pub filter: Option<String>,

    /// Non-interactive message modal. Renders only the message text — no
    /// input, no result list. Escape dismisses. When set, --show/--dmenu/
    /// --filter are ignored.
    #[arg(short = 'e', long = "message")]
    pub message: Option<String>,

    /// Override the window width in pixels (integer only; % syntax not
    /// supported in v1). Defaults to 720.
    #[arg(long = "width")]
    pub width: Option<u32>,

    /// Override the number of visible result rows (replaces VISIBLE_ROWS in
    /// the window height calculation). Defaults to 8.
    #[arg(short = 'l', long = "lines")]
    pub lines: Option<usize>,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Drun,
    Run,
    Dmenu,
    Ssh,
    Emoji,
    Clipboard,
    Calc,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Cli {
        let mut full = vec!["pikr"];
        full.extend_from_slice(args);
        Cli::parse_from(full)
    }

    // ── --password / -P ───────────────────────────────────────────────────

    #[test]
    fn password_long_flag() {
        let cli = parse(&["--password"]);
        assert!(cli.password);
    }

    #[test]
    fn password_short_flag() {
        let cli = parse(&["-P"]);
        assert!(cli.password);
    }

    #[test]
    fn password_default_false() {
        let cli = parse(&[]);
        assert!(!cli.password);
    }

    // ── --filter and aliases ───────────────────────────────────────────────

    #[test]
    fn filter_long() {
        let cli = parse(&["--filter", "hello"]);
        assert_eq!(cli.filter.as_deref(), Some("hello"));
    }

    #[test]
    fn filter_alias_query() {
        let cli = parse(&["--query", "world"]);
        assert_eq!(cli.filter.as_deref(), Some("world"));
    }

    #[test]
    fn filter_alias_prefill() {
        let cli = parse(&["--prefill", "foo"]);
        assert_eq!(cli.filter.as_deref(), Some("foo"));
    }

    #[test]
    fn filter_alias_input_text() {
        let cli = parse(&["--input-text", "bar"]);
        assert_eq!(cli.filter.as_deref(), Some("bar"));
    }

    #[test]
    fn filter_default_none() {
        let cli = parse(&[]);
        assert!(cli.filter.is_none());
    }

    // ── --message / -e ────────────────────────────────────────────────────

    #[test]
    fn message_long() {
        let cli = parse(&["--message", "hi there"]);
        assert_eq!(cli.message.as_deref(), Some("hi there"));
    }

    #[test]
    fn message_short() {
        let cli = parse(&["-e", "error!"]);
        assert_eq!(cli.message.as_deref(), Some("error!"));
    }

    #[test]
    fn message_default_none() {
        let cli = parse(&[]);
        assert!(cli.message.is_none());
    }

    // ── --width / --lines ─────────────────────────────────────────────────

    #[test]
    fn width_long() {
        let cli = parse(&["--width", "1024"]);
        assert_eq!(cli.width, Some(1024));
    }

    #[test]
    fn width_default_none() {
        let cli = parse(&[]);
        assert!(cli.width.is_none());
    }

    #[test]
    fn lines_long() {
        let cli = parse(&["--lines", "12"]);
        assert_eq!(cli.lines, Some(12));
    }

    #[test]
    fn lines_short() {
        let cli = parse(&["-l", "5"]);
        assert_eq!(cli.lines, Some(5));
    }

    #[test]
    fn lines_default_none() {
        let cli = parse(&[]);
        assert!(cli.lines.is_none());
    }

    // ── Combined flags ────────────────────────────────────────────────────

    #[test]
    fn filter_and_password_together() {
        let cli = parse(&["--filter", "secret", "--password"]);
        assert_eq!(cli.filter.as_deref(), Some("secret"));
        assert!(cli.password);
    }

    #[test]
    fn width_and_lines_together() {
        let cli = parse(&["--width", "800", "--lines", "10"]);
        assert_eq!(cli.width, Some(800));
        assert_eq!(cli.lines, Some(10));
    }
}
