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

[Unreleased]: https://github.com/kryptic-sh/pikr/compare/HEAD...HEAD
