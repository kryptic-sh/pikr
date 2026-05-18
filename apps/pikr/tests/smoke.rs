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

/// Without `WAYLAND_DISPLAY`, pikr must exit non-zero and print the
/// "WAYLAND_DISPLAY is not set" guard message. The full live-render path
/// is exercised by the e2e harness (`tests/e2e/`), which runs pikr inside
/// a `sway --headless` fixture so dev machines don't see a stray window
/// pop on every `cargo test` invocation.
#[test]
fn missing_wayland_display_exits_with_guard() {
    let out = Command::new(bin())
        .args(["--show", "drun"])
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DISPLAY")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .expect("spawn pikr");

    assert!(
        !out.status.success(),
        "pikr without WAYLAND_DISPLAY must exit non-zero; got {}",
        out.status
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("WAYLAND_DISPLAY"),
        "expected guard message, got stderr: {err}"
    );
}
