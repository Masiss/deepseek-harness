#!/usr/bin/env bash
# M1 probe: run the real Tauri shell and drive the plugin manager through the
# Host it supervises, with and without the package-manager arguments.
#
# Usage: plugin-probe.sh [with-package-manager|without-package-manager]
set -u

repo=/home/masis/deepseek-harness
mode=${1:-with-package-manager}
work=$(mktemp -d /tmp/opencode/plugin.XXXXXX)

case "$mode" in
  with-package-manager)
    extra=(
      DSH_DESKTOP_PRIMARY_RUNTIME=/tmp/opencode/plugin-primary
      "DSH_DESKTOP_PNPM_ENTRY=$repo/node_modules/pnpm/bin/pnpm.mjs"
      "DSH_DESKTOP_NODE_BIN=$(dirname "$(command -v node)")"
    )
    ;;
  without-package-manager)
    extra=()
    ;;
  *)
    echo "usage: plugin-probe.sh [with-package-manager|without-package-manager]" >&2
    exit 2
    ;;
esac

# A `pnpm` shim that refuses, so a package operation that falls back to PATH is
# unmistakable. PATH keeps its usual entries: the dev launcher needs dirname.
mkdir -p "$work/fakebin"
printf '#!/bin/sh\necho "PROBE_PNPM_SHIM_REFUSED" >&2\nexit 127\n' > "$work/fakebin/pnpm"
chmod +x "$work/fakebin/pnpm"

env DSH_DESKTOP_HOST_BIN="$repo/apps/desktop-tauri/scripts/dev-host.sh" \
  DSH_DESKTOP_RUNTIME_DIR="$HOME/.dsh/profiles" \
  DSH_DESKTOP_PROJECT_DIR="$HOME/.dsh/profiles/web" \
  DSH_HOME="$work/home" \
  GDK_BACKEND=x11 \
  PATH="$work/fakebin:$PATH" \
  "${extra[@]+"${extra[@]}"}" \
  "$repo/apps/desktop-tauri/src-tauri/target/release/dsh-desktop-tauri" \
  > "$work/out" 2> "$work/err" &
shell_pid=$!

url=""
for _ in $(seq 1 120); do
  url=$(grep -o 'http://127\.0\.0\.1:[0-9]*/?token=[A-Za-z0-9_-]*' "$work/err" 2>/dev/null | head -1)
  [ -n "$url" ] && break
  sleep 0.5
done

if [ -z "$url" ]; then
  echo "RESULT mode=$mode ready=FAIL"
  tail -10 "$work/err"
  kill -9 "$shell_pid" 2>/dev/null
  exit 1
fi

port=$(printf '%s' "$url" | sed -E 's#.*:([0-9]+)/.*#\1#')
base="http://127.0.0.1:$port"
curl -s -L -c "$work/cookies" -b "$work/cookies" -o /dev/null "$url"
echo "RESULT mode=$mode ready=OK port=$port"

rpc() {
  curl -s -m 60 -b "$work/cookies" -H 'content-type: application/json' \
    -X POST "$base/api/pluginManager/$1" \
    -d "{\"type\":\"client-request\",\"rpcId\":\"m1-$1\",\"method\":\"pluginManager/$1\",\"payload\":{\"args\":$2}}"
}

plugins=$(rpc listPlugins '{}')
printf 'listPlugins=%s\n' "$(printf '%s' "$plugins" | head -c 200)"

bundles=$(rpc listBundles '{}')
printf 'listBundles=%s\n' "$(printf '%s' "$bundles" | head -c 320)"

inspect=$(rpc inspect '{"spec":"@deepseek-ai/dsh-not-installed-probe"}')
printf 'inspect_uninstalled=%s\n' "$(printf '%s' "$inspect" | head -c 400)"

kill -9 "$shell_pid" 2>/dev/null
ps -eo pid,args --no-headers | grep -F "release/dsh-desktop-tauri" | grep -v grep | awk '{print $1}' \
  | while read -r pid; do kill -9 "$pid" 2>/dev/null; done
true
