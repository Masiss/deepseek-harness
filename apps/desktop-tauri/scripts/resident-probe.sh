#!/usr/bin/env bash
# Measure the resident set of the whole desktop application at idle.
#
# `rss-probe.sh` covers the Host alone. A desktop user pays for the shell and
# its WebKit processes too, so this probe boots the release shell on a
# headless compositor, waits for the served application to finish loading and
# settle, then sums the resident set of the shell's process tree.
#
# RSS counts shared WebKit mappings once per process, so it bounds the cost
# from above. The probe also sums PSS from `smaps_rollup`, which apportions
# each shared page across its users and is the fairer idle figure. The
# per-process table shows where the memory goes.
#
#   usage: resident-probe.sh [linux-x64|linux-arm64] [settle-seconds]
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
target=${1:-linux-x64}
settle=${2:-10}
tree="$repo/apps/desktop/.desktop-build/targets/$target"
shell="$repo/apps/desktop-tauri/src-tauri/target/release/dsh-desktop-tauri"
[ -x "$shell" ] || { echo "RESULT shell=missing"; exit 1; }
[ -x "$tree/bin/desktop-host" ] || { echo "RESULT host=missing"; exit 1; }

work=${DSH_RESIDENT_WORK:-$(mktemp -d /tmp/dsh-resident.XXXXXX)}
mkdir -p "$work/run" "$work/home" "$work/tmp"
chmod 700 "$work/run"

XDG_RUNTIME_DIR="$work/run" weston --backend=headless-backend.so --width=1280 --height=860 \
  --no-config > "$work/weston.log" 2>&1 &
weston_pid=$!
cleanup() {
  kill -9 "$weston_pid" 2>/dev/null
  kill_tree "$shell_pid" 2>/dev/null
  true
}
trap cleanup EXIT

# Stop only the shell's descendants, so a probe failure cannot reach the
# user's session applications.
# shellcheck source=probe-lib.sh
. "$repo/apps/desktop-tauri/scripts/probe-lib.sh"

display=""
for _ in $(seq 1 100); do
  display=$(find "$work/run" -maxdepth 1 -name 'wayland-*' -type s 2>/dev/null | head -1)
  [ -n "$display" ] && break
  sleep 0.1
done
[ -n "$display" ] || { echo "RESULT compositor=FAIL"; tail -5 "$work/weston.log"; exit 1; }
display=${display##*/}

profile="$work/home/desktop-project"
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
  DSH_DESKTOP_WEB_CONSOLE=1 \
  DSH_DESKTOP_WEBKIT_LIGHT="${DSH_DESKTOP_WEBKIT_LIGHT:-1}" \
  "$shell" > "$work/out" 2> "$work/err" &
shell_pid=$!

# The shell loads the start page, then the Host's page; the number of
# finished loads therefore settles once the application is on screen. Waiting
# for a fixed count would stop at the Host's first response instead.
auth=none
for _ in $(seq 1 600); do
  auth=$(grep -ho 'dsh: webview-auth=[A-Za-z0-9]*' "$work/out" "$work/err" 2>/dev/null | tail -1)
  [ "$auth" = "dsh: webview-auth=200" ] && break
  kill -0 "$shell_pid" 2>/dev/null || break
  sleep 0.25
done
sleep "$settle"

echo "RESULT webview_auth=$auth"
if [ "$auth" != "dsh: webview-auth=200" ]; then
  echo "RESULT resident=FAIL reason=webview-not-authorized"
fi

python3 - "$shell_pid" <<'PY'
import os, subprocess, sys, time
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

def cpu_ticks(pids):
    ticks = 0
    for pid in pids:
        try:
            with open(f'/proc/{pid}/stat') as stat:
                fields = stat.read().rsplit(')', 1)[1].split()
        except OSError:
            continue
        ticks += int(fields[11]) + int(fields[12])
    return ticks

def usage(pids):
    used = 0
    for pid in pids:
        try:
            with open(f'/proc/{pid}/stat') as stat:
                fields = stat.read().rsplit(')', 1)[1].split()
        except OSError:
            continue
        used += int(fields[11]) + int(fields[12])
    return used

pids = sorted(seen)
before = usage(pids)
started = time.monotonic()
time.sleep(5)
after = usage(pids)
elapsed = time.monotonic() - started
hz = os.sysconf('SC_CLK_TCK')
percent = (after - before) / hz / elapsed * 100 if elapsed > 0 else 0.0
print(f'RESULT cpu_idle_percent={percent:.1f} window_seconds={elapsed:.1f} processes={len(pids)}')
print(f'RESULT resident=OK total_mib={total / 1024:.1f} pss_mib={shared / 1024:.1f} processes={len(parts)}')
for rss, pid, comm in parts:
    print(f'RSS {rss / 1024:7.1f} MiB {pid:8d} {comm}')
PY

kill_tree "$shell_pid"
for _ in $(seq 1 50); do kill -0 "$shell_pid" 2>/dev/null || break; sleep 0.1; done
