# Backlog

## Open findings

## Accepted trade-offs (not scheduled)

- drun warm-start residual: the mtime cache key walks every file under the XDG
  applications dirs on every launch, and the rerank frecency loop still forms a
  `payload_key` string per entry on machines with usage history. Documented in
  the perf review; both are deliberate.
