#!/usr/bin/env bash
# Report the bytes the packaged desktop webview loads by bundle/package.
#
# The probe boots the release shell against a packaged Desktop tree, waits for
# the webview's own authorization result, then prints every resource and the
# group totals emitted by the shell. Resource transferSize is the network/cache
# transfer weight reported by the WebView; it is not decoded memory.
#
#   usage: load-weight-probe.sh [linux-x64|linux-arm64]
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
target=${1:-linux-x64}
tree="$repo/apps/desktop/.desktop-build/targets/$target"
shell="$repo/apps/desktop-tauri/src-tauri/target/release/dsh-desktop-tauri"
[ -x "$shell" ] || { echo "RESULT shell=missing"; exit 1; }
[ -x "$tree/bin/desktop-host" ] || { echo "RESULT host=missing"; exit 1; }
# shellcheck source=probe-lib.sh
. "$repo/apps/desktop-tauri/scripts/probe-lib.sh"

work=$(mktemp -d /tmp/dsh-load-weight.XXXXXX)
mkdir -p "$work/run" "$work/home" "$work/tmp"
chmod 700 "$work/run"
cp -a "$tree/project" "$work/home/desktop-project"

XDG_RUNTIME_DIR="$work/run" weston --backend=headless-backend.so --width=1280 --height=860 \
  --no-config > "$work/weston.log" 2>&1 &
weston_pid=$!
shell_pid=""
cleanup() {
  kill -9 "$weston_pid" 2>/dev/null || true
  kill_tree "$shell_pid"
  true
}
trap cleanup EXIT

display=""
for _ in $(seq 1 100); do
  display=$(find "$work/run" -maxdepth 1 -name 'wayland-*' -type s 2>/dev/null | head -1)
  [ -n "$display" ] && break
  sleep 0.1
done
[ -n "$display" ] || { echo "RESULT compositor=FAIL"; tail -5 "$work/weston.log"; exit 1; }
display=${display##*/}

env -i PATH=/usr/bin:/bin HOME="$work/home" TMPDIR="$work/tmp" \
  XDG_RUNTIME_DIR="$work/run" WAYLAND_DISPLAY="$display" \
  DSH_HOME="$work/home/.dsh" \
  DSH_DESKTOP_HOST_BIN="$tree/bin/desktop-host" \
  DSH_DESKTOP_RUNTIME_DIR="$tree/dsh" \
  DSH_DESKTOP_PROJECT_DIR="$work/home/desktop-project" \
  DSH_DESKTOP_PRIMARY_RUNTIME="$tree/runtime/primary-runtime" \
  DSH_DESKTOP_PNPM_ENTRY="$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" \
  DSH_DESKTOP_NODE_BIN="$tree/runtime/bin" \
  DSH_DESKTOP_VERSION=1.0.0 DSH_CLIENT_VERSION=1.0.0 \
  DSH_DESKTOP_WEB_CONSOLE=1 \
  "$shell" > "$work/out" 2> "$work/err" &
shell_pid=$!

# The authorization marker is emitted by the page. Read both shell streams so
# the verdict belongs to the webview rather than to a separate HTTP request.
auth=none
for _ in $(seq 1 600); do
  auth=$(grep -ho 'dsh: webview-auth=[A-Za-z0-9]*' "$work/out" "$work/err" 2>/dev/null | tail -1)
  [ "$auth" = 'dsh: webview-auth=200' ] && break
  kill -0 "$shell_pid" 2>/dev/null || break
  sleep 0.2
done
echo "RESULT webview_auth=$auth"
if [ "$auth" != 'dsh: webview-auth=200' ]; then
  echo "RESULT load_weight=FAIL reason=webview-not-authorized"
  sed -n '1,120p' "$work/out" "$work/err" 2>/dev/null || true
  exit 1
fi

# The shell reports this after its frame window, so authorization alone is not
# enough to read a complete resource list.
reported=0
for _ in $(seq 1 200); do
  if grep -q 'dsh: webview-resources=[0-9]* bytes=[0-9]*' "$work/out" "$work/err" 2>/dev/null; then
    reported=1
    break
  fi
  kill -0 "$shell_pid" 2>/dev/null || break
  sleep 0.25
done
if [ "$reported" -ne 1 ]; then
  echo "RESULT load_weight=FAIL reason=resource-report-missing"
  tail -40 "$work/out" "$work/err" 2>/dev/null || true
  exit 1
fi

python3 - "$work/out" "$work/err" <<'PY'
import json
import sys

resources = []
groups = []
total = None
seen_groups = set()
for path in sys.argv[1:]:
    try:
        lines = open(path, encoding='utf-8', errors='replace')
    except OSError:
        continue
    for line in lines:
        for marker, destination in (
            ('dsh: webview-resource=', resources),
            ('dsh: webview-resource-group=', groups),
        ):
            if marker in line:
                raw = line.split(marker, 1)[1].strip()
                try:
                    value = json.loads(raw)
                except json.JSONDecodeError:
                    continue
                if destination is resources:
                    destination.append(value)
                    continue
                key = json.dumps(value, sort_keys=True, separators=(',', ':'))
                if key not in seen_groups:
                    seen_groups.add(key)
                    destination.append(value)
                break
        else:
            marker = 'dsh: webview-resources='
            if marker in line:
                raw = line.split(marker, 1)[1].strip()
                fields = raw.split()
                if len(fields) == 2 and fields[1].startswith('bytes='):
                    try:
                        total = {'resources': int(fields[0]), 'bytes': int(fields[1][6:])}
                    except ValueError:
                        pass

if total is None:
    print('RESULT load_weight=FAIL reason=invalid-resource-total')
    raise SystemExit(1)

for resource in resources:
    print('RESULT resource=' + json.dumps(resource, separators=(',', ':')))
for group in sorted(groups, key=lambda value: value.get('group', '')):
    print('RESULT group=' + json.dumps(group, separators=(',', ':')))
print('RESULT total=' + json.dumps(total, separators=(',', ':')))
if len(resources) != total['resources']:
    print(f"RESULT load_weight=FAIL reason=resource-count-mismatch listed={len(resources)} total={total['resources']}")
    raise SystemExit(1)
print('RESULT load_weight=OK')
PY
