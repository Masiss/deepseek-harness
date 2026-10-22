#!/usr/bin/env bash
# Reproduce the shell's silence when the Host dies after it was ready.
#
# The reader stops at the end of the Host's output. When the Host had already
# reported readiness, nothing is reported to the shell, and the readiness flag
# stays set, so the startup watchdog sees a healthy Host and the window keeps
# showing the page the dead Host served.
#
#   usage: repro-host-death-after-ready.sh [linux-x64|linux-arm64]
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../../.." && pwd)
target=${1:-linux-x64}
tree="$repo/apps/desktop/.desktop-build/targets/$target"
shell="$repo/apps/desktop-tauri/src-tauri/target/release/dsh-desktop-tauri"
work=$(mktemp -d /tmp/dsh-repro-host-death.XXXXXX)
mkdir -p "$work/run" "$work/home" "$work/tmp"
chmod 700 "$work/run"
cp -a "$tree/project" "$work/home/desktop-project"

XDG_RUNTIME_DIR="$work/run" weston --backend=headless-backend.so --width=1280 --height=860 \
  --no-config > "$work/weston.log" 2>&1 &
weston_pid=$!
cleanup() {
  kill -9 "$weston_pid" 2>/dev/null
  [ -n "${shell_pid:-}" ] && kill -9 "$shell_pid" 2>/dev/null
  true
}
trap cleanup EXIT

display=""
for _ in $(seq 1 100); do
  display=$(find "$work/run" -maxdepth 1 -name 'wayland-*' -type s 2>/dev/null | head -1)
  [ -n "$display" ] && break
  sleep 0.1
done
display=${display##*/}

env -i PATH=/usr/bin:/bin HOME="$work/home" TMPDIR="$work/tmp" \
  XDG_RUNTIME_DIR="$work/run" WAYLAND_DISPLAY="$display" \
  DSH_HOME="$work/home/.dsh" \
  DSH_DESKTOP_HOST_BIN="$tree/bin/desktop-host" \
  DSH_DESKTOP_RUNTIME_DIR="$tree/dsh" \
  DSH_DESKTOP_PROJECT_DIR="$work/home/desktop-project" \
  DSH_DESKTOP_PRIMARY_RUNTIME="$tree/runtime/primary-runtime" \
  DSH_DESKTOP_PNPM_ENTRY="$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" \
  DSH_DESKTOP_NODE_BIN="$tree/runtime/bin" \
  DSH_DESKTOP_VERSION=1.0.0 DSH_CLIENT_VERSION=1.0.0 \
  "$shell" > "$work/out" 2> "$work/err" &
shell_pid=$!

port=""
for _ in $(seq 1 240); do
  port=$(grep -ho 'http://127\.0\.0\.1:[0-9]*' "$work/err" | head -1 | grep -o '[0-9]*$')
  [ -n "$port" ] && break
  sleep 0.25
done
[ -n "$port" ] || { echo 'RESULT repro=FAIL reason=host-not-ready'; exit 1; }

# The Host is the shell's child that runs the runtime.
host_pid=""
for _ in $(seq 1 60); do
  host_pid=$(ps -eo pid=,ppid=,comm= | awk -v shell="$shell_pid" '$2==shell && $3=="MainThread" {print $1}' | head -1)
  [ -n "$host_pid" ] && break
  sleep 0.25
done
[ -n "$host_pid" ] || { echo 'RESULT repro=FAIL reason=host-not-found'; exit 1; }

before=$(grep -c '"event"' "$work/err" || true)
kill -9 "$host_pid"
sleep 4

after=$(grep -c '"event"' "$work/err" || true)
kill -0 "$shell_pid" 2>/dev/null && alive=yes || alive=no
reported=$(grep -c 'desktop shell: the Host\|the Host stopped\|the Host failed' "$work/err" || true)

echo "RESULT host-died shell_alive=$alive milestones_before=$before milestones_after=$after failure_reports=$reported"
if [ "$alive" = yes ] && [ "$reported" -eq 0 ]; then
  echo 'RESULT repro=host-death CONFIRMED: the shell stays silent and keeps the dead page'
  exit 0
fi
echo 'RESULT repro=host-death REFUTED: the shell reported the failure'
exit 1
