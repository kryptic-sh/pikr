//! Modes — sources of selectable entries.
//!
//! A `Mode` produces a `Vec<Entry>`. Each `Entry` carries a `Payload` that
//! describes what to do when it is selected. Execution is mode-agnostic and
//! handled by [`execute`] — modes never run code themselves.

use anyhow::{Context, Result};
use std::process::{Command, Stdio};

pub mod calc;
pub mod clipboard;
pub mod dmenu;
// drun: XDG `.desktop` launcher on Unix; Start Menu `.lnk` walker on Windows.
pub mod drun;
pub mod emoji;
// run walks $PATH (Unix: executable-bit filter; Windows: PATHEXT extension
// filter). Cross-platform; internals are cfg-gated per OS.
pub mod run;
// ssh works cross-platform: reads ~/.ssh/config on all OSes, probes OS-native
// terminal emulators (unix: alacritty/kitty/foot/xterm; Windows: wt/pwsh/cmd).
pub mod ssh;

/// One selectable row.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Primary label shown in the picker list.
    pub label: String,
    /// Optional sub-text shown on the right of the row.
    pub description: Option<String>,
    /// Optional icon: either a freedesktop icon name (e.g. `"firefox"`) or
    /// an absolute path. Resolved at render time via `IconCache`.
    pub icon: Option<String>,
    /// What happens on accept.
    pub payload: Payload,
}

impl Entry {
    pub fn stdout(label: impl Into<String>) -> Self {
        let label = label.into();
        Self {
            payload: Payload::Stdout(label.clone()),
            label,
            description: None,
            icon: None,
        }
    }

    pub fn exec(label: impl Into<String>, program: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            description: None,
            icon: None,
            payload: Payload::Exec {
                program: program.into(),
                args: Vec::new(),
            },
        }
    }

    pub fn with_description(mut self, d: impl Into<String>) -> Self {
        self.description = Some(d.into());
        self
    }

    pub fn with_args(mut self, args: Vec<String>) -> Self {
        if let Payload::Exec { args: a, .. } = &mut self.payload {
            *a = args;
        }
        self
    }

    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }
}

#[derive(Debug, Clone)]
pub enum Payload {
    /// Print this string to stdout.
    Stdout(String),
    /// Spawn `program` with `args` detached from pikr.
    Exec { program: String, args: Vec<String> },
    /// Write a string directly to the system clipboard (no subprocess).
    SetClipboard(String),
}

pub trait Mode {
    fn name(&self) -> &'static str;
    fn collect(&mut self) -> Result<Vec<Entry>>;
}

/// Execute a selected entry's payload. The runtime calls this once after the
/// user accepts; the mode is no longer involved.
pub fn execute(payload: &Payload) -> Result<()> {
    match payload {
        Payload::Stdout(s) => {
            println!("{s}");
            Ok(())
        }
        Payload::Exec { program, args } => spawn_detached(program, args),
        Payload::SetClipboard(text) => set_clipboard(text),
    }
}

/// Write `text` to the system clipboard.
///
/// On Unix the clipboard write is a no-op here: the unix clipboard mode
/// shells out to `cliphist decode | wl-copy` via `Payload::Exec`, so
/// `Payload::SetClipboard` is never emitted on that platform. The body
/// below is only reachable on Windows (and other non-unix targets) where
/// `arboard` provides the implementation.
fn set_clipboard(text: &str) -> Result<()> {
    #[cfg(unix)]
    {
        // Unreachable on Unix; unix clipboard mode uses Payload::Exec.
        let _ = text;
        Ok(())
    }
    #[cfg(windows)]
    {
        let mut cb = arboard::Clipboard::new().with_context(|| "open system clipboard")?;
        cb.set_text(text)
            .with_context(|| "write to system clipboard")?;
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = text;
        Ok(())
    }
}

/// Spawn `program` with `args` so the child survives pikr exiting and never
/// inherits pikr's stdio.
fn spawn_detached(program: &str, args: &[String]) -> Result<()> {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("spawn {program}"))?;
    Ok(())
}
