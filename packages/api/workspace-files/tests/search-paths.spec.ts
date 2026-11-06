/** The `searchPaths` endpoint: bounded literal substring search over workspace paths. */
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { mkdir, rm, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { openWorkspace, signal, type Harness } from './harness.ts'

let harness: Harness
let workspace: string

beforeEach(async () => {
  harness = await openWorkspace('dsh-workspace-files-search-paths-')
  workspace = harness.workspace
})

afterEach(async () => {
  await harness.dispose()
  await rm(workspace, { recursive: true, force: true })
})

describe('workspaceFiles.searchPaths', () => {
  it('matches path substrings case-insensitively across directories', async () => {
    await mkdir(join(workspace, 'src', 'components'), { recursive: true })
    await writeFile(join(workspace, 'src', 'components', 'Button.tsx'), 'x', 'utf8')
    await writeFile(join(workspace, 'src', 'index.ts'), 'x', 'utf8')
    await writeFile(join(workspace, 'readme.md'), 'x', 'utf8')
    const result = await harness.endpoint().searchPaths(harness.scope, 'button', undefined, signal())
    expect(result).toEqual({ paths: ['src/components/Button.tsx'], truncated: false })
  })

  it('returns sorted paths and bounds the result count', async () => {
    for (const name of ['c.ts', 'a.ts', 'b.ts']) {
      await writeFile(join(workspace, name), 'x', 'utf8')
    }
    const result = await harness.endpoint().searchPaths(harness.scope, '.ts', undefined, signal())
    expect(result.paths).toEqual(['a.ts', 'b.ts', 'c.ts'])
    expect(result.truncated).toBe(false)

    const bounded = await harness.endpoint().searchPaths(harness.scope, '.ts', { maxResults: 2 }, signal())
    expect(bounded.paths).toHaveLength(2)
    expect(bounded.truncated).toBe(true)
  })

  it('reports truncation when the examined-file cap drops entries', async () => {
    await writeFile(join(workspace, 'a.ts'), 'x', 'utf8')
    await writeFile(join(workspace, 'b.ts'), 'x', 'utf8')
    const result = await harness.endpoint().searchPaths(harness.scope, 'b.ts', { maxFiles: 1 }, signal())
    expect(result).toEqual({ paths: [], truncated: true })
  })

  it('rejects an empty or oversized query and out-of-range bounds', async () => {
    await expect(harness.endpoint().searchPaths(harness.scope, '', undefined, signal()))
      .rejects.toMatchObject({ code: 'gateway/bad-request' })
    await expect(harness.endpoint().searchPaths(harness.scope, 'q', { maxResults: 0 }, signal()))
      .rejects.toMatchObject({ code: 'gateway/bad-request' })
    await expect(harness.endpoint().searchPaths(harness.scope, 'q', { maxResults: 501 }, signal()))
      .rejects.toMatchObject({ code: 'gateway/bad-request' })
    await expect(harness.endpoint().searchPaths(harness.scope, 'q', { maxFiles: 5001 }, signal()))
      .rejects.toMatchObject({ code: 'gateway/bad-request' })
  })
})
