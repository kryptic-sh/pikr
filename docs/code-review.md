# Code review — 2026-08-04

Scope: full codebase (working tree clean, `main` @ `31980af`). All 32 `.rs`
files (~7,900 lines) read. Findings verified by tracing against the resolved
dependency sources (floem `8c52a32` git checkout, freedesktop-desktop-entry
0.7.19) and, for finding 1, reproduced live on the release binary under
`sway --headless` + `wtype`.

## Findings

### 1. HIGH — `:mode` ex-command deadlocks the UI thread (every mode switch hangs pikr)

`apps/pikr/src/ui/view.rs:1275` calls `switch_mode` while holding the
`AppState` mutex; `switch_mode` (view.rs:812–814) mutates `picker.query` /
`picker.selected` / `picker.query_cursor` signals inside that guard. Floem
`RwSignal::set` dispatches subscribers **synchronously** on the UI thread when
not inside `Effect::batch` (verified in `reactive/src/signal.rs:624–640` of the
resolved floem checkout: `run_effects` → `run_pending_effects` when
`!batching`). The subscribers re-entered are the rerank effect (view.rs:995–1021)
and the status-bar count label (view.rs:593–602), both of which do
`state.lock()` on the **same** non-reentrant `std::sync::Mutex` — the one
already held by the keydown handler. This is exactly the deadlock the
`PickerState` regression test (state.rs:107–143) documents for the
unbatched case; `:mode` is the one signal-mutation path that skips the
`Effect::batch` wrapper the rerank effect uses.

The `Effect::batch` inside the rerank effect does not help: the batch only
defers *subsequent* signal dispatch; the `state_rerank.lock()` inside the batch
closure runs immediately, on a mutex already held by the same thread.

Reproduced live (release binary, `sway --headless` + `wtype`, isolated
`XDG_CONFIG_HOME`/`XDG_STATE_HOME`):

```
Repro: pikr --show drun; type "x"; Esc; ':'; "run"; Enter; then Esc Esc
Expect: picker switches to run mode; Esc Esc exits 1
Actual: process hangs (alive >3 s, UI thread stuck; the trailing Esc Esc
        is never processed; "picker query reranked" fires for the typed
        query but never for the cleared query)

Repro: pikr --show drun; (empty query) Esc; ':'; "run"; Enter; then Esc Esc
Expect: exits 1
Actual: hangs — deadlocks at selected.set(0) → count-label derived locks the
        held mutex (control without the ex-mode part exits 1)
```

Reachable from any mode command (`calc`, `clipboard`, `dmenu`, `drun`, `emoji`,
`run`, `ssh`) whenever the picker is open. No e2e test covers the ex-command
path, which is why the suite is green.

Fix direction (not applied): wrap `switch_mode`'s signal mutations in
`Effect::batch`, or drop the mutex guard before calling it.

### 2. MEDIUM-HIGH — `--password` typed query is persisted to disk in plaintext

`apps/pikr/src/ui/view.rs:1419–1423` (Accept), `view.rs:1463` (AcceptCustom),
and `view.rs:1436` (dmenu no-match fallthrough) push the live query into
`History` and `save()` it; the mouse-click accept path (view.rs:347–349) does
the same. `History::save` (picker/history.rs:62–64) writes
`$XDG_STATE_HOME/pikr/history.toml` unconditionally. The `--password` flag
(cli.rs:39–42) only masks the query bar glyphs; nothing gates history/usage
persistence on it.

```
Repro: printf 'a\nb\n' | pikr --dmenu -P; type "S3cr3t"; Shift+Enter
Expect: "S3cr3t" printed to stdout, nothing written to disk
Actual: "S3cr3t" also appears in $XDG_STATE_HOME/pikr/history.toml
        (and, for Accept on a matched row, usage.toml) in plaintext
```

The whole point of `-P` is that the typed value is sensitive; persisting it
defeats the flag. Fix direction: skip `history.push`/`usage.record` when
`AppState::password` is set.

### 3. MEDIUM — drun: system `.desktop` files silently override user-local ones

`apps/pikr/src/modes/drun.rs:88` dedupes by app id with
`by_id.insert(id, entry)` (last writer wins). Iteration order is
**user-first**: `freedesktop-desktop-entry`'s `default_paths()`
(0.7.19 `src/lib.rs:656–663`) yields `$XDG_DATA_HOME/applications` before
`$XDG_DATA_DIRS/applications`, and `Iter` walks them in order. So the *system*
copy (iterated later) overwrites the user's `~/.local/share/applications`
override. The code comment (drun.rs:46–47) asserts the opposite direction —
"later .desktop files (user-local) override earlier ones (system)" — so the
intended behaviour and the actual behaviour disagree, and the actual behaviour
contradicts the freedesktop convention (first match in search order wins;
user-local is the override layer that GNOME/KDE/rofi all honour).

```
Repro: ~/.local/share/applications/firefox.desktop (custom Exec/Name)
       + /usr/share/applications/firefox.desktop (system)
Expect: user's custom entry is shown
Actual: the system entry is shown; the user's override is silently discarded
```

A user who customises a launcher entry sees the stock one — or nothing
custom — with no error.

### 4. LOW-MEDIUM — Windows drun cache never invalidates on shortcuts added/removed inside existing subfolders (platform-gated, not compiled locally)

`apps/pikr/src/modes/drun.rs:257–269` keys the cache on the **root** Start
Menu `Programs` directory mtimes, and `load_cache` (drun.rs:278) invalidates
only when a root mtime changes. Directory mtimes update only when the
directory's own entries change, not when a grandchild changes: installing
`Programs\Vendor\App.lnk` into an *existing* `Vendor` folder leaves the
`Programs` root mtime untouched, so the new app is missing from the picker
until some unrelated change touches the root. The comment at drun.rs:167–172
documents only the uninstall-staleness tradeoff; the add-in-subfolder staleness
is undocumented and far more common (new installs land in subfolders).

```
Repro (Windows): cache written; create Programs\Games\Steam.lnk where Games/
                 already exists; relaunch pikr
Expect: Steam in the list
Actual: Steam absent until the root mtime changes (new subfolder, .lnk
        directly in Programs, etc.)
```

### 5. LOW — non-unix `:drun`/`:run`/`:ssh` mode switch leaves stale entries under the new mode label (platform-gated, not compiled locally)

`apps/pikr/src/ui/view.rs:795`, `view.rs:800`, `view.rs:804` —
`#[cfg(not(unix))] CliMode::Drun => return` exits `switch_mode` **after**
`self.cli_mode = mode` (line 787) but before replacing `entries`, clearing the
query, or reranking. On macOS/Windows, `:drun` from inside another mode sets
the status-bar mode to "drun" while the result list still shows the previous
mode's entries and the previous query.

### 6. LOW — history-recall state leaks across `:mode` switches

`history_cursor`/`history_draft` (view.rs:879–880) are picker_view-local
signals; `switch_mode` (view.rs:786–816) does not reset them, and the recall
handlers (view.rs:1550–1603) only reset on user edits.

```
Repro: Insert; type "fi"; Ctrl-P (history_cursor=Some(0), draft="fi");
       Esc; ':'; "run"; Enter; i; Ctrl-N
Expect: no-op (nothing recalled in the new mode)
Actual: query is restored to "fi" (the previous mode's draft), and the next
        Ctrl-P walks the run-mode history starting at index 1, skipping the
        most recent entry
```

## Cleared

- **Matcher unicode/grapheme positions** (picker/matcher.rs): NFC'd query vs
  first-codepoint-per-grapheme haystack, positions converted back via
  `grapheme_positions_to_codepoints` — traced and covered by the suite
  (decomposed/composed, ZWJ family, emoji). Correct.
- **Frecency math** (picker/frecency.rs): `count·0.5^(Δt/HALF_LIFE)·80`
  clamped to u16; `saturating_add` at both accumulation and score-merge sites;
  pre-1970 clock handled. Correct.
- **History push/dedupe/trim/cap** (picker/history.rs): verified against the
  documented semantics; `get`/`list`/`len` agree.
- **row_key Arc-pointer reuse** (ui/view.rs:1069): suspected stale calc rows
  if a freed Arc's address were reused — impossible for consecutive
  generations, because generation N+1's `Arc` is allocated while generation N
  is still alive, so the pointers cannot collide.
- **Deadlock in the rerank path itself**: the `Effect::batch` wrap at
  view.rs:1006 is real and correct — the no-results hang is fixed. The bug is
  that `:mode` bypasses the same pattern (finding 1).
- **Visual mode with `visual_anchor = None`** (CLI `--mode visual`): all
  consumers match `(Visual, Some(a))` and fall through to single-row behaviour.
  Safe.
- **Clipboard `sh -c "cliphist decode {id} | wl-copy"` injection**: `id` is a
  `u64` parsed from the line, digits only — no injection surface.
- **`parse_exec` / field-code stripping**: shlex tokens, `%%`→`%`, single-char
  `%X` dropped; argv spawn, no shell. Spec-quirky `%`-in-arg lines degrade
  (token loses trailing `%`) but cannot inject.
- **`gg`/`g_pending`**: pending flag cleared on any non-`g` key; `G` still
  bottoms. Correct.
- **`clamp_selected` no-op suppression**: avoids spurious subscriber dispatch
  as claimed (state.rs tests cover the fire/no-fire split).
- **Caret/word math** (`with_cursor`, `char_idx_to_byte`,
  `word_boundary_back`, `mask_password`): char-index arithmetic consistent;
  byte-offset helpers clamp; all unit-tested including multibyte.

## Hardening

- **`payload_key` ambiguity** (picker/frecency.rs:127): `Exec{program:"a b"}`
  and `Exec{program:"a", args:["b"]}` stringify to the same key, so two
  distinct payloads share one frecency record. Reachable via `Exec=` lines with
  a quoted program or space-containing args. Soft ranking bias only.
- **Raster icon `img()` re-reads the file per row build** (ui/view.rs:249):
  `std::fs::read` on every row rebuild; PNG/JPEG bytes are not cached (only SVG
  rasters are, icons.rs:141–150). Perf, not correctness — the startup target
  and typing latency both touch this.
- **SVG with zero width/height** (picker/icons.rs:172): `size_px / 0` → `inf`
  scale into tiny-skia. Only reachable via a crafted `Icon=` SVG in a
  `.desktop` file; may render garbage or panic. Untested.
- **Count-prefix overflow** (picker/state.rs:74): `cur * 10 + d` on usize
  wraps in release for ~19+ digits of count prefix. Absurd input, silent wrap.
- **e2e harness does not isolate XDG state/config** (tests/e2e/support/pikr.rs:39–54,
  sway.rs:52–61): pikr under test reads the developer's real
  `~/.config/pikr/config.toml` and writes the real `history.toml`/`usage.toml`
  (the accept tests push "ban"/"banana" into the real state dir). A developer
  with `case_sensitive = true` in their real config also gets e2e failures
  (the accept tests assume case-insensitive matching).
- **Dead second arm in Visual keymap** (picker/keys.rs:70): `s == "v" && ctrl`
  is unreachable — the arm above matches `"v"` regardless of ctrl. Both produce
  `EnterNormal`, so behaviour is identical today; the arm should be deleted or
  the ctrl check hoisted.
- **`parse_color` (ui/view.rs:56) accepts non-6-digit hex silently** — an
  icky `#abc` or 8-digit `#aarrggbb` in the theme config yields wrong colours
  with no error.

## Coverage

- Reviewed: all 32 `.rs` files — 23 in `apps/pikr/src/` (matcher, frecency,
  history, icons, keys, state, view, css, app, cli, config, main,
  console_attach, all 8 modes), plus `xtask/src/main.rs` and the test harness
  (`smoke.rs`, `keyboard.rs`, `e2e/mod.rs` + support).
- Live-reproduced: finding 1 (deadlock) under `sway --headless` + `wtype` on
  the release binary.
- NOT compiled or run: all `#[cfg(windows)]` code (drun `windows_impl` +
  cache, `drun_icons_windows.rs`, run.rs PATHEXT, clipboard `windows_impl`,
  `console_attach.rs`) and the `#[cfg(not(unix))]` arms of `switch_mode`
  (findings 4 and 5 are reading-only reviews of platform-gated code; verify on
  a Windows/macOS runner).
- Not reviewed (non-code): `deny.toml`, `Cargo.lock`, `CHANGELOG.md`,
  `README.md`, `performance-review.md`, `docs/`, `scripts/`,
  `.github/workflows`.
