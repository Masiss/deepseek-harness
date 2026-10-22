#!/usr/bin/env bash
# Drive the packaged runtime through the journeys a user performs.
#
# The probe boots the packaged Host (the same tree the AppImage carries) in a
# private harness home and exercises:
#
#   plugin lifecycle   install a local bundle from a tarball, confirm the Host
#                      applied it, restart, confirm it still loads, then
#                      remove it and confirm it is gone
#   session lifecycle  create a blank Session, rename it, restart, and read it
#                      back from the durable store
#
# Both journeys use the public Host RPC surface, not internal modules, so the
# evidence covers the packaged composition rather than the source tree.
#
#   usage: packaged-journey-probe.sh [linux-x64|linux-arm64]
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
target=${1:-linux-x64}
tree="$repo/apps/desktop/.desktop-build/targets/$target"
work=${DSH_JOURNEY_WORK:-$(mktemp -d "$tree/probe/journey.XXXXXX")}
mkdir -p "$work/tarball" "$work/home" "$work/workspace"
home="$work/home/.dsh"
profile="$home/desktop-project"
mkdir -p "$home"
cp -a "$tree/project" "$profile"
home_pnpm="$work/home/.local/share"
mkdir -p "$home_pnpm"

failures=0
check() {
  local name=$1 condition=$2 detail=${3:-}
  if [ "$condition" -eq 0 ]; then
    echo "RESULT journey=$name ok${detail:+ $detail}"
  else
    echo "RESULT journey=$name FAIL${detail:+ $detail}"
    failures=$((failures + 1))
  fi
}

# A local bundle with an apply() that reports itself on stderr.
pnpm --dir "$repo/apps/desktop-tauri/scripts/load-probe" pack --pack-destination "$work/tarball" \
  > "$work/pack.log" 2>&1
tarball=$(find "$work/tarball" -name '*.tgz' | head -1)
if [ -z "$tarball" ]; then
  echo "RESULT journey=fixture FAIL reason=pack"
  tail -5 "$work/pack.log"
  exit 1
fi

host_args=("$tree/dsh" "$profile" "$tree/runtime/primary-runtime"
  "$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" "$tree/runtime/bin")

rpc() {
  local method=$1 args=$2
  curl -s -m 180 -b "$work/cookies" -H 'content-type: application/json' \
    -X POST "$base/api/$method" \
    -d "{\"type\":\"client-request\",\"rpcId\":\"journey-$RANDOM\",\"method\":\"$method\",\"payload\":{\"args\":$args}}"
}

start() {
  local run=$1
  mkdir -p "$run"
  mkfifo "$run/in" 2>/dev/null || true
  env XDG_DATA_HOME="$home_pnpm" DSH_HOME="$home" DSH_DESKTOP_RUNTIME_DIR="$tree/dsh" \
    DSH_DESKTOP_PROJECT_DIR="$profile" \
    DSH_DESKTOP_PRIMARY_RUNTIME="$tree/runtime/primary-runtime" \
    DSH_DESKTOP_PNPM_ENTRY="$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" \
    DSH_DESKTOP_NODE_BIN="$tree/runtime/bin" DSH_DESKTOP_VERSION=1.0.0 \
    "$tree/bin/desktop-host" "${host_args[@]}" \
    < "$run/in" > "$run/out" 2> "$run/err" &
  pid=$!
  exec 9> "$run/in"
  url=""
  for _ in $(seq 1 200); do
    url=$(grep -ho 'http://127\.0\.0\.1:[0-9]*/?token=[A-Za-z0-9_-]*' "$run/out" "$run/err" 2>/dev/null | head -1)
    [ -n "$url" ] && break
    kill -0 "$pid" 2>/dev/null || break
    sleep 0.25
  done
  base=""
  [ -n "$url" ] && base="http://127.0.0.1:$(printf '%s' "$url" | sed -E 's#.*:([0-9]+)/.*#\1#')"
  [ -n "$base" ] && curl -s -L -c "$work/cookies" -b "$work/cookies" -o /dev/null "$url"
  return 0
}

stop() {
  printf '@@DSH@@{"type":"shutdown"}\n' >&9 2>/dev/null || true
  exec 9>&- 2>/dev/null || true
  for _ in $(seq 1 100); do kill -0 "$pid" 2>/dev/null || break; sleep 0.1; done
  kill -9 "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null
}

echo "--- boot the packaged Host ---"
start "$work/run-1"
[ -n "$base" ] || { echo "RESULT journey=boot FAIL"; tail -5 "$work/run-1/err"; exit 1; }
echo "RESULT journey=boot ok url=${base#http://}"

echo "--- install a local plugin bundle ---"
installed=$(rpc pluginManager/installBundle "{\"spec\":\"file:$tarball\",\"options\":{\"acceptRisk\":true}}")
printf '%s' "$installed" | grep -q '"exitCode":0' \
  && printf '%s' "$installed" | grep -q '"application":"applied"'
check plugin-install $? "bundle=$(basename "$tarball")"

echo "--- the plugin is active in the running Host ---"
plugins=$(rpc pluginManager/listPlugins '{}')
printf '%s' "$plugins" | grep -q 'dsh-load-probe'
check plugin-list $? "$(printf '%s' "$plugins" | grep -o '"enabled":[a-z]*' | head -1)"

echo "--- create and rename a Session ---"
created=$(rpc session/create "{\"request\":{\"cwd\":\"$work/workspace\"}}")
session=$(printf '%s' "$created" | grep -o '"sessionId":"[^"]*"' | head -1 | cut -d'"' -f4)
renamed=$(rpc session/rename "{\"request\":{\"sessionId\":\"$session\",\"title\":\"goal-2 journey\"}}")
printf '%s' "$renamed" | grep -q '"title":"goal-2 journey"'
check session-create $? "session=$session"

echo "--- restart: the plugin and the Session survive ---"
stop
start "$work/run-2"
[ -n "$base" ] || { echo "RESULT journey=restart FAIL reason=not-ready"; exit 1; }
grep -q DSH_LOAD_PROBE_ACTIVE "$work/run-2/err"
check plugin-after-restart $? "marker=DSH_LOAD_PROBE_ACTIVE"
plugins=$(rpc pluginManager/listPlugins '{}')
printf '%s' "$plugins" | grep -q 'dsh-load-probe'
check plugin-list-after-restart $?
sessions=$(rpc session/list '{"_request":{}}')
printf '%s' "$sessions" > "$work/sessions.json"
printf '%s' "$sessions" | grep -q "$session"
check session-after-restart $? "session=$session"

echo "--- remove the plugin and restart again ---"
removed=$(rpc pluginManager/removeBundle '{"name":"@masiss/dsh-load-probe"}')
printf '%s' "$removed" | grep -q '"ok":true'
check plugin-remove $?
stop
start "$work/run-3"
[ -n "$base" ] || { echo "RESULT journey=remove-restart FAIL reason=not-ready"; exit 1; }
if grep -q DSH_LOAD_PROBE_ACTIVE "$work/run-3/err"; then marker=present; else marker=absent; fi
plugins=$(rpc pluginManager/listPlugins '{}')
if [ "$marker" = absent ] && ! printf '%s' "$plugins" | grep -q 'dsh-load-probe'; then
  check plugin-removed 0 "marker=absent"
else
  check plugin-removed 1 "marker=$marker"
fi
stop

if [ "$failures" -eq 0 ]; then
  echo "RESULT packaged-journey=OK"
  exit 0
fi
echo "RESULT packaged-journey=FAIL failed=$failures"
exit 1
