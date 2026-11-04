# 06 Measure, test and roll out

## Work

- Quiet-window timings, interleaved: in-process against daemon, dmenu and drun,
  on Windows D3D12 and Linux RADV (and ANV when available), from a hotkey-style
  launch to first paint, with resident memory beside them.
- An update drill on each platform: daemon running, install the next build
  through the package manager, launch twice, and confirm the first launch ran
  in-process, the second was warm on the new version, and no old daemon or stale
  shadow copy remains.
- A CI gate for the daemon path, like the startup e2e time gate, and a test of
  the mismatch path (a client of one build against a daemon of another).
- Changelog entries for the daemon, the client fallback, the update path and the
  service files, and a minor version bump (new CLI surface).

## Exit

Numbers in the backlog, the plan closed out per the planning rules, and anything
left undone moved to the backlog.
