# 02 A host that outlives one picker

Reshape the code so one window and GPU can serve many pickers, with no change in
behaviour: the in-process path runs one picker and exits as today.

## Work

- Split `ui::run::run` into a `Host` (shell, window, `Gpu`, fonts, icons: what
  is expensive to make) and a per-picker run that takes a `Content` and a
  `WindowSpec` and returns `(payloads, code)` instead of executing them and
  calling `std::process::exit`. `run` keeps its signature by building a `Host`,
  running once and doing what it does now with the result. Split `ui/run.rs`
  along this seam while doing it; it is already over 1100 lines.
- A per-picker `WindowSpec` can differ from the last (`--width`, `--lines`,
  `-P`, `--message`): resize the window and let the `Presenter` reconfigure, and
  reload fonts only when the theme's family or size changed.
- Modes read the process environment today: `run` walks `PATH`, `ssh` reads
  `USERPROFILE`/`HOME` and `TERMINAL`, `drun` reads XDG paths. Give
  `Mode::collect` a request context (environment and working directory) so a
  daemon can collect with the client's, not its own. In-process, the context is
  the process's own.
- Config, history and usage are reloaded per picker (config loads in about 1.3
  ms), so a daemon never serves stale settings and does not fight a non-daemon
  pikr writing the same files.

## Exit

Workspace gate green, the e2e keyboard suite and the startup gate pass, and a
Windows release build behaves as v0.9.8 on the backlog's Windows checks.
