#!/usr/bin/env bash
# Probe: one real model turn through the ChatGPT-backed Codex provider.
#
# The `openai-codex` route is OAuth-only: it authenticates with a ChatGPT
# access token and takes the account id from that token's claims. This probe
# answers a question the Desktop lane cannot answer offline — is a live
# provider reachable from this machine, through the shipped Harness
# composition, with the credential the local store holds?
#
# Steps:
#   1. copy the Codex OAuth grant into the Harness credential store;
#   2. run one headless turn through the Codex route;
#   3. require the model's reply to carry the sentinel.
#
# The ChatGPT account only serves some models: `gpt-5.5` and
# `gpt-5.3-codex-spark` are refused for a ChatGPT account, while
# `gpt-5.6-terra` answers. The model id lives in the patch beside this script.
#
# usage: codex-model-probe.sh [--check]
#   --check  report whether the store holds a usable grant, and stop
set -uo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
patch_file="$root/apps/desktop-tauri/scripts/codex-model.patch.yml"
sentinel='CODEX_OK'
work=$(mktemp -d "${TMPDIR:-/tmp}/dsh-codex-probe.XXXXXX")
trap 'rm -rf "$work"' EXIT

export TMPDIR="$work/tmp"
mkdir -p "$TMPDIR"

cd "$root"

if ! command -v node >/dev/null 2>&1; then
  echo "RESULT codex-model=FAIL reason=no-node" >&2
  exit 1
fi

# 1. The grant the route reads.
if ! node --import ./node_modules/tsx/dist/esm/index.mjs apps/desktop-tauri/scripts/codex-credential.ts; then
  echo "RESULT codex-model=FAIL reason=no-credential" >&2
  exit 1
fi

if [ "${1:-}" = "--check" ]; then
  echo "RESULT codex-model=OK reason=credential-present"
  exit 0
fi

# 2. One live turn. The native addon needs Node, so Bun cannot run this.
# The store's file watcher keeps the process alive after the answer, so the
# probe reads the reply from the output and stops the run itself.
out="$work/out.log"
err="$work/err.log"
node --import ./node_modules/tsx/dist/esm/index.mjs \
  apps/cli/src/bin.ts --profile headless --patch "$patch_file" \
  "Reply with exactly $sentinel" >"$out" 2>"$err" &
pid=$!

answered=0
for _ in $(seq 1 300); do
  if grep -q "$sentinel" "$out" 2>/dev/null; then
    answered=1
    break
  fi
  if ! kill -0 "$pid" 2>/dev/null; then
    break
  fi
  sleep 1
done
kill "$pid" 2>/dev/null
wait "$pid" 2>/dev/null

if [ "$answered" -ne 1 ]; then
  echo "RESULT codex-model=FAIL reason=no-sentinel" >&2
  tail -5 "$err" >&2
  exit 1
fi

echo "RESULT codex-model=OK"
