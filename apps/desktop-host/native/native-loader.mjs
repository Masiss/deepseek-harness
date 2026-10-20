/**
 * Module hook that points the native providers at the Rust module and the
 * Office engine at the installed component.
 *
 * The terminal service resolves `node-pty` lazily at spawn time and the
 * session lease imports `@deepseek-ai/node-addon-system/flock`, so a resolve
 * hook registered before the profile boots replaces both addons for the whole
 * runtime without touching `packages/`. A payload without the native module
 * keeps the bundled implementations: the hook does nothing unless the module
 * is present, and Windows stays on node-pty's ConPTY host.
 *
 * The Office engine travels with the on-demand Office component, so the hook
 * points the engine's package at the installed copy when the component is
 * present. Without it, the engine stays unresolved and the plugins that need
 * it are skipped until the user installs the component.
 *
 * Loaded by the Desktop Host launcher with `--import`.
 */

import { existsSync, readFileSync } from 'node:fs'
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
  ['sharp', join(here, 'sharp.cjs')],
])

/**
 * Locate the Office engine of the installed component.
 * @returns The engine package directory, or undefined when no component is installed.
 */
function officeEngineRoot() {
  const home = process.env.DSH_HOME
  if (home === undefined || home === '') return undefined
  const marker = join(home, 'components', 'current.json')
  if (!existsSync(marker)) return undefined
  let version
  try {
    version = JSON.parse(readFileSync(marker, 'utf8')).version
  } catch {
    return undefined
  }
  if (typeof version !== 'string') return undefined
  const root = join(home, 'components', 'versions', version, 'runtime', 'node_modules', '@deepseek-ai', 'libreoffice-kit')
  return existsSync(join(root, 'lib', 'index.js')) ? root : undefined
}

const urls = new Map()
if (process.platform !== 'win32' && existsSync(binary)) {
  for (const [specifier, path] of replacements) {
    if (existsSync(path)) urls.set(specifier, pathToFileURL(path).href)
  }
}

const engine = officeEngineRoot()
if (engine !== undefined) {
  urls.set('@deepseek-ai/libreoffice-kit', pathToFileURL(join(engine, 'lib', 'index.js')).href)
  urls.set('@deepseek-ai/libreoffice-kit/package.json', pathToFileURL(join(engine, 'package.json')).href)
  urls.set('@deepseek-ai/libreoffice-kit/cli', pathToFileURL(join(engine, 'lib', 'cli.js')).href)
}

if (urls.size > 0) {
  registerHooks({
    resolve(specifier, context, next) {
      const url = urls.get(specifier)
      if (url !== undefined) return { url, shortCircuit: true }
      return next(specifier, context)
    },
  })
}
