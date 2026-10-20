/**
 * Module hook that points `node-pty` at the Rust provider.
 *
 * The terminal service resolves `node-pty` lazily at spawn time, so a resolve
 * hook registered before the profile boots replaces the C++ addon for the whole
 * runtime without touching `packages/`. A payload without the native module
 * keeps the bundled implementation: the hook is a no-op unless both files are
 * present, and Windows stays on node-pty's ConPTY host.
 *
 * Loaded by the Desktop Host launcher with `--import`.
 */

import { existsSync } from 'node:fs'
import { registerHooks } from 'node:module'
import { dirname, join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const replacement = join(here, 'node-pty.cjs')
const binary = join(here, 'dsh-native.node')

if (process.platform !== 'win32' && existsSync(replacement) && existsSync(binary)) {
  const url = pathToFileURL(replacement).href
  registerHooks({
    resolve(specifier, context, next) {
      if (specifier === 'node-pty') return { url, shortCircuit: true }
      return next(specifier, context)
    },
  })
}
