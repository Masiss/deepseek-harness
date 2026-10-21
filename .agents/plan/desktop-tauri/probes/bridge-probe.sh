#!/usr/bin/env bash
# P2 probe: boot the Desktop Host from source and exercise the desktop bridge
# over the real HTTP surface — the token fence, the device description, the
# shortcut document, and the update phase.
set -u

repo=/home/masis/deepseek-harness
work=$(mktemp -d /tmp/opencode/bridge.XXXXXX)
home=$(mktemp -d /tmp/opencode/bridge-home.XXXXXX)
mkfifo "$work/in"

DSH_HOME="$home" node --import "$repo/node_modules/tsx/dist/esm/index.mjs" "$repo/apps/desktop-host/src/tauri.ts" \
  "$HOME/.dsh/profiles" "$HOME/.dsh/profiles/web" \
  < "$work/in" > "$work/out" 2> "$work/err" &
pid=$!
exec 3> "$work/in"

url=""
for _ in $(seq 1 120); do
  url=$(grep -o 'http://127\.0\.0\.1:[0-9]*/?token=[A-Za-z0-9_-]*' "$work/out" 2>/dev/null | head -1)
  [ -n "$url" ] && break
  sleep 0.25
done

if [ -z "$url" ]; then
  echo "RESULT ready=FAIL"
  tail -20 "$work/err"
  kill "$pid" 2>/dev/null
  exit 1
fi

port=$(printf '%s' "$url" | sed -E 's#.*:([0-9]+)/.*#\1#')
base="http://127.0.0.1:$port"
bridge="$base/__dsh-desktop"
echo "RESULT ready=OK port=$port"

# 1. The fence: no cookie, no bridge. This is the Tauri-command-equivalent check.
printf 'anon_device_info=%s anon_shortcuts_get=%s\n' \
  "$(curl -s -o /dev/null -w '%{http_code}' -X POST "$bridge/device-info" -d null)" \
  "$(curl -s -o /dev/null -w '%{http_code}' -X POST "$bridge/shortcuts-get" -d null)"

# 2. Token handoff mints the cookie the page's fetch uses.
curl -s -L -c "$work/cookies" -b "$work/cookies" -o "$work/index.html" "$url"
echo "index_has_shim=$(grep -c 'globalThis.dshDesktop' "$work/index.html")"
echo "index_has_platform=$(grep -c 'dataset.platform' "$work/index.html")"

J='content-type: application/json'

# 3. Device description.
echo "device_info=$(curl -s -b "$work/cookies" -H "$J" -X POST "$bridge/device-info" -d null)"

# 4. Shortcut document: read, edit, read back.
snapshot=$(curl -s -b "$work/cookies" -H "$J" -X POST "$bridge/shortcuts-get" \
  -d '{"definitions":[{"id":"app.palette","defaults":{}}]}')
revision=$(printf '%s' "$snapshot" | sed -E 's/.*"revision":"([^"]+)".*/\1/')
echo "shortcuts_get_status=$(printf '%s' "$snapshot" | sed -E 's/^\{"status":"([^"]+)".*/\1/')"

edit=$(curl -s -b "$work/cookies" -H "$J" -X POST "$bridge/shortcuts-edit" \
  -d "{\"edit\":{\"type\":\"set\",\"id\":\"app.palette\",\"binding\":{\"code\":\"KeyK\",\"modifiers\":[\"control\"]}},\"revision\":\"$revision\"}")
echo "shortcuts_edit_status=$(printf '%s' "$edit" | sed -E 's/^\{"status":"([^"]+)".*/\1/')"
echo "keybindings_file=$(test -f "$home/keybindings.json" && echo present || echo missing)"
echo "keybindings_has_code=$(grep -c KeyK "$home/keybindings.json" 2>/dev/null || echo 0)"

# 5. Update phase.
echo "updates_status=$(curl -s -b "$work/cookies" -H "$J" -X POST "$bridge/updates-status" -d null)"

# 6. The method fence.
printf 'get_status=%s unknown_status=%s\n' \
  "$(curl -s -o /dev/null -w '%{http_code}' -b "$work/cookies" "$bridge/device-info")" \
  "$(curl -s -o /dev/null -w '%{http_code}' -b "$work/cookies" -H "$J" -X POST "$bridge/absent" -d null)"

printf '@@DSH@@{"type":"shutdown"}\n' >&3
for _ in $(seq 1 40); do
  kill -0 "$pid" 2>/dev/null || break
  sleep 0.25
done
kill -9 "$pid" 2>/dev/null
exec 3>&-
echo "home=$home"
