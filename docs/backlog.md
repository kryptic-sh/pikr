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
    reorder across them. crcbl main (after 0.7.0) has
    `TextLayout::caret(byte) -> Caret { line, x, top, height }`, bidi-correct;
    once a release carries it, lay the whole query out once and draw the caret
    over it instead of splitting spans.
  - **Confirm injected unsided Ctrl for crcbl.** crcbl main (after 0.7.0) counts
    `VK_CONTROL`/`VK_MENU`/`VK_SHIFT` without a side toward the modifier state;
    on 0.6.0 an injected generic `VK_CONTROL` lost Ctrl, so pikr's on-screen
    paste test sent `VK_LCONTROL` (0xA2, scancode 0x1D). crcbl asked pikr to
    confirm on Windows 11 that a generic `VK_CONTROL` + V pastes once a release
    carries the fix (their change is unit-tested only). It needs synthesized
    keys, so the user's go-ahead.
  - IME, checked on Windows 2026-10-20 with the Microsoft Japanese IME (keys
    synthesized while pikr held the foreground): the pre-edit draws inline and
    underlined at the caret, the candidate list opens right below the caret, the
    Enter that commits does not reach pikr, and the commit arrives whole (a
    second Enter printed `日本語`). On crcbl 0.6.0 Windows also drew its own
    composition window beside pikr's inline pre-edit, so the reading showed
    twice; crcbl 0.7.0 clears `ISC_SHOWUICOMPOSITIONWINDOW`, which pikr now
    uses, but the IME has not been driven on screen since the bump. Not checked:
    AppKit.
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
    launch-to-result against the 500 ms deadline). Hyprland 0.56.2 (Arch,
    2026-10-20, Intel RPL-U iGPU on Vulkan, scale 1) was run and looked at in
    screenshots: the overlay layer maps centred, the corners are see-through,
    and dmenu typing, highlighting and Return work, with Hebrew, CJK and colour
    emoji rows drawn. First paint (`RUST_LOG=pikr=debug`) came at 366 ms on the
    live desktop (fonts 229 ms), 943 ms on the first launch in a nested Hyprland
    (fonts 554 ms, the binary just rebuilt) and 86 ms on the next one. Pointer
    input was driven with `wlrctl` (virtual-pointer) in the nested Hyprland:
    hover lights the row under the pointer with a hand cursor, the wheel scrolls
    the list with the hover following, and a left click accepted the row under
    the pointer after the scroll. niri and KWin were never run.
  - The X11 fallback was run on the nested Hyprland's own Xwayland
    (`WAYLAND_DISPLAY` unset, `DISPLAY` set to it; 2026-10-20): pikr logs "no
    panel windows; opening a toplevel", warns that it draws opaque, and opens as
    a floating 720x448 window. First paint came at 276 ms. Keys sent through the
    compositor's own keyboard
    (`hl.dsp.send_shortcut({ mods = "", key = "b", window = "class:pikr" })`
    through `hyprctl eval`) typed `be`, highlighted `beta`, and Return printed
    it. **wtype does not work for X11 clients**: its uploaded keymap does not
    reach Xwayland, so `wtype be` left pikr in Normal mode with an empty query
    (Escape-like) and the run exited 1. Use `send_shortcut` there. Not checked:
    X11 under a real X server, and transparency with a compositing manager
    (crcbl's X11 shell has no ARGB visual yet).
  - **The first launch after a few idle minutes misses the 500 ms target on
    Linux.** Five drun launches in a row in a nested Hyprland (2026-10-20)
    reached first paint at 1075, 176, 153, 163 and 240 ms; the first one spent
    657 ms in `ui::run::load_fonts`. That function reads every fallback face
    whole (`Font::from_system`): 16 faces, about 70 MB, the two Noto CJK
    collections 46 MB of it. fontconfig already trims the list to faces that add
    coverage. But pikr's own files are not the whole cause: with the fallback
    faces, the theme font, fontconfig's caches and the binary evicted from the
    page cache (`posix_fadvise(DONTNEED)`, no root needed), three launches
    reached first paint at 460, 275 and 274 ms (fonts 282, 224 and 178 ms). The
    rest of the after-idle cost was not found; candidates are Mesa's driver and
    LLVM libraries, GPU power-up, and load from other work on the machine. In a
    probe, reading the faces in parallel (one thread each) took 85 to 184 ms
    evicted against 170 to 205 ms in sequence: a possible saving, not adopted on
    that evidence. Reading faces only when a glyph needs them would need a crcbl
    API for lazily loaded fallbacks.
  - **Run GUI checks in a nested Hyprland, never on the user's live desktop**:
    pikr takes the keyboard on the overlay layer, so a test run there swallows
    whatever the user is typing. Start `Hyprland -c <lua config>` through
    `hyprctl eval "hl.exec_cmd(cmd, { workspace = 'special:x silent', no_initial_focus = true })"`
    (0.56 configures and dispatches in Lua). The live compositor never draws
    that hidden window, so its output gets no frame callbacks: grim hangs and
    the app never gets a frame. Inside it, add `hyprctl output create headless`
    and disable `WAYLAND-1`, then point `WAYLAND_DISPLAY` and
    `HYPRLAND_INSTANCE_SIGNATURE` at it for pikr, wtype, wlrctl (AUR; pointer
    moves are relative, so `move -5000 -5000` first to reach 0,0) and grim. The
    un-drawn output is also a way to reproduce the frame-pacing freeze fixed on
    2026-10-20 (`ui::run`'s `may_present`); turning a headless output off with
    DPMS does not reproduce it.
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
  - While CI was stalled org-wide (2026-10-19 to 2026-10-20; it ran again from
    run 39368692698, green), the Linux suite was run locally in WSL (Ubuntu
    26.04, uutils coreutils) with `WAYLAND_DISPLAY`, `DISPLAY` and
    `WAYLAND_SOCKET` unset and an empty `XDG_RUNTIME_DIR`, so no test could
    reach WSLg: every test passed at `0cb21fa` (288 unit, 10 smoke). The sway
    suite and the startup gate were skipped there (no sway or wtype). Run WSL
    tests that way: with WSLg's `wayland-0` reachable, a test that launches pikr
    opens a real window on the Windows desktop.
  - FreeBSD is only type-checked: CI's Ubuntu clippy job lints the
    `x86_64-unknown-freebsd` target. Nothing links or runs it.
  - macOS: CI builds it and runs the unit tests and the `--version`/`--help`
    smoke tests; the window was never opened.
  - Windows (D3D12): the picker, match highlighting, a scrolled list with its
    scrollbar and the `--message` modal were rendered and looked at in
    screenshots. Keyboard input was driven on 2026-10-20 with keys synthesized
    only while pikr held the foreground: typing, Enter, Ctrl+V paste and the
    Microsoft IME (above). In that run's log pikr's focus was first refused,
    then arrived about three seconds later. Clicks, the wheel and drun icons
    were not exercised, and a launch from a hotkey tool was never tried.
  - Real theme icons and HiDPI were checked on Hyprland at scale 2 (nested,
    2026-10-20): drun rows drew hicolor PNGs (Discord at 256 px, Firefox at 48
    px) sharp, with sharp text, and the layer mapped centred at its logical
    size. At 1.5 crcbl uses `wp_fractional_scale_v1` (`preferred_scale(180)`)
    with a viewport and draws at 1.5 itself; since `ui::run::snapped` the buffer
    is a whole 1080x672 and text is sharp (before that it was 1080x671 for 670.5
    device pixels, and soft). A scale change from 1.5 to 1 while pikr was open
    was picked up in place. Fractional scales other than 1.5 were not looked at.
  - **Startup at a scale other than 1 draws at three sizes** (`WAYLAND_DEBUG=1`
    at 1.5): buffers at 1x, then at the output's integer scale (2, from
    `wl_output.scale`), then at 1.5 once `preferred_scale` arrives after
    `wl_surface.enter`, so the swapchain is rebuilt twice before the frame that
    stays. pikr cannot know the scale before its first commit is on an output;
    crcbl could hold the first present until `preferred_scale` (or
    `preferred_buffer_scale`) comes, or skip the integer step when fractional
    scale is bound. Not measured how much startup this costs. crcbl also sends
    `set_buffer_scale(2)` alongside the viewport there, which Hyprland accepts;
    whether a strict compositor objects (the 1080x672 buffer is divisible by 2,
    an odd `--width` might not be) was not checked.
  - **The generic fallback icon draws blurry: crcbl reads `Type=Scalable` as a
    threshold directory.** `crcbl_image::theme::parse_index` (crcbl 0.7.0, still
    on crcbl main at `0bc5f7cb`) matches `"Scaled"`, but the freedesktop spec's
    value, which Adwaita uses, is `Scalable`. Every scalable directory then
    becomes `Threshold(2)` around its `Size` (Adwaita says 128), so for 24 px
    `application-x-executable` resolves to Adwaita's `16x16/mimetypes` PNG,
    stretched to 24 px at scale 1 and 48 px at scale 2, instead of
    `scalable/mimetypes/application-x-executable.svg`. Found with a throwaway
    probe calling `IconThemes::lookup` the way `picker::icons::IconCache` does.
    Any theme icon that only a scalable directory carries is hit the same way.
    Fixed on crcbl main (2026-10-20, after `0bc5f7cb`, with a regression test
    for this Adwaita case); bump pikr once a crcbl release carries it and check
    the dmenu rows' icon at scale 2.
  - Icon theme order (`picker::icon_theme::chosen`), as the owner set it on
    2026-10-20: pikr's `theme.icon_theme`, then KDE's `kdeglobals`, then GTK 3's
    and GTK 4's `settings.ini`, then `FALLBACK_THEMES`, then hicolor. Not read:
    GNOME's gsettings `icon-theme` (dconf, no file to read) and the XDG settings
    portal; a GNOME desktop that only sets it there gets the fallbacks.
  - IME: pikr draws `TextPreedit` at the query caret (`ui::view::Preedit`) and
    sends the caret through `set_text_input_area` after every frame that moves
    it (`ui::run`). The Microsoft IME on Win32 was checked (see above). On
    Wayland, text-input-v3 was checked on Hyprland 0.56.2 (2026-10-20) with
    fcitx5 and mozc, run against the nested compositor only (its own
    `XDG_CONFIG_HOME`, `DISPLAY` unset, `--disable=dbus,dbusfrontend,xim,…` so
    it stays off the live session): the pre-edit draws inline and underlined at
    the caret, the candidate list opens under the caret, Backspace inside the
    composition edits the composition and not the query (no rerank), and the
    Enter that commits does not reach pikr, the commit arriving whole. Not
    checked: AppKit, and Backspace inside a composition on Win32.
  - **Screen readers cannot see pikr on Linux, and cannot until crcbl ships
    AT-SPI.** Checked 2026-10-20 on Hyprland: with pikr open, the AT-SPI desktop
    listed only xdg-desktop-portal-gtk, waybar and swaync. pikr turns on neither
    crate's `accesskit` feature and never calls `Shell::update_accessibility`,
    and on Linux that would not help yet: crcbl 0.7.0's `accesskit` feature
    brings only `accesskit_windows`, and the Wayland and X11 shells keep the
    trait's default, which drops the tree. crcbl's backlog lists Wayland and X11
    through `accesskit_unix` (with `zbus`) as still to do. On Windows the UI
    Automation path exists in crcbl and pikr could adopt it now: enable
    `accesskit` on crcbl-shell and crcbl-ui, and send `Ui::accessibility_update`
    after each frame (and answer `accessibility_action`). Not started, because
    it adds dependencies, which is the owner's call.
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
  - Arch, checked 2026-10-20: every name in `PKGBUILD-bin.in`'s `depends` and
    `optdepends` exists in the repos (`vulkan-driver` as a virtual provide of
    vulkan-intel, vulkan-radeon and nvidia-utils). A crcbl 0.7.0 release build
    links only libc, libm and libgcc_s. Drawing drun on Hyprland, the libraries
    it mapped (`/proc/<pid>/maps`, owners from `pacman -Qo`) beyond those were
    wayland, libxkbcommon, vulkan-icd-loader and fontconfig, plus what
    fontconfig and the Mesa driver pull in themselves. The template, filled in
    for v0.8.13, builds with `makepkg` and puts `usr/bin/pikr`, the licence and
    the README in place. The X11 fallback (on Xwayland) also mapped
    libxkbcommon-x11, already listed, and `libxcb-cursor`, which crcbl dlopens
    as optional (themed cursors) and which was missing: it is now an
    `optdepends` (xcb-util-cursor, which pulls in xcb-util, -image and
    -renderutil) and a deb (`libxcb-cursor0`) and rpm (`xcb-util-cursor`)
    recommends; those two names are from memory, not checked against the
    distributions. The Alpine `APKBUILD.in` lists no X11 packages at all. Not
    checked: installing the package on a clean system.
- **The Alpine `.apk` is restored; its package step has not run yet.** The
  `build-musl` job in `.github/workflows/ci.yml` builds
  `x86_64-unknown-linux-musl` in an `alpine:latest` container with
  `-C target-feature=-crt-static` (a static musl binary cannot `dlopen`
  libvulkan) and `GETTEXT_SYSTEM=1` (musl implements gettext), and checks the
  result with `ldd` and `--version`; it passed on main in run 39368692698
  (2026-10-20). `publish-github-release` needs it. The `alpine` job that
  packages the tarball (its `if: false` gate is gone) runs only on a tag, so the
  first release cut is its first run: check it then. Not checked: the package
  installed on a real Alpine system drawing a window against Mesa.

## Hardening (correct today, fragile — not defects)

Re-verified against the tree on 2026-10-18.

- Coarse-granularity same-tick cache staleness; `--lines 0` / `--width 0`;
  non-UTF8 PATH names (skipped by `run::scan_dir`). (review pass 2026-08-06; not
  re-checked individually)
- No accept path runs a shell on a string pikr builds from untrusted text:
  launches are `Command::new(program).args(args)`. The two script strings are
  clipboard's `cliphist decode {id} | wl-copy` (`clipboard::make_entry`, `id`
  parsed as `u64`) and ssh's pwsh and cmd arms (`ssh::build_terminal_args`: pwsh
  single-quotes the host, cmd takes only hosts in `is_safe_host`'s charset).
  Re-checked 2026-10-19; keep it that way.
- State writes (`picker::write_private_state`: history, frecency, the drun
  caches) rename a complete temporary file into place but do not `fsync`, so a
  power cut can still lose a save. On Windows the rename fails while another
  process holds the file open without delete sharing; that save is dropped with
  a warning, never half-written.
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
- **crcbl-app: only its adapter choice is adopted (since crcbl 0.6.0).** pikr
  uses `crcbl_app::adapter::{choose, pin}` in `ui::gpu::Gpu::open` and nothing
  else. The rest would replace `ui::gpu` and the window bring-up in `ui::run`:
  crcbl 0.7.0's `Presenter` also paces presents on `Shell::request_frame`; pikr
  now does the same itself (`ui::run`'s `may_present`), after a window that the
  compositor was not drawing froze its loop on Hyprland.
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
