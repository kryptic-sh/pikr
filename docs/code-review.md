# Code review — 2026-08-04 (post-fix pass)

Scope: working tree clean, `main` @ `cf1f186`. The prior review (this file at
`a5147c7`) read all 32 `.rs` files at `31980af`. Since then five fix commits
landed (`26391ae` batch `:mode`, `2980205` `-P` persistence gate, `035a673`
user-local drun overrides, `46c33db` Windows drun cache, `afd8579` picker edge
cases, `220d957` icon byte cache, `f88bcda` e2e isolation) — 762 changed lines
across 9 code/test files. This pass re-traced that complete diff hunk-by-hunk
against the current sources, plus every surrounding site each hunk depends on
(keydown handler, rerank effect, `entry_row`, virtual_stack data fn,
status-bar labels, Accept arms, `state.rs`, `frecency.rs`, the drun cache
probe, the e2e harness). Unix gate green: fmt, clippy `-D warnings`, release
build, nextest.

## Findings

### 1. LOW — saturated count prefix feeds a non-saturating consumer: `usize::MAX` count overflows `cur + n` in MoveDown

`apps/pikr/src/picker/state.rs:76` now saturates `push_count_digit` at
`usize::MAX`; the consumer at `apps/pikr/src/ui/view.rs:1365` is still
`let next = (cur + n).min(total.saturating_sub(1));` with `n` from
`state.take_count()`. Under the old wrapping arithmetic `cur + n` could not
overflow for any realistic selection index (the wrapped count was always a
multiple of 2^25 below `usize::MAX`); with saturation, `n == usize::MAX` makes
`cur + n` overflow for every `cur >= 1` — a debug-build panic, and in release a
silent wrap to `cur - 1` (the selection moves *up*, the opposite of the
motion). The fix's stated purpose — "must not silently wrap" — is not achieved
for the downstream consumer.

```
Repro: pikr --show drun; Esc; j (selected = 1); type 20 nines
       (99999999999999999999, count saturates to usize::MAX); j
Expect: no panic; selection advances (clamps to the bottom row)
Actual: debug build panics on integer overflow in `cur + n`;
        release wraps to `cur - 1` (row 0 here) — moves up
```

Fix direction: `cur.saturating_add(n)` at view.rs:1365 (MoveUp already uses
`saturating_sub`). The new unit test `push_count_digit_saturates_on_overflow`
(state.rs:197) pins the saturated value but no test exercises a motion with a
saturated count.

## Cleared

- **Deadlock fix (view.rs:1308–1315)**: `Effect::batch` wraps the lock and
  `switch_mode`; the guard is created inside the closure and drops before the
  queued subscriber effects run — matches the verified floem behavior (set
  dispatches subscribers synchronously only when not batching) and the
  existing `signal_set_inside_held_mutex_does_not_deadlock_when_batched`
  regression test (state.rs:110). Traced every signal set in the keydown
  handler: all other locks are statement-scoped (dropped at `;`), so no
  lock-held `set` remains. `rev.update` after the batch fires its subscribers
  (data fn, count label, empty-state) with the mutex free.
- **`entry_row`'s new `state → icons` double-lock (view.rs:258)**: the icons
  mutex is locked in exactly one place, always state-then-icons, always
  statement-scoped; the virtual_stack data fn's guard (view.rs:1067) drops
  before row builders run; no reverse lock order exists anywhere → no deadlock.
  Row builds occur during the update pass, outside every handler/effect lock
  scope.
- **`-P` persistence gate**: all four persistence sites gated on
  `!s.password` — click accept (view.rs:354), Accept (1450), dmenu no-match
  fallthrough (1479), AcceptCustom (1507). No other `history.push` /
  `usage.record` / `.save()` call sites exist in the tree; app.rs:129 wires
  `cli.password` into `AppState`. The e2e test asserts `history.toml` and
  `usage.toml` never appear under `-P`; load paths are read-only so the
  assertion can't false-positive.
- **`insert_first` (drun.rs:33)**: `default_paths()` yields user-local dirs
  before system dirs (verified in the prior review against
  freedesktop-desktop-entry 0.7.19), first-wins therefore = user override
  wins — matches the freedesktop search-order convention and the corrected
  comment. `by_id.into_values()` order is arbitrary, but that predates this
  change.
- **`tree_mtime` + strict `!=` probe (drun.rs:277, 304)**: whole-tree max with
  exact-equality compare is correct for add/remove/modify anywhere; deleting
  the max-mtime file makes the new max strictly smaller → miss → re-walk, never
  a stale hit. `None` on an empty tree only disables the cache, never serves
  stale data.
- **`parse_color` strictness (view.rs:56–62)**: all five default theme colors
  (config.rs:57–61) are exactly 6 hex digits; `from_str_radix` still blackens
  non-hex 6-char strings. The behavior change (short/8-digit → black) is
  intentional and documented in the changelog.
- **keys.rs dead-arm removal**: `<C-v>` already matched the first `"v"/"V"`
  arm — the deleted second arm was unreachable; no behavior change.
- **frecency separator (frecency.rs:135)**: `payload_key` is the single key
  function used by both `record` and `bonus`, so within a session keys are
  consistent; the args-empty arm (`program` alone) is unchanged; ssh/run
  payloads are `Exec{program, args:[]}` and unaffected. Old space-joined keys
  become orphans → a one-time soft reset of Exec-with-args frecency (advisory
  data, documented in the changelog).
- **`render_svg_to_png` guard (icons.rs:198–204)**: `dim <= 0.0 || !dim.is_finite()`
  catches both zero and NaN (`f32::max` drops a NaN only when the other operand
  is finite); `Pixmap::new(0, 0)` also bails.
- **`count` subscribers**: the count label reads `rev`/`selected`, not
  `count`, so `take_count`'s `count.set(None)` inside the keydown lock
  (pre-existing) fires no state-locking subscriber.
- **e2e XDG isolation (support/sway.rs, support/pikr.rs)**: per-test config/
  state dirs are created and injected; env overrides still win; the `-P` test's
  file-absence assertions target only the files that would hold the secret.

## Hardening

- **drun cache mtime granularity (drun.rs:304)**: `load_cache` compares
  seconds-truncated max mtimes with `!=`; a shortcut added within the same
  second (or within FAT's 2 s granularity) as the cached max is invisible until
  some later change. Pre-existing class (the old roots-only key had the same
  truncation), Windows-only, not a regression.
- **Icon byte cache never invalidates (icons.rs:157–170)**: `file_bytes` (and
  the pre-existing `raster_cache`/`resolve_or_fallback`) hold per-path bytes
  for the whole session — a PNG/JPEG icon replaced on disk mid-session keeps
  rendering the old bytes until restart. Consistent with the SVG cache; the
  change merely extends the policy to raster icons.
- **U+001F separator (frecency.rs:128–135)**: on Unix a program path may
  legally contain U+001F, so `Exec{program:"a\u{1f}b"}` still collides with
  `Exec{program:"a", args:["b"]}` — the comment's "can't appear in a program
  path" overstates. Absurd input, ranking bias only.
- **`tree_mtime` walks the whole Start Menu tree on every launch (drun.rs:277)**:
  the cache probe no longer skips the walk on hit; on a large or
  network-mounted tree this is a per-launch stat cost the roots-only key
  avoided. Deliberate, documented tradeoff; perf, not correctness.

## Coverage

- Reviewed this pass: the complete `31980af..HEAD` diff (762 changed lines) —
  every hunk traced against the current source, with the surrounding code each
  hunk depends on re-read (keydown handler, rerank effect, `entry_row`,
  virtual_stack data fn, status-bar labels, Accept/AcceptCustom/dmenu arms,
  `state.rs`, `frecency.rs` in full, drun unix+windows collect/cache, e2e
  harness).
- Relied on the prior review (same day, `a5147c7`, all 32 files at `31980af`)
  for the unchanged remainder — matcher, history, keys/state/icons unchanged
  regions, css, app (beyond the password wiring), cli, config, main,
  console_attach, calc/clipboard/dmenu/emoji/ssh/run modes,
  drun_icons_windows, xtask, keyboard.rs, smoke.rs — spot-checked at the
  interaction points with the diff but not re-read line-by-line.
- NOT compiled or run (no Windows/macOS runner): all `#[cfg(windows)]` code —
  the diff's `tree_mtime`/cache changes and the `cfg(not(unix))` arms of
  `switch_mode` were reviewed by reading only. The unix gate (fmt, clippy,
  release build, nextest) is green.
