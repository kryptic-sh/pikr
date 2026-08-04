# Backlog

- Validate the under-500-ms cold-launch target in a native graphical session.
  Run `RUST_LOG=pikr=debug pikr` to capture config, collection, initial-rank,
  first-paint-pass, and first-focus markers, then pair them with an external
  launch-to-visible/input-ready measurement. This terminal had no
  `WAYLAND_DISPLAY`, so compositor presentation and GPU initialization were not
  measurable.
- Flaky smoke test: `startup_readiness_script::accepts_matching_candidate_after_focus_within_deadline`
  fails intermittently under full-suite parallel load (script reports "got
  status 0 and output ''" at ~0.1 s; passes in isolation). Cause is a
  coproc/process-substitution race in `scripts/test-startup-readiness.sh` —
  the fake pikr's stdout "banana" is lost. Found 2026-08-04; not fixed.
- drun warm-start residual (documented in `docs/performance-review.md`): the
  mtime cache key requires walking every file under the XDG applications dirs
  on every launch, and the rerank frecency loop still forms a `payload_key`
  string per entry on machines with usage history. Both are accepted
  trade-offs, not scheduled work.
