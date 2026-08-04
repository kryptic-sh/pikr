# Backlog

- drun warm-start residual (documented in `docs/performance-review.md`): the
  mtime cache key requires walking every file under the XDG applications dirs
  on every launch, and the rerank frecency loop still forms a `payload_key`
  string per entry on machines with usage history. Both are accepted
  trade-offs, not scheduled work.
