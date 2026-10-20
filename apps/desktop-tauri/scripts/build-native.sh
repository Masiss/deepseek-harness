#!/usr/bin/env bash
# Build the Rust native providers the Desktop payload ships.
#
# The Host package ships the loader, the `node-pty`-shaped wrapper, and the
# napi module; the Desktop launcher registers the loader before the profile
# boots. The Landlock launcher is a separate binary that replaces the C one in
# the runtime tree.
#
# The Electron carrier keeps node-pty and the C launcher, so both artifacts are
# only built for the Tauri lane.
#
#   usage: build-native.sh [linux-x64|linux-arm64]
set -eu

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
target=${1:-linux-$(uname -m | sed 's/x86_64/x64/; s/aarch64/arm64/')}
case "$target" in
  linux-x64 | linux-arm64) ;;
  *) echo "build-native: unsupported target $target" >&2; exit 2 ;;
esac

cargo build --release --manifest-path "$repo/native/dsh-native/Cargo.toml"

case "$(uname -s)" in
  Darwin) library=libdsh_native.dylib ;;
  Linux) library=libdsh_native.so ;;
  *) library=dsh_native.dll ;;
esac
cp "$repo/native/dsh-native/target/release/$library" "$repo/apps/desktop-host/native/dsh-native.node"
echo "build-native: module $(wc -c < "$repo/apps/desktop-host/native/dsh-native.node") bytes"

# The Landlock launcher replaces the C one in the runtime tree. A release
# binary shrinks with `strip`; unlike the bundled Python, a Rust binary has no
# relocation layout that stripping breaks.
cargo build --release --features landlock-bin --bin landlock-run \
  --manifest-path "$repo/native/dsh-native/Cargo.toml"
strip "$repo/native/dsh-native/target/release/landlock-run" 2>/dev/null || true
launcher_dir="$repo/apps/desktop/.desktop-build/targets/$target/native"
mkdir -p "$launcher_dir"
cp "$repo/native/dsh-native/target/release/landlock-run" "$launcher_dir/landlock-run"
echo "build-native: launcher $(wc -c < "$launcher_dir/landlock-run") bytes"
