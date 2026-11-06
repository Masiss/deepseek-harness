#!/usr/bin/env bash
# Drive native journeys against the packaged runtime in isolated homes.
#
# The journeys use the native app's probe hooks under headless Weston and a
# packaged Host; no OS window is opened.
#
#   terminal        create a Session and a terminal, attach over the shared mux,
#                   write one command, and read the output back
#   model-default   exercise the profile default through a loopback-only /models
#                   response and a fake account grant
#   timed-question  answer a timed `ask_user_question` through the question-card
#                   code path, after observing the foreground wait stream
#
# Every mode uses a private DSH_HOME and never opens or types into the operator's
# desktop or profile.
#
# Usage: native-journey-probe.sh [terminal|timed-question|model-default]
set -euo pipefail

mode=${1:-terminal}
case "$mode" in
  terminal|timed-question|model-default) ;;
  *) echo "usage: $0 [terminal|timed-question|model-default]" >&2; exit 2 ;;
esac

root=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
bin="$root/apps/desktop-tauri/native-app/target/release/dsh-desktop-native-app"
tree="$root/apps/desktop/.desktop-build/targets/linux-x64"
node="$tree/runtime/bin/node"
mock="$root/apps/desktop-tauri/scripts/mock-model-server.mjs"
host_package="$tree/dsh/node_modules/@deepseek-ai/dsh-desktop-host/package.json"
for required in "$bin" "$mock" "$node" "$tree/bin/desktop-host" \
  "$tree/runtime/primary-runtime/dependencies/node/bin/node" \
  "$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" "$host_package" \
  "$tree/project/package.json"; do
  [ -e "$required" ] || { echo "native-journey-probe: missing input: $required" >&2; exit 1; }
done
[ -x "$bin" ] || { echo "native-journey-probe: not executable: $bin" >&2; exit 1; }

version=$("$node" --input-type=module -e \
  'import { readFileSync } from "node:fs"; const { version } = JSON.parse(readFileSync(process.argv[1], "utf8")); if (typeof version !== "string") throw new Error("invalid version"); console.log(version)' \
  "$host_package")

work_root="${DSH_PROBE_TMPDIR:-$HOME/.cache/dsh-probe}"
mkdir -p "$work_root"
work=$(mktemp -d "$work_root/native-journey.XXXXXX")
mkdir -p "$work/home/.dsh" "$work/run" "$work/tmp" "$work/cache" "$work/config"
chmod 700 "$work/run"
project="$work/home/.dsh/desktop-project"
cp -a "$tree/project" "$project"
chmod -R u+w "$project"

if [ "$mode" = model-default ]; then
  mkdir -p "$work/model-default"
  DSH_ISOLATION_DIR="$work/model-default" DSH_DESKTOP_TARGET=linux-x64 \
    bash "$root/apps/desktop-tauri/scripts/isolated-run.sh" \
      bash "$root/apps/desktop-tauri/scripts/model-default-probe.sh"
  exit $?
fi

case "$mode" in
  terminal)
    probe_env=("DSH_DESKTOP_PROBE_TERMINAL=1")
    marker='dsh: native terminal='
    ;;
  timed-question)
    # The shipped composition answers with the blocking tool; the timed variant
    # is opt-in, so the probe enables it in its own profile copy only.
    cat > "$project/cordis.patch.yml" <<'PATCH'
# Probe-only layer: the timed question tool keeps the answer open past the
# foreground wait so the lane can prove `userQuestions/attachWait`.
- id: tool-ask-user
  config:
    mode: timed
    timeout: 2
PATCH
    probe_env=("DSH_DESKTOP_PROBE_TIMED_QUESTION=1")
    marker='dsh: native timed-question='
    ;;
esac

mock_pid=''
model_env=()
if [ "$mode" = timed-question ]; then
  DSH_MOCK_MODEL_MODE=timed "$node" "$mock" > "$work/mock.out" 2> "$work/mock.err" &
  mock_pid=$!
  model_url=''
  for _ in $(seq 1 100); do
    model_url=$(grep -ho '^MOCK_MODEL_URL=.*' "$work/mock.out" 2>/dev/null | head -1 | cut -d= -f2- || true)
    [ -n "$model_url" ] && break
    sleep 0.1
  done
  [ -n "$model_url" ] || { echo "RESULT native-journey=FAIL reason=mock-model"; tail -3 "$work/mock.err"; exit 1; }
  model_env=("DEEPSEEK_API_KEY=mock" "DEEPSEEK_BASE_URL=$model_url")
fi

XDG_RUNTIME_DIR="$work/run" weston --backend=headless-backend.so --width=1280 --height=860 \
  --no-config > "$work/weston.log" 2>&1 &
weston_pid=$!
app_pid=''
cleanup() {
  if [ -n "$app_pid" ] && kill -0 "$app_pid" 2>/dev/null; then
    kill -TERM "$app_pid" 2>/dev/null || true
    for _ in $(seq 1 100); do kill -0 "$app_pid" 2>/dev/null || break; sleep 0.1; done
    kill -KILL "$app_pid" 2>/dev/null || true
  fi
  [ -n "$mock_pid" ] && kill -TERM "$mock_pid" 2>/dev/null || true
  kill -9 "$weston_pid" 2>/dev/null || true
}
trap cleanup EXIT

display=''
for _ in $(seq 1 100); do
  display=$(find "$work/run" -maxdepth 1 -name 'wayland-*' -type s 2>/dev/null | head -1)
  [ -n "$display" ] && break
  sleep 0.1
done
[ -n "$display" ] || { echo "RESULT compositor=FAIL"; tail -5 "$work/weston.log"; exit 1; }
display=${display##*/}

out="$work/app.out"
err="$work/app.err"
env -i PATH=/usr/bin:/bin HOME="$work/home" TMPDIR="$work/tmp" \
  XDG_RUNTIME_DIR="$work/run" WAYLAND_DISPLAY="$display" \
  XDG_CACHE_HOME="$work/cache" XDG_CONFIG_HOME="$work/config" \
  DSH_HOME="$work/home/.dsh" \
  DSH_DESKTOP_HOST_BIN="$tree/bin/desktop-host" \
  DSH_DESKTOP_RUNTIME_DIR="$tree/dsh" \
  DSH_DESKTOP_PROJECT_DIR="$project" \
  DSH_DESKTOP_PRIMARY_RUNTIME="$tree/runtime/primary-runtime" \
  DSH_DESKTOP_PNPM_ENTRY="$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" \
  DSH_DESKTOP_NODE_BIN="$tree/runtime/bin" \
  DSH_DESKTOP_VERSION="$version" DSH_CLIENT_VERSION="$version" \
  "${model_env[@]}" \
  "${probe_env[@]}" \
  "$bin" > "$out" 2> "$err" &
app_pid=$!

wait_for() {
  local needle=$1 seconds=$2
  for _ in $(seq 1 $((seconds * 10))); do
    grep -qF "$needle" "$err" 2>/dev/null && return 0
    kill -0 "$app_pid" 2>/dev/null || return 1
    sleep 0.1
  done
  return 1
}

if ! wait_for 'dsh: native host=ready' 120; then
  echo "RESULT native-journey=FAIL stage=host"
  tail -20 "$err" || true
  exit 1
fi
echo "RESULT native-host=ready"
if ! wait_for 'dsh: native auth=ok' 60; then
  echo "RESULT native-journey=FAIL stage=auth"
  tail -20 "$err" || true
  exit 1
fi
echo "RESULT native-auth=ok"

if ! wait_for "$marker" 180; then
  echo "RESULT native-journey=FAIL stage=$mode"
  grep -F "$marker" "$err" | tail -5 || true
  tail -20 "$err" || true
  exit 1
fi
grep -F "$marker" "$err" | tail -5
if grep -F "$marker" "$err" | grep -q 'failed'; then
  echo "RESULT native-journey=FAIL mode=$mode"
  exit 1
fi
echo "RESULT native-journey=OK mode=$mode"
echo "work=$work"
