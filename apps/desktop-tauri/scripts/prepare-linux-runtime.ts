/**
 * Materialize the Linux Desktop runtime tree the Tauri shell supervises.
 *
 * This is the fork-owned counterpart of `apps/desktop/scripts/prepare-dsh.ts`.
 * It performs the same package-set install into the same target directories,
 * but differs where the Electron carrier cannot serve Linux:
 *
 * - pnpm runs on the bundled primary-runtime Node, not on an Electron binary.
 * - nothing is signed, and no Electron distribution is downloaded.
 *
 * The directories come from `desktop-build-paths.mjs`, so the Linux lane and
 * the Electron lane agree on the layout by construction.
 *
 * The result feeds the shell's documented environment contract:
 *
 * ```
 * DSH_DESKTOP_RUNTIME_DIR=<dsh>
 * DSH_DESKTOP_PROJECT_DIR=<root>/project
 * DSH_DESKTOP_PRIMARY_RUNTIME=<runtime>/primary-runtime
 * DSH_DESKTOP_PNPM_ENTRY=<runtime>/primary-runtime/dependencies/pnpm/bin/pnpm.mjs
 * DSH_DESKTOP_NODE_BIN=<runtime>/bin
 * ```
 *
 * Usage, after packing the release tarballs and preparing the primary runtime
 * (see `pack-linux.sh` and `scripts/primary-runtime/prepare.ts`):
 *
 * ```
 * bunx tsx apps/desktop-tauri/scripts/prepare-linux-runtime.ts [linux-x64|linux-arm64]
 * ```
 */

import { spawn } from 'node:child_process'
import { copyFileSync, cpSync, existsSync, mkdirSync, readFileSync, rmSync, statSync, symlinkSync, writeFileSync } from 'node:fs'
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
import { desktopTargetPlatform, resolveDesktopTargetBuildPaths } from '../../desktop/scripts/desktop-build-paths.mjs'
import { prepareDesktopPackageSet } from '../../desktop/scripts/prepare-package-set.ts'
import { prepareRuntimePatches, verifyRuntimePatches } from '../../desktop/scripts/prepare-runtime-patches.ts'
import { desktopRuntimeFileExclusion } from '../../desktop/scripts/runtime-file-policy.ts'
import { linuxRuntimeFileExclusion, rustProviderPresent } from './linux-runtime-file-policy.ts'
import { selectOfficeEngine } from '../../../scripts/libreoffice-packages.mjs'
import { buildOfficeComponent, type BuiltOfficeComponent } from './office-component.ts'
import { prunePythonPayload, verifyPythonPayload } from './prune-python-payload.ts'

const APP_ROOT = resolve(import.meta.dirname, '..')
const REPOSITORY_ROOT = resolve(APP_ROOT, '..', '..')
const DESKTOP_APP_ROOT = join(REPOSITORY_ROOT, 'apps', 'desktop')

/**
 * Launcher the shell spawns as its Host executable.
 *
 * The shell passes the runtime directory as the first argument and appends the
 * remaining positional arguments, so the entry resolves inside that runtime.
 * It sits beside `runtime/`, which is why it reaches the bundled Node through
 * a sibling path.
 */
const HOST_LAUNCHER = `#!/bin/sh
# Run the installed Desktop Host with the bundled Node.
#
# V8 sizes its heap from the machine's memory, not from what the Host needs:
# an idle Host held a 217 MB heap (78 MB used) on a 32 GB machine. The size
# heuristic cuts the reservation to 73 MB and idle RSS from 314 MB to 170 MB,
# trading a small amount of JIT throughput for memory this shell does not need.
# The value grows the heap again when a session asks for it; set
# DSH_DESKTOP_NODE_FLAGS to empty to disable the flag.
set -e
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
runtime=$1
exec "$here/../runtime/bin/node" \${DSH_DESKTOP_NODE_FLAGS---optimize-for-size} \
  --import="$runtime/node_modules/@deepseek-ai/dsh-desktop-host/native/native-loader.mjs" \
  "$runtime/node_modules/@deepseek-ai/dsh-desktop-host/lib/tauri.js" "$@"
`

function targetName(value: string | undefined): 'linux-x64' | 'linux-arm64' {
  const name = value ?? `linux-${process.arch === 'arm64' ? 'arm64' : 'x64'}`
  if (name !== 'linux-x64' && name !== 'linux-arm64') {
    throw new Error(`linux runtime: unsupported target ${JSON.stringify(name)}; expected linux-x64 or linux-arm64`)
  }
  return name
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
        PATH: `${dirname(node)}${delimiter}${process.env.PATH ?? ''}`,
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
  const name = targetName(process.argv[2])
  const target = desktopTargetPlatform(name)
  const paths = resolveDesktopTargetBuildPaths({
    ...process.env,
    DSH_DESKTOP_TARGET_PLATFORM: target.platform,
    DSH_DESKTOP_TARGET_ARCH: target.arch,
  })

  const primaryRuntime = join(paths.runtime, 'primary-runtime')
  const node = join(primaryRuntime, 'dependencies', 'node', 'bin', 'node')
  const pnpm = join(primaryRuntime, 'dependencies', 'pnpm', 'bin', 'pnpm.mjs')
  const project = join(paths.root, 'project')

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

  // Office authoring needs a Python payload and the Office skill assets, but a
  // user who never opens a document should not download either. Prune the
  // payload, prove the interpreter still imports every documented module, then
  // move it out of the bundle into the component archive the Host installs on
  // demand (ADR-0008). `runtime.json` keeps its Python record: the installer
  // copies it into the composed runtime.
  const pythonDir = join(primaryRuntime, 'dependencies', 'python')
  const components = join(paths.root, 'components')
  let officeComponent: BuiltOfficeComponent
  if (existsSync(pythonDir)) {
    const pruned = prunePythonPayload(pythonDir)
    verifyPythonPayload(pythonDir)
    console.error(`linux runtime: pruned ${String(pruned.removed.length)} Python path(s), deduplicated library=${String(pruned.deduplicatedLibrary)}`)
    mkdirSync(components, { recursive: true })
    officeComponent = buildOfficeComponent({
      pythonDir,
      assetsDir: join(REPOSITORY_ROOT, 'packages', 'skill', 'skill-office', 'assets'),
      outDir: components,
      version: release.version,
      target: name,
    })
    console.error(`linux runtime: office component ${officeComponent.file} (${String(officeComponent.bytes)} bytes)`)
  } else {
    const published = join(components, 'components.json')
    if (!existsSync(published)) {
      throw new Error('linux runtime: the primary runtime has no Python payload and no published component')
    }
    const manifest = JSON.parse(readFileSync(published, 'utf8')) as { components?: BuiltOfficeComponent[] }
    const entry = manifest.components?.[0]
    if (entry === undefined) throw new Error('linux runtime: the component manifest lists no archive')
    officeComponent = entry
    console.error(`linux runtime: reusing office component ${officeComponent.file}`)
  }

  // 1. Select and copy the local tarball closure that supplies dsh itself.
  prepareDesktopPackageSet([paths.packedDsh, paths.packedVendor, paths.packedLandlock], paths.packageSet)

  // 2. Resolve the runtime project against the local tarballs only.
  const staging = join(paths.dshPnpm, 'staging')
  rmSync(staging, { recursive: true, force: true })
  rmSync(paths.dshPnpm, { recursive: true, force: true })
  mkdirSync(staging, { recursive: true })
  cpSync(join(paths.packageSet, DESKTOP_PACKAGE_SET_FILE), join(staging, DESKTOP_PACKAGE_SET_FILE))
  cpSync(join(paths.packageSet, DESKTOP_PACKAGES_DIR), join(staging, DESKTOP_PACKAGES_DIR), { recursive: true })
  createRuntimeProjectMetadata(staging, release)
  const patches = prepareRuntimePatches(staging, REPOSITORY_ROOT)
  const config = join(paths.dshPnpm, 'config')
  mkdirSync(config, { recursive: true })
  writeFileSync(join(config, 'npmrc'), '')
  const pnpmFlags = [`--config.userconfig=${join(config, 'npmrc')}`, `--config.store-dir=${join(paths.dshPnpm, 'store')}`]
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
  const rustProvider = rustProviderPresent(modules)
  console.error(`linux runtime: Rust provider present=${String(rustProvider)}`)
  rmSync(paths.dsh, { recursive: true, force: true })
  cpSync(modules, join(paths.dsh, 'node_modules'), {
    recursive: true,
    dereference: true,
    filter: (source) => {
      const relativePath = relative(modules, source)
      if (desktopRuntimeFileExclusion(relativePath, target, officeEngine) !== undefined) return false
      return linuxRuntimeFileExclusion(relativePath, { platform: target.platform, rustProvider }) === undefined
    },
  })
  writeFileSync(join(paths.dsh, 'package.json'), `${JSON.stringify({
    name: '@deepseek-ai/dsh-desktop-runtime',
    private: true,
    version: release.version,
    type: 'module',
    dependencies: Object.fromEntries(packageSet.packages.map(entry => [entry.name, entry.version])),
  }, undefined, 2)}\n`)
  for (const file of DESKTOP_HOST_RUNTIME_FILES) {
    readFileSync(join(paths.dsh, 'node_modules', DESKTOP_HOST_PACKAGE, file))
  }

  // The Rust Landlock launcher replaces the C one the platform package ships.
  // Its CLI contract is the same, so the sandbox consumer needs no change.
  const builtLauncher = join(paths.root, 'native', 'landlock-run')
  if (existsSync(builtLauncher)) {
    const platformPackage = join(
      paths.dsh,
      'node_modules',
      '@deepseek-ai',
      `node-addon-system-${target.platform}-${target.arch}`,
    )
    const installedLauncher = join(platformPackage, 'bin', 'landlock-run')
    if (!existsSync(installedLauncher)) {
      throw new Error(`linux runtime: the platform package has no Landlock launcher at ${installedLauncher}`)
    }
    copyFileSync(builtLauncher, installedLauncher)
    console.error(`linux runtime: Landlock launcher replaced with the Rust build (${String(statSync(installedLauncher).size)} bytes)`)
  }

  // 4. The profile the Host edits. The Desktop application owns this directory
  // at runtime; preparing it here keeps the tree bootable on its own.
  rmSync(project, { recursive: true, force: true })
  createPluginProfile(project)

  // 5. Publish the launcher contract the shell reads.
  const bin = join(paths.runtime, 'bin')
  mkdirSync(bin, { recursive: true })
  rmSync(join(bin, 'node'), { force: true })
  symlinkSync(join('..', 'primary-runtime', 'dependencies', 'node', 'bin', 'node'), join(bin, 'node'))
  writeFileSync(join(paths.runtime, 'versions.json'), `${JSON.stringify({
    schemaVersion: 1,
    node: release.nodeVersion,
    pnpm: release.pnpmVersion,
  }, undefined, 2)}\n`)

  const launcher = join(paths.root, 'bin', 'desktop-host')
  mkdirSync(dirname(launcher), { recursive: true })
  writeFileSync(launcher, HOST_LAUNCHER, { mode: 0o755 })

  process.stdout.write(`${JSON.stringify({
    target: name,
    launcher,
    runtime: paths.dsh,
    project,
    primaryRuntime,
    pnpmEntry: pnpm,
    nodeBin: bin,
    officeComponent: officeComponent.file,
  }, undefined, 2)}\n`)
}

await main()
