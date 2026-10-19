#!/usr/bin/env bash
# P0 risk probe: can the Tauri Host read Sessions written by the current
# Electron build, including the zstd generations?
#
# The invariant this checks is the one the plan forbids breaking: the Session
# log format and its reader. It runs against the real DSH home, read-only.
set -u

repo=/home/masis/deepseek-harness
work=$(mktemp -d /tmp/opencode/sessions.XXXXXX)
mkfifo "$work/in"

node "$repo/apps/desktop-host/src/tauri.ts" \
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

rpc() {
  curl -s -b "$work/cookies" -H 'content-type: application/json' \
    -X POST "$base/api/session/$1" \
    -d "{\"type\":\"client-request\",\"rpcId\":\"probe-$1\",\"method\":\"session/$1\",\"payload\":{\"args\":$2}}"
}

echo "files_total=$(find "$HOME/.dsh/sessions" -name 'session*.jsonl*' 2>/dev/null | wc -l)"
echo "files_by_generation=$(find "$HOME/.dsh/sessions" -name 'session*.jsonl*' 2>/dev/null | sed -E 's#.*/session(\.v[0-9]+)?\.jsonl.*#\1#' | sort | uniq -c | tr '\n' ' ')"

listed=$(rpc list '{"_request":{}}')
printf '%s' "$listed" | python3 -c '
import json, sys
envelope = json.load(sys.stdin)
result = envelope.get("result", {})
value = result.get("value") or {}
items = value.get("items") or []
if not result.get("ok"):
    print("list_ok=0"); print(json.dumps(result)[:400]); raise SystemExit
statuses = {}
for item in items:
    key = item.get("formatStatus")
    statuses[key] = statuses.get(key, 0) + 1
print(f"list_ok=1 list_items={len(items)}")
print("format_status=" + json.dumps(statuses))
print("any_blank=" + str(any(item.get("blank") for item in items)))
print("first=" + json.dumps(items[0])[:220] if items else "first=<none>")
'

printf '@@DSH@@{"type":"shutdown"}\n' >&3
for _ in $(seq 1 40); do
  kill -0 "$pid" 2>/dev/null || break
  sleep 0.25
done
kill -9 "$pid" 2>/dev/null
exec 3>&-
