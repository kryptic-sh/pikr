#!/usr/bin/env bash

set -uo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/test-startup-readiness.sh [--delay SECONDS]

Launch the release Pikr binary in dmenu mode, require its monotonic first-focus
marker to fall within the chosen deadline, then verify that it accepts F12 and
Return and emits the matching candidate.

Options:
  --delay SECONDS  First-focus deadline in seconds (default: 0.500)
  -h, --help       Show this help

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
watchdog_seconds=$(awk -v delay="$delay" 'BEGIN { printf "%.3f", delay + 2 }')
default_focus_attempts=$(awk -v delay="$delay" 'BEGIN { printf "%.0f", (delay + 1) * 100 }')
pid=""

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
}

trap cleanup EXIT
trap 'exit 130' INT TERM HUP

printf 'Testing first focus within %s seconds, then candidate acceptance...\n' "$delay"
coproc PIKR_PROCESS {
  exec setsid "$timeout_bin" --signal=TERM --kill-after=0.1 "$watchdog_seconds" \
    env NO_COLOR=1 RUST_LOG=pikr=debug "$pikr_bin" \
    --dmenu --filter ban <<< $'apple\nbanana\ncherry' \
    2> >(while IFS= read -r line; do printf 'STDERR\t%s\n' "$line"; done) \
    > >(while IFS= read -r line; do printf 'STDOUT\t%s\n' "$line"; done)
}
pid=$PIKR_PROCESS_PID
output_fd=${PIKR_PROCESS[0]}
declare -a output_lines=()
focus_line=""
focus_pattern='^STDERR[[:space:]][^[:space:]]+[[:space:]]+DEBUG[[:space:]]+pikr::ui::view:[[:space:]]+startup first focus received[[:space:]]+elapsed_us=[0-9]+$'
focus_attempts="${PIKR_FOCUS_ATTEMPTS:-$default_focus_attempts}"

for ((attempt = 0; attempt < focus_attempts; attempt++)); do
  if IFS= read -r -t 0.01 line <&"$output_fd"; then
    output_lines+=("$line")
    if [[ "$line" =~ $focus_pattern ]]; then
      focus_line="$line"
      break
    fi
  elif ! kill -0 "$pid" 2>/dev/null; then
    break
  fi
done

if [[ -z "$focus_line" ]]; then
  printf 'FAIL: Pikr did not report an authentic first-focus event before the probe timeout\n' >&2
  terminate_child
  printf '%s\n' "${output_lines[@]}" >&2
  exit 1
fi

focus_us=$(printf '%s\n' "$focus_line" | grep -oE 'elapsed_us=[0-9]+' | cut -d= -f2)
if ((focus_us > deadline_us)); then
  printf 'FAIL: first focus took %s us, exceeding the %s us deadline\n' \
    "$focus_us" "$deadline_us" >&2
  terminate_child
  printf '%s\n' "${output_lines[@]}" >&2
  exit 1
fi

if ! "$timeout_bin" --signal=TERM --kill-after=0.1 2 \
  "$wtype_bin" -k F12 -k Return; then
  printf 'FAIL: wtype could not send the acceptance keys after Pikr received focus\n' >&2
  terminate_child
  printf '%s\n' "${output_lines[@]}" >&2
  exit 1
fi

while IFS= read -r line <&"$output_fd"; do
  output_lines+=("$line")
done
wait "$pid"
status=$?
pid=""

result=""
for line in "${output_lines[@]}"; do
  if [[ "$line" == $'STDOUT\tbanana' ]]; then
    result="banana"
  fi
done

if [[ "$status" -eq 0 && "$result" == "banana" ]]; then
  printf 'PASS: first focus at %s us met the %s us deadline; accepted "banana"\n' \
    "$focus_us" "$deadline_us"
  for line in "${output_lines[@]}"; do
    if [[ "$line" == *"startup config loaded"* ||
      "$line" == *"startup entries collected"* ||
      "$line" == *"startup state ranked"* ||
      "$line" == *"startup first paint pass reached"* ||
      "$line" == *"startup first focus received"* ]]; then
      printf '%s\n' "${line#*$'\t'}"
    fi
  done
  exit 0
fi

printf 'FAIL: expected exit status 0 and output "banana"; got status %s and output %q\n' \
  "$status" "$result" >&2
printf '%s\n' "${output_lines[@]}" >&2
exit 1
