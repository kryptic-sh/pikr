# 06 Measure, test and roll out

## Work

- Quiet-window timings, interleaved: in-process against daemon, dmenu and drun,
  on Windows D3D12 and Linux RADV (and ANV when available), from a hotkey-style
  launch to first paint, with resident memory beside them.
- An update drill on each platform (Windows through Scoop, Linux through a
  package and the AUR, macOS through Homebrew): daemon running, install the next
  build, launch nothing, and confirm the daemon reports the new version within
  seconds, the next launch is warm on it, and no old daemon or stale shadow copy
  remains. Repeat with the self-check disabled to exercise the client backstop:
  the first launch runs in-process and the second is warm on the new version.
- A CI gate for the daemon path, like the startup e2e time gate, and a test of
  the mismatch path (a client of one build against a daemon of another).
- Changelog entries for the daemon, the client fallback, the update path and the
  service files, and a minor version bump (new CLI surface).

## Exit

Numbers in the backlog, the plan closed out per the planning rules, and anything
left undone moved to the backlog.
