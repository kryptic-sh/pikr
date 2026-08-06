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
