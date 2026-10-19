//! End-to-end keyboard harness for pikr.
//!
//! Each test spins up an isolated `sway --headless` session, spawns the pikr
//! release binary inside it, drives input via `wtype`, and asserts on the
//! exit code (and optionally stdout).
//!
//! Only what needs the real binary and a compositor lives here: reading
//! stdin (and `--loading`), the config file, the state files, and keys
//! arriving through Wayland. What keys do once they reach pikr is tested
//! headless and on every OS in `src/ui/run/headless.rs`, without sway, a
//! GPU or a window.
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

// ── The real binary: config, stdin, state files ───────────────────────────────────────────────────────

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

/// `--password` (-P) masks the query bar glyphs, but a plaintext leak of the
/// typed value defeats the whole point: the review found Shift+Enter still
/// wrote the secret to `$XDG_STATE_HOME/pikr/history.toml` (and bumped usage
/// frecency) on accept. The harness gives every pikr an isolated
/// config/state dir under the sway runtime dir, so this test never reads the
/// developer's real config or writes their real history. F12 is the
/// unhandled sacrificial first event, matching the other accept tests;
/// `text` types the literal secret.
#[test]
fn password_accept_never_persists_typed_query() {
    if !require_tools() {
        return;
    }
    let sway = Sway::headless();
    let pikr = Pikr::spawn(&sway, &["--dmenu", "-P"], Some("a\nb\n")).unwrap();
    let out = pikr
        .wait_with_retry(Duration::from_secs(15), Duration::from_millis(1000), || {
            // F12 first: the fresh wtype connection's first event is swallowed
            // while the compositor installs its keymap, so lead with the
            // sacrificial key before the secret text (single_esc_message_modal
            // convention) — a swallowed 'S' would accept the wrong query.
            let _ = Wtype::new(&sway)
                .keys(&[Key::F12])
                .text("S3cr3t")
                .keys(&[Key::ShiftReturn])
                .send();
        })
        .unwrap();
    assert_eq!(
        out.exit_code,
        Some(0),
        "ShiftReturn with -P must exit 0; stderr:\n{}",
        out.stderr
    );
    assert_eq!(
        out.stdout.trim(),
        "S3cr3t",
        "-P must still emit the typed query to stdout; got {:?}",
        out.stdout
    );
    // The secret must never land on disk: neither the history nor the usage
    // file may exist after a -P accept (the empty `state/pikr/` dir itself is
    // pre-created by the xdg crate during load on every launch, so assert on
    // the files that would hold the secret).
    let pikr_state = sway.runtime_dir.join("state/pikr");
    assert!(
        !pikr_state.join("history.toml").exists(),
        "-P accept must not persist the typed query to history.toml"
    );
    assert!(
        !pikr_state.join("usage.toml").exists(),
        "-P accept must not write usage.toml"
    );
}

// ── --loading ───────────────────────────────────────────────────────────────

/// With `--loading`, rows read on the background thread still land and
/// accept normally.
#[test]
fn loading_rows_arrive_and_accept() {
    if !require_tools() {
        return;
    }
    let sway = Sway::headless();
    let pikr = Pikr::spawn(
        &sway,
        &["--dmenu", "--loading", "Loading…", "--filter", "ban"],
        Some("apple\nbanana\ncherry\n"),
    )
    .unwrap();
    let out = pikr
        .wait_with_retry(Duration::from_secs(15), Duration::from_millis(1000), || {
            let _ = Wtype::new(&sway).keys(&[Key::F12, Key::Return]).send();
        })
        .unwrap();
    assert_eq!(out.exit_code, Some(0), "stderr:\n{}", out.stderr);
    assert_eq!(out.stdout.trim(), "banana");
}
