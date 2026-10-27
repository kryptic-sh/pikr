# Backlog

## GUI on crcbl

- **The port to crcbl landed on 2026-10-16.** floem, its winit fork, resvg,
  `freedesktop-icons`, `image`, `hjkl-css` and `imbl` are gone. Every crcbl
  crate in the workspace `Cargo.toml` is on the same crates.io release; bump
  them together. If a fix pikr needs is only on crcbl main, pin every crate to
  the same CI-green rev and add the repo back to `deny.toml`'s `allow-git` until
  a release carries it.
- **crcbl 0.8.1** (2026-10-21) fixes Wayland events that arrive during a Vulkan
  present waiting out `wait_events`' timeout. Measured on Hyprland (nested,
  Intel iGPU) from wtype to pikr's rerank log: keys 300 ms apart took a median 7
  to 10 ms on both 0.8.0 and 0.8.1; a 30-key burst at 25 ms intervals took
  medians of 79 to 89 ms (0.8.0) and 74 to 76 ms (0.8.1), maxima 148 to 160 and
  140 to 146 ms. No timeout-long stall showed on either, so the fix was not
  observed here; the burst numbers assume wtype keeps to its 25 ms interval.
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
- **GPU choice is crcbl-app's rule (crcbl 0.6.0).** `ui::gpu::Gpu::open` opens
  crcbl-app's `Presenter`, which ranks adapters with
  `crcbl_app::adapter::choose`: `CRCBL_ADAPTER` if set, else the adapter driving
  the window's display when it is discrete, else `LowPower` (integrated before
  discrete). On the Windows test desktop `Instance::display_adapter` named the
  RX 7900 XTX, which drives the display, and it was chosen. On Wayland, from
  crcbl 0.8, `display_adapter` comes from the device the compositor composites
  on: on the Arch laptop (2026-10-21, crcbl 0.8.1) crcbl-vk logged "the
  compositor composites on DRM device 0xe280, adapter Some(AdapterId(1))", the
  Intel iGPU, and pikr chose it because that display GPU is integrated. Not
  verified: a Wayland desktop whose display is on a discrete GPU (the rule picks
  it there), and X11 and macOS, where the preference alone was all that was
  seen. Not handled: a window moved to another GPU's display after start (pikr
  is short-lived).
  - **Hybrid laptop, checked 2026-10-21** (the Arch machine: Intel Raptor Lake
    UHD driving eDP and HDMI, NVIDIA GeForce MX550 with no outputs, NVIDIA's
    proprietary Vulkan driver; nested Hyprland). Default: Intel, first paint 264
    ms. `CRCBL_ADAPTER=discrete`: the MX550 renders and presents across to the
    Intel-driven output, first paint 512 ms. `cpu`: llvmpipe, 250 ms. All three
    typed, matched and accepted.
  - **A saved GPU choice skips the other drivers (crcbl 0.12.0).**
    `ui::gpu_choice` keeps crcbl-app's `GpuChoice` in the state file
    `gpu-choice`; `Gpu::open` passes it as `PresenterDesc::gpu_choice`, and
    `ui::run::run` saves a new one (`Gpu::choice_to_save`) after the chosen
    program has started, because `GpuChoice::locate_vulkan_driver` loads every
    driver. A launch someone chose for (`CRCBL_ADAPTER`, or a loader `VK_*`
    driver variable, checked by `gpu_choice::overridden`, which repeats crcbl's
    own rule because crcbl 0.12.0 does not export it) neither uses nor saves
    one. crcbl main `b76abe21` adds `Presenter::gpu_choice_overridden()`
    (unreleased on 2026-10-26): once pikr is on a release with it, delete
    `gpu_choice::overridden` and pass that instead. Checked 2026-10-26 in a
    nested Hyprland on the Arch laptop: the first launch saved Intel with
    `intel_icd.json`; later launches reported `Used` and held no `/dev/nvidia*`
    descriptor (14 without the file); a choice edited to a missing device
    reported `Stale` and was replaced; `CRCBL_ADAPTER=discrete` reported
    `Ignored` and left the file unchanged, and with no file saved nothing. Not
    verified: the D3D12 path on Windows (compiled by CI only), and whether
    opening a saved driver still wakes a runtime-suspended dGPU, which this
    machine cannot test (its MX550 has runtime D3 off and the compositor holds
    `/dev/nvidia0` open all session).
- **Startup profile, 2026-10-26** (release build, nested Hyprland on the Arch
  laptop, Papirus-Dark icons, after the first-page icon warm-up in
  `ui::run::run`). Re-checked the GPU: Intel renders, chosen as above, but every
  launch still holds 14 `/dev/nvidia*` descriptors. What remains of first paint
  (about 100 ms for dmenu, emoji and, from crcbl 0.11.0, drun):
  - **GPU setup** was 80 to 110 ms, mostly the Vulkan loader loading and
    enumerating every ICD; the saved GPU choice above removes that. Measured
    2026-10-26 under load (load average about 7 on 12 cores, other sessions
    building), six dmenu launches each, interleaved: first paint a median 242.5
    ms (218–253) with no saved choice against 172 ms (134–217) with one.
    Rejected before crcbl took this on: re-executing pikr with
    `VK_LOADER_DRIVERS_SELECT` set, `set_var` (unsafe under Rust 2024), a
    vendor-to-driver lookup table (a guess), and a global variable in the shared
    `hyprland.lua` (the dotfiles also run on an AMD machine).
  - **Icon lookup no longer bounds drun (crcbl 0.11.0).** crcbl 0.11.0 indexes
    each icon theme once and reads a symlinked directory once; before it, a name
    Papirus-Dark lacks read every directory of breeze-dark, breeze, hicolor,
    Papirus and Adwaita (105 ms on crcbl 0.9.0). Measured 2026-10-26, ten
    launches each: drun's first paint went from a median 157 ms (148–216) on
    0.9.0 to 103 ms (96–115) on 0.11.0, with dmenu unchanged (87–97 against
    90–105 ms), so drun now paints when the GPU is up, like dmenu. Not tried: a
    lookup that stats the candidate paths, or reads GTK's `icon-theme.cache`,
    instead of listing directories (a Python probe: about 6 ms of stats against
    145 ms of listings for one miss).
  - **Fonts now bound first paint.** With the saved GPU choice, first paint came
    8 to 30 ms after `startup fonts loaded` in every launch measured above
    (fonts 125–187 ms under that load, 66–88 ms on a quieter machine earlier the
    same day). `ui::run::load_fonts` reads every fallback face whole (see the
    loaded-machine entry below). Options, in order of gain: fallbacks loaded
    only when a glyph needs one (a crcbl API), font files mapped instead of read
    (crcbl), or the faces read in parallel (pikr alone; a probe was faster only
    with the files evicted, a warm launch was not measured). Lazy fallbacks are
    on crcbl main (`ee2f614b`, unreleased). Tried through a local `[patch]` on
    2026-10-27 (`Font::from_system_lazy` in `ui::run::load_fonts`): dmenu's
    resident size fell from 112 to 44 MB and drun's from 122 to 48 MB, the font
    thread finished at 17–38 ms, no fallback face was read for Latin rows, and
    a 日本語 row read 3 of 20 and drew correctly. First paint barely moved,
    because the icon thread then bounds it: its first lookup builds
    crcbl-image's `IconThemes` index in 80–105 ms (load average about 4), in
    dmenu too, since an iconless row draws the generic fallback icon. Asked
    crcbl for a lookup that stats candidate paths, or an index cached on disk.
    crcbl `3a80d84d` (unreleased) reads GTK's own `icon-theme.cache` instead.
    With both, tried the same way (five launches each, interleaved with 0.9.4,
    load average 3–6): first paint fell from a median 86 ms (75–188) to 34 ms
    (30–43) in dmenu and from 127 ms (91–210) to 44 ms (33–51) in drun; the icon
    warm-up took 1 ms in dmenu and 13–26 ms in drun; resident size was 32 MB in
    dmenu and 33 MB in drun. The GPU now bounds first paint (presenter ready at
    24–40 ms, paint 3–10 ms later). Adopt both once a crcbl release carries
    them: `Font::from_system_lazy` in `ui::run::load_fonts`; the icon change
    needs no pikr code.
  - **Baseline for that, 2026-10-27** (pikr 0.9.4, crcbl 0.12.0, saved choice
    `Used`, load average 0.1, three launches each): the GPU's presenter was
    ready at 15–19 ms, fonts loaded at 66–80 ms, and first paint came at 74–85
    ms in dmenu and 86–87 ms in drun. Resident size was 112 MB in dmenu and 122
    MB in drun, of it 91–101 MB anonymous, mostly the fallback faces; a first
    launch that loads every driver peaked at 177 MB. While shown and idle pikr
    made about 8 voluntary context switches a second (the caret blink) and used
    0–1 CPU tick per 5 s.
  - Collection, history and ranking before the window opens: under 3 ms in every
    mode but run (13–19 ms warm, about 100 ms cold, from scanning `$PATH`). Not
    worth moving off the critical path.

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
    fonts. Beyond Latin, checked 2026-10-21 on Arch (Noto fonts) at scale 2: a
    drun icon SVG with three `<text>` lines drew 日本, Hebrew עבר in
    right-to-left order, and Arabic عربي shaped and joined (the break after ر is
    correct Arabic). The Arabic line drew smaller than the other two at the same
    `font-size`, and librsvg (`rsvg-convert`) draws it the same: rendered at 480
    px, the Arabic line's ink was 260x51 px in both and the Japanese
    215x109/110, so it is the fallback face's design, not crcbl. crcbl's own
    gaps: arrow keys move in logical order, and SVG text clusters are
    approximate.
  - While an input method composes, `ui::view::composing` still splits the query
    around the caret to underline the pre-edit, so right-to-left text being
    composed into is reordered per piece, as the whole query was before
    `TextLayout::caret` (2026-10-21). Not seen with a real RTL input method.
  - IME, checked on Windows 2026-10-20 with the Microsoft Japanese IME (keys
    synthesized while pikr held the foreground): the pre-edit draws inline and
    underlined at the caret, the candidate list opens right below the caret, the
    Enter that commits does not reach pikr, and the commit arrives whole (a
    second Enter printed `日本語`). On crcbl 0.6.0 Windows also drew its own
    composition window beside pikr's inline pre-edit, so the reading showed
    twice. **Still doubled on crcbl 0.8.1 and 0.9.0** (driven 2026-10-23): crcbl
    0.7.0 clears `ISC_SHOWUICOMPOSITIONWINDOW` in `WM_IME_SETCONTEXT`, yet
    during the pre-edit pikr's thread shows a visible `MSCTFIME Composition`
    window (60x18, right after the inline reading), and screenshots
    showed にほんご and, after Space, 日本語 twice. The commit still worked
    (Enter printed `日本語`). crcbl confirmed the cause: its window procedure
    still passes `WM_IME_STARTCOMPOSITION` and `WM_IME_COMPOSITION` to
    `DefWindowProc`, which raises the window through the TSF bridge even with
    the flag cleared. Their fix (not yet in a release) keeps both away from
    `DefWindowProc` and takes the commit from `GCS_RESULTSTR` instead of
    `WM_CHAR`; their CI test with the Japanese IME asserts no visible
    composition window. Re-check on the crcbl release that carries it: no
    `MSCTFIME Composition` window on pikr's thread, and since the commit path
    changed, Space then two Enters must still print `日本語` exactly once. Not
    checked: AppKit.
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
  block-wrapped caret. crcbl main `8ab4c02e` (after the 0.12.1 tag, so in the
  release after it) keeps a `white-space: nowrap` span's trailing spaces, as CSS
  `pre` does; `normal` text still lets them hang. Once pikr is on that release,
  the caret span can hold a plain space if it is `nowrap`. Whether
  `ui::view::uncollapsed` can go too depends on the query and row spans being
  `nowrap`: check each before dropping it, and re-run the `--filter "a  b  "`
  and "client for" cases from its doc comment.
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
  - Not done, by design so far: no `--` before the ssh host (the config is
    trusted); no `setsid` for Unix spawns. drun honours `TryExec`, `OnlyShowIn`
    and `NotShowIn` since 2026-10-22 (`drun::unix_impl::ShowIf`, checked after
    the cache so a new desktop or program needs no rebuild). Checked on a
    headless sway with scratch `XDG_DATA_HOME` probes: 2 entries under
    `XDG_CURRENT_DESKTOP=Hyprland`, 3 under `GNOME` (the `OnlyShowIn=GNOME`
    probe), never the probe whose `TryExec` program is missing, the same on
    cache hits. Desktop names compare without case, as
    `freedesktop_desktop_entry::current_desktop` lowercases them (and maps
    `Unity` to `gnome`). A relative `TryExec` with a slash is looked up under
    each `$PATH` directory, as the spec's wording says, not from the working
    directory.
  - ssh lists `known_hosts` hosts (2026-10-22, `modes::ssh::known_hosts`), with
    `[host]:port` entries run as `ssh -p PORT host` (every `build_terminal_args`
    arm takes the ssh argv; pwsh quotes each argument, cmd still refuses
    anything outside `is_safe_host`), skipping hashed entries, `@` marker lines,
    wildcard and negated patterns, and names starting with `-`. `Include` is
    followed (2026-10-22, `modes::ssh::read_config`) with `~/`, relative and
    absolute paths, glob(7) wildcards in any component (`glob_paths`, its
    results checked against glibc's `glob(3)` in `glob_paths_agree_with_glibc`),
    `%%`, `%d` and `${VAR}` (`expand_include_word`). A path with any other token
    is skipped: `%h`, `%p`, `%r` and the rest name the host being connected to,
    which pikr does not have while it lists, and `%u`, `%i`, `%L`, `%l` would
    need `getpwuid`/`gethostname`, which pikr does not link (no `libc`
    dependency). `~user/` is not expanded. OpenSSH 10.6 checked with `ssh -G`:
    it follows `${VAR}` and a class in a directory component, and skips an
    `Include` whose variable is unset while applying the rest of the file.
    Arch's `/etc/ssh/ssh_config` includes systemd's `20-systemd-ssh-proxy.conf`,
    so ssh mode now also lists its `.host` and `machine/.host` (real targets:
    `ssh .host` reaches the local machine through systemd's proxy).
- **Not verified (coverage gaps).**
  - `ui/` and `picker::session` were reviewed for correctness and security on
    2026-10-22 (one read-only agent; every finding re-checked and fixed with a
    test or an on-screen check): `:mode` during `--loading`, a Ctrl chord
    swallowing a later IME commit, the `:` prompt's composition, and `-P` keys
    in trace logs. Checked by it and found sound: frame pacing never stalls (an
    out-of-date present leaves the frame gate open), the char/byte index
    handling in `ui::view`, `fitted_size`, shutdown order and exit codes. A
    reader's Click on a result row does nothing (rows are `Role::None`, which
    crcbl refuses to click); not a regression. The rest of the tree (`picker`
    outside `session`, `config`, `app`, `cli`) was reviewed the same day, the
    same way: frecency never pruned, Windows clipboard accepts written to disk,
    lost updates between two open pikrs, an unparsable state file silently
    wiped, an overflow on a corrupt `last_used`, `--kb-custom Alt++=…` refused,
    and icon files read whole before any limit; all fixed. Found sound there:
    the matcher's char positions, `-P` never recording, `Cli::validate`'s
    combinations, and the icon-theme key-file reader. `modes/` was reviewed
    again the same day: a user `.desktop` copy that hides an app was ignored,
    ssh config comments and quotes were misread, `Include` fan-out was
    exponential, a failed `cliphist decode` emptied the clipboard, and an
    evalexpr panic killed calc mode; all fixed. ssh mode's `-e ssh <host>` holds
    for every Unix terminal it looks for (checked 2026-10-22 from their docs and
    source, none of foot or kitty installed): alacritty's `-e, --command` (its
    `--help` here), xterm's `-e`, foot's man page ("Ignored; for compatibility
    with xterm -e", trailing words being the command), and kitty's
    `simple_cli_definitions.py`, which defines `--execute -e` as a hidden flag
    taking no value, so the words after it are the program.
  - Linux was verified only through CI's sway session on lavapipe: the e2e
    keyboard suite and the startup gate passed (run 39002212977, 237 ms
    launch-to-result against the 500 ms deadline). One flake: in run 39788163184
    (`67bbf56`, 2026-10-22) `accept_matched_candidate_with_return_emits_stdout`
    failed because pikr did not exit within 15 s while Return was resent; no log
    showed why (a window never mapped or focused, or a stalled first present).
    It passed on rerun and 40 times out of 40 on a local headless sway. A repeat
    is worth reading the job log for. Hyprland 0.56.2 (Arch, 2026-10-20, Intel
    RPL-U iGPU on Vulkan, scale 1) was run and looked at in screenshots: the
    overlay layer maps centred, the corners are see-through, and dmenu typing,
    highlighting and Return work, with Hebrew, CJK and colour emoji rows drawn.
    First paint (`RUST_LOG=pikr=debug`) came at 366 ms on the live desktop
    (fonts 229 ms), 943 ms on the first launch in a nested Hyprland (fonts 554
    ms, the binary just rebuilt) and 86 ms on the next one. Pointer input was
    driven with `wlrctl` (virtual-pointer) in the nested Hyprland: hover lights
    the row under the pointer with a hand cursor, the wheel scrolls the list
    with the hover following, and a left click accepted the row under the
    pointer after the scroll.
  - niri 26.04 (2026-10-21, nested inside a headless sway, since a niri window
    hidden on the live Hyprland gets no frames): pikr maps on the overlay layer
    with exclusive keyboard, the corners are see-through, bidi, CJK and emoji
    rows draw, and typing, highlighting and Return work (first paint 232 ms).
  - KWin 6 (2026-10-21, `kwin_wayland --virtual` under `dbus-run-session`, so it
    stays off the user's session bus; nested in pixman sway it segfaulted): pikr
    maps through layer-shell on the overlay layer with exclusive keyboard, KWin
    configures it at its size, keyboard focus arrives, and first paint came at
    212 ms (`WAYLAND_DEBUG=1`). KWin offers neither wlr-screencopy nor
    virtual-keyboard, so it was not looked at and no key was typed there.
    Driving it would need KWin's fake-input or EIS (libei), or a KWin nested in
    a GPU-rendering compositor screenshotted from outside.
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
  - Clipboard mode on Linux was checked on headless sway (2026-10-21, cliphist
    pointed at a scratch `XDG_CACHE_HOME`, which keeps the user's history out):
    a seeded history lists newest first, and accepting a row put its text on the
    clipboard (`wl-paste` read it back). Since 2026-10-22 the accept runs
    `clipboard::unix_impl::copy_script`, which decodes to a temporary file and
    runs `wl-copy` only on success; with the real tools, a missing id exited 1
    and left the clipboard as it was, where the old pipe set it empty. Without
    cliphist pikr warns and shows "No entries.". A history nothing was ever
    stored in makes `cliphist list` exit 1 ("please store something first"), so
    `clipboard::unix_impl::collect` logs a non-zero exit at debug level only. An
    image entry (2026-10-21) is listed under cliphist's own preview label
    (`[[ binary data 123 B png 64x32 ]]`), and accepting it put an `image/png`
    on the clipboard byte-identical to the stored PNG.
  - **Linux startup over 500 ms seen only on a loaded machine.** Five drun
    launches in a row in a nested Hyprland (2026-10-20) reached first paint at
    1075, 176, 153, 163 and 240 ms; the first spent 657 ms in
    `ui::run::load_fonts`, which reads every fallback face whole
    (`Font::from_system`: 16 faces, about 70 MB, the two Noto CJK collections 46
    MB of it; fontconfig already trims the list to faces that add coverage).
    Ruled out on headless sway (2026-10-21) with `posix_fadvise(DONTNEED)`, no
    root needed: evicting pikr's fonts, fontconfig caches and binary gave 712,
    330 and 242 ms; also evicting every shared library pikr maps (Mesa's Intel
    and lavapipe Vulkan drivers, LLVM, the loader) and Mesa's shader cache gave
    538, 336 and 323 ms, the same. Idle alone: launches after 90 s idle reached
    460 and 433 ms against 355 and 245 ms back to back. A third pair, taken at
    load average 16.6 on 12 cores (other sessions compiling), took 4397 and 1358
    ms, with GPU setup, fonts and paint all slow together. So the misses track
    CPU load, not idle time or the page cache; the earlier 1075 ms run was taken
    with other builds going. Idle adds about 100 ms. Reading the faces in
    parallel (one thread each) took 85 to 184 ms evicted against 170 to 205 ms
    in sequence in a probe: not adopted on that evidence. Reading faces only
    when a glyph needs them would need a crcbl API for lazily loaded fallbacks.
  - **Run GUI checks in a nested Hyprland, never on the user's live desktop**:
    pikr takes the keyboard on the overlay layer, so a test run there swallows
    whatever the user is typing. Start `Hyprland -c <lua config>` through
    `hyprctl eval "hl.exec_cmd(cmd, { workspace = 'special:x silent', no_initial_focus = true })"`
    (0.56 configures and dispatches in Lua). The live compositor never draws
    that hidden window, so its output gets no frame callbacks: grim hangs and
    the app never gets a frame. Inside it, add `hyprctl output create headless`
    and disable `WAYLAND-1`, then point `WAYLAND_DISPLAY` and
    `HYPRLAND_INSTANCE_SIGNATURE` at it for pikr, wtype, wlrctl (AUR; pointer
    moves are relative, so `move -5000 -5000` first to reach 0,0) and grim.
    **Also set `XDG_STATE_HOME` (and `XDG_CACHE_HOME`) to a scratch directory
    for every pikr a test launches**: each accept writes history and frecency
    (`picker::state_file_path`), and tests on 2026-10-20 wrote theirs into the
    user's real state until they were taken out by hand. The un-drawn output is
    also a way to reproduce the frame-pacing freeze fixed on 2026-10-20 (the
    frame gate, now crcbl-app's `Presenter`); turning a headless output off with
    DPMS does not reproduce it. For checks that need no Hyprland, sway is also
    installed on the Arch machine: the e2e suite starts its own headless sway in
    a private `XDG_RUNTIME_DIR`, and
    `cargo test --release --test keyboard -- --test-threads=1` (6 passed) and
    `--test startup_gate` (345 ms against the 500 ms deadline) ran there on
    2026-10-20. Keep build output off `/tmp` on that machine: it is a shared
    tmpfs, and a second release build filled it.
  - Key handling is tested headless on every OS (`ui::run::headless`: crcbl's
    `HeadlessShell` and crcbl-hal's null backend driving the real window loop),
    which puts no frame on a screen and no key through a real keymap. The sway
    suite in `tests/e2e` keeps only what needs the binary: stdin and
    `--loading`, the config file, state files, and keys arriving through
    Wayland. Named keys go through `HeadlessShell::key_press`, which has given
    them the real backends' keysyms since crcbl 0.6.0. The loop handles every
    scripted event in its first pump, so these runs end before a frame is drawn:
    layout, presenting and the swapchain's out-of-date retry (`ui::run`, which
    now waits `PRESENT_RETRY` before redrawing rather than spinning) are not
    exercised by them. Scripting input across frames would need a hook in the
    loop. What one frame builds can be checked through
    `headless::builds_a_list`, which calls `Window::frame` directly.
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
    then arrived about three seconds later. drun's icons were looked at on
    2026-10-22 (`--show drun`, 138 apps): each row drew its app's own icon from
    `%LOCALAPPDATA%\pikr\icon-cache`, and Windows' generic icon where an app has
    none. A first drun launch with no `drun-apps.toml` walks `AppsFolder` in the
    foreground (`drun_windows::collect`): 943 ms to collect, first paint at 1313
    ms, with the icons already cached; the next five launches collected in 1 ms
    and painted at 315 to 360 ms. Clicks and the wheel were not exercised, and a
    launch from a hotkey tool was never tried.
  - Real theme icons and HiDPI were checked on Hyprland at scale 2 (nested,
    2026-10-20): drun rows drew hicolor PNGs (Discord at 256 px, Firefox at 48
    px) sharp, with sharp text, and the layer mapped centred at its logical
    size. At 1.5 crcbl uses `wp_fractional_scale_v1` (`preferred_scale(180)`)
    with a viewport and draws at 1.5 itself; since `ui::run::snapped` the buffer
    is a whole 1080x672 and text is sharp (before that it was 1080x671 for 670.5
    device pixels, and soft). A scale change from 1.5 to 1 while pikr was open
    was picked up in place. Other scales (nested Hyprland, 1920x1080 output,
    2026-10-21): 1.25 gave a whole 900x560 buffer. Hyprland snaps a 1.75 request
    to 5/3 there, and also takes 4/3 and 1.6, where 720x448 is not whole
    (buffers 960x597, 1152x717, 1200x747). At 1.6 a throwaway build with a size
    step of 5 (a whole 1152x720) measured the same edge sharpness as the 717-row
    buffer, so Hyprland does not visibly resample a size it rounds the same way.
    The softness seen at 1.5 came from an exact half pixel (670.5), where
    rounding can go either way. A multiple of 4 also avoids those halves for the
    default sizes at the other scales checked.
  - Two outputs at different scales (nested Hyprland, 2026-10-21, standing in
    for the laptop panel beside an external monitor): pikr opens centred on the
    focused output and draws at that output's scale (`preferred_scale(240)` and
    1440x896 buffers on the scale-2 one, 720x448 on the scale-1 one), and
    accepts on both. Removing the output pikr was open on (an unplugged monitor)
    made it exit at once with code 1 and nothing on stdout, like a cancel. Not
    checked: a real eDP panel with the lid opened or closed while pikr is open
    (the test outputs were headless).
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
  - **Screen readers: adopted 2026-10-21** (crcbl's `accesskit` feature on
    crcbl-shell and crcbl-ui; `ui::run` sends `Ui::accessibility_update` after
    each frame while `reading`, and passes `AccessibilityAction` to
    `Ui::accessibility_action`). Checked on Hyprland with Python's `Atspi`: pikr
    appears on the AT-SPI desktop only while `org.a11y.Status IsEnabled` is true
    (a screen reader sets it; accesskit_unix waits for it, so nothing is
    registered otherwise), and its tree held the prompt, the query, one list
    item per row with its label and icon, the scrollbar and the status bar,
    updated as the query was typed. First paint with the feature was 122 to 149
    ms against 120 to 177 ms without (interleaved, load ~7). cargo-deny's three
    zbus "unmaintained" advisories are ignored in `deny.toml`, as crcbl does.
    Since crcbl 0.9.0 (2026-10-23) `ui::view` names a highlighted label whole
    (`highlighted` calls `Ui::set_accessible_label`), builds the results with
    `Ui::selectable_list` so the row Return accepts is selected, and marks the
    query and the `:` line as text inputs with their caret (`accessible_field`,
    `Ui::set_accessible_text_input`; under `-P` the value is the bullets drawn).
    The headless test
    `screen_readers_hear_whole_labels_the_selected_row_and_the_query` reads all
    three from `Ui::accessibility_update`. On AT-SPI, the label and the selected
    state were checked in a patched clone on 2026-10-22 ("gamma" read as one
    label, `selected`); the text input was not read through AT-SPI. On Windows,
    .NET's `System.Windows.Automation` client read `028b1a0` on 2026-10-23
    (dmenu, keys synthesized while pikr held the foreground): the query is an
    Edit whose ValuePattern and TextPattern caret follow typing and Backspace
    (`a` caret 1, `alp` caret 3, Backspace `al` caret 2), under `-P` its value
    is `●●●`; each row's label is one Text (`alpha`); the selected ListItem
    follows Down (`alpha` to `beta`) and re-ranking. Each ListItem's own Name
    was still empty there, the label being its child; since `2550ee8`
    `result_row` names the list row itself (`Ui::current_key`, its label and
    `(description)`), and the same client read the ListItems as `alpha`, `beta`,
    `gamma` (2026-10-23). The `:` line opens as a second Edit beside the query,
    its value including the colon (`:`, then `:ab` with caret 3). Not checked: a
    row with a description through UI Automation (dmenu rows have none), and
    Orca or Narrator speaking any of it. Before the change (2026-10-22,
    `1712f0b`) no row was selected and the query was a Text with no
    ValuePattern.
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

## Dependencies

- **evalexpr is held at 11 (declined: 12 and later).** evalexpr 12.0.0
  relicensed it under AGPL-3.0-only, which pikr's MIT binaries cannot ship;
  cargo-deny's licence check rejects it. `cargo upgrade --incompatible` offers
  13 and has to be undone each time (the reason is beside the requirement in the
  workspace `Cargo.toml`). Moving off evalexpr to another permissively licensed
  expression evaluator, or writing calc mode's own, is the alternative if 11
  ever needs a fix it will not get.

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
- **The Alpine `.apk` publishes from v0.9.1.** v0.9.0's `alpine` job, its first
  run, failed in `abuild -r` because `apk add --no-cache` left no package index;
  `ci.yml` now runs `apk update` first, and v0.9.1 (cut for this) shipped
  `pikr-0.9.1-r0.apk`. The `build-musl` job builds with
  `-C target-feature=-crt-static` (a static musl binary cannot `dlopen`
  libvulkan) and `GETTEXT_SYSTEM=1`. A podman `alpine:latest` run of the job's
  steps installed the package with its dependencies and ran `pikr --version`.
  Not checked: the package drawing a window on a real Alpine system against
  Mesa.
- **macOS runners sometimes never start.** Twice on 2026-10-21 a macOS job (an
  aarch64 build, then `smoke macos-latest` on the v0.9.1 tag) sat assigned for
  17 to 26 minutes and was cancelled with no step run, which skips every publish
  job behind it. `gh run rerun <id> --failed` reran it and the dependents green
  both times; nothing in pikr was at fault.

## Hardening (correct today, fragile — not defects)

Re-verified against the tree on 2026-10-18.

- Coarse-granularity same-tick cache staleness; non-UTF8 PATH names (skipped by
  `run::scan_dir`). (review pass 2026-08-06; not re-checked individually)
- `--width 0` and oversized `--lines` / `--width` are handled by
  `ui::run::fitted_size` since 2026-10-22. `--lines 0` is an input-only prompt
  (`AppState::lists_rows`, the owner's call on 2026-10-22).
- No accept path runs a shell on a string pikr builds from untrusted text:
  launches are `Command::new(program).args(args)`. The two script strings are
  clipboard's `copy_script` (`clipboard::unix_impl`, `id` parsed as `u64`, the
  only value in the script) and ssh's pwsh and cmd arms
  (`ssh::build_terminal_args`: pwsh single-quotes each ssh argument, cmd takes
  only arguments in `is_safe_host`'s charset). Re-checked 2026-10-19; keep it
  that way.
- State writes (`picker::write_private_state`: history, frecency, the drun
  caches, the Windows icon cache) sync the temporary file, rename it into place
  and sync the directory (`picker::sync_dir`); a directory that cannot be synced
  only warns. On Linux the order was observed with gdb syscall catchpoints
  (2026-10-22): `fsync`, `rename`, `fsync`. The Windows directory flush
  (`FlushFileBuffers` on a `FILE_FLAG_BACKUP_SEMANTICS` handle) is checked only
  by `a_state_directory_can_be_synced` in CI. History and frecency saves run
  under `picker::with_state_lock` (an exclusive `File::lock` on a `<name>.lock`
  sibling, never deleted); where no lock can be had they save unlocked, with a
  warning. On Windows the rename fails while another process holds the file open
  without delete sharing; that save is dropped with a warning, never
  half-written.
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

- **Windows startup is under the 500 ms target.** Release build at `cba69cf`
  (crcbl-app's `Presenter`), D3D12, RX 7900 XTX + iGPU + WARP, 2026-10-22: ten
  warm background `--dmenu` launches reached first paint at 284, 312, 319, 337,
  317, 369, 359, 474, 414 and 478 ms, median ~348 ms
  (`RUST_LOG=pikr=debug,crcbl_app=debug`; other sessions were compiling, CPU
  load 74% after the run). On crcbl 0.6.0 before the `Presenter` (2026-10-20)
  the median was ~376 ms with outliers at 963 and 2080 ms, and on crcbl 0.5.0
  three launches gave 617, 443 and 493 ms. GPU setup is nearly all of it;
  config, entries, ranking and fonts are done within ~50 ms.
- **crcbl-app's `Presenter` draws pikr's frames (2026-10-22).** `ui::gpu::Gpu`
  keeps only the UI renderer, the render graph and the transient pool; the
  backend, adapter walk, transparency fallback (`Transparency::Preferred`),
  swapchain, frame request gate (`observe` / `frame_due`) and teardown are
  crcbl-app's. pikr names the backend (`ui::gpu::platform_backend`), so
  `CRCBL_GPU` is not read and Windows never opens Vulkan before D3D12; it asks
  for one frame in flight, `Pacing::Vsync` and no present feedback or timing,
  which keeps the old behaviour of a frame finished before the next is recorded.
  One rule changed: the walk now prefers a lower-ranked adapter that can
  composite the transparent window over a higher-ranked one that can only draw
  it opaque. Checked on headless sway (Arch, Vulkan, 2026-10-22): the swapchain
  came up `PreMultiplied` and a screenshot showed the rounded corners
  see-through over a magenta window; typing and Return accepted `gamma` six
  times out of six, the same as the v0.9.2 binary; the e2e keyboard suite and
  the startup gate passed (351 ms against 500). Checked on Windows 11 (D3D12,
  2026-10-22, release build at `cba69cf`): crcbl-app opened dx12, picked the RX
  7900 XTX (discrete, the display's adapter, first of 3 that can present) as
  before, and logged a 720x448 `Bgra8UnormSrgb Fifo PreMultiplied` swapchain; a
  screenshot showed the picker drawn whole, and the pixels outside the rounded
  border matched the desktop just outside the window (31,31,31, not the panel's
  41,46,66 or an opaque fallback's black). A `SetWindowPos` to 900x600 from a
  probe logged "reconfiguring the swapchain to 900x600" and redrew the picker
  laid out at the new size, the corners still see-through. Not checked: a scale
  (DPI) change on D3D12, and Metal on macOS (CI builds and runs the unit tests
  there). What is left unadopted is `text_input::TextPump`, which feeds
  crcbl-ui's `TextInput` while pikr edits its own query.

### Coverage gaps (not verified)

- Window posture (issue #9): re-read at `cba69cf` (crcbl 0.7.0, 2026-10-22) with
  `GetWindowLongPtr`: pikr has one visible window, unowned, style `0x96000000`
  (`WS_POPUP | WS_VISIBLE | WS_CLIPSIBLINGS | WS_CLIPCHILDREN`) and extended
  style `0x00200088`
  (`WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOOLWINDOW | WS_EX_TOPMOST`), without
  `WS_EX_APPWINDOW`, as at crcbl 0.3.0, when no taskbar button showed in a
  screenshot. The Alt-Tab switcher, opened over pikr with synthesized Alt+Tab
  (2026-10-23, crcbl 0.8.1), listed 8 windows and none was pikr's.
- Closing on focus loss (decided 2026-10-24): once pikr has had the keyboard
  focus, `ui::run`'s `ShellEvent::Focus` arm exits 1 when it goes (`had_focus`);
  a loss before the first focus does not close it. Headless tests cover both
  halves and each went red on its mutant. On Windows (2026-10-24, release
  build), opening Alt-Tab over pikr closed it with exit 1 and the log line "pikr
  lost the keyboard focus; closing". Not run on unix, where only the toplevel
  cases change (read from crcbl-shell 0.8.1 and `create_window`): with
  layer-shell (Hyprland, sway) pikr is an Overlay surface with exclusive
  keyboard (`PANEL_EXCLUSIVE_KEYBOARD`), so a leave should come only from
  something above it such as a lock screen; GNOME's Mutter and X11 open it as an
  ordinary toplevel, which a click elsewhere now closes. If the sway e2e suite
  or the startup gate turns flaky, check whether a focus leave during setup now
  closes pikr. **It does (2026-10-26):** the local sway e2e keyboard suite
  failed 3 times in 31 runs, each a test expecting exit 0 getting exit 1. With
  `RUST_LOG=pikr=debug` the failing run logged "startup first focus received"
  and, 17 ms later, "pikr lost the keyboard focus; closing", before any key was
  sent. Not yet read: why headless sway sends that leave (a second wtype virtual
  keyboard, or the layer surface's remap) and whether a real sway session does
  the same; a fix would ignore a leave the focus comes straight back from, or
  one during startup. Whether Hyprland's own binds still fire over an exclusive
  layer surface was not checked. Also not checked, and the likeliest false
  close: as a toplevel (X11, Wayland without layer-shell) a transient leave,
  such as an IME's candidate window or the compositor briefly taking the focus,
  would now close pikr mid-typing. On Windows the Microsoft IME's candidate list
  does not: with `7f1f66b` (2026-10-24) pikr stayed open through two Space
  conversions, the second opening the candidate list, and committed. If it shows
  up, ignore a leave that a focus regain follows within a short grace period
  instead of closing at once.
- Launched from a script, pikr often does not get the foreground: in 33 launches
  on 2026-10-23 (`Process.Start` from PowerShell), the first launch from a fresh
  PowerShell process usually never became the foreground window within 8 s (11
  of 12), and every later launch from the same process did (21 of 21). That fits
  Windows' foreground lock refusing a process that has not had recent input; a
  launch from a hotkey tool, which owns the last input, was not tried.
- `--kb-custom KEY=PROMPT` with no matching row now shows the confirm card in
  the empty-state row; its on-screen placement was not looked at. The e2e test
  `kb_custom_confirm_with_no_match_prints_query` covers the behaviour (CI,
  Linux).
- run and ssh modes on Windows were launched and listed (2026-10-22, release
  build): run collected 1044 `PATH` executables in 8 ms and drew them (first
  paint 384 ms; the counter reads 256, `max_results`); ssh listed this host's
  three configured hosts in 5 ms (not screenshotted, the names are private).
  Accepts driven with synthesized keys on 2026-10-23: drun `notepad` + Return
  started Notepad from its WindowsApps package, run `notepad` + Return started
  it through `C:\WINDOWS\system32\notepad.exe`, each recorded in `usage.toml`; a
  dmenu accept wrote `history.toml` newest first and Ctrl-P recalled it. The ssh
  accept ran with `TERMINAL` pointed at a pwsh probe script (so no connection
  was made): pikr launched it with `-NoExit -Command "ssh '<host>'"` and the
  argument parsed back to `ssh` and a host from `known_hosts`, unquoted. Not
  exercised: this host's default terminal, `wt.exe` (`-- ssh <host>`), and a
  real connection.
- Clipboard mode reads Windows' clipboard history through WinRT
  (`clipboard::windows_impl::history_texts`, on a thread of its own in a
  single-threaded apartment, since `Clipboard` refuses activation from an MTA:
  the first run on 2026-10-22 failed with `0x8000001D` until it moved there).
  Checked on Windows 11 with history off (the default): it logs that and lists
  the current clipboard text, collected in 35 ms. Not checked: a history that is
  on (turning it on starts Windows recording the user's clipboard, so it waits
  for the owner), text items' order and accepting one, and how long a long
  history takes to read at startup.
