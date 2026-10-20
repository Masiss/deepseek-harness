#!/usr/bin/env bash
# Measure the packaged Linux runtime: Host cold start and idle RSS.
#
# The Host's stdout is a pipe in production, so it is one here too: a regular
# file would let `read` hit end-of-file before the ready frame arrives.
#
#   usage: measure-linux-tree.sh
set -eu

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
tree=$repo/apps/desktop/.desktop-build/targets/${DSH_DESKTOP_TARGET:-linux-x64}
mkdir -p "$tree/probe"
work=$(mktemp -d "$tree/probe/measure-tree.XXXXXX")
mkfifo "$work/in" "$work/out"

start=$(date +%s%N)
"$tree/bin/desktop-host" \
  "$tree/dsh" "$tree/project" "$tree/runtime/primary-runtime" \
  "$tree/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs" "$tree/runtime/bin" \
  < "$work/in" > "$work/out" 2> "$work/err" &
host_pid=$!
exec 3> "$work/in"
exec 4< "$work/out"

ready=""
while IFS= read -r -u 4 line; do
  case "$line" in
    @@DSH@@*'"type":"ready"'*) ready=$line; break ;;
  esac
done
end=$(date +%s%N)

if [ -z "$ready" ]; then
  echo "RESULT ready=FAIL"
  tail -10 "$work/err"
  exit 1
fi

printf 'RESULT ready=OK cold_start_ms=%s\n' "$(( (end - start) / 1000000 ))"
printf 'rss_kib=%s\n' "$(ps -o rss= -p "$host_pid" | tr -d ' ')"
printf 'node_version=%s\n' "$("$tree/runtime/bin/node" --version)"

printf '@@DSH@@{"type":"shutdown"}\n' >&3
wait "$host_pid" 2>/dev/null || true
exec 3>&- 4<&-
