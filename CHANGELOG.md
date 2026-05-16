# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-05-16

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

[Unreleased]: https://github.com/kryptic-sh/pikr/compare/v0.2.0...HEAD
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
