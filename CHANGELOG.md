# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/kryptic-sh/pikr/compare/v0.3.0...HEAD
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
