# Backlog

## GUI on crcbl

- **The port to crcbl landed on 2026-10-16.** floem, its winit fork, resvg,
  `freedesktop-icons`, `image`, `hjkl-css` and `imbl` are gone. Every crcbl
  crate in the workspace `Cargo.toml` is on crates.io 0.3.0; bump them together.
  If a fix pikr needs is only on crcbl main, pin every crate to the same
  CI-green rev and add the repo back to `deny.toml`'s `allow-git` until a
  release carries it.
- **Decision needed: silent opaque fallback in `ui::gpu::Gpu::open`.** When the
  window asked for transparency but the surface offers no
  `CompositeAlpha::PreMultiplied`, pikr draws opaque without saying so. The
  owner's rule as relayed by crcbl's session on 2026-10-17 ("every feature works
  on every backend or is refused loudly") would forbid that; not yet confirmed
  for pikr by the user. Options: a `tracing::warn!` and carry on opaque (pikr
  still works, corners go square), or fail with an error (loud, but a working
  picker is lost over a cosmetic gap).
- **Windows transparency is D3D12 only.** crcbl draws a transparent Win32 window
  through a DirectComposition swapchain and refuses one on Vulkan, which pikr
  does not use on Windows. Pixels with alpha 0 (outside the panel's rounded
  corners) still take the pointer. Rounded, see-through corners were confirmed
  in a `--dmenu` screenshot at `4d48eaf4`.
- `picker::icons::tests::svg_text_is_drawn` needs a system sans-serif face; the
  Ubuntu test job installs no fonts, so it relies on the runner image.
- **Waiting on crcbl.** Each is a gap in pikr until it lands; crcbl's session
  has every one queued and reports green revs as they land.
  - Regional-indicator flags (🇺🇸) draw as two letters with Segoe UI Emoji, which
    has no flag ligatures; Noto Color Emoji draws them. Not a crcbl bug.
  - Bidi, UAX #14 line breaking, full UAX #29 clusters, and shaping in SVG text.
    Shaped text beyond Latin and emoji (Arabic, Indic) was not checked from
    pikr.
  - SVG text is drawn one glyph per character from cmap, without shaping, and
    `flowRoot` is skipped as browsers skip it. Text in characters no installed
    face has is refused, and the icon falls back to the generic one. Coverage of
    real icon themes was not measured from pikr; crcbl's CI holds the breeze
    corpus against resvg (floors per crcbl's 0.2.1 release notes).
  - S9 IME on/off on Win32 and AppKit.
  - D3D12 startup (crcbl's S13): `Dx12Instance::open` probes every adapter
    before returning, and pikr pays for all of them. At crcbl 0.3.0 one
    `RUST_LOG=debug` launch logged ~306 ms for the RX 7900 XTX, ~272 ms for the
    integrated GPU and ~15 ms for WARP, then ~257 ms for `create_device`. crcbl
    is working on lazy adapter probing and will report before/after timings.
  - Win32 reports `Focus { focused: true }` right after `FocusRefused` when
    Windows refuses pikr the foreground (seen 2026-10-18 at crcbl 0.3.0, pikr
    launched from a background shell while explorer kept the foreground), so
    pikr blinks its caret while keys go elsewhere. crcbl's diagnosis:
    `ShowWindow(SW_SHOWNORMAL)` activates the panel within its own thread, and
    the `WM_SETFOCUS` that follows is reported as focus. crcbl will report focus
    only while the thread also holds the foreground; recheck on a desktop when
    it lands.
  - crcbl-dx12 turns on DRED in every build; debug builds also ask for the D3D12
    debug layer (a WARN without the Graphics Tools feature). By design;
    `CRCBL_DX12_VALIDATION=0` turns the debug layer off.
- **The Normal-mode block caret past the end of the query sits on a no-break
  space** (`ui::view::query`): a span holding only a plain space collapses to
  nothing in crcbl-ui (it supports `white-space: normal | nowrap` only), and the
  caret did not draw at all. Seen on Windows with and without the old
  block-wrapped caret.
- **Named keys must come from the keysym, not `ShellEvent::Key`'s `key_code`**
  (`ui::keyboard::named_from_keysym`). `key_code` is the physical key from the
  evdev code, and wtype uploads its own keymap that hands out key codes from the
  bottom, so its first key arrived as Escape: the first sway e2e run of the port
  (CI run 38947484014) exited 1 on every test that typed or pressed Return, and
  the Esc-only tests passed by coincidence. The same holds for any virtual or
  remapped keyboard.
- **Not verified (coverage gaps).**
  - Linux was verified only through CI's sway session on lavapipe: the e2e
    keyboard suite and the startup gate passed (run 39002212977, 237 ms
    launch-to-result against the 500 ms deadline). Hyprland, niri and KWin were
    never run, and no Linux run was looked at by eye.
  - FreeBSD is only type-checked: CI's Ubuntu clippy job lints the
    `x86_64-unknown-freebsd` target. Nothing links or runs it.
  - macOS: CI builds it and runs the unit tests and the `--version`/`--help`
    smoke tests; the window was never opened.
  - Windows (D3D12): the picker, match highlighting, a scrolled list with its
    scrollbar and the `--message` modal were rendered and looked at in
    screenshots. Keyboard input, clicks, the wheel and drun icons were not
    exercised: pikr launched from a background shell never got the foreground,
    and synthesized keys would have gone to another window.
  - No icon from a real icon theme was drawn; icon decoding is covered by unit
    tests only. HiDPI (scale ≠ 1) was not exercised.
  - IME pre-edit and commit through text-input-v3, and accessibility reaching
    AT-SPI (crcbl's AccessKit row), were never checked from pikr.
- **Project constraints for the GUI** (from the migration's design, still
  binding): pure-Rust libraries only, with GTK and GNOME-based stacks excluded
  by project direction; keep `ui/styles/default.css`, its class names and theme
  placeholders as the styling contract; render only the visible rows of large
  modes (emoji); and reach a usable picker in under 500 ms from a cold start.
- **GUI stacks considered and declined** before crcbl was chosen (2026-10-15).
  Re-propose one only with new evidence.
  - Iced with `iced_layershell`: a third-party layer-shell event loop with
    unproven IME, AT-SPI and FreeBSD support, and no CSS, so pikr would map
    every style itself.
  - COSMIC/libcosmic: brings COSMIC's own app and theme conventions, which
    pikr's stylesheet would have to bypass.
  - Slint: no layer-shell in its desktop backends, so pikr would own a custom
    platform backend.
  - egui: no mature layer-shell integration, and the whole CSS mapping would sit
    in pikr.
  - A pikr-only frontend on SCTK, tiny-skia and cosmic-text: pikr would own
    surfaces, scaling, XKB/IME, clipboard, accessibility and compositor quirks
    forever.
  - A general toolkit from scratch, or a browser/WebView frontend: scope and
    startup cost out of proportion for a transient launcher.
  - wayr: Wayland-only by design, so pikr would still need separate Windows and
    macOS paths. wayr's own `README.md` still lists pikr as a consumer; that
    needs fixing in the wayr repo (no session was available to do it).

## Release pipeline

- The Alpine `.apk` job in `.github/workflows/ci.yml` (`alpine`, named "Build
  .apk for Alpine (x86_64)") is gated `if: false`, so every tag run skips it and
  no release carries an Alpine package — confirmed on the v0.8.13 run, where the
  job reported "skipped" while the other publish jobs ran. Its own comment gives
  the cause: it downloads the `x86_64-unknown-linux-musl` tarball from the
  release, and that target is not in the binary build matrix (the matrix builds
  linux-gnu, both macOS targets and windows-msvc). Restoring Alpine means adding
  the musl target back to the matrix first, then flipping the gate; the job
  itself is written and untested since the gate went in. Not investigated: why
  musl was dropped — it is absent from the matrix as far back as `417a2e1`
  (2026-05-16, the workflow port from hodl), so no commit in this repo removed
  it. Left as-is 2026-09-15: a deliberate pre-existing gate, not a v0.8.13
  regression.

  Since the crcbl port a plain musl target would not do either: crcbl-vk opens
  `libvulkan.so.1` with `dlopen` (`ash::Entry::load`), which a statically linked
  musl binary (the target's default) cannot do, so it would build and then exit
  "no usable GPU". An Alpine package needs a dynamically linked musl build
  (`-C target-feature=-crt-static`, built on Alpine itself, e.g. in an `alpine`
  container job), checked by running it against Mesa there.

## Hardening (correct today, fragile — not defects)

Re-verified against the tree on 2026-10-18.

- Symlinked-dir mtime cache blindness; coarse-granularity same-tick cache
  staleness; non-atomic state writes; `--lines 0` / `--width 0`; row-key
  Arc-pointer fragility; non-UTF8 PATH names. (review pass 2026-08-06; not
  re-checked individually)
- drun cache hit skips re-validation (`drun::load_cache` returns the cached
  entries once dirs, locales and the mtime key match) — a same-user attacker
  with a predictable cache key gets a second execution path.
- run-mode label ↔ executed-binary mismatch — `run::scan_path` dedupes by name
  across PATH dirs and each entry is `Entry::exec(name, name)`, so spawn
  re-resolves the bare name via PATH at accept and can launch a different binary
  than the one listed.
- A drun cache written before `parse_exec` refused relative `Exec=` programs can
  still list one until the cache is invalidated (any `.desktop` change, or a
  dirs or locale change); the cache has no format version to force it.
- dmenu stdin is unbounded (`dmenu::read_entries`) — a hostile caller piping
  gigabytes bloats the process.

## Accepted trade-offs (not scheduled)

- **Rank cost after precomputed match keys** (`picker::matcher::MatchKey`),
  measured 2026-10-18 with a throwaway release-build probe over emoji mode's
  1914 entries: 168–303 µs per keystroke (865–1190 µs before), plus ~1.8 ms to
  build the keys when the entries load. Declined from the 2026-08-06 perf
  review: a two-phase rank (score everything, compute positions only for the
  shown rows). `Atom::score` and `Atom::indices` cost the same (242 vs 233 µs
  for "a"), so it would save only the survivors' position allocations, ~100 µs
  at 1392 survivors. Also unscheduled: the double sort per keystroke (matcher,
  then frecency) and calc mode rebuilding its ≤101 entries' keys on every
  keystroke.
- drun warm-start residual: the mtime cache key walks every file under the XDG
  applications dirs on every launch. Documented in the perf review; deliberate.
  (The second half of the original entry — the rerank frecency loop forming a
  `payload_key` string per entry — was FIXED by `dd8915a` "perf(frecency):
  precompute usage keys per entry"; see the 2026-08-06 perf review.)
- Icon byte caches (`picker/icons.rs` `file_bytes`/`rasterise_svg`) stat per
  lookup on every row rebuild (~10 µs/keystroke for ~10 visible rows). This is
  the documented freshness mechanism behind the mtime+len invalidation; not
  material on its own — revisit only if profiling shows it.

## Windows support pass 2026-09-15

First session run on a Windows 11 host; the `#[cfg(windows)]` arms were
compiled, tested and exercised here, and CI now runs the test suite on ubuntu,
macOS and Windows.

### Open

- **`-P` still leaks through filtering.** Masking the calc live row and dropping
  match highlights (`AppState::hide_match_positions_if_password`) closed the
  direct echoes, but which dmenu / calc-history rows stay visible still reveals
  which rows match the secret, and calc's dedupe hides a history row exactly
  equal to the live expression. Inherent to filtering on a secret; the options
  are to document it or to stop filtering under `-P` (show all rows unranked).
  Needs a product call.
- **Windows startup is past the 500 ms usable-picker target.** Release build on
  crcbl 0.3.0/D3D12, 2026-10-18: three launches reached first paint at 794, 869
  and 990 ms (`RUST_LOG=pikr=debug`); earlier figures were ~1.1 s at `6ca46474`
  and 574–682 ms at `4c53204e`, and the runs are too noisy to call that a
  regression. GPU setup is nearly all of it (see "GUI on crcbl"); config,
  entries, ranking and fonts are done within ~50 ms.

### Coverage gaps (not verified)

- Window posture (issue #9): at crcbl 0.3.0 the visible window is unowned,
  `WS_EX_TOOLWINDOW` and `WS_EX_TOPMOST`, without `WS_EX_APPWINDOW` (read with
  `GetWindowLongPtr`), and no taskbar button showed in a screenshot. The Alt-Tab
  switcher itself was not opened.
- drun accept on Windows was not driven through the GUI keystroke path; only
  `drun_windows::launch` was called directly (Notepad from its WindowsApps
  package and `cmd.exe` from its known-folder id both started). dmenu, emoji and
  calc accepts WERE driven end-to-end with synthesized keys.
- History/frecency persistence on Windows is covered by the path tests
  (`state_file_path_resolves_under_a_pikr_dir`,
  `default_path_resolves_under_a_pikr_dir`); a real accept writing
  `%LOCALAPPDATA%\pikr\history.toml` was not observed (the session lost its
  foreground window before that probe ran).
- `--kb-custom KEY=PROMPT` with no matching row now shows the confirm card in
  the empty-state row; its on-screen placement was not looked at. The e2e test
  `kb_custom_confirm_with_no_match_prints_query` covers the behaviour (CI,
  Linux).
- run, ssh and clipboard modes were not exercised on Windows beyond unit tests.

## review review 2026-08-06

Whole-codebase correctness pass (clean tree). Findings verified by the
orchestrator at the cited lines, including an empirical probe of the symlink
semantics. Both findings FIXED 2026-08-06 (run symlinks `aebd016`; Windows
dedupe `9cc7dfb`).

### Cleared (sub-agent-traced, not independently re-verified by orchestrator)

- ssh inline-comment hosts — matches ssh's own token semantics.
- visual-anchor staleness, calc row-key pointer reuse, marker parsing, deadlock
  batching, `%20` field codes — traced through, safe.

### Coverage

- Whole codebase read: entry/app/cli/config, all modes, picker, ui, xtask,
  tests. Windows/macOS arms read but NOT executed locally — the icon-cache PNG
  write race (fixed 2026-09-15) was in that gap.

## audit review 2026-08-06

Whole-codebase security audit (clean tree). Verdict: no critical/high/medium;
four low findings. Findings 1-2 and 4 re-verified by the orchestrator (finding 1
empirically: evalexpr probe crashes the 8 MB stack at ~100k chars of nested
parens, survives at 80k); finding 3 is Windows-only, code-traced, not
runtime-verified. Findings 1, 2, 4 FIXED 2026-08-06 (calc cap `b65c2d9`, field
codes `c7d1c14`, 0600 state `081faf4`/`acc5e0c`); finding 3 (all-users Start
Menu writable → planted shortcut) CLOSED 2026-09-15: `icacls` on a Windows 11
host shows `BUILTIN\Users:(RX)` on `%ProgramData%\...\Start Menu\Programs`, so
only admins can plant there. drun now lists `shell:AppsFolder` — exactly what
Start itself launches — so pikr adds no path Start does not already expose.

### Cleared

- **Command injection in `.desktop` Exec / run-mode / ssh payloads** — every
  execution path is `Command::new(program).args(args)` (`modes/mod.rs:137-143`),
  argv, no shell; the only `sh -c` is the cliphist pipe
  (`modes/clipboard.rs:75`) where the interpolated value is a `u64`.
- **ssh terminal quoting** — pwsh arm single-quotes with `''` doubling; cmd `/K`
  arm gated on `is_safe_host` charset `[A-Za-z0-9._\-:[\]]` with argv fallback;
  wt/unix arms are pure argv. Regression tests cover `foo; notepad` and metachar
  fallback.
- **SVG external-resource exfiltration** — usvg `Options::default()` has
  `resources_dir: None` and an empty fontdb; the default resolver reads only
  image-format files and the result is rendered locally, never emitted.
- **Calc integer overflow / div-by-zero panics** — evalexpr uses checked
  arithmetic returning errors; `Exp` promotes to `powf` with a `is_finite()`
  guard at `calc.rs:43`.
- **Count-prefix / selection overflow** — `push_count_digit` saturates
  (`state.rs:86`), `move_down_selection` saturates, frecency clamps to
  `u16::MAX`.
- **Zero-size SVG scale division** — guarded at `icons.rs:253-255` with a
  regression test.
- **drun cache poisoning** — cache lives in the user's own state dir; stale
  mtime serves stale _benign_ entries; writing the cache requires the same user
  who owns `.desktop` execution.
- **Mutex deadlocks in the UI** — all `state.lock()` sites are on the single
  main thread; the blink thread touches no shared state; `query_sig.set` under
  the lock is batched with a regression test; history-recall drops the guard
  before `query_sig.set`.
- **CSS injection via config values** — values land in a fixed template parsed
  by hjkl-css with a whitelisted property set; user's own file anyway.

### Coverage

- Entry points walked: CLI args, config TOML, XDG config/state files, dmenu
  stdin, `.desktop` parsing + Exec + Icon, ssh config + `$TERMINAL`, subprocess
  spawns, clipboard, calc eval, history/frecency persistence, icon loading,
  floem UI event loop, blink thread, ex-mode commands, `--message` modal.
  Classes walked: injection, memory/resource, crypto, AuthZ/AuthN, data
  integrity/TOCTOU, error handling, concurrency.
- NOT audited in depth: the ~916 lines of test code (`tests/e2e/*`, smoke,
  keyboard, startup_gate) — dev harnesses, no production paths. Windows-only
  code (`drun_icons_windows.rs`, `console_attach.rs`, drun `windows_impl`)
  cannot be compiled or executed on this Linux host — the Start Menu ACL finding
  is code-traced only.
- NOTE: the audit agent reported `docs/performance-review.md` missing; it exists
  at the repo ROOT as `performance-review.md`.

## review review 2026-08-06 (evening sweep)

Whole-codebase correctness pass (clean tree), evening re-run, verified by the
orchestrator at the cited lines (including the installed xdg-3.0.0 crate
source).

### Cleared (suspected, traced, disproved — verified by the orchestrator unless marked)

- **`xdg::place_state_file` truncating the drun cache on probe** — verified
  against xdg-3.0.0 source: `write_file` (base_directories.rs:739-745) only
  `create_dir_all`s the parent, never opens the file.
- **Matcher non-ASCII position mapping** — `matcher.rs:135-154` NFCs each
  grapheme into one `text_buf` slot; `grapheme_positions_to_codepoints`
  (158-167) walks the same grapheme stream expanding matched indices to full
  codepoint ranges; tests pin the shapes (258-269).
- **Mutex/batch deadlock discipline** — rerank (`view.rs:1048-1054`) and
  `switch_mode` (1329-1337) create the guard inside `Effect::batch`; history
  recall drops the guard before `query_sig.set` (1632, 1659); the
  `state.rs:112-149` regression test deadlocks if the batch is ever removed.
- **`-P` persistence gates** — all four accept paths (keyboard `view.rs:1469`,
  dmenu fallthrough 1498, AcceptCustom 1526, click 349) gate
  `usage.record`/`history.push` on `!s.password`.
- **calc live/history index math** — `view.rs:753-789`: `live_offset` is
  consistent between `self.entries` construction and both match paths; `m.index`
  always stays `< entries.len()`; truncation only touches `matches`.
- **`g_pending` lifecycle** — `view.rs:1359-1376`: only set in Normal
  (1370-1373); any other key clears it (1374-1376) before `StartEx`; a mode
  switch via `:mode` can't carry a pending `g`.
- **Accept with no matches (non-dmenu)** — `view.rs:1508-1516` executes nothing
  and leaves the picker open (rofi parity); dmenu gets its own fallthrough
  (1491-1507) returning the typed query.
- **frecency cross-variant key collision** — `frecency.rs:136-160`: documented
  non-injective across variants (146-149) but each mode emits a single variant
  and tables are keyed per mode.
- **drun first-wins dedupe** — `insert_first` (`drun.rs:113-115`) dedupes during
  iteration before the label sort (100); user-local wins.
- **`pick_locale` fallback chain** — `drun.rs:129-161`: `C`/`POSIX`/empty fall
  through; modifier + territory-only steps emitted per spec; tests pin shapes.
- **ssh user/system dedupe** — `ssh.rs:52-55`: user config parsed first
  (`config_paths`), stable sort + `dedup_by` keeps the user copy.
- **`--filter` cursor placement** — `app.rs:107-110`: `query_cursor` set to char
  count, matching the char-index caret invariant.
- **dmenu CRLF** — `BufRead::lines()` strips `\r\n` per std docs.
  (sub-agent-traced, std behavior)
- **row_key Arc-pointer staleness (calc)** — the live previous `imbl::Vector`
  keeps old entry Arcs alive across the `rev` bump on the single UI thread, so
  allocator reuse of a dropped pointer can't occur while it is referenced.
  (sub-agent-traced)
- **Windows drun dedupe-before-sort** (`drun.rs:621-623`) — parallels the unix
  path. (sub-agent-traced; Windows-only, not executed)

### Coverage

- Read in full: `apps/pikr/src/` (app, cli, config, console_attach, main; all of
  modes/, picker/, ui/), `xtask/src`, `tests/` (keyboard, smoke, startup_gate,
  e2e incl. harness).
- Windows-only, read but NOT compiled/executed (Linux host):
  `drun_icons_windows.rs`, `console_attach.rs`, and the `#[cfg(windows)]` arms
  of drun/ssh/clipboard/run. The agent flagged `hicon_to_png`'s unchecked
  `stride = width*4` u32 multiply (`drun_icons_windows.rs:185`) as untested in
  that gap; icon sizes are tiny in practice — not reported as a finding.
- Build/test gate not run (report-only pass); verification is line-by-line
  tracing. The one dependency-behavior claim (`xdg::place_state_file`) was
  verified against the installed crate source.

## audit review 2026-08-06 (evening sweep)

Whole-codebase security audit (clean tree, evening re-run). Verdict: no
critical/high/medium/low findings; overall risk low. The drun cache 0644 finding
(the only divergence from the state-file 0600 policy) was FIXED 2026-08-12: both
`write_cache` arms now route through `picker::write_private_state`, with
regression test `cache_written_0600_like_state_files`. The remaining hardening
item was verified by the orchestrator at the cited lines (the perms one
empirically on this host).

### Cleared (suspected, traced, disproved — verified by the orchestrator unless marked)

- **`sh -c` in the clipboard pipe** (`clipboard.rs:60-74`) — the only
  interpolated value is `{id}`, parsed as `u64` from `cliphist list`; digits
  only, no metacharacters can reach the string.
- **`.desktop` Exec / run-mode / ssh payload injection** — every execution path
  is `Command::new(program).args(args)` with stdio nulled
  (`modes/mod.rs:183-192`); no shell on any accept path.
- **ssh pwsh/cmd arms** — pwsh single-quotes with `''` doubling; cmd `/K` gated
  on the `is_safe_host` charset (excludes `=`, so `-oProxyCommand=…` cannot
  form); bare option flags error out with no host. Tests pin the metachar cases.
- **`$TERMINAL` env** — shlex-split into argv; a hostile value fails
  `is_tool_installed` and falls through to candidates. Probe-then-spawn TOCTOU
  is same-user.
- **calc eval depth** — `MAX_EXPR_LEN` 4096 (`calc.rs:22`) rejects before
  evalexpr; prior audit empirically crashed the 8 MB stack only at ~100k chars.
- **Ex-mode command injection** — `:command` matches a fixed whitelist
  (`view.rs:1302-1315`); `q`/`q!` quit, mode names switch modes, anything else
  is a no-op.
- **`-P` persistence gates** — all four accept paths gate
  `usage.record`/`history.push` on `!s.password`; no log line emits query or
  password text (only lengths/counts).
- **SVG external-resource exfiltration** — usvg default resolver reads only
  image-format files, rendered locally, never emitted. (sub-agent-traced vs
  installed usvg-0.48.1 source)
- **`.desktop` walker symlink loops** — `freedesktop-desktop-entry-0.8.1`
  canonicalizes and keeps a `visited` set, with a regression test.
  (sub-agent-traced)
- **State-file permissions** — verified on this host: all three state files 0644
  but `~/.local/state` is 0700; history/usage are re-chmod'd to 0600 on next
  save.
- **`matches`/`entries` index divergence** — rebuilt together under the single
  `AppState` mutex on the single UI thread; no interleaving window.
- **Icon absolute-path handling / matcher grapheme `expect` / `pick_locale`
  lookup strings** — non-executed, non-panicking paths.

### Coverage

- Entry points walked: CLI args, config TOML, XDG config/state files, dmenu
  stdin, `.desktop` parsing (Exec/field-codes/Icon/locale), ssh config +
  `$TERMINAL`, subprocess spawns, calc eval, icon loading (SVG + PNG/JPEG),
  floem event loop + key events, ex-mode commands, `--message` modal,
  `--password`, Windows `.lnk` parse + icon extraction. Classes walked:
  injection, memory/resource, crypto, AuthZ/AuthN, data integrity, error
  handling, concurrency.
- NOT audited: the `tests/` tree (~916 lines of dev harnesses, no production
  paths). Windows-only code (`drun_icons_windows.rs`, `console_attach.rs`,
  `#[cfg(windows)]` arms) read but NOT compiled/executed on this Linux host —
  the `hicon_to_png` stride analysis is code-traced only.
- Summary: 0 findings (0/0/0/0). Overall risk low — every execution path is
  argv-based, the cross-domain inputs (clipboard, dmenu stdin) are handled
  safely, and the tracked items remain the only knowns. Fix first (1) — drun
  cache through `write_private_state` — DONE 2026-08-12; (2) overflow-checked
  buffer size in `hicon_to_png` — DONE 2026-09-15. Remaining: (3) tracked dmenu
  stdin cap / cache re-validation if the threat model tightens.
