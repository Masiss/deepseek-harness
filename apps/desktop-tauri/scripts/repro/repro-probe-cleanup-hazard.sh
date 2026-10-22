#!/usr/bin/env bash
# Reproduce the cold-start probe's cleanup killing a foreign process.
#
# The probe's cleanup lists every command line that contains the shell's
# release path and kills it, without checking who started it. A process that
# only mentions that path in its arguments therefore dies with the probe.
#
#   usage: repro-probe-cleanup-hazard.sh [linux-x64|linux-arm64]
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../../.." && pwd)
target=${1:-linux-x64}
work=$(mktemp -d /tmp/dsh-repro-cleanup.XXXXXX)

# A decoy that only carries the path in its arguments.
setsid bash -c 'exec -a "/opt/apps/release/dsh-desktop-tauri --decoy" sleep 300' &
decoy=$!
sleep 0.5
kill -0 "$decoy" 2>/dev/null || { echo 'RESULT repro=FAIL reason=decoy-died'; exit 1; }

DSH_COLD_START_WORK="$work/cold" bash "$repo/apps/desktop-tauri/scripts/cold-start-probe.sh" "$target" >/dev/null 2>&1

if kill -0 "$decoy" 2>/dev/null; then
  kill -9 "$decoy" 2>/dev/null
  echo 'RESULT repro=cleanup-hazard REFUTED: the decoy survived the probe'
  exit 1
fi
echo 'RESULT repro=cleanup-hazard CONFIRMED: the probe killed a process it did not start'
exit 0
