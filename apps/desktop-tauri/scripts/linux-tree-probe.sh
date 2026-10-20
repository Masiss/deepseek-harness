#!/usr/bin/env bash
# Drive the packaged Linux runtime tree through the real Tauri shell.
#
#   usage: linux-tree-probe.sh rpc <method> <args-json> [<method> <args-json> ...]
#          linux-tree-probe.sh install <spec>
set -u

repo=/home/masis/deepseek-harness
target=${DSH_DESKTOP_TARGET:-linux-x64}
tree=$repo/apps/desktop/.desktop-build/targets/$target
mode=${1:?usage: linux-tree-probe.sh rpc|install ...}
shift

work=$(mktemp -d /tmp/opencode/linux-tree.XXXXXX)

env DSH_DESKTOP_HOST_BIN="$tree/bin/desktop-host" \
  DSH_DESKTOP_RUNTIME_DIR="$tree/dsh" \
  DSH_DESKTOP_PROJECT_DIR="$tree/project" \
  DSH_DESKTOP_PRIMARY_RUNTIME="$tree/runtime/primary-runtime" \
  DSH_DESKTOP_PNPM_ENTRY="$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" \
  DSH_DESKTOP_NODE_BIN="$tree/runtime/bin" \
  GDK_BACKEND=x11 \
  DSH_HOME="$work/home" \
  "$repo/apps/desktop-tauri/src-tauri/target/release/dsh-desktop-tauri" \
  > "$work/out" 2> "$work/err" &
shell_pid=$!

cleanup() {
  printf '@@DSH@@{"type":"shutdown"}\n' > /dev/null 2>&1 || true
  kill -9 "$shell_pid" 2>/dev/null
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
echo "load_probe_marker=$(grep -c DSH_LOAD_PROBE_ACTIVE "$work/err" || true)"
curl -s -L -c "$work/cookies" -b "$work/cookies" -o /dev/null "$url"
echo "cookie_bytes=$(stat -c %s "$work/cookies")"

rpc() {
  local method=$1 args=$2
  curl -s -m 180 -b "$work/cookies" -H 'content-type: application/json' \
    -X POST "$base/api/$method" \
    -d "{\"type\":\"client-request\",\"rpcId\":\"probe-$(date +%s%N)\",\"method\":\"$method\",\"payload\":{\"args\":$args}}"
}

case "$mode" in
  rpc)
    while [ $# -ge 2 ]; do
      method=$1; args=$2; shift 2
      echo "CALL $method $args"
      rpc "$method" "$args" > "$work/rpc.json"
      head -c 4000 "$work/rpc.json"
      echo
    done
    ;;
  install)
    spec=${1:?usage: linux-tree-probe.sh install <spec>}
    echo "CALL installBundle $spec"
    rpc pluginManager/installBundle "{\"spec\":\"$spec\",\"options\":{\"acceptRisk\":true}}" > "$work/install.json"
    head -c 2000 "$work/install.json"
    echo
    ;;
esac

cleanup
