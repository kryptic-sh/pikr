# Backlog

## GUI on crcbl

- **The port to crcbl landed on 2026-10-16.** floem, its winit fork, resvg,
  `freedesktop-icons`, `image`, `hjkl-css` and `imbl` are gone. Every crcbl
  crate in the workspace `Cargo.toml` is on the same crates.io release; bump
  them together. If a fix pikr needs is only on crcbl main, pin every crate to
  the same CI-green rev and add the repo back to `deny.toml`'s `allow-git` until
  a release carries it.
- **Windows transparency is D3D12 only.** crcbl draws a transparent Win32 window
  through a DirectComposition swapchain and refuses one on Vulkan, which pikr
  does not use on Windows. From crcbl 0.6.0 `ui::run`'s `update_input_region`
  gives a transparent window a rounded input region (`PhysicalRect::rounded`),
  so a click on a see-through corner reaches the window beneath; on Win32 the
  region also clips drawing, so it is the corners' outer bound. Checked on the
  Windows test desktop against crcbl main `588ce48b`: `GetWindowRgnBox` reports
  a complex region covering the 720x447 window, and a magnified screenshot shows
  the rounded border whole with the desktop behind the corner. Hit-testing
  (`WindowFromPoint`, which applies window regions) puts the outermost corner
  pixel on the window beneath and the top edge and centre on pikr; no real click
  was sent.
- `picker::icons::tests::svg_text_is_drawn` needs a system sans-serif face; the
  Ubuntu test job installs no fonts, so it relies on the runner image.
- **GPU choice is crcbl-app's rule (crcbl 0.6.0).** `ui::gpu::Gpu::open` ranks
  adapters with `crcbl_app::adapter::choose`: `CRCBL_ADAPTER` if set, else the
  adapter driving the window's display when it is discrete, else `LowPower`
  (integrated before discrete), so a laptop's dGPU can sleep. On the Windows
  test desktop `Instance::display_adapter` named the RX 7900 XTX, which drives
  the display, and it was chosen. Not verified: a hybrid laptop (none to hand),
  and Wayland, X11 and macOS, where crcbl's `display_adapter` answers `None` so
  the preference alone decides. Not handled: a window moved to another GPU's
  display after start (pikr is short-lived).
- **Unverified from pikr after crcbl 0.6.0.**
  - Regional-indicator flags (🇺🇸) draw as two letters with Segoe UI Emoji, which
    has no flag ligatures; Noto Color Emoji draws them. Not a crcbl bug.
  - crcbl-ui now lays out right-to-left text (UAX #9), breaks lines by UAX #14
    and moves the caret by UAX #29 clusters, and SVG `<text>` is shaped with
    harfrust. Checked from pikr on Windows (2026-10-20, Segoe UI fallback):
    Hebrew and Arabic dmenu rows draw in visual order, Arabic shaped and joined,
    and `mixed: hello עולם 123` draws as `mixed: hello 123 עולם`, UAX #9's
    answer. Rows stay left-aligned whatever their direction (pikr's row layout,
    as rofi does). Breeze icons (KDE `breeze-icons` at `6ee38f1`, 2026-10-20):
    the six of its SVGs that still hold `<text>` (applets-template, kbruch,
    khangman, kile, kwave, xorg) all render through `crcbl_image::svg::decode`
    with `svg-text` at 96 px, every label in place, on Windows with its system
    fonts. Not checked: SVG text in scripts beyond Latin. crcbl's own gaps:
    arrow keys move in logical order, and SVG text clusters are approximate.
  - **Right-to-left text in the query puts the caret in the wrong place.**
    Pasting `שלום עולם` and pressing Left three times (logical index 6 of 9)
    drew the caret at the text's right edge, not inside the second word
    (Windows, 2026-10-20). `ui::view::query` draws the caret by splitting the
    query into before/caret/after spans in a left-to-right row, so bidi cannot
    reorder across them. A fix needs the caret's position from crcbl-ui's text
    layout (its own text field places its caret that way) rather than span
    splitting.
  - IME, checked on Windows 2026-10-20 with the Microsoft Japanese IME (keys
    synthesized while pikr held the foreground): the pre-edit draws inline and
    underlined at the caret, the candidate list opens right below the caret, the
    Enter that commits does not reach pikr, and the commit arrives whole (a
    second Enter printed `日本語`). One crcbl bug, reported: Windows also draws
    its own composition window beside pikr's inline pre-edit, so the reading
    shows twice (crcbl leaves `ISC_SHOWUICOMPOSITIONWINDOW` set). Not checked:
    Wayland's text-input-v3 and AppKit.
  - `ui::run::headless` drives named keys through `HeadlessShell::key_press`. An
    "events due at pump N" queue, which would let those tests script input
    across frames, is in crcbl's backlog, unscheduled.
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
- **Open findings from the 2026-10-19 review of `ui/` and `picker::session`**
  (one read-only agent; the two confirmed defects it found, the panel border
  missing from `ui::view::window_height` and the "square corners" wording of the
  opaque-fallback warnings, are fixed). Not re-verified by the orchestrator:
  - Cleared 2026-10-19: click attribution. `ui::view::result_row` reads
    `Ui::clicked` after its `.row` closes, so it reads the enclosing block,
    which is the per-row `.list-row` crcbl-ui's `Ui::list` opens around each row
    (`tree/widgets/list.rs`, documented to make `clicked` read the row). In
    `ui::run::Window::frame` the click comes from the first build; the second
    runs with the pointer at rest.
- **Open findings from the 2026-10-19 review of `modes/`** (one read-only agent;
  its ssh findings and the drun locale-modifier and deprecated field-code ones
  are fixed). Not yet fixed or re-verified by the orchestrator:
  - Fixed 2026-10-19, behaviour worth knowing: on Windows
    `modes::spawn_detached` runs inside `console_attach::without_console` (free
    the console, clear the std handles, spawn, re-attach) and passes no stdio.
    Measured with a GUI-subsystem probe on Windows 11: `pwsh -NoExit` and
    `cmd /K` given NUL handles closed within 4 s, with or without an attached
    parent console; spawned that way they stayed open, and the probe
    re-attached. Not checked: typing into the opened shell, and that pikr's own
    output after the spawn still reaches the launching terminal.
  - Cleared 2026-10-19: the review's guess that
    `drun::icons_windows::hbitmap_to_png` should treat `IShellItemImageFactory`
    bitmaps as premultiplied. The 139 icons in this Windows 11 host's cache,
    stored byte-for-byte as `GetImage` returned them, hold 15,071 partly
    transparent pixels, 7,105 of them with a colour channel above alpha (in 110
    icons), which premultiplied data cannot have. `Alpha::Straight` is right
    here; another Windows version or shell extension was not checked.
  - Not done, by design so far: `TryExec`, `OnlyShowIn`/`NotShowIn`; no `--`
    before the ssh host (the config is trusted); no `setsid` for Unix spawns.
  - ssh has no `known_hosts` parsing and no `Include` support; `~/.ssh/config`
    files that split hosts across `Include`d files list only the main file's.
- **Not verified (coverage gaps).**
  - No whole-codebase correctness or security review since the crcbl port. The
    last ones (2026-08-06) read the floem UI, which is gone; `ui/` and
    `picker::session` (the event loop, view, key dispatch, `-P` prompt and IME
    handling) have only had the review of the commits that wrote them.
  - Linux was verified only through CI's sway session on lavapipe: the e2e
    keyboard suite and the startup gate passed (run 39002212977, 237 ms
    launch-to-result against the 500 ms deadline). Hyprland, niri and KWin were
    never run, and no Linux run was looked at by eye.
  - Key handling is tested headless on every OS (`ui::run::headless`: crcbl's
    `HeadlessShell` and crcbl-hal's null backend driving the real window loop),
    which puts no frame on a screen and no key through a real keymap. The sway
    suite in `tests/e2e` keeps only what needs the binary: stdin and
    `--loading`, the config file, state files, and keys arriving through
    Wayland. crcbl's `HeadlessShell` gives named keys (Enter, Escape, arrows) no
    keysym, so those tests inject raw `ShellEvent::Key`s with X11 keysyms. The
    loop handles every scripted event in its first pump, so these runs end
    before a frame is drawn: layout, presenting and the swapchain's out-of-date
    retry (`ui::run`, which now waits `PRESENT_RETRY` before redrawing rather
    than spinning) are not exercised by them. Scripting input across frames
    would need a hook in the loop.
  - With CI stalled org-wide (no kryptic-sh run started since 2026-10-19
    morning), the Linux suite was run locally in WSL (Ubuntu 26.04, uutils
    coreutils) with `WAYLAND_DISPLAY`, `DISPLAY` and `WAYLAND_SOCKET` unset and
    an empty `XDG_RUNTIME_DIR`, so no test could reach WSLg: every test passed
    at `0cb21fa` (288 unit, 10 smoke). The sway suite and the startup gate were
    skipped there (no sway or wtype). Run WSL tests that way: with WSLg's
    `wayland-0` reachable, a test that launches pikr opens a real window on the
    Windows desktop.
  - FreeBSD is only type-checked: CI's Ubuntu clippy job lints the
    `x86_64-unknown-freebsd` target. Nothing links or runs it.
  - macOS: CI builds it and runs the unit tests and the `--version`/`--help`
    smoke tests; the window was never opened.
  - Windows (D3D12): the picker, match highlighting, a scrolled list with its
    scrollbar and the `--message` modal were rendered and looked at in
    screenshots. Keyboard input, clicks, the wheel and drun icons were not
    exercised: pikr launched from a background shell never got the foreground,
    and synthesized keys would have gone to another window. A launch Windows
    should grant the foreground (from a hotkey tool or a foreground terminal)
    was never tried, so pikr receiving focus on Windows is unverified; only the
    refused case was checked (`FocusRefused` and no `Focus`, crcbl 0.4.0+).
  - No icon from a real icon theme was drawn; icon decoding is covered by unit
    tests only. HiDPI (scale ≠ 1) was not exercised.
  - IME: pikr draws `TextPreedit` at the query caret (`ui::view::Preedit`) and
    sends the caret through `set_text_input_area` after every frame that moves
    it (`ui::run`). Unit-tested only for the pixel conversion (`device_rect`);
    no real input method was driven through it, on Win32 (Microsoft IME, which
    crcbl's Win32 backend has also never met) or through text-input-v3 on
    Wayland. Check: composition underlined at the caret, candidate list beside
    it, Backspace inside a composition editing the composition and not the
    query, and the commit landing whole.
  - Accessibility reaching AT-SPI (crcbl's AccessKit row) was never checked from
    pikr.
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

- Package runtime dependencies (`pkg/nfpm/nfpm.yaml.in`, `pkg/aur/`,
  `pkg/alpine/`) were taken from the libraries crcbl 0.6.0 `dlopen`s
  (crcbl-shell, crcbl-vk, crcbl-font sources) and the package names from memory
  of each distribution; no package was installed on a clean system to prove the
  list complete or the names right. CI builds the `.deb` and `.rpm` but does not
  install them.
- **The Alpine `.apk` is restored but unverified (2026-10-19).** The
  `build-musl` job in `.github/workflows/ci.yml` builds
  `x86_64-unknown-linux-musl` in an `alpine:latest` container with
  `-C target-feature=-crt-static` (a static musl binary cannot `dlopen`
  libvulkan) and `GETTEXT_SYSTEM=1` (musl implements gettext), checks the result
  with `ldd` and `--version`, and uploads the tarball the `alpine` job now
  packages (its `if: false` gate is gone). `publish-github-release` needs it, so
  a broken musl build blocks a release. Pushed with CI stalled org-wide, at the
  user's call: it has never run. It runs on every main push, so check its first
  run before cutting a tag. Not checked even then: the package installed on a
  real Alpine system and drawing a window against Mesa.

## Hardening (correct today, fragile — not defects)

Re-verified against the tree on 2026-10-18.

- Symlinked-dir mtime cache blindness; coarse-granularity same-tick cache
  staleness; non-atomic state writes; `--lines 0` / `--width 0`; non-UTF8 PATH
  names (skipped by `run::scan_dir`). (review pass 2026-08-06; not re-checked
  individually)
- No accept path runs a shell on a string pikr builds from untrusted text:
  launches are `Command::new(program).args(args)`. The two script strings are
  clipboard's `cliphist decode {id} | wl-copy` (`clipboard::make_entry`, `id`
  parsed as `u64`) and ssh's pwsh and cmd arms (`ssh::build_terminal_args`: pwsh
  single-quotes the host, cmd takes only hosts in `is_safe_host`'s charset).
  Re-checked 2026-10-19; keep it that way.
- drun cache hit skips re-validation (`drun::load_cache` returns the cached
  entries once dirs, locales and the mtime key match) — a same-user attacker
  with a predictable cache key gets a second execution path.

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

- **Windows startup is under the 500 ms target in the median, with outliers.**
  Release build on crcbl 0.6.0 (lazy adapter description, display-adapter
  choice), D3D12, RX 7900 XTX + iGPU + WARP, 2026-10-20: ten warm background
  `--dmenu` launches reached first paint at 329, 480, 384, 515, 345, 367, 963,
  2080, 356 and 339 ms, median ~376 ms (`RUST_LOG=pikr=debug`; other sessions
  were compiling on the machine, which may explain the two outliers). On crcbl
  0.5.0 three launches gave 617, 443 and 493 ms. GPU setup is nearly all of it;
  config, entries, ranking and fonts are done within ~50 ms.
- **crcbl-app: only its adapter choice is adopted (crcbl 0.6.0).** pikr uses
  `crcbl_app::adapter::{choose, pin}` in `ui::gpu::Gpu::open` and nothing else.
  The rest would replace `ui::gpu` and the window bring-up in `ui::run`:
  `window::wait_for_configure` now hands pumped events to a sink (fixed for
  0.6.0 after pikr's report), and `Presenter` gained `Transparency::Preferred`,
  an opaque fallback with a warning, which matches pikr's rule. Both objections
  from 2026-10-19 are answered; what is left is `text_input::TextPump`, which
  feeds crcbl-ui's `TextInput` while pikr edits its own query. Moving `ui::gpu`
  onto `Presenter` is open and not reviewed against pikr's code.

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
