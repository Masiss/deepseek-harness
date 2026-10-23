#!/usr/bin/env bash
# Drive one live approval round-trip on the packaged runtime.
#
# A local mock model asks for a bash call that widens the sandbox; the native
# app answers the Host's `$events` approval request with allow-once, and the
# command must then write the marker outside the workspace. The app runs on an
# isolated DSH_HOME and the mock never reaches a real provider.
#
# Usage: approval-probe.sh
# Output: RESULT markers on stdout; app, Host, and mock logs stay in the work
# directory.
set -euo pipefail

root=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
bin="$root/apps/desktop-tauri/native-app/target/release/dsh-desktop-native-app"
mock="$root/apps/desktop-tauri/scripts/mock-model-server.mjs"
tree="$root/apps/desktop/.desktop-build/targets/linux-x64"
node="$tree/runtime/bin/node"
host_package="$tree/dsh/node_modules/@deepseek-ai/dsh-desktop-host/package.json"
for required in "$bin" "$mock" "$node" "$tree/bin/desktop-host" \
  "$tree/runtime/bin/node" "$tree/runtime/primary-runtime/dependencies/node/bin/node" \
  "$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" "$host_package" \
  "$tree/project/package.json" "$tree/project/cordis.yml"; do
  [ -e "$required" ] || { echo "approval-probe: missing input: $required" >&2; exit 1; }
done
[ -x "$bin" ] || { echo "approval-probe: not executable: $bin" >&2; exit 1; }
if [ -z "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ]; then
  echo "approval-probe: no graphical session (WAYLAND_DISPLAY/DISPLAY unset)" >&2
  echo "RESULT approval=SKIP reason=no-display"
  exit 0
fi

version=$("$node" --input-type=module -e \
  'import { readFileSync } from "node:fs"; const { version } = JSON.parse(readFileSync(process.argv[1], "utf8")); if (typeof version !== "string" || !/^[0-9A-Za-z.+-]+$/.test(version)) throw new Error("invalid packaged Host version"); console.log(version)' \
  "$host_package")

work_root="${DSH_PROBE_TMPDIR:-$HOME/.cache/dsh-probe}"
mkdir -p "$work_root"
work=$(mktemp -d "$work_root/approval.XXXXXX")
home="$work/home"
project="$work/project"
marker="$work/approval-marker.txt"
mkdir -p "$home"
cp -a "$tree/project" "$project"
chmod -R u+w "$project"

app_pid=''
mock_pid=''

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

cleanup() {
  if [ -n "$app_pid" ] && kill -0 "$app_pid" 2>/dev/null; then
    kill -TERM "$app_pid" 2>/dev/null || true
    for _ in $(seq 1 100); do
      kill -0 "$app_pid" 2>/dev/null || break
      sleep 0.1
    done
    kill -KILL "$app_pid" 2>/dev/null || true
  fi
  if [ -n "$app_pid" ]; then
    for child in $(cardinal_children "$app_pid" 2>/dev/null || true); do
      kill -TERM "$child" 2>/dev/null || true
    done
  fi
  if [ -n "$mock_pid" ] && kill -0 "$mock_pid" 2>/dev/null; then
    kill -TERM "$mock_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT

DSH_APPROVAL_PROBE_MARKER="$marker" "$node" "$mock" >"$work/mock.out" 2>"$work/mock.err" &
mock_pid=$!
for _ in $(seq 1 100); do
  grep -q '^MOCK_MODEL_URL=' "$work/mock.out" 2>/dev/null && break
  kill -0 "$mock_pid" 2>/dev/null || { echo "approval-probe: mock exited" >&2; cat "$work/mock.err" >&2; exit 1; }
  sleep 0.1
done
mock_url=$(sed -n 's/^MOCK_MODEL_URL=//p' "$work/mock.out" | head -1)
[ -n "$mock_url" ] || { echo "approval-probe: mock did not report a URL" >&2; exit 1; }

launcher="$work/launch.sh"
cat >"$launcher" <<LAUNCH
#!/bin/sh
export DSH_HOME='$home'
export DSH_DESKTOP_AUTO_CONNECT='1'
export DSH_DESKTOP_PROBE_APPROVAL='1'
export DSH_APPROVAL_PROBE_MARKER='$marker'
export DEEPSEEK_API_KEY='approval-probe-key'
export DEEPSEEK_BASE_URL='$mock_url'
export DSH_DESKTOP_HOST_BIN='$tree/bin/desktop-host'
export DSH_DESKTOP_RUNTIME_DIR='$tree/dsh'
export DSH_DESKTOP_PROJECT_DIR='$project'
export DSH_DESKTOP_PRIMARY_RUNTIME='$tree/runtime/primary-runtime'
export DSH_DESKTOP_PNPM_ENTRY='$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs'
export DSH_DESKTOP_NODE_BIN='$tree/runtime/bin'
export DSH_DESKTOP_VERSION='$version'
export DSH_CLIENT_VERSION='$version'
export TMPDIR='$work'
exec '$bin'
LAUNCH
chmod +x "$launcher"

"$launcher" >"$work/app.out" 2>"$work/app.err" &
app_pid=$!

wait_for() {
  local wanted=$1 seconds=$2
  for _ in $(seq 1 $((seconds * 10))); do
    grep -qF "$wanted" "$work/app.err" 2>/dev/null && return 0
    kill -0 "$app_pid" 2>/dev/null || return 1
    sleep 0.1
  done
  return 1
}

status=0
if ! wait_for 'dsh: native auth=ok' 120; then
  echo "RESULT approval=FAIL stage=auth"
  tail -30 "$work/app.err" >&2 || true
  status=1
else
  echo "RESULT approval-auth=ok"
  if wait_for 'dsh: native approval=asked' 120; then
    echo "RESULT approval=asked"
    if wait_for 'dsh: native approval=allowed-once' 60; then
      echo "RESULT approval=allowed-once"
      if wait_for 'dsh: native approval=roundtrip-ok' 90; then
        echo "RESULT approval=tool-executed"
        echo "RESULT approval-roundtrip=OK"
        if wait_for 'dsh: native job=follow-ok' 90; then
          echo "RESULT job-follow=OK"
        else
          echo "RESULT job-follow=FAIL"
          status=1
        fi
      else
        echo "RESULT approval=FAIL stage=marker"
        status=1
      fi
    else
      echo "RESULT approval=FAIL stage=answer"
      status=1
    fi
  else
    echo "RESULT approval=FAIL stage=ask"
    tail -30 "$work/app.err" >&2 || true
    status=1
  fi
fi
echo "work=$work"
exit "$status"
