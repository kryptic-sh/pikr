#!/usr/bin/env bash

set -uo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/test-startup-readiness.sh [--delay SECONDS] [--e2e-delay SECONDS]

Launch the release Pikr binary in dmenu mode, require its monotonic first-focus
marker to fall within the chosen deadline, then verify that it accepts F12 and
Return and emits the matching candidate. Also measures the end-to-end time —
launch to process exit after the selection is locked in — and requires it to
fall within the e2e deadline.

Options:
  --delay SECONDS     First-focus deadline in seconds (default: 0.500)
  --e2e-delay SECONDS  End-to-end launch-to-exit deadline in seconds (default: 0.500)
  -h, --help          Show this help

Environment:
  PIKR_BIN          Explicit binary to test (default: build current source)
  WTYPE_BIN         wtype executable (default: wtype)
  TIMEOUT_BIN       timeout executable (default: timeout)

Run this inside the graphical Wayland session being measured. Do not switch
focus while the probe runs. Keys are sent only after Pikr reports compositor
focus. Repeat the probe several times; lower --delay to find the boundary.
EOF
}

delay="0.500"
e2e_delay="0.500"
while (($# > 0)); do
  case "$1" in
    --delay)
      if (($# < 2)); then
        printf 'error: --delay requires a value\n' >&2
        exit 2
      fi
      delay="$2"
      shift 2
      ;;
    --e2e-delay)
      if (($# < 2)); then
        printf 'error: --e2e-delay requires a value\n' >&2
        exit 2
      fi
      e2e_delay="$2"
      shift 2
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      printf 'error: unknown argument: %s\n' "$1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ ! "$delay" =~ ^[0-9]+([.][0-9]+)?$ ]]; then
  printf 'error: --delay must be a non-negative number of seconds\n' >&2
  exit 2
fi
if [[ ! "$e2e_delay" =~ ^[0-9]+([.][0-9]+)?$ ]]; then
  printf 'error: --e2e-delay must be a non-negative number of seconds\n' >&2
  exit 2
fi

if [[ -z "${WAYLAND_DISPLAY:-}" ]]; then
  printf 'error: WAYLAND_DISPLAY is not set; run inside the target Wayland session\n' >&2
  exit 2
fi

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$script_dir/.." && pwd)
wtype_bin="${WTYPE_BIN:-wtype}"
timeout_bin="${TIMEOUT_BIN:-timeout}"

if [[ -n "${PIKR_BIN:-}" ]]; then
  pikr_bin="$PIKR_BIN"
  if [[ ! -x "$pikr_bin" ]]; then
    printf 'error: PIKR_BIN is not executable: %s\n' "$pikr_bin" >&2
    exit 2
  fi
else
  pikr_bin="$repo_root/target/release/pikr"
  printf 'Building current source in release mode...\n'
  if ! CARGO_TARGET_DIR="$repo_root/target" cargo build \
    --manifest-path "$repo_root/Cargo.toml" \
    --release --locked --bin pikr; then
    printf 'error: release build failed\n' >&2
    exit 2
  fi
fi

for tool in "$wtype_bin" "$timeout_bin" setsid awk; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    printf 'error: required executable not found: %s\n' "$tool" >&2
    exit 2
  fi
done

deadline_us=$(awk -v delay="$delay" 'BEGIN { printf "%.0f", delay * 1000000 }')
e2e_deadline_ms=$(awk -v d="$e2e_delay" 'BEGIN { printf "%.0f", d * 1000 }')
watchdog_seconds=$(awk -v delay="$delay" 'BEGIN { printf "%.3f", delay + 2 }')
default_focus_attempts=$(awk -v delay="$delay" 'BEGIN { printf "%.0f", (delay + 1) * 100 }')
pid=""
err_file=""
out_file=""

terminate_child() {
  if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
    kill -TERM -- "-$pid" 2>/dev/null || true
    sleep 0.1
    kill -KILL -- "-$pid" 2>/dev/null || true
  fi
  if [[ -n "$pid" ]]; then
    wait "$pid" 2>/dev/null || true
    pid=""
  fi
}

# Invoked indirectly by the EXIT trap below.
# shellcheck disable=SC2329
cleanup() {
  terminate_child
  if [[ -n "$err_file" ]]; then
    rm -f "$err_file"
  fi
  if [[ -n "$out_file" ]]; then
    rm -f "$out_file"
  fi
}

trap cleanup EXIT
trap 'exit 130' INT TERM HUP

err_file=$(mktemp) || exit 2
out_file=$(mktemp) || exit 2

printf 'Testing first focus within %s seconds, end-to-end within %s seconds, then candidate acceptance...\n' \
  "$delay" "$e2e_delay"
e2e_start_ms=$(date +%s%3N)
# Capture stdout/stderr in regular files instead of pipes: after `wait` the
# files are complete, so a line Pikr wrote just before exiting can never be
# lost to a pipe-EOF race. setsid keeps the process group intact so
# terminate_child's `kill -TERM -- "-$pid"` reaches the whole tree.
setsid "$timeout_bin" --signal=TERM --kill-after=0.1 "$watchdog_seconds" \
  env NO_COLOR=1 RUST_LOG=pikr=debug "$pikr_bin" \
  --dmenu --filter ban <<< $'apple\nbanana\ncherry' \
  >"$out_file" 2>"$err_file" &
pid=$!
focus_line=""
focus_pattern='^[^[:space:]]+[[:space:]]+DEBUG[[:space:]]+pikr::ui::view:[[:space:]]+startup first focus received[[:space:]]+elapsed_us=[0-9]+$'
focus_attempts="${PIKR_FOCUS_ATTEMPTS:-$default_focus_attempts}"

for ((attempt = 0; attempt < focus_attempts; attempt++)); do
  if focus_line=$(grep -m1 -E "$focus_pattern" "$err_file"); then
    break
  elif ! kill -0 "$pid" 2>/dev/null; then
    break
  fi
  # Headless-compositor nudge: sway's headless backend has no input devices,
  # so a layer-shell surface never receives keyboard focus until some client
  # connects. Send one sacrificial F12 (unmapped in pikr, inert to other apps)
  # once, early, to trigger the seat — the focus-marker gate below still
  # applies. In a real session pikr is focused within a few attempts and the
  # nudge never fires.
  if ((attempt == 10)) && [[ -z "$focus_line" ]]; then
    "$timeout_bin" --signal=TERM --kill-after=0.1 2 \
      "$wtype_bin" -k F12 >/dev/null 2>&1 || true
  fi
  sleep 0.01
done

if [[ -z "$focus_line" ]]; then
  printf 'FAIL: Pikr did not report an authentic first-focus event before the probe timeout\n' >&2
  terminate_child
  cat "$err_file" >&2
  cat "$out_file" >&2
  exit 1
fi

focus_us=$(printf '%s\n' "$focus_line" | grep -oE 'elapsed_us=[0-9]+' | cut -d= -f2)
if ((focus_us > deadline_us)); then
  printf 'FAIL: first focus took %s us, exceeding the %s us deadline\n' \
    "$focus_us" "$deadline_us" >&2
  terminate_child
  cat "$err_file" >&2
  cat "$out_file" >&2
  exit 1
fi

if ! "$timeout_bin" --signal=TERM --kill-after=0.1 2 \
  "$wtype_bin" -k F12 -k Return; then
  printf 'FAIL: wtype could not send the acceptance keys after Pikr received focus\n' >&2
  terminate_child
  cat "$err_file" >&2
  cat "$out_file" >&2
  exit 1
fi

wait "$pid"
status=$?
pid=""
e2e_ms=$(( $(date +%s%3N) - e2e_start_ms ))

result=""
if grep -qx 'banana' "$out_file"; then
  result="banana"
fi

if [[ "$status" -eq 0 && "$result" == "banana" ]]; then
  if ((e2e_ms > e2e_deadline_ms)); then
    printf 'FAIL: end-to-end launch-to-exit took %s ms, exceeding the %s ms deadline\n' \
      "$e2e_ms" "$e2e_deadline_ms" >&2
    terminate_child
    cat "$err_file" >&2
    cat "$out_file" >&2
    exit 1
  fi
  printf 'PASS: first focus at %s us met the %s us deadline; accepted "banana" in %s ms end-to-end (deadline %s ms)\n' \
    "$focus_us" "$deadline_us" "$e2e_ms" "$e2e_deadline_ms"
  grep -E 'startup (config loaded|entries collected|state ranked|first paint pass reached|first focus received)' "$err_file"
  exit 0
fi

printf 'FAIL: expected exit status 0 and output "banana"; got status %s and output %q\n' \
  "$status" "$result" >&2
cat "$err_file" >&2
cat "$out_file" >&2
exit 1
