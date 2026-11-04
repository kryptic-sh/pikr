# 04 Daemon mode and updates

## Work

- `pikr daemon` (a subcommand or `--daemon`): opens the `Host` with the window
  hidden, listens, and serves one picker at a time. An IPC thread wakes the
  event loop through `Shell::waker`.
- A request arriving while a picker is open ends that picker with exit 1 and
  takes over, matching today's behaviour where a second pikr takes the keyboard
  and the first closes.
- Focus loss, Escape and accept hide the window and answer the client; they no
  longer end the process.
- Placement: show on the monitor the client is on (the pointer's or the focused
  one's), not where the window was last.
- The accessibility tree is updated only while shown; a hidden picker must not
  be read out.
- Recovery: a lost GPU device (driver update, TDR) or a display change while
  hidden rebuilds the `Gpu` on the next request; losing the display connection
  (compositor restart, logout) ends the daemon, and clients fall back to
  in-process.
- Caches across requests: fonts while the theme is unchanged, icon lookups,
  drun's app list with its existing refresh. Memory that is only needed while
  shown (frame transients) can be released on hide; measure before adding that.
- `pikr daemon --stop` and `--status`, through the same socket.

## Updates without killing anything by hand

- **Windows: the daemon never runs from the installed binary.** `pikr daemon`
  copies its own executable to
  `%LOCALAPPDATA%\pikr\daemon\pikr-<build identity>.exe` (skipping the copy when
  that file already exists and matches) and re-launches itself from there, then
  exits. The installed `pikr.exe`, and Scoop's app directory, are never held
  open, so `scoop update`, an MSI or a manual copy replace them freely. On start
  the daemon deletes the older copies in that directory by name, skipping any
  still in use (a delete of a running copy fails, which is expected and not an
  error).
- **Linux and macOS** let a package manager replace a running binary (the old
  file is unlinked and the daemon keeps running the old code from it), so the
  daemon runs from the installed path. Nothing blocks the update there; the work
  is making the running daemon pick the new version up, below.
- **On every platform the daemon restarts itself on the new version.** It
  records the installed path it was started from (on Windows the original
  `pikr.exe`, not the shadow copy) with that file's identity (size, modification
  time and, on unix, the inode), and checks it on each request and every few
  seconds while idle. When it changed and the new file runs (`<path> --version`
  answers with a different build identity, which also waits out a package
  manager that is still writing), the daemon hands over:
  - Linux and macOS: `exec` the installed path with the same arguments. The
    process keeps its pid, so a service manager sees no exit; the socket is
    closed on exec and bound again by the new image.
  - Windows: start the new `pikr daemon` (which makes its own shadow copy), wait
    until it answers on the pipe, then exit. The new daemon takes over the pipe
    name when the old one closes its instance.
  - A picker open at that moment finishes first; the hand-over waits for the
    window to hide.
- **Every client also replaces a stale daemon** (phase 03), as the backstop for
  a daemon that missed the change (a check that failed, a file that came back
  with the same modification time): on a build identity mismatch it asks the old
  daemon to exit, serves its own request in-process, and starts a new daemon
  detached from its own binary.
- So after an update the first launch is usually already warm on the new version
  (the daemon restarted within seconds of the install), and at worst one launch
  is cold. No platform needs the daemon killed by hand.
- Package managers' own checks are covered by the shadow copy, but each manifest
  is checked in phase 05 rather than assumed.

## Exit

The backlog's Windows checks pass through the daemon (transparency, rounded
hit-test, resize, live DPI, IME, Alt-Tab closing, hotkey foreground), the e2e
keyboard suite passes with a daemon running in the sway fixture, and on each
platform an update with the daemon running (`scoop update pikr` or a copy over
`pikr.exe` on Windows; a package upgrade or a copy over the binary on Linux and
macOS) succeeds, after which the daemon reports the new version without any
launch in between, and no old daemon process remains. Tests cover the change
detection (a replaced file is seen, an unchanged one is not, each shown red
against a broken check).
