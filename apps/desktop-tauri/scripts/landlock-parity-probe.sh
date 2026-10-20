#!/usr/bin/env bash
# Compare the Rust Landlock launcher with the C launcher it replaces.
#
# Both binaries must answer the same way for the pinned CLI contract: the
# argument grammar, the exit codes, the report lines, and the confinement
# effects. The script runs the same matrix against both and prints the two
# reports plus their difference.
#
#   usage: landlock-parity-probe.sh [linux-x64|linux-arm64]
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
target=${1:-linux-x64}
tree="$repo/apps/desktop/.desktop-build/targets/$target"
rust="$repo/native/dsh-native/target/release/landlock-run"
reference="$tree/dsh/node_modules/@deepseek-ai/node-addon-system-linux-x64/bin/landlock-run"
[ -x "$rust" ] || { echo "RESULT rust_launcher=missing"; exit 1; }
[ -x "$reference" ] || { echo "RESULT reference_launcher=missing"; exit 1; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Run one matrix and print one line per scenario.
matrix() {
  local launcher=$1
  local report="$work/report"
  : > "$report"

  record() { printf '%s=%s\n' "$1" "$2" >> "$report"; }

  # The probe report line and exit code.
  local out status
  out=$("$launcher" --probe 2>/dev/null)
  status=$?
  record probe_status "$status"
  record probe_report "$out"

  # Usage errors.
  out=$("$launcher" 2>&1 >/dev/null)
  status=$?
  record no_args_status "$status"
  record no_args_message "$out"
  out=$("$launcher" --unknown 2>&1 >/dev/null)
  status=$?
  record unknown_flag_status "$status"
  record unknown_flag_message "$out"
  out=$("$launcher" --probe --ro / 2>&1 >/dev/null)
  status=$?
  record probe_extra_status "$status"
  record probe_extra_message "$out"
  out=$("$launcher" --ro 2>&1 >/dev/null)
  status=$?
  record missing_path_status "$status"
  record missing_path_message "$out"
  out=$("$launcher" --ro /nonexistent-grant-root -- /bin/true 2>&1 >/dev/null)
  status=$?
  record unopenable_status "$status"
  record unopenable_message "$out"

  # Exec failure and exit-code passthrough.
  out=$("$launcher" --ro / -- /nonexistent-program 2>&1 >/dev/null)
  status=$?
  record exec_failure_status "$status"
  record exec_failure_message "$out"
  "$launcher" --ro / -- /bin/sh -c 'exit 42' >/dev/null 2>&1
  record child_status "$?"

  # Confinement: a read-only grant denies writes outside it.
  local sandbox="$work/$(basename "$launcher")"
  mkdir -p "$sandbox"
  "$launcher" --ro / --rw /dev/null -- /bin/sh -c "echo hi > '$sandbox/denied.txt'" >/dev/null 2>&1
  record read_only_write_status "$?"
  record read_only_write_file "$([ -e "$sandbox/denied.txt" ] && echo present || echo absent)"

  out=$("$launcher" --ro / --rw /dev/null -- /bin/sh -c 'ls / > /dev/null && echo dev-ok' 2>/dev/null)
  status=$?
  record read_only_devnull_status "$status"
  record read_only_devnull_stdout "$out"

  # Confinement: a read-write grant allows writes inside it.
  "$launcher" --ro / --rw /dev/null --rw "$sandbox" -- /bin/sh -c "echo hi > '$sandbox/allowed.txt'" >/dev/null 2>&1
  record workspace_write_status "$?"
  record workspace_write_file "$([ -e "$sandbox/allowed.txt" ] && echo present || echo absent)"

  # Confinement: a grant beneath a directory does not cover a sibling.
  "$launcher" --ro / --rw "$sandbox" -- /bin/sh -c "echo hi > '$sandbox/../escaped.txt'" >/dev/null 2>&1
  record escape_status "$?"
  record escape_file "$([ -e "$work/escaped.txt" ] && echo present || echo absent)"

  cat "$report"
}

matrix "$reference" > "$work/reference.txt"
matrix "$rust" > "$work/rust.txt"

echo "RESULT reference=$(basename "$reference") rust=$(basename "$rust")"
if diff -u "$work/reference.txt" "$work/rust.txt" > "$work/diff.txt"; then
  echo "RESULT parity=identical"
  cat "$work/rust.txt"
else
  echo "RESULT parity=differs"
  cat "$work/diff.txt"
  exit 1
fi
