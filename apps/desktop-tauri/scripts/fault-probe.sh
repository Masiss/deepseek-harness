#!/usr/bin/env bash
# Inject the failures a desktop application must survive.
#
# Every scenario runs the packaged Host tree headlessly, in its own harness
# home, and reports one verdict. The probe covers:
#
#   startup-missing-runtime  a payload that lost its runtime must fail loudly
#                            instead of hanging the shell
#   startup-bad-profile      a corrupt profile must fail before serving, and
#                            must not leave the desktop lease behind
#   crash-takeover           a killed Host leaves a stale lease; the next
#                            launch must take it over and serve
#   shutdown                 the control frame must exit 0 and release the
#                            lease, with no Host left running
#   crash-leftovers          a killed install leaves scratch names; the next
#                            launch reclaims them without touching versions
#   offline-update           an unreachable update feed must classify as a
#                            network failure, not crash or install
#   offline-component        an unreachable component feed must classify as a
#                            network failure and leave no partial install
#
# Disk exhaustion is exercised separately through `isolated-run.sh`, whose
# writable directories are sized by a budget.
#
#   usage: fault-probe.sh [linux-x64|linux-arm64]
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
target=${1:-linux-x64}
tree="$repo/apps/desktop/.desktop-build/targets/$target"
work=${DSH_FAULT_WORK:-$tree/probe/fault}
rm -rf "$work"
mkdir -p "$work"
failures=0

host_args=("$tree/dsh" "$tree/project" "$tree/runtime/primary-runtime"
  "$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" "$tree/runtime/bin")

report() {
  local name=$1 status=$2 detail=$3
  echo "RESULT fault=$name $status${detail:+ $detail}"
  [ "$status" = ok ] || failures=$((failures + 1))
}

# Start one Host in its own harness home and wait for its authenticated URL.
# @param dir - Scenario directory.
# @param project - Profile directory the Host should load.
# @param home - Harness home the Host should own.
start() {
  local dir=$1 project=$2 home=$3
  shift 3
  mkdir -p "$dir" "$home"
  mkfifo "$dir/in" 2>/dev/null || true
  env DSH_HOME="$home" DSH_DESKTOP_RUNTIME_DIR="$tree/dsh" \
    DSH_DESKTOP_PROJECT_DIR="$project" \
    DSH_DESKTOP_PRIMARY_RUNTIME="$tree/runtime/primary-runtime" \
    DSH_DESKTOP_PNPM_ENTRY="$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" \
    DSH_DESKTOP_NODE_BIN="$tree/runtime/bin" DSH_DESKTOP_VERSION=1.0.0 \
    "$@" "$tree/bin/desktop-host" "${host_args[@]}" \
    < "$dir/in" > "$dir/out" 2> "$dir/err" &
  pid=$!
  exec 9> "$dir/in"
  url=""
  for _ in $(seq 1 120); do
    url=$(grep -ho 'http://127\.0\.0\.1:[0-9]*/?token=[A-Za-z0-9_-]*' "$dir/out" "$dir/err" 2>/dev/null | head -1)
    [ -n "$url" ] && break
    kill -0 "$pid" 2>/dev/null || break
    sleep 0.25
  done
  base=""
  if [ -n "$url" ]; then
    base="http://127.0.0.1:$(printf '%s' "$url" | sed -E 's#.*:([0-9]+)/.*#\1#')"
    curl -s -L -c "$dir/cookies" -b "$dir/cookies" -o /dev/null "$url"
  fi
}

stop() {
  printf '@@DSH@@{"type":"shutdown"}\n' >&9 2>/dev/null || true
  exec 9>&- 2>/dev/null || true
  for _ in $(seq 1 80); do kill -0 "$pid" 2>/dev/null || break; sleep 0.1; done
  kill -9 "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null
}

# --- startup: the payload lost its runtime ---------------------------------
mkdir -p "$work/missing"
timeout 20 "$tree/bin/desktop-host" "$work/missing/runtime" "$work/missing/project" \
  > "$work/missing/out" 2> "$work/missing/err"
code=$?
if [ "$code" -ne 0 ] && [ "$code" -ne 124 ] && grep -qi 'cannot find\|enoent\|error' "$work/missing/err" "$work/missing/out"; then
  report startup-missing-runtime ok "exit=$code"
else
  report startup-missing-runtime FAIL "exit=$code"
fi

# --- startup: a corrupt profile fails before serving ------------------------
mkdir -p "$work/bad-profile/profile" "$work/bad-profile/home"
printf '{ not json' > "$work/bad-profile/profile/package.json"
mkdir -p "$work/bad-profile/home"
timeout 30 env DSH_HOME="$work/bad-profile/home" \
  "$tree/bin/desktop-host" "$tree/dsh" "$work/bad-profile/profile" \
  "$tree/runtime/primary-runtime" \
  "$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" "$tree/runtime/bin" \
  > "$work/bad-profile/out" 2> "$work/bad-profile/err"
code=$?
lease="$work/bad-profile/home/desktop-host.lease"
if [ "$code" -ne 0 ] && [ "$code" -ne 124 ] && [ ! -e "$lease" ]; then
  report startup-bad-profile ok "exit=$code lease=released"
else
  report startup-bad-profile FAIL "exit=$code lease=$([ -e "$lease" ] && echo held || echo released)"
fi

# --- crash: the next launch takes over the stale lease ----------------------
start "$work/crash-a" "$tree/project" "$work/crash-home"
if [ -n "$base" ]; then
  kill -9 "$pid" 2>/dev/null
  wait "$pid" 2>/dev/null
  exec 9>&- 2>/dev/null || true
  lease="$work/crash-home/desktop-host.lease"
  stale=$([ -e "$lease" ] && echo held || echo missing)
  start "$work/crash-b" "$tree/project" "$work/crash-home"
  if [ -n "$base" ] && [ "$stale" = held ]; then
    report crash-takeover ok "stale=$stale takeover=ready"
  else
    report crash-takeover FAIL "stale=$stale takeover=${base:+ready}"
  fi
  stop
else
  report crash-takeover FAIL "first-launch=not-ready"
fi

# --- shutdown: exit 0, lease released, process gone ------------------------
start "$work/shutdown" "$tree/project" "$work/shutdown-home"
if [ -n "$base" ]; then
  printf '@@DSH@@{"type":"shutdown"}\n' >&9
  exec 9>&- 2>/dev/null || true
  for _ in $(seq 1 100); do kill -0 "$pid" 2>/dev/null || break; sleep 0.1; done
  wait "$pid"
  code=$?
  alive=no
  kill -0 "$pid" 2>/dev/null && alive=yes
  lease="$work/shutdown-home/desktop-host.lease"
  if [ "$code" -eq 0 ] && [ "$alive" = no ] && [ ! -e "$lease" ]; then
    report shutdown ok "exit=0 lease=released alive=no"
  else
    report shutdown FAIL "exit=$code lease=$([ -e "$lease" ] && echo held || echo released) alive=$alive"
  fi
else
  report shutdown FAIL "not-ready"
fi

# --- crash leftovers: the next launch reclaims scratch space --------------
mkdir -p "$work/leftover-home/components/download-stale" \
  "$work/leftover-home/components/staging-stale"
printf 'partial' > "$work/leftover-home/components/download-stale/artifact"
start "$work/leftover" "$tree/project" "$work/leftover-home"
if [ -n "$base" ] \
  && [ ! -e "$work/leftover-home/components/download-stale" ] \
  && [ ! -e "$work/leftover-home/components/staging-stale" ]; then
  report crash-leftovers ok "download=removed staging=removed"
else
  report crash-leftovers FAIL "download=$([ -e "$work/leftover-home/components/download-stale" ] && echo present || echo removed)"
fi
stop

# --- offline: update feed unreachable --------------------------------------
start "$work/offline-update" "$tree/project" "$work/offline-update-home" \
  DSH_DESKTOP_UPDATE_FEED=http://127.0.0.1:1/manifest.json
if [ -n "$base" ]; then
  curl -s -m 20 -b "$work/offline-update/cookies" -H 'content-type: application/json' \
    -X POST "$base/__dsh-desktop/updates-open" -d null > /dev/null
  status=$(curl -s -m 10 -b "$work/offline-update/cookies" -H 'content-type: application/json' \
    -X POST "$base/__dsh-desktop/updates-status" -d null)
  if printf '%s' "$status" | grep -q '"failure":"check-network"'; then
    report offline-update ok "status=$status"
  else
    report offline-update FAIL "status=$status"
  fi
  stop
else
  report offline-update FAIL "not-ready"
fi

# --- offline: component feed unreachable -----------------------------------
start "$work/offline-component" "$tree/project" "$work/offline-component-home" \
  DSH_DESKTOP_COMPONENT_FEED=http://127.0.0.1:1
if [ -n "$base" ]; then
  status=$(curl -s -m 60 -b "$work/offline-component/cookies" -H 'content-type: application/json' \
    -X POST "$base/__dsh-desktop/components-install" -d null)
  if printf '%s' "$status" | grep -q '"failure":"feed-network"' \
    && [ ! -e "$work/offline-component-home/components/current.json" ]; then
    report offline-component ok "status=$status"
  else
    report offline-component FAIL "status=$status"
  fi
  stop
else
  report offline-component FAIL "not-ready"
fi

if [ "$failures" -eq 0 ]; then
  echo "RESULT fault-probe=OK"
  exit 0
fi
echo "RESULT fault-probe=FAIL failed=$failures"
exit 1
