/** The `rename` and `delete` endpoints: guarded file moves and deletions. */
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { mkdir, readFile, stat, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { openWorkspace, signal, type Harness } from './harness.ts'

let harness: Harness
let workspace: string

beforeEach(async () => {
  harness = await openWorkspace('dsh-workspace-files-mutate-')
  workspace = harness.workspace
})

afterEach(async () => {
  await harness.dispose()
})

describe('workspaceFiles.rename', () => {
  it('moves a file and returns the destination version', async () => {
    await writeFile(join(workspace, 'a.txt'), 'one\n', 'utf8')
    const read = await harness.endpoint().read(harness.scope, 'a.txt', {}, signal())
    const result = await harness.endpoint().rename(harness.scope, 'a.txt', 'b.txt', read.version, signal())
    expect(result.previousPath).toBe('a.txt')
    expect(result.path).toBe('b.txt')
    expect(result.version).not.toBe(read.version)
    await expect(readFile(join(workspace, 'b.txt'), 'utf8')).resolves.toBe('one\n')
    await expect(stat(join(workspace, 'a.txt'))).rejects.toThrow()
  })

  it('creates a destination directory path relative to the workspace', async () => {
    await mkdir(join(workspace, 'nested'), { recursive: true })
    await writeFile(join(workspace, 'a.txt'), 'one\n', 'utf8')
    const result = await harness.endpoint().rename(harness.scope, 'a.txt', 'nested/b.txt', undefined, signal())
    expect(result.path).toBe('nested/b.txt')
    await expect(readFile(join(workspace, 'nested', 'b.txt'), 'utf8')).resolves.toBe('one\n')
  })

  it('refuses a stale version and an occupied destination', async () => {
    await writeFile(join(workspace, 'a.txt'), 'one\n', 'utf8')
    await writeFile(join(workspace, 'b.txt'), 'two\n', 'utf8')
    await expect(harness.endpoint().rename(harness.scope, 'a.txt', 'c.txt', 'stale', signal()))
      .rejects.toMatchObject({ code: 'workspace-file/stale-version' })
    await expect(harness.endpoint().rename(harness.scope, 'a.txt', 'b.txt', undefined, signal()))
      .rejects.toMatchObject({ code: 'workspace-file/already-exists' })
    await expect(harness.endpoint().rename(harness.scope, 'missing.txt', 'd.txt', undefined, signal()))
      .rejects.toMatchObject({ code: 'workspace-file/not-found' })
  })

  it('rejects empty or identical paths', async () => {
    await expect(harness.endpoint().rename(harness.scope, '', 'b.txt', undefined, signal()))
      .rejects.toMatchObject({ code: 'gateway/bad-request' })
    await expect(harness.endpoint().rename(harness.scope, 'a.txt', 'a.txt', undefined, signal()))
      .rejects.toMatchObject({ code: 'gateway/bad-request' })
  })
})

describe('workspaceFiles.delete', () => {
  it('deletes a file and reports its canonical path', async () => {
    await writeFile(join(workspace, 'a.txt'), 'one\n', 'utf8')
    const result = await harness.endpoint().delete(harness.scope, 'a.txt', undefined, signal())
    expect(result.path).toBe('a.txt')
    expect(result.absolutePath.endsWith('a.txt')).toBe(true)
    await expect(stat(join(workspace, 'a.txt'))).rejects.toThrow()
  })

  it('refuses a stale version, a missing file, and a directory', async () => {
    await writeFile(join(workspace, 'a.txt'), 'one\n', 'utf8')
    await mkdir(join(workspace, 'dir'))
    await expect(harness.endpoint().delete(harness.scope, 'a.txt', 'stale', signal()))
      .rejects.toMatchObject({ code: 'workspace-file/stale-version' })
    await expect(harness.endpoint().delete(harness.scope, 'missing.txt', undefined, signal()))
      .rejects.toMatchObject({ code: 'workspace-file/not-found' })
    await expect(harness.endpoint().delete(harness.scope, 'dir', undefined, signal()))
      .rejects.toMatchObject({ code: 'workspace-file/not-regular-file' })
  })
})
