#!/usr/bin/env bash
# Launch the native egui app on the packaged Linux runtime and report measured memory.
#
# Requires the native release binary and apps/desktop/.desktop-build/targets/linux-x64.
# The app runs against an isolated DSH_HOME so it never touches the operator's profile,
# and auto-connects to the Host sidecar so the probe can observe authentication.
#
# Usage: native-gui-probe.sh [--no-measure]
# Output: RESULT markers on stdout; app stdout/stderr in the work directory.
set -euo pipefail

measure=true
case "${1:-}" in
  '') ;;
  --no-measure) measure=false ;;
  *) echo "usage: $0 [--no-measure]" >&2; exit 2 ;;
esac

root=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
bin="$root/apps/desktop-tauri/native-app/target/release/dsh-desktop-native-app"
tree="$root/apps/desktop/.desktop-build/targets/linux-x64"
host_package="$tree/dsh/node_modules/@deepseek-ai/dsh-desktop-host/package.json"
for required in "$bin" "$tree/bin/desktop-host" "$tree/runtime/bin/node" \
  "$tree/runtime/primary-runtime/dependencies/node/bin/node" \
  "$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" "$host_package" \
  "$tree/project/package.json" "$tree/project/cordis.yml"; do
  [ -e "$required" ] || { echo "native-gui-probe: missing input: $required" >&2; exit 1; }
done
[ -x "$bin" ] || { echo "native-gui-probe: not executable: $bin" >&2; exit 1; }
case "$(uname -m)" in
  x86_64|aarch64) ;;
  *) echo "native-gui-probe: unsupported architecture $(uname -m)" >&2; exit 1 ;;
esac
if [ -z "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ]; then
  echo "native-gui-probe: no graphical session (WAYLAND_DISPLAY/DISPLAY unset)" >&2
  echo "RESULT native-gui=SKIP reason=no-display"
  exit 0
fi

version=$("$tree/runtime/bin/node" --input-type=module -e \
  'import { readFileSync } from "node:fs"; const { version } = JSON.parse(readFileSync(process.argv[1], "utf8")); if (typeof version !== "string" || !/^[0-9A-Za-z.+-]+$/.test(version)) throw new Error("invalid packaged Host version"); console.log(version)' \
  "$host_package")

work_root="${DSH_PROBE_TMPDIR:-$HOME/.cache/dsh-probe}"
mkdir -p "$work_root"
work=$(mktemp -d "$work_root/native-gui.XXXXXX")
home="$work/home"
project="$work/project"
mkdir -p "$home"
cp -a "$tree/project" "$project"
chmod -R u+w "$project"
out="$work/app.out"
err="$work/app.err"
pidfile="$work/app.pid"

cleanup() {
  local pid
  pid=$(cat "$pidfile" 2>/dev/null || true)
  if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then
    kill -TERM "$pid" 2>/dev/null || true
    for _ in $(seq 1 100); do
      kill -0 "$pid" 2>/dev/null || break
      sleep 0.1
    done
    kill -KILL "$pid" 2>/dev/null || true
  fi
  # Host children keep the same process group; terminate stragglers by pid only.
  local child
  for child in $(cardinal_children "$pid" 2>/dev/null || true); do
    kill -TERM "$child" 2>/dev/null || true
  done
}
trap cleanup EXIT

launcher="$work/launch.sh"
cat >"$launcher" <<LAUNCH
#!/bin/sh
export DSH_HOME='$home'
export DSH_DESKTOP_AUTO_CONNECT='1'
export DSH_DESKTOP_PROBE_STREAMS='1'
export DSH_DESKTOP_HOST_BIN='$tree/bin/desktop-host'
export DSH_DESKTOP_RUNTIME_DIR='$tree/dsh'
export DSH_DESKTOP_PROJECT_DIR='$project'
export DSH_DESKTOP_PRIMARY_RUNTIME='$tree/runtime/primary-runtime'
export DSH_DESKTOP_PNPM_ENTRY='$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs'
export DSH_DESKTOP_NODE_BIN='$tree/runtime/bin'
export DSH_DESKTOP_VERSION='$version'
export DSH_CLIENT_VERSION='$version'
export TMPDIR='$work'
exec '$bin' >'$out' 2>'$err'
LAUNCH
chmod +x "$launcher"

"$launcher" &
pid=$!
echo "$pid" >"$pidfile"

wait_for() {
  local marker=$1 seconds=$2
  for _ in $(seq 1 $((seconds * 10))); do
    grep -qF "$marker" "$err" 2>/dev/null && return 0
    kill -0 "$pid" 2>/dev/null || return 1
    sleep 0.1
  done
  return 1
}

if ! wait_for 'dsh: native host=ready' 120; then
  echo "native-gui-probe: Host did not become ready" >&2
  tail -20 "$err" >&2 || true
  echo "RESULT native-gui=FAIL stage=host"
  exit 1
fi
echo "RESULT native-host=ready"
if ! wait_for 'dsh: native auth=ok' 60; then
  echo "native-gui-probe: authentication did not complete" >&2
  tail -20 "$err" >&2 || true
  echo "RESULT native-gui=FAIL stage=auth"
  exit 1
fi
echo "RESULT native-auth=ok"
if wait_for 'dsh: native events=ready' 40; then
  echo "RESULT native-events=ready"
else
  echo "RESULT native-events=FAIL"
fi
# The stream probe runs once the session list is available; give it its own budget.
wait_for 'dsh: native stream job/list=' 60 || true
sleep 1
while read -r line; do
  echo "RESULT ${line#dsh: native stream }"
done < <(grep -F 'dsh: native stream ' "$err" || true)
if grep -F 'dsh: native stream ' "$err" | grep -q 'failed'; then
  echo "RESULT native-streams=FAIL"
else
  echo "RESULT native-streams=OK"
fi
wait_for 'dsh: native plugins=' 40 || true
if grep -F 'dsh: native plugins=' "$err" | grep -q 'failed'; then
  echo "RESULT native-plugins=FAIL"
elif grep -F 'dsh: native plugins=' "$err" >/dev/null; then
  echo "RESULT native-plugins=OK"
else
  echo "RESULT native-plugins=FAIL"
fi
wait_for 'dsh: native models=' 40 || true
if grep -F 'dsh: native models=' "$err" | grep -q 'failed'; then
  echo "RESULT native-models=FAIL"
elif grep -F 'dsh: native models=' "$err" >/dev/null; then
  echo "RESULT native-models=OK"
else
  echo "RESULT native-models=FAIL"
fi

# Let the session list settle before measuring steady-state memory.
sleep 3

cardinal_children() {
  local parent=${1:-}
  [ -n "$parent" ] || return 0
  local task
  for task in "/proc/$parent/task/"*; do
    [ -r "$task/children" ] || continue
    cat "$task/children" 2>/dev/null
    printf '\n'
  done | tr ' ' '\n' | grep -E '^[0-9]+$' | sort -u
}

tree_pids() {
  local root_pid=$1
  printf '%s\n' "$root_pid"
  local queue=("$root_pid") next
  while [ "${#queue[@]}" -gt 0 ]; do
    next=${queue[0]}
    queue=("${queue[@]:1}")
    local child
    for child in $(cardinal_children "$next"); do
      printf '%s\n' "$child"
      queue+=("$child")
    done
  done
}

if $measure; then
  rss_kib=0
  pss_kib=0
  while read -r member; do
    [ -r "/proc/$member/smaps_rollup" ] || continue
    rss=$(awk '/^Rss:/{print $2}' "/proc/$member/smaps_rollup")
    pss=$(awk '/^Pss:/{print $2}' "/proc/$member/smaps_rollup")
    rss_kib=$((rss_kib + ${rss:-0}))
    pss_kib=$((pss_kib + ${pss:-0}))
  done < <(tree_pids "$pid" | sort -u)
  rss_mib=$((rss_kib / 1024))
  pss_mib=$((pss_kib / 1024))
  echo "RESULT native-rss-mib=$rss_mib"
  echo "RESULT native-pss-mib=$pss_mib"
  if [ "$rss_mib" -lt 300 ]; then
    echo "RESULT native-rss-budget=OK"
  else
    echo "RESULT native-rss-budget=OVER (target < 300 MiB)"
  fi
fi

if grep -qF 'dsh: native auth=failed' "$err"; then
  echo "RESULT native-gui=FAIL stage=auth"
  exit 1
fi
echo "work=$work"
echo "RESULT native-gui=OK"
