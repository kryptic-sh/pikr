# Security Policy

## Supported versions

pikr is pre-1.0. Only the latest 0.1.x patch release receives security fixes.
Older 0.x minors are best-effort once 0.2.0 ships.

| Version | Supported |
| ------- | --------- |
| 0.1.x   | yes       |

## Reporting a vulnerability

**Do not open a public GitHub issue for security reports.**

Email `mxaddict@kryptic.sh` with:

- Affected version(s)
- Description of the issue and impact
- Reproduction steps or proof-of-concept
- Disclosure timeline preference

Acknowledgment within 72 hours. Coordinated disclosure window is typically 30
days from acknowledgment, extendable for complex issues.

## Threat model

pikr is a launcher: it reads `.desktop` files and `$PATH` executables, then
spawns processes on behalf of the user. Key design constraints:

- **No automatic execution of stdin** — `dmenu` mode prints to stdout; the
  caller decides what to do with the selection.
- **`.desktop` `Exec=` parsing** follows the freedesktop spec strictly; no
  shell-string interpolation of untrusted fields.
- **Spawned processes are detached** (setsid + double-fork) so pikr's exit
  cannot kill the launched application; conversely, pikr does not capture
  launched-process file descriptors.
- **Config is pure data** (TOML); no executable hooks, no plugin loading in
  v0.1. Plugin sandboxing will be designed before any plugin host lands (tracked
  under followup issues).
- **No network I/O** in the core binary.

## Dependencies

`cargo deny` runs in cron CI checking RUSTSEC advisories. Vulnerable transitive
dependencies trigger an issue automatically.
