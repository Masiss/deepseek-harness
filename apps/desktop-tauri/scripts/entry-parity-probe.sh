#!/usr/bin/env bash
# Prove both Desktop Host entries boot the same profile.
#
# The Tauri shell runs the Tauri entry; the Electron application runs the
# Electron entry. They boot the same profile through the same application, and
# a change to one must not change what the other serves. The probe boots each
# entry against the packaged tree and compares what they report.
#
# The Electron entry carries the Office plugin, so the Office component has to
# be installed first: the probe installs it through the Tauri entry's own
# bridge, exactly as a user would, and then boots both entries.
#
#   usage: entry-parity-probe.sh [linux-x64|linux-arm64]
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
target=${1:-linux-x64}
tree=$(CDPATH= cd -- "$repo/apps/desktop/.desktop-build/targets/$target" && pwd)
work=$(mktemp -d "$tree/probe/entry-parity.XXXXXX")
mkdir -p "$work/home" "$work/tmp"

host="$tree/dsh/node_modules/@deepseek-ai/dsh-desktop-host"
[ -f "$host/lib/index.js" ] || { echo "RESULT entry-parity=FAIL reason=missing-host"; exit 1; }
[ -f "$tree/components/components.json" ] || { echo "RESULT entry-parity=FAIL reason=missing-components"; exit 1; }
home="$work/home"

# The Host verifies the component manifest against the release key, so this
# rehearsal signs the tree's manifest with an ephemeral key and hands the Host
# the matching public key.
mkdir -p "$work/components"
cp -a "$tree/components/." "$work/components/"
cat > "$work/sign.mjs" <<'SIGNER'
import { generateKeyPairSync, sign } from 'node:crypto'
import { readFileSync, writeFileSync } from 'node:fs'
const [source, destination, publicFile] = process.argv.slice(2)
const { privateKey, publicKey } = generateKeyPairSync('ed25519')
const manifest = JSON.parse(readFileSync(source, 'utf8'))
const document = { schemaVersion: 1, components: manifest.components }
const signature = sign(null, Buffer.from(JSON.stringify(document), 'utf8'), privateKey).toString('base64')
writeFileSync(destination, `${JSON.stringify({ ...document, signature }, undefined, 2)}\n`)
writeFileSync(publicFile, publicKey.export({ type: 'spki', format: 'pem' }).toString())
SIGNER
"$tree/runtime/bin/node" "$work/sign.mjs" \
  "$work/components/components.json" "$work/components/components.json" "$work/public.pem"

# The component feed, served from the tree's own components directory.
cat > "$work/serve.mjs" <<'SERVER'
import { createReadStream, statSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:http'
import { join } from 'node:path'
const [dir, portFile] = process.argv.slice(2)
const server = createServer((request, response) => {
  const name = decodeURIComponent((request.url ?? '/').split('?')[0]).replace(/^\/+/u, '')
  const file = join(dir, name)
  try {
    const entry = statSync(file)
    if (!entry.isFile()) throw new Error('not a file')
    response.writeHead(200, { 'content-length': entry.size })
    createReadStream(file).pipe(response)
  } catch {
    response.writeHead(404).end()
  }
})
server.listen(0, '127.0.0.1', () => { writeFileSync(portFile, String(server.address().port)) })
SERVER
"$tree/runtime/bin/node" "$work/serve.mjs" "$work/components" "$work/port" &
server_pid=$!
for _ in $(seq 1 40); do [ -s "$work/port" ] && break; sleep 0.1; done
feed="http://127.0.0.1:$(cat "$work/port")"

# The environment both entries read.
export DSH_HOME="$home"
export DSH_DESKTOP_RUNTIME_DIR="$tree/dsh"
export DSH_DESKTOP_PROJECT_DIR="$tree/project"
export DSH_DESKTOP_PRIMARY_RUNTIME="$tree/runtime/primary-runtime"
export DSH_DESKTOP_PNPM_ENTRY="$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs"
export DSH_DESKTOP_NODE_BIN="$tree/runtime/bin"
export DSH_DESKTOP_VERSION="1.0.0"
export DSH_DESKTOP_COMPONENT_FEED="$feed"
export DSH_DESKTOP_COMPONENT_PUBLIC_KEY="$(cat "$work/public.pem")"

# Start one entry and wait for it to report the profile it serves.
# @param label - The name of the run.
# @param entry - The command that starts the entry.
start_entry() {
  local label=$1
  shift
  local run="$work/$label"
  mkdir -p "$run"
  mkfifo "$run/in" 2>/dev/null || true

  # The Host speaks over a control channel; a pipe on its standard input is
  # that channel.
  "$@" < "$run/in" > "$run/out" 2> "$run/err" &
  entry_pid=$!
  exec 3> "$run/in"

  url=""
  for _ in $(seq 1 240); do
    url=$(grep -ho 'http://127\.0\.0\.1:[0-9]*/?token=[A-Za-z0-9_-]*' "$run/out" "$run/err" 2>/dev/null | head -1)
    [ -n "$url" ] && break
    if ! kill -0 "$entry_pid" 2>/dev/null; then
      break
    fi
    sleep 0.5
  done
  if [ -n "$url" ]; then
    base="http://127.0.0.1:$(printf '%s' "$url" | sed -E 's#.*:([0-9]+)/.*#\1#')"
    curl -s -L -c "$run/cookies" -b "$run/cookies" -o /dev/null "$url"
    entry_ready=OK
  else
    entry_ready=FAIL
  fi
}

stop_entry() {
  printf '@@DSH@@{"type":"shutdown"}\n' >&3 2>/dev/null || true
  exec 3>&- 2>/dev/null || true
  sleep 1
  kill -9 "$entry_pid" 2>/dev/null || true
  wait "$entry_pid" 2>/dev/null || true
}

# The Tauri entry: the shell's own entry, with the arguments the shell passes.
tauri_args=("$tree/dsh" "$tree/project" "$tree/runtime/primary-runtime"
  "$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" "$tree/runtime/bin")

echo "--- install the Office component through the Tauri entry ---"
start_entry install "$tree/bin/desktop-host" "${tauri_args[@]}"
if [ "$entry_ready" != "OK" ]; then
  echo "RESULT entry-parity=FAIL reason=install-host"
  tail -5 "$work/install/err"
  kill "$server_pid" 2>/dev/null
  exit 1
fi
curl -s -m 900 -b "$work/install/cookies" -H 'content-type: application/json' \
  -X POST "$base/__dsh-desktop/components-install" -d null > "$work/install.json" || true
stop_entry

version=$("$tree/runtime/bin/node" -e "try{process.stdout.write(JSON.parse(require('node:fs').readFileSync('$home/components/current.json','utf8')).version)}catch{}")
office_runtime="$home/components/versions/$version/runtime"
[ -d "$office_runtime" ] || { echo "RESULT entry-parity=FAIL reason=no-office-runtime"; kill "$server_pid" 2>/dev/null; exit 1; }
echo "office_version=$version"

tauri_ready=0
electron_ready=0

echo "--- the Tauri entry ---"
start_entry tauri "$tree/bin/desktop-host" "${tauri_args[@]}"
if [ "$entry_ready" = "OK" ]; then
  tauri_ready=1
else
  tail -5 "$work/tauri/err"
fi
stop_entry

echo "--- the Electron entry ---"
start_entry electron node "$host/lib/index.js" "$tree/dsh" "$tree/project" "$office_runtime" \
  "$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" "$tree/runtime/bin"
if [ "$entry_ready" = "OK" ]; then
  electron_ready=1
else
  tail -5 "$work/electron/err"
fi
stop_entry

kill "$server_pid" 2>/dev/null

if [ "$tauri_ready" -eq 1 ] && [ "$electron_ready" -eq 1 ]; then
  echo "RESULT entry-parity=OK"
  exit 0
fi
echo "RESULT entry-parity=FAIL tauri=$tauri_ready electron=$electron_ready"
exit 1
