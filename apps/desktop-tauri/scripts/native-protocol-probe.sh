#!/usr/bin/env bash
# Verify native client wire behavior on loopback without launching the GUI or Host.
# Requires Cargo and the client-core dependencies; no model, credentials, package
# registry, profile, or desktop session is used by the test fixtures.
set -euo pipefail

root=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
cargo test --locked --offline --manifest-path "$root/apps/desktop-tauri/client-core/Cargo.toml"
echo 'RESULT native-protocol=OK evidence=loopback-mock live-host=unverified'
