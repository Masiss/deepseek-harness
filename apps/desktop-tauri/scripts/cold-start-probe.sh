#!/usr/bin/env bash
# Measure the desktop shell's cold start in a headless compositor.
#
# The shell reports its startup milestones as NDJSON on stderr: the Host URL
# arrived, the window was created, and the page finished loading. The probe
# runs the shell against a headless weston compositor, so no window appears on
# the user's session, and reports the elapsed milliseconds.
#
# The last milestone is the page's load finishing, which is the first moment
# the application content is on screen: it bounds the first paint from above.
#
#   usage: cold-start-probe.sh [linux-x64|linux-arm64]
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
target=${1:-linux-x64}
tree="$repo/apps/desktop/.desktop-build/targets/$target"
shell="$repo/apps/desktop-tauri/src-tauri/target/release/dsh-desktop-tauri"
[ -x "$shell" ] || { echo "RESULT shell=missing"; exit 1; }
[ -x "$tree/bin/desktop-host" ] || { echo "RESULT host=missing"; exit 1; }

# The compositor socket path is short by requirement, so the work directory
# lives in the private temporary space of the sandbox.
work=${DSH_COLD_START_WORK:-/tmp/dsh-cold-start}
rm -rf "$work"
mkdir -p "$work/run" "$work/home" "$work/tmp"
chmod 700 "$work/run"

# A headless compositor keeps the webview real without touching the session.
XDG_RUNTIME_DIR="$work/run" weston --backend=headless-backend.so --width=1280 --height=860 \
  --no-config > "$work/weston.log" 2>&1 &
weston_pid=$!
cleanup() {
  kill -9 "$weston_pid" 2>/dev/null
  ps -eo pid,args --no-headers | grep -F "release/dsh-desktop-tauri" | grep -v grep | awk '{print $1}' \
    | while read -r pid; do kill -9 "$pid" 2>/dev/null; done
  true
}
trap cleanup EXIT

display=""
for _ in $(seq 1 100); do
  display=$(find "$work/run" -maxdepth 1 -name 'wayland-*' -type s 2>/dev/null | head -1)
  [ -n "$display" ] && break
  sleep 0.1
done
[ -n "$display" ] || { echo "RESULT compositor=FAIL"; tail -5 "$work/weston.log"; exit 1; }
display=${display##*/}

# A seeded profile keeps the measurement on the steady state: the profile is
# copied in, as the first run of a real installation would have it.
profile="$work/home/desktop-project"
mkdir -p "$work/home"
cp -a "$tree/project" "$profile"

env -i PATH=/usr/bin:/bin HOME="$work/home" TMPDIR="$work/tmp" \
  XDG_RUNTIME_DIR="$work/run" WAYLAND_DISPLAY="$display" \
  DSH_HOME="$work/home/.dsh" \
  DSH_DESKTOP_HOST_BIN="$tree/bin/desktop-host" \
  DSH_DESKTOP_RUNTIME_DIR="$tree/dsh" \
  DSH_DESKTOP_PROJECT_DIR="$profile" \
  DSH_DESKTOP_PRIMARY_RUNTIME="$tree/runtime/primary-runtime" \
  DSH_DESKTOP_PNPM_ENTRY="$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" \
  DSH_DESKTOP_NODE_BIN="$tree/runtime/bin" \
  DSH_DESKTOP_VERSION=1.0.0 DSH_CLIENT_VERSION=1.0.0 \
  "$shell" > "$work/out" 2> "$work/err" &
shell_pid=$!

for _ in $(seq 1 300); do
  grep -q '"event":"page-load-finished"' "$work/err" 2>/dev/null && break
  kill -0 "$shell_pid" 2>/dev/null || break
  sleep 0.1
done

echo "RESULT milestones=$(grep -c '"event"' "$work/err" 2>/dev/null || echo 0)"
# The application the Host serves must answer on the URL the shell navigated
# to; the start page alone would not prove the desktop lane works.
url=$(grep -o 'http://127\.0\.0\.1:[0-9]*/?token=[A-Za-z0-9_-]*' "$work/err" 2>/dev/null | head -1)
base=$(printf '%s' "$url" | sed -E 's#.*:([0-9]+)/.*#http://127.0.0.1:\1#')
if [ -n "$url" ]; then
  curl -s -L -c "$work/cookies" -b "$work/cookies" -o /dev/null "$url"
  echo "RESULT bridge=$(curl -s -m 20 -b "$work/cookies" -H 'content-type: application/json' -X POST "$base/__dsh-desktop/updates-status" -d null 2>/dev/null | head -c 120)"
fi
grep -o '{"event":"[^"]*","ms":[0-9]*}' "$work/err" 2>/dev/null | while read -r line; do
  echo "MILESTONE $line"
done
if grep -q '"event":"page-load-finished"' "$work/err" 2>/dev/null; then
  echo "RESULT cold_start=OK"
else
  echo "RESULT cold_start=FAIL"
  tail -5 "$work/err"
  exit 1
fi
