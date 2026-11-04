# 03 Client, daemon and the protocol

## Transport

- Linux and macOS: a Unix socket at `$XDG_RUNTIME_DIR/pikr/daemon.sock` (macOS:
  under the per-user temp dir), directory mode 0700, and the peer's uid checked
  (`SO_PEERCRED` / `getpeereid`) before a request is read.
- Windows: a named pipe `\\.\pipe\pikr-<user SID>`, created with a DACL that
  grants only the current user and `PIPE_REJECT_REMOTE_CLIENTS`, and the
  client's session checked (`GetNamedPipeClientProcessId`,
  `ProcessIdToSessionId`).
- The client connects with a short timeout (a few ms). No answer means no
  daemon: run in-process exactly as today. `--no-daemon` (and an environment
  variable for scripts) forces in-process.

## Messages

Length-prefixed frames, TOML-encoded through the existing `serde`/`toml`.

- **Hello, first, both ways:** the protocol version, pikr's version, and a build
  identity (the version plus a hash of the binary, so two builds of the same
  version still differ). The handshake is the one part of the protocol that may
  never change shape, because it is how an old daemon and a new client find out
  they disagree.
- **Version mismatch:** the client sends `Shutdown`, the daemon answers, hides
  everything and exits, and the client serves this request in-process (no wait
  on the new daemon) and starts the new daemon detached, from its own, updated
  binary (phase 04). This is the normal path after every update, not an error.
- Request: argv, working directory, the environment variables modes read, and on
  Windows the client's pid so the daemon can be granted the foreground. dmenu's
  stdin is streamed after the request as line frames, reusing the `--loading`
  path (`modes::dmenu::read_stdin_in_background`'s receiver), so a long or slow
  pipe does not delay the window.
- Response: the payloads and the exit code. **The client executes the
  payloads**, not the daemon: `Stdout` must reach the client's stdout,
  `Exec`/`ShellApp` should start with the client's environment and foreground
  right, and `SetClipboard`/`ExecWait` report their errors where the user
  launched pikr. The daemon then never starts a program itself.
- Cancel: the client closing its end (Ctrl-C, killed parent) closes the picker
  with exit 1.

## Exit

Unit tests for framing and for the version handshake, each shown red against a
broken encoder or a forced mismatch, and an integration test that runs a dmenu
request through a daemon in a headless backend and gets the chosen line on the
client's stdout.
