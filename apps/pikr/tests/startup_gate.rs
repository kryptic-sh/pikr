//! External end-to-end time gate for pikr's launch-to-use latency.
//!
//! Drives the full release `pikr` binary inside a headless sway session and
//! measures the wall-clock time from process spawn to exit after a selection
//! is locked in — an EXTERNAL timer, so the measurement includes process
//! spawn, Wayland connection, surface creation, first paint, key delivery and
//! accept, not just pikr's internal startup markers. The perf markers pikr
//! logs under `RUST_LOG=pikr=debug` (config / collect / rank / first paint /
//! first focus, each with `elapsed_us`) are parsed out of stderr and reported
//! alongside, so a regression can be attributed to a phase.
//!
//! The gate deadline is 500 ms end-to-end (matching the usable-picker startup
//! target). It is a solo CI job that runs only after the full test suite, so
//! the measurement gets the whole runner.
//!
//! Unix-only: the harness drives `sway --headless` + `wtype`.

#![cfg(unix)]
// The support harness is shared with the keyboard e2e binary; this gate only
// exercises the spawn/accept subset, so the rest is legitimately unused here.
#![allow(dead_code)]

#[path = "e2e/support/mod.rs"]
mod support;

use std::time::{Duration, Instant};
use support::{Key, Pikr, Sway, Wtype};

/// The acceptance gate: 500 ms from spawn to exit after accept.
const E2E_DEADLINE: Duration = Duration::from_millis(500);

/// Parse `elapsed_us=N` from the first stderr region naming `marker` (e.g.
/// `startup first focus received`). pikr's perf markers print as
/// `... DEBUG pikr::ui::view: <name> elapsed_us=<us>`; the collect marker has
/// `phase_us=` before `elapsed_us=`, so search after the name, not adjacent.
///
/// ANSI escape codes are stripped first: tracing-subscriber emits colors
/// unless `NO_COLOR` is set (its default), and the escapes wrap field names
/// (`elapsed_us` + ESC + `=`), which would break the literal search. The gate
/// sets `NO_COLOR=1` on the spawn too, but the parser stays robust to a
/// colorized stderr regardless.
fn marker_us(stderr: &str, marker: &str) -> Option<u64> {
    let stderr = strip_ansi(stderr);
    let rest = &stderr[stderr.find(marker)? + marker.len()..];
    let idx = rest.find("elapsed_us=")?;
    rest[idx + "elapsed_us=".len()..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .ok()
}

/// Remove CSI escape sequences (`ESC [ … final byte`) from a string.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if ('\x40'..='\x7e').contains(&c) {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Drive one full pikr lifecycle: spawn, accept the matched candidate via
/// F12+Return, exit. Returns the outcome for assertion.
fn run_once(sway: &Sway) -> Result<support::pikr::Outcome, String> {
    let pikr = Pikr::spawn_with_env(
        sway,
        &["--dmenu", "--filter", "ban"],
        Some("apple\nbanana\ncherry\n"),
        &[
            ("RUST_LOG", "pikr=debug"),
            // tracing-subscriber colors stderr unless NO_COLOR is set; the
            // ANSI escapes wrap the marker field names and break parsing.
            ("NO_COLOR", "1"),
        ],
    )?;
    pikr.wait_with_retry(Duration::from_secs(15), Duration::from_millis(100), || {
        let _ = Wtype::new(sway).keys(&[Key::F12, Key::Return]).send();
    })
}

#[test]
fn external_e2e_time_within_500ms() {
    for tool in &["sway", "wtype"] {
        if std::process::Command::new("which")
            .arg(tool)
            .output()
            .map(|o| !o.status.success())
            .unwrap_or(true)
        {
            eprintln!("{tool} not on PATH — skipping startup gate");
            return;
        }
    }

    let sway = Sway::headless();
    // Warmup: absorb the first-spawn cold cost (shader JIT, page cache,
    // the first-surface race the keyboard suite's `aaa_warmup` absorbs) so
    // the measured launch is the steady-state one. Timing discarded.
    let _ = run_once(&sway);

    // External timer: start BEFORE spawning pikr, stop when the process exits.
    let spawn_start = Instant::now();
    let out = run_once(&sway).expect("pikr must exit");
    let e2e = spawn_start.elapsed();

    assert_eq!(
        out.exit_code,
        Some(0),
        "Return on a matched candidate must exit 0; stderr:\n{}",
        out.stderr
    );
    assert_eq!(
        out.stdout.trim(),
        "banana",
        "stdout must be the matched candidate; got {:?}; stderr:\n{}",
        out.stdout,
        out.stderr
    );

    let collected = marker_us(&out.stderr, "startup entries collected");
    let ranked = marker_us(&out.stderr, "startup state ranked");
    let paint = marker_us(&out.stderr, "startup first paint pass reached");
    let focus = marker_us(&out.stderr, "startup first focus received");
    eprintln!(
        "startup-gate: e2e={} ms (deadline {} ms) | collected={}us ranked={}us paint={}us focus={}us",
        e2e.as_millis(),
        E2E_DEADLINE.as_millis(),
        collected.unwrap_or(0),
        ranked.unwrap_or(0),
        paint.unwrap_or(0),
        focus.unwrap_or(0),
    );

    assert!(
        e2e <= E2E_DEADLINE,
        "end-to-end launch→accept→exit took {} ms, exceeding the {} ms gate; \
         perf markers: collected={}us ranked={}us paint={}us focus={}us\nstderr:\n{}",
        e2e.as_millis(),
        E2E_DEADLINE.as_millis(),
        collected.unwrap_or(0),
        ranked.unwrap_or(0),
        paint.unwrap_or(0),
        focus.unwrap_or(0),
        out.stderr,
    );
    assert!(
        focus.is_some(),
        "pikr never reported `startup first focus received`; stderr:\n{}",
        out.stderr
    );
}
