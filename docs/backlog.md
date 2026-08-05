# Backlog

## Open findings

### Performance

- **Query-bar blink thread redraws forever.** `ui/view.rs` 530 ms blink effect
  rebuilds the query-bar `dyn_view` (2 `TextLayout` measurements) plus a repaint
  every 530 ms even when idle/focused-out. Gate on focus or on `blink_on`
  toggling only while the view is visible.

## Hardening (correct today, fragile — not defects)

- `payload_key` is not injective _across_ `Payload` variants: `Stdout("2:ab")`
  == `Exec{program:"ab", args:[]}` == `"2:ab"` (also `SetClipboard(x)` ==
  `Stdout(x)`). The comment claiming "injective over all byte strings"
  overstates — it is injective within `Exec` only. Unreachable today because
  each mode emits a single variant; fix the comment, or variant-tag the key if a
  mode ever mixes variants.
- Clipboard accept (`modes/clipboard.rs`) pipes `cliphist decode {id} | wl-copy`
  through `sh -c` and never observes the child's exit — a missing `wl-copy`
  fails silently (user selects an entry, nothing happens). Not an injection (id
  is a parsed `u64`).
- Icon name→path misses are cached forever (`picker/icons.rs` `resolve`) — a
  theme installed while pikr runs never resolves until restart.
- Windows drun icon extraction (`modes/drun_icons_windows.rs`, called from a
  rayon `par_iter` in drun) writes cache PNGs without a lock; same-target
  duplicates or a first-miss storm on `__fallback__.png` can interleave
  truncate/write and yield a corrupt PNG (identical bytes in practice; cosmetic,
  Windows-only, survives until cache wipe).
- `config.rs` `font_size: f32` accepts TOML `nan`/`inf`; effect on layout
  unverified.
- `$TERMINAL` is treated as a single binary name — the common
  `TERMINAL="alacritty --class foo"` idiom silently falls back to a candidate.
- `current_locales` (drun) drops the codeset and `@modifier` — significant for
  e.g. `sr_RS@latin`.

## Accepted trade-offs (not scheduled)

- drun warm-start residual: the mtime cache key walks every file under the XDG
  applications dirs on every launch. Documented in the perf review; deliberate.
  (The second half of the original entry — the rerank frecency loop forming a
  `payload_key` string per entry — was FIXED by `dd8915a` "perf(frecency):
  precompute usage keys per entry"; see the 2026-08-06 perf review.)
- Icon byte caches (`picker/icons.rs` `file_bytes`/`rasterise_svg`) stat per
  lookup on every row rebuild (~10 µs/keystroke for ~10 visible rows). This is
  the documented freshness mechanism behind the mtime+len invalidation; not
  material on its own — revisit only if profiling shows it.

## review review 2026-08-06

Whole-codebase correctness pass (clean tree). Findings verified by the
orchestrator at the cited lines, including an empirical probe of the symlink
semantics.

### Findings

- **Medium — run mode never lists symlinked executables.** `modes/run.rs:122`
  (`if !ft.is_file() { continue; }`) drops every `$PATH` entry whose readdir
  `d_type` is a symlink, because `DirEntry::file_type()` does not follow links.
  NOT a regression from `c19ce0b` (the commit's title claims it "filtered via
  d_type", but the pre-commit `entry.metadata()` did not follow links either —
  verified by probe: `DirEntry::metadata()` on `/usr/bin/sh` returns
  `is_file=false`, mode `lrwxrwxrwx`; only `fs::metadata(path)` follows).
  Impact: 528/3866 `/usr/bin` entries on this host are symlinks — `sh`,
  `python`, `awk` are never listed (their targets `bash`, `python3`, `gawk`
  still are). Fix: when `file_type().is_symlink()`, pay one following
  `fs::metadata(path)` stat; keeps the d_type fast path for the other ~3300
  entries.

  ```
  Repro: scan_path() over a PATH containing /usr/bin
  Expect: entry "sh" present
  Actual: absent (DirEntry::file_type → is_file=false for symlinks)
  ```

- **Low — Windows drun dedupes nothing; apps in both Start Menu roots render
  twice.** `modes/drun.rs:556` (`windows_impl::collect`) walks both `%APPDATA%`
  and `%ProgramData%` Start Menu trees into one `Vec`, sorts (`drun.rs:602`),
  and returns — with no per-id dedupe. The unix sibling dedupes via
  `insert_first` (`drun.rs:106`, first occurrence wins) and ssh via `dedup_by`
  (`modes/ssh.rs:29`). Same app installed for both users ⇒ duplicate rows.
  Windows-only; not executed locally (see Coverage).

  ```
  Repro: same app has a .lnk under both Start Menu roots
  Expect: one row (per-user wins, matching unix insert_first)
  Actual: two identical rows
  ```

### Cleared (sub-agent-traced, not independently re-verified by orchestrator)

- ssh inline-comment hosts — matches ssh's own token semantics.
- visual-anchor staleness, calc row-key pointer reuse, marker parsing, deadlock
  batching, `%20` field codes — traced through, safe.
- All backlog items — none regressed or fixed since documented.

### Hardening

- Symlinked-dir mtime cache blindness; coarse-granularity same-tick cache
  staleness; non-atomic state writes; `--lines 0` / `--width 0`; row-key
  Arc-pointer fragility; non-UTF8 PATH names.

### Coverage

- Whole codebase read: entry/app/cli/config, all modes, picker, ui, xtask,
  tests. Windows/macOS arms read but NOT executed locally — `drun.rs` finding 2
  and the documented PNG-lock race are in that gap.

## tidy review 2026-08-06

Whole-codebase quality pass (clean tree). All items behavior-preserving;
dead-code claims grep-verified by the orchestrator. Note: `main.rs:10` carries a
crate-level `#![allow(dead_code)]` that suppresses compiler warnings — grep, not
rustc, is the evidence.

### Dead code (CSS-migration / refactor orphans)

- **Delete five unused layout constants** — `ui/view.rs:43-46,52`: `HORIZ_PAD`,
  `DESC_GAP`, `PANEL_RADIUS`, `ROW_RADIUS`, `ICON_GAP` — each declared once,
  referenced nowhere. Values moved to `ui/styles/default.css` during the CSS
  migration.
- **Delete the dead `NavAction` machinery** — `ui/view.rs:1270-1273` (enum
  `NavAction { None, Rerank }`), `:1377` (`let after = NavAction::None;`),
  `:1666` (`let _ = after;`). `Rerank` is never constructed, `after` is never
  read — leftover from a refactor of the keydown handler.
- **Drop `status_bar`'s four dead parameters** — `ui/view.rs:562-564`
  (`_selected_bg`, `_font_family`, `_font_size`) and `:567` (`let _ = fg;`).
  Remove the params and the `let _ = fg;`, and drop
  `fg, selected_bg, font_family.clone(), font_size` from the sole call site
  `view.rs:1197-1209`.
- **Remove the dead `_bg` destructure** — `ui/view.rs:888-900`:
  `parse_color(&t.bg)` computed and immediately discarded; drop it from the
  tuple and keep the other six elements.
- **Delete test-only `Usage::bonus`** — `picker/frecency.rs:118-121`: production
  uses `bonus_for_key` (`view.rs:695`); `bonus` is called only from frecency
  tests. Gate `#[cfg(test)]` or delete and rewrite the eight test call sites.
- **Delete test-only `History::is_empty`** — `picker/history.rs:107-109`: sole
  caller is the test at `history.rs:160`; use
  `assert_eq!(h.len(CliMode::Drun), 0)` instead.
- **Remove the stale crate-level `#![allow(dead_code)]`** — `main.rs:10`; its
  comment ("v0.1 scaffold — stubs land before consumers") no longer holds at
  v0.8.9. It hides every orphan above. After deleting the dead items, remove the
  allow so rustc guards future dead code (keep the documented
  `clippy::arc_with_non_send_sync` allow at `main.rs:14`).

### Duplicated logic → call the existing helper

- **`mode_key` is implemented three times** — `picker/frecency.rs:156-158` and
  `picker/history.rs:112-114` (identical `format!("{mode:?}").to_lowercase()`),
  plus the inline `format!("{:?}", s.cli_mode).to_lowercase()` at
  `ui/view.rs:593`. Add one `pub(crate) fn key(self) -> String` on `CliMode` in
  `cli.rs`, delete the two private `mode_key` fns, call from all three.
- **The "exec entry + optional description/icon" pattern is repeated four times
  in drun** — `modes/drun.rs:99-105` (unix collect), `:220-231` (unix
  `From<CachedEntry>`), `:449-460` (windows `From<CachedEntry>`), `:703-718`
  (windows `parse_lnk`): all `Entry::exec(label, program) .with_args(args)` then
  conditional `.with_description(d)` / `.with_icon(i)`. Add one shared
  constructor (e.g. `Entry::exec_with(label, program, args, description, icon)`)
  and call it at all four sites.
- **ssh / clipboard build `Payload::Exec` via raw struct literals** —
  `modes/ssh.rs:154-162` (`make_entry`) and `modes/clipboard.rs:68-78` (unix
  `make_entry`). Use `Entry::exec(...).with_args(...)` + conditional
  `with_description`, as drun does.
- **Redundant re-sort in calc rerank** — `ui/view.rs:789`: `matcher.rank`
  already sorts by `(score desc, index asc)` (`matcher.rs:127`, identical
  comparator); `rerank_calc` adds no frecency bonus, so the second sort is a
  no-op. Delete line 789.
- **Duplicate `Entry` construction in `rerank_calc`** — `ui/view.rs:737-744`
  (live entry) vs `:746-751` (history loop): identical
  `Entry { label: format!("{expr} = {result}"), description: None, icon: None, payload: Stdout(result.clone()) }`.
  Build both from one closure.
- **`hover_bg` computed twice with the same blend** — `ui/view.rs:299`
  (`entry_row`) and `:1061` (`picker_view`): both
  `blend(accent, selected_bg, 0.18)`. Hoist a `fn hover_bg(...)`.
- **Icon lookup-builder chain duplicated in `resolve`** —
  `picker/icons.rs:101-113`:
  `freedesktop_icons::lookup(name).with_size(32) .with_scale(1)` spelled out for
  the primary lookup and again per fallback theme. Bind a closure and call
  `.find()` / `.with_theme(t).find()` on it.
- **ssh executable-probe predicate duplicated** — `modes/ssh.rs:266-268`
  (`is_tool_installed` path branch) and `:281-283` (`probe_in_path`): identical
  `std::fs::metadata(p).map(|m| m.is_file() && is_executable(p, &m)).unwrap_or(false)`.
  Extract one `is_executable_file(path)`.

### css.rs

- **`bg`/`fg` parsed then discarded** — `ui/css.rs:26-27`
  (`let bg/fg = parse_hex(...)`) with throwaway `let _ = fg; let _ = bg;` at
  `:49-50` ("reserved for future class entries" — YAGNI, nothing registers; the
  template replaces use the raw `theme.bg`/`theme.fg` strings). Delete lines
  26-27 and 49-50; `parse_hex` stays (accent/muted/selected_bg).
- **`ex_bg` and `status_bg` are the same value** — `ui/css.rs:31-32`: both
  `blend_hex(muted, selected_bg, 0.15)` fed to two distinct placeholders.
  Compute once and reuse.

### Needless allocations

- **`view.rs:276` clones the description only to format it** —
  `let body: String = d.clone();` feeds `format!("({body})")` at `:281` and
  nothing else. `format!("({d})")` directly, drop the clone.
- **`frecency.rs:146` allocates a temp String per arg in `payload_key`** —
  `key.push_str(&format!("\u{1f}{}:{arg}", arg.len()))`.
  `write!(key, "\u{1f}{}:{arg}", arg.len())` writes straight into the target.
  (The per-entry key rebuild at rerank time is the documented perf trade-off;
  this per-arg temp is not.)

### Minor YAGNI

- **Unused `Default` impls** — `picker/state.rs:6-8` (`VimMode` derives
  `Default` with `#[default] Normal`) and `:91-95`
  (`impl Default for PickerState`): no call sites of either (`app.rs` uses
  `PickerState::new()`; cli uses `default_value_t = VimMode::Insert`). Drop
  both.

### Out of scope (not tidy)

- `run.rs:122` symlink executables and `drun.rs:556` Windows dedupe —
  correctness items already documented by the `review` pass above.
- The `state.lock().unwrap().icons.lock().unwrap()` repetitions in
  `view.rs:221-252` and the drun unix/windows `tree_mtime` twins —
  platform-gated or cosmetic; left out.

## audit review 2026-08-06

Whole-codebase security audit (clean tree). Verdict: no critical/high/medium;
four low findings. Findings 1-2 and 4 re-verified by the orchestrator (finding 1
empirically: evalexpr probe crashes the 8 MB stack at ~100k chars of nested
parens, survives at 80k); finding 3 is Windows-only, code-traced, not
runtime-verified.

### Findings

- **Low — `modes/calc.rs:31`: deeply nested calc expressions abort the process
  via stack overflow in evalexpr's recursive evaluator.** `evalexpr::eval`
  recurses one frame per tree level (`evalexpr-11.3.1/src/tree/mod.rs:334-341`)
  and its `Drop` recurses too; no depth limit exists anywhere in the crate.
  Empirically verified: `"("*N + "1" + ")"*N` overflows the 8 MB main-thread
  stack between 80k and 100k chars (N≈50k). Reached from every keystroke in calc
  mode — live query `ui/view.rs:716`, up to 100 stored history entries
  `ui/view.rs:731` — and from startup via
  `pikr --show calc --filter "$untrusted"` (`app.rs:107-110` pre-fills the
  query). Amplifier: the crashing expression, once accepted, is stored in calc
  history and re-evaluated on every keystroke of every subsequent calc session.
  Fix: cap expression length/depth in `calc::eval` before calling evalexpr.

  ```
  Repro: pikr --show calc --filter "$(python3 -c 'print("("*50000+"1"+")"*50000)')"
  Expect: process stays alive, expression rejected
  Actual: SIGSEGV / "fatal runtime error: stack overflow, aborting"
  ```

- **Low — `modes/drun.rs:167-182`: `strip_field_codes` drops _any_ `%X` pair,
  not just the freedesktop field codes, mangling `Exec=` arguments that
  legitimately contain `%`.** Trace: `.desktop` `Exec=` → `parse_exec`
  (`drun.rs:150-163`) → `shlex::split` → per-token `strip_field_codes`: `%`
  followed by any single char removes both; a trailing `%` is removed too (`50%`
  → `50`). `Exec=notify-send "50%"` launches with arg `50`;
  `Exec=yt-dlp -o "%(title)s.%(ext)s"` mangles the template. The spec reserves
  `%f/%F/%u/%U/%i/%c/%k` (and `%%` = literal); unknown codes are not defined to
  be deleted. Same trust domain (the `.desktop` already grants execution), but
  it silently changes what installed apps receive. Fix: only strip the spec's
  actual codes.

  ```
  Repro: Exec=notify-send "50%" → parse_exec returns args ["50"]
  Expect: args ["50%"]
  Actual: "50" (percent dropped)
  ```

- **Low — Windows drun executes `.lnk` targets from the all-users Start Menu,
  which standard users may be able to write to; any local user can plant a
  shortcut pikr runs as the pikr user.** `start_menu_roots()` includes
  `%ProgramData%\Microsoft\Windows\Start Menu\Programs` (`drun.rs:629-637`);
  `parse_lnk` filters only on target extension and `target_path.exists()`
  (`drun.rs:670-682`); target + args become `Payload::Exec` spawned on Accept
  (`modes/mod.rs:137`). Whether standard users can write that folder is a
  Windows ACL question — NOT verified on this Linux host. Inherent to the
  launcher trust model (rofi/wofi drun behave the same); worth a documented note
  or a same-user-only filter on the all-users root.

- **Low — `picker/history.rs`, `picker/frecency.rs`: persisted query history and
  frecency payload keys are world-readable (0644), leaking typed queries to
  other local users.** `History::save` / `Usage::save` write via
  `std::fs::write` (0666 & umask → 0644) into a `create_dir_all`-made 0755
  `~/.local/state/pikr/`. Content includes per-mode query strings (dmenu
  selections, ssh hostnames, command fragments, calc expressions) and
  `payload_key` strings (program+args, hosts). `-P` skips persistence by design;
  the non-`-P` default leaves a world-readable transcript. Fix: write the files
  0600 (temp file + rename, or `OpenOptions` + `set_permissions`).

### Cleared

- **Command injection in `.desktop` Exec / run-mode / ssh payloads** — every
  execution path is `Command::new(program).args(args)` (`modes/mod.rs:137-143`),
  argv, no shell; the only `sh -c` is the cliphist pipe
  (`modes/clipboard.rs:75`) where the interpolated value is a `u64`.
- **ssh terminal quoting** — pwsh arm single-quotes with `''` doubling; cmd `/K`
  arm gated on `is_safe_host` charset `[A-Za-z0-9._\-:[\]]` with argv fallback;
  wt/unix arms are pure argv. Regression tests cover `foo; notepad` and metachar
  fallback.
- **SVG external-resource exfiltration** — usvg `Options::default()` has
  `resources_dir: None` and an empty fontdb; the default resolver reads only
  image-format files and the result is rendered locally, never emitted.
- **Calc integer overflow / div-by-zero panics** — evalexpr uses checked
  arithmetic returning errors; `Exp` promotes to `powf` with a `is_finite()`
  guard at `calc.rs:43`.
- **Count-prefix / selection overflow** — `push_count_digit` saturates
  (`state.rs:86`), `move_down_selection` saturates, frecency clamps to
  `u16::MAX`.
- **Zero-size SVG scale division** — guarded at `icons.rs:253-255` with a
  regression test.
- **drun cache poisoning** — cache lives in the user's own state dir; stale
  mtime serves stale _benign_ entries; writing the cache requires the same user
  who owns `.desktop` execution.
- **Mutex deadlocks in the UI** — all `state.lock()` sites are on the single
  main thread; the blink thread touches no shared state; `query_sig.set` under
  the lock is batched with a regression test; history-recall drops the guard
  before `query_sig.set` (`view.rs:1626-1631`).
- **CSS injection via config values** — values land in a fixed template parsed
  by hjkl-css with a whitelisted property set; user's own file anyway.

### Hardening (correct today, fragile)

- Icon `Icon=` misses cached forever (`icons.rs:86-88`) — backlog-documented.
- `freedesktop-icons` joins the icon name into the theme path; a name containing
  `..` can escape the theme dir, and usvg's default resolver reads an
  absolute-path `<image href>` from a crafted SVG. Display-only, same trust
  domain; reject names containing `/` or `..` at `icons.rs:90` if the trust
  model changes.
- drun cache-hit skips re-validation — a same-user attacker with a predictable
  cache key gets a second execution path.
- run-mode label ↔ executed-binary mismatch — `scan_path` dedupes by name across
  PATH dirs but spawn re-resolves the bare name via PATH at Accept, so the
  listed entry can be a different binary than launched.
- `Exec=` relative program (`Exec=./foo`) resolves against pikr's CWD.
- dmenu stdin is unbounded (`dmenu.rs:22-28`) — a hostile caller piping
  gigabytes bloats the process.
- calc history re-evaluation — up to 100 stored expressions re-evaluated per
  keystroke (`view.rs:719-734`); amplifies finding 1.

### Coverage

- Entry points walked: CLI args, config TOML, XDG config/state files, dmenu
  stdin, `.desktop` parsing + Exec + Icon, ssh config + `$TERMINAL`, subprocess
  spawns, clipboard, calc eval, history/frecency persistence, icon loading,
  floem UI event loop, blink thread, ex-mode commands, `--message` modal.
  Classes walked: injection, memory/resource, crypto, AuthZ/AuthN, data
  integrity/TOCTOU, error handling, concurrency.
- NOT audited in depth: the ~916 lines of test code (`tests/e2e/*`, smoke,
  keyboard, startup_gate) — dev harnesses, no production paths. Windows-only
  code (`drun_icons_windows.rs`, `console_attach.rs`, drun `windows_impl`)
  cannot be compiled or executed on this Linux host — finding 3 is code-traced
  only.
- NOTE: the audit agent reported `docs/performance-review.md` missing; it exists
  at the repo ROOT as `performance-review.md`.
- Backlog items re-checked: none fixed, none regressed — all documented items
  still present as documented.

## perf review 2026-08-06

Whole-codebase performance pass (clean tree). Bottom line: no O(n²)+ blowups,
nothing new on the startup path; the four findings are per-keystroke
recomputation of session-constant work (tens-to-hundreds of µs/keystroke —
figures are estimates from operation counts, not a profiler). Findings 1-4
re-verified by the orchestrator at the cited lines.

### Findings

- **`picker/matcher.rs:135-144` — every non-ASCII label is re-decomposed
  (grapheme split + NFC) per entry, per keystroke, though labels never change.**
  `match_field` runs inside the per-entry loop (`matcher.rs:101-102`, from
  `view.rs:684` rerank on every keystroke). In emoji mode all ~1800 labels are
  non-ASCII (glyph prefix, `modes/emoji.rs:21`), so every keystroke re-runs
  unicode-segmentation + NFC over all of them. The per-entry allocation was
  already removed (reused `text_buf`), but the processing is repeated. Fix:
  precompute each entry's grapheme-normalized form once at collect/switch time
  and have `match_field` consume it (keep `text_buf` for descriptions). ~2 kB
  memory for 1800 labels. ~100–400 µs/keystroke in emoji mode (measure
  before/after with the existing `RUST_LOG=pikr=debug` spans).
- **`ui/view.rs:730-733` — calc mode re-parses and re-evaluates every history
  expression on every keystroke, though results are constant.** `rerank_calc`
  runs `calc::eval` for each of up to `HISTORY_CAP`=100 stored expressions per
  keystroke, plus rebuilds `usage_keys` via `entry_keys` at `view.rs:754` (a
  `payload_key` String per entry per keystroke — the one place the dd8915a
  precompute fix does not apply). History strings never change during the
  session. Fix: precompute an `expr → result` map at history load / mode switch;
  only the live query's eval (`view.rs:716`) runs per keystroke. ~100–300
  µs/keystroke with a full history.
- **`ui/view.rs:1667` + `ui/view.rs:1049` — the whole match list is rebuilt into
  an `imbl::Vector` twice per character and once per navigation keystroke, even
  when nothing matched changed.** The virtual_stack data closure
  (`view.rs:1080-1100`) re-reads `rev` and rebuilds all up-to-256 items (Arc
  clone + two Rc clones + imbl insert each) on every `rev` bump. `rev` is bumped
  unconditionally at the end of every handled keydown (`view.rs:1667`, including
  selection-only `MoveDown`/`MoveUp`/`PageDown`) and a second time per character
  inside the rerank effect batch (`view.rs:1049`). Selection moves don't change
  matches — row styles already track selection reactively via `selected_sig` —
  so the rebuild is pure waste. Fix: memoize the `imbl::Vector` keyed by a
  matches-version signal (imbl clone is O(1)), and split `rev` into "matches
  changed" vs "selection changed". ~50–100 µs/keystroke across all modes.
- **`ui/css.rs:61-85` — every style evaluation re-scans and re-sorts the whole
  stylesheet; per row, per keystroke.** `apply()` iterates all `sheet.rules` ×
  selectors, collects matches, and `sort_by_key`s on every invocation — from
  each row's icon/desc/row style closures (`view.rs:204, 291, 333`), query bar
  (`view.rs:1027`), input row (`view.rs:1067`), ex/status bars, scroll handle,
  whenever floem re-evaluates those styles (row rebuild on keystroke plus
  selection change). The matched-rule set depends only on the compile-time
  constant `(element, classes)` pair. Fix: memoize the sorted matched-rule list
  per `(element, classes)` at stylesheet build time (the sheet is immutable
  after `build_stylesheet`); `apply` walks the cached list. Honest caveat: 21
  single-class rules today ⇒ ~20–40 µs/keystroke — low, but the most-repeated
  per-keystroke computation in pikr's own code, and it scales linearly as the
  sheet grows.

### Documented items — status (not re-reported)

- Blink thread (`view.rs:946-953`), drun mtime walk (`drun.rs:272-301`), icon
  stat-per-lookup (`icons.rs:165/199`) — still present, unchanged.
- **FIXED:** "the rerank frecency loop still forms a `payload_key` string per
  entry" — commit `dd8915a` ("perf(frecency): precompute usage keys per entry")
  lands `usage_keys` (`view.rs:834`, `app.rs:116`); the rerank loop now uses
  `self.usage_keys[m.index]` (`view.rs:695`). The matching "Accepted trade-offs"
  backlog bullet has been updated.
- Confirmed still-present tidy items with perf flavor (not re-reported):
  `view.rs:789` redundant re-sort; `view.rs:276` `d.clone()`; `frecency.rs:146`
  temp String in `payload_key`.

### Coverage

- Traced in full: entry path, all modes, all picker modules, all ui modules,
  xtask, and the resolved floem fork (`mxaddict/floem@8c52a32`) for `rich_text`,
  `virtual_stack`, and style-evaluation semantics (rich_text compute runs once
  per row build, not per frame).
- NOT executed: `drun_icons_windows.rs` (Windows-only, startup-only). Exact µs
  figures are estimates from operation counts — the codebase already has startup
  `phase_us` tracing (`app.rs:94-99`); a per-keystroke span around `rerank`
  would confirm before/after. Startup budget: nothing new beyond the documented
  drun mtime walk; run-mode `scan_path`, emoji collect, and the `cliphist list`
  spawn are each once-per-launch; dominant startup cost (floem/wgpu init) is
  outside pikr's source.
- NOTE: the perf agent also reported `performance-review.md` missing — it exists
  at the repo ROOT, not `docs/`.
