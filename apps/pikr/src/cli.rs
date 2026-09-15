//! CLI surface.

use clap::error::ErrorKind;
use clap::{ArgAction, CommandFactory, Parser, ValueEnum};
use std::path::PathBuf;

pub use crate::picker::keyspec::KbCustom;
pub use crate::picker::state::VimMode;

/// Exit code for the first `--kb-custom` binding; the Nth binding exits
/// with `KB_CUSTOM_EXIT_BASE + N - 1`. Matches rofi's `-kb-custom-N`.
pub const KB_CUSTOM_EXIT_BASE: i32 = 10;

/// Upper bound on `--kb-custom` bindings (exit codes 10–28, as in rofi).
pub const KB_CUSTOM_MAX: usize = 19;

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

    /// dmenu mode: an alternate key that accepts the highlighted row. Prints
    /// the row like Enter, but exits 10 for the first binding, 11 for the
    /// second, and so on (rofi's `-kb-custom-N`), so the calling script can
    /// tell which key was used. Repeatable, up to 19. KEY is a chord such as
    /// `Shift+Delete`, `Ctrl+d`, `Alt+Right` or `F2`; bindings take
    /// precedence over the built-in keymap. `KEY=PROMPT` shows PROMPT in a
    /// confirm card on the highlighted row first: Enter accepts, Esc or Left
    /// dismisses. Left/Right/Home/End bindings fire only when the query
    /// caret can't move that way, so they don't steal caret movement.
    #[arg(
        long = "kb-custom",
        value_name = "KEY[=PROMPT]",
        action = ArgAction::Append
    )]
    pub kb_custom: Vec<KbCustom>,

    /// dmenu mode: open the window immediately and show TEXT where the list
    /// goes while stdin is still being written, e.g. `--loading Scanning…`.
    /// Rows appear once stdin closes; accepting is disabled until then.
    /// Without it pikr reads all of stdin before opening.
    #[arg(long = "loading", value_name = "TEXT")]
    pub loading: Option<String>,
}

impl Cli {
    /// The mode pikr opens in: `--dmenu` wins over `--show`.
    pub fn chosen_mode(&self) -> Mode {
        if self.dmenu { Mode::Dmenu } else { self.show }
    }

    /// Cross-argument checks the derive can't express. `requires = "dmenu"`
    /// would reject `--show dmenu`, so the dmenu-only flags are checked here
    /// against the chosen mode. Failures are clap errors: `.exit()` prints
    /// them with usage and exits 2, like any other bad argument.
    pub fn validate(self) -> Result<Self, clap::Error> {
        let fail = |kind: ErrorKind, msg: String| Err(Cli::command().error(kind, msg));

        if self.chosen_mode() != Mode::Dmenu {
            if !self.kb_custom.is_empty() {
                return fail(
                    ErrorKind::MissingRequiredArgument,
                    "--kb-custom requires --dmenu or --show dmenu".into(),
                );
            }
            if self.loading.is_some() {
                return fail(
                    ErrorKind::MissingRequiredArgument,
                    "--loading requires --dmenu or --show dmenu".into(),
                );
            }
        }

        if self.kb_custom.len() > KB_CUSTOM_MAX {
            return fail(
                ErrorKind::TooManyValues,
                format!(
                    "--kb-custom: at most {KB_CUSTOM_MAX} bindings (exit codes {}–{})",
                    KB_CUSTOM_EXIT_BASE,
                    KB_CUSTOM_EXIT_BASE + KB_CUSTOM_MAX as i32 - 1,
                ),
            );
        }

        for (i, binding) in self.kb_custom.iter().enumerate() {
            let key = &binding.key;
            if let Some(earlier) = self.kb_custom[..i].iter().find(|e| e.key.overlaps(key)) {
                let msg = if earlier.key == *key {
                    format!("--kb-custom: `{key}` is bound more than once")
                } else {
                    format!(
                        "--kb-custom: `{}` and `{key}` match the same key press",
                        earlier.key
                    )
                };
                return fail(ErrorKind::ValueValidation, msg);
            }
        }

        Ok(self)
    }
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

impl Mode {
    /// Lowercase the Debug repr — the key under which frecency/history store
    /// this mode, matching the status bar's mode-name label and staying
    /// stable across `Mode` reorderings.
    pub(crate) fn key(self) -> String {
        format!("{self:?}").to_lowercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Cli {
        try_parse(args).unwrap()
    }

    /// Parse plus [`Cli::validate`], as `main` does.
    fn try_parse(args: &[&str]) -> Result<Cli, clap::Error> {
        let mut full = vec!["pikr"];
        full.extend_from_slice(args);
        Cli::try_parse_from(full).and_then(Cli::validate)
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

    // ── --kb-custom ───────────────────────────────────────────────────────

    #[test]
    fn kb_custom_repeatable_in_order() {
        let cli = parse(&["-d", "--kb-custom", "Shift+Delete", "--kb-custom", "Ctrl+d"]);
        let want: Vec<KbCustom> = ["Shift+Delete", "Ctrl+d"]
            .iter()
            .map(|s| s.parse().unwrap())
            .collect();
        assert_eq!(cli.kb_custom, want);
    }

    #[test]
    fn kb_custom_with_confirm_prompt() {
        let cli = parse(&["-d", "--kb-custom", "Right=Forget?"]);
        assert_eq!(cli.kb_custom[0].confirm.as_deref(), Some("Forget?"));
    }

    #[test]
    fn loading_text() {
        let cli = parse(&["-d", "--loading", "Scanning…"]);
        assert_eq!(cli.loading.as_deref(), Some("Scanning…"));
    }

    #[test]
    fn loading_requires_dmenu() {
        let err = try_parse(&["--loading", "x"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MissingRequiredArgument);
        assert_eq!(err.exit_code(), 2);
        let err = try_parse(&["--show", "drun", "--loading", "x"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn kb_custom_default_empty() {
        let cli = parse(&["-d"]);
        assert!(cli.kb_custom.is_empty());
    }

    #[test]
    fn kb_custom_requires_dmenu() {
        let err = try_parse(&["--kb-custom", "Shift+Delete"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::MissingRequiredArgument);
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn dmenu_flags_accept_show_dmenu() {
        let cli = parse(&["--show", "dmenu", "--kb-custom", "F2", "--loading", "…"]);
        assert_eq!(cli.chosen_mode(), Mode::Dmenu);
        assert_eq!(cli.kb_custom.len(), 1);
        assert_eq!(cli.loading.as_deref(), Some("…"));
    }

    #[test]
    fn kb_custom_rejects_bad_key() {
        assert!(try_parse(&["-d", "--kb-custom", "Shift+Nope"]).is_err());
    }

    /// `-d` plus `n` distinct bindings: `Alt+F1`…`Alt+F12`, then `Ctrl+F1`….
    fn with_bindings(n: usize) -> Result<Cli, clap::Error> {
        let keys: Vec<String> = (0..n)
            .map(|i| format!("{}+F{}", if i < 12 { "Alt" } else { "Ctrl" }, i % 12 + 1))
            .collect();
        let mut args = vec!["-d"];
        for k in &keys {
            args.extend(["--kb-custom", k.as_str()]);
        }
        try_parse(&args)
    }

    #[test]
    fn kb_custom_limit_is_19() {
        assert_eq!(with_bindings(19).unwrap().kb_custom.len(), 19);
        let err = with_bindings(20).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::TooManyValues);
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn kb_custom_rejects_duplicate_binding() {
        let err =
            try_parse(&["-d", "--kb-custom", "Ctrl+d", "--kb-custom", "ctrl+D=Sure?"]).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ValueValidation);
        assert!(
            err.to_string().contains("`Ctrl+d` is bound more than once"),
            "{err}"
        );
        // A different modifier set is a different binding.
        assert!(try_parse(&["-d", "--kb-custom", "Ctrl+d", "--kb-custom", "Alt+d"]).is_ok());
    }

    #[test]
    fn kb_custom_rejects_overlapping_symbol_bindings() {
        let err =
            try_parse(&["-d", "--kb-custom", "Ctrl+?", "--kb-custom", "Ctrl+Shift+?"]).unwrap_err();
        assert!(
            err.to_string()
                .contains("`Ctrl+?` and `Ctrl+Shift+?` match the same key press"),
            "{err}"
        );
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
