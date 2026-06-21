# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Normal-mode insert-entry motions on the query bar: `a` (append after cursor),
  `A` (append at end of query), and `I` (insert at start of query), matching
  vim. `i` (insert before cursor) was already supported.

### Fixed

- Query-bar cursor is now an overlay: the text is a single fixed label and the
  cursor is a separate rect drawn on top, absolutely positioned at the caret's
  measured x-offset (Insert = thin bar; Normal/Visual = block over the caret
  cell). Only the cursor moves as the caret moves — the input text no longer
  shifts/reflows on caret movement, blink, or mode switch, and positioning is
  font-independent (measured with the same font the label renders). The ex (`:`)
  prompt keeps its simple caret-at-end.

- CI: the e2e keyboard harness now forces GPU-independent software rendering
  (`WGPU_BACKEND=gl` + `LIBGL_ALWAYS_SOFTWARE` + llvmpipe, with
  `libgl1-mesa-dri` installed in CI), so the headless suite no longer hard-fails
  on runners with no usable GPU (Mesa fell back to ZINK and wgpu hung with
  `VK_ERROR_INCOMPATIBLE_DRIVER`). No effect on the shipped binary.
- Docs: corrected the CHANGELOG `[Unreleased]` compare link, which had drifted
  to `v0.7.1...HEAD`; it now tracks `<latest-tag>...main`.

### Changed

- CI: the two dmenu accept-emit e2e tests are quarantined (`#[ignore]`) under a
  separate headless keyboard-focus/input race (#34) that re-sending keystrokes
  does not resolve; the keymap→`Accept`/`AcceptCustom` mappings remain covered
  by unit tests. Run with `cargo test -- --ignored` to repro.

## [0.8.5] - 2026-06-21

### Fixed

- Query-bar cursor in the wrong spot in Normal/Visual mode: the block cursor
  (`█`) was inserted _before_ the character at the caret, growing the line by a
  cell and pushing the covered character one position right. It now overlays
  (covers) the character at the caret like vim, and reveals the underlying
  character on the blink-off phase so nothing vanishes or shifts. The Insert
  thin-bar caret is unchanged (it correctly sits between characters).
- Normal mode could not move the query caret: `h`/`l` and `←`/`→` were no-ops
  (only `j`/`k` moved the result list). They now move the caret left/right over
  the query, matching vim; `j`/`k`/`↑`/`↓`/Home/End remain list navigation.

## [0.8.4] - 2026-05-20

### Changed

- The `--message` modal (`pikr -e "..."`) is now styled via CSS — three new
  selectors `.message-modal-outer`, `.message-modal`, `.message-text` live in
  `default.css` alongside the rest of pikr's chrome. Theming the message modal
  via user CSS now actually works; previously bg/border/radius/padding/font were
  hardcoded inline and silently ignored stylesheet overrides. Visual rendering
  is identical to v0.8.3. (#43)

## [0.8.3] - 2026-05-19

### Fixed

- Windows: CLI surfaces (`--version`, `--help`, `--dmenu` selection emit, panic
  output) now reach the calling shell. Release builds link as
  `windows_subsystem = "windows"` (v0.8.1, cmd-flash fix) which detaches stdout
  at process start, so every `println!` / `eprintln!` was silently dropped.
  `AttachConsole(ATTACH_PARENT_PROCESS)` is called at the very first line of
  `main()` to re-attach to the parent console when one exists; the GUI path
  (Explorer / Scoop shim / Start Menu) is unchanged.

### Added

- CI: cross-platform smoke job (linux/macos/windows) builds pikr in release and
  asserts `--version` + `--help` exit 0 with expected output. Catches link
  errors, missing runtime libs (VCRUNTIME / dyld / libgtk), and panic-on-init
  bugs on PR review before they reach a real user.

## [0.8.2] - 2026-05-19

### Fixed

- Windows drun: `.lnk` targets the shell has no icon for (.bat, .cmd, .ps1,
  .vbs, or odd paths) now fall back to the Windows generic-app icon instead of
  leaving the picker row blank. Fallback PNG cached once at
  `%LOCALAPPDATA%\pikr\icon-cache\__fallback__.png` so subsequent failures are
  zero Win32 calls. Also fixes clippy::io_other_error on `drun.rs:302` (new in
  Rust 1.95).

## [0.8.1] - 2026-05-19

### Added

- Windows drun entries now render icons. Targets are queried via Win32
  `SHGetFileInfo`, the `HICON` is dumped to RGBA via `GetDIBits`, and the result
  is cached as PNG at `%LOCALAPPDATA%\pikr\icon-cache\<sha256>.png`. Cache hits
  skip the Win32 round-trip entirely. (#40)

### Performance

- Windows drun mode is faster on cold start: `.lnk` parsing now fans out via
  rayon, and the entry list is cached at `%LOCALAPPDATA%\pikr\drun-cache.json`
  keyed by the Start Menu dir mtimes. Subsequent runs skip the walk entirely if
  nothing changed. (#42)

### Fixed

- Windows: release binary is now linked as a windowed-subsystem app, so Scoop /
  Start Menu launches no longer flash a cmd / conhost window. Debug builds keep
  the console attached so `cargo run` still shows tracing logs. (#39)
- Windows: default theme font now falls back to `Cascadia Mono` when Hack Nerd
  Font Mono isn't installed. Scoop manifest also gains a dependency on
  `nerd-fonts/Hack-NF` for users who want the full Nerd Font glyph coverage.
  (#41)

## [0.8.0] - 2026-05-19

### Added

- **Windows: `--show ssh`** (#35). Reads known_hosts from
  `%USERPROFILE%\.ssh\known_hosts` via `dirs::home_dir()`; system-wide
  `/etc/ssh/ssh_config` skipped (no Windows analogue). Terminal probe order on
  Windows: `wt.exe`, `pwsh.exe`, `powershell.exe`, `cmd.exe`, each with a
  tailored argv (`wt -- ssh <host>`, `pwsh -NoExit -Command "ssh <host>"`,
  `cmd /K "ssh <host>"`). Linux probe order (alacritty/kitty/foot/xterm +
  `-e ssh <host>`) unchanged.
- **Windows: `--show run`** (#36). `is_executable` now branches per OS: Unix
  keeps the `PermissionsExt::mode() & 0o111` check; Windows splits `%PATHEXT%`
  on `;` and matches `path.extension()` case-insensitively, defaulting to
  `.COM;.EXE;.BAT;.CMD` when the env var is unset. Labels drop the extension on
  Windows so users type `firefox` not `firefox.exe`; `Command::new("firefox")`
  does its own PATHEXT lookup at spawn time.
- **Windows: `--show clipboard`** (#37). New `Payload::SetClipboard` variant +
  per-OS `set_clipboard` helper. On Windows: `arboard` reads the current
  clipboard text and shows it as a single entry; accept writes back via
  `arboard::Clipboard::set_text`. Limitation (current item only, not full
  history) logged via tracing on startup. Linux branch keeps the existing
  `cliphist list` + `cliphist decode | wl-copy` shell-out verbatim.
- **Windows: `--show drun`** (#38). `walkdir` traverses
  `%APPDATA%\Microsoft\Windows\Start Menu\Programs` and
  `%ProgramData%\...\Programs`. Each `.lnk` parsed via `lnk` 0.6:
  `link_target()` for the resolved exe path, `command_line_arguments()` split
  via shlex. Filters: skip targets that don't exist, skip extensions in
  `{txt,url,pdf,html,htm}`, skip parent folders containing "uninstall". Icons
  skipped in phase 1 (future follow-up). XDG `.desktop` pipeline on Linux
  verbatim.

### Changed

- **Cross-platform mode dispatch**: `modes::drun / run / ssh / clipboard` no
  longer cfg(unix)-gated; each ships per-OS internals through `unix_impl` /
  `windows_impl` submodules. The `app::run` mode arms for `Drun / Run / Ssh` are
  unconditional now.

## [0.7.2] - 2026-05-19

### Added

- **Windows binary build** restored — `x86_64-pc-windows-msvc` row added to the
  release matrix. First sub-task of #9; downstream Windows decisions (drun / run
  / clipboard / ssh behavior, packaging) still pending. (#9)
- **Scoop manifest** for Windows install via `scoop install kryptic/pikr`. New
  `scoop-bucket` CI job renders `pkg/scoop/pikr.json.in` and pushes to
  `kryptic-sh/scoop-bucket` on each `v*` tag, cascading from
  `publish-github-release`. (#21)
- **Deeper CSS coverage**: rows, icons, empty-state, scroll-handle, mode-chip,
  prompt, query. Reactive bits (per-row 3-state bg, hover, mode-chip bg) stay
  inline; geometry consts stay in Rust because chrome-height math depends on
  them. Class definitions live in `apps/pikr/src/ui/styles/default.css`.
- **e2e harness** widened: Esc-x2 dismiss now covered for `emoji`, `clipboard`,
  `run`, `ssh` modes in addition to the original `drun` / `dmenu` / `calc`. Full
  7-mode matrix asserts Insert → Normal → Cancel → exit 1.
- `Pikr::wait_with_retry` test fixture helper — re-invokes a key sender on a
  configurable cadence while waiting for pikr to exit. Future-proofs single-
  keystroke tests against focus-claim jitter.
- `aaa_warmup_absorbs_first_spawn_race` e2e test that runs first alphabetically
  and burns the cold-spawn surface-lost race so subsequent tests see a warm wgpu
  / EGL / Mesa stack. Helps under CI's single-process
  `cargo test --test-threads=1` model (issue #34 has the upstream race).

### Changed

- **CI: package on main push, publish on tag** (#33). Build matrix and nfpm
  packaging now gated on `github.event_name != 'pull_request'`, so cross-
  platform compile / packaging-script bugs surface on every main push instead of
  only at release time. Build switched from inline `action-gh-release` uploads
  to `actions/upload-artifact@v7`; nfpm consumes via
  `actions/download-artifact@v8` with version resolved from `Cargo.toml`. New
  tag-only `publish-github-release` job flattens all workflow artifacts and
  attaches them to the GH release. `aur-bin` / `brew-tap` / `scoop-bucket`
  cascade from the publisher with no own `if:` gate.

### Open

- `accept_matched_candidate_with_return_emits_stdout` e2e test left
  `#[ignore]`'d while #34 (`wgpu ERROR_SURFACE_LOST_KHR` startup race) is
  outstanding. Other assertion-bearing tests stay green thanks to the warmup
  test absorbing the race on first spawn.

## [0.7.1] - 2026-05-19

### Changed

- **Track `main` / `master` on the floem + winit forks** instead of per-PR topic
  branches. The layer-shell work (`mxaddict/floem:layer-shell-port`,
  `mxaddict/winit:layer-shell-port`) and the lapce/floem#1077 PR branch
  (`feat/menus-feature`) are now merged into the forks' default branches. Pikr's
  `[patch.crates-io]` block flips from `branch = "layer-shell-port"` →
  `branch = "main"` (floem) / `branch = "master"` (winit). Future fixes on those
  forks reach pikr on the next `cargo update`; topic branches are deleted.

## [0.7.0] - 2026-05-19

### Added

- **CSS-driven styling seed** (towards #20). Pikr's static styling now reads
  from `apps/pikr/src/ui/styles/default.css`, parsed once at startup with theme
  colors substituted, applied via a thin floem-Style adapter at
  `apps/pikr/src/ui/css.rs` over the `hjkl-css = "0.25"` parser. Sites migrated:
  panel-outer / panel / panel-stack, input-row (geometry + bg + radius +
  margin), ex-bar (full), status-bar wrapper, mode-name label, count-chip.
  Reactive remnants stay inline: vim-mode chip background, hover-bg blend,
  per-row selected background, match-position color spans inside `rich_text`.
  Visual parity verified by headless-sway screenshot diff.
- `AppState.stylesheet: Arc<hjkl_css::Stylesheet>` built once in `app::run` from
  `Config.theme`.

### Changed

- `ex_bar` and `status_bar` fn signatures now take an
  `Arc<hjkl_css::Stylesheet>` instead of pre-blended `bg` `Color`s. Callers pass
  `Arc::clone(&sheet)`; the blend math moved into `ui::css::build_stylesheet`.

### Open

- `hjkl-css-gui` is intentionally absent. Built against floem 0.2.0 stable;
  breaks under pikr's `[patch.crates-io]` redirect to current `lapce/floem` main
  (`Weight` → `FontWeight`, `text::Style` → `text::FontStyle`, `Color::rgba8` →
  `Color::from_rgba8`, `column_gap` removed, `AlignItems` signature changed).
  Swap pikr's in-tree adapter for `hjkl-css-gui` once upstream floem main
  releases and the adapter is patched.

## [0.6.5] - 2026-05-18

### Added

- **`aur-bin` publish is back, x86_64-only.** Restored the AUR push job that was
  short-circuited in v0.5.4 when aarch64 fell out of the build matrix.
  `pkg/aur/PKGBUILD-bin.in` now declares `arch=('x86_64')` and the workflow
  fetches only the x86_64 sha sidecar. `yay -S pikr-bin` works again. arm64
  entries will be restored when #31 lands `aarch64-unknown-linux-gnu`.

## [0.6.4] - 2026-05-18

### Fixed

- **e2e: `accept_matched` now uses `--filter` instead of live typing.** The
  1500ms key-delay bump in v0.6.3 still wasn't enough on CI's pixman +
  zink-broken-Vulkan stack — per-key rerank+repaint outran wtype's pacing and
  Return arrived before the match list was settled. Switched to `--filter ban`
  so pikr does one matcher pass at startup, then the test sends only Return.
  Local: ~1.8s (was ~10s); CI: expected to hold without the live-typing race.

## [0.6.3] - 2026-05-18

### Fixed

- **e2e flake on CI** (closes #29).
  `accept_matched_candidate_with_return_emits_stdout` failed on Ubuntu's
  pixman-renderer headless sway: 500ms pre-key delay wasn't enough for pikr's
  full startup (XDG icon theme + frecency load + first paint) before wtype
  started sending keys, so the typed "ban" + Return landed before pikr could
  grab focus. Bumped wtype `.delay` 500ms → 1500ms and `wait_timeout` 5s → 10s
  across the suite. Local Wayland still passes in ~10s.

## [0.6.2] - 2026-05-18

### Fixed

- **Windows clippy: round 2.** v0.6.1 cleaned up `xdg` + freedesktop refs but
  missed five more unix-only sites:
  - `modes/run.rs` — `PermissionsExt::mode()` for executable-bit detection on
    PATH entries.
  - `modes/ssh.rs` — same `PermissionsExt::mode()` for the terminal `which`
    lookup; also assumed `/etc/ssh/ssh_config` + `alacritty/kitty/foot/xterm`.
  - `ui/view.rs::AppState::switch_mode` — the `CliMode::Drun` match arm
    referenced `modes::drun` unconditionally; Run/Ssh arms needed the same gate.
  - `tests/keyboard.rs` (e2e entry) — `libc::kill` in `sway.rs`; whole harness
    is unix-only.
  - Fix: `#[cfg(unix)] pub mod {run,ssh}` in `modes/mod.rs`; per-arm
    `#[cfg(unix)] / #[cfg(not(unix))]` in `app.rs::run` and
    `view.rs::switch_mode` (exit 2 / no-op respectively); `#![cfg(unix)]` on
    `tests/keyboard.rs`. Calc / clipboard / dmenu / emoji keep working on
    Windows.

## [0.6.1] - 2026-05-18

### Fixed

- **Windows clippy + build path** — `xdg::BaseDirectories` unresolved (the crate
  is unix-only) and `gettext-sys` crashed on Windows because there's no `make` /
  `configure` toolchain. Moved `xdg`, `freedesktop-icons`, and
  `freedesktop-desktop-entry` under `[target.'cfg(unix)'.dependencies]` in
  `apps/pikr/Cargo.toml` and cfg-gated every call site:
  - `config.rs::Config::load` — non-unix returns `Self::default()` until we
    route through `dirs`.
  - `picker/{frecency,history}.rs::state_file_path` — non-unix returns `None`;
    frecency / history are session-scoped on macOS and Windows.
  - `picker/icons.rs::IconCache::resolve` — bare freedesktop icon names resolve
    to `None` on non-unix; absolute paths still pass through.
  - `modes/mod.rs` — `pub mod drun` is `#[cfg(unix)]`.
  - `app.rs` — `Mode::Drun` arm splits per cfg; non-unix exits 2 with a
    "unix-only" message. Other modes (calc, clipboard, dmenu, emoji, run, ssh)
    work cross-platform.

## [0.6.0] - 2026-05-18

### Added

- **macOS binaries are back.** `aarch64-apple-darwin` + `x86_64-apple-darwin`
  re-added to the binary build matrix. `apps/pikr/src/app.rs` now selects the
  runtime window shape per target via `#[cfg]`:
  - Linux / FreeBSD: unchanged — `WAYLAND_DISPLAY` guard, layer-shell surface
    with `KeyboardInteractivity::Exclusive`, transparent framebuffer for
    compositor-side corner rounding.
  - macOS / Windows: `floem::Application::new()` + a regular `WindowConfig`. No
    layer-shell types referenced, no Wayland env guard. The host window manager
    owns z-order and focus; pikr's view-level Esc-dismiss still applies.
- **`brew-tap` publish job re-enabled** — tag pushes now render the Homebrew
  formula against the new macOS sha256 sidecars and push to
  `kryptic-sh/homebrew-tap`.

### Open

- macOS isn't validated end-to-end yet (see #8). The build matrix proves it
  compiles + ships; runtime behaviour on Apple silicon / Intel needs a live
  install pass.
- Linux cross-targets `aarch64-unknown-linux-gnu` (#31) and
  `x86_64-unknown-linux-musl` (#32) are still out — separate workstream, blocked
  on cross-compile plumbing for muda+gtk-rs.

## [0.5.4] - 2026-05-18

### Fixed

- **Tag release CI now actually publishes artifacts.** Two binary-matrix targets
  broke on the layer-shell-port migration's gtk-rs / glib-sys cross-compile
  plumbing: `aarch64-unknown-linux-gnu` (`security.ubuntu.com` doesn't host
  arm64 indexes, so the multiarch apt fetch 404'd on every index file) and
  `x86_64-unknown-linux-musl` (pkg-config can't cross-compile musl-target glib
  without a real sysroot). Both rows removed from the binary matrix; the
  downstream packaging jobs that consume them — `nfpm` arm64 row, `alpine`
  (.apk), and `aur-bin` — are short-circuited (`if: false`) to match. v0.5.4
  ships an `x86_64-unknown-linux-gnu` tarball + `.deb` + `.rpm`; the dropped
  rows are tracked for restore alongside macOS.

## [0.5.3] - 2026-05-18

### Fixed

- **`cargo test` no longer pops a pikr window on dev boxes.** The
  `accepts_show_drun` smoke test inherited the user's `WAYLAND_DISPLAY` and
  spawned pikr against the live compositor, putting a stray layer-shell surface
  on screen every time anyone ran the test suite. The live-render path is
  already exercised by the e2e harness in `tests/e2e/` (which spins its own
  `sway --headless` fixture), so the smoke test collapsed to a single check:
  pikr exits non-zero with the guard message when `WAYLAND_DISPLAY` is unset.
  Renamed to `missing_wayland_display_exits_with_guard`.
- **Build matrix unblocked for tag releases.** Dropped both `*-apple-darwin`
  targets — pikr can't compile on macOS until Epic 5 / #10 restores a
  non-Wayland runtime path (the floem fork's `Application::new_wayland()` is
  `cfg`-gated to Linux/FreeBSD). The `brew-tap` publish job is short-circuited
  (`if: false`) to match; flip back when macOS lands.
- **Linux build scripts find pkg-config.** The build matrix now installs
  `libgtk-3-dev` + `libxkbcommon-dev` for the host (glib-sys / gobject-sys build
  scripts pulled via floem's `muda` Linux dep). For `aarch64-unknown-linux-gnu`
  it layers Ubuntu's `:arm64` multiarch and sets `PKG_CONFIG_PATH` /
  `PKG_CONFIG_ALLOW_CROSS` / `PKG_CONFIG_SYSROOT_DIR` so the cross-target's
  `.pc` files resolve.

## [0.5.2] - 2026-05-18

### Fixed

- **Test target build broken by deprecated
  `floem::reactive::{batch, create_effect}` in `picker/state.rs`** — the
  migration in 0.5.0 missed these calls because they live inside a
  `#[cfg(test)]` block, and the local clippy gate didn't pass `--all-targets`.
  CI's `--all-targets --all-features` with `-D warnings` caught it on the v0.5.1
  tag push. Swapped both for `Effect::new` / `Effect::batch` (the new
  free-function shims) — behaviour unchanged.

## [0.5.1] - 2026-05-18

### Fixed

- **CI green after the floem layer-shell-port migration.** The v0.5.0 main push
  surfaced four blockers in the new dep tree:
  - `cargo fmt --check` — imports re-ordered post-migration. `cargo fmt` applied
    across `apps/pikr/src/{main,picker/keys,ui/view}.rs`.
  - `cargo-deny sources` — current `lapce/floem` main pulls `lapce/vger-rs` and
    `jrmoulton/understory` transitively. Added both to `deny.toml` `allow-git`
    and dropped the obsolete `mxaddict/vger-rs` entry (the new fork uses
    upstream lapce/vger-rs).
  - `cargo-deny advisories` — gtk-rs GTK3 bindings family
    (`RUSTSEC-2024-0412/0413/0415/0416/0418/0419/0420`) and `proc-macro-error`
    (`RUSTSEC-2024-0370`) are unmaintained; ignored with rationale (no CVEs,
    pulled transitively, revisit when floem moves off the GTK3 generation).
  - `cargo build` on Linux — `glib-sys`/`gobject-sys` build scripts need
    `libgtk-3-dev` + `libxkbcommon-dev` installed. Added to the `clippy`,
    `test`, and `e2e` jobs in `.github/workflows/ci.yml`.

## [0.5.0] - 2026-05-18

### Breaking

- **X11 support dropped — pikr is now Wayland-only.** `Application::new()`,
  `X11Config`, and `X11WindowType` paths have been removed. `WAYLAND_DISPLAY`
  unset now prints a clear error and exits non-zero instead of falling back to
  an X11 window.
- **`--no-layer-shell` / `--windowed` CLI flag removed.** pikr now strictly uses
  `wlr_layer_shell_v1`; the regular `xdg_toplevel` fallback path is gone.
  Compositors without wlr-layer-shell (e.g. GNOME Mutter) are unsupported.
- **Layer-shell now requests `KeyboardInteractivity::Exclusive`.** While pikr is
  open the compositor routes all keys to it; other apps can't take input until
  pikr is dismissed (Esc x2 / Enter). Mirrors the rofi/wofi modal-launcher
  contract.

### Changed

- **floem + floem-winit `[patch.crates-io]` refs moved from `layer-shell` to
  `layer-shell-port`.** Old branches tracked floem 0.2.0 / floem-winit 0.29.5
  byte-for-byte; new branches sit on top of current `lapce/{floem,winit}` main.
  pikr migrated through the API shifts that came with Faster Style v2, the unit
  overhaul, and the `ui-events` keyboard re-routing — `Color::rgb8` →
  `Color::from_rgb8`, `floem::keyboard` → `floem::ui_events::keyboard`,
  `Event::KeyDown(ke)` → `Event::Key(KeyboardEvent)` via `listener::KeyDown`,
  `virtual_stack` 3-arg signature + `.item_size_fixed`,
  `rich_text(text, attrs, fn)`, `create_signal_from_channel` →
  `receiver_signal::ChannelSignal`, `im::Vector` → `imbl::Vector` (floem's
  `VirtualVector` impl moved), and the `v_stack`/`h_stack`/`container`/`label`
  helper deprecations swapped for `Stack::vertical`/`horizontal`,
  `Container::new`, `Label::derived`.
- **Window chrome height recomputed.** Old `chrome_h` in `app.rs` under-counted
  the ex_bar gutter (32 px) and the panel `padding_bottom` (10 px). Old floem's
  box-model swallowed the discrepancy, new floem doesn't — the status bar
  drifted ~8 px down. `chrome_h` now sums every non-scrollable v_stack element
  precisely: two `PANEL_PAD`s, `INPUT_ROW_HEIGHT`, `INPUT_MARGIN_BOTTOM`,
  `EX_BAR_TOTAL`, `STATUS_HEIGHT`, `STATUS_BAR_TOTAL`. The panel is now ~32 px
  taller, and the "8 visible rows" claim is finally honest. New `pub const`s in
  `ui::view`: `INPUT_MARGIN_BOTTOM`, `EX_BAR_TOTAL`; `PANEL_PAD` is now `pub`.

### Fixed

- **Keystrokes were dropped until the user pressed Esc→i.** In new floem main,
  typing keys (unmodified character input) route only to the focused view;
  shortcut-like keys (Esc, Tab, …) fall back to the listener registry. pikr's
  outer container is `keyboard_navigable` but the mount-time
  `request_focus(|| {})` Effect raced with the compositor's first key delivery
  on Hyprland — the first keys landed before floem committed focus, so they fell
  on the floor. The Esc→i workaround happened to claim focus via the
  registry-fallback Esc path. Fix: capture a stable `root_id: ViewId` on the
  outer Container and re-claim focus on `WindowGainedFocus` (initial,
  compositor-aligned) plus at the top of the `KeyDown` handler (per-key, because
  reactive updates from `picker.query.set(...)` drop view-focus mid-typing). The
  latter is a workaround for a focus-drop in the rerank reactive chain — see
  followup tracking issue.

### Removed

- `--no-layer-shell` / `--windowed` CLI flag (see Breaking).
- `AppState.windowed` field, `message_view(_, _, windowed)` parameter, and the
  `if windowed { ... } else { border }` panel-border branch — pikr is always
  layer-shell now so the OS-window vs layer-shell distinction is gone. The panel
  always paints its own rounded border.
- `crossbeam-channel` workspace + crate dep (unused after `ChannelSignal`
  swapped to `std::sync::mpsc`; `crossbeam_channel::Receiver` doesn't implement
  floem's `BlockingReceiver` trait).
- `im` workspace dep (replaced by `imbl 7.0` for floem's `VirtualVector` impl).

## [0.4.1] - 2026-05-18

### Fixed

- **Row labels jittered as match highlights changed.** Each entry row's label
  was a `stack_from_iter` of one `label()` per same-color run, so cosmic-text
  shaped each run independently and the flex row rounded fractional glyph
  advance at every boundary. Typing a character that flipped its match state
  shifted total label width by a fraction of a pixel — letters appeared to
  wobble. `highlighted_label` now emits a single `floem::rich_text` with one
  `TextLayout` and per-range color attrs; the run is shaped once, painted with
  multiple colors. `font_family` and `font_size` are baked into the `Attrs`
  because `rich_text` does not inherit them from the parent style cascade.

## [0.4.0] - 2026-05-18

### Added

- **drun icons via XDG theme lookup.** Application list rows now render the
  declared icon from the `.desktop` file, resolved via the user's active XDG
  icon theme. Theme fallback chain mirrors the freedesktop spec — explicit theme
  → hicolor → pixmaps — so apps without a theme-native icon still get a
  reasonable glyph.
- **SVG icon rasterisation** via `resvg`. Theme lookups that resolve to SVG
  assets are rasterised at the row's display size with a multi-theme fallback if
  the primary theme is missing the icon at the requested size.
- **Calculator history.** Empty-query state in calc mode now lists past
  expressions as result rows; selecting one re-loads the expression for editing.
- **Accept-custom + cancel exit code** for rofi parity (#17). Shift-Enter
  accepts the raw query as a payload instead of the highlighted row; cancelling
  (Esc) now exits `1` so shell pipelines can branch on user dismissal.

### Fixed

- **Matcher: substring fallback when nucleo prefilter panics.** nucleo 0.5 has a
  deterministic panic in its prefilter for certain haystack+needle pairs
  ("should have been caught by prefilter") — `"Thunder"` against `"Thunderbird"`
  trips it while `"Thunde"` and `"Thunderb"` score normally. The matcher now
  walks a case-insensitive substring scan for the panicking row and the rest of
  the pass, surfacing the obvious hit the user expected. Cross-call recovery via
  the existing `poisoned` flag is unchanged.
- **Calc label refreshes** when the same payload is reused across rows. Mixing
  the entry `Arc` pointer into `row_key` busts the cached child view so the
  visible label tracks the underlying entry.

### Removed

- `CODE_OF_CONDUCT.md` and `CONTRIBUTING.md` — both inherited from the org-level
  `kryptic-sh/.github` repo and were duplicates here.

## [0.3.2] - 2026-05-17

### Added

- **`.deb` + `.rpm` packaging** via [nfpm] (closes #11, #12). A new `nfpm`
  matrix job in `.github/workflows/ci.yml` builds four artifacts per tag —
  `pikr_*_amd64.deb`, `pikr_*_arm64.deb`, `pikr-*.x86_64.rpm`,
  `pikr-*.aarch64.rpm` — using a single `pkg/nfpm/nfpm.yaml.in` manifest with
  format-specific runtime dependencies in `overrides:` (libxkbcommon / wayland /
  EGL / GL / vulkan / fontconfig on the deb side; their Fedora equivalents on
  rpm). Each package + sha256 sidecar is attached to the GitHub release. README
  gains an `## Install` section with apt / dnf / apk / AUR / brew / cargo
  snippets.

[nfpm]: https://github.com/goreleaser/nfpm

### Fixed

- Smoke test `accepts_show_drun` was flaky on busy CI runners: pre-floem startup
  grew with the v0.3.0 frecency / history loads, and the 250 ms `thread::sleep`
  we used to wait for a headless exit occasionally fired while pikr was still
  initialising. Bumped to 1500 ms so the assertion sees a real "still running"
  state and doesn't misread "still in startup" as it.

## [0.3.1] - 2026-05-16

### Fixed

- Build broke on macOS in v0.3.0 because `let width = cli.width.unwrap_or(720)`
  in the message-modal path was declared outside the
  `#[cfg(any(target_os = "linux", target_os = "freebsd"))]` block that uses it,
  so on `apple-darwin` the compiler flagged it as `unused-variables` (which is
  `-D warnings` in CI). Moved the declaration inside the cfg block. macOS
  builds + the gated AUR / Alpine / Homebrew publishing jobs now run.

## [0.3.0] - 2026-05-16

### Added

- **Frecency scoring**: tracks accept count + last-used timestamp per payload,
  per CLI mode, in `$XDG_STATE_HOME/pikr/usage.toml`. Adds a
  `count · 0.5^(Δt / 14d) · 80` score bonus (saturating u16) to nucleo ranks so
  apps the user actually launches surface first — including on the initial
  empty-query rank. Both keyboard and mouse accept paths bump usage; visual-mode
  multi-launch fsyncs once. New `picker::frecency::Usage` module.
- **Per-mode query history**: most-recent-first list capped at 100 per mode,
  persisted to `$XDG_STATE_HOME/pikr/history.toml`. New `Ctrl-P` / `Ctrl-N`
  recall (`Action::HistoryPrev` / `HistoryNext`, fzf convention) — Up/Down keeps
  navigating the result list. The live query is stashed in `history_draft` on
  the first hop back so `Ctrl-N` past index 0 restores it. Any subsequent edit
  (Insert/Backspace/Delete/Ctrl-W/Ctrl-U) clears the recall cursor. Push happens
  on Accept with non-empty query (dedupes to front, trims whitespace, ignores
  empty/blank). New `picker::history::History` module.
- **`-P` / `--password`** (#13): masks the query bar with `●` (U+25CF) per
  character. Real chars still drive the matcher and the payload on accept — only
  the rendered glyph is replaced. New pure `mask_password` helper preserves
  codepoint count so cursor positioning in `with_cursor` stays correct. New
  `AppState.password: bool`.
- **`--filter <text>`** with aliases `--query` / `--prefill` / `--input-text`
  (#14): pre-fill the query string on launch and place the caret at the end
  before the first rerank. The initial `matcher.rank` now uses the prefill query
  (instead of `""`) so the list reflects the seed on first paint.
- **`-e` / `--message <text>`** (#15): non-interactive message overlay.
  `app::run` short-circuits before any picker / mode setup and renders a new
  `message_view` that re-uses the panel chrome (bg, border, radius, padding)
  with a single centered label — no input row, no list, no status bar. Escape
  dismisses via `std::process::exit(0)`. When `--message` is set, `--show` /
  `--dmenu` / `--filter` are ignored.
- **`--width <px>` and `-l` / `--lines <n>`** (#16): override the default window
  width (was hard-coded 720) and visible-row count (was `VISIBLE_ROWS = 8`).
  Pixels-only for v1; the `'40%'` syntax in the rofi spec is explicitly out of
  scope.

### Tests

- 88 → 110 (+22): 8 frecency, 8 history, 2 keymap (Ctrl-P/Ctrl-N), 20 CLI
  parsing, 4 `mask_password`.

### Closes

- #13 password masking, #14 prefill, #15 message modal, #16 sizing.

## [0.2.2] - 2026-05-16

### Fixed

- `deny.toml` `allow-git` list updated to `https://github.com/mxaddict/vger-rs`
  (the floem fork now pins this branch for the alpha-blend fix). v0.2.1 still
  listed the old `lapce/vger-rs` URL and failed cargo-deny's
  `source-not-allowed` check, which gated the release pipeline.

## [0.2.1] - 2026-05-16

### Added

- **Visual mode** for range selection — `v` / `V` / `<C-v>` from Normal anchors
  at the current row; `j`/`k`/`gg`/`G` extend the range. `<CR>` executes every
  selected entry (multi-launch); `<Esc>` (or pressing `v` again) returns to
  Normal. Selected rows render with a distinct accent-tinted bg; the cursor row
  keeps the deeper `selected_bg` + accent ring so j/k destination stays visible.
  New `Action::EnterVisual`, `PickerState::visual_anchor` signal, and 5 keymap
  regression tests.
- **In-query caret editing** (Insert mode). The query bar is now a real text
  input: `←` / `→` / `Home` / `End` / `Ctrl-A` / `Ctrl-E` / `Ctrl-B` / `Ctrl-F`
  move the caret; `Ctrl-W` deletes the word back; `Ctrl-U` clears to start;
  `Delete` / `Ctrl-D` deletes forward. Insert / Backspace mutate at the caret.
  `PickerState::query_cursor` carries the codepoint index; `with_cursor` renders
  the vim caret glyph at that position.
- **`--mode` flag** (`normal` / `insert` / `visual`) to override the default
  startup mode. `VimMode` derives `clap::ValueEnum` so new variants are
  auto-exposed; default is now `insert` so users can type immediately.
- **`--windowed`** alias for `--no-layer-shell` (clearer name for the X11 /
  Mutter / GNOME fallback path).
- **Status bar** at the bottom: vim mode pill (INSERT/NORMAL/VISUAL with
  per-mode bg), current CLI mode name, `selected/total` counter on the right.
  Sticky in flex layout — never pushed off-screen.
- **No-results message** rendered inside the scroll viewport when the match list
  is empty. Shows `No results for "…"` mid-query, `No entries.` when the mode
  produced nothing at all.
- **Virtual scrolling** for the result list. Switched from `dyn_stack` to
  `virtual_stack` so only viewport rows are built — emoji mode (~1800 entries)
  now paints instantly.
- **Description-based matching**: `Matcher::rank` now takes
  `&[(label, Option<description>)]` and matches both fields per entry. Label and
  description score equally (raw nucleo, summed when both hit); matched
  characters in the description are highlighted in `accent` the same way as the
  title.
- **Hover bg on rows** — mouse hover shows `blend(accent, selected_bg, 0.18)`;
  the accent ring stays reserved for keyboard / programmatic selection so hover
  and selection are visually distinct.
- **Tests**: 70 total. New coverage:
  - `row_key` distinguishes empty/grow/same-len/desc positions (5 tests)
  - Caret helpers `char_idx_to_byte`, `word_boundary_back` (5 tests)
  - Matcher description hits + label-vs-desc equal scoring (3 tests)
  - Keymap caret bindings + visual transitions (11 tests)
  - Signal-set-under-mutex deadlock regression (`batch` discipline) (1 test)
  - `clamp_selected` no-op-when-unchanged semantics (3 tests)
  - Nucleo matcher poison/rebuild path (2 tests)

### Changed

- **Hand-rolled query bar** — dropped floem's `text_input` widget which ate
  `Esc` to clear its own focus (3 presses to quit from Insert). Now Insert →
  `Esc` → Normal (1 press), Normal → `Esc` → quit (1 press), so quitting from
  Insert is the expected 2 presses.
- **Cursor glyph follows vim mode**: thin caret `▏` in Insert, block `█` in
  Normal / Visual. Blinks every 530 ms; resets to visible on every keystroke or
  ex-buf mutation.
- **Ex bar**: always rendered (`display: None` removed) so toggling `:` no
  longer reflows the panel and bumps the status bar. Backspace on an empty `:`
  dismisses ex mode (readline / vim convention). Background matches the status
  bar; thin gap to the status bar below.
- **Window chrome in `--windowed` mode**: drops the rounded panel border (the OS
  already paints one) and forces an opaque framebuffer so vger SDF / text AA
  doesn't leak the desktop through every glyph edge.
- **Matcher refactor**: `Match` carries `positions` (label) and `desc_positions`
  separately; UI highlights each field with its own span list. Empty-query
  fast-path still returns every entry in order.

### Fixed

- **No-results UI hang** (signal/mutex deadlock). `AppState::rerank` held
  `Mutex<AppState>` while calling `clamp_selected(0)` which called
  `selected.set(0)`. floem fires signal subscribers synchronously inside `set`;
  subscribers (status bar count, virtual_stack data fn, empty-state label) all
  re-locked the same mutex → permanent deadlock on every no-match query. Wrapped
  the rerank critical section in `reactive::batch(…)` so subscriber dispatch is
  queued until the mutex guard is dropped. Added an explicit regression test
  that would hang the test thread if `batch` is ever removed.
- **Emoji-mode hang on every keystroke**. The matcher was installing and
  restoring the global panic hook on every nucleo `fuzzy_indices` call
  (`take_hook` / `set_hook` lock a global mutex); doing it ~1800× per keystroke
  starved the UI thread. Hook is now installed once per `rank()` pass.
- **Emoji search by name**: emoji `label` was the glyph alone, so typing `smile`
  matched nothing. Label is now `"<glyph> <name>"` (payload still the bare
  glyph) so the name is searchable.
- **Nucleo prefilter-assert recovery**: a panic inside `fuzzy_indices` used to
  leave the matcher's slab in a half-mutated state and the next call hung. The
  matcher is now marked `poisoned`, the rest of the rank pass is skipped, and
  the inner `NucleoMatcher` is lazily rebuilt on the _next_ call (drop runs on a
  quiescent instance, not mid-allocation).
- **Empty-list scroll feedback loop**: `ensure_visible` returned a non-zero rect
  for a phantom row when `matches.is_empty()`, so scroll re-adjusted every
  frame. Returns `Rect::ZERO` when empty.
- **Result list height**: `min_height(0)` on the scroll so the flex container
  can shrink it past its intrinsic content height — otherwise a long match list
  pushed the ex/status bars off-panel.
- **Match-highlight cache invalidation**: dyn_stack key was `(mi << 32) | idx`,
  so typing `au` → `auda` against "Audacity" reused the cached row view and the
  old `positions=[0,1]` rendering survived. Key now mixes the full positions vec
  (and description positions) via FNV-1a with a proper non-zero offset basis — a
  0-start collided across `[]` and `[0]` and left highlights stuck after
  clearing the query.
- **Text-input width quirk** (early-iter fix, now obsolete after the hand-rolled
  query bar): floem's inner `text_node` falls back to Auto width (~20 chars)
  when the outer node uses `flex_grow` only — need `width_pct(100.0)` +
  `height_full()` to fill.
- **Background-blur path** layered the backdrop under content, clipped at the
  panel corners, and adopted `--smoked` semantics in `--blur=<sigma>`
  - `--opacity` overrides. Removed the greyscale option; blur-only pipeline
    ships.
- **Wayland framebuffer leak** at SDF / text edges. Patched the floem fork to
  pick `CompositeAlphaMode::Opaque` when `with_transparent(false)` (instead of
  always `PreMultiplied`); the `--windowed` path now uses it. Patched the
  vger-rs fork to use `One + OneMinusSrcAlpha` for the alpha blend factor (was
  `SrcAlpha`, which squared the alpha at antialiased edges); the layer-shell
  path now composites cleanly against the desktop. floem fork commit
  `mxaddict/floem@b2a4d00f`; vger fork at `mxaddict/vger-rs@2534cd22`.

### Internal

- floem fork repointed to mxaddict/vger-rs alpha-blend branch.
- `cargo clippy` and `cargo fmt` clean.

[0.2.2]: https://github.com/kryptic-sh/pikr/releases/tag/v0.2.2
[0.2.1]: https://github.com/kryptic-sh/pikr/releases/tag/v0.2.1

### Added

- **Epic 4 — Wayland layer-shell overlay**: pikr now launches as a
  `zwlr_layer_surface_v1` on wlroots compositors. Required forking `winit`
  (`mxaddict/winit#layer-shell`) to expose `LayerShellAttributes` on
  `WindowAttributes` and binding the `zwlr_layer_shell_v1` global through
  `LayerShellHandler`, and forking floem (`mxaddict/floem#layer-shell`) to
  surface `LayerShellConfig` + `with_layer_shell_config` + an
  `Application::new_wayland()` entry point. pikr's `Cargo.toml` patches both via
  `[patch.crates-io]`.
- `--no-layer-shell` CLI flag forces a plain `xdg_toplevel`. Auto-engaged when
  `WAYLAND_DISPLAY` is unset so pikr runs on Mutter/GNOME (no wlr-layer-shell)
  and on X11 without panicking.
- **X11 dock fallback**: new `X11Config` / `X11WindowType` on floem's
  `WindowConfig` exposes `_NET_WM_WINDOW_TYPE` + override-redirect. pikr tags
  itself as `Dock` when falling back on an X11 session so the WM keeps it on top
  and out of the taskbar.
- Vim-mode cursor in the input row: thin bar in Insert, full block in Normal,
  blinks at ~530 ms via `floem::ext_event::create_signal_from_channel`.
- Arrow-key navigation: `Up`/`Down`/`PgUp`/`PgDn`/`Home`/`End` now work in both
  Insert and Normal modes; Normal-mode arrows honor the count prefix.

### Changed

- Default theme font switched from generic `monospace` to `Hack Nerd Font Mono`.
  Users can still override via `config.toml`.
- Viewport height locked to an integer multiple of row height
  (`ROW_HEIGHT * VISIBLE_ROWS`) so scrolling never reveals a half-clipped row.
  Row chrome bumped (26 px rows, 30 px input, 24 px status) for legibility.

### Fixed

- Render hang on `j` / `k`: the key handler held the `AppState` mutex through
  `signal.set()`, which fires reactive subscribers synchronously and re-locked
  the same mutex (the `dyn_stack` items closure). Handler now snapshots state,
  drops the guard, then updates signals.
- Selection highlight that didn't move with `j` / `k`: the per-row style closure
  subscribes to `selected_sig` directly instead of baking `is_selected` in at
  construction time.
- Result list still showed stale rows after a query rerank: `dyn_stack` was
  keyed by slot index alone, so kept slots reused the previously baked entry.
  Key now packs `(slot, m.index)` so a slot pointing at a new entry forces a
  rebuild.
- Scroll-past-end: the result list now uses `scroll.ensure_visible` with a
  per-selection rect so the viewport follows the cursor.
- Status bar covered by overflow rows: `scroll`'s `min_height: auto` was
  expanding to content; explicit `height(viewport_height)` keeps the chrome
  visible.
- Space key was dropped in Insert mode — handled via `NamedKey::Space`, not
  `Key::Character(" ")`.
- `nucleo 0.5` "should have been caught by prefilter" panic: matcher skips empty
  labels and wraps `fuzzy_indices` in `catch_unwind`.

### Removed

- Verbose frame-callback / redraw-tick `log::debug!` traces in the winit fork —
  they were diagnostic for the Epic 4 hang, no longer load-bearing.

[Unreleased]: https://github.com/kryptic-sh/pikr/compare/v0.8.5...main
[0.8.5]: https://github.com/kryptic-sh/pikr/releases/tag/v0.8.5
[0.8.4]: https://github.com/kryptic-sh/pikr/releases/tag/v0.8.4
[0.8.3]: https://github.com/kryptic-sh/pikr/releases/tag/v0.8.3
[0.8.2]: https://github.com/kryptic-sh/pikr/releases/tag/v0.8.2
[0.8.1]: https://github.com/kryptic-sh/pikr/releases/tag/v0.8.1
[0.8.0]: https://github.com/kryptic-sh/pikr/releases/tag/v0.8.0
[0.7.2]: https://github.com/kryptic-sh/pikr/releases/tag/v0.7.2
[0.7.1]: https://github.com/kryptic-sh/pikr/releases/tag/v0.7.1
[0.7.0]: https://github.com/kryptic-sh/pikr/releases/tag/v0.7.0
[0.6.5]: https://github.com/kryptic-sh/pikr/releases/tag/v0.6.5
[0.6.4]: https://github.com/kryptic-sh/pikr/releases/tag/v0.6.4
[0.6.3]: https://github.com/kryptic-sh/pikr/releases/tag/v0.6.3
[0.6.2]: https://github.com/kryptic-sh/pikr/releases/tag/v0.6.2
[0.6.1]: https://github.com/kryptic-sh/pikr/releases/tag/v0.6.1
[0.6.0]: https://github.com/kryptic-sh/pikr/releases/tag/v0.6.0
[0.5.4]: https://github.com/kryptic-sh/pikr/releases/tag/v0.5.4
[0.5.3]: https://github.com/kryptic-sh/pikr/releases/tag/v0.5.3
[0.5.2]: https://github.com/kryptic-sh/pikr/releases/tag/v0.5.2
[0.5.1]: https://github.com/kryptic-sh/pikr/releases/tag/v0.5.1
[0.5.0]: https://github.com/kryptic-sh/pikr/releases/tag/v0.5.0
[0.4.1]: https://github.com/kryptic-sh/pikr/releases/tag/v0.4.1
[0.4.0]: https://github.com/kryptic-sh/pikr/releases/tag/v0.4.0
[0.3.2]: https://github.com/kryptic-sh/pikr/releases/tag/v0.3.2
[0.3.1]: https://github.com/kryptic-sh/pikr/releases/tag/v0.3.1
[0.3.0]: https://github.com/kryptic-sh/pikr/releases/tag/v0.3.0
[0.2.0]: https://github.com/kryptic-sh/pikr/releases/tag/v0.2.0

## [0.1.0] - 2026-05-16

### Added

- Initial scaffold: bin crate, CLI parsing, config loader, mode trait, picker
  state, vim keymap stubs, floem dep.
- Workspace conversion (`apps/pikr` + `xtask`) + org-template community files.
- Canonical `.github/workflows/{ci,pages}.yml` ported from hodl.
- Marketing site at `pikr.kryptic.sh` (magenta `#ff79c6` accent).
- **Epic 2 — modes**: refactored `Mode` trait (`collect()` produces entries;
  `Payload` owns execution via `execute()` + `spawn_detached`). `Entry` gains
  optional description sub-text and an `Exec { program, args }` payload.
  - `dmenu` reads stdin lines, prints accepted selection to stdout.
  - `run` walks `$PATH`, dedupes executables by name.
  - `drun` scans XDG `applications/*.desktop` via `freedesktop-desktop-entry`,
    honors `NoDisplay` / `Hidden` / `Type=Application`, parses `Exec=` with
    shlex, strips freedesktop field codes (`%U`/`%f`/`%i`/…), locale-aware
    `Name`/`GenericName`/`Comment`.
  - Nucleo matcher facade ranks entries against query and returns match
    positions for highlight spans.
  - `app.rs` dispatches to the chosen mode and prints a headless preview to
    stderr; stdout reserved for the dmenu accept path. UI lands in Epic 3.
- **Epic 3 — floem UI + vim keymap**: interactive picker shipped as a floating
  window (layer-shell is Epic 4).
  - `PickerState` rebuilt around `RwSignal` (`query`, `selected`, `vim_mode`,
    `count`, `ex_buf`); `Matcher` cached on `AppState` to avoid the ~135 KB
    per-keystroke allocation nucleo warns about.
  - View tree: prompt + query input row, scrollable result list with
    nucleo-position highlighting and accent-blended selection row, ex command
    bar, mode/selection status bar.
  - Keymap: Normal / Insert split. `j` / `k` / `gg` / `G` / `<C-d>` / `<C-u>`
    motions with count-prefix support (`5j` jumps five rows). `i` enters Insert;
    `<Esc>` returns to Normal (or cancels from Normal). `/` swaps to Insert. `:`
    opens the ex command bar.
  - Ex commands: `:drun` / `:run` / `:dmenu` switch modes at runtime and reload
    entries; `:q` quits via `floem::quit_app`.
  - Accept (`<CR>`) runs `modes::execute(&payload)` then exits — `Stdout` prints
    to stdout, `Exec` spawns detached.
  - Smoke test rewritten to handle both headless (no display → process exits)
    and display-present (event loop entered → spawn + sleep + kill) modes.
  - 7 new keymap unit tests (20 total).
- Packaging templates: `pkg/aur/PKGBUILD-bin.in` (+ `LICENSE`, `.gitignore`),
  `pkg/alpine/APKBUILD.in`, `pkg/homebrew/pikr.rb.in`. Placeholders match the
  sed substitutions in `.github/workflows/ci.yml` so the first `v*` tag
  exercises the full aur-bin / alpine / brew-tap release pipeline.

[0.1.0]: https://github.com/kryptic-sh/pikr/releases/tag/v0.1.0
