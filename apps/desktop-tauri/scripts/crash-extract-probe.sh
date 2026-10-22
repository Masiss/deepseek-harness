#!/usr/bin/env bash
# Kill the Host while it extracts the Office archive, then boot it again.
#
# A crash between `mkdir staging-*` and the final rename can leave an
# extracted tree that no marker names. The next launch must reclaim it and
# must not leave an installable version behind. The archive is served at full
# speed so the kill lands during extraction rather than during the download.
#
#   usage: crash-extract-probe.sh [linux-x64|linux-arm64]
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
target=${1:-linux-x64}
tree="$repo/apps/desktop/.desktop-build/targets/$target"
work=${DSH_CRASH_WORK:-$tree/probe/crash-extract}
rm -rf "$work"
mkdir -p "$work/feed" "$work/home" "$work/run"
home="$work/home"
profile="$work/project"
cp -a "$tree/project" "$profile"

cp "$tree/components/components.json" "$work/feed/components.json"
archive=$(python3 - "$tree/components/components.json" <<'PY'
import json, sys
for component in json.load(open(sys.argv[1]))['components']:
    if component['id'] == 'office':
        print(component['file'])
        break
PY
)
[ -n "$archive" ] || { echo 'RESULT crash-extract=FAIL reason=no-artifact'; exit 1; }

# The archive copy is only needed while the probe runs; keep its logs.
trap 'rm -f "$work/feed/$archive"' EXIT

cat > "$work/sign.mjs" <<'SIGNER'
import { generateKeyPairSync, sign } from 'node:crypto'
import { readFileSync, writeFileSync } from 'node:fs'
const [source, destination, publicFile] = process.argv.slice(2)
const { privateKey, publicKey } = generateKeyPairSync('ed25519')
const manifest = JSON.parse(readFileSync(source, 'utf8'))
const document = { schemaVersion: manifest.schemaVersion, components: manifest.components }
const signature = sign(null, Buffer.from(JSON.stringify(document), 'utf8'), privateKey).toString('base64')
writeFileSync(destination, `${JSON.stringify({ ...document, signature }, undefined, 2)}\n`)
writeFileSync(publicFile, publicKey.export({ type: 'spki', format: 'pem' }).toString())
SIGNER
"$tree/runtime/bin/node" "$work/sign.mjs" \
  "$work/feed/components.json" "$work/feed/components.json" "$work/public.pem"

cp "$tree/components/$archive" "$work/feed/$archive"
cat > "$work/serve.mjs" <<'SERVER'
import { createReadStream, readFileSync, statSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:http'
import { join } from 'node:path'
const [directory, archive, portFile] = process.argv.slice(2)
const manifest = readFileSync(join(directory, 'components.json'))
const server = createServer((request, response) => {
  const url = request.url ?? ''
  if (url.startsWith('/components.json')) {
    response.writeHead(200, { 'content-type': 'application/json', 'content-length': manifest.byteLength })
    response.end(manifest)
    return
  }
  if (url !== `/${archive}`) {
    response.writeHead(404).end()
    return
  }
  const file = join(directory, archive)
  response.writeHead(200, { 'content-type': 'application/octet-stream', 'content-length': statSync(file).size })
  createReadStream(file).pipe(response)
})
server.listen(0, '127.0.0.1', () => { writeFileSync(portFile, String(server.address().port)) })
SERVER

"$tree/runtime/bin/node" "$work/serve.mjs" "$work/feed" "$archive" "$work/port" &
server_pid=$!
for _ in $(seq 1 40); do [ -s "$work/port" ] && break; sleep 0.1; done
feed="http://127.0.0.1:$(cat "$work/port")"

start() {
  local run=$1
  mkdir -p "$run"
  mkfifo "$run/in" 2>/dev/null || true
  env DSH_HOME="$home" DSH_DESKTOP_RUNTIME_DIR="$tree/dsh" \
    DSH_DESKTOP_PROJECT_DIR="$profile" \
    DSH_DESKTOP_PRIMARY_RUNTIME="$tree/runtime/primary-runtime" \
    DSH_DESKTOP_PNPM_ENTRY="$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" \
    DSH_DESKTOP_NODE_BIN="$tree/runtime/bin" DSH_DESKTOP_VERSION=1.0.0 \
    DSH_DESKTOP_COMPONENT_FEED="$feed" \
    DSH_DESKTOP_COMPONENT_PUBLIC_KEY="$(cat "$work/public.pem")" \
    "$tree/bin/desktop-host" "$tree/dsh" "$profile" "$tree/runtime/primary-runtime" \
    "$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" "$tree/runtime/bin" \
    < "$run/in" > "$run/out" 2> "$run/err" &
  pid=$!
  exec 9> "$run/in"
  url=""
  for _ in $(seq 1 200); do
    url=$(grep -ho 'http://127\.0\.0\.1:[0-9]*/?token=[A-Za-z0-9_-]*' "$run/out" "$run/err" 2>/dev/null | head -1)
    [ -n "$url" ] && break
    kill -0 "$pid" 2>/dev/null || break
    sleep 0.25
  done
  base=""
  [ -n "$url" ] && base="http://127.0.0.1:$(printf '%s' "$url" | sed -E 's#.*:([0-9]+)/.*#\1#')"
  [ -n "$base" ] && curl -s -L -c "$work/cookies" -b "$work/cookies" -o /dev/null "$url"
}

stop() {
  printf '@@DSH@@{"type":"shutdown"}\n' >&9 2>/dev/null || true
  exec 9>&- 2>/dev/null || true
  for _ in $(seq 1 100); do kill -0 "$pid" 2>/dev/null || break; sleep 0.1; done
  kill -9 "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null
}

echo "--- start an extract and kill the Host inside staging ---"
start "$work/run-1"
[ -n "$base" ] || { echo 'RESULT crash-extract=FAIL reason=not-ready'; kill "$server_pid" 2>/dev/null; exit 1; }
curl -s -m 120 -b "$work/cookies" -H 'content-type: application/json' \
  -X POST "$base/__dsh-desktop/components-install" -d null > "$work/install.json" &
curl_pid=$!
staging=""
for _ in $(seq 1 600); do
  staging=$(find "$home/components" -maxdepth 1 -name 'staging-*' -type d 2>/dev/null | head -1)
  [ -n "$staging" ] && break
  kill -0 "$pid" 2>/dev/null || break
  sleep 0.02
done
if [ -z "$staging" ]; then
  echo 'RESULT crash-extract=FAIL reason=no-staging'
  kill "$curl_pid" "$pid" "$server_pid" 2>/dev/null
  exit 1
fi
kill -9 "$pid" 2>/dev/null
wait "$pid" 2>/dev/null
exec 9>&- 2>/dev/null || true
kill "$curl_pid" 2>/dev/null
wait "$curl_pid" 2>/dev/null
staging_before=$(find "$home/components" -maxdepth 1 -name 'staging-*' -type d | wc -l)
current_before=$([ -e "$home/components/current.json" ] && echo present || echo absent)

echo "--- restart and check the staging tree is gone ---"
start "$work/run-2"
status=$(curl -s -m 20 -b "$work/cookies" -H 'content-type: application/json' \
  -X POST "$base/__dsh-desktop/components-status" -d null)
staging_after=$(find "$home/components" -maxdepth 1 -name 'staging-*' -type d 2>/dev/null | wc -l)
versions=$(find "$home/components/versions" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | wc -l)
stop
kill "$server_pid" 2>/dev/null
wait "$server_pid" 2>/dev/null

if [ "$staging_before" -eq 1 ] && [ "$current_before" = absent ] \
  && [ "$staging_after" -eq 0 ] && [ "$versions" -eq 0 ] \
  && printf '%s' "$status" | grep -q '"installed":false'; then
  echo "RESULT crash-extract=OK staging_before=$staging_before cleaned=$staging_after versions=$versions"
  exit 0
fi
echo "RESULT crash-extract=FAIL staging_before=$staging_before cleaned=$staging_after versions=$versions status=$status"
exit 1
