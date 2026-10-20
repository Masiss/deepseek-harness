#!/usr/bin/env bash
# Measure the idle resident set of the packaged Host.
#
# The WebView is a separate process tree owned by WebKitGTK; this probe covers
# the Host, which is what the shell's launcher starts and what the desktop
# bundle controls. It boots the packaged tree headlessly, lets the profile and
# plugins settle, then samples the process RSS. Run it under isolated-run.sh.
#
#   usage: DSH_ISOLATION_DIR=... isolated-run.sh bash rss-probe.sh [settle-seconds]
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
target=${DSH_DESKTOP_TARGET:-linux-x64}
tree=${DSH_OFFICE_ROOT:-$repo/apps/desktop/.desktop-build/targets/$target}
settle=${1:-15}
work=${DSH_ISOLATION_DIR:?set DSH_ISOLATION_DIR}
home="$work/home/.dsh"

mkdir -p "$home" "$work/tmp"
rm -rf "$home/desktop-project"
cp -a "$tree/project" "$home/desktop-project"
[ -p "$work/in" ] || mkfifo "$work/in"

cat > "$work/run.sh" <<EOF
#!/bin/sh
exec env -i PATH=/usr/bin:/bin HOME="$work/home" TMPDIR="$work/tmp" \\
  DSH_HOME="$home" DSH_DESKTOP_RUNTIME_DIR="$tree/dsh" DSH_DESKTOP_PROJECT_DIR="$home/desktop-project" \\
  DSH_DESKTOP_PRIMARY_RUNTIME="$tree/runtime/primary-runtime" \\
  DSH_DESKTOP_PNPM_ENTRY="$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" \\
  DSH_DESKTOP_NODE_BIN="$tree/runtime/bin" DSH_DESKTOP_VERSION=1.0.0 \\
  ${DSH_DESKTOP_NODE_FLAGS+D外地} \\
  "$tree/bin/desktop-host" "$tree/dsh" "$home/desktop-project" "$tree/runtime/primary-runtime" \\
  "$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" "$tree/runtime/bin"
EOF
chmod +x "$work/run.sh"
setsid "$work/run.sh" < "$work/in" > "$work/out" 2> "$work/err" &
sleep 1
pgid=$(ps -eo pid,args | grep -F "$work/run.sh" | grep -v grep | awk '{print $1}' | head -1)
exec 3> "$work/in"
for _ in $(seq 1 160); do grep -q 'http://127.0.0.1' "$work/out" 2>/dev/null && break; sleep 0.5; done
sleep "$settle"
echo "RESULT node_flags=$(ps -eo args | grep -F 'dsh-desktop-host/lib/tauri.js' | grep -v grep | head -1 | grep -o -- '--[a-z-]*' | tr '\n' ' ')"
ps -eo pid,rss,args --sort=-rss | grep -F "dsh-desktop-host/lib/tauri.js" | grep -v grep | sed -E 's/^ *([0-9]+) +([0-9]+).*/RESULT host_rss_kib=\2 pid=\1/'
printf '@@DSH@@{"type":"shutdown"}\n' >&3
exec 3>&-
for _ in $(seq 1 20); do kill -0 "$pgid" 2>/dev/null || break; sleep 0.5; done
kill -KILL -- "-$pgid" 2>/dev/null
echo "RESULT done=1"
