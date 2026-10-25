#!/usr/bin/env bash
# Verify the on-demand Office component against the prepared tree.
#
# The bundle carries no Python payload; the Host installs it from a release
# feed on request. This probe serves the tree's own `components/` directory over
# loopback, asks the Host to install, checks the composed runtime, and restarts
# the Host to prove the Office plugin only composes once the component exists.
#
# Run it under isolated-run.sh; DSH_ISOLATION_DIR must be writable.
#
#   usage: DSH_ISOLATION_DIR=... isolated-run.sh bash office-component-probe.sh
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
target=${DSH_DESKTOP_TARGET:-linux-x64}
tree="$repo/apps/desktop/.desktop-build/targets/$target"
# DSH_OFFICE_ROOT points the probe at an extracted package (AppImage, .deb);
# DSH_OFFICE_COMPONENTS names the directory served as the component feed.
root=${DSH_OFFICE_ROOT:-$tree}
components=${DSH_OFFICE_COMPONENTS:-$tree/components}
[ -f "$components/components.json" ] || { echo "RESULT components=missing"; exit 1; }

work=${DSH_ISOLATION_DIR:?set DSH_ISOLATION_DIR}
home="$work/home/.dsh"
profile="$home/desktop-project"
server=''
market_server=''
cleanup_probe_servers() {
  local pid
  for pid in "$market_server" "$server"; do
    [ -n "$pid" ] && kill -TERM "$pid" 2>/dev/null || true
  done
  for pid in "$market_server" "$server"; do
    [ -n "$pid" ] && wait "$pid" 2>/dev/null || true
  done
}
trap cleanup_probe_servers EXIT
rm -rf "$work/home" "$work/tmp" "$work/cache" "$work/config"
mkdir -p "$work/home" "$work/tmp" "$work/cache" "$work/config" "$home" "$work/components"
cp -a "$root/project" "$profile"
# The Host verifies the component manifest with the release key, so this
# rehearsal signs the tree's manifest with an ephemeral key and hands the Host
# the matching public key.
cp -a "$components/." "$work/components/"
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
"$root/runtime/bin/node" "$work/sign.mjs" \
  "$work/components/components.json" "$work/components/components.json" "$work/public.pem"

echo "RESULT bundle_python=$([ -d "$root/runtime/primary-runtime/dependencies/python" ] && echo present || echo absent)"
echo "RESULT component=$("$root/runtime/bin/node" -e "const m=require('$work/components/components.json');process.stdout.write(m.components[0].file+' bytes='+m.components[0].bytes)")"

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

"$root/runtime/bin/node" "$work/serve.mjs" "$work/components" "$work/port" &
server=$!
for _ in $(seq 1 40); do [ -s "$work/port" ] && break; sleep 0.1; done
feed="http://127.0.0.1:$(cat "$work/port")"

mkfifo "$work/in"
cat > "$work/run.sh" <<EOF
#!/bin/sh
echo \$\$ > "$work/pid"
exec env -i PATH=/usr/bin:/bin HOME="$work/home" TMPDIR="$work/tmp" \\
  XDG_CACHE_HOME="$work/cache" XDG_CONFIG_HOME="$work/config" DSH_HOME="$home" \\
  DSH_DESKTOP_RUNTIME_DIR="$root/dsh" DSH_DESKTOP_PROJECT_DIR="$profile" \\
  DSH_DESKTOP_PRIMARY_RUNTIME="$root/runtime/primary-runtime" \\
  DSH_DESKTOP_PNPM_ENTRY="$root/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" \\
  DSH_DESKTOP_NODE_BIN="$root/runtime/bin" DSH_DESKTOP_VERSION=1.0.0 \\
  DSH_DESKTOP_COMPONENT_FEED="$feed" \\
  DSH_DESKTOP_COMPONENT_PUBLIC_KEY="$(cat "$work/public.pem")" \\
  "$root/bin/desktop-host" "$root/dsh" "$profile" "$root/runtime/primary-runtime" \\
  "$root/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" "$root/runtime/bin"
EOF
chmod +x "$work/run.sh"

start_host() {
  rm -f "$work/out" "$work/err" "$work/pid"
  setsid "$work/run.sh" < "$work/in" > "$work/out" 2> "$work/err" &
  sleep 0.5
  pgid=$(cat "$work/pid" 2>/dev/null || true)
  exec 3> "$work/in"
  url=""
  for _ in $(seq 1 160); do
    url=$(grep -o 'http://127\.0\.0\.1:[0-9]*/?token=[A-Za-z0-9_-]*' "$work/out" 2>/dev/null | head -1)
    [ -n "$url" ] && break
    sleep 0.5
  done
  [ -n "$url" ] || { echo "RESULT host=FAIL"; tail -5 "$work/err"; return 1; }
  base="http://127.0.0.1:$(printf '%s' "$url" | sed -E 's#.*:([0-9]+)/.*#\1#')"
  curl -s -L -c "$work/cookies" -b "$work/cookies" -o /dev/null "$url"
}

stop_host() {
  printf '@@DSH@@{"type":"shutdown"}\n' >&3
  exec 3>&-
  for _ in $(seq 1 40); do kill -0 "$pgid" 2>/dev/null || break; sleep 0.5; done
  kill -KILL -- "-$pgid" 2>/dev/null
}

rpc() {
  local method=$1 args=$2
  curl -s -m 300 -b "$work/cookies" -H 'content-type: application/json' \
    -X POST "$base/api/$method" \
    -d "{\"type\":\"client-request\",\"rpcId\":\"office-$RANDOM\",\"method\":\"$method\",\"payload\":{\"args\":$args}}"
}

# --- first run: component absent, install on request ----------------------
start_host || { kill "$server" 2>/dev/null; exit 1; }
echo "status_initial=$(curl -s -m 20 -b "$work/cookies" -H 'content-type: application/json' -X POST "$base/__dsh-desktop/components-status" -d null)"
echo "status_install=$(curl -s -m 600 -b "$work/cookies" -H 'content-type: application/json' -X POST "$base/__dsh-desktop/components-install" -d null)"
echo "restart_frame=$([ "$(grep -c '"type":"restart"' "$work/out" 2>/dev/null)" -gt 0 ] && echo yes || echo no)"
version=$("$root/runtime/bin/node" -e "try{process.stdout.write(JSON.parse(require('node:fs').readFileSync('$home/components/current.json','utf8')).version)}catch{}")
python="$home/components/versions/$version/runtime/dependencies/python/bin/python3"
echo "installed_version=$version python_exec=$([ -x "$python" ] && echo yes || echo no)"
echo "python_imports=$("$python" -c 'import numpy, pandas, openpyxl, docx, pptx, PIL, lxml' >/dev/null 2>&1 && echo ok || echo FAIL)"
echo "composed_runtime_json=$([ -f "$home/components/versions/$version/runtime/runtime.json" ] && echo yes || echo no)"
echo "composed_engine=$([ -f "$home/components/versions/$version/runtime/node_modules/@deepseek-ai/libreoffice-kit/lib/cli.js" ] && echo yes || echo no)"
echo "bundle_engine=$([ -d "$root/dsh/node_modules/@deepseek-ai/libreoffice-kit-wasm" ] && echo present || echo absent)"
echo "node_link=$(readlink "$home/components/versions/$version/runtime/dependencies/node")"
stop_host

# --- second run: the installed component composes Office ------------------
start_host || { kill "$server" 2>/dev/null; exit 1; }
echo "status_restarted=$(curl -s -m 20 -b "$work/cookies" -H 'content-type: application/json' -X POST "$base/__dsh-desktop/components-status" -d null)"

# A Session is the workspace scope the Host authorizes an Office path against,
# so the conversion journey creates one whose cwd holds the document.
mkdir -p "$work/workspace"
cat > "$work/workspace/probe.txt" <<'EOF'
Workspace Explorer read-only preview fixture.
EOF
session=$(rpc session/create "{\"request\":{\"cwd\":\"$work/workspace\"}}" \
  | "$root/runtime/bin/node" -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{const v=JSON.parse(s).result?.value;process.stdout.write(v?.sessionId??"")})')
echo "render_session=${session:-none}"
"$python" -c 'import docx, sys; d = docx.Document(); d.add_paragraph("packaged office probe"); d.save(sys.argv[1])' \
  "$work/workspace/probe.docx" 2>/dev/null
if [ -n "$session" ] && [ -f "$work/workspace/probe.docx" ]; then
  # A binary RPC answer is multipart/form-data: text `metadata` holds the
  # server-response envelope and the document bytes ride as a file part.
  render_http_status=$(curl -sS -m 300 -w '%{http_code}' -b "$work/cookies" -H 'content-type: application/json' \
    -X POST "$base/api/officeToPdf/render" \
    -d "{\"type\":\"client-request\",\"rpcId\":\"office-render\",\"method\":\"officeToPdf/render\",\"payload\":{\"args\":{\"workspaceFileScopeId\":\"$session\",\"path\":\"probe.docx\",\"priority\":\"foreground\"}}}" \
    -o "$work/render.bin" -D "$work/render.hdr" 2>"$work/render.curl.err" || true)
  echo "office_render_http_status=${render_http_status:-none}"
  if [ ! -s "$work/render.hdr" ]; then
    echo "office_render_curl_error=$(cat "$work/render.curl.err" 2>/dev/null | tail -1)"
    tail -10 "$work/err" 2>/dev/null || true
  fi
  "$root/runtime/bin/node" -e '
    const fs = require("node:fs")
    const [headerFile, bodyFile] = process.argv.slice(1)
    const header = fs.readFileSync(headerFile, "utf8").split(/\r?\n/)
      .find(line => line.toLowerCase().startsWith("content-type:")) ?? ""
    const boundary = /boundary=(?:"([^"]+)"|([^;]+))/.exec(header)
    if (boundary === null) {
      console.log(`RESULT office-render=FAIL reason=not-multipart header=${header.slice(0, 80)}`)
      process.exit(0)
    }
    const marker = Buffer.from(`--${boundary[1] ?? boundary[2]}`)
    const body = fs.readFileSync(bodyFile)
    const parts = []
    let offset = body.indexOf(marker)
    while (offset !== -1) {
      const start = offset + marker.length
      if (body.subarray(start, start + 2).toString() === "--") break
      const next = body.indexOf(marker, start)
      if (next === -1) break
      const raw = body.subarray(start + 2, next - 2)
      const split = raw.indexOf("\r\n\r\n")
      parts.push({
        headers: raw.subarray(0, split).toString("utf8"),
        body: raw.subarray(split + 4),
      })
      offset = next
    }
    const metadata = parts.find(part => /name="metadata"/.test(part.headers))
    if (metadata === undefined) {
      console.log("RESULT office-render=FAIL reason=no-metadata")
      process.exit(0)
    }
    const envelope = JSON.parse(metadata.body.toString("utf8"))
    if (envelope.result?.ok !== true) {
      console.log(`RESULT office-render=FAIL error=${JSON.stringify(envelope.result?.error ?? null).slice(0, 200)}`)
      process.exit(0)
    }
    const file = parts.find(part => part !== metadata && part.body.subarray(0, 5).toString("latin1") === "%PDF-")
    const missing = (envelope.result.value?.missingFonts ?? []).length
    if (file === undefined) {
      console.log(`RESULT office-render=FAIL reason=no-pdf-part parts=${parts.length}`)
      process.exit(0)
    }
    console.log(`RESULT office-render=OK bytes=${file.body.length} missing_fonts=${missing}`)
  ' "$work/render.hdr" "$work/render.bin"
else
  echo "RESULT office-render=SKIP reason=no-session-or-document"
fi
stop_host

if [ "${DSH_OFFICE_PANEL_TEST:-0}" = "1" ]; then
  # The existing UI harness talks to this packaged Host and to a loopback
  # registry; no UI success is inferred from a direct API-only install.
  package_dir="$work/market-package/package"
  mkdir -p "$package_dir"
  cp "$repo/apps/desktop-tauri/scripts/load-probe/package.json" \
    "$repo/apps/desktop-tauri/scripts/load-probe/index.js" \
    "$repo/apps/desktop-tauri/scripts/load-probe/cordis.patch.yml" "$package_dir/"
  tarball="$work/dsh-load-probe.tgz"
  tar -czf "$tarball" -C "$work/market-package" package
  market_name=$("$root/runtime/bin/node" -p "require('$repo/apps/desktop-tauri/scripts/load-probe/package.json').name")
  market_version=$("$root/runtime/bin/node" -p "require('$repo/apps/desktop-tauri/scripts/load-probe/package.json').version")
  cat > "$work/market-registry.mjs" <<'MARKET'
import { createHash } from 'node:crypto'
import { createReadStream, readFileSync, statSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:http'
const [tarball, manifestPath, name, version, portFile] = process.argv.slice(2)
const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'))
const bytes = readFileSync(tarball)
const shasum = createHash('sha1').update(bytes).digest('hex')
const integrity = `sha512-${createHash('sha512').update(bytes).digest('base64')}`
const encoded = name.replace('/', '%2F')
const tarballName = `${name.split('/')[1]}-${version}.tgz`
const server = createServer((request, response) => {
  const path = decodeURIComponent(new URL(request.url ?? '/', 'http://127.0.0.1').pathname)
  if (path === '/index.json') {
    const base = `http://127.0.0.1:${server.address().port}/`
    response.writeHead(200, { 'content-type': 'application/json' })
    response.end(JSON.stringify({
      format: 1,
      title: 'Native panel probe',
      plugins: [{
        name,
        title: 'Native panel probe bundle',
        description: 'Installed through the native Marketplace panel.',
        versions: [{
          version,
          publishTimestamp: Math.floor(Date.now() / 1000),
          source: { type: 'npm', registry: base },
        }],
      }],
    }))
    return
  }
  if (path === `/tarballs/${tarballName}`) {
    const size = statSync(tarball).size
    response.writeHead(200, { 'content-type': 'application/octet-stream', 'content-length': size })
    createReadStream(tarball).pipe(response)
    return
  }
  if (path === `/${name}` || request.url?.startsWith(`/${encoded}`)) {
    const base = `http://127.0.0.1:${server.address().port}`
    response.writeHead(200, { 'content-type': 'application/json' })
    response.end(JSON.stringify({
      name,
      'dist-tags': { latest: version },
      versions: {
        [version]: {
          ...manifest,
          dist: { tarball: `${base}/tarballs/${tarballName}`, shasum, integrity },
        },
      },
    }))
    return
  }
  response.writeHead(404, { 'content-type': 'application/json' }).end('{}')
})
server.listen(0, '127.0.0.1', () => writeFileSync(portFile, String(server.address().port)))
MARKET
  market_port_file="$work/market.port"
  "$root/runtime/bin/node" "$work/market-registry.mjs" \
    "$tarball" "$repo/apps/desktop-tauri/scripts/load-probe/package.json" \
    "$market_name" "$market_version" "$market_port_file" \
    > "$work/market.out" 2> "$work/market.err" &
  market_server=$!
  for _ in $(seq 1 100); do [ -s "$market_port_file" ] && break; sleep 0.1; done
  market_port=$(cat "$market_port_file" 2>/dev/null || echo 0)
  [ "$market_port" != 0 ] || { echo "RESULT native-ui-panels=FAIL reason=market-registry"; tail -10 "$work/market.err"; exit 1; }

  cat > "$profile/cordis.patch.yml" <<PATCH
- insert:
    - id: plugin-market
      name: '@deepseek-ai/dsh-plugin-market'
      config:
        indexes:
          - http://127.0.0.1:$market_port/index.json
PATCH
  cat > "$work/workspace/tui_probe.js" <<'TUI'
const ESC = '\x1b'
const input = process.stdin
input.setRawMode(true)
input.resume()
let data = Buffer.alloc(0)
let pasteSeen = false
let mouseSeen = false
process.stdout.write(`${ESC}[?1049h${ESC}[?2004h${ESC}[?1002h${ESC}[?1006h\r\nTUI_READY\r\n`)
input.on('data', chunk => {
  data = Buffer.concat([data, chunk])
  const text = data.toString('utf8')
  if (!pasteSeen && text.includes(`${ESC}[200~native paste\nround trip${ESC}[201~`)) {
    pasteSeen = true
    process.stdout.write('\r\nPASTE_OK\r\n')
  }
  if (!mouseSeen && /\x1b\[<0;(\d+);(\d+)M\x1b\[<0;\1;\2m/.test(text)) {
    mouseSeen = true
    process.stdout.write('\r\nMOUSE_OK\r\n')
  }
  if (pasteSeen && mouseSeen) {
    process.stdout.write(`${ESC}[?2004l${ESC}[?1002l${ESC}[?1006l${ESC}[?1049l\r\nTUI_DONE\r\n`, () => {
      input.setRawMode(false)
      process.exit(0)
    })
  }
  if (data.length > 4096) data = data.subarray(-1024)
})
TUI

  test_bin=${DSH_NATIVE_UI_PANEL_TEST_BIN:-}
  if [ -z "$test_bin" ]; then
    test_bin=$(find "$repo/apps/desktop-tauri/native-app/target/debug/deps" -maxdepth 1 \
      -type f -name 'dsh_desktop_native_app-*' -executable -printf '%T@ %p\n' \
      | sort -nr | sed -n '1s/^[^ ]* //p')
  fi
  [ -x "$test_bin" ] || { echo "RESULT native-ui-panels=FAIL reason=test-binary"; exit 1; }
  host_log_wrapper="$work/desktop-host-log-wrapper"
  cat > "$host_log_wrapper" <<WRAPPER
#!/bin/sh
"$root/bin/desktop-host" "\$@" 2>>"$work/ui-host.err"
status=\$?
echo "desktop-host-exit=\$status" >>"$work/ui-host.err"
exit "\$status"
WRAPPER
  chmod +x "$host_log_wrapper"
  DSH_HOME="$home" DSH_DESKTOP_HOST_BIN="$host_log_wrapper" \
    DSH_DESKTOP_RUNTIME_DIR="$root/dsh" DSH_DESKTOP_PROJECT_DIR="$profile" \
    DSH_DESKTOP_PRIMARY_RUNTIME="$root/runtime/primary-runtime" \
    DSH_DESKTOP_PNPM_ENTRY="$root/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" \
    DSH_DESKTOP_NODE_BIN="$root/runtime/bin" DSH_DESKTOP_VERSION=1.0.0 \
    DSH_CLIENT_VERSION=1.0.0 DSH_NATIVE_UI_PANEL_SESSION="$session" \
    DSH_NATIVE_UI_PANEL_PACKAGE="$market_name" \
    "$test_bin" --ignored --exact \
      ui_harness::packaged_host_marketplace_office_and_terminal_panels_round_trip --nocapture
  result=$?
  if [ "$result" -ne 0 ]; then
    echo "RESULT native-ui-panels=FAIL"
    tail -80 "$work/ui-host.err" 2>/dev/null || true
    exit "$result"
  fi
  echo "RESULT native-ui-panels=OK market=$market_name session=$session"
fi

echo "RESULT done=1"
