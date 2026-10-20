#!/usr/bin/env bash
# Prove the packaged shell checks a real update feed through the Host bridge.
#
# Serves a manifest for a fabricated newer version, boots the release shell with
# `DSH_DESKTOP_UPDATE_FEED` and `DSH_DESKTOP_VERSION`, then asks the bridge for
# status, triggers the manual check, and reads the event stream.
#
#   usage: update-probe.sh
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
target=${DSH_DESKTOP_TARGET:-linux-x64}
tree=$repo/apps/desktop/.desktop-build/targets/$target
work=$(mktemp -d /tmp/opencode/update-probe.XXXXXX)
mkdir -p "$work/feed" "$work/home"

head -c 65536 /dev/urandom > "$work/feed/app.bin"
sha=$(sha256sum "$work/feed/app.bin" | cut -d' ' -f1)
size=$(stat -c %s "$work/feed/app.bin")
feed_port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1]); s.close()')
python3 -m http.server "$feed_port" --bind 127.0.0.1 --directory "$work/feed" > "$work/feed.log" 2>&1 &
feed_pid=$!
sleep 0.5

cat > "$work/feed/manifest.json" <<EOF
{"schemaVersion":1,"version":"9.9.9","artifacts":{"$target":{"url":"http://127.0.0.1:$feed_port/app.bin","sha256":"$sha","size":$size}}}
EOF

env DSH_DESKTOP_HOST_BIN="$tree/bin/desktop-host" \
  DSH_DESKTOP_RUNTIME_DIR="$tree/dsh" \
  DSH_DESKTOP_PROJECT_DIR="$tree/project" \
  DSH_DESKTOP_PRIMARY_RUNTIME="$tree/runtime/primary-runtime" \
  DSH_DESKTOP_PNPM_ENTRY="$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" \
  DSH_DESKTOP_NODE_BIN="$tree/runtime/bin" \
  DSH_DESKTOP_UPDATE_FEED="http://127.0.0.1:$feed_port/manifest.json" \
  DSH_DESKTOP_VERSION="1.0.0" \
  GDK_BACKEND=x11 \
  DSH_HOME="$work/home" \
  "$repo/apps/desktop-tauri/src-tauri/target/release/dsh-desktop-tauri" \
  > "$work/out" 2> "$work/err" &
shell_pid=$!

cleanup() {
  kill -9 "$shell_pid" "$feed_pid" 2>/dev/null
  ps -eo pid,args --no-headers | grep -F "release/dsh-desktop-tauri" | grep -v grep | awk '{print $1}' \
    | while read -r pid; do kill -9 "$pid" 2>/dev/null; done
  true
}

url=""
for _ in $(seq 1 200); do
  url=$(grep -o 'http://127\.0\.0\.1:[0-9]*/?token=[A-Za-z0-9_-]*' "$work/err" 2>/dev/null | head -1)
  [ -n "$url" ] && break
  sleep 0.5
done

if [ -z "$url" ]; then
  echo "RESULT ready=FAIL"
  tail -20 "$work/err"
  cleanup
  exit 1
fi

port=$(printf '%s' "$url" | sed -E 's#.*:([0-9]+)/.*#\1#')
base="http://127.0.0.1:$port"
echo "RESULT ready=OK port=$port"
curl -s -L -c "$work/cookies" -b "$work/cookies" -o /dev/null "$url"

bridge() {
  curl -s -m 30 -b "$work/cookies" -H 'content-type: application/json' \
    -X POST "$base/__dsh-desktop/$1" -d "${2:-null}"
}

echo "status_before=$(bridge updates-status)"
echo "open=$(bridge updates-open '{}' | head -c 200)"
echo "status_after=$(bridge updates-status)"

# The client subscribes once; the first frame must arrive without a reload.
timeout 10 curl -s -N -b "$work/cookies" "$base/__dsh-desktop/updates-events" | head -2

echo "feed_requests=$(wc -l < "$work/feed.log")"
cleanup
