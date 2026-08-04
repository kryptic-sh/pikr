# Backlog

## Open findings

### Performance

- **Query-bar blink thread redraws forever.** `ui/view.rs` 530 ms blink effect
  rebuilds the query-bar `dyn_view` (2 `TextLayout` measurements) plus a repaint
  every 530 ms even when idle/focused-out. Gate on focus or on `blink_on`
  toggling only while the view is visible.

## Hardening (correct today, fragile — not defects)

- `payload_key` is not injective _across_ `Payload` variants: `Stdout("2:ab")`
  == `Exec{program:"ab", args:[]}` == `"2:ab"` (also `SetClipboard(x)` ==
  `Stdout(x)`). The comment claiming "injective over all byte strings"
  overstates — it is injective within `Exec` only. Unreachable today because
  each mode emits a single variant; fix the comment, or variant-tag the key if a
  mode ever mixes variants.
- Clipboard accept (`modes/clipboard.rs`) pipes `cliphist decode {id} | wl-copy`
  through `sh -c` and never observes the child's exit — a missing `wl-copy`
  fails silently (user selects an entry, nothing happens). Not an injection (id
  is a parsed `u64`).
- Icon name→path misses are cached forever (`picker/icons.rs` `resolve`) — a
  theme installed while pikr runs never resolves until restart.
- Windows drun icon extraction (`modes/drun_icons_windows.rs`, called from a
  rayon `par_iter` in drun) writes cache PNGs without a lock; same-target
  duplicates or a first-miss storm on `__fallback__.png` can interleave
  truncate/write and yield a corrupt PNG (identical bytes in practice; cosmetic,
  Windows-only, survives until cache wipe).
- `config.rs` `font_size: f32` accepts TOML `nan`/`inf`; effect on layout
  unverified.
- `$TERMINAL` is treated as a single binary name — the common
  `TERMINAL="alacritty --class foo"` idiom silently falls back to a candidate.
- `current_locales` (drun) drops the codeset and `@modifier` — significant for
  e.g. `sr_RS@latin`.

## Accepted trade-offs (not scheduled)

- drun warm-start residual: the mtime cache key walks every file under the XDG
  applications dirs on every launch, and the rerank frecency loop still forms a
  `payload_key` string per entry on machines with usage history. Documented in
  the perf review; both are deliberate.
- Icon byte caches (`picker/icons.rs` `file_bytes`/`rasterise_svg`) stat per
  lookup on every row rebuild (~10 µs/keystroke for ~10 visible rows). This is
  the documented freshness mechanism behind the mtime+len invalidation; not
  material on its own — revisit only if profiling shows it.
