# Backlog

## Open findings

### Correctness / security

- **MEDIUM — `:mode` switching from Visual leaves Visual armed with a stale
  anchor; the next Enter executes a range of the new mode's list.**
  `switch_mode` (`ui/view.rs`) resets `query`/`query_cursor`/`selected` but
  never `vim_mode` or `visual_anchor`; `:` is reachable from Visual
  (`picker/keys.rs`, `normal_or_visual_key`), and `Accept` in Visual computes
  the range `anchor.min(sel)..=anchor.max(sel)` over the _new_ mode's matches.
  Repro: drun with ≥6 rows → `v` (anchor=0) → `5j` (selected=5) → `:` → `run` →
  Enter (switch: selected→0, anchor stays 5, mode stays Visual) → Enter again
  (Accept) → executes `matches[0..=5]` of the run list. Expect: accept row 0 (or
  return to Normal). Actual: up to 6 payloads launched by one Enter. Fix: in
  `switch_mode`, also `vim_mode.set(Normal)`, `visual_anchor.set(None)`, and
  clear `count`.
- **LOW — ssh host interpolated verbatim into a shell string for pwsh/cmd.**
  `modes/ssh.rs` `build_terminal_args` emits `format!("ssh {host}")` for
  `pwsh -Command` / `cmd /K`, which parse it as a script; `Host foo; notepad` in
  `~/.ssh/config` executes `notepad` after `ssh foo` (also reachable on Unix
  with `$TERMINAL=pwsh`). Input is the user's own config — no privilege gain —
  but it is unescaped file data in a shell string. Fix: shell-quote or validate
  the host against a safe charset; prefer argv (`wt --`/`-e`) paths.
- **LOW — space-separated `Host a b` pattern lists parsed as one alias.**
  `modes/ssh.rs` keeps the whole value as `current_host`, so `ssh` receives a
  single bogus hostname `"a b"` (on pwsh/cmd it instead runs the remote command
  `b` on host `a`). Fix: split the value on whitespace into per-pattern entries
  (still skipping `*`/`?`/`!` patterns).
- **LOW — locale precedence wrong in `current_locales`.** `modes/drun.rs`
  iterates `["LC_MESSAGES", "LC_ALL", "LANG"]` and breaks on the first set var;
  POSIX says `LC_ALL` > `LC_*` > `LANG`, so with both `LC_ALL` and `LC_MESSAGES`
  set the names come out in the wrong language (and that locale is baked into
  the drun cache key). Fix: iterate `["LC_ALL", "LC_MESSAGES", "LANG"]`.

### Performance

- **Matcher allocates ~4-7k transient heap buffers per keystroke on emoji (~1800
  entries).** `picker/matcher.rs` `rank` allocates `lp`/`dp`
  (`Vec::with_capacity(query_len)`) per entry, and `match_field` allocates a
  fresh `text_buf` per non-ASCII field — every emoji label is non-ASCII, and
  most allocations die on arrival when the entry fails to match; the
  `grapheme_positions_to_codepoints` scan is O(graphemes × positions). Fix:
  reuse scratch buffers (`Matcher` already owns `scratch`) and/or precompute
  each entry's grapheme-segmented NFC `Vec<u32>` at collect time so
  `match_field` feeds nucleo a `Utf32Str::Unicode` slice directly (also removes
  the codepoint conversion).
- **Frecency builds and hashes a fresh `payload_key` String per entry per
  keystroke.** `ui/view.rs` rerank loop calls `usage.bonus(...)` per matched
  entry → `frecency.rs` `payload_key` (a `clone()` for Stdout, a `format!` chain
  for Exec) + HashMap hash. Key is invariant across reranks — precompute once
  per entry at collect time. Also hoist the `0.5_f64.powf(1/HALF_LIFE)` to a
  constant (`exp2(-dt/half)` is cheaper than `powf`).
- **ssh terminal probe spawns a subprocess per candidate on the launch path.**
  `modes/ssh.rs` `resolve_terminal` runs `Command::new(name).arg("--version")`
  - wait per candidate (~2-10 ms each, worst case 10-30+ ms in the 500 ms
    budget, pure waste on the common miss). Fix: stat `$PATH` for the binary
    (run mode already has the machinery) instead of spawning.
- **run mode stats every PATH entry at startup.** `modes/run.rs` `scan_dir`
  calls `entry.metadata()` before the `is_file` filter — ~1000-3000 stats once
  per launch. Fix: use `entry.file_type()` (readdir `d_type`, no syscall) for
  the file check, stat only regular files.
- **Query-bar blink thread redraws forever.** `ui/view.rs` 530 ms blink effect
  rebuilds the query-bar `dyn_view` (2 `TextLayout` measurements) plus a repaint
  every 530 ms even when idle/focused-out. Gate on focus or on `blink_on`
  toggling only while the view is visible.

## Hardening (correct today, fragile — not defects)

- `payload_key` is not injective _across_ `Payload` variants: `Stdout("2:ab")`
  == `Exec{program:"ab", args:[]}` == `"2:ab"` (also `SetClipboard(x)` ==
  `Stdout(x)`). The comment claiming "injective over all byte strings"
  overstates — it is injective within `Exec` only. Unreachable today because
  each mode emits a single variant; fix the comment, or variant-tag the key if a
  mode ever mixes variants.
- Clipboard accept (`modes/clipboard.rs`) pipes `cliphist decode {id} | wl-copy`
  through `sh -c` and never observes the child's exit — a missing `wl-copy`
  fails silently (user selects an entry, nothing happens). Not an injection (id
  is a parsed `u64`).
- Icon name→path misses are cached forever (`picker/icons.rs` `resolve`) — a
  theme installed while pikr runs never resolves until restart.
- Windows drun icon extraction (`modes/drun_icons_windows.rs`, called from a
  rayon `par_iter` in drun) writes cache PNGs without a lock; same-target
  duplicates or a first-miss storm on `__fallback__.png` can interleave
  truncate/write and yield a corrupt PNG (identical bytes in practice; cosmetic,
  Windows-only, survives until cache wipe).
- `config.rs` `font_size: f32` accepts TOML `nan`/`inf`; effect on layout
  unverified.
- `$TERMINAL` is treated as a single binary name — the common
  `TERMINAL="alacritty --class foo"` idiom silently falls back to a candidate.
- `current_locales` (drun) drops the codeset and `@modifier` — significant for
  e.g. `sr_RS@latin`.

## Accepted trade-offs (not scheduled)

- drun warm-start residual: the mtime cache key walks every file under the XDG
  applications dirs on every launch, and the rerank frecency loop still forms a
  `payload_key` string per entry on machines with usage history. Documented in
  the perf review; both are deliberate.
- Icon byte caches (`picker/icons.rs` `file_bytes`/`rasterise_svg`) stat per
  lookup on every row rebuild (~10 µs/keystroke for ~10 visible rows). This is
  the documented freshness mechanism behind the mtime+len invalidation; not
  material on its own — revisit only if profiling shows it.
