#!/usr/bin/env bash
# Verify the packaged runtime's terminal service on the Rust PTY provider.
#
# The probe imports the shipped `subprocess-local` build and resolves `node-pty`
# through the Desktop Host loader, so it exercises the same substitution the
# launcher installs. It checks provider resolution, pty window size, input
# echo, and termination, all headless and inside `isolated-run.sh`.
#
#   usage: native-pty-probe.sh [linux-x64|linux-arm64]
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
target=${1:-linux-x64}
tree="$repo/apps/desktop/.desktop-build/targets/$target"
loader="$tree/dsh/node_modules/@deepseek-ai/dsh-desktop-host/native/native-loader.mjs"
[ -f "$loader" ] || { echo "RESULT provider=missing"; exit 1; }

script="$tree/dsh/probe/pty-probe.mjs"
work="$tree/probe/pty-probe"
mkdir -p "$(dirname "$script")" "$work"

cat > "$script" <<'PROBE'
import { createRequire } from 'node:module'
import { Context } from '@deepseek-ai/cordis'
import { LocalSubprocessRuntime } from '@deepseek-ai/dsh-subprocess-local'

const require = createRequire(import.meta.url)
const provider = require.resolve('node-pty', {
  paths: [new URL('node_modules/@deepseek-ai/dsh-subprocess-local/', import.meta.url).pathname],
})
console.log('RESULT provider=' + (provider.includes('dsh-desktop-host') ? 'rust' : provider))

const ctx = new Context()
const fiber = await ctx.plugin(LocalSubprocessRuntime)
const base = { cwd: '/', rows: 33, cols: 111, terminalType: 'xterm-256color', graceMs: 2000 }

async function run(spec, interact) {
  const handle = await ctx.subprocess.spawnTerminal({ ...base, ...spec })
  let output = ''
  const ended = new Promise((resolve) => handle.output.once('end', resolve))
  handle.output.on('data', (chunk) => { output += chunk.toString('utf8') })
  await interact?.(handle)
  const done = await handle.done
  await ended
  return { output, done }
}

const size = await run({ argv: ['/bin/sh', '-c', 'stty size'] })
console.log('RESULT size=' + JSON.stringify(size.output.trim()) + ' exit=' + JSON.stringify(size.done))

const started = Date.now()
const echo = await run({ argv: ['/bin/cat'] }, async (handle) => {
  await new Promise((resolve) => setTimeout(resolve, 100))
  await handle.write('ping\n')
  await new Promise((resolve) => setTimeout(resolve, 200))
  await handle.terminate()
})
const echoed = (echo.output.match(/ping/g) ?? []).length
console.log('RESULT write_echo=' + echoed + ' terminated_ms=' + (Date.now() - started) + ' signal=' + JSON.stringify(echo.done.signal))

await fiber.dispose()
PROBE

DSH_ISOLATION_DIR="$work" "$repo/apps/desktop-tauri/scripts/isolated-run.sh" \
  "$tree/runtime/bin/node" --import="$loader" "$script"
