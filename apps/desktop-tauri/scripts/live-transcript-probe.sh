#!/usr/bin/env bash
# Prove that a live model's transcript replays identically.
#
# The Desktop lane must not change what the model sees and what the session
# records. The probe runs one turn against a real model, records the session
# it wrote, replays the same turn from that recording, and compares the two
# normalized session logs.
#
# The model is the Codex route: it needs the ChatGPT credential the local
# store holds, which the probe copies into place before the run.
#
#   usage: live-transcript-probe.sh
set -uo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
cd "$repo"

if ! command -v bun >/dev/null 2>&1; then
  echo "RESULT live-transcript=FAIL reason=no-bun" >&2
  exit 1
fi

if ! timeout 900 bun "$repo/apps/desktop-tauri/scripts/live-transcript-probe.mjs"; then
  echo "RESULT live-transcript=FAIL reason=probe-failed" >&2
  exit 1
fi
