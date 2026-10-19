#!/usr/bin/env bash
# P1 shell probe: single instance, window state, and host liveness.
set -u

repo=/home/masis/deepseek-harness
bin="$repo/apps/desktop-tauri/src-tauri/target/release/dsh-desktop-tauri"
work=$(mktemp -d /tmp/opencode/shell.XXXXXX)

export DSH_DESKTOP_HOST_BIN="$repo/apps/desktop-tauri/scripts/dev-host.sh"
export DSH_DESKTOP_RUNTIME_DIR="$HOME/.dsh/profiles"
export DSH_DESKTOP_PROJECT_DIR="$HOME/.dsh/profiles/web"

"$bin" > "$work/first.out" 2> "$work/first.err" &
first=$!

port=""
for _ in $(seq 1 80); do
  port=$(grep -o '127\.0\.0\.1:[0-9]*' "$work/first.err" 2>/dev/null | head -1 | cut -d: -f2)
  [ -n "$port" ] && break
  sleep 0.25
done
echo "first_pid=$first host_port=${port:-none}"

sleep 4
echo "first_alive_before_second=$(kill -0 "$first" 2>/dev/null && echo yes || echo no)"

# Second launch must hand off to the first instance and exit on its own.
echo "=== second launch ==="
second_start=$(date +%s%N)
"$bin" > "$work/second.out" 2> "$work/second.err" &
second=$!
for _ in $(seq 1 60); do
  kill -0 "$second" 2>/dev/null || break
  sleep 0.25
done
second_end=$(date +%s%N)
second_exit=$(kill -0 "$second" 2>/dev/null && echo "still-running" || echo "exited")
echo "second=$second_exit second_lifetime_ms=$(( (second_end - second_start) / 1000000 ))"
echo "first_alive_after_second=$(kill -0 "$first" 2>/dev/null && echo yes || echo no)"
echo "host_still_serving=$(ss -tn 2>/dev/null | grep -c ":${port}")"

echo "=== window state files ==="
find "$HOME/.config" -name '*window-state*' -newermt '-10 minutes' 2>/dev/null | head -5
find "$HOME/.local/share" -name '*window-state*' -newermt '-10 minutes' 2>/dev/null | head -5

echo "=== tray / plugin errors ==="
grep -iE "tray|single-instance|window-state|panic" "$work/first.err" "$work/second.err" 2>/dev/null | head -8

kill "$first" 2>/dev/null
kill "$second" 2>/dev/null
wait "$first" 2>/dev/null
wait "$second" 2>/dev/null
echo "work=$work"
