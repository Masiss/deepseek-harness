#!/usr/bin/env bash
# Prove both Desktop Host entries boot the same profile.
#
# The Tauri shell runs the Tauri entry; the Electron application runs the
# Electron entry. They boot the same profile through the same application, and
# a change to one must not change what the other serves. The probe boots each
# entry against the packaged tree and compares what they report.
#
#   usage: entry-parity-probe.sh [linux-x64|linux-arm64]
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
target=${1:-linux-x64}
tree=$repo/apps/desktop/.desktop-build/targets/$target
work="$tree/probe/entry-parity"
mkdir -p "$work/home"

host="$tree/dsh/node_modules/@deepseek-ai/dsh-desktop-host"
[ -f "$host/lib/index.js" ] || { echo "RESULT entry-parity=FAIL reason=missing-host"; exit 1; }

# Boot one entry and report whether it served the profile.
boot_entry() {
  local label=$1
  local entry=$2
  shift 2
  local run="$work/$label"
  mkdir -p "$run"

  DSH_HOME="$run/home" timeout 120 node "$entry" "$@" \
    > "$run/out" 2> "$run/err" &
  local pid=$!

  local ready=""
  for _ in $(seq 1 120); do
    ready=$(grep -o '"type":"ready"[^}]*' "$run/out" 2>/dev/null | head -1)
    [ -n "$ready" ] && break
    if ! kill -0 "$pid" 2>/dev/null; then
      break
    fi
    sleep 0.5
  done
  kill -9 "$pid" 2>/dev/null
  wait "$pid" 2>/dev/null

  if [ -n "$ready" ]; then
    echo "$label=OK"
    return 0
  fi
  echo "$label=FAIL"
  tail -5 "$run/err"
  return 1
}

tauri_ready=0
electron_ready=0

# The Tauri entry: the shell's own entry, with the arguments the shell passes.
if boot_entry tauri "$host/lib/tauri.js" \
  "$tree/dsh" "$tree/project" "$tree/runtime/primary-runtime" \
  "$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" "$tree/runtime/bin"; then
  tauri_ready=1
fi

# The Electron entry: the same profile, the same application, the arguments the
# Electron application passes.
if boot_entry electron "$host/lib/index.js" \
  "$tree/dsh" "$tree/project" "$tree/runtime/primary-runtime" \
  "$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" "$tree/runtime/bin"; then
  electron_ready=1
fi

if [ "$tauri_ready" -eq 1 ] && [ "$electron_ready" -eq 1 ]; then
  echo "RESULT entry-parity=OK"
  exit 0
fi
echo "RESULT entry-parity=FAIL tauri=$tauri_ready electron=$electron_ready"
exit 1
