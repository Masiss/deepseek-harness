#!/usr/bin/env bash
# P1 probe: the three close paths — tray visible, no tray unacknowledged, no tray acknowledged.
set -u

repo=/home/masis/deepseek-harness
bin="$repo/apps/desktop-tauri/src-tauri/target/debug/dsh-desktop-tauri"
marker="$HOME/.config/ai.deepseek.harness.tauri.dev/background-close-confirmed"
send_close=/tmp/opencode/send-close

cleanup() {
  ps -eo pid,args --no-headers | grep -F "dsh-desktop-tauri" | grep -v grep | awk '{print $1}' \
    | while read -r pid; do kill -9 "$pid" 2>/dev/null; done
  sleep 1
}

run_case() {
  local name=$1 tray=$2 marker_state=$3
  cleanup
  if [ "$marker_state" = "present" ]; then
    mkdir -p "$(dirname "$marker")" && : > "$marker"
  else
    rm -f "$marker"
  fi

  local work
  work=$(mktemp -d /tmp/opencode/close.XXXXXX)
  DSH_DESKTOP_HOST_BIN="$repo/apps/desktop-tauri/scripts/dev-host.sh" \
  DSH_DESKTOP_RUNTIME_DIR="$HOME/.dsh/profiles" \
  DSH_DESKTOP_PROJECT_DIR="$HOME/.dsh/profiles/web" \
  DSH_DESKTOP_TRAY="$tray" \
  GDK_BACKEND=x11 \
    "$bin" > "$work/out" 2> "$work/err" &
  local pid=$!

  local port=""
  for _ in $(seq 1 80); do
    port=$(grep -o '127\.0\.0\.1:[0-9]*' "$work/err" 2>/dev/null | head -1 | cut -d: -f2)
    [ -n "$port" ] && break
    sleep 0.5
  done
  sleep 2

  local state
  state=$("$send_close" "DeepSeek Harness" 2>/dev/null | grep map_state | head -1)
  local alive
  alive=$(kill -0 "$pid" 2>/dev/null && echo yes || echo no)
  local host
  host=$(ss -tn 2>/dev/null | grep -c ":${port}")
  local tray_note
  tray_note=$(grep -c "desktop tray" "$work/err" 2>/dev/null)

  printf '%-28s tray_env=%s marker=%s | %s | alive=%s host_conns=%s tray_log=%s\n' \
    "$name" "$tray" "$marker_state" "${state:-<no window>}" "$alive" "$host" "$tray_note"
}

run_case "tray visible" 1 absent
run_case "no tray, unacknowledged" 0 absent
run_case "no tray, acknowledged" 0 present
cleanup
rm -f "$marker"
echo "done"
