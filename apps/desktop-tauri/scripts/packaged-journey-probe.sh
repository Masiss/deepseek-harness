#!/usr/bin/env bash
# Drive the packaged runtime through the journeys a user performs.
#
# The probe boots the packaged Host (the same tree the AppImage carries) in a
# private harness home and exercises:
#
#   plugin lifecycle   install a local bundle from a tarball, confirm the Host
#                      applied it, restart, confirm it still loads, then
#                      remove it and confirm it is gone
#   session lifecycle  create a blank Session, rename it, read its history,
#                      restart, and read it back from the durable store
#   session fork       complete a turn against a local mock model, fork the
#                      Session, then read the fork back after a restart
#   goal lifecycle     create, pause, resume, edit, complete, and clear a goal,
#                      and read a live goal back from the durable store
#   settings lifecycle mutate the default-model namespace, restart, verify the
#                      saved user override, and remove it
#
# All journeys use the public Host RPC surface, not internal modules, so the
# evidence covers the packaged composition rather than the source tree.
#
#   usage: packaged-journey-probe.sh [linux-x64|linux-arm64]
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
target=${1:-linux-x64}
tree="$repo/apps/desktop/.desktop-build/targets/$target"
node="$tree/runtime/bin/node"
# The probe directory belongs to the built tree, and a fresh tree has none.
mkdir -p "$tree/probe"
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

# A local Messages provider answers the completed-turn journeys. It never
# reaches a real provider: the Host is pointed at loopback with a dummy key.
DSH_MOCK_MODEL_MODE=plain DSH_MOCK_MODEL_TEXT='packaged journey answer' \
  "$node" "$repo/apps/desktop-tauri/scripts/mock-model-server.mjs" \
  > "$work/mock.out" 2> "$work/mock.err" &
mock_pid=$!
model_url=""
for _ in $(seq 1 100); do
  model_url=$(grep -ho '^MOCK_MODEL_URL=.*' "$work/mock.out" 2>/dev/null | head -1 | cut -d= -f2-)
  [ -n "$model_url" ] && break
  sleep 0.1
done
if [ -z "$model_url" ]; then
  echo "RESULT journey=mock-model FAIL"
  tail -3 "$work/mock.err"
  exit 1
fi
cleanup() {
  if [ -n "${mock_pid:-}" ] && kill -0 "$mock_pid" 2>/dev/null; then
    kill -TERM "$mock_pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT

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

settings_revision() {
  "$tree/runtime/bin/node" -e '
    const response = JSON.parse(process.argv[1])
    const namespace = response.result?.value?.namespaces?.find(item => item.ns === "agent-default-model")
    if (namespace === undefined) process.exit(3)
    process.stdout.write(String(namespace.revision))
  ' "$1"
}

settings_user_provider() {
  "$tree/runtime/bin/node" -e '
    const response = JSON.parse(process.argv[1])
    const namespace = response.result?.value?.namespaces?.find(item => item.ns === "agent-default-model")
    if (namespace === undefined) process.exit(3)
    process.stdout.write(String(namespace.user?.provider ?? ""))
  ' "$1"
}

# Reads one nested value out of a server response without pulling in jq.
json_field() {
  "$node" -e '
    const [text, path] = process.argv.slice(1)
    let value
    try { value = JSON.parse(text) } catch { process.exit(3) }
    for (const key of path.split(".")) {
      if (value === null || typeof value !== "object" || !(key in value)) process.exit(3)
      value = value[key]
    }
    process.stdout.write(value === null || value === undefined ? "" : String(value))
  ' "$1" "$2"
}

# Current durable cursor for a Session; `session/page` rejects a bound past it.
session_cursor() {
  "$node" -e '
    const [text, id] = process.argv.slice(1)
    const items = JSON.parse(text).result?.value?.items ?? []
    const item = items.find(entry => entry.sessionId === id)
    if (item === undefined) process.exit(3)
    const seq = item.projections?.asOfSeq
    if (typeof seq !== "number") process.exit(4)
    process.stdout.write(String(seq))
  ' "$1" "$2"
}

page_at_cursor() {
  local id=$1
  local listed cursor
  listed=$(rpc session/list '{"_request":{}}')
  cursor=$(session_cursor "$listed" "$id") || return 3
  rpc session/page "{\"request\":{\"address\":{\"kind\":\"session\",\"sessionId\":\"$id\"},\"throughSeq\":$cursor}}"
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
    DEEPSEEK_API_KEY=mock DEEPSEEK_BASE_URL="$model_url" \
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
through_seq=$(printf '%s' "$renamed" | grep -o '"seq":[0-9]*' | head -1 | cut -d: -f2)
if [ -n "$through_seq" ]; then
  page=$(rpc session/page "{\"request\":{\"address\":{\"kind\":\"session\",\"sessionId\":\"$session\"},\"throughSeq\":$through_seq}}")
else
  page=''
fi
if printf '%s' "$page" | grep -q 'goal-2 journey'; then
  check session-page 0 "throughSeq=$through_seq"
else
  echo "session-page-response=$page"
  check session-page 1 "throughSeq=${through_seq:-missing}"
fi
echo "--- complete one turn and fork the Session ---"
prompt=$(rpc session/prompt "{\"request\":{\"requestId\":\"journey-prompt-1\",\"sessionId\":\"$session\",\"mode\":\"queue\",\"content\":[{\"type\":\"text\",\"text\":\"answer the packaged journey\"}]}}")
printf '%s' "$prompt" | grep -q '"accepted":true'
check session-prompt $? "$(printf '%s' "$prompt" | grep -o '"accepted":[a-z]*' | head -1)"
turn_text=""
for _ in $(seq 1 120); do
  turn_page=$(page_at_cursor "$session")
  if printf '%s' "$turn_page" | grep -q 'packaged journey answer'; then
    turn_text=present
    break
  fi
  sleep 0.5
done
[ "$turn_text" = present ] && check session-turn 0 "assistant=present" || check session-turn 1 "assistant=absent"
forked=$(rpc session/fork "{\"request\":{\"sessionId\":\"$session\"}}")
fork_session=$(json_field "$forked" "result.value.sessionId")
if [ -n "$fork_session" ]; then
  check session-fork 0 "fork=$fork_session"
else
  echo "session-fork-response=$forked"
  check session-fork 1
fi

echo "--- goal lifecycle ---"
goal_created=$(rpc goals/create "{\"agentId\":\"$session\",\"request\":{\"objective\":\"packaged journey goal\"}}")
goal_id=$(json_field "$goal_created" "result.value.ref.id")
goal_rev=$(json_field "$goal_created" "result.value.ref.revision")
[ -n "$goal_id" ] && check goal-create 0 "id=$goal_id" || { echo "goal-create-response=$goal_created"; check goal-create 1; }

goal_view=$(rpc goals/get "{\"agentId\":\"$session\"}")
if [ "$(json_field "$goal_view" "result.value.objective")" = "packaged journey goal" ] \
  && [ "$(json_field "$goal_view" "result.value.phase")" = active ]; then
  check goal-get 0
else
  check goal-get 1
fi

goal_paused=$(rpc goals/pause "{\"agentId\":\"$session\",\"ref\":{\"id\":\"$goal_id\",\"revision\":$goal_rev}}")
goal_rev=$(json_field "$goal_paused" "result.value.revision")
[ "$(json_field "$goal_paused" "result.value.phase")" = paused ] && check goal-pause 0 || check goal-pause 1

goal_resumed=$(rpc goals/resume "{\"agentId\":\"$session\",\"ref\":{\"id\":\"$goal_id\",\"revision\":$goal_rev}}")
goal_rev=$(json_field "$goal_resumed" "result.value.revision")
[ "$(json_field "$goal_resumed" "result.value.phase")" = active ] && check goal-resume 0 || check goal-resume 1

goal_edited=$(rpc goals/edit "{\"agentId\":\"$session\",\"ref\":{\"id\":\"$goal_id\",\"revision\":$goal_rev},\"request\":{\"objective\":\"packaged journey goal edited\"}}")
goal_rev=$(json_field "$goal_edited" "result.value.revision")
[ "$(json_field "$goal_edited" "result.value.objective")" = "packaged journey goal edited" ] \
  && check goal-edit 0 || check goal-edit 1

goal_completed=$(rpc goals/complete "{\"agentId\":\"$session\",\"ref\":{\"id\":\"$goal_id\",\"revision\":$goal_rev}}")
goal_rev=$(json_field "$goal_completed" "result.value.revision")
[ "$(json_field "$goal_completed" "result.value.phase")" = complete ] && check goal-complete 0 || check goal-complete 1

goal_cleared=$(rpc goals/clear "{\"agentId\":\"$session\",\"ref\":{\"id\":\"$goal_id\",\"revision\":$goal_rev}}")
[ -n "$(json_field "$goal_cleared" "result.value.revision")" ] && check goal-clear 0 || check goal-clear 1
goal_after_clear=$(rpc goals/get "{\"agentId\":\"$session\"}")
if [ -z "$(json_field "$goal_after_clear" "result.value.id")" ]; then
  check goal-cleared-get 0
else
  check goal-cleared-get 1
fi

durable_created=$(rpc goals/create "{\"agentId\":\"$session\",\"request\":{\"objective\":\"packaged journey durable goal\"}}")
durable_id=$(json_field "$durable_created" "result.value.ref.id")
durable_rev=$(json_field "$durable_created" "result.value.ref.revision")

settings=$(rpc settings/describe '{}')
printf '%s' "$settings" | grep -q '"writable":true'
check settings-describe $?
settings_names=$(printf '%s' "$settings" | grep -o '"ns":"[^"]*"' | cut -d'"' -f4 | paste -sd, -)
echo "settings-namespaces=${settings_names:-none}"
settings_revision_before=$(settings_revision "$settings")
settings_write=$(rpc settings/mutate "{\"ns\":\"agent-default-model\",\"expectedRevision\":$settings_revision_before,\"ops\":[{\"op\":\"set\",\"path\":[\"provider\"],\"value\":\"native-probe-provider\"}]}")
if printf '%s' "$settings_write" | grep -q '"ok":true'; then
  settings_after_write=$(rpc settings/describe '{}')
  if [ "$(settings_user_provider "$settings_after_write")" = native-probe-provider ]; then
    check settings-write 0
  else
    check settings-write 1
  fi
else
  check settings-write 1
fi

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
page_after_restart=$(rpc session/page "{\"request\":{\"address\":{\"kind\":\"session\",\"sessionId\":\"$session\"},\"throughSeq\":$through_seq}}")
printf '%s' "$page_after_restart" | grep -q 'goal-2 journey'
check session-page-after-restart $?
if [ -n "$fork_session" ]; then
  sessions_after_restart=$(rpc session/list '{"_request":{}}')
  printf '%s' "$sessions_after_restart" | grep -q "$fork_session"
  check session-fork-persist $? "fork=$fork_session"
  fork_page=$(page_at_cursor "$fork_session")
  printf '%s' "$fork_page" | grep -q 'packaged journey answer'
  check session-fork-content $?
else
  check session-fork-persist 1 "fork=missing"
fi
durable_view=$(rpc goals/get "{\"agentId\":\"$session\"}")
if [ "$(json_field "$durable_view" "result.value.objective")" = "packaged journey durable goal" ]; then
  check goal-persist 0
else
  check goal-persist 1
fi
durable_rev=$(json_field "$durable_view" "result.value.revision")
durable_cleared=$(rpc goals/clear "{\"agentId\":\"$session\",\"ref\":{\"id\":\"$durable_id\",\"revision\":$durable_rev}}")
[ -n "$(json_field "$durable_cleared" "result.value.revision")" ] && check goal-durable-clear 0 || check goal-durable-clear 1
settings_after_restart=$(rpc settings/describe '{}')
if [ "$(settings_user_provider "$settings_after_restart")" = native-probe-provider ]; then
  check settings-persist-after-restart 0
else
  check settings-persist-after-restart 1
fi
settings_revision_after=$(settings_revision "$settings_after_restart")
settings_restore=$(rpc settings/mutate "{\"ns\":\"agent-default-model\",\"expectedRevision\":$settings_revision_after,\"ops\":[{\"op\":\"unset\",\"path\":[\"provider\"]}]}")
settings_after_restore=$(rpc settings/describe '{}')
if printf '%s' "$settings_restore" | grep -q '"ok":true' \
  && [ "$(settings_user_provider "$settings_after_restore")" = "" ]; then
  check settings-restore 0
else
  check settings-restore 1
fi

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

echo "--- install the same bundle from a loopback npm registry ---"
# The marketplace path resolves a package name through a registry instead of a
# local tarball. A loopback registry speaking the npm packument protocol proves
# that path without reaching a public registry.
cat > "$work/registry.mjs" <<'SERVER'
import { createReadStream, readFileSync, statSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:http'
const [tarball, name, version, portFile] = process.argv.slice(2)
const bytes = readFileSync(tarball)
const shasum = (await import('node:crypto')).createHash('sha1').update(bytes).digest('hex')
const integrity = `sha512-${(await import('node:crypto')).createHash('sha512').update(bytes).digest('base64')}`
const encoded = name.replace('/', '%2F')
const packument = {
  name,
  'dist-tags': { latest: version },
  versions: {
    [version]: {
      name,
      version,
      dist: { tarball: `/tarballs/${name.split('/')[1]}-${version}.tgz`, shasum, integrity },
    },
  },
}
const server = createServer((request, response) => {
  const path = decodeURIComponent((request.url ?? '/').split('?')[0])
  if (path === '/tarballs/' + name.split('/')[1] + '-' + version + '.tgz') {
    const entry = statSync(tarball)
    response.writeHead(200, { 'content-type': 'application/octet-stream', 'content-length': entry.size })
    createReadStream(tarball).pipe(response)
    return
  }
  if (path === '/' + encoded || path === '/' + name) {
    response.writeHead(200, { 'content-type': 'application/json' })
    response.end(JSON.stringify(packument))
    return
  }
  response.writeHead(404, { 'content-type': 'application/json' })
  response.end('{}')
})
server.listen(0, '127.0.0.1', () => { writeFileSync(portFile, String(server.address().port)) })
SERVER
registry_port_file="$work/registry.port"
rm -f "$registry_port_file"
fixture_name=$("$node" -p "require('$repo/apps/desktop-tauri/scripts/load-probe/package.json').name")
fixture_version=$("$node" -p "require('$repo/apps/desktop-tauri/scripts/load-probe/package.json').version")
"$node" "$work/registry.mjs" "$tarball" "$fixture_name" "$fixture_version" "$registry_port_file" \
  > "$work/registry.out" 2> "$work/registry.err" &
registry_pid=$!
for _ in $(seq 1 100); do [ -s "$registry_port_file" ] && break; sleep 0.1; done
registry="http://127.0.0.1:$(cat "$registry_port_file" 2>/dev/null || echo 0)/"
if [ "$registry" = "http://127.0.0.1:0/" ]; then
  echo "RESULT journey=registry-install FAIL reason=registry-not-listening"
else
  installed=$(rpc pluginManager/installBundle "{\"spec\":\"$fixture_name@$fixture_version\",\"options\":{\"acceptRisk\":true,\"registry\":\"$registry\"}}")
  if printf '%s' "$installed" | grep -q '"exitCode":0' \
    && printf '%s' "$installed" | grep -q '"application":"applied"'; then
    check registry-install 0 "spec=$fixture_name@$fixture_version"
  else
    echo "registry-install-response=$(printf '%s' "$installed" | head -c 400)"
    check registry-install 1
  fi
  plugins=$(rpc pluginManager/listPlugins '{}')
  printf '%s' "$plugins" | grep -q 'dsh-load-probe'
  check registry-install-listed $?
  registry_remove=$(rpc pluginManager/removeBundle "{\"name\":\"$fixture_name\"}")
  printf '%s' "$registry_remove" | grep -q '"ok":true'
  check registry-remove $?
fi
kill "$registry_pid" 2>/dev/null
stop

if [ "$failures" -eq 0 ]; then
  echo "RESULT packaged-journey=OK"
  exit 0
fi
echo "RESULT packaged-journey=FAIL failed=$failures"
exit 1
