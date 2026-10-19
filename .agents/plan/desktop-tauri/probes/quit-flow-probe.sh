#!/usr/bin/env bash
# P1 probe: drive the tray D-Bus menu to quit, exercising the quit-inspection flow.
set -u

repo=/home/masis/deepseek-harness
bin="$repo/apps/desktop-tauri/src-tauri/target/release/dsh-desktop-tauri"
work=$(mktemp -d /tmp/opencode/quit.XXXXXX)

export DSH_DESKTOP_HOST_BIN="$repo/apps/desktop-tauri/scripts/dev-host.sh"
export DSH_DESKTOP_RUNTIME_DIR="$HOME/.dsh/profiles"
export DSH_DESKTOP_PROJECT_DIR="$HOME/.dsh/profiles/web"
export GDK_BACKEND=x11

before=$(timeout 20 gdbus call --session --dest org.kde.StatusNotifierWatcher \
  --object-path /StatusNotifierWatcher \
  --method org.freedesktop.DBus.Properties.Get \
  org.kde.StatusNotifierWatcher RegisteredStatusNotifierItems 2>/dev/null)

"$bin" > "$work/out" 2> "$work/err" &
pid=$!

port=""
for _ in $(seq 1 80); do
  port=$(grep -o '127\.0\.0\.1:[0-9]*' "$work/err" 2>/dev/null | head -1 | cut -d: -f2)
  [ -n "$port" ] && break
  sleep 0.5
done
echo "app_pid=$pid host_port=${port:-none}"
sleep 4

items=$(timeout 20 gdbus call --session --dest org.kde.StatusNotifierWatcher \
  --object-path /StatusNotifierWatcher \
  --method org.freedesktop.DBus.Properties.Get \
  org.kde.StatusNotifierWatcher RegisteredStatusNotifierItems 2>/dev/null)

# Our item is the one the launch added.
target=""
for entry in $(printf '%s' "$items" | tr ',[]' '\n\n\n\n'); do
  case "$entry" in
    *StatusNotifierItem|*NotificationItem*) ;;
    *) continue ;;
  esac
  case "$before" in
    *"$entry"*) continue ;;
  esac
  entry_clean=$(printf '%s' "$entry" | tr -d "'")
  target="${entry_clean%%/*} /${entry_clean#*/}"
done

if [ -z "$target" ]; then
  echo "RESULT tray_item=not_found"
  kill "$pid" 2>/dev/null
  exit 1
fi
echo "RESULT tray_item=$target"

bus=${target%% *}
path=${target#* }
menupath="$path/Menu"
echo "menu_path=$menupath"
timeout 15 gdbus introspect --session --dest "$bus" --object-path "$menupath" 2>/dev/null \
  | grep -E "interface|method|Event|GetLayout" | head -10

layout=$(timeout 15 gdbus call --session --dest "$bus" --object-path "$menupath" \
  --method com.canonical.dbusmenu.GetLayout 0 -1 "[]" 2>/dev/null)
echo "layout_head=$(printf '%s' "$layout" | head -c 400)"

# Menu item ids are the first integer of each node; quit is the last entry.
quit_id=$(printf '%s' "$layout" | grep -oE '[0-9]+' | tail -1)
echo "quit_candidate_id=$quit_id"
timeout 15 gdbus call --session --dest "$bus" --object-path "$menupath" \
  --method com.canonical.dbusmenu.Event "$quit_id" clicked "<''>" 0 2>&1 | head -3

for _ in $(seq 1 40); do
  kill -0 "$pid" 2>/dev/null || break
  sleep 0.25
done
if kill -0 "$pid" 2>/dev/null; then
  echo "RESULT quit=still_running"
  kill "$pid" 2>/dev/null
else
  wait "$pid" 2>/dev/null
  echo "RESULT quit=exited code=$?"
fi
echo "host_connections_after=$(ss -tn 2>/dev/null | grep -c ":${port}")"
echo "work=$work"
