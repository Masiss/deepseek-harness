#!/usr/bin/env bash
# PTY probe: boot the Tauri Host once and drive terminal RPC calls through it.
#
# Usage: pty-probe.sh <method> <args-json> [<method> <args-json> ...]
# Prints one RESULT line per call. The Host stays up for the whole run so
# several calls can share one authenticated session.
set -u

repo=/home/masis/deepseek-harness
work=$(mktemp -d /tmp/opencode/pty.XXXXXX)
home=$(mktemp -d /tmp/opencode/pty-home.XXXXXX)
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
curl -s -L -c "$work/cookies" -b "$work/cookies" -o /dev/null "$url"
echo "RESULT ready=OK port=$port"

call() {
  local method=$1 args=$2 id=$3
  curl -s -b "$work/cookies" -H 'content-type: application/json' \
    -X POST "$base/api/$method" \
    -d "{\"type\":\"client-request\",\"rpcId\":\"$id\",\"method\":\"$method\",\"payload\":{\"args\":$args}}"
}

# A session must exist before an agent-addressed call can resolve; later calls
# substitute its identity for @SESSION@.
session=$(call 'session/create' '{"request":{}}' 'probe-session')
session_id=$(printf '%s' "$session" | sed -E 's/.*"sessionId":"([^"]+)".*/\1/')
echo "session=$session_id"

while [ $# -ge 2 ]; do
  method=$1; args=${2//@SESSION@/$session_id}; shift 2
  echo "CALL $method $args"
  call "$method" "$args" "probe-$(date +%s%N)"
  echo
done

printf '@@DSH@@{"type":"shutdown"}\n' >&3
for _ in $(seq 1 40); do
  kill -0 "$pid" 2>/dev/null || break
  sleep 0.25
done
kill -9 "$pid" 2>/dev/null
exec 3>&-
