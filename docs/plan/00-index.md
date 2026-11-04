# Plans

## Resident pikr: a daemon that keeps the window and GPU warm

Goal: a hotkey launch reaches first paint in well under 100 ms on Windows, where
today about 300 ms of every launch is `D3D12CreateDevice` (see the backlog's
D3D12 device creation entry). A `pikr daemon` holds the shell, a hidden window
and the GPU `Presenter`; a plain `pikr` invocation becomes a thin client that
hands its request to the daemon and falls back to today's in-process path when
no daemon answers.

**Hard requirement: updating pikr never needs the daemon killed by hand.**
Windows will not overwrite or delete a running `.exe` (it can only be renamed),
and Scoop refuses to update an app while a process from its directory runs. So
the daemon never runs from the installed binary on Windows, and every client
checks the daemon's version and replaces a stale one itself (phase 03 and 04).

| Phase                                                        | Status  |
| ------------------------------------------------------------ | ------- |
| [01 Spike: re-show a warm window](01-spike-reshow.md)        | planned |
| [02 A host that outlives one picker](02-reusable-host.md)    | planned |
| [03 Client, daemon and the protocol](03-ipc-protocol.md)     | planned |
| [04 Daemon mode and updates](04-daemon-mode.md)              | planned |
| [05 Starting it with the session](05-service-integration.md) | planned |
| [06 Measure, test and roll out](06-verify-rollout.md)        | planned |

### Decisions for the user

- **Opt-in or automatic.** Recommended: opt-in to start, automatic after. The
  daemon first runs when the user enables it (`pikr daemon`, or a service file
  pikr ships); from then on a client restarts it itself after an update or a
  crash. The alternative, a client that spawns a daemon on its very first
  launch, makes a background process appear that nobody asked for.
- **Platforms.** Recommended: build the protocol for all three, ship and
  document it for Windows first. On Linux first paint is already about 20 ms
  (RADV), so the daemon buys little there beyond ANV's `UiRenderer::new`; macOS
  has never been run with a window.
- **Dependencies.** Recommended: none new. Unix sockets come from std, Windows
  named pipes from the `windows` crate pikr already uses (more features:
  `Win32_System_Pipes`, `Win32_Security`, `Win32_System_Threading`), and
  messages are length-prefixed TOML through the existing `serde` and `toml`. A
  crate such as `interprocess` would save the pipe code but is a new dependency,
  so it is the user's call.
