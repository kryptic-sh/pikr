//! pikr process fixture.

use super::sway::Sway;
use std::io::{Read as _, Write as _};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// What pikr logs at debug level once the window system has given it the
/// keyboard: keys sent before it go nowhere.
const FOCUS_MARKER: &str = "startup first focus received";

/// Outcome of a pikr run.
pub struct Outcome {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// A running pikr process.
pub struct Pikr {
    child: Child,
    /// Everything pikr has written to stderr so far.
    stderr: Arc<Mutex<String>>,
    /// The thread copying pikr's stderr into `stderr`; it ends at EOF.
    stderr_reader: Option<JoinHandle<()>>,
}

impl Pikr {
    /// Spawn pikr with the given args inside the provided sway session.
    ///
    /// `stdin_data`: optional bytes to pipe to pikr's stdin (used by
    /// `--dmenu` to feed candidates).
    pub fn spawn(sway: &Sway, args: &[&str], stdin_data: Option<&str>) -> Result<Self, String> {
        Self::spawn_with_env(sway, args, stdin_data, &[])
    }

    /// Spawn pikr with additional environment variables.
    ///
    /// Every pikr runs with isolated `XDG_CONFIG_HOME`/`XDG_STATE_HOME`
    /// pointing into the sway fixture's tempdir, so tests never read the
    /// developer's real `~/.config/pikr/config.toml` nor write their real
    /// `~/.local/state/pikr/history.toml`/`usage.toml`. It also logs at
    /// debug level without colour, so [`Pikr::wait_with_retry`] can see
    /// [`FOCUS_MARKER`]. Env vars in `env` override these defaults.
    pub fn spawn_with_env(
        sway: &Sway,
        args: &[&str],
        stdin_data: Option<&str>,
        env: &[(&str, &str)],
    ) -> Result<Self, String> {
        let bin = pikr_bin();
        let mut cmd = Command::new(&bin);
        cmd.args(args)
            .envs(sway.env_vars())
            .env("XDG_CONFIG_HOME", sway.config_dir.to_str().unwrap())
            .env("XDG_STATE_HOME", sway.state_dir.to_str().unwrap())
            .env("RUST_LOG", "pikr=debug")
            .env("NO_COLOR", "1")
            .envs(env.iter().copied())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if stdin_data.is_some() {
            cmd.stdin(Stdio::piped());
        } else {
            cmd.stdin(Stdio::null());
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("failed to spawn pikr ({bin:?}): {e}"))?;

        if let Some(data) = stdin_data {
            let mut stdin = child.stdin.take().expect("stdin piped");
            stdin
                .write_all(data.as_bytes())
                .map_err(|e| format!("write to pikr stdin: {e}"))?;
            // stdin is dropped here → EOF sent.
        }

        let stderr = Arc::new(Mutex::new(String::new()));
        let mut pipe = child.stderr.take().expect("stderr piped");
        let sink = Arc::clone(&stderr);
        let stderr_reader = std::thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            while let Ok(n @ 1..) = pipe.read(&mut chunk) {
                sink.lock()
                    .unwrap()
                    .push_str(&String::from_utf8_lossy(&chunk[..n]));
            }
        });

        Ok(Pikr {
            child,
            stderr,
            stderr_reader: Some(stderr_reader),
        })
    }

    /// Whether pikr has logged that it has the keyboard focus.
    fn has_focus(&self) -> bool {
        self.stderr.lock().unwrap().contains(FOCUS_MARKER)
    }

    /// All of pikr's stderr, once it has exited and the pipe is drained.
    fn collected_stderr(&mut self) -> String {
        if let Some(reader) = self.stderr_reader.take() {
            let _ = reader.join();
        }
        std::mem::take(&mut *self.stderr.lock().unwrap())
    }

    /// Wait for the process to exit (up to `timeout`), invoking
    /// `send_keys` on every `retry_every` interval until then, starting once
    /// pikr reports the keyboard focus.
    ///
    /// Designed for tests that act on a single keystroke: CI's pixman +
    /// zink-fallback render path can drop the first input event if the
    /// layer-shell surface hasn't claimed focus yet, and a static
    /// "send keys, wait, hope" recipe flakes. Re-sending the keys on a
    /// retry cadence covers the race without bumping the warmup delay
    /// to absurd values. Once pikr exits the loop returns immediately.
    pub fn wait_with_retry<F>(
        mut self,
        timeout: Duration,
        retry_every: Duration,
        mut send_keys: F,
    ) -> Result<Outcome, String>
    where
        F: FnMut(),
    {
        let deadline = Instant::now() + timeout;
        let mut last_send = Instant::now()
            .checked_sub(retry_every)
            .unwrap_or_else(Instant::now);

        loop {
            if last_send.elapsed() >= retry_every && self.has_focus() {
                send_keys();
                last_send = Instant::now();
            }
            match self
                .child
                .try_wait()
                .map_err(|e| format!("try_wait: {e}"))?
            {
                Some(status) => {
                    let mut stdout = String::new();
                    if let Some(mut r) = self.child.stdout.take() {
                        let _ = r.read_to_string(&mut stdout);
                    }
                    return Ok(Outcome {
                        exit_code: status.code(),
                        stdout,
                        stderr: self.collected_stderr(),
                    });
                }
                None => {
                    if Instant::now() >= deadline {
                        let _ = self.child.kill();
                        let _ = self.child.wait();
                        let stderr = self.collected_stderr();
                        return Err(format!(
                            "pikr did not exit within {timeout:?}; stderr:\n{stderr}"
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
    }
}

impl Drop for Pikr {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ── binary resolution ─────────────────────────────────────────────────────────

static BIN: OnceLock<PathBuf> = OnceLock::new();

fn pikr_bin() -> PathBuf {
    BIN.get_or_init(|| {
        // The release binary next to the workspace Cargo.toml. Tests run from
        // the workspace root, so we can navigate from CARGO_MANIFEST_DIR
        // (apps/pikr) up two levels to the workspace root.
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let workspace = manifest
            .parent() // apps/
            .and_then(|p| p.parent()) // workspace root
            .expect("cannot resolve workspace root from CARGO_MANIFEST_DIR");

        // Where the build below puts it: it inherits `CARGO_TARGET_DIR`, and
        // runs in the workspace, which a relative one is resolved against.
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map_or_else(|| workspace.join("target"), |dir| workspace.join(dir));
        let release_bin = target.join("release/pikr");

        // Always build: an existing binary may predate the sources (a pull
        // since the last release build), and driving it reports its old
        // behaviour as a regression. Cargo makes this a no-op when current.
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let status = Command::new(cargo)
            .args(["build", "--release", "--bin", "pikr"])
            .current_dir(workspace)
            .status()
            .expect("cargo build --release --bin pikr failed to spawn");
        assert!(
            status.success(),
            "cargo build --release --bin pikr exited with {status}"
        );
        assert!(
            release_bin.exists(),
            "cargo build succeeded but {release_bin:?} still not found"
        );
        release_bin
    })
    .clone()
}
