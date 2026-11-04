# 01 Spike: re-show a warm window

Prove the gain before reshaping anything. A scratch build, not committed.

## Work

- In `ui::run::run`, after a picker exits, keep the shell, window and `Gpu`
  instead of destroying them: hide the window (`Shell::set_visible(false)`),
  wait for a trigger, build a fresh `Session` the way `app::run` does, show the
  window again and run it.
- Log the time from "show requested" to the first paint pass, next to the cold
  launch's `startup first paint pass reached`.
- On Windows, run the spike binary from a copy outside its build directory and
  replace the original while it runs, to confirm the shadow-copy idea in phase
  04 (the original must be overwritable, the copy must keep running).

## What it must answer

- Time to first paint on a re-show, per platform: Windows D3D12 (the RX 7900 XTX
  here) and Linux on RADV, and on ANV if the Intel laptop is available.
- Whether the `Presenter` survives a hidden window unchanged, or needs a
  swapchain reconfigure on re-show, and what that costs.
- Focus on re-show. Windows: a hidden process has no foreground right, so test
  the client granting it (`AllowSetForegroundWindow` with the daemon's pid,
  called by a process that has just had input, as a hotkey launch does; the
  backlog records hotkey launches getting the foreground 6 of 6). Wayland
  layer-shell: whether re-mapping with `PANEL_EXCLUSIVE_KEYBOARD` takes the
  keyboard. X11 and GNOME toplevels: whether an activation token from the client
  is needed (`XDG_ACTIVATION_TOKEN`, `DESKTOP_STARTUP_ID`).
- Transparency, the rounded input region, DPI and the IME after a hide and show,
  checked the way the backlog's Windows entries check them.
- Resident memory while hidden (the backlog has 73–76 MB private bytes on the
  AMD D3D12 driver, 39–42 MB on WARP).

## Exit

Numbers taken in a quiet window and written to the backlog. Go if re-show first
paint on Windows is under 100 ms with focus working; otherwise record why and
stop.
