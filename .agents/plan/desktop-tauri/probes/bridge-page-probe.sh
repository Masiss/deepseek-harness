#!/usr/bin/env bash
# P2 probe: boot the Desktop Host from source and observe the desktop bridge
# from inside a real browser page (see bridge-page.mjs).
set -u

repo=/home/masis/deepseek-harness
work=$(mktemp -d /tmp/opencode/bridge-page.XXXXXX)
home=$(mktemp -d /tmp/opencode/bridge-page-home.XXXXXX)
mkfifo "$work/in"

DSH_HOME="$home" node "$repo/apps/desktop-host/src/tauri.ts" \
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

echo "RESULT ready=OK"
node "$repo/.agents/plan/desktop-tauri/probes/bridge-page.mjs" "$url" 2>&1 | head -20
echo "keybindings_after_page=$(test -f "$home/keybindings.json" && echo present || echo absent)"

printf '@@DSH@@{"type":"shutdown"}\n' >&3
for _ in $(seq 1 40); do
  kill -0 "$pid" 2>/dev/null || break
  sleep 0.25
done
kill -9 "$pid" 2>/dev/null
exec 3>&-
