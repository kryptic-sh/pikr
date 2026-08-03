# Backlog

- Prototype the pure-Rust GUI replacement described in
  [`gui-backend-research.md`](gui-backend-research.md). Test Iced with
  `iced_layershell` and software rendering against the current layout, CSS
  contract, Wayland behavior, accessibility, FreeBSD build, and cold-start
  target before changing the production frontend. Use a narrow SCTK frontend
  only if the Iced prototype fails an intrinsic requirement.
- Validate the under-500-ms cold-launch target in a native graphical session.
  Run `RUST_LOG=pikr=debug pikr` to capture config, collection, initial-rank,
  first-paint-pass, and first-focus markers, then pair them with an external
  launch-to-visible/input-ready measurement. This terminal had no
  `WAYLAND_DISPLAY`, so compositor presentation and GPU initialization were not
  measurable.
