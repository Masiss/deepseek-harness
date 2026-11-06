/** The `search` endpoint: bounded literal substring search over workspace files. */
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { mkdir, rm, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { openWorkspace, signal, type Harness } from './harness.ts'

let harness: Harness
let workspace: string

beforeEach(async () => {
  harness = await openWorkspace('dsh-workspace-files-search-')
  workspace = harness.workspace
})

afterEach(async () => {
  await harness.dispose()
})

describe('workspaceFiles.search', () => {
  it('finds case-insensitive matches with 1-based lines in nested files', async () => {
    await writeFile(join(workspace, 'notes.txt'), 'hello\nNeedle here\nplain\nneedle again\n', 'utf8')
    await mkdir(join(workspace, 'sub'), { recursive: true })
    await writeFile(join(workspace, 'sub', 'code.ts'), 'const needle = 1\n', 'utf8')
    const result = await harness.endpoint().search(harness.scope, 'needle', undefined, signal())
    expect(result.truncated).toBe(false)
    expect(result.scannedFiles).toBeGreaterThanOrEqual(2)
    expect(result.matches).toContainEqual({ path: 'notes.txt', line: 2, text: 'Needle here' })
    expect(result.matches).toContainEqual({ path: 'notes.txt', line: 4, text: 'needle again' })
    expect(result.matches).toContainEqual({ path: join('sub', 'code.ts').replaceAll('\\', '/'), line: 1, text: 'const needle = 1' })
  })

  it('skips binary files and bounds the result count', async () => {
    await writeFile(join(workspace, 'bin.dat'), Buffer.from([0x6e, 0x65, 0x65, 0x64, 0x6c, 0x65, 0x00, 0x6e, 0x65, 0x65]))
    await writeFile(join(workspace, 'many.txt'), Array.from({ length: 20 }, () => 'needle line').join('\n'), 'utf8')
    const result = await harness.endpoint().search(harness.scope, 'needle', { maxResults: 3 }, signal())
    expect(result.truncated).toBe(true)
    expect(result.matches).toHaveLength(3)
    expect(result.matches.every(match => match.path === 'many.txt')).toBe(true)
  })

  it('rejects empty and malformed bounds', async () => {
    await expect(harness.endpoint().search(harness.scope, '', undefined, signal()))
      .rejects.toMatchObject({ code: 'gateway/bad-request' })
    await expect(harness.endpoint().search(harness.scope, 'q', { maxResults: 0 }, signal()))
      .rejects.toMatchObject({ code: 'gateway/bad-request' })
    await expect(harness.endpoint().search(harness.scope, 'q', { maxFiles: 5001 }, signal()))
      .rejects.toMatchObject({ code: 'gateway/bad-request' })
  })

  it('removes the per-call scratch workspace', async () => {
    await rm(join(workspace, 'gone.txt'), { force: true })
    await writeFile(join(workspace, 'gone.txt'), 'needle\n', 'utf8')
    const result = await harness.endpoint().search(harness.scope, 'needle', undefined, signal())
    expect(result.matches).toHaveLength(1)
  })
})
