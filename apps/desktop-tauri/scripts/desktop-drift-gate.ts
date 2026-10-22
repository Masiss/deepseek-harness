/**
 * Refuse changes to the Electron carrier the Tauri lane does not own.
 *
 * The fork keeps the Electron application as the Windows/macOS bridge while
 * the Tauri shell takes over Linux. Two files were changed to let the Tauri
 * lane read the Electron layout: its `package.json` scripts and the build-path
 * helper. Every other edit under `apps/desktop/` would be an unreviewed rewrite
 * of the carrier that still ships to users, and would conflict with every
 * upstream change to it.
 *
 *   usage: desktop-drift-gate.ts [<base-ref>]
 *
 * @module @deepseek-ai/dsh-desktop-tauri/scripts/desktop-drift-gate
 */

import { spawnSync } from 'node:child_process'

/** Files the Tauri lane is allowed to change under `apps/desktop/`. */
export const ALLOWED_ELECTRON_CHANGES: readonly string[] = [
  'apps/desktop/package.json',
  'apps/desktop/scripts/desktop-build-paths.mjs',
]

/**
 * Select the changed files the lane does not own.
 *
 * @param changed - Paths changed under `apps/desktop/`.
 * @returns The paths outside the allowed set, in input order.
 */
export function electronDriftViolations(changed: readonly string[]): string[] {
  const allowed = new Set(ALLOWED_ELECTRON_CHANGES)
  return changed.filter(path => !allowed.has(path))
}

if (import.meta.main) {
  const base = process.argv[2] ?? 'origin/master'
  const result = spawnSync('git', ['diff', '--name-only', `${base}...HEAD`, '--', 'apps/desktop'], {
    encoding: 'utf8',
  })
  if (result.error !== undefined) throw result.error
  if (result.status !== 0) {
    throw new Error(`desktop drift gate: git diff against ${base} failed: ${result.stderr.trim()}`)
  }

  const changed = result.stdout.split('\n').map(line => line.trim()).filter(line => line !== '')
  const violations = electronDriftViolations(changed)
  if (violations.length === 0) {
    console.log(`desktop drift gate: ${String(changed.length)} changed file(s), all allowed`)
    process.exit(0)
  }
  console.error('desktop drift gate: changes outside the two files the Tauri lane owns:')
  for (const path of violations) console.error(`  ${path}`)
  console.error(`allowed: ${ALLOWED_ELECTRON_CHANGES.join(', ')}`)
  process.exit(1)
}
