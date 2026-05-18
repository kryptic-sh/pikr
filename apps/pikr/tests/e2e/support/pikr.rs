//! pikr process fixture.

use super::sway::Sway;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Outcome of a pikr run.
pub struct Outcome {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// A running pikr process.
pub struct Pikr {
    child: Child,
}

impl Pikr {
    /// Spawn pikr with the given args inside the provided sway session.
    ///
    /// `stdin_data`: optional bytes to pipe to pikr's stdin (used by
    /// `--dmenu` to feed candidates).
    pub fn spawn(sway: &Sway, args: &[&str], stdin_data: Option<&str>) -> Result<Self, String> {
        let bin = pikr_bin();
        let mut cmd = Command::new(&bin);
        cmd.args(args)
            .envs(sway.env_vars())
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

        Ok(Pikr { child })
    }

    /// Wait for the process to exit (up to `timeout`).
    ///
    /// Returns `Err` if the timeout is reached and the process is still alive.
    pub fn wait_timeout(mut self, timeout: Duration) -> Result<Outcome, String> {
        let deadline = Instant::now() + timeout;

        loop {
            match self
                .child
                .try_wait()
                .map_err(|e| format!("try_wait: {e}"))?
            {
                Some(status) => {
                    // Drain stdout + stderr.
                    use std::io::Read;
                    let mut stdout = String::new();
                    let mut stderr = String::new();
                    if let Some(mut r) = self.child.stdout.take() {
                        let _ = r.read_to_string(&mut stdout);
                    }
                    if let Some(mut r) = self.child.stderr.take() {
                        let _ = r.read_to_string(&mut stderr);
                    }
                    return Ok(Outcome {
                        exit_code: status.code(),
                        stdout,
                        stderr,
                    });
                }
                None => {
                    if Instant::now() >= deadline {
                        let _ = self.child.kill();
                        let _ = self.child.wait();
                        use std::io::Read;
                        let mut stderr = String::new();
                        if let Some(mut r) = self.child.stderr.take() {
                            let _ = r.read_to_string(&mut stderr);
                        }
                        return Err(format!(
                            "pikr did not exit within {:?}; stderr:\n{}",
                            timeout, stderr
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
        // Prefer the pre-built release binary next to the workspace Cargo.toml.
        // Tests run from the workspace root, so we can navigate from
        // CARGO_MANIFEST_DIR (apps/pikr) up two levels to the workspace root.
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let workspace = manifest
            .parent() // apps/
            .and_then(|p| p.parent()) // workspace root
            .expect("cannot resolve workspace root from CARGO_MANIFEST_DIR");

        let release_bin = workspace.join("target/release/pikr");
        if release_bin.exists() {
            return release_bin;
        }

        // Fall back: build it now (slow, but self-healing in CI).
        eprintln!("target/release/pikr not found — building (this may take a while)…");
        let status = Command::new("cargo")
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
