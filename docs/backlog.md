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
  does not use on Windows. Pixels with alpha 0 (outside the panel's rounded
  corners) still take the pointer. Rounded, see-through corners were confirmed
  in a `--dmenu` screenshot at `4d48eaf4`.
- `picker::icons::tests::svg_text_is_drawn` needs a system sans-serif face; the
  Ubuntu test job installs no fonts, so it relies on the runner image.
- **Render on the GPU that drives the window's display (asked 2026-10-19).** On
  a laptop the internal panel hangs off the iGPU, so pikr renders there and the
  dGPU can stay asleep; on a desktop, or a laptop's external monitor wired to
  the dGPU, the display's own GPU avoids copying every frame across adapters.
  When that GPU cannot be found, prefer the low-power (integrated) adapter.
  Today `ui::gpu::Gpu::open` takes the first adapter crcbl lists that can
  present (the RX 7900 XTX on the Windows test desktop, which does drive its
  display); it neither knows the display's GPU nor prefers low power.
  Mechanisms, from the platform APIs and not yet checked in crcbl: Win32, the
  DXGI adapter whose outputs include the window's monitor (`MonitorFromWindow`);
  Wayland, the compositor's main device from linux-dmabuf feedback matched
  against Vulkan's DRM device properties; macOS,
  `CGDirectDisplayCopyCurrentMetalDevice`. crcbl decided (2026-10-19) to build
  it into its D3D12 start-up slice: adapters described lazily in preference
  order, the display's adapter first through an `Instance` query for a surface's
  adapter, then integrated before discrete, `CRCBL_ADAPTER` still overriding.
  pikr then opens on what that returns; lyr follows the same rule. Not handled:
  a window moved to another GPU's display after start (pikr is short-lived).
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
  - Win32 reports keys the input method consumes (`VK_PROCESSKEY`) as ordinary
    `ShellEvent::Key`s at 0.5.0; crcbl drops them from 0.6.0, as Wayland already
    does. Until then `ui::run` ignores key presses while a pre-edit is showing;
    remove that guard with the 0.6.0 bump.
  - D3D12 startup (crcbl's S13): `Dx12Instance::open` still creates one driver
    device per hardware adapter before returning, and pikr pays for each of
    them. crcbl 0.4.0 reuses the probe device for the first open, which took
    `create_device` from ~257 ms to near nothing; see the Windows startup entry.
    crcbl's remaining option, describing adapters lazily, would change
    `Instance::adapters`' contract and is undecided on their side.
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
  - Win32 IME: crcbl passes every `WM_IME_*` to `DefWindowProc` and never clears
    `ISC_SHOWUICOMPOSITIONWINDOW`, so the system composition window may draw
    over pikr's inline pre-edit (confirmed from crcbl's source by its session,
    not seen on screen). Tracked in crcbl's backlog; seeing the reading twice
    during the Microsoft IME check is this bug.
  - Not reviewed by it: the list's click attribution under the double build in
    `frame`, and `modes/`.
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
  `pkg/alpine/`) were taken from the libraries crcbl 0.5.0 `dlopen`s
  (crcbl-shell, crcbl-vk, crcbl-font sources) and the package names from memory
  of each distribution; no package was installed on a clean system to prove the
  list complete or the names right. CI builds the `.deb` and `.rpm` but does not
  install them.
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

- **Windows startup is at the edge of the 500 ms usable-picker target.** Release
  build on crcbl 0.4.0/D3D12 (RX 7900 XTX + iGPU + WARP), 2026-10-18: three
  background `--dmenu` launches reached first paint at 539, 457 and 460 ms
  (`RUST_LOG=pikr=debug`), against 794, 869 and 990 ms on crcbl 0.3.0 earlier
  the same day; the runs are noisy. GPU setup is nearly all of it (see "GUI on
  crcbl"); config, entries, ranking and fonts are done within ~50 ms.
- **crcbl-app considered and not adopted (2026-10-19, crcbl 0.5.0).** It would
  replace `ui::gpu` and the window bring-up in `ui::run` with `Presenter`,
  `open_window` and `wait_for_configure`, but as of 0.5.0 it would undo three
  things pikr does on purpose:
  - `window::wait_for_configure` counts and discards the events it pumps; pikr's
    `wait_for_size` keeps them, which is how an early `FocusRefused` and the
    first scale factor reach the loop (reported to crcbl).
  - `Presenter` skips an adapter whose surface offers no premultiplied alpha for
    a transparent window and fails when none does; pikr's rule is to warn and
    draw opaque instead (`ui::gpu::Gpu::open`).
  - `text_input::TextPump` feeds crcbl-ui's `TextInput`, while pikr edits its
    query itself (vim modes, history recall, `-P`). What it would add, backend
    fall-through, matters little here: pikr opens one backend per OS, and on
    Windows the alternative (Vulkan) cannot draw a transparent window. Revisit
    if crcbl changes the first two.

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
