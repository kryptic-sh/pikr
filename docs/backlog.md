# Backlog

- Validate the under-500-ms cold-launch target in a native graphical session.
  Run `RUST_LOG=pikr=debug pikr` to capture config, collection, initial-rank,
  first-paint-pass, and first-focus markers, then pair them with an external
  launch-to-visible/input-ready measurement. This terminal had no
  `WAYLAND_DISPLAY`, so compositor presentation and GPU initialization were not
  measurable.
