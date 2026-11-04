# 05 Starting it with the session

## Work

- Windows: a Startup-folder shortcut or a logon Task Scheduler task running
  `pikr daemon`, installed by an opt-in command (`pikr daemon --install`,
  `--uninstall`) and documented. It points at the installed `pikr.exe`, which
  then moves itself to its shadow copy (phase 04).
- Scoop: confirm `scoop update pikr` succeeds with the daemon running (Scoop
  checks for processes running from the app's directory, which the shadow copy
  avoids), and that `scoop uninstall` stops the daemon and removes the startup
  entry (a `pre_uninstall` running `pikr daemon --uninstall`).
- Linux: a systemd user unit bound to `graphical-session.target`, with
  `Restart=always` so it comes back on the new binary after an update exits it,
  getting `WAYLAND_DISPLAY`/`DISPLAY` from the session; documented `exec-once`
  (Hyprland) and `exec` (sway) lines. Packages (deb, rpm, AUR, Alpine) ship the
  unit, disabled.
- macOS: a launchd agent plist with `KeepAlive`, documented, after macOS has
  been run with a window at all. Homebrew upgrades replace the binary in place,
  which the handshake covers.
- README: what the daemon costs (resident memory per platform, from phase 01)
  and buys (first paint), how updates work, and how to turn it off.

## Exit

A fresh install on each platform starts the daemon at login when enabled, and an
update through each package manager replaces the binary with the daemon running,
with no manual step.
