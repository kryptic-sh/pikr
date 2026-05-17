//! Smoke tests for pikr.

use std::process::Command;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_pikr")
}

#[test]
fn prints_version() {
    let out = Command::new(bin()).arg("--version").output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.starts_with("pikr "), "got: {stdout}");
}

#[test]
fn prints_help() {
    let out = Command::new(bin()).arg("--help").output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("--show"));
    assert!(stdout.contains("--dmenu"));
}

/// Spawns the binary in drun mode and checks that it either enters the floem
/// event loop (display present → still running after 250 ms, we kill it) or
/// exits with a missing-display error (headless).
#[test]
fn accepts_show_drun() {
    use std::thread;
    use std::time::Duration;

    let has_display =
        std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some();

    let mut child = Command::new(bin())
        .args(["--show", "drun"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn pikr");

    // Wait long enough for pikr's startup to fail headless (when there's no
    // display) before we sample try_wait. Startup grew with frecency +
    // history loads + the initial rerank in v0.3.0; on a busy CI runner the
    // 250 ms timeout we used previously occasionally fired while pikr was
    // still in its pre-floem init path, making the test flake with a
    // misleading "pikr is running without any display set" panic.
    thread::sleep(Duration::from_millis(1500));

    match child.try_wait().expect("try_wait") {
        Some(status) => {
            // Already exited — only acceptable headless.
            assert!(
                !has_display,
                "pikr exited unexpectedly under a display ({status})"
            );
            let mut buf = String::new();
            if let Some(mut err) = child.stderr.take() {
                use std::io::Read;
                let _ = err.read_to_string(&mut buf);
            }
            assert!(
                buf.contains("WAYLAND_DISPLAY") || buf.contains("DISPLAY") || !status.success(),
                "unexpected headless failure: {buf}"
            );
        }
        None => {
            // Still running — event loop entered. Kill it.
            child.kill().ok();
            child.wait().ok();
            assert!(has_display, "pikr is running without any display set");
        }
    }
}
