//! Sway headless fixture.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// A running `sway --headless` session.
///
/// The session is terminated on `Drop`.
pub struct Sway {
    child: Child,
    /// A virtual keyboard held for the whole session; see [`hold_keyboard`].
    keyboard: Child,
    /// `XDG_RUNTIME_DIR` for this session (a per-test tempdir).
    pub runtime_dir: PathBuf,
    /// `XDG_CONFIG_HOME` for pikr under test (isolated from the developer's
    /// real `~/.config`).
    pub config_dir: PathBuf,
    /// `XDG_STATE_HOME` for pikr under test (isolated from the developer's
    /// real `~/.local/state`).
    pub state_dir: PathBuf,
    /// `WAYLAND_DISPLAY` socket name (e.g. `"wayland-1"`).
    pub display: String,
}

impl Sway {
    /// Spawn a headless sway session. Blocks until the Wayland socket is
    /// ready (up to 10 s) or panics.
    pub fn headless() -> Self {
        let runtime_dir = tempdir();
        let config_dir = runtime_dir.join("config");
        let state_dir = runtime_dir.join("state");
        std::fs::create_dir_all(&config_dir).expect("create config dir");
        std::fs::create_dir_all(&state_dir).expect("create state dir");
        let config_path = write_config(&runtime_dir);

        let child = Command::new("sway")
            .args([
                "--config",
                config_path.to_str().unwrap(),
                "--unsupported-gpu",
            ])
            .env("WLR_BACKENDS", "headless")
            .env("WLR_RENDERER", "pixman")
            .env("XDG_RUNTIME_DIR", &runtime_dir)
            // Suppress sway's verbose stderr in test output unless a test
            // explicitly checks it.
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn sway — is sway installed?");

        // Sway creates `<XDG_RUNTIME_DIR>/wayland-1` (or wayland-N) once the
        // compositor is ready. Poll for it.
        let display = wait_for_socket(&runtime_dir);
        let keyboard = hold_keyboard(&runtime_dir, &display);

        Sway {
            child,
            keyboard,
            runtime_dir,
            config_dir,
            state_dir,
            display,
        }
    }

    /// Returns env vars that child processes need to connect to this session.
    pub fn env_vars(&self) -> [(&'static str, String); 2] {
        [
            (
                "XDG_RUNTIME_DIR",
                self.runtime_dir.to_str().unwrap().to_owned(),
            ),
            ("WAYLAND_DISPLAY", self.display.clone()),
        ]
    }
}

impl Drop for Sway {
    fn drop(&mut self) {
        let _ = self.keyboard.kill();
        let _ = self.keyboard.wait();
        // SIGTERM first, escalate to SIGKILL if the process doesn't exit in 2 s.
        let pid = self.child.id();
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if self.child.try_wait().ok().flatten().is_some() {
                break;
            }
            if Instant::now() >= deadline {
                let _ = self.child.kill();
                let _ = self.child.wait();
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        // Clean up the runtime dir we created.
        let _ = std::fs::remove_dir_all(&self.runtime_dir);
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn tempdir() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "pikr-e2e-{}",
        std::process::id().wrapping_add(rand_suffix())
    ));
    std::fs::create_dir_all(&path).expect("create tempdir");
    path
}

/// Monotonic per-process counter so back-to-back fixtures in the same
/// test binary land in distinct tempdirs. A nanos-based suffix was
/// considered but two `Sway::headless()` calls within ~1 ms can share
/// `subsec_nanos` on coarse clocks; a counter avoids that entirely.
fn rand_suffix() -> u32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    SEQ.fetch_add(1, Ordering::Relaxed)
}

fn write_config(runtime_dir: &Path) -> PathBuf {
    let cfg = runtime_dir.join("sway-config");
    std::fs::write(
        &cfg,
        "output HEADLESS-1 mode 800x600\nexec sleep infinity\n",
    )
    .expect("write sway config");
    cfg
}

/// How long the held keyboard's wtype sleeps: longer than any test runs.
const KEYBOARD_HOLD_MS: &str = "3600000";

/// Starts a wtype that only sleeps, so its virtual keyboard stays on the
/// seat until the session ends. Headless sway has no keyboard of its own:
/// without this the seat's only keyboard is each test's short-lived wtype,
/// and pikr gets the focus when one connects and loses it when that wtype
/// exits, which pikr takes as a cancel (exit 1). A wtype that typed before
/// pikr mapped thus closed pikr before the retry could type again.
/// Releasing shift after the sleep presses nothing.
fn hold_keyboard(runtime_dir: &Path, display: &str) -> Child {
    Command::new("wtype")
        .args(["-s", KEYBOARD_HOLD_MS, "-m", "shift"])
        .env("XDG_RUNTIME_DIR", runtime_dir)
        .env("WAYLAND_DISPLAY", display)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn wtype — is wtype installed?")
}

fn wait_for_socket(runtime_dir: &Path) -> String {
    // `runtime_dir` is the per-test tempdir, *not* the host XDG_RUNTIME_DIR,
    // so sway is the only writer and the socket name starts at `wayland-0`
    // / `wayland-1`. Probe a small fixed set; any name past `wayland-3`
    // would indicate a stale sway in this tempdir, which is a bug.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        for name in &["wayland-1", "wayland-2", "wayland-3", "wayland-0"] {
            let sock = runtime_dir.join(name);
            if sock.exists() {
                return name.to_string();
            }
        }
        if Instant::now() >= deadline {
            panic!(
                "sway did not create a Wayland socket in {:?} within 10 s",
                runtime_dir
            );
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
