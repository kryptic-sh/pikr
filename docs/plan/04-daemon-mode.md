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
- **Linux and macOS** replace a running binary by unlinking it, so the daemon
  runs from the installed path and only the version handshake is needed.
- **Every client replaces a stale daemon** (phase 03): on a build identity
  mismatch it asks the old daemon to exit, serves its own request in-process,
  and starts a new daemon detached from its own binary. The first launch after
  an update is a cold one; every launch after it is warm again.
- **The daemon also notices on its own:** it records the path and modification
  time of the binary it was started from, checks them when idle and on each
  request, and exits when they changed, so a daemon nobody calls does not linger
  in memory on an old version. A service manager restarts it from the new binary
  (phase 05); without one, the next client does.
- Package managers' own checks are covered by the shadow copy, but each manifest
  is checked in phase 05 rather than assumed.

## Exit

The backlog's Windows checks pass through the daemon (transparency, rounded
hit-test, resize, live DPI, IME, Alt-Tab closing, hotkey foreground), the e2e
keyboard suite passes with a daemon running in the sway fixture, and on Windows
a `scoop update pikr` (or a copy over the installed `pikr.exe`) with the daemon
running succeeds, after which the next launch reports the new version and the
old daemon process is gone.
