#!/usr/bin/env bash
# Check the packaged Linux artifacts against the size budget.
#
# The budget has two sides: what a user downloads, and what the application
# occupies once installed. The installed side carries the runtime and the
# harness packages, so its budget is larger; the downloaded side is the one
# that decides whether a release is shippable.
#
#   usage: size-gate.sh [linux-x64|linux-arm64]
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
target=${1:-linux-x64}
tree="$repo/apps/desktop/.desktop-build/targets/$target"

# Download budget: the largest artifact a user may fetch, in bytes.
readonly DOWNLOAD_BUDGET=$((105 * 1024 * 1024))
# Installed budget: the runtime, the harness packages, and the shell.
readonly INSTALLED_BUDGET=$((400 * 1024 * 1024))

failures=0

report() {
  local label=$1 bytes=$2 budget=$3
  if [ "$bytes" -gt "$budget" ]; then
    printf 'SIZE %s bytes=%s budget=%s over=%s\n' "$label" "$bytes" "$budget" "$((bytes - budget))"
    failures=$((failures + 1))
  else
    printf 'SIZE %s bytes=%s budget=%s within\n' "$label" "$bytes" "$budget"
  fi
}

for artifact in "$tree"/appimage/*.AppImage "$tree"/deb/*.deb "$tree"/rpm/*.rpm; do
  [ -f "$artifact" ] || continue
  report "$(basename "$artifact")" "$(stat -c %s "$artifact")" "$DOWNLOAD_BUDGET"
done

installed=0
for part in dsh runtime project; do
  [ -d "$tree/$part" ] || continue
  installed=$((installed + $(du -sb "$tree/$part" | cut -f1)))
done
report "installed-tree" "$installed" "$INSTALLED_BUDGET"

if [ "$failures" -gt 0 ]; then
  echo "RESULT size-gate=FAIL over=$failures"
  exit 1
fi
echo "RESULT size-gate=OK"
