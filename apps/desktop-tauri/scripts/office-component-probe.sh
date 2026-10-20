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
rm -rf "$work/home" "$work/tmp" "$work/cache" "$work/config"
mkdir -p "$work/home" "$work/tmp" "$work/cache" "$work/config" "$home"
cp -a "$root/project" "$profile"

echo "RESULT bundle_python=$([ -d "$root/runtime/primary-runtime/dependencies/python" ] && echo present || echo absent)"
echo "RESULT component=$("$root/runtime/bin/node" -e "const m=require('$components/components.json');process.stdout.write(m.components[0].file+' bytes='+m.components[0].bytes)")"

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

"$root/runtime/bin/node" "$work/serve.mjs" "$components" "$work/port" &
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
stop_host
kill "$server" 2>/dev/null
echo "RESULT done=1"
