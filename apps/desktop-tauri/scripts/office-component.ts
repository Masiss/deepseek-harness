/**
 * Build the on-demand Office component the Host installs at run time.
 *
 * The archive carries the Python payload, the Office skill assets, and the
 * Office engine — the LibreOffice kit and its WebAssembly build. The
 * `runtime.json` that makes them a payload is not shipped: the installer copies
 * the one from the bundled primary runtime, so Node and pnpm always match the
 * running application.
 *
 * The archive is zstd-compressed tar, produced with the host `tar`; the Host
 * reads it with its own minimal reader, so the build and run sides agree on
 * exactly one format.
 */

import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, readlinkSync, rmSync, symlinkSync, unlinkSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'

/** One archive this build publishes. */
export interface BuiltOfficeComponent {
  /** Archive file name, relative to the component directory. */
  readonly file: string
  /** Exact byte length. */
  readonly bytes: number
  /** Lowercase SHA-256 of the archive. */
  readonly sha256: string
  /** Component version. */
  readonly version: string
}

/** Inputs for {@link buildOfficeComponent}. */
export interface BuildOfficeComponentOptions {
  /** Python payload inside the primary runtime; moved into the archive. */
  readonly pythonDir: string
  /** Office skill assets, copied to `office-skills/` in the archive. */
  readonly assetsDir: string
  /** Installed runtime tree the Office engine is copied from. */
  readonly modulesDir: string
  /** Directory receiving the archive and `components.json`. */
  readonly outDir: string
  /** Application version the component belongs to. */
  readonly version: string
  /** Target name, recorded in the file name. */
  readonly target: string
}

/** The Office engine packages the component carries. */
const ENGINE_PACKAGES = [
  '@deepseek-ai/libreoffice-kit',
  '@deepseek-ai/libreoffice-kit-wasm',
] as const

/**
 * Collect a package and the runtime dependencies it needs.
 * @param modulesDir - Installed runtime tree holding the packages.
 * @param names - Package names to start from.
 * @returns The package names of the closure, in discovery order.
 */
function collectClosure(modulesDir: string, names: readonly string[]): string[] {
  const found = new Set<string>()
  const queue = [...names]
  while (queue.length > 0) {
    const name = queue.shift() as string
    if (found.has(name)) continue
    const manifest = join(modulesDir, ...name.split('/'), 'package.json')
    if (!existsSync(manifest)) continue
    found.add(name)
    const parsed = JSON.parse(readFileSync(manifest, 'utf8')) as { dependencies?: Record<string, string> }
    for (const dependency of Object.keys(parsed.dependencies ?? {})) {
      queue.push(dependency)
    }
  }
  return [...found]
}

/**
 * Point every symlink inside the payload at its target by relative path.
 *
 * The bundled interpreter ships absolute symlinks into the build machine's
 * directory, which no other machine can follow. The archive is read by a
 * minimal tar reader that refuses links leaving the extraction root, so the
 * links are rewritten before the payload is packed.
 * @param root - Payload root copied into the archive.
 */
function relativizeSymlinks(root: string): void {
  for (const entry of readdirSync(root, { withFileTypes: true, recursive: true })) {
    if (!entry.isSymbolicLink()) continue
    const path = join(entry.parentPath, entry.name)
    const target = readlinkSync(path)
    if (!target.startsWith('/')) continue
    // The link points into the build machine's directory. Its target is a
    // file of this payload, so the link can name it directly.
    const name = target.slice(target.lastIndexOf('/') + 1)
    if (!existsSync(join(dirname(path), name))) continue
    unlinkSync(path)
    symlinkSync(name, path)
  }
}

/**
 * Pack the Office payload and write the manifest the Host reads.
 *
 * @param options - Payload sources, output directory, and release identity.
 * @returns The published archive description.
 * @throws When `tar` fails or the archive is empty.
 */
export function buildOfficeComponent(options: BuildOfficeComponentOptions): BuiltOfficeComponent {
  const staging = join(options.outDir, '.office-staging')
  rmSync(staging, { recursive: true, force: true })
  mkdirSync(join(staging, 'runtime', 'dependencies'), { recursive: true })
  const python = join(staging, 'runtime', 'dependencies', 'python')
  cpSync(options.pythonDir, python, { recursive: true })
  relativizeSymlinks(python)
  cpSync(options.assetsDir, join(staging, 'office-skills'), { recursive: true })

  // The Office engine travels with the component: a user who never opens a
  // document never downloads it, and the core payload stays small.
  const engine = join(staging, 'runtime', 'node_modules')
  mkdirSync(engine, { recursive: true })
  for (const name of collectClosure(options.modulesDir, ENGINE_PACKAGES)) {
    const source = join(options.modulesDir, ...name.split('/'))
    if (!existsSync(source)) continue
    const target = join(engine, ...name.split('/'))
    mkdirSync(join(target, '..'), { recursive: true })
    cpSync(source, target, { recursive: true, dereference: true })
  }

  const file = `office-${options.version}-${options.target}.tar.zst`
  const archive = join(options.outDir, file)
  rmSync(archive, { force: true })
  const tar = spawnSync('tar', ['--zstd', '-cf', archive, '-C', staging, '.'], { stdio: 'inherit' })
  if (tar.status !== 0) throw new Error(`office component: tar exited with ${String(tar.status)}`)
  rmSync(staging, { recursive: true, force: true })

  const bytes = readFileSync(archive)
  const built: BuiltOfficeComponent = {
    file,
    bytes: bytes.byteLength,
    sha256: createHash('sha256').update(bytes).digest('hex'),
    version: options.version,
  }
  writeFileSync(join(options.outDir, 'components.json'), `${JSON.stringify({
    schemaVersion: 1,
    components: [{ id: 'office', ...built }],
  }, undefined, 2)}\n`)
  return built
}
