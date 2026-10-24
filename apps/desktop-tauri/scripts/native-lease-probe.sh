#!/usr/bin/env bash
# Prove two packaged native Host processes cannot share one Desktop home lease.
#
# The first Host owns the lease, a second Host must refuse it, and a third may
# take it after the first process dies. All profiles and home data are temporary.
#
#   usage: native-lease-probe.sh [linux-x64|linux-arm64]
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
target=${1:-linux-x64}
tree=$(CDPATH= cd -- "$repo/apps/desktop/.desktop-build/targets/$target" && pwd)
work=$(mktemp -d "$tree/probe/native-lease.XXXXXX")
home="$work/home"
project="$work/project"
mkdir -p "$home"
cp -a "$tree/project" "$project"
chmod -R u+w "$project"

host_pid=""

cleanup() {
  if [ -n "$host_pid" ] && kill -0 "$host_pid" 2>/dev/null; then
    kill -TERM "$host_pid" 2>/dev/null || true
    wait "$host_pid" 2>/dev/null || true
  fi
  exec 9>&- 2>/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT

host_args=("$tree/dsh" "$project" "$tree/runtime/primary-runtime"
  "$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" "$tree/runtime/bin")

start_host() {
  local run=$1
  mkdir -p "$run"
  mkfifo "$run/in"
  env HOME="$work/home" DSH_HOME="$home/.dsh" \
    DSH_DESKTOP_RUNTIME_DIR="$tree/dsh" DSH_DESKTOP_PROJECT_DIR="$project" \
    DSH_DESKTOP_PRIMARY_RUNTIME="$tree/runtime/primary-runtime" \
    DSH_DESKTOP_PNPM_ENTRY="$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" \
    DSH_DESKTOP_NODE_BIN="$tree/runtime/bin" DSH_DESKTOP_VERSION=1.0.0 DSH_CLIENT_VERSION=1.0.0 \
    "$tree/bin/desktop-host" "${host_args[@]}" \
    < "$run/in" > "$run/out" 2> "$run/err" &
  host_pid=$!
  exec 9>"$run/in"
  for _ in $(seq 1 200); do
    if grep -q '"type":"ready"' "$run/out" 2>/dev/null; then
      echo "RESULT host-ready=OK pid=$host_pid"
      return 0
    fi
    if ! kill -0 "$host_pid" 2>/dev/null; then
      cat "$run/err" >&2
      return 1
    fi
    sleep 0.1
  done
  echo "RESULT host-ready=FAIL"
  cat "$run/err" >&2
  return 1
}

stop_host() {
  printf '@@DSH@@{"type":"shutdown"}\n' >&9
  exec 9>&-
  for _ in $(seq 1 100); do
    kill -0 "$host_pid" 2>/dev/null || break
    sleep 0.1
  done
  wait "$host_pid" 2>/dev/null || true
  host_pid=""
}

start_host "$work/first"
lease="$home/.dsh/desktop-host.lease"
[ -f "$lease" ] || { echo "RESULT lease=FAIL reason=missing"; exit 1; }
first_owner=$(tr -d '\n' < "$lease")
[ "$first_owner" = "$host_pid" ] || { echo "RESULT lease=FAIL owner=$first_owner expected=$host_pid"; exit 1; }

if env HOME="$work/home" DSH_HOME="$home/.dsh" \
  DSH_DESKTOP_RUNTIME_DIR="$tree/dsh" DSH_DESKTOP_PROJECT_DIR="$project" \
  DSH_DESKTOP_PRIMARY_RUNTIME="$tree/runtime/primary-runtime" \
  DSH_DESKTOP_PNPM_ENTRY="$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" \
  DSH_DESKTOP_NODE_BIN="$tree/runtime/bin" DSH_DESKTOP_VERSION=1.0.0 DSH_CLIENT_VERSION=1.0.0 \
  "$tree/bin/desktop-host" "${host_args[@]}" </dev/null >"$work/second.out" 2>"$work/second.err"; then
  echo "RESULT second-host=FAIL reason=unexpected-ready"
  exit 1
fi
grep -q 'in use by another Desktop application' "$work/second.err" || {
  cat "$work/second.err" >&2
  echo "RESULT second-host=FAIL reason=wrong-refusal"
  exit 1
}
echo "RESULT second-host=REFUSED"

kill -KILL "$host_pid"
wait "$host_pid" 2>/dev/null || true
host_pid=""
start_host "$work/third"
third_owner=$(tr -d '\n' < "$lease")
[ "$third_owner" = "$host_pid" ] || { echo "RESULT lease-takeover=FAIL owner=$third_owner expected=$host_pid"; exit 1; }
echo "RESULT lease-takeover=OK"
stop_host
echo "RESULT native-lease=OK"
