# Backlog

## Open findings

- MoveDown overflow with a saturated count prefix: `push_count_digit` saturates
  at `usize::MAX` (`picker/state.rs`), but the consumer in the MoveDown handler
  (`ui/view.rs`, `(cur + n).min(...)` with `n` from `take_count()`) is not
  saturating — `n == usize::MAX` overflows for `cur >= 1` (debug panic, release
  wrap to `cur - 1`, so the selection moves up). Repro: 20 nines then `j`. Fix:
  `cur.saturating_add(n)`.
- Windows drun cache mtime granularity: `load_cache` compares seconds-truncated
  max mtimes with `!=` — a shortcut added within the same second (or FAT's 2 s
  granularity) as the cached max is invisible until a later change. Pre-existing
  class, Windows-only.
- Icon byte cache never invalidates mid-session: `file_bytes` / `raster_cache` /
  `resolve_or_fallback` in `picker/icons.rs` hold per-path bytes for the whole
  session — an icon file replaced on disk while pikr is running keeps rendering
  the old bytes until restart.
- Frecency `payload_key` U+001F separator: on Unix a program path may legally
  contain U+001F, so `Exec{program:"a\u{1f}b"}` still collides with
  `Exec{program:"a", args:["b"]}` — the comment's "can't appear in a program
  path" overstates. Absurd input, ranking bias only.

## Accepted trade-offs (not scheduled)

- drun warm-start residual: the mtime cache key walks every file under the XDG
  applications dirs on every launch, and the rerank frecency loop still forms a
  `payload_key` string per entry on machines with usage history. Documented in
  the perf review; both are deliberate.
