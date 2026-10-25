#!/usr/bin/env bash
# Exercise profile-default selection through the native picker and packaged Host.
#
# Run under isolated-run.sh; the fake account grant, /models endpoint, Host,
# profile and Session all belong to DSH_ISOLATION_DIR.
set -euo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
tree="$repo/apps/desktop/.desktop-build/targets/${DSH_DESKTOP_TARGET:-linux-x64}"
runtime_node="$tree/runtime/bin/node"
test_bin=${DSH_NATIVE_UI_MODEL_TEST_BIN:-}
work=${DSH_ISOLATION_DIR:?set DSH_ISOLATION_DIR}
home="$work/home/.dsh"
profile="$home/desktop-project"
for required in "$runtime_node" "$tree/bin/desktop-host" "$tree/runtime/primary-runtime" "$tree/dsh/node_modules/@deepseek-ai/dsh-desktop-host/package.json"; do
  [ -e "$required" ] || { echo "model-default-probe: missing $required" >&2; exit 1; }
done

mkdir -p "$home"
cp -a "$tree/project" "$profile"
chmod -R u+w "$profile"

cat > "$home/.credentials.yaml" <<'CREDENTIAL'
version: 1
records:
  deepseek-account-platform/default:
    kind: grant
    payload:
      version: 1
      token: local-probe-token
      issuer: https://platform.deepseek.com
CREDENTIAL
chmod 600 "$home/.credentials.yaml"

cat > "$work/models-server.mjs" <<'MODELS'
import { createServer } from 'node:http'
import { writeFileSync } from 'node:fs'
const [portFile, hitFile] = process.argv.slice(2)
const listing = {
  data: [{
    id: 'deepseek-flash', name: 'DeepSeek-V4.1-Flash',
    context_window: 1048576, max_output_tokens: 393216,
    input_modalities: ['text'], output_modalities: ['text'],
    api_capabilities: { anthropic_messages: {} },
  }],
}
const server = createServer((request, response) => {
  if (request.method === 'GET' && new URL(request.url ?? '/', 'http://127.0.0.1').pathname === '/models') {
    writeFileSync(hitFile, JSON.stringify({
      authorization: typeof request.headers.authorization === 'string',
      cookie: typeof request.headers.cookie === 'string',
    }))
    response.writeHead(200, { 'content-type': 'application/json' })
    response.end(JSON.stringify(listing))
    return
  }
  response.writeHead(404).end()
})
server.listen(0, '127.0.0.1', () => {
  const address = server.address()
  if (address === null || typeof address === 'string') throw new Error('loopback listener did not bind TCP')
  writeFileSync(portFile, String(address.port))
})
MODELS

# The loopback port is selected atomically by the server and written into this
# disposable profile only after the listener reports its bound address.
port_file="$work/models.port"
hit_file="$work/models.hit"
"$runtime_node" "$work/models-server.mjs" "$port_file" "$hit_file" \
  > "$work/models.out" 2> "$work/models.err" &
models_pid=$!
cleanup() {
  if kill -0 "$models_pid" 2>/dev/null; then kill -TERM "$models_pid" 2>/dev/null || true; fi
  wait "$models_pid" 2>/dev/null || true
}
trap cleanup EXIT
for _ in $(seq 1 100); do [ -s "$port_file" ] && break; sleep 0.1; done
port=$(cat "$port_file" 2>/dev/null || echo 0)
[ "$port" != 0 ] || { echo "RESULT model-default=FAIL reason=models-server"; tail -10 "$work/models.err"; exit 1; }
origin="http://127.0.0.1:$port"
cat > "$profile/cordis.patch.yml" <<PATCH
- id: deepseek-account
  config:
    inferenceOrigin: $origin
    allowLoopbackHttp: true
- id: llm-deepseek-account
  config:
    baseURL: $origin/anthropic
PATCH

if [ -z "$test_bin" ]; then
  test_bin=$(find "$repo/apps/desktop-tauri/native-app/target/debug/deps" -maxdepth 1 \
    -type f -name 'dsh_desktop_native_app-*' -executable -printf '%T@ %p\n' \
    | sort -nr | sed -n '1s/^[^ ]* //p')
fi
[ -x "$test_bin" ] || { echo "model-default-probe: no native test binary" >&2; exit 1; }

DSH_HOME="$home" \
DSH_DESKTOP_HOST_BIN="$tree/bin/desktop-host" \
DSH_DESKTOP_RUNTIME_DIR="$tree/dsh" \
DSH_DESKTOP_PROJECT_DIR="$profile" \
DSH_DESKTOP_PRIMARY_RUNTIME="$tree/runtime/primary-runtime" \
DSH_DESKTOP_PNPM_ENTRY="$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" \
DSH_DESKTOP_NODE_BIN="$tree/runtime/bin" \
DSH_DESKTOP_VERSION=1.0.0 \
DSH_CLIENT_VERSION=1.0.0 \
DEEPSEEK_BASE_URL="$origin/anthropic" \
"$test_bin" --ignored --exact \
  ui_harness::packaged_profile_default_model_round_trips_through_the_native_picker --nocapture

if [ ! -s "$hit_file" ]; then
  echo "RESULT model-default=FAIL reason=Host-did-not-read-loopback-models"
  tail -10 "$work/models.err"
  exit 1
fi
echo "RESULT model-default=OK listing_origin=$origin"
