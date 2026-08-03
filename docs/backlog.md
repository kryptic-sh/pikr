# Backlog

- Validate the under-500-ms cold-launch target in a native graphical session.
  Run `RUST_LOG=pikr=debug pikr` to capture config, collection, initial-rank,
  first-paint-pass, and first-focus markers, then pair them with an external
  launch-to-visible/input-ready measurement. This terminal had no
  `WAYLAND_DISPLAY`, so compositor presentation and GPU initialization were not
  measurable.
- Re-run the headless keyboard end-to-end harness in an environment that permits
  POSIX shared-memory files under `/dev/shm`. This terminal sandbox denies those
  writes, causing wlroots to log `Failed to allocate shm file for keymap`; Sway
  then crashes in `xkb_state_key_get_layout`, and pikr observes a reset Wayland
  connection. The same sandbox failure occurs before the startup optimization,
  so no repository change can make this local gate meaningful.
