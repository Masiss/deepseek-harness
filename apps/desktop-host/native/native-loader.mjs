/**
 * Module hook that points the native providers at the Rust module.
 *
 * The terminal service resolves `node-pty` lazily at spawn time and the
 * session lease imports `@deepseek-ai/node-addon-system/flock`, so a resolve
 * hook registered before the profile boots replaces both addons for the whole
 * runtime without touching `packages/`. A payload without the native module
 * keeps the bundled implementations: the hook does nothing unless the module
 * is present, and Windows stays on node-pty's ConPTY host.
 *
 * Loaded by the Desktop Host launcher with `--import`.
 */

import { existsSync } from 'node:fs'
import { registerHooks } from 'node:module'
import { dirname, join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const binary = join(here, 'dsh-native.node')

/** Specifiers the Rust module replaces, with the wrapper that calls it. */
const replacements = new Map([
  ['node-pty', join(here, 'node-pty.cjs')],
  ['@deepseek-ai/node-addon-system/flock', join(here, 'flock.cjs')],
  ['koffi', join(here, 'koffi.cjs')],
])

if (process.platform !== 'win32' && existsSync(binary)) {
  const urls = new Map()
  for (const [specifier, path] of replacements) {
    if (existsSync(path)) urls.set(specifier, pathToFileURL(path).href)
  }
  registerHooks({
    resolve(specifier, context, next) {
      const url = urls.get(specifier)
      if (url !== undefined) return { url, shortCircuit: true }
      return next(specifier, context)
    },
  })
}
