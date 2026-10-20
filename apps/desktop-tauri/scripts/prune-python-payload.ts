/**
 * Remove the parts of the bundled Python payload no Desktop workflow uses.
 *
 * The locked payload is `python-build-standalone`'s `install_only_stripped`
 * build plus the Office wheels. What remains after this module runs still
 * imports every module the Office skills document (numpy, pandas, openpyxl,
 * python-docx, python-pptx, Pillow, lxml); what it drops is packaging and
 * development surface:
 *
 * - a second, byte-identical `libpython3.12.so` (the loader resolves the
 *   versioned name, so the unversioned entry becomes a relative symlink);
 * - the terminal database and manual pages;
 * - C headers, `pip`/`ensurepip`, IDLE, `lib2to3`, `pydoc_data`;
 * - Tcl/Tk and its dynamic module, used only through `tkinter`.
 *
 * Stripping is deliberately absent: GNU `strip`, including `--strip-debug`,
 * corrupts `python-build-standalone`'s relocated section layout and the
 * interpreter then fails to resolve its own symbols.
 *
 * This is a fork-owned step for the Linux lane; the Electron payload keeps its
 * full tree.
 */

import { execFileSync } from 'node:child_process'
import { existsSync, lstatSync, readdirSync, rmSync, symlinkSync, unlinkSync } from 'node:fs'
import { join } from 'node:path'

/** Paths relative to the Python root that no documented Desktop workflow loads. */
const UNUSED_PATHS: readonly string[] = [
  'include',
  'share/man',
  'share/terminfo',
]

/** Standard-library packages removed entirely, relative to the version directory. */
const UNUSED_STDLIB: readonly string[] = [
  'ensurepip',
  'idlelib',
  'lib2to3',
  'pydoc_data',
  'tkinter',
]

/** Wheel content removed rather than imported at run time. */
const UNUSED_SITE_PACKAGES: readonly string[] = ['pip']

/** Modules every Office skill imports; the payload must keep resolving them. */
const REQUIRED_IMPORTS = ['numpy', 'pandas', 'openpyxl', 'docx', 'pptx', 'PIL', 'lxml'] as const

/** What pruning changed. */
export interface PythonPruneResult {
  /** Paths removed, relative to the Python root. */
  readonly removed: readonly string[]
  /** Whether the duplicate shared library became a symlink. */
  readonly deduplicatedLibrary: boolean
}

function versionDirectory(pythonDir: string): string {
  const lib = join(pythonDir, 'lib')
  const candidates = readdirSync(lib).filter(entry => /^python3\.\d+$/u.test(entry))
  if (candidates.length !== 1) {
    throw new Error(`python payload: expected one lib/python3.N directory, found ${String(candidates.length)}`)
  }
  return candidates[0] as string
}

/**
 * Drop the unused payload surface and collapse the duplicate shared library.
 *
 * @param pythonDir - Extracted payload root (the directory holding `bin` and `lib`).
 * @returns the removed paths and whether the library was deduplicated.
 * @throws when the payload does not look like one interpreter tree.
 */
export function prunePythonPayload(pythonDir: string): PythonPruneResult {
  const removed: string[] = []
  const drop = (relativePath: string): void => {
    if (!existsSync(join(pythonDir, relativePath))) return
    rmSync(join(pythonDir, relativePath), { recursive: true, force: true })
    removed.push(relativePath)
  }

  for (const path of UNUSED_PATHS) drop(path)
  const versioned = versionDirectory(pythonDir)
  for (const name of UNUSED_STDLIB) drop(join('lib', versioned, name))
  for (const name of UNUSED_SITE_PACKAGES) drop(join('lib', versioned, 'site-packages', name))

  // Tcl/Tk trees and their loadable module carry version numbers in their names.
  const lib = join(pythonDir, 'lib')
  for (const entry of readdirSync(lib)) {
    if (/^(?:tcl|tk|itcl|thread)[\d.]*$/u.test(entry) || /^lib(?:tcl|tk)[\w.-]*\.so(?:\.\d+)*$/u.test(entry)) {
      drop(join('lib', entry))
    }
  }
  const dynload = join(lib, versioned, 'lib-dynload')
  if (existsSync(dynload)) {
    for (const entry of readdirSync(dynload)) {
      if (entry.startsWith('_tkinter')) drop(join('lib', versioned, 'lib-dynload', entry))
    }
  }

  const deduplicatedLibrary = deduplicateSharedLibrary(lib)
  return { removed, deduplicatedLibrary }
}

/** Point the unversioned soname at the versioned file instead of shipping both. */
function deduplicateSharedLibrary(lib: string): boolean {
  const versioned = readdirSync(lib).filter(entry => /^libpython3\.\d+\.so\.\d+\.\d+$/u.test(entry)).sort()
  if (versioned.length !== 1) return false
  const target = versioned[0] as string
  const unversioned = target.replace(/\.\d+\.\d+$/u, '')
  const path = join(lib, unversioned)
  if (!existsSync(path)) return false
  if (lstatSync(path).isSymbolicLink()) return false
  unlinkSync(path)
  symlinkSync(target, path)
  return true
}

/**
 * Prove the pruned interpreter still imports every documented module.
 *
 * @param pythonDir - Pruned payload root.
 * @throws when the interpreter cannot start or a required module is missing.
 */
export function verifyPythonPayload(pythonDir: string): void {
  const interpreters = readdirSync(join(pythonDir, 'bin'))
    .filter(entry => /^python3\.\d+$/u.test(entry))
    .sort()
  const interpreter = interpreters[0]
  if (interpreter === undefined) throw new Error('python payload: no bin/python3.N interpreter')
  execFileSync(join(pythonDir, 'bin', interpreter), [
    '-I', '-B', '-c',
    `import ${REQUIRED_IMPORTS.join(', ')}\n`,
  ], { stdio: 'inherit', timeout: 120_000 })
}
