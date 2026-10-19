#!/bin/sh
# Dev launcher: run the TypeScript Host with the workstation's own Node.
#
# The packaged launcher will run the bundled primary-runtime Node instead; this
# script exists so the Tauri shell can be exercised before that runtime ships.
#
#   DSH_DESKTOP_HOST_BIN=.../scripts/dev-host.sh \
#   DSH_DESKTOP_RUNTIME_DIR=<dsh install dir> \
#   DSH_DESKTOP_PROJECT_DIR=<profile dir> \
#   cargo run --manifest-path src-tauri/Cargo.toml
set -e
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec node "$here/../../desktop-host/src/tauri.ts" "$@"
