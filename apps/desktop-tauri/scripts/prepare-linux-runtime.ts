/**
 * Materialize the Linux Desktop runtime tree the Tauri shell supervises.
 *
 * This is the fork-owned counterpart of `apps/desktop/scripts/prepare-dsh.ts`.
 * It performs the same package-set install and keeps the same directory
 * contract, but differs where the Electron lane cannot serve Linux:
 *
 * - the target is `linux-<arch>`; the Electron build paths only accept
 *   `mac-arm64`, `mac-x64`, and `win-x64`.
 * - pnpm runs on the bundled primary-runtime Node, not on an Electron binary.
 * - nothing is signed, and no Electron distribution is downloaded.
 *
 * The result feeds the shell's documented environment contract:
 *
 * ```
 * DSH_DESKTOP_RUNTIME_DIR=<out>/dsh
 * DSH_DESKTOP_PROJECT_DIR=<out>/project
 * DSH_DESKTOP_PRIMARY_RUNTIME=<out>/runtime/primary-runtime
 * DSH_DESKTOP_PNPM_ENTRY=<out>/runtime/primary-runtime/dependencies/pnpm/bin/pnpm.mjs
 * DSH_DESKTOP_NODE_BIN=<out>/runtime/bin
 * ```
 *
 * Usage, after packing the release tarballs and preparing the primary runtime
 * (see `pack-linux.sh` and `scripts/primary-runtime/prepare.ts`):
 *
 * ```
 * bunx tsx apps/desktop-tauri/scripts/prepare-linux-runtime.ts <target>
 * ```
 */

import { spawn } from 'node:child_process'
import { cpSync, mkdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { delimiter, dirname, join, relative, resolve } from 'node:path'
import {
  DESKTOP_HOST_PACKAGE,
  DESKTOP_HOST_RUNTIME_FILES,
  DESKTOP_PACKAGES_DIR,
  DESKTOP_PACKAGE_SET_FILE,
  readDesktopCorePackageSet,
  verifyDesktopCoreLockfile,
} from '../../desktop/src/core-package-set.ts'
import { DESKTOP_HOST_PROTOCOL_VERSION } from '../../desktop/src/host-protocol.ts'
import { createPluginProfile, createRuntimeProjectMetadata } from '../../desktop/src/project-manager.ts'
import { parseDesktopRelease } from '../../desktop/src/release.ts'
import { prepareDesktopPackageSet } from '../../desktop/scripts/prepare-package-set.ts'
import { prepareRuntimePatches, verifyRuntimePatches } from '../../desktop/scripts/prepare-runtime-patches.ts'
import { desktopRuntimeFileExclusion } from '../../desktop/scripts/runtime-file-policy.ts'
import { selectOfficeEngine } from '../../../scripts/libreoffice-packages.mjs'

const APP_ROOT = resolve(import.meta.dirname, '..')
const REPOSITORY_ROOT = resolve(APP_ROOT, '..', '..')
const BUILD_ROOT = join(APP_ROOT, '.build')
const DESKTOP_APP_ROOT = join(REPOSITORY_ROOT, 'apps', 'desktop')

/**
 * Launcher the shell spawns as its Host executable.
 *
 * The shell passes the runtime directory as the first argument and appends the
 * remaining positional arguments, so the entry resolves inside that runtime.
 */
const HOST_LAUNCHER = `#!/bin/sh
# Run the installed Desktop Host with the bundled Node.
set -e
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
runtime=$1
exec "$here/../runtime/bin/node" "$runtime/node_modules/@deepseek-ai/dsh-desktop-host/lib/tauri.js" "$@"
`

/** Target platform facts the runtime filter and the Office engine selection need. */
interface LinuxTarget {
  readonly name: 'linux-x64' | 'linux-arm64'
  readonly platform: 'linux'
  readonly arch: 'x64' | 'arm64'
}

function resolveTarget(value: string | undefined): LinuxTarget {
  const name = value ?? `linux-${process.arch === 'arm64' ? 'arm64' : 'x64'}`
  if (name !== 'linux-x64' && name !== 'linux-arm64') {
    throw new Error(`linux runtime: unsupported target ${JSON.stringify(name)}; expected linux-x64 or linux-arm64`)
  }
  return { name, platform: 'linux', arch: name === 'linux-arm64' ? 'arm64' : 'x64' }
}

function manifestVersion(path: string, subject: string): string {
  const manifest = JSON.parse(readFileSync(path, 'utf8')) as { version?: unknown }
  if (typeof manifest.version !== 'string') throw new Error(`linux runtime: ${subject} has no version`)
  return manifest.version
}

/** Run the bundled pnpm under the bundled Node, in the given directory. */
function runPnpm(pnpm: string, node: string, cwd: string, args: readonly string[]): Promise<void> {
  return new Promise((resolvePromise, reject) => {
    const child = spawn(node, ['--expose-internals', pnpm, ...args], {
      cwd,
      env: {
        ...process.env,
        // Package scripts must resolve the bundled Node, never a host install.
        PATH: `${join(BUILD_ROOT, 'runtime', 'bin')}${delimiter}${process.env.PATH ?? ''}`,
        DSH_DESKTOP_NODE_EXECUTABLE: node,
      },
      stdio: 'inherit',
    })
    child.once('error', reject)
    child.once('close', (code) => {
      if (code === 0) resolvePromise()
      else reject(new Error(`linux runtime: pnpm ${args[0]} exited with ${String(code)}`))
    })
  })
}

async function main(): Promise<void> {
  const target = resolveTarget(process.argv[2])
  const out = join(BUILD_ROOT, target.name)
  const packed = join(out, 'packed')
  const packageSetDir = join(out, 'package-set')
  const runtime = join(out, 'runtime')
  const primaryRuntime = join(runtime, 'primary-runtime')
  const node = join(primaryRuntime, 'dependencies', 'node', 'bin', 'node')
  const pnpm = join(primaryRuntime, 'dependencies', 'pnpm', 'bin', 'pnpm.mjs')
  const dsh = join(out, 'dsh')
  const project = join(out, 'project')

  const primary = JSON.parse(readFileSync(join(primaryRuntime, 'runtime.json'), 'utf8')) as {
    node?: unknown
    pnpm?: unknown
  }
  if (typeof primary.node !== 'string' || typeof primary.pnpm !== 'string') {
    throw new Error('linux runtime: the primary runtime declares no Node and pnpm versions')
  }
  const release = parseDesktopRelease({
    schemaVersion: 1,
    version: manifestVersion(join(DESKTOP_APP_ROOT, 'package.json'), 'desktop package'),
    hostProtocolVersion: DESKTOP_HOST_PROTOCOL_VERSION,
    nodeVersion: primary.node,
    pnpmVersion: primary.pnpm,
  })

  // 1. Select and copy the local tarball closure that supplies dsh itself.
  prepareDesktopPackageSet(
    [join(packed, 'dsh'), join(packed, 'vendor'), join(packed, 'landlock')],
    packageSetDir,
  )

  // 2. Resolve the runtime project against the local tarballs only.
  const staging = join(BUILD_ROOT, 'tmp', `dsh-install-${target.name}`)
  rmSync(staging, { recursive: true, force: true })
  mkdirSync(staging, { recursive: true })
  cpSync(join(packageSetDir, DESKTOP_PACKAGE_SET_FILE), join(staging, DESKTOP_PACKAGE_SET_FILE))
  cpSync(join(packageSetDir, DESKTOP_PACKAGES_DIR), join(staging, DESKTOP_PACKAGES_DIR), { recursive: true })
  createRuntimeProjectMetadata(staging, release)
  const patches = prepareRuntimePatches(staging, REPOSITORY_ROOT)
  const config = join(BUILD_ROOT, 'tmp', 'pnpm-config', target.name)
  mkdirSync(config, { recursive: true })
  writeFileSync(join(config, 'npmrc'), '')
  const pnpmFlags = [`--config.userconfig=${join(config, 'npmrc')}`, `--config.store-dir=${join(BUILD_ROOT, 'tmp', 'store')}`]
  await runPnpm(pnpm, node, staging, [...pnpmFlags, 'install', '--lockfile-only'])
  verifyRuntimePatches(staging, patches)
  verifyDesktopCoreLockfile(
    readFileSync(join(staging, 'pnpm-lock.yaml'), 'utf8'),
    readDesktopCorePackageSet(staging, release.version),
  )
  await runPnpm(pnpm, node, staging, [...pnpmFlags, 'install', '--prod', '--frozen-lockfile', '--trust-lockfile'])

  // 3. Copy the resolved modules into the immutable runtime directory.
  const modules = join(staging, 'node_modules')
  const office = JSON.parse(readFileSync(join(modules, '@deepseek-ai', 'libreoffice-kit', 'package.json'), 'utf8')) as {
    optionalDependencies?: Record<string, string>
  }
  const officeEngine = selectOfficeEngine(office, target)
  const packageSet = readDesktopCorePackageSet(staging, release.version)
  rmSync(dsh, { recursive: true, force: true })
  cpSync(modules, join(dsh, 'node_modules'), {
    recursive: true,
    dereference: true,
    filter: source => desktopRuntimeFileExclusion(relative(modules, source), target, officeEngine) === undefined,
  })
  writeFileSync(join(dsh, 'package.json'), `${JSON.stringify({
    name: '@deepseek-ai/dsh-desktop-runtime',
    private: true,
    version: release.version,
    type: 'module',
    dependencies: Object.fromEntries(packageSet.packages.map(entry => [entry.name, entry.version])),
  }, undefined, 2)}\n`)
  for (const file of DESKTOP_HOST_RUNTIME_FILES) {
    const path = join(dsh, 'node_modules', DESKTOP_HOST_PACKAGE, file)
    readFileSync(path)
  }

  // 4. The profile the Host edits. The Desktop application owns this directory
  // at runtime; preparing it here keeps the tree bootable on its own.
  rmSync(project, { recursive: true, force: true })
  createPluginProfile(project)

  // 5. Publish the launcher contract the shell reads.
  const runtimeBin = join(runtime, 'bin')
  mkdirSync(runtimeBin, { recursive: true })
  rmSync(join(runtimeBin, 'node'), { force: true })
  symlinkSync(join('..', 'primary-runtime', 'dependencies', 'node', 'bin', 'node'), join(runtimeBin, 'node'))
  writeFileSync(join(runtime, 'versions.json'), `${JSON.stringify({
    schemaVersion: 1,
    node: release.nodeVersion,
    pnpm: release.pnpmVersion,
  }, undefined, 2)}\n`)

  const launcher = join(out, 'bin', 'desktop-host')
  mkdirSync(dirname(launcher), { recursive: true })
  writeFileSync(launcher, HOST_LAUNCHER, { mode: 0o755 })

  process.stdout.write(`${JSON.stringify({
    target: target.name,
    runtime: dsh,
    project,
    primaryRuntime,
    pnpmEntry: pnpm,
    nodeBin: runtimeBin,
  }, undefined, 2)}\n`)
}

await main()
