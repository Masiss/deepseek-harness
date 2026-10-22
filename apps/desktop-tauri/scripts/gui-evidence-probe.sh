#!/usr/bin/env bash
# Repeatable Wayland evidence for the packaged Desktop GUI.
#
# Launches the release shell against the linux-x64 target, keeps all
# profile and workspace state private, authenticates the webview, captures its
# pixels, inspects Host headers, and sends one prompt through the RPC protocol.
set -uo pipefail

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
target=${1:-linux-x64}
tree="$repo/apps/desktop/.desktop-build/targets/$target"
# The release build of the shell carries the measurement hooks; the packaged
# AppImage in the tree may predate them.
shell="$repo/apps/desktop-tauri/src-tauri/target/release/dsh-desktop-tauri"
[ -x "$shell" ] || { echo "RESULT shell=missing path=$shell"; exit 1; }
# shellcheck source=probe-lib.sh
. "$repo/apps/desktop-tauri/scripts/probe-lib.sh"

work=$(mktemp -d /tmp/dsh-gui-evidence.XXXXXX)
mkdir -p "$work/run" "$work/home" "$work/tmp" "$work/workspace"
chmod 700 "$work/run"
# This is intentionally a private project copy: the probe never edits the checkout.
cp -a "$tree/project/." "$work/workspace/"

XDG_RUNTIME_DIR="$work/run" weston --backend=headless-backend.so --width=1280 --height=860 --no-config \
  >"$work/weston.log" 2>&1 &
weston_pid=$!
shell_pid=""
cleanup() {
  kill_tree "$shell_pid" 2>/dev/null || true
  kill "$weston_pid" 2>/dev/null || true
  rm -rf -- "$work"
}
trap cleanup EXIT

display=""
for _ in $(seq 1 100); do
  display=$(find "$work/run" -maxdepth 1 -name 'wayland-*' -type s 2>/dev/null | head -1)
  [ -n "$display" ] && break
  sleep 0.1
done
if [ -z "$display" ]; then
  echo "RESULT compositor=FAIL"
  tail -20 "$work/weston.log"
  exit 1
fi
display=${display##*/}

# Launch the release shell from the packaged target with private state and the
# same runtime paths AppRun would export.
env -i PATH=/usr/bin:/bin HOME="$work/home" TMPDIR="$work/tmp" \
  XDG_RUNTIME_DIR="$work/run" WAYLAND_DISPLAY="$display" \
  DSH_HOME="$work/home/.dsh" DSH_DESKTOP_HOST_BIN="$tree/bin/desktop-host" \
  DSH_DESKTOP_RUNTIME_DIR="$tree/dsh" DSH_DESKTOP_PROJECT_DIR="$work/workspace" \
  DSH_DESKTOP_PRIMARY_RUNTIME="$tree/runtime/primary-runtime" \
  DSH_DESKTOP_PNPM_ENTRY="$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" \
  DSH_DESKTOP_NODE_BIN="$tree/runtime/bin" DSH_DESKTOP_VERSION=1.0.0 DSH_CLIENT_VERSION=1.0.0 \
  DSH_DESKTOP_WEB_CONSOLE=1 DSH_DESKTOP_WEBKIT_LIGHT=1 DSH_DESKTOP_TRAY=0 DSH_DESKTOP_LAUNCH_URL=1 \
  DSH_DESKTOP_WEBVIEW_SNAPSHOT="$work/picture.png" \
  "$shell" >"$work/out" 2>"$work/err" &
shell_pid=$!

auth=none
for _ in $(seq 1 240); do
  auth=$(grep -ho 'dsh: webview-auth=[A-Za-z0-9]*' "$work/out" "$work/err" 2>/dev/null | tail -1)
  [ "$auth" = 'dsh: webview-auth=200' ] && break
  kill -0 "$shell_pid" 2>/dev/null || break
  sleep 0.25
done
echo "RESULT webview-auth=$auth"
[ "$auth" = 'dsh: webview-auth=200' ] || { tail -20 "$work/err"; exit 1; }

# The release shell's WebKit snapshot is the painted application surface. It
# avoids treating a headless compositor with no output backend as a black GUI.
picture="$work/picture.png"
for _ in $(seq 1 120); do
  [ -s "$picture" ] && break
  sleep 0.1
done
nonblack=0
if [ -s "$picture" ]; then
  nonblack=$(convert "$picture" -alpha off -threshold 1 -format '%[fx:mean*w*h]' info: 2>/dev/null | awk '{printf "%.0f", $1}')
fi
if [ "${nonblack:-0}" -le 0 ]; then
  echo "RESULT picture=FAIL nonblack_pixels=${nonblack:-0}"
  exit 1
fi
echo "RESULT picture=OK nonblack_pixels=${nonblack:-0}"

url=""
for _ in $(seq 1 120); do
  url=$(grep -Eho 'http://127\.0\.0\.1:[0-9]+/?\?token=[A-Za-z0-9_-]+' "$work/err" "$work/out" 2>/dev/null | head -1)
  [ -n "$url" ] && break
  sleep 0.25
done
if [ -z "$url" ]; then
  echo "RESULT host=FAIL reason=no-auth-url"
  exit 1
fi
port=$(printf '%s' "$url" | sed -E 's#.*:([0-9]+)/.*#\1#')
base="http://127.0.0.1:$port"
# Tauri applies CSP to the desktop webview from its application configuration;
# it is not a page response header.
csp=$(node -e 'const fs = require("node:fs"); const config = JSON.parse(fs.readFileSync(process.argv[1], "utf8")); process.stdout.write(config.app.security.csp ?? "")' "$repo/apps/desktop-tauri/src-tauri/tauri.conf.json")
echo "RESULT csp=${csp:-missing}"
if [ -z "$csp" ]; then
  echo 'RESULT policy=FAIL reason=missing'
  exit 1
fi
case "$csp" in
  *"default-src 'self'"*) echo 'RESULT default-src-self=OK';;
  *) echo 'RESULT default-src-self=FAIL'; exit 1;;
esac
case "$csp" in
  *"object-src 'none'"*) echo 'RESULT object-src-none=OK';;
  *) echo 'RESULT object-src-none=FAIL'; exit 1;;
esac
case "$csp" in
  *unsafe-eval*) echo 'RESULT unsafe-eval=FAIL'; exit 1;;
  *) echo 'RESULT unsafe-eval=absent';;
esac
echo 'RESULT policy=OK'

# Pass the launch URL to the helper. It performs the same token exchange as
# the WebView before posting same-origin RPC requests with the resulting cookie.
export DSH_GUI_BASE="$base"
export DSH_GUI_LAUNCH_URL="$url"
export DSH_GUI_WORKSPACE="$work/workspace"
node "$repo/apps/desktop-tauri/scripts/gui-evidence-prompt.mjs" \
  || { echo 'RESULT model-turn=FAIL'; exit 1; }

echo 'RESULT cleanup=OK workspace=private temp=removed-on-exit'
