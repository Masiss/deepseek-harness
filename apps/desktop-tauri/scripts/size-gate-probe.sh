#!/usr/bin/env bash
# Prove the size gate fails when an artifact exceeds its budget.
#
# The gate is the release's budget check, and a gate that never fails is not
# a gate. This probe builds a tree whose artifact is over the download
# budget and one that is under it, and checks that the gate reports the
# first as a failure and the second as a pass.
#
# The gate reads its tree from a fixed path under the build directory, so
# the probe runs the gate against a scratch tree by pointing the build root
# at it. The budgets are read from the gate itself rather than repeated
# here, so a changed budget does not silently make this probe assert the
# wrong thing.
#
#   usage: size-gate-probe.sh [linux-x64|linux-arm64]
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
target=${DSH_DESKTOP_TARGET:-linux-x64}
gate="$repo/apps/desktop-tauri/scripts/size-gate.sh"

# The scratch tree holds a file one byte over the download budget, so it
# lives on the build filesystem rather than the small temporary one.
work=$(mktemp -d "$repo/apps/desktop/.desktop-build/size-gate-probe.XXXXXX") || exit 1
trap 'chmod -R u+rwX "$work" 2>/dev/null; rm -rf "$work"' EXIT

# The gate derives its tree from the build root, so a scratch root holds
# the scratch tree the probe judges.
build_root="$work/build"
tree="$build_root/targets/$target"
mkdir -p "$tree/appimage" "$tree/dsh" "$tree/runtime" "$tree/project"

# The budgets the gate enforces, read from its own text so the probe cannot
# drift from the gate.
budget_line=$(grep -E '^readonly DOWNLOAD_BUDGET=' "$gate")
download_budget=$(printf '%s' "$budget_line" | sed 's/[^0-9]//g')
[ -n "$download_budget" ] || { echo "RESULT budget=unreadable"; exit 1; }
echo "RESULT download_budget=$download_budget"

# One byte over the budget is enough to fail, so the probe does not have to
# build a huge file.
over=$((download_budget + 1))
head -c "$over" /dev/zero > "$tree/appimage/probe.AppImage"

output=$(DSH_DESKTOP_BUILD_ROOT="$build_root" bash "$gate" "$target" 2>&1)
status=$?
echo "RESULT over_status=$status"
printf '%s\n' "$output" | grep -E "^SIZE " | head -3
if [ "$status" -eq 0 ]; then
  echo "RESULT size-gate-probe=FAIL reason=over-budget-accepted"
  exit 1
fi
if ! printf '%s\n' "$output" | grep -q "RESULT size-gate=FAIL"; then
  echo "RESULT size-gate-probe=FAIL reason=no-failure-report"
  exit 1
fi

# An empty tree has no artifact to judge, so the gate passes.
rm -f "$tree/appimage/probe.AppImage"
output=$(DSH_DESKTOP_BUILD_ROOT="$build_root" bash "$gate" "$target" 2>&1)
status=$?
echo "RESULT under_status=$status"
if [ "$status" -ne 0 ]; then
  echo "RESULT size-gate-probe=FAIL reason=under-budget-rejected"
  printf '%s\n' "$output" | head -5
  exit 1
fi

echo "RESULT size-gate-probe=ok"
