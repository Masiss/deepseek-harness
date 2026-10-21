#!/usr/bin/env bash
# Run the Windows-native test suites under Wine.
#
# The Windows provider in `packages/` binds Win32 through koffi, and its
# behaviour cannot be observed on Linux. Wine supplies the Win32 APIs, and a
# Windows Node under it loads the Windows koffi prebuild, so the suites that
# cover the Windows provider can run here.
#
# The install happens in a snapshot of the tree, not in the workspace: the
# Windows Node does not realpath pnpm's isolated-layout symlinks, and the
# Windows build of the native packages has to materialize. The snapshot
# therefore installs with the hoisted linker and the win32-x64 platform
# packages, exactly as the upstream Windows gate does.
#
# The Wine run does not replace a Windows runner: Wine implements the
# documented Win32 surface, not every corner of it. It gives the P5-win32
# work a koffi baseline to compare a Rust provider against.
#
# Wine does not implement Windows security descriptors: `SetNamedSecurityInfoW`
# and `GetNamedSecurityInfoW` fail with ERROR_ACCESS_DENIED, so the ACL suite's
# write and revoke cases fail here while its other cases pass. The other three
# suites pass in full. A Windows runner still decides the ACL behaviour.
#
#   usage: windows-native-probe.sh [--keep]
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
keep=false
[ "${1:-}" = "--keep" ] && keep=true

scratch="$repo/apps/desktop/.desktop-build/targets/linux-x64/probe-windows"
cache="$repo/.cache/wine-windows"
node_version=24.22.0
node_zip="$cache/node-v$node_version-win-x64.zip"

command -v wine >/dev/null || { echo "windows-native-probe: wine is missing" >&2; exit 2; }
command -v unzip >/dev/null || { echo "windows-native-probe: unzip is missing" >&2; exit 2; }
command -v pnpm >/dev/null || { echo "windows-native-probe: pnpm is missing" >&2; exit 2; }

mkdir -p "$cache"
if [ ! -f "$node_zip" ]; then
  echo "windows-native-probe: fetching the Windows Node $node_version"
  curl -fsSL -o "$node_zip" "https://nodejs.org/dist/v$node_version/node-v$node_version-win-x64.zip"
fi

rm -rf "$scratch"
mkdir -p "$scratch/tree" "$scratch/node" "$scratch/logs"

unzip -q -o "$node_zip" -d "$scratch/node"
# Wine resolves a relative program path against the current directory, and
# the suites run from the snapshot tree, so the Node path is absolute.
node_exe=$(find "$scratch/node" -name node.exe | head -1)
[ -n "$node_exe" ] || { echo "windows-native-probe: the Windows Node is missing" >&2; exit 2; }

# The snapshot carries the tracked and untracked-unignored files. The
# workspace's own node_modules stay untouched.
git -C "$repo" ls-files -z --cached --others --exclude-standard -- . ':!.claude' ':!.codex' \
  | while IFS= read -r -d '' file; do [ -e "$repo/$file" ] && printf '%s\0' "$file"; done \
  | tar -C "$repo" --null --files-from=- -cf - \
  | tar -C "$scratch/tree" -xf -

# The hoisted linker lets the Windows Node resolve the workspace packages;
# the win32-x64 platform packages materialize the Windows native binaries.
cat >> "$scratch/tree/pnpm-workspace.yaml" << 'EOF'

nodeLinker: hoisted
supportedArchitectures:
  os: [current, win32]
  cpu: [current, x64]
EOF

echo "windows-native-probe: installing the snapshot for Windows"
(cd "$scratch/tree" && pnpm install --frozen-lockfile --ignore-scripts > "$scratch/logs/install.log" 2>&1) \
  || { tail -40 "$scratch/logs/install.log" >&2; echo "windows-native-probe: the snapshot install failed" >&2; exit 2; }

# Wine maps the host filesystem under Z:, and a Windows path spells the same
# location. The suites write their own output, so a scratch directory on the
# host keeps the workspace clean.
to_windows_path() {
  printf 'Z:\\%s' "$(printf '%s' "$1" | sed 's|^/||; s|/|\\|g')"
}
windows_tree=$(to_windows_path "$scratch/tree")
windows_scratch=$(to_windows_path "$scratch/out")
mkdir -p "$scratch/out"

suites=(sandbox/sandbox-windows-acl subprocess/win32-process session/session-persistence-jsonl fs/fs-local)
status=0
for suite in "${suites[@]}"; do
  echo "windows-native-probe: running packages/$suite"
  (
    cd "$scratch/tree"
    TMPDIR="$windows_scratch" \
    wine "$node_exe" \
      "$windows_tree\node_modules\vitest\vitest.mjs" run \
      "packages/$suite" --reporter=verbose
  ) > "$scratch/logs/$(basename "$suite").log" 2>&1 || status=1
  grep -aE "Test Files|Tests  " "$scratch/logs/$(basename "$suite").log" | tail -2 || true
done

if [ "$keep" = true ]; then
  echo "windows-native-probe: kept $scratch"
else
  rm -rf "$scratch/tree" "$scratch/node"
fi

if [ "$status" -eq 0 ]; then
  echo "windows-native-probe: RESULT suites=ok"
else
  echo "windows-native-probe: RESULT suites=failed" >&2
fi
exit "$status"
