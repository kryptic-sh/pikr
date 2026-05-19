//! End-to-end keyboard harness for pikr.
//!
//! Each test spins up an isolated `sway --headless` session, spawns the pikr
//! release binary inside it, drives input via `wtype`, and asserts on the
//! exit code (and optionally stdout).
//!
//! Prerequisites: `sway` and `wtype` on `$PATH`, and a pre-built
//! `target/release/pikr` (or the harness will build it automatically).

mod support;

use std::time::Duration;
use support::{Key, Pikr, Sway, Wtype};

// ── helpers ───────────────────────────────────────────────────────────────────

/// Checks that `sway` and `wtype` are on `$PATH` — returns `false` if not so
/// callers can `return` cleanly without proceeding to spawn a fixture that
/// would panic with an unhelpful "no such file or directory" message.
///
/// Tests must use the early-return idiom: `if !require_tools() { return; }`.
/// The CI job is gated to `ubuntu-latest` where both tools are installed
/// via apt; this guard only matters for local runs on macOS / fresh hosts.
#[must_use]
fn require_tools() -> bool {
    for tool in &["sway", "wtype"] {
        if std::process::Command::new("which")
            .arg(tool)
            .output()
            .map(|o| !o.status.success())
            .unwrap_or(true)
        {
            eprintln!("SKIP: `{tool}` not found on PATH");
            return false;
        }
    }
    true
}

// ── Escape-based dismiss tests ────────────────────────────────────────────────

/// Two Escapes from Insert mode in `--show drun` must exit 1 (dismissed).
#[test]
fn esc_x2_from_insert_exits_1_drun() {
    if !require_tools() {
        return;
    }
    let sway = Sway::headless();
    let pikr = Pikr::spawn(&sway, &["--show", "drun"], None).unwrap();
    Wtype::new(&sway)
        .delay(Duration::from_millis(1500))
        .keys(&[Key::Escape, Key::Escape])
        .send()
        .unwrap();
    let out = pikr.wait_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(
        out.exit_code,
        Some(1),
        "Esc Esc from drun must exit 1; stderr:\n{}",
        out.stderr
    );
}

/// Two Escapes in `--dmenu` mode must exit 1.
#[test]
fn esc_x2_from_insert_exits_1_dmenu() {
    if !require_tools() {
        return;
    }
    let sway = Sway::headless();
    let pikr = Pikr::spawn(&sway, &["--dmenu"], Some("alpha\nbeta\ngamma\n")).unwrap();
    Wtype::new(&sway)
        .delay(Duration::from_millis(1500))
        .keys(&[Key::Escape, Key::Escape])
        .send()
        .unwrap();
    let out = pikr.wait_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(
        out.exit_code,
        Some(1),
        "Esc Esc from dmenu must exit 1; stderr:\n{}",
        out.stderr
    );
}

/// Two Escapes in `--show calc` mode must exit 1.
#[test]
fn esc_x2_from_insert_exits_1_calc() {
    if !require_tools() {
        return;
    }
    let sway = Sway::headless();
    let pikr = Pikr::spawn(&sway, &["--show", "calc"], None).unwrap();
    Wtype::new(&sway)
        .delay(Duration::from_millis(1500))
        .keys(&[Key::Escape, Key::Escape])
        .send()
        .unwrap();
    let out = pikr.wait_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(
        out.exit_code,
        Some(1),
        "Esc Esc from calc must exit 1; stderr:\n{}",
        out.stderr
    );
}

/// Two Escapes in `--show emoji` mode must exit 1.
#[test]
fn esc_x2_from_insert_exits_1_emoji() {
    if !require_tools() {
        return;
    }
    let sway = Sway::headless();
    let pikr = Pikr::spawn(&sway, &["--show", "emoji"], None).unwrap();
    Wtype::new(&sway)
        .delay(Duration::from_millis(1500))
        .keys(&[Key::Escape, Key::Escape])
        .send()
        .unwrap();
    let out = pikr.wait_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(
        out.exit_code,
        Some(1),
        "Esc Esc from emoji must exit 1; stderr:\n{}",
        out.stderr
    );
}

/// Two Escapes in `--show clipboard` mode must exit 1.
#[test]
fn esc_x2_from_insert_exits_1_clipboard() {
    if !require_tools() {
        return;
    }
    let sway = Sway::headless();
    let pikr = Pikr::spawn(&sway, &["--show", "clipboard"], None).unwrap();
    Wtype::new(&sway)
        .delay(Duration::from_millis(1500))
        .keys(&[Key::Escape, Key::Escape])
        .send()
        .unwrap();
    let out = pikr.wait_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(
        out.exit_code,
        Some(1),
        "Esc Esc from clipboard must exit 1; stderr:\n{}",
        out.stderr
    );
}

/// Two Escapes in `--show run` mode must exit 1.
#[test]
fn esc_x2_from_insert_exits_1_run() {
    if !require_tools() {
        return;
    }
    let sway = Sway::headless();
    let pikr = Pikr::spawn(&sway, &["--show", "run"], None).unwrap();
    Wtype::new(&sway)
        .delay(Duration::from_millis(1500))
        .keys(&[Key::Escape, Key::Escape])
        .send()
        .unwrap();
    let out = pikr.wait_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(
        out.exit_code,
        Some(1),
        "Esc Esc from run must exit 1; stderr:\n{}",
        out.stderr
    );
}

/// Two Escapes in `--show ssh` mode must exit 1.
#[test]
fn esc_x2_from_insert_exits_1_ssh() {
    if !require_tools() {
        return;
    }
    let sway = Sway::headless();
    let pikr = Pikr::spawn(&sway, &["--show", "ssh"], None).unwrap();
    Wtype::new(&sway)
        .delay(Duration::from_millis(1500))
        .keys(&[Key::Escape, Key::Escape])
        .send()
        .unwrap();
    let out = pikr.wait_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(
        out.exit_code,
        Some(1),
        "Esc Esc from ssh must exit 1; stderr:\n{}",
        out.stderr
    );
}

// ── Message modal ─────────────────────────────────────────────────────────────

/// `--message` mode dismisses on a single Escape and exits 0.
#[test]
fn single_esc_message_modal_exits_0() {
    if !require_tools() {
        return;
    }
    let sway = Sway::headless();
    let pikr = Pikr::spawn(&sway, &["--message", "hello"], None).unwrap();
    Wtype::new(&sway)
        .delay(Duration::from_millis(1500))
        .keys(&[Key::Escape])
        .send()
        .unwrap();
    let out = pikr.wait_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(
        out.exit_code,
        Some(0),
        "--message Esc must exit 0; stderr:\n{}",
        out.stderr
    );
}

// ── dmenu accept-custom ───────────────────────────────────────────────────────

/// Typing a query that matches no candidate and pressing Shift+Enter
/// (`AcceptCustom`) must print the typed text to stdout and exit 0.
#[test]
fn accept_typed_query_with_shift_enter_emits_stdout() {
    if !require_tools() {
        return;
    }
    let sway = Sway::headless();
    let pikr = Pikr::spawn(&sway, &["--dmenu"], Some("apple\nbanana\ncherry\n")).unwrap();
    Wtype::new(&sway)
        .delay(Duration::from_millis(1500))
        .text("xyz") // no match → query stays as-is
        .keys(&[Key::ShiftReturn]) // Shift-Enter = AcceptCustom → exit 0 + stdout
        .send()
        .unwrap();
    let out = pikr.wait_timeout(Duration::from_secs(10)).unwrap();
    assert_eq!(
        out.exit_code,
        Some(0),
        "ShiftReturn must exit 0; stderr:\n{}",
        out.stderr
    );
    assert_eq!(
        out.stdout.trim(),
        "xyz",
        "stdout must be the typed query; got {:?}",
        out.stdout
    );
}

/// Typing a prefix that matches a candidate and pressing Return
/// (`Accept`) must print the matched candidate to stdout and exit 0.
/// Distinguishes the `Accept` arm from `AcceptCustom` (shift-enter).
///
/// Uses `--filter` to pre-seed the query so pikr runs ONE matcher pass
/// at startup; the test then sends only Return. Typing characters live
/// was racy on CI's pixman+broken-Vulkan path — each keystroke
/// triggered a per-key rerank+repaint that occasionally outran wtype's
/// pacing, leaving Return queued before pikr finished re-rendering.
#[test]
fn accept_matched_candidate_with_return_emits_stdout() {
    if !require_tools() {
        return;
    }
    let sway = Sway::headless();
    let pikr = Pikr::spawn(
        &sway,
        &["--dmenu", "--filter", "ban"],
        Some("apple\nbanana\ncherry\n"),
    )
    .unwrap();
    // First sway+pikr spawn in the binary is the slow one on CI: zink
    // fails (VK_ERROR_INCOMPATIBLE_DRIVER), EGL falls back to pixman,
    // and the layer-shell surface can take >3s to map + claim focus.
    // wtype's `-s` only spaces events between each other, not before
    // the first, so a sleep here is the only way to guarantee Return
    // lands after focus claim. Two mitigations:
    //   1. 3s pre-key sleep so pikr finishes paint + focus claim.
    //   2. Send Return twice — second is a no-op after Action::Accept
    //      exits, but covers the dropped-first-key race.
    std::thread::sleep(Duration::from_secs(3));
    Wtype::new(&sway)
        .delay(Duration::from_millis(500))
        .keys(&[Key::Return, Key::Return])
        .send()
        .unwrap();
    let out = pikr.wait_timeout(Duration::from_secs(15)).unwrap();
    assert_eq!(
        out.exit_code,
        Some(0),
        "Return on a matched candidate must exit 0; stderr:\n{}",
        out.stderr
    );
    assert_eq!(
        out.stdout.trim(),
        "banana",
        "stdout must be the matched candidate; got {:?}",
        out.stdout
    );
}
