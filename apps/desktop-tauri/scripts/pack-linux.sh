#!/bin/sh
# Pack the release tarballs the Linux runtime tree installs from.
#
# Mirrors the pack steps the Electron packaging target runs, minus the parts
# that exist only for Electron, macOS, and Windows: `build:official`, then one
# `release:pack` per family, plus the private Desktop Host tarball the `dsh`
# family does not publish.
#
# Output goes to the same target directories `desktop-build-paths.mjs` hands
# the Electron lane, so both lanes read one layout.
#
# Every step runs through pnpm: `release:pack` resolves its own executable from
# `npm_execpath`, which only a pnpm lifecycle provides.
#
#   usage: pack-linux.sh [linux-x64|linux-arm64]
set -eu

target=${1:-linux-$(uname -m | sed 's/x86_64/x64/; s/aarch64/arm64/')}
case "$target" in
  linux-x64 | linux-arm64) ;;
  *) echo "pack-linux: unsupported target $target" >&2; exit 2 ;;
esac

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
out="$repo/apps/desktop/.desktop-build/targets/$target/packed"
cd "$repo"

mkdir -p "$out/dsh" "$out/vendor" "$out/landlock"

pnpm run build:official
pnpm run release:pack --family dsh --out "$out/dsh"
pnpm --dir apps/desktop-host pack --pack-destination "$out/dsh"
pnpm run release:pack --family vendor --out "$out/vendor"
pnpm --dir native/system run build:ts
pnpm --dir native/system/packages/entry pack --pack-destination "$out/landlock"
