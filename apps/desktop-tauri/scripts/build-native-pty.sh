#!/usr/bin/env bash
# Build the Rust PTY provider into the Desktop Host's native directory.
#
# The Host package ships the loader, the `node-pty`-shaped wrapper, and this
# binary; the Desktop launcher registers the loader before the profile boots.
# The Electron carrier keeps node-pty, so the binary is only built for the
# Tauri lane.
set -eu

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
cargo build --release --manifest-path "$repo/native/dsh-native/Cargo.toml"

case "$(uname -s)" in
  Darwin) library=libdsh_native.dylib ;;
  Linux) library=libdsh_native.so ;;
  *) library=dsh_native.dll ;;
esac
cp "$repo/native/dsh-native/target/release/$library" "$repo/apps/desktop-host/native/dsh-native.node"
echo "build-native-pty: $(wc -c < "$repo/apps/desktop-host/native/dsh-native.node") bytes"
