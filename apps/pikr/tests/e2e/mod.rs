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

/// Drives a dismiss-style test to completion: re-sends `keys` on a 1s
/// cadence until pikr exits (or 15s), then asserts the exit code.
///
/// The old recipe — fixed 1500ms warmup, send once, wait — flaked on slow
/// CI runners: the layer-shell surface can claim focus late and the only
/// keystroke burst gets dropped (same race `wait_with_retry` documents).
/// Re-sending is safe here: once the first burst lands the process exits
/// and later sends hit nothing.
fn assert_keys_exit(sway: &Sway, pikr: Pikr, keys: &[Key], want: i32, what: &str) {
    let out = pikr
        .wait_with_retry(Duration::from_secs(15), Duration::from_millis(1000), || {
            let _ = Wtype::new(sway).keys(keys).send();
        })
        .unwrap();
    assert_eq!(out.exit_code, Some(want), "{what}; stderr:\n{}", out.stderr);
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

#[test]
fn startup_ranks_query_once() {
    if !require_tools() {
        return;
    }
    let sway = Sway::headless();
    let pikr = Pikr::spawn_with_env(
        &sway,
        &["--show", "drun"],
        None,
        &[("RUST_LOG", "pikr=debug")],
    )
    .unwrap();
    let out = pikr
        .wait_with_retry(Duration::from_secs(15), Duration::from_millis(100), || {
            let _ = Wtype::new(&sway).keys(&[Key::Escape, Key::Escape]).send();
        })
        .unwrap();

    assert_eq!(out.exit_code, Some(1), "stderr:\n{}", out.stderr);
    assert_eq!(
        out.stderr.matches("picker query reranked").count(),
        1,
        "startup must rank the initial query exactly once; stderr:\n{}",
        out.stderr
    );
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
    assert_keys_exit(
        &sway,
        pikr,
        &[Key::Escape, Key::Escape],
        1,
        "Esc Esc from drun must exit 1",
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
    assert_keys_exit(
        &sway,
        pikr,
        &[Key::Escape, Key::Escape],
        1,
        "Esc Esc from dmenu must exit 1",
    );
}

#[test]
fn initial_no_results_remains_dismissible() {
    if !require_tools() {
        return;
    }
    let sway = Sway::headless();
    let pikr = Pikr::spawn(
        &sway,
        &["--dmenu", "--filter", "zzz"],
        Some("alpha\nbeta\ngamma\n"),
    )
    .unwrap();
    assert_keys_exit(
        &sway,
        pikr,
        &[Key::Escape, Key::Escape],
        1,
        "initial no-results picker must remain dismissible",
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
    assert_keys_exit(
        &sway,
        pikr,
        &[Key::Escape, Key::Escape],
        1,
        "Esc Esc from calc must exit 1",
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
    assert_keys_exit(
        &sway,
        pikr,
        &[Key::Escape, Key::Escape],
        1,
        "Esc Esc from emoji must exit 1",
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
    assert_keys_exit(
        &sway,
        pikr,
        &[Key::Escape, Key::Escape],
        1,
        "Esc Esc from clipboard must exit 1",
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
    assert_keys_exit(
        &sway,
        pikr,
        &[Key::Escape, Key::Escape],
        1,
        "Esc Esc from run must exit 1",
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
    assert_keys_exit(
        &sway,
        pikr,
        &[Key::Escape, Key::Escape],
        1,
        "Esc Esc from ssh must exit 1",
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
    let out = pikr
        .wait_with_retry(Duration::from_secs(15), Duration::from_millis(1000), || {
            // The first event of every fresh wtype virtual-keyboard
            // connection is reliably swallowed while the compositor
            // installs its keymap — a bare single-Esc burst never lands
            // no matter how often it's retried (the esc_x2 tests dodge
            // this only because they send two). Lead with a sacrificial
            // 'a' the modal ignores so the Esc is never the first event;
            // the single-Esc-dismisses semantic stays honest because
            // only the Esc can exit.
            let _ = Wtype::new(&sway).text("a").keys(&[Key::Escape]).send();
        })
        .unwrap();
    assert_eq!(
        out.exit_code,
        Some(0),
        "--message Esc must exit 0; stderr:\n{}",
        out.stderr
    );
}

// ── dmenu accept-custom ───────────────────────────────────────────────────────

#[test]
fn case_sensitive_config_controls_startup_matcher() {
    if !require_tools() {
        return;
    }
    let sway = Sway::headless();
    let config = sway.runtime_dir.join("pikr-config.toml");
    std::fs::write(&config, "case_sensitive = true\n").unwrap();
    let config = config.to_str().unwrap();
    let pikr = Pikr::spawn(
        &sway,
        &["--dmenu", "--filter", "ban", "--config", config],
        Some("apple\nBanana\ncherry\n"),
    )
    .unwrap();
    let out = pikr
        .wait_with_retry(Duration::from_secs(15), Duration::from_millis(100), || {
            let _ = Wtype::new(&sway).keys(&[Key::F12, Key::Return]).send();
        })
        .unwrap();

    assert_eq!(out.exit_code, Some(0), "stderr:\n{}", out.stderr);
    assert_eq!(
        out.stdout.trim(),
        "ban",
        "case-sensitive query must not select differently-cased candidate"
    );
}

/// With a matching candidate, Shift+Enter (`AcceptCustom`) must print the
/// query rather than the selected candidate. F12 is an unhandled sacrificial
/// event on the same virtual-keyboard connection, so Shift+Enter is not lost
/// while the compositor installs that connection's keymap.
#[test]
fn accept_typed_query_with_shift_enter_emits_stdout() {
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
        .wait_with_retry(Duration::from_secs(15), Duration::from_millis(100), || {
            let _ = Wtype::new(&sway).keys(&[Key::F12, Key::ShiftReturn]).send();
        })
        .unwrap();
    assert_eq!(
        out.exit_code,
        Some(0),
        "ShiftReturn must exit 0; stderr:\n{}",
        out.stderr
    );
    assert_eq!(
        out.stdout.trim(),
        "ban",
        "stdout must be the typed query, not the matched candidate; got {:?}",
        out.stdout
    );
}

/// Typing a prefix that matches a candidate and pressing Return
/// (`Accept`) must print the matched candidate to stdout and exit 0.
/// Distinguishes the `Accept` arm from `AcceptCustom` (shift-enter).
///
/// Uses `--filter` to pre-seed the query. F12 is an unhandled sacrificial
/// event on the same virtual-keyboard connection, so Return is not lost while
/// the compositor installs that connection's keymap.
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
    let out = pikr
        .wait_with_retry(Duration::from_secs(15), Duration::from_millis(100), || {
            let _ = Wtype::new(&sway).keys(&[Key::F12, Key::Return]).send();
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
