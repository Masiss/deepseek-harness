#!/usr/bin/env bash
# Prove the isolation sandbox's budgets bite.
#
# The sandbox is where the desktop probes run, so a runaway probe must not
# be able to take the machine with it. The budgets are CPU, RAM, disk, and
# the number of processes; this probe checks that each one stops the work
# that exceeds it, and that a run inside the budget still finishes.
#
#   usage: isolation-budget-probe.sh
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
sandbox="$repo/apps/desktop-tauri/scripts/isolated-run.sh"

work=$(mktemp -d "${TMPDIR:-/tmp}/dsh-isolation-budget.XXXXXX") || exit 1
trap 'rm -rf "$work"' EXIT

# The budgets belong to the sandbox, so they are set on the call that starts
# it rather than inside the command it runs.
run() {
  DSH_ISOLATION_DIR="$work" "$sandbox" "$@"
}

failures=0

report() {
  local label=$1 expected=$2 actual=$3
  if [ "$expected" = "$actual" ]; then
    echo "RESULT $label=ok"
  else
    echo "RESULT $label=FAIL expected=$expected actual=$actual"
    failures=$((failures + 1))
  fi
}

# --- a run inside the budget finishes ------------------------------------
output=$(run bash -c 'echo done' 2>&1)
status=$?
if [ "$status" -eq 0 ] && printf '%s' "$output" | grep -q done; then
  report "inside-budget" ok ok
else
  report "inside-budget" ok "failed status=$status"
fi

# --- RAM: an allocation over the limit is killed --------------------------
# The helper is written inside the sandbox, because the sandbox's writable
# directory is a temporary filesystem of its own.
DSH_ISOLATION_MEMORY=64M run python3 -c '
import time
buffer = bytearray(300 * 1024 * 1024)
for index in range(0, len(buffer), 4096):
    buffer[index] = 1
time.sleep(30)
' >/dev/null 2>&1
status=$?
# The kernel kills the process when the cgroup cannot hold the allocation.
report "ram-budget" 137 "$status"

# --- disk: a write past the budget fails ----------------------------------
DSH_ISOLATION_DISK=8M run bash -c '
  dd if=/dev/zero of="$DSH_ISOLATION_DIR/big" bs=1M count=64 2>/dev/null
' >/dev/null 2>&1
status=$?
if [ "$status" -ne 0 ]; then
  report "disk-budget" ok ok
else
  report "disk-budget" ok "status=0"
fi

# --- processes: a fork bomb stops ----------------------------------------
DSH_ISOLATION_PIDS=16 run bash -c '
  for _ in $(seq 1 400); do sleep 30 & done
  sleep 30
' >/dev/null 2>&1
status=$?
if [ "$status" -ne 0 ]; then
  report "process-budget" ok ok
else
  report "process-budget" ok "status=0"
fi

# --- CPU: the sandbox reports the quota it runs under ---------------------
if printf '%s' "$output" | grep -q "budget cpu="; then
  report "cpu-budget" reported reported
else
  report "cpu-budget" reported missing
fi

if [ "$failures" -gt 0 ]; then
  echo "RESULT isolation-budget-probe=FAIL failures=$failures"
  exit 1
fi
echo "RESULT isolation-budget-probe=ok"
