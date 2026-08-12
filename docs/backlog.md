# Backlog

## Open findings

### Performance

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
  before/after with the existing `RUST_LOG=pikr=debug` spans). _Deferred
  2026-08-06: the win is emoji-mode-specific and was not profiled this session;
  the review itself recommends measuring before optimizing._

- **`ui/view.rs` — the whole match list is rebuilt into an `imbl::Vector` twice
  per character and once per navigation keystroke, even when nothing matched
  changed.** The virtual*stack data closure (`view.rs:1080-1100`) re-reads `rev`
  and rebuilds all up-to-256 items (Arc clone + two Rc clones + imbl insert
  each) on every `rev` bump. `rev` is bumped unconditionally at the end of every
  handled keydown (including selection-only `MoveDown`/`MoveUp`/ `PageDown`) and
  a second time per character inside the rerank effect batch. Selection moves
  don't change matches — row styles already track selection reactively via
  `selected_sig` — so the rebuild is pure waste. Fix: memoize the `imbl::Vector`
  keyed by a matches-version signal (imbl clone is O(1)), and split `rev` into
  "matches changed" vs "selection changed". ~50–100 µs/keystroke across all
  modes. \_Deferred 2026-08-06: touches the floem reactivity core (rev drives
  the virtual_stack data closure, scroll `ensure_visible`, and the empty-state
  visibility) where the no-results-hang regression lives; judged not worth the
  regression risk without a profiler confirming the win.*

## Hardening (correct today, fragile — not defects)

- Windows drun icon extraction (`modes/drun_icons_windows.rs`, called from a
  rayon `par_iter` in drun) writes cache PNGs without a lock; same-target
  duplicates or a first-miss storm on `__fallback__.png` can interleave
  truncate/write and yield a corrupt PNG (identical bytes in practice; cosmetic,
  Windows-only, survives until cache wipe).
- Symlinked-dir mtime cache blindness; coarse-granularity same-tick cache
  staleness; non-atomic state writes; `--lines 0` / `--width 0`; row-key
  Arc-pointer fragility; non-UTF8 PATH names. (review pass 2026-08-06)
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
semantics. Both findings FIXED 2026-08-06 (run symlinks `aebd016`; Windows
dedupe `9cc7dfb`).

### Cleared (sub-agent-traced, not independently re-verified by orchestrator)

- ssh inline-comment hosts — matches ssh's own token semantics.
- visual-anchor staleness, calc row-key pointer reuse, marker parsing, deadlock
  batching, `%20` field codes — traced through, safe.

### Coverage

- Whole codebase read: entry/app/cli/config, all modes, picker, ui, xtask,
  tests. Windows/macOS arms read but NOT executed locally — the documented
  PNG-lock race (Hardening) is in that gap.

## audit review 2026-08-06

Whole-codebase security audit (clean tree). Verdict: no critical/high/medium;
four low findings. Findings 1-2 and 4 re-verified by the orchestrator (finding 1
empirically: evalexpr probe crashes the 8 MB stack at ~100k chars of nested
parens, survives at 80k); finding 3 is Windows-only, code-traced, not
runtime-verified. Findings 1, 2, 4 FIXED 2026-08-06 (calc cap `b65c2d9`, field
codes `c7d1c14`, 0600 state `081faf4`/`acc5e0c`); finding 3 addressed via the
documented trust note in `start_menu_roots` (`9cc7dfb`) — a same-user-only
filter on the all-users root remains open if the threat model tightens.

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
  before `query_sig.set`.
- **CSS injection via config values** — values land in a fixed template parsed
  by hjkl-css with a whitelisted property set; user's own file anyway.

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
  cannot be compiled or executed on this Linux host — the Start Menu ACL finding
  is code-traced only.
- NOTE: the audit agent reported `docs/performance-review.md` missing; it exists
  at the repo ROOT as `performance-review.md`.

## review review 2026-08-06 (evening sweep)

Whole-codebase correctness pass (clean tree), evening re-run. One low finding;
verified by the orchestrator at the cited lines (including the installed
xdg-3.0.0 crate source). No code changed.

### Findings

**LOW — `modes/ssh.rs:128-133` — `Match` blocks' settings are attributed to the
preceding `Host` block.** `parse_config` treats only the `host` keyword as a
block boundary; a `Match` line falls into the `_ => {}` arm with no flush, so
the `HostName` / `User` lines under it accumulate into the still-open previous
block and bake into its entries' descriptions. Real ssh semantics: a `Match`
block is an independent conditional context.

```
Repro: config = "Host github\n  HostName github.com\nMatch host prod\n  User deploy\nHost prod\n  HostName prod.example\n"
Expect: github's description == "github.com"; prod's == "prod.example (deploy)"
Actual: github's description == "github.com (deploy)"   (flush at "Host prod"
        emits github with user=Some("deploy") from the Match block)
```

Impact is display-only: the executed argv is `terminal -e ssh <host>` built from
the entry label (`make_entry`, ssh.rs:181-183), parsed independently of
HostName/User — they never reach the payload. Fix: flush (or reset
hostname/user) on a `Match` keyword, or document Match blocks as unsupported.

### Cleared (suspected, traced, disproved — verified by the orchestrator unless marked)

- **`xdg::place_state_file` truncating the drun cache on probe** — verified
  against xdg-3.0.0 source: `write_file` (base_directories.rs:739-745) only
  `create_dir_all`s the parent, never opens the file.
- **Matcher non-ASCII position mapping** — `matcher.rs:135-154` NFCs each
  grapheme into one `text_buf` slot; `grapheme_positions_to_codepoints`
  (158-167) walks the same grapheme stream expanding matched indices to full
  codepoint ranges; tests pin the shapes (258-269).
- **Mutex/batch deadlock discipline** — rerank (`view.rs:1048-1054`) and
  `switch_mode` (1329-1337) create the guard inside `Effect::batch`; history
  recall drops the guard before `query_sig.set` (1632, 1659); the
  `state.rs:112-149` regression test deadlocks if the batch is ever removed.
- **`-P` persistence gates** — all four accept paths (keyboard `view.rs:1469`,
  dmenu fallthrough 1498, AcceptCustom 1526, click 349) gate
  `usage.record`/`history.push` on `!s.password`.
- **calc live/history index math** — `view.rs:753-789`: `live_offset` is
  consistent between `self.entries` construction and both match paths; `m.index`
  always stays `< entries.len()`; truncation only touches `matches`.
- **`g_pending` lifecycle** — `view.rs:1359-1376`: only set in Normal
  (1370-1373); any other key clears it (1374-1376) before `StartEx`; a mode
  switch via `:mode` can't carry a pending `g`.
- **Accept with no matches (non-dmenu)** — `view.rs:1508-1516` executes nothing
  and leaves the picker open (rofi parity); dmenu gets its own fallthrough
  (1491-1507) returning the typed query.
- **frecency cross-variant key collision** — `frecency.rs:136-160`: documented
  non-injective across variants (146-149) but each mode emits a single variant
  and tables are keyed per mode.
- **drun first-wins dedupe** — `insert_first` (`drun.rs:113-115`) dedupes during
  iteration before the label sort (100); user-local wins.
- **`pick_locale` fallback chain** — `drun.rs:129-161`: `C`/`POSIX`/empty fall
  through; modifier + territory-only steps emitted per spec; tests pin shapes.
- **ssh user/system dedupe** — `ssh.rs:52-55`: user config parsed first
  (`config_paths`), stable sort + `dedup_by` keeps the user copy.
- **`--filter` cursor placement** — `app.rs:107-110`: `query_cursor` set to char
  count, matching the char-index caret invariant.
- **dmenu CRLF** — `BufRead::lines()` strips `\r\n` per std docs.
  (sub-agent-traced, std behavior)
- **row_key Arc-pointer staleness (calc)** — the live previous `imbl::Vector`
  keeps old entry Arcs alive across the `rev` bump on the single UI thread, so
  allocator reuse of a dropped pointer can't occur while it is referenced.
  (sub-agent-traced)
- **Windows drun dedupe-before-sort** (`drun.rs:621-623`) — parallels the unix
  path. (sub-agent-traced; Windows-only, not executed)

### Hardening (correct today, fragile — not defects)

- **ssh `Match` keyword** — the finding above; if `Match` support is in scope,
  flushing block state on the keyword is the fix. Currently treated as a
  comment-level no-op that silently inherits the previous Host's block state.
- **`Key::Character` multi-codepoint strings** — `keys.rs:101/190` take
  `s.chars().next()?` and drop the rest. xkb delivers one scalar per event
  today, but an IME composing a whole grapheme (or word) into one
  `Key::Character` would silently insert only its first char.

### Coverage

- Read in full: `apps/pikr/src/` (app, cli, config, console_attach, main; all of
  modes/, picker/, ui/), `xtask/src`, `tests/` (keyboard, smoke, startup_gate,
  e2e incl. harness).
- Windows-only, read but NOT compiled/executed (Linux host):
  `drun_icons_windows.rs`, `console_attach.rs`, and the `#[cfg(windows)]` arms
  of drun/ssh/clipboard/run. The agent flagged `hicon_to_png`'s unchecked
  `stride = width*4` u32 multiply (`drun_icons_windows.rs:185`) as untested in
  that gap; icon sizes are tiny in practice — not reported as a finding.
- Build/test gate not run (report-only pass); verification is line-by-line
  tracing. The one dependency-behavior claim (`xdg::place_state_file`) was
  verified against the installed crate source.

## audit review 2026-08-06 (evening sweep)

Whole-codebase security audit (clean tree, evening re-run). Verdict: no
critical/high/medium/low findings; overall risk low. The drun cache 0644 finding
(the only divergence from the state-file 0600 policy) was FIXED 2026-08-12: both
`write_cache` arms now route through `picker::write_private_state`, with
regression test `cache_written_0600_like_state_files`. The remaining hardening
item was verified by the orchestrator at the cited lines (the perms one
empirically on this host).

### Hardening (correct today, fragile — not defects)

- **`drun_icons_windows.rs:185-186` — `stride = width * 4` u32 multiply, then
  `vec![0u8; (stride * height) as usize]`.** A wrap under-allocates the
  `GetDIBits` buffer (unsafe overflow); a non-wrap huge value OOM-aborts. GDI
  realistically bounds icon dimensions to tiny sizes; code-traced only on this
  Linux host. If Windows CI lands: `checked_mul` + a dimension cap. (Also
  flagged by the evening review pass — cross-cutting.)

### Cleared (suspected, traced, disproved — verified by the orchestrator unless marked)

- **`sh -c` in the clipboard pipe** (`clipboard.rs:60-74`) — the only
  interpolated value is `{id}`, parsed as `u64` from `cliphist list`; digits
  only, no metacharacters can reach the string.
- **`.desktop` Exec / run-mode / ssh payload injection** — every execution path
  is `Command::new(program).args(args)` with stdio nulled
  (`modes/mod.rs:183-192`); no shell on any accept path.
- **ssh pwsh/cmd arms** — pwsh single-quotes with `''` doubling; cmd `/K` gated
  on the `is_safe_host` charset (excludes `=`, so `-oProxyCommand=…` cannot
  form); bare option flags error out with no host. Tests pin the metachar cases.
- **`$TERMINAL` env** — shlex-split into argv; a hostile value fails
  `is_tool_installed` and falls through to candidates. Probe-then-spawn TOCTOU
  is same-user.
- **calc eval depth** — `MAX_EXPR_LEN` 4096 (`calc.rs:22`) rejects before
  evalexpr; prior audit empirically crashed the 8 MB stack only at ~100k chars.
- **Ex-mode command injection** — `:command` matches a fixed whitelist
  (`view.rs:1302-1315`); `q`/`q!` quit, mode names switch modes, anything else
  is a no-op.
- **`-P` persistence gates** — all four accept paths gate
  `usage.record`/`history.push` on `!s.password`; no log line emits query or
  password text (only lengths/counts).
- **SVG external-resource exfiltration** — usvg default resolver reads only
  image-format files, rendered locally, never emitted. (sub-agent-traced vs
  installed usvg-0.48.1 source)
- **`.desktop` walker symlink loops** — `freedesktop-desktop-entry-0.8.1`
  canonicalizes and keeps a `visited` set, with a regression test.
  (sub-agent-traced)
- **State-file permissions** — verified on this host: all three state files 0644
  but `~/.local/state` is 0700; history/usage are re-chmod'd to 0600 on next
  save.
- **`matches`/`entries` index divergence** — rebuilt together under the single
  `AppState` mutex on the single UI thread; no interleaving window.
- **Icon absolute-path handling / matcher grapheme `expect` / `pick_locale`
  lookup strings** — non-executed, non-panicking paths.

### Coverage

- Entry points walked: CLI args, config TOML, XDG config/state files, dmenu
  stdin, `.desktop` parsing (Exec/field-codes/Icon/locale), ssh config +
  `$TERMINAL`, subprocess spawns, calc eval, icon loading (SVG + PNG/JPEG),
  floem event loop + key events, ex-mode commands, `--message` modal,
  `--password`, Windows `.lnk` parse + icon extraction. Classes walked:
  injection, memory/resource, crypto, AuthZ/AuthN, data integrity, error
  handling, concurrency.
- NOT audited: the `tests/` tree (~916 lines of dev harnesses, no production
  paths). Windows-only code (`drun_icons_windows.rs`, `console_attach.rs`,
  `#[cfg(windows)]` arms) read but NOT compiled/executed on this Linux host —
  the `hicon_to_png` stride analysis is code-traced only.
- Summary: 0 findings (0/0/0/0). Overall risk low — every execution path is
  argv-based, the cross-domain inputs (clipboard, dmenu stdin) are handled
  safely, and the tracked items remain the only knowns. Fix first (1) — drun
  cache through `write_private_state` — DONE 2026-08-12. Remaining: (2)
  `checked_mul` + dimension cap in `hicon_to_png` if Windows CI ever lands, (3)
  tracked dmenu stdin cap / cache re-validation if the threat model tightens.

## tidy review 2026-08-06

Whole-codebase cleanup pass (clean tree). Eight candidates, all verified
behavior-identical by the orchestrator (grep for the dead-code claims, direct
reads for the rest). No code changed. Ranked by value:

### Findings

1. **Duplicated accept-custom flow — dmenu fallthrough ≡ `AcceptCustom`
   (`ui/view.rs:1491-1507` ≡ `1518-1535`).** Both blocks do the same 14-line
   sequence — read `query_sig` untracked, build `Payload::Stdout(trim)`, lock
   state, push+save history gated on `!password && !q.is_empty()`,
   `modes::execute`, `eprintln!` on error, `exit(0)` — differing only in where
   `cli_mode` comes from. Two copies of a flow that includes the `-P`
   persistence gate must stay in lockstep. Extract one helper
   (`accept_custom(query, cli_mode)`), call it from both arms; the triplicated
   `execute` + `eprintln!` 2-liner (`1503-1506`, `1509-1513`, `1531-1534`) folds
   in (the `Accept` loop calls it per payload without the exit).

2. **Dead `impl Default for Matcher` + test-only `Matcher::new()`
   (`picker/matcher.rs:36-40`, `43-45`).** Grep across src + tests: no
   `Matcher::default()`/`Default::default()` resolves to it; `new()` is called
   only from `matcher.rs`'s own tests (production uses `with_case_sensitive` at
   `app.rs:112`). Delete the `Default` impl and gate `new()` with `#[cfg(test)]`
   — otherwise the non-test build warns dead_code once `Default` (its only
   non-test caller) is gone. Binary crate, no external API surface.

3. **Dead `impl Default for IconCache` (`picker/icons.rs:299-303`).** Same
   shape: zero callers (production and tests use `IconCache::new()`,
   `app.rs:119`). Delete the impl; `new()` stays.

4. **Duplicated history-recall block (`ui/view.rs:1630-1639` ≡ `1657-1664`).**
   `HistoryPrev` and `HistoryNext` repeat the identical 6-line recall
   (`entry.to_string()` → `drop(s)` → `history_cursor.set` → char-count →
   `query_sig.set` → `query_cursor_sig.set`). Hoist into a local closure
   capturing the three Copy signals; the `drop(s)`-before-`query_sig.set`
   deadlock discipline is preserved (closure runs after the drop).

5. **Dead statement `let _ = cli_mode;` (`ui/view.rs:1508`).** `cli_mode` is
   already used at `1491` in the same scope; the statement is a no-op leftover.
   Delete the line.

6. **Redundant duplicate startup log (`app.rs:100`).** Line 100
   (`count = entries.len(), "entries collected"`) immediately re-logs the same
   count with less info than the block at `94-99`
   (`"startup entries collected"`, with `phase_us`/`elapsed_us`). Delete line
   100 — flagged since a debug log line is technically observable output.

7. **Duplicated icon-cache-dir resolution (`modes/drun_icons_windows.rs:47-54`
   vs `61-64`).** `icon_cache_path` and `fallback_icon_path` both repeat
   `dirs::data_local_dir()?.join("pikr").join("icon-cache")` + `create_dir_all`.
   Extract `fn cache_dir() -> Option<PathBuf>`. (Windows-only file — read, not
   compiled here.)

8. **Meaning-free alias `let status_bg = ex_bg;` (`ui/css.rs:47`).** Used once
   (`57`), identical value to `ex_bg` (`46`). The
   `"ex bar and status bar share the same derived background"` comment (`45`) is
   the only content — keep the comment, replace the use with `hex(ex_bg)`, drop
   the binding. Lowest value; the alias does carry the naming intent.

### Coverage

- Walked in full: every line of `apps/pikr/src/` (main, app, cli, config,
  console_attach, ui/{css,mod,view}, picker/{frecency,history,icons,keys,
  matcher,mod,state}, modes/{calc,clipboard,dmenu,drun,drun_icons_windows,
  emoji,mod,run,ssh}) and `xtask/src/main.rs`.
- Dead-code claims verified by grep against `apps/pikr/src` AND
  `apps/pikr/tests/` plus cfg-gated arms. Test-only helpers with live test
  callers (`Usage::bonus`, `IconCache::*_len`, `rerank_if_query_changed`,
  `resolve_at`, `pick_locale`'s closure param) were checked and kept.
- Not walked: `tests/` for tidiness (out of scope; grepped only to clear
  dead-code claims); no build/test run (report-only). Windows/macOS arms read,
  not compiled.
- Not re-reported (already tracked): matcher grapheme re-decomposition,
  virtual_stack rebuild-on-rev, drun cache 0644, icons stat-per-lookup, drun
  mtime walk.

## perf review 2026-08-06

Whole-codebase performance pass (clean tree), hot paths: startup (under the 500
ms usable-picker target) and per-keystroke rerank. Two new findings, both on the
per-keystroke rerank path — verified by the orchestrator at the cited lines; the
fix-soundness claim for finding 1 checked against the installed
nucleo-matcher-0.3.1 source. Estimates are allocation-count arithmetic, not
profiles. No code changed.

### Findings

1. **`picker/matcher.rs:106-123` — every surviving match allocates 2-4 heap
   objects (`Rc::new(lp.clone())` / `Rc::new(dp.clone())`) per keystroke; a
   broad emoji query allocates for ~1500+ entries.** `rank` fills a position
   vector for every survivor before the caller sorts and truncates to
   `max_results` (256, `config.rs:30`), yet positions are only read for the
   truncated top matches (`view.rs:1090-1103` row build + `row_key` /
   `highlighted_label`). Emoji mode N≈1800 with a short broad query → ~3000-5000
   small malloc+free per keystroke (rerank runs on every query-changing key,
   `view.rs:1037-1055`). Est. ~100-200 µs/keystroke worst case — same order as
   the tracked grapheme item. Fix: two-phase rank — pass 1 `Atom::score` over
   all N (score-only), apply the frecency bonus + sort + truncate in `rerank`,
   then pass 2 `Atom::indices` only for the ≤256 survivors. Verified
   behavior-identical: `score` and `indices` are the same
   `fuzzy_matcher_impl<const INDICES>` instantiation (nucleo-matcher-0.3.1
   `lib.rs:191`/`209`/`212`, `pattern.rs:302`/`331`), so scores match exactly;
   `indices` merely also backtracks positions. Trades per-survivor Vec+Rc churn
   for one `(index, score)` Vec of N. Empty-query path (`matcher.rs:68-80`)
   already shares one `empty` Rc — untouched.

2. **`ui/view.rs:677-681` — `pairs: Vec<(&str, Option<&str>)>` rebuilt from
   scratch every keystroke.** `rerank` materializes an N×16-byte tuple Vec (plus
   ~11 growth reallocs from `collect()` with no capacity hint) solely to hand
   `matcher.rank` a slice it immediately iterates — ~29 KB churn per keystroke
   in emoji mode. Same pattern in `rerank_calc` (`view.rs:779-784`, small N).
   Fix: change `rank`'s parameter to iterate the entries directly
   (`&[Arc<Entry>]` or an iterator yielding `(&str, Option<&str>)`), keeping
   tuples as transient stack values. Modest, zero-risk, removes an allocation
   from every keystroke in every mode.

### Minor, deliberately not ranked (each <10 µs/keystroke or cold)

- `ui/css.rs:79-82` — `apply()` builds a fresh `(String, Vec<String>)` memo key
  (2-3 allocs) per call; runs per row-style closure per rebuild (~30-100
  calls/keystroke when matched rows rebuild). Fixable by interning keys once per
  sheet; noise relative to findings 1-2 and the tracked items.
- `ui/view.rs:751` — calc mode rebuilds `usage_keys` (`frecency::entry_keys`,
  one String alloc per entry) every keystroke; N ≤ 101 (history cap), calc-only.
- Double sort per keystroke (`matcher.rs:127` + `view.rs:696`) — ~42k comparison
  ops for 1800 entries, ~µs. Not worth merging the frecency bonus into the
  matcher.
- `css.rs`/`state.rs`/`keys.rs` — no O(n²) or wrong-container patterns found.

### Coverage

- Traced in full: startup (config → per-mode `collect` → `entry_keys` →
  usage/history load → calc precompute → initial rerank → first paint/focus via
  the `view.rs:1037` effect + `startup_gate.rs` markers); per-keystroke
  (`KeyDown` → `InsertChar` → rerank effect → `Matcher::rank` → frecency loop →
  truncate → `rev` bump → virtual_stack data closure → row rebuild → icon
  resolve/`file_bytes` stat → `highlighted_label`/styles); per-navigation
  (MoveDown/Up/PageDown — no rerank, only the tracked rev rebuild); calc rerank;
  `switch_mode`; mode collects (drun walk + cache, run PATH scan with `d_type`
  fast path, ssh parse, clipboard spawn, dmenu stdin).
- Verified against installed sources: nucleo-0.5.0 / nucleo-matcher-0.3.1
  (`score`/`indices` equivalence for finding 1's fix).
- Not measured: µs estimates are allocation-count arithmetic, not profiles; both
  findings sit on the same path as the tracked matcher item and would show in
  the same `RUST_LOG=pikr=debug` spans. Not executed (Linux host): Windows-only
  arms — read, not compiled.
- Could not settle without profiling: first-paint icon resolution (per-name
  `freedesktop_icons::lookup().find()` walks on cold cache — bounded to distinct
  names, cached after), the two `TextLayout` measurements in the query bar per
  keystroke (`view.rs:981-1003`), the 530 ms blink rebuild — all
  sub-µs-to-low-µs, none reaching the tracked items' magnitude.
- Not re-reported (already tracked): matcher grapheme re-decomposition
  (`matcher.rs:135-144`), virtual_stack rebuild-on-`rev` (`view.rs:1084-1104`),
  drun mtime walk, icons stat-per-lookup.

## release ops 2026-08-06

- **AUR publish failed for v0.8.10 and v0.8.11** — `Publish pikr-bin to AUR`
  died on `git push: Could not read from remote repository` (exit 128) in both
  tag runs (31093225592, 31096381308) — the aur.archlinux.org maintenance window
  signature that also cost v0.8.7–v0.8.9. GitHub release, scoop-bucket, and
  brew-tap published for both. **RESOLVED 2026-08-13 — backfill NOT run,
  deliberately:** the outage cleared (v0.8.12's `Publish pikr-bin to AUR`
  succeeded), but the documented `workflow_dispatch` recovery
  (`gh workflow run ci.yml --ref v0.8.11`) is now UNSAFE: the aur-bin step's
  only idempotence check is `git diff --cached --quiet` against the AUR head,
  which pushes whenever the rendered PKGBUILD differs — there is no
  version-ordering guard. With the head now at pkgver 0.8.12, dispatching for
  v0.8.10/v0.8.11 would DOWNGRADE the AUR package. It is also moot: 0.8.12 is
  strictly newer and supersedes both. If AUR backfill for older tags is ever
  wanted, the step first needs a "skip when remote pkgver >= tag version" guard.
