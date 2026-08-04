# Performance review — 2026-08-04

Scope: entire codebase, working tree clean at `5e5b120`. Weighted toward the
two paths that matter for this project: cold-start latency (acceptance target:
usable picker < 500 ms) and per-keystroke rerank latency (typing lag). Read
with the callers and data sizes: drun collects ~1000–3000 `.desktop` entries on
a typical system; `max_results` defaults to 256, `VISIBLE_ROWS` to 8.

## Findings (ranked by impact)

### 1. Frecency loop allocates per entry, per rerank — `mode_key` is loop-invariant

`apps/pikr/src/ui/view.rs:692–698` calls `usage.bonus(cli_mode, payload, now)`
for every entry on **every** rerank — each keystroke and the startup rerank
(the effect fires once at registration with `prev = None`, view.rs:1016, and
`rerank_if_query_changed` reranks since `None != Some("")`). Each call pays
`mode_key(cli_mode)` at `apps/pikr/src/picker/frecency.rs:106` →
`format!("{mode:?}").to_lowercase()` (frecency.rs:144–146) — two String
allocations per entry, for a value that is identical across the whole loop and
the whole rerank. At ~2000 drun entries that is ~4000 allocations per
keystroke, plus once on the startup critical path (empty-query rerank). The
same loop then pays `payload_key(payload)` (frecency.rs:109): for
`Exec { program, args }` entries with args (drun's common shape) that is
`args.join("\u{1f}")` + `format!` — two more allocations per entry, again even
for entries that have no usage record (lookup misses dominate on a
fresh-ish `usage.toml`). Verified loop-invariant: `cli_mode` is fixed for the
duration of `rerank` (view.rs:681–700 holds no mode mutation).

Fix: hoist the mode key out of the loop —
`let mode_key = mode_key(self.cli_mode)` once per rerank and have `bonus` take
`&str` instead of `CliMode` (or cache the key on `Usage`). That removes the
per-entry `mode_key` cost outright (N → 1). The `payload_key` cost is the
string-keyed design's residual; it only applies per entry whose payload key
must be formed, and could be skipped entirely when the mode's table is empty —
the existing `per_mode` check (frecency.rs:107) already short-circuits that
case before forming the key, so the residual only bites machines with usage
history. Expect the biggest win on the empty-query startup rerank (every entry
gets a bonus lookup) and on each keystroke over large corpora.

### 2. Matcher allocates a fresh positions `Vec` per field per entry

`apps/pikr/src/picker/matcher.rs:127` — `let mut positions = Vec::new();`
inside `match_field`, called twice per entry per rerank (label + description,
matcher.rs:83–84). That is 2N heap allocations per keystroke on top of finding
1, and the buffers are throwaway — `atom.indices` (nucleo-matcher 0.3.1
`pattern.rs:331`) fills whatever `&mut Vec<u32>` it is handed and does **not**
clear it, so a `Vec` reused across calls is correct as long as it is `clear()`ed
first. Fix: give `Matcher` a scratch `Vec<u32>` field, `clear()` it at the top
of `match_field`, pass `&mut self.scratch`. Eliminates 2N allocations per
rerank for ASCII text (the common case); the non-ASCII `text_buf`
(matcher.rs:117–125) is unaffected.

### 3. drun on unix re-parses every `.desktop` file on every launch — the dominant cold-start cost

`apps/pikr/src/app.rs:93` runs `mode.collect()` synchronously before the
window exists; for drun that is `apps/pikr/src/modes/drun.rs:44–95` — a full
`Iter::new(default_paths())` walk, `DesktopEntry::from_path` parse, `parse_exec`
shlex split and `to_lowercase` sort-key per entry, every launch. The Windows
side solves this with an mtime-keyed cache (drun.rs:262–333); the unix side has
no equivalent, so the 500 ms usable-startup target is gated on disk I/O +
parse of the whole `.desktop` corpus each cold start. Not a code defect — a
staleness-vs-startup design decision, and exactly the trade the Windows cache
already made. If measurements (`scripts/test-startup-readiness.sh`) show drun
startup missing the target, an mtime-keyed cache over the `.desktop` dirs (same
shape as the Windows one) is the highest-leverage fix available; the icon
resolution is already deferred out of collect (lazy per-row, cached), so
collect is pure parse cost.

### 4. Minor: per-rerank clones in the virtual_stack data fn and per-row label rebuild

- `apps/pikr/src/ui/view.rs:1079–1080` — the data fn clones `positions` +
  `desc_positions` for every match into the `imbl::Vector` on each `rev` bump
  (bounded by `max_results` = 256, so ~512 small Vec allocations per rerank;
  free for the empty query since the Vecs are empty). Could clone less by
  storing `Rc<Vec<u32>>` on `Match` (positions are immutable after rank), but
  the win is bounded and the row-key contract (view.rs:1087–1093) compares
  them — low priority.
- `apps/pikr/src/ui/view.rs:56` — `highlighted_label`'s compute closure runs
  `FamilyOwned::parse_list(&font_family)` per visible row per rebuild (8 rows ×
  per keystroke + per scroll). `font_family` is loop-invariant per picker
  session; parse it once outside the closure. Bounded by `VISIBLE_ROWS`, minor.

## Coverage

- Traced end-to-end: startup (`app.rs::run` → `Config::load` → `collect` →
  `Usage::load`/`History::load` → `build_stylesheet` → `picker_view` → first
  rerank-effect fire → bonus loop → data fn → first row build) and the typing
  path (`keydown` → `rerank` → `matcher::rank` → bonus loop → sort → truncate
  → `rev` → data fn → visible-row rebuilds → `entry_row` icon/label work).
- Verified against sources: `mode_key`/`payload_key` allocation sites
  (frecency.rs:106, 109, 135, 144–146), nucleo `indices` non-clearing contract
  (nucleo-matcher 0.3.1 `pattern.rs:330`), data sizes (`max_results` 256,
  `VISIBLE_ROWS` 8, `HISTORY_CAP` 100).
- Not profiled: no timing run against `scripts/test-startup-readiness.sh` or
  the release binary — allocation counts above are static estimates, not
  measured. Not audited: `freedesktop-desktop-entry`'s per-file parse cost
  (drun collect's bulk), nucleo's fuzzy-match engine internals, floem's
  render/repaint costs. The icon-byte cache (220d957) and SVG raster cache
  mean per-row icon work is hash-lookup + Arc-clone + one `Vec` clone per
  `img()` call — not re-litigated here.
