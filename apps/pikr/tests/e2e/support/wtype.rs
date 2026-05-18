//! Thin wrapper around the `wtype` CLI for driving keyboard input into the
//! sway headless session.

use super::sway::Sway;
use std::process::Command;
use std::time::Duration;

/// Named keys for `wtype -k`. Extend the enum as new tests need additional
/// keys; the args() match arm is the only place that needs to know about
/// each variant.
#[derive(Debug, Clone)]
pub enum Key {
    Escape,
    Return,
    /// Shift+Return (AcceptCustom in pikr dmenu).
    ShiftReturn,
}

impl Key {
    /// Returns the wtype argument fragment for this key.
    ///
    /// `wtype -k Escape` = press-and-release named key.
    /// `Shift+Return` needs `-M shift -k Return -m shift`.
    fn args(&self) -> Vec<&'static str> {
        match self {
            Key::Escape => vec!["-k", "Escape"],
            Key::Return => vec!["-k", "Return"],
            Key::ShiftReturn => vec!["-M", "shift", "-k", "Return", "-m", "shift"],
        }
    }
}

/// Builder for a wtype invocation.
pub struct Wtype<'a> {
    sway: &'a Sway,
    args: Vec<String>,
}

impl<'a> Wtype<'a> {
    pub fn new(sway: &'a Sway) -> Self {
        Wtype {
            sway,
            args: Vec::new(),
        }
    }

    /// Insert a sleep (milliseconds) before the following events.
    pub fn delay(mut self, d: Duration) -> Self {
        self.args.push("-s".to_owned());
        self.args.push(d.as_millis().to_string());
        self
    }

    /// Type a literal text string.
    pub fn text(mut self, s: &str) -> Self {
        self.args.push(s.to_owned());
        self
    }

    /// Append named key presses.
    pub fn keys(mut self, keys: &[Key]) -> Self {
        for k in keys {
            for arg in k.args() {
                self.args.push(arg.to_owned());
            }
        }
        self
    }

    /// Execute the wtype command, propagating the wayland env vars from `sway`.
    ///
    /// Panics if wtype exits non-zero or cannot be spawned.
    pub fn send(self) -> Result<(), String> {
        let status = Command::new("wtype")
            .args(&self.args)
            .envs(self.sway.env_vars())
            .status()
            .map_err(|e| format!("wtype spawn failed: {e}"))?;

        if status.success() {
            Ok(())
        } else {
            Err(format!("wtype exited with {status}"))
        }
    }
}
