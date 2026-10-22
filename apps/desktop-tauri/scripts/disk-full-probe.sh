#!/usr/bin/env bash
# Fill the disk while the on-demand component installs, and check the failure.
#
# The component archive is roughly 10x the budget this probe runs with. The
# install must stop with a classified failure, leave no installed version and
# no partial download behind, and keep the Host serving.
#
# Run it under isolated-run.sh with a small disk budget:
#
#   DSH_ISOLATION_DIR=... DSH_ISOLATION_DISK=48M \
#     isolated-run.sh bash disk-full-probe.sh [linux-x64|linux-arm64]
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
target=${1:-linux-x64}
tree="$repo/apps/desktop/.desktop-build/targets/$target"
work=${DSH_ISOLATION_DIR:?set DSH_ISOLATION_DIR (run under isolated-run.sh)}
home="$work/home/.dsh"
profile="$home/desktop-project"
components="$work/components"
rm -rf "$home" "$components"
mkdir -p "$home" "$components"
cp -a "$tree/project" "$profile"

# Serve the tree's archives without writing them: the server reads the
# read-only tree, while the Host writes into the bounded work directory.
cat > "$work/serve.mjs" <<'SERVER'
import { createReadStream, statSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:http'
import { join } from 'node:path'
const [primary, fallback, portFile] = process.argv.slice(2)
const server = createServer((request, response) => {
  const name = decodeURIComponent((request.url ?? '/').split('?')[0]).replace(/^\/+/u, '')
  for (const dir of [primary, fallback]) {
    const file = join(dir, name)
    try {
      const entry = statSync(file)
      if (!entry.isFile()) continue
      response.writeHead(200, { 'content-length': entry.size })
      createReadStream(file).pipe(response)
      return
    } catch {
      continue
    }
  }
  response.writeHead(404).end()
})
server.listen(0, '127.0.0.1', () => { writeFileSync(portFile, String(server.address().port)) })
SERVER

"$tree/runtime/bin/node" "$work/serve.mjs" "$components" "$tree/components" "$work/port" &
server_pid=$!
for _ in $(seq 1 40); do [ -s "$work/port" ] && break; sleep 0.1; done
feed="http://127.0.0.1:$(cat "$work/port")"

# Sign the manifest with a key of this run, because the Host verifies it
# before downloading a byte. The archive itself stays in the read-only tree.
mkdir -p "$components"
cp "$tree/components/components.json" "$components/components.json"
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
  "$components/components.json" "$components/components.json" "$work/public.pem"

work_host="$work/host"
mkdir -p "$work_host"
mkfifo "$work_host/in"
env DSH_HOME="$home" DSH_DESKTOP_RUNTIME_DIR="$tree/dsh" \
  DSH_DESKTOP_PROJECT_DIR="$profile" \
  DSH_DESKTOP_PRIMARY_RUNTIME="$tree/runtime/primary-runtime" \
  DSH_DESKTOP_PNPM_ENTRY="$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" \
  DSH_DESKTOP_NODE_BIN="$tree/runtime/bin" DSH_DESKTOP_VERSION=1.0.0 \
  DSH_DESKTOP_COMPONENT_FEED="$feed" \
  DSH_DESKTOP_COMPONENT_PUBLIC_KEY="$(cat "$work/public.pem")" \
  "$tree/bin/desktop-host" "$tree/dsh" "$profile" "$tree/runtime/primary-runtime" \
  "$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" "$tree/runtime/bin" \
  < "$work_host/in" > "$work_host/out" 2> "$work_host/err" &
host_pid=$!
exec 9> "$work_host/in"

url=""
for _ in $(seq 1 160); do
  url=$(grep -ho 'http://127\.0\.0\.1:[0-9]*/?token=[A-Za-z0-9_-]*' "$work_host/out" "$work_host/err" 2>/dev/null | head -1)
  [ -n "$url" ] && break
  kill -0 "$host_pid" 2>/dev/null || break
  sleep 0.25
done

if [ -z "$url" ]; then
  echo "RESULT disk-full=FAIL reason=host-not-ready"
  tail -5 "$work_host/err"
  kill "$server_pid" "$host_pid" 2>/dev/null
  exit 1
fi
base="http://127.0.0.1:$(printf '%s' "$url" | sed -E 's#.*:([0-9]+)/.*#\1#')"
curl -s -L -c "$work_host/cookies" -b "$work_host/cookies" -o /dev/null "$url"

status=$(curl -s -m 300 -b "$work_host/cookies" -H 'content-type: application/json' \
  -X POST "$base/__dsh-desktop/components-install" -d null)
feed_alive=$(curl -s -m 10 -b "$work_host/cookies" -H 'content-type: application/json' \
  -X POST "$base/__dsh-desktop/components-status" -d null)

printf '@@DSH@@{"type":"shutdown"}\n' >&9 2>/dev/null || true
exec 9>&- 2>/dev/null || true
for _ in $(seq 1 100); do kill -0 "$host_pid" 2>/dev/null || break; sleep 0.1; done
kill -9 "$host_pid" 2>/dev/null || true
wait "$host_pid" 2>/dev/null
kill "$server_pid" 2>/dev/null

leftovers=$(find "$home/components" -maxdepth 2 \( -name 'download-*' -o -name 'staging-*' \) 2>/dev/null | wc -l)
if printf '%s' "$status" | grep -q '"phase":"error"' \
  && [ ! -e "$home/components/current.json" ] \
  && [ "$leftovers" -eq 0 ] \
  && printf '%s' "$feed_alive" | grep -q '"phase":"error"'; then
  echo "RESULT disk-full=OK status=$status leftovers=$leftovers host=serving"
  exit 0
fi
echo "RESULT disk-full=FAIL status=$status leftovers=$leftovers host=$feed_alive"
exit 1
