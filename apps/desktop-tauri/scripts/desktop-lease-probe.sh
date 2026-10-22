#!/usr/bin/env bash
# Prove two Desktop carriers cannot write the same state.
#
# The Tauri shell and the Electron application can both be installed. They
# keep their profile directories apart, but they share the session store, the
# settings document, and the credential store under the harness home. A lease
# in that home keeps the second Host out; a carrier that was killed leaves a
# lease the next one may take over.
#
#   usage: desktop-lease-probe.sh
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
# shellcheck source=probe-lib.sh
. "$repo/apps/desktop-tauri/scripts/probe-lib.sh"
target=${DSH_DESKTOP_TARGET:-linux-x64}
tree=$repo/apps/desktop/.desktop-build/targets/$target
mkdir -p "$tree/probe"
work=$(mktemp -d "$tree/probe/desktop-lease.XXXXXX")
home="$work/home"
mkdir -p "$home"
lease="$home/desktop-host.lease"

shell_pid=""

cleanup() {
  [ -n "$shell_pid" ] && kill -9 "$shell_pid" 2>/dev/null
  kill_tree "$shell_pid"
  true
}
trap cleanup EXIT

# Boot the release shell and wait for the Host URL it reports.
boot_shell() {
  local run=$1
  mkdir -p "$run"
  env DSH_DESKTOP_HOST_BIN="$tree/bin/desktop-host" \
  DSH_DESKTOP_LAUNCH_URL=1 \
    DSH_DESKTOP_RUNTIME_DIR="$tree/dsh" \
    DSH_DESKTOP_PROJECT_DIR="$tree/project" \
    DSH_DESKTOP_PRIMARY_RUNTIME="$tree/runtime/primary-runtime" \
    DSH_DESKTOP_PNPM_ENTRY="$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" \
    DSH_DESKTOP_NODE_BIN="$tree/runtime/bin" \
    DSH_DESKTOP_VERSION="1.0.0" \
    GDK_BACKEND=x11 \
    DSH_HOME="$home" \
    "$repo/apps/desktop-tauri/src-tauri/target/release/dsh-desktop-tauri" \
    > "$run/out" 2> "$run/err" &
  shell_pid=$!

  for _ in $(seq 1 200); do
    if grep -q 'http://127\.0\.0\.1:[0-9]*/?token=' "$run/err" 2>/dev/null; then
      echo "booted=OK"
      return 0
    fi
    if ! kill -0 "$shell_pid" 2>/dev/null; then
      echo "booted=FAIL"
      tail -10 "$run/err"
      return 1
    fi
    sleep 0.5
  done
  echo "booted=FAIL reason=timeout"
  tail -10 "$run/err"
  return 1
}

# The first carrier owns the state.
boot_shell "$work/first" || exit 1
if [ ! -f "$lease" ]; then
  echo "RESULT desktop-lease=FAIL reason=no-lease-file"
  exit 1
fi
echo "lease_owner=$(tr -d '\n' < "$lease")"

# A second Host on the same state finds the lease taken and must refuse.
env DSH_DESKTOP_HOST_BIN="$tree/bin/desktop-host" \
  DSH_DESKTOP_LAUNCH_URL=1 \
  DSH_DESKTOP_RUNTIME_DIR="$tree/dsh" \
  DSH_DESKTOP_PROJECT_DIR="$tree/project" \
  DSH_DESKTOP_PRIMARY_RUNTIME="$tree/runtime/primary-runtime" \
  DSH_DESKTOP_PNPM_ENTRY="$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" \
  DSH_DESKTOP_NODE_BIN="$tree/runtime/bin" \
  DSH_DESKTOP_VERSION="1.0.0" \
  DSH_HOME="$home" \
  "$tree/bin/desktop-host" "$tree/dsh" "$tree/project" \
  > "$work/second.out" 2> "$work/second.err"
second_exit=$?
if [ "$second_exit" -ne 0 ] && grep -q 'is in use by another Desktop application' "$work/second.err"; then
  echo "second_refused=OK"
else
  echo "second_refused=FAIL exit=$second_exit"
  tail -10 "$work/second.err"
fi

# A carrier that was killed leaves its lease behind; the next one takes it over.
kill -9 "$shell_pid" 2>/dev/null
wait "$shell_pid" 2>/dev/null
shell_pid=""
ps -eo pid,args --no-headers | grep -F "$tree/bin/desktop-host" | grep -v grep | awk '{print $1}' \
  | while read -r pid; do kill -9 "$pid" 2>/dev/null; done
sleep 1

boot_shell "$work/third" || exit 1
echo "lease_after_takeover=$(tr -d '\n' < "$lease")"

echo "RESULT desktop-lease=OK"
