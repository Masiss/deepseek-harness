#!/usr/bin/env bash
# Measure the desktop application under one WebKit configuration.
#
# The probe boots the release shell on a headless compositor, waits until the
# webview says it is authorized, and reports what that configuration costs:
# the resident set of the whole process tree, the frame rate the page
# received, the weight of the resources it loaded, and a picture of the
# window. Comparing two runs answers whether a setting costs graphics.
#
#   usage: graphics-probe.sh [label] [linux-x64|linux-arm64]
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
label=${1:-default}
target=${2:-linux-x64}
tree="$repo/apps/desktop/.desktop-build/targets/$target"
shell="$repo/apps/desktop-tauri/src-tauri/target/release/dsh-desktop-tauri"
[ -x "$shell" ] || { echo "RESULT shell=missing"; exit 1; }
[ -x "$tree/bin/desktop-host" ] || { echo "RESULT host=missing"; exit 1; }
# shellcheck source=probe-lib.sh
. "$repo/apps/desktop-tauri/scripts/probe-lib.sh"

work=$(mktemp -d /tmp/dsh-graphics.XXXXXX)
mkdir -p "$work/run" "$work/home" "$work/tmp"
chmod 700 "$work/run"
cp -a "$tree/project" "$work/home/desktop-project"

XDG_RUNTIME_DIR="$work/run" weston --backend=headless-backend.so --width=1280 --height=860 \
  --no-config > "$work/weston.log" 2>&1 &
weston_pid=$!
shell_pid=""
cleanup() {
  kill -9 "$weston_pid" 2>/dev/null
  kill_tree "$shell_pid"
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
  DSH_DESKTOP_WEB_CONSOLE=1 \
  DSH_DESKTOP_WEBKIT_LIGHT="${DSH_DESKTOP_WEBKIT_LIGHT:-1}" \
  DSH_DESKTOP_WEBKIT_DMABUF="${DSH_DESKTOP_WEBKIT_DMABUF:-1}" \
  DSH_DESKTOP_WEBKIT_ACCEL="${DSH_DESKTOP_WEBKIT_ACCEL:-1}" \
  DSH_DESKTOP_WEBKIT_CACHE="${DSH_DESKTOP_WEBKIT_CACHE:-1}" \
  DSH_DESKTOP_WEBKIT_WEBGL="${DSH_DESKTOP_WEBKIT_WEBGL:-1}" \
  DSH_DESKTOP_WEBVIEW_SNAPSHOT="$work/shot.png" \
  DSH_DESKTOP_NODE_FLAGS="${DSH_DESKTOP_NODE_FLAGS:---optimize-for-size}" \
  "$shell" > "$work/out" 2> "$work/err" &
shell_pid=$!

# The webview has to be authorized before the measurement means anything.
auth=none
for _ in $(seq 1 600); do
  auth=$(grep -ho 'dsh: webview-auth=[A-Za-z0-9]*' "$work/out" "$work/err" 2>/dev/null | tail -1)
  [ "$auth" = "dsh: webview-auth=200" ] && break
  kill -0 "$shell_pid" 2>/dev/null || break
  sleep 0.25
done
fps=none
resources=none
for _ in $(seq 1 200); do
  found=$(grep -ho 'dsh: webview-fps=[0-9]*' "$work/out" "$work/err" 2>/dev/null | tail -1)
  [ -n "$found" ] && fps=$found
  found=$(grep -ho 'dsh: webview-resources=[0-9]* bytes=[0-9]*' "$work/out" "$work/err" 2>/dev/null | tail -1)
  [ -n "$found" ] && resources=$found
  [ "$fps" != "none" ] && [ "$resources" != "none" ] && break
  sleep 0.25
done
sleep 5

memory=$(python3 - "$shell_pid" <<'PY'
import subprocess, sys
root = int(sys.argv[1])
rows = {}
for line in subprocess.check_output(['ps', '-eo', 'pid=,ppid=,rss=,comm='], text=True).splitlines():
    pid, ppid, rss, comm = line.split(None, 3)
    rows[int(pid)] = (int(ppid), int(rss), comm)
children = {}
for pid, (ppid, _, _) in rows.items():
    children.setdefault(ppid, []).append(pid)

def proportional(pid):
    try:
        with open(f'/proc/{pid}/smaps_rollup') as rollup:
            for line in rollup:
                if line.startswith('Pss:'):
                    return int(line.split()[1])
    except OSError:
        pass
    return 0

stack = [root]
seen = set()
total = 0
shared = 0
parts = []
while stack:
    pid = stack.pop()
    if pid in seen:
        continue
    seen.add(pid)
    _, rss, comm = rows.get(pid, (0, 0, '?'))
    total += rss
    shared += proportional(pid)
    parts.append((rss, pid, comm))
    stack.extend(children.get(pid, []))
parts.sort(reverse=True)
print(f'{total / 1024:.1f} {shared / 1024:.1f}')
for rss, pid, comm in parts:
    print(f'{rss / 1024:.1f} {comm}')
PY
)
rss_total=$(printf '%s' "$memory" | head -1 | cut -d' ' -f1)
pss_total=$(printf '%s' "$memory" | head -1 | cut -d' ' -f2)

# The picture comes from the webview itself, so it shows the application and
# not whatever else the session had in front.
shot="$work/shot.png"

echo "RESULT label=$label rss_mib=$rss_total pss_mib=$pss_total fps=${fps#dsh: webview-fps=} resources=${resources#dsh: webview-resources=} shot=$shot"
printf '%s' "$memory" | tail -n +2 | while read -r rss comm; do
  echo "RSS $rss MiB $comm"
done
