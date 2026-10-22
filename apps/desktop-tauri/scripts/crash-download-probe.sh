#!/usr/bin/env bash
# Kill the Host while it writes a component download, then boot it again.
#
# The first run proves the crash leaves a partial `download-*` file because a
# SIGKILL skips the installer's `finally` blocks. The second run proves the
# Host reclaims that scratch space at startup without disturbing versions or
# the `current.json` marker.
#
#   usage: crash-download-probe.sh [linux-x64|linux-arm64]
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
target=${1:-linux-x64}
tree="$repo/apps/desktop/.desktop-build/targets/$target"
work=${DSH_CRASH_WORK:-$(mktemp -d "$tree/probe/crash-download.XXXXXX")}
mkdir -p "$work/feed" "$work/home" "$work/run"
home="$work/home"
profile="$work/project"
cp -a "$tree/project" "$profile"

# Reuse the tree's manifest and archive; sign the manifest with a key of this
# run so the Host accepts the feed.
cp "$tree/components/components.json" "$work/feed/components.json"
archive=$(python3 - "$tree/components/components.json" <<'PY'
import json, sys
for component in json.load(open(sys.argv[1]))['components']:
    if component['id'] == 'office':
        print(component['file'])
        break
PY
)
[ -n "$archive" ] || { echo 'RESULT crash-download=FAIL reason=no-artifact'; exit 1; }

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

# Serve the manifest immediately and the archive in small, throttled chunks,
# so the kill lands between the first write and the end of the download.
cat > "$work/serve.mjs" <<'SERVER'
import { createReadStream, readFileSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:http'
import { join } from 'node:path'
const [directory, archive, portFile] = process.argv.slice(2)
const manifest = readFileSync(join(directory, 'components.json'))
const server = createServer((request, response) => {
  if ((request.url ?? '').startsWith('/components.json')) {
    response.writeHead(200, { 'content-type': 'application/json', 'content-length': manifest.byteLength })
    response.end(manifest)
    return
  }
  if ((request.url ?? '') !== `/${archive}`) {
    response.writeHead(404).end()
    return
  }
  response.writeHead(200, { 'content-type': 'application/octet-stream' })
  const stream = createReadStream(join(directory, archive), { highWaterMark: 64 * 1024 })
  stream.on('data', (chunk) => {
    stream.pause()
    response.write(chunk)
    setTimeout(() => stream.resume(), 100)
  })
  stream.on('end', () => response.end())
  stream.on('error', () => response.destroy())
})
server.listen(0, '127.0.0.1', () => { writeFileSync(portFile, String(server.address().port)) })
SERVER

cp "$tree/components/$archive" "$work/feed/$archive"
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

echo "--- start a download and kill the Host ---"
start "$work/run-1"
[ -n "$base" ] || { echo 'RESULT crash-download=FAIL reason=not-ready'; kill "$server_pid" 2>/dev/null; exit 1; }
curl -s -m 120 -b "$work/cookies" -H 'content-type: application/json' \
  -X POST "$base/__dsh-desktop/components-install" -d null > "$work/install.json" &
curl_pid=$!
partial=""
for _ in $(seq 1 200); do
  partial=$(find "$home/components" -maxdepth 1 -name 'download-*' -type f 2>/dev/null | head -1)
  if [ -n "$partial" ] && [ "$(stat -c %s "$partial" 2>/dev/null || echo 0)" -gt 65536 ]; then break; fi
  sleep 0.1
done
if [ -z "$partial" ]; then
  echo 'RESULT crash-download=FAIL reason=no-partial-write'
  kill "$curl_pid" "$pid" "$server_pid" 2>/dev/null
  exit 1
fi
kill -9 "$pid" 2>/dev/null
wait "$pid" 2>/dev/null
exec 9>&- 2>/dev/null || true
kill "$curl_pid" 2>/dev/null
wait "$curl_pid" 2>/dev/null
left_before=$(find "$home/components" -maxdepth 1 -name 'download-*' -type f | wc -l)

echo "--- restart and check the scratch file is gone ---"
start "$work/run-2"
status=$(curl -s -m 20 -b "$work/cookies" -H 'content-type: application/json' \
  -X POST "$base/__dsh-desktop/components-status" -d null)
left_after=$(find "$home/components" -maxdepth 1 -name 'download-*' -type f | wc -l)
current=$([ -e "$home/components/current.json" ] && echo present || echo absent)
stop
kill "$server_pid" 2>/dev/null
wait "$server_pid" 2>/dev/null

if [ "$left_before" -eq 1 ] && [ "$left_after" -eq 0 ] \
  && [ "$current" = absent ] && printf '%s' "$status" | grep -q '"installed":false'; then
  echo "RESULT crash-download=OK left_before=$left_before cleaned=$left_after current=$current"
  exit 0
fi
echo "RESULT crash-download=FAIL left_before=$left_before cleaned=$left_after current=$current status=$status"
exit 1
