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

    /// Exec entry with every field supplied up front — the four drun
    /// construction sites (unix collect, unix/windows cache restore,
    /// windows `parse_lnk`) all followed `exec(...).with_args(...)` with the
    /// same conditional description/icon dance; this collapses it to one call.
    pub fn exec_with(
        label: impl Into<String>,
        program: impl Into<String>,
        args: Vec<String>,
        description: Option<String>,
        icon: Option<String>,
    ) -> Self {
        Self {
            label: label.into(),
            description,
            icon,
            payload: Payload::Exec {
                program: program.into(),
                args,
            },
        }
    }

    /// `ExecWait` twin of [`Entry::exec`] — a must-succeed short pipeline
    /// (the clipboard's `cliphist decode | wl-copy`) whose exit status the
    /// accept path must observe. Args are supplied via [`Entry::with_args`].
    pub fn exec_wait(label: impl Into<String>, program: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            description: None,
            icon: None,
            payload: Payload::ExecWait {
                program: program.into(),
                args: Vec::new(),
            },
        }
    }

    pub fn with_args(mut self, args: Vec<String>) -> Self {
        match &mut self.payload {
            Payload::Exec { args: a, .. } | Payload::ExecWait { args: a, .. } => *a = args,
            _ => {}
        }
        self
    }
}

#[derive(Debug, Clone)]
pub enum Payload {
    /// Print this string to stdout.
    Stdout(String),
    /// Spawn `program` with `args` detached from pikr.
    Exec { program: String, args: Vec<String> },
    /// Spawn `program` with `args` and wait for it to exit, returning an
    /// error if the child fails or exits non-zero. Used for short,
    /// must-succeed pipelines (the clipboard's `cliphist decode | wl-copy`)
    /// where a detached spawn would silently drop a missing `wl-copy`.
    ExecWait { program: String, args: Vec<String> },
    /// Write a string directly to the system clipboard (no subprocess).
    /// Constructed only by the Windows clipboard mode (`modes/clipboard.rs`
    /// `windows_impl`); it must exist on every platform because `execute` and
    /// the frecency `payload_key` match it exhaustively. On non-Windows
    /// builds nothing constructs it, so silence the dead-code lint there.
    #[cfg_attr(not(windows), allow(dead_code))]
    SetClipboard(String),
}

pub trait Mode {
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
        Payload::ExecWait { program, args } => spawn_and_wait(program, args),
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

/// Spawn `program` with `args` and wait for it to exit, returning an error
/// when the child fails to start or exits non-zero.
///
/// The clipboard accept path uses this for its `sh -c "cliphist decode | wl-copy"`
/// pipeline: a missing `wl-copy` (or a decode failure) otherwise fails
/// silently — the user selects an entry and nothing reaches the clipboard.
/// Stderr is inherited so the child's own error message ("wl-copy: command
/// not found") shows in the terminal that launched pikr; stdout is nulled —
/// the pipeline's stdout is nothing, and a stray child must not print into
/// the launcher's terminal.
fn spawn_and_wait(program: &str, args: &[String]) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()
        .with_context(|| format!("spawn {program}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("{program} exited with {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::{Payload, spawn_and_wait};

    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn spawn_and_wait_ok_on_success() {
        let r = spawn_and_wait("sh", &args(&["-c", "exit 0"]));
        assert!(r.is_ok(), "exit 0 must succeed: {r:?}");
    }

    #[test]
    fn spawn_and_wait_errs_on_nonzero_exit() {
        // The clipboard pipe's failure mode: wl-copy missing → sh exits 127.
        let r = spawn_and_wait("sh", &args(&["-c", "exit 3"]));
        let err = r.expect_err("non-zero exit must surface as an error");
        assert!(err.to_string().contains("exited with"), "got: {err}");
    }

    #[test]
    fn spawn_and_wait_errs_on_missing_program() {
        let r = spawn_and_wait("no-such-binary-xyz-12345", &[]);
        assert!(r.is_err(), "missing program must fail, not silently pass");
    }

    #[test]
    fn execwait_parity_with_exec_in_payload_key() {
        // ExecWait and Exec with identical program+args must share one usage
        // key (frecency): the clipboard's sh -c pipeline and a plain exec of
        // the same command are the "same" launch for ranking purposes.
        let exec = Payload::Exec {
            program: "sh".into(),
            args: args(&["-c", "cliphist decode 42 | wl-copy"]),
        };
        let wait = Payload::ExecWait {
            program: "sh".into(),
            args: args(&["-c", "cliphist decode 42 | wl-copy"]),
        };
        let exec_key =
            crate::picker::frecency::entry_keys(&[std::sync::Arc::new(crate::modes::Entry {
                label: "x".into(),
                description: None,
                icon: None,
                payload: exec,
            })]);
        let wait_key =
            crate::picker::frecency::entry_keys(&[std::sync::Arc::new(crate::modes::Entry {
                label: "x".into(),
                description: None,
                icon: None,
                payload: wait,
            })]);
        assert_eq!(exec_key, wait_key);
    }
}
