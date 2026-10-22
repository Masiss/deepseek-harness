import { describe, expect, it } from 'vitest'
import { ALLOWED_ELECTRON_CHANGES, electronDriftViolations } from '../scripts/desktop-drift-gate.ts'

describe('electronDriftViolations', () => {
  it('accepts the two files the Tauri lane owns', () => {
    expect(electronDriftViolations([...ALLOWED_ELECTRON_CHANGES])).toEqual([])
  })

  it('reports an edit to the Electron carrier', () => {
    expect(electronDriftViolations([
      'apps/desktop/package.json',
      'apps/desktop/src/main.ts',
    ])).toEqual(['apps/desktop/src/main.ts'])
  })

  it('reports a new Electron source file', () => {
    expect(electronDriftViolations(['apps/desktop/scripts/new-packager.mjs']))
      .toEqual(['apps/desktop/scripts/new-packager.mjs'])
  })
})
