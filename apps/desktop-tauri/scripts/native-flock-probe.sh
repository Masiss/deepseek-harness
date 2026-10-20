#!/usr/bin/env bash
# Verify the packaged runtime's session lease and process bootstrap on the
# Rust providers.
#
# The probe resolves the flock entry and the koffi entry through the Desktop
# Host loader, so it exercises the same substitution the launcher installs. It
# checks that the lock reports contention, that the descriptor outlives the
# call, and that the libc binding replaces the process image.
#
#   usage: native-flock-probe.sh [linux-x64|linux-arm64]
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
target=${1:-linux-x64}
tree="$repo/apps/desktop/.desktop-build/targets/$target"
loader="$tree/dsh/node_modules/@deepseek-ai/dsh-desktop-host/native/native-loader.mjs"
[ -f "$loader" ] || { echo "RESULT provider=missing"; exit 1; }

script="$tree/dsh/probe/flock-probe.mjs"
work="$tree/probe/flock-probe"
mkdir -p "$(dirname "$script")" "$work"

cat > "$script" <<'PROBE'
import { createRequire } from 'node:module'
import { openSync, closeSync, unlinkSync } from 'node:fs'

const require = createRequire(import.meta.url)
const leaseDir = new URL('node_modules/@deepseek-ai/dsh-session-persistence-jsonl/', import.meta.url).pathname
const flockEntry = require.resolve('@deepseek-ai/node-addon-system/flock', { paths: [leaseDir] })
const koffiEntry = require.resolve('koffi', { paths: [leaseDir] })
console.log('RESULT flock_provider=' + (flockEntry.includes('dsh-desktop-host') ? 'rust' : flockEntry))
console.log('RESULT koffi_provider=' + (koffiEntry.includes('dsh-desktop-host') ? 'rust' : koffiEntry))

const flock = require(flockEntry)
const path = `${process.env.TMPDIR ?? '/tmp'}/flock-probe.lock`
const first = openSync(path, 'w+')
const second = openSync(path, 'r+')
await flock.tryLockExclusive(first)
let contention = 'none'
try {
  await flock.tryLockExclusive(second)
} catch (error) {
  contention = `${error.code}/${error.errno}/${error.syscall}`
}
closeSync(first)
await flock.tryLockExclusive(second)
closeSync(second)
unlinkSync(path)
console.log('RESULT contention=' + contention + ' after_close=locked')

const koffi = require(koffiEntry)
const libc = koffi.load(null)
const fcntl = libc.func('int fcntl(int fd, int cmd, int arg)')
const flags = fcntl(1, 1, 0)
console.log('RESULT fcntl=' + (flags >= 0 ? 'ok' : String(flags)))
// The bootstrap clears the close-on-exec flag before it replaces the image.
for (const fd of [0, 1, 2]) {
  const descriptorFlags = fcntl(fd, 1, 0)
  if (descriptorFlags !== -1 && (descriptorFlags & 1) !== 0) fcntl(fd, 2, descriptorFlags & ~1)
}
const execve = libc.func('int execve(const char *pathname, const char **argv, const char **envp)')
execve('/bin/echo', ['echo', 'execve-ok'], ['PATH=/usr/bin:/bin'])
PROBE

DSH_ISOLATION_DIR="$work" "$repo/apps/desktop-tauri/scripts/isolated-run.sh" \
  "$tree/runtime/bin/node" --import="$loader" "$script"
