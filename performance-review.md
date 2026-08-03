# Performance review

## Findings

1. `apps/pikr/src/app.rs:92` — Default `drun` collection is synchronous, but it
   took 0.998 ms for 61 entries in a release diagnostic on this host. Moving
   collection to a worker would add state complexity without addressing the
   observed startup cost.
2. `apps/pikr/src/app.rs:112` and `apps/pikr/src/ui/view.rs:995` — Startup
   previously ranked the same entry set in `app::run`, reranked it for frecency,
   then reranked again when the query effect registered. The first results were
   discarded. Startup now initializes an empty result set and lets the query
   effect perform the sole frecency-aware initial rank.
3. `apps/pikr/src/modes/run.rs:116` and `apps/pikr/src/modes/run.rs:123` — Unix
   `run` scanning fetched metadata through `DirEntry::metadata()` and fetched it
   again in the executable predicate. The predicate now reuses the first result,
   eliminating one metadata syscall per file candidate.
4. `apps/pikr/src/ui/view.rs:217` — Initial visible rows synchronously resolve
   icons and rasterize SVGs before presentation. The first eight default-mode
   icons took 23.929 ms in a release diagnostic on this host. This is measurable
   but remains well below the 500 ms target.
5. Floem/wgpu/window initialization was the largest unmeasured segment during
   the original review. A later native graphical run on the target machine
   reached the first paint-ready update at 133.144 ms and first keyboard focus
   at 146.471 ms. These one-shot internal markers remain distinct from proof
   that the compositor presented a frame and accepted input by the deadline.

## Native readiness test

Run the external acceptance probe inside the graphical Wayland session:

```bash
scripts/test-startup-readiness.sh
```

The script rebuilds the current release binary, launches a deterministic dmenu
query, and waits for Pikr's monotonic first-focus marker. It rejects a marker
past the 500 ms deadline without injecting input. After timely compositor focus,
it sends an unhandled `F12` followed by Return through `wtype` and requires the
selected `banana` result. It prints the internal startup markers and exits
non-zero on late focus, missing focus, failed input, or the wrong result.
Because `wtype` input is global, do not switch focus while the probe runs.

Use `--delay` to find the lower readiness boundary:

```bash
scripts/test-startup-readiness.sh --delay 0.400
```

Repeat each deadline several times. The script validates input readiness; use an
external recording or compositor probe separately when launch-to-visible timing
is required.

## Coverage

Traced default `drun`, `run` PATH scanning, config, initial ranking, history,
usage, stylesheet, initial icon rendering, Floem application creation, and first
focus. A headless software-rendering probe emitted a 215 ms process-exit sample,
but Sway crashed during that run, so the sample cannot establish readiness and
is excluded. A later native graphical run measured internal paint and focus, but
external candidate acceptance and compositor presentation remain pending.
Clipboard subprocess latency, SSH terminal probing, dmenu stdin latency, macOS,
and Windows were not profiled.

Deterministic unit regressions cover the initial, unchanged, and changed-query
rerank policy plus executable-only PATH scanning. A headless E2E assertion
launches the real release binary and checks that startup emits exactly one
`AppState::rerank` trace before dismissal. A fixed 500 ms wall-clock assertion
remains unsuitable for CI: compositor, shader compilation, disk cache, and GPU
state make it nondeterministic. Native launch timing must be measured in the
target graphical session.
