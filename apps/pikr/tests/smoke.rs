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

    thread::sleep(Duration::from_millis(250));

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
