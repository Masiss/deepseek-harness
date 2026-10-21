#!/usr/bin/env bash
# Compare the Rust image provider with the sharp package it replaces.
#
# The probe runs the same operations on both providers: the intrinsic facts of
# a raster, the encoded outputs of every format the native module writes, and
# the metadata a caller asks to keep. It reports both results side by side and
# fails when the facts differ or a lossless output differs by a byte.
#
# The provider under test is the one in the packaged tree, where sharp is not
# installed: an operation that still needed the sharp package would fail there
# rather than fall back.
#
#   usage: image-parity-probe.sh [linux-x64|linux-arm64]
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
target=${1:-linux-x64}
tree="$repo/apps/desktop/.desktop-build/targets/$target"
shim="$tree/dsh/node_modules/@deepseek-ai/dsh-desktop-host/native/sharp.cjs"
[ -f "$shim" ] || { echo "RESULT shim=missing"; exit 1; }

script="$tree/dsh/probe/image-parity.mjs"
work="$tree/probe/image-parity"
mkdir -p "$(dirname "$script")" "$work/home" "$work/tmp"
cp "$repo/apps/desktop-tauri/scripts/image-parity.mjs" "$script"

DSH_ISOLATION_DIR="$work" "$repo/apps/desktop-tauri/scripts/isolated-run.sh" \
  env DSH_IMAGE_SHIM="$shim" \
  DSH_IMAGE_SHARP="$repo/packages/attachment/attachment-local/node_modules/sharp" \
  "$tree/runtime/bin/node" "$script"
