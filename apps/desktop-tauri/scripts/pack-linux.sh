#!/bin/sh
# Pack the release tarballs the Linux runtime tree installs from.
#
# Mirrors the pack steps the Electron packaging target runs, minus the parts
# that exist only for Electron, macOS, and Windows: `build:official`, then one
# `release:pack` per family, plus the private Desktop Host tarball the `dsh`
# family does not publish.
#
# Every step runs through pnpm: `release:pack` resolves its own executable from
# `npm_execpath`, which only a pnpm lifecycle provides.
#
#   usage: pack-linux.sh <output-directory>
set -eu

out=${1:?usage: pack-linux.sh <output-directory>}
repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$repo"

mkdir -p "$out/dsh" "$out/vendor" "$out/landlock"

pnpm run build:official
pnpm run release:pack --family dsh --out "$out/dsh"
pnpm --dir apps/desktop-host pack --pack-destination "$out/dsh"
pnpm run release:pack --family vendor --out "$out/vendor"
pnpm --dir native/system run build:ts
pnpm --dir native/system/packages/entry pack --pack-destination "$out/landlock"
