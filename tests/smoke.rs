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

#[test]
fn accepts_show_drun() {
    let out = Command::new(bin())
        .args(["--show", "drun"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
