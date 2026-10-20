#!/usr/bin/env bash
# Drive the sandbox provider with the Rust Landlock launcher.
#
# The probe confines real commands through the packaged runtime's
# `LocalSandboxProvider` with the Rust launcher forced as the confiner, and
# reports what the confined command could and could not do. It runs inside
# `isolated-run.sh`.
#
#   usage: landlock-sandbox-probe.sh [path-to-landlock-run]
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
target=${DSH_DESKTOP_TARGET:-linux-x64}
tree="$repo/apps/desktop/.desktop-build/targets/$target"
launcher=${1:-$repo/native/dsh-native/target/release/landlock-run}
[ -x "$launcher" ] || { echo "RESULT launcher=missing"; exit 1; }

script="$tree/dsh/probe/landlock-sandbox-probe.mjs"
work="$tree/probe/landlock-sandbox"
mkdir -p "$(dirname "$script")" "$work/home" "$work/tmp"

cat > "$script" <<'PROBE'
import { spawnSync } from 'node:child_process'
import { existsSync, mkdtempSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import { LocalSandboxProvider } from '@deepseek-ai/dsh-sandbox-local'

const launcher = process.env.DSH_LANDLOCK_LAUNCHER
const ctx = new Context()
await ctx.plugin(LocalSandboxProvider, {})
const sandbox = ctx.sandbox
sandbox.internals = { probeBwrap: () => false, landlockLauncher: launcher }

async function confine(command, policy) {
  const confined = await sandbox.confine(['bash', '-c', command], policy)
  const result = spawnSync(confined.argv[0], confined.argv.slice(1), {
    timeout: 30_000,
    encoding: 'utf8',
  })
  return {
    status: result.status,
    stdout: result.stdout ?? '',
    stderr: result.stderr ?? '',
    enforcement: confined.enforcement,
  }
}

const workdir = mkdtempSync(join(tmpdir(), 'dsh-landlock-probe-'))
const denied = join(workdir, 'denied.txt')
const allowed = join(workdir, 'allowed.txt')

const readOnly = await confine(`echo hi > ${denied}`, { mode: 'read-only', workspaceRoot: workdir })
console.log(`RESULT read_only_status=${String(readOnly.status)} file=${existsSync(denied) ? 'present' : 'absent'} enforcement=${readOnly.enforcement}`)

const device = await confine('ls / > /dev/null && echo dev-ok', { mode: 'read-only', workspaceRoot: workdir })
console.log(`RESULT devnull_status=${String(device.status)} stdout=${JSON.stringify(device.stdout.trim())}`)

const workspace = await confine(`echo hi > ${allowed}`, { mode: 'workspace-write', workspaceRoot: workdir })
console.log(`RESULT workspace_status=${String(workspace.status)} file=${existsSync(allowed) ? 'present' : 'absent'}`)

await ctx.fiber.dispose()
PROBE

DSH_ISOLATION_DIR="$work" "$repo/apps/desktop-tauri/scripts/isolated-run.sh" \
  env DSH_LANDLOCK_LAUNCHER="$launcher" "$tree/runtime/bin/node" "$script"
