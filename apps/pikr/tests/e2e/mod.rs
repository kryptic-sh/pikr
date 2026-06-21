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

// ── Warmup ────────────────────────────────────────────────────────────────────

/// Burns the first sway+pikr cold-spawn so later tests don't get hit by
/// the `ERROR_SURFACE_LOST_KHR` race (pikr issue #34). Asserts nothing —
/// any exit code, any output, even a hard hang is fine. Named to sort
/// first alphabetically so it always runs before the real assertions.
/// Cost: ~1s of test time on the assumption that whatever happens to
/// the first pikr is sacrificed for the rest of the suite.
#[test]
fn aaa_warmup_absorbs_first_spawn_race() {
    if !require_tools() {
        return;
    }
    let sway = Sway::headless();
    let pikr = Pikr::spawn(&sway, &["--show", "drun"], None).unwrap();
    // Give pikr a beat to either succeed at first paint or trip the
    // race; either way we kill it. Length is empirical: shorter than
    // ~750ms and the next test still occasionally hits the race.
    std::thread::sleep(Duration::from_millis(1000));
    drop(pikr);
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
///
/// Ignored: same input/accept race as #34. The software-GL harness fix
/// (`Pikr::spawn`) removed the zink/Vulkan render crash that previously broke
/// this on CI (stderr is now clean), but the keystroke still intermittently
/// fails to land on the layer-shell surface before focus is claimed, so pikr
/// never accepts and the test times out. The keymap→`AcceptCustom` mapping is
/// covered by the `insert_shift_enter_accept_custom` unit test; un-ignore once
/// the focus/input race is fixed. Run with `cargo test -- --ignored` to repro.
#[test]
#[ignore = "input/accept race — see pikr#34 (render race fixed via software GL)"]
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
/// at startup; the test then sends Return on a retry cadence until
/// pikr exits.
///
/// Ignored: tracks pikr issue #34. The harness now forces software GL
/// (`WGPU_BACKEND=gl` + `LIBGL_ALWAYS_SOFTWARE`, see `Pikr::spawn`), which
/// removed the original `ERROR_SURFACE_LOST_KHR` / zink-Vulkan render failure
/// (stderr is now clean). But a separate input/accept race remains: the Return
/// keystroke intermittently doesn't land, so pikr never exits and the test
/// times out (~1 in 2 runs locally). Keeps `Pikr::wait_with_retry`; un-ignore
/// once the accept race is fixed. Run with `cargo test -- --ignored` to repro.
#[test]
#[ignore = "input/accept race — see pikr#34 (render race fixed via software GL)"]
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
    let out = pikr
        .wait_with_retry(Duration::from_secs(15), Duration::from_millis(1000), || {
            let _ = Wtype::new(&sway).keys(&[Key::Return]).send();
        })
        .unwrap();
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
