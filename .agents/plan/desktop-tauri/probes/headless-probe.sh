#!/usr/bin/env bash
# Headless P0 probe: boot the Desktop Host with no GUI, then exercise the HTTP
# and WebSocket surfaces a WebView would use.
set -u

repo=/home/masis/deepseek-harness
work=$(mktemp -d /tmp/opencode/headless.XXXXXX)
mkfifo "$work/in"

node "$repo/apps/desktop-host/lib/tauri.js" \
  "$HOME/.dsh/profiles" "$HOME/.dsh/profiles/web" \
  < "$work/in" > "$work/out" 2> "$work/err" &
pid=$!
exec 3> "$work/in"

url=""
for _ in $(seq 1 80); do
  url=$(grep -o 'http://127\.0\.0\.1:[0-9]*/?token=[A-Za-z0-9_-]*' "$work/out" 2>/dev/null | head -1)
  [ -n "$url" ] && break
  sleep 0.25
done

if [ -z "$url" ]; then
  echo "RESULT ready=FAIL"
  tail -5 "$work/err"
  kill "$pid" 2>/dev/null
  exit 1
fi

port=$(printf '%s' "$url" | sed -E 's#.*:([0-9]+)/.*#\1#')
token=$(printf '%s' "$url" | sed -E 's#.*token=##')
base="http://127.0.0.1:$port"
echo "RESULT ready=OK port=$port token_len=${#token}"

# 1. Unauthenticated root: the token fence must not serve the app.
printf 'anon_status=%s anon_location_has_token=%s\n' \
  "$(curl -s -o /dev/null -w '%{http_code}' "$base/")" \
  "$(curl -s -o /dev/null -D - "$base/" | grep -ci 'location:.*token=')"

# 2. Token handoff: expect a cookie and the boot payload.
curl -s -L -c "$work/cookies" -b "$work/cookies" -o "$work/index.html" -w 'auth_status=%{http_code}\n' "$url"
echo "auth_sets_cookie=$(grep -c dsh "$work/cookies" 2>/dev/null)"
echo "index_has_boot_payload=$(grep -c '__DSH_BOOT__' "$work/index.html")"
echo "index_has_app_title=$(grep -ci 'deepseek' "$work/index.html")"

# 3. WebSocket multiplexer: a 101 proves the live channel is usable.
ws=$(curl -s -i -m 4 -b "$work/cookies" \
  -H 'Connection: Upgrade' -H 'Upgrade: websocket' \
  -H 'Sec-WebSocket-Version: 13' -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' \
  "$base/api/remote.mux" 2>/dev/null | head -1)
echo "mux_status_line=${ws:-<none>}"

# 4. A session written by the current Electron build must be readable.
sessions=$(find "$HOME/.dsh/sessions" -name 'session*.jsonl.zstd' 2>/dev/null | wc -l)
echo "session_files_on_disk=$sessions"
echo "session_formats=$(find "$HOME/.dsh/sessions" -name 'session*.jsonl.zstd' 2>/dev/null | sed -E 's#.*/session(\.v[0-9]+)?\.jsonl\.zstd#\1#' | sort -u | tr '\n' ' ')"

printf '@@DSH@@{"type":"shutdown"}\n' >&3
for _ in $(seq 1 40); do
  kill -0 "$pid" 2>/dev/null || break
  sleep 0.25
done
if kill -0 "$pid" 2>/dev/null; then
  echo "RESULT shutdown=TIMEOUT"
  kill "$pid" 2>/dev/null
else
  wait "$pid"
  echo "RESULT shutdown=OK exit=$?"
fi
echo "work=$work"
