# Backlog

- Validate the under-500-ms cold-launch target in a native graphical session.
  Run `RUST_LOG=pikr=debug pikr` to capture config, collection, initial-rank,
  first-paint-pass, and first-focus markers, then pair them with an external
  launch-to-visible/input-ready measurement. This terminal had no
  `WAYLAND_DISPLAY`, so compositor presentation and GPU initialization were not
  measurable.
- Diagnose local headless Sway connection resets in the keyboard end-to-end
  harness. `cargo nextest run --workspace --locked --no-fail-fast` failed eight
  keyboard tests after Sway reset each Wayland connection; serial execution via
  `NEXTEST_TEST_THREADS=1` failed identically. Format, clippy, release build,
  and non-E2E unit tests passed.
