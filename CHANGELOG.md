# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/kryptic-sh/pikr/compare/HEAD...HEAD
