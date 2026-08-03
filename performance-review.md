# Performance review

## Findings

1. `apps/pikr/src/app.rs:92` — Default `drun` collection is synchronous, but it
   took 0.998 ms for 61 entries in a release diagnostic on this host. Moving
   collection to a worker would add state complexity without addressing the
   observed startup cost.
2. `apps/pikr/src/app.rs:112` and `apps/pikr/src/ui/view.rs:982` — Startup
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
5. Floem/wgpu/window initialization remains the largest unmeasured segment on
   the user's compositor and GPU. Debug startup events report microseconds since
   entry into `main` through config load, collection, initial ranking, the first
   paint-ready update phase, and first keyboard focus. Paint-ready and focus are
   one-shot internal markers, not proof that the compositor presented a frame.
   Run `RUST_LOG=pikr=debug pikr` in the graphical session, then use an external
   launch-to-visible/input-ready probe to validate the 500 ms target.

## Coverage

Traced default `drun`, `run` PATH scanning, config, initial ranking, history,
usage, stylesheet, initial icon rendering, Floem application creation, and first
focus. A headless software-rendering probe emitted a 215 ms process-exit sample,
but Sway crashed during the run, so the sample cannot establish readiness and is
excluded. No valid native-compositor or native-GPU measurement was available
because this terminal session has no `WAYLAND_DISPLAY`. Clipboard subprocess
latency, SSH terminal probing, dmenu stdin latency, macOS, and Windows were not
profiled.

Wall-clock regression tests were not added: compositor, shader compilation, disk
cache, and GPU state make a fixed 500 ms CI assertion nondeterministic. The full
project gate verifies behavior; native launch timing must be measured in the
target graphical session.
