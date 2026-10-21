#!/usr/bin/env bash
# Prove the packaged shell checks a real update feed through the Host bridge.
#
# Serves a manifest for a fabricated newer version, boots the release shell with
# `DSH_DESKTOP_UPDATE_FEED` and `DSH_DESKTOP_VERSION`, then asks the bridge for
# status and reads the event stream.
#
# The feed is signed: the probe signs the manifest with a key of its own and
# hands the Host the matching public key, so a tampered manifest must be
# refused instead of installed.
#
#   usage: update-probe.sh
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
target=${DSH_DESKTOP_TARGET:-linux-x64}
tree=$repo/apps/desktop/.desktop-build/targets/$target
mkdir -p "$tree/probe"
work=$(mktemp -d "$tree/probe/update-probe.XXXXXX")
mkdir -p "$work/feed" "$work/home"

head -c 65536 /dev/urandom > "$work/feed/app.bin"
sha=$(sha256sum "$work/feed/app.bin" | cut -d' ' -f1)
size=$(stat -c %s "$work/feed/app.bin")
feed_port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')
python3 -m http.server "$feed_port" --bind 127.0.0.1 --directory "$work/feed" > "$work/feed.log" 2>&1 &
feed_pid=$!
sleep 0.5

cat > "$work/feed/sign.mjs" <<'EOF'
import { generateKeyPairSync, sign } from 'node:crypto'
import { readFileSync, writeFileSync } from 'node:fs'
import { pathToFileURL } from 'node:url'

const [manifestPath, keyPath, signingModule] = process.argv.slice(2)
const { signedFeedBytes } = await import(pathToFileURL(signingModule).href)
const document = JSON.parse(readFileSync(manifestPath, 'utf8'))
const { privateKey, publicKey } = generateKeyPairSync('ed25519')
const signature = sign(null, signedFeedBytes(document), privateKey).toString('base64')
writeFileSync(manifestPath, JSON.stringify({ ...document, signature }))
writeFileSync(keyPath, publicKey.export({ type: 'spki', format: 'pem' }).toString())
EOF

cat > "$work/feed/manifest.json" <<EOF
{"schemaVersion":2,"version":"9.9.9","artifacts":{"$target":{"url":"http://127.0.0.1:$feed_port/app.bin","sha256":"$sha","size":$size}}}
EOF

bunx tsx "$work/feed/sign.mjs" "$work/feed/manifest.json" "$work/feed/public.pem" \
  "$repo/apps/desktop-host/src/update/signing.ts"

# The Host reads the pinned public key from the environment as PEM text, not as
# a path to it.
public_key=$(cat "$work/feed/public.pem")

shell_pid=""

cleanup() {
  [ -n "$shell_pid" ] && kill -9 "$shell_pid" 2>/dev/null
  kill -9 "$feed_pid" 2>/dev/null
  ps -eo pid,args --no-headers | grep -F "release/dsh-desktop-tauri" | grep -v grep | awk '{print $1}' \
    | while read -r pid; do kill -9 "$pid" 2>/dev/null; done
  true
}
trap cleanup EXIT

# Boot the release shell, wait for its bridge, and print the update status it
# reports for the feed the manifest currently describes.
run_shell() {
  local label=$1
  local run="$work/$label"
  mkdir -p "$run/home"

  env DSH_DESKTOP_HOST_BIN="$tree/bin/desktop-host" \
    DSH_DESKTOP_RUNTIME_DIR="$tree/dsh" \
    DSH_DESKTOP_PROJECT_DIR="$tree/project" \
    DSH_DESKTOP_PRIMARY_RUNTIME="$tree/runtime/primary-runtime" \
    DSH_DESKTOP_PNPM_ENTRY="$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" \
    DSH_DESKTOP_NODE_BIN="$tree/runtime/bin" \
    DSH_DESKTOP_UPDATE_FEED="http://127.0.0.1:$feed_port/manifest.json" \
    DSH_DESKTOP_UPDATE_PUBLIC_KEY="$public_key" \
    DSH_DESKTOP_VERSION="1.0.0" \
    GDK_BACKEND=x11 \
    DSH_HOME="$run/home" \
    "$repo/apps/desktop-tauri/src-tauri/target/release/dsh-desktop-tauri" \
    > "$run/out" 2> "$run/err" &
  shell_pid=$!

  local url=""
  for _ in $(seq 1 200); do
    url=$(grep -o 'http://127\.0\.0\.1:[0-9]*/?token=[A-Za-z0-9_-]*' "$run/err" 2>/dev/null | head -1)
    [ -n "$url" ] && break
    sleep 0.5
  done

  if [ -z "$url" ]; then
    echo "RESULT ready=FAIL label=$label"
    tail -20 "$run/err"
    return 1
  fi

  local port base
  port=$(printf '%s' "$url" | sed -E 's#.*:([0-9]+)/.*#\1#')
  base="http://127.0.0.1:$port"
  echo "RESULT ready=OK label=$label port=$port"
  curl -s -L -c "$run/cookies" -b "$run/cookies" -o /dev/null "$url"

  # The automatic check the shell runs at boot answers the status the settings
  # page renders.
  local status
  for _ in $(seq 1 60); do
    status=$(curl -s -m 30 -b "$run/cookies" -H 'content-type: application/json' \
      -X POST "$base/__dsh-desktop/updates-status" -d 'null')
    case "$status" in
      *'"phase":"available"'*|*'"phase":"error"'*|*'"phase":"idle"'*) break ;;
    esac
    sleep 0.5
  done
  echo "status_$label=$status"

  # The client subscribes once; the first frame must arrive without a reload.
  timeout 10 curl -s -N -b "$run/cookies" "$base/__dsh-desktop/updates-events" | head -2

  kill -9 "$shell_pid" 2>/dev/null
  wait "$shell_pid" 2>/dev/null
  shell_pid=""
}

run_shell signed

# A manifest whose signature does not match the key the Host trusts must be
# refused: the feed is only as good as its signature.
python3 - "$work/feed/manifest.json" <<'EOF'
import json, sys
path = sys.argv[1]
document = json.loads(open(path).read())
signature = document.get('signature', '')
document['signature'] = ('A' if signature[:1] != 'A' else 'B') + signature[1:]
open(path, 'w').write(json.dumps(document))
EOF

run_shell tampered

echo "feed_requests=$(wc -l < "$work/feed.log")"
