import { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, expect, it } from 'vitest'
import { DesktopOfficeComponent } from '../src/components/office.ts'

const roots: string[] = []
afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true })
})

function fixture(current: string, target: string, backup: string): {
  readonly directory: string
  readonly marker: string
  readonly targetPath: string
  readonly backupPath: string
} {
  const directory = mkdtempSync(join(tmpdir(), 'dsh-rename-'))
  roots.push(directory)
  const versions = join(directory, 'versions')
  mkdirSync(join(versions, current), { recursive: true })
  mkdirSync(join(versions, target), { recursive: true })
  mkdirSync(join(versions, backup), { recursive: true })
  writeFileSync(join(versions, current, 'tree'), 'old')
  writeFileSync(join(versions, target, 'tree'), 'new')
  writeFileSync(join(versions, backup, 'tree'), 'old-backup')
  const marker = join(directory, 'current.json')
  writeFileSync(marker, JSON.stringify({ version: current }))
  return { directory, marker, targetPath: join(versions, target), backupPath: join(versions, backup) }
}

/** Read the version the marker names, without trusting the parsed value. */
function markerVersion(marker: string): string | undefined {
  const value: unknown = JSON.parse(readFileSync(marker, 'utf8'))
  if (typeof value !== 'object' || value === null || !('version' in value)) return undefined
  return typeof value.version === 'string' ? value.version : undefined
}

it('rolls back a rename interrupted before marker publication', async () => {
  const f = fixture('old', 'new', '.backup-crash')
  writeFileSync(join(f.directory, 'rename-journal.json'), JSON.stringify({
    oldVersion: 'old', version: 'new', target: f.targetPath, backup: f.backupPath,
    markerTemporary: join(f.directory, 'current.json.tmp'), staging: join(f.directory, 'staging-crash'),
  }))
  const component = new DesktopOfficeComponent({ feed: 'http://127.0.0.1', directory: f.directory, primaryRuntime: f.directory })
  await component.recover()
  expect(markerVersion(f.marker)).toBe('old')
  expect(readFileSync(join(f.directory, 'versions', 'old', 'tree'), 'utf8')).toBe('old')
  expect(readdirSync(join(f.directory, 'versions'))).toEqual(['old'])
  expect(readdirSync(f.directory).filter(name => name.includes('journal') || name.includes('tmp'))).toEqual([])
})

it('keeps the new tree when marker publication completed before a crash', async () => {
  const f = fixture('old', 'new', '.backup-crash')
  writeFileSync(f.marker, JSON.stringify({ version: 'new' }))
  writeFileSync(join(f.directory, 'rename-journal.json'), JSON.stringify({
    oldVersion: 'old', version: 'new', target: f.targetPath, backup: f.backupPath,
    markerTemporary: join(f.directory, 'current.json.tmp'), staging: join(f.directory, 'staging-crash'),
  }))
  const component = new DesktopOfficeComponent({ feed: 'http://127.0.0.1', directory: f.directory, primaryRuntime: f.directory })
  await component.recover()
  expect(markerVersion(f.marker)).toBe('new')
  expect(readdirSync(join(f.directory, 'versions'))).toEqual(['new'])
  expect(readdirSync(f.directory).filter(name => name.includes('journal') || name.includes('tmp'))).toEqual([])
})
