/**
 * Build the on-demand Office component the Host installs at run time.
 *
 * The archive carries the Python payload and the Office skill assets. The
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
import { cpSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

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
  /** Directory receiving the archive and `components.json`. */
  readonly outDir: string
  /** Application version the component belongs to. */
  readonly version: string
  /** Target name, recorded in the file name. */
  readonly target: string
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
  renameSync(options.pythonDir, join(staging, 'runtime', 'dependencies', 'python'))
  cpSync(options.assetsDir, join(staging, 'office-skills'), { recursive: true })

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
