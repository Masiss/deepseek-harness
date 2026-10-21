import { createHash } from 'node:crypto'
import { existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { zstdCompressSync } from 'node:zlib'
import { afterEach, describe, expect, it } from 'vitest'
import { parseComponentsManifest } from '../src/components/manifest.ts'
import { DesktopOfficeComponent } from '../src/components/office.ts'
import { extractTar, extractZstdTar } from '../src/components/tar.ts'

const roots: string[] = []

function scratch(): string {
  const root = mkdtempSync(join(tmpdir(), 'dsh-components-'))
  roots.push(root)
  return root
}

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true })
})

interface Entry {
  readonly name: string
  readonly type?: 'file' | 'dir' | 'symlink'
  readonly content?: string
  readonly mode?: number
  readonly link?: string
}

function field(value: string, length: number): Buffer {
  const buffer = Buffer.alloc(length)
  buffer.write(value, 0, 'ascii')
  return buffer
}

/** Write a ustar archive with exactly the entry types the reader accepts. */
function tarArchive(entries: readonly Entry[]): Buffer {
  const blocks: Buffer[] = []
  for (const entry of entries) {
    const data = Buffer.from(entry.content ?? '')
    const header = Buffer.alloc(512)
    field(entry.name, 100).copy(header, 0)
    field(`${(entry.mode ?? (entry.type === 'dir' ? 0o755 : 0o644)).toString(8).padStart(7, '0')}\0`, 8).copy(header, 100)
    field('0000000\0', 8).copy(header, 108)
    field('0000000\0', 8).copy(header, 116)
    field(`${(entry.type === 'file' || entry.type === undefined ? data.length : 0).toString(8).padStart(11, '0')}\0`, 12).copy(header, 124)
    field('00000000000\0', 12).copy(header, 136)
    field('        ', 8).copy(header, 148)
    field(entry.type === 'dir' ? '5' : entry.type === 'symlink' ? '2' : '0', 1).copy(header, 156)
    field(entry.link ?? '', 100).copy(header, 157)
    field('ustar', 6).copy(header, 257)
    field('00', 2).copy(header, 263)
    const checksum = header.reduce((sum, byte) => sum + byte, 0)
    field(`${checksum.toString(8).padStart(6, '0')}\0 `, 8).copy(header, 148)
    blocks.push(header)
    if (data.length > 0) {
      blocks.push(data)
      const padding = (512 - (data.length % 512)) % 512
      if (padding > 0) blocks.push(Buffer.alloc(padding))
    }
  }
  blocks.push(Buffer.alloc(1024))
  return Buffer.concat(blocks)
}

describe('component manifest', () => {
  const valid = {
    schemaVersion: 1,
    components: [{ id: 'office', version: '0.2.1-rc.2', file: 'office.tar.zst', bytes: 10, sha256: 'a'.repeat(64) }],
  }

  it('accepts the published shape', () => {
    expect(parseComponentsManifest(valid)).toEqual(valid)
  })

  it.each([
    ['wrong schema', { ...valid, schemaVersion: 2 }],
    ['malformed id', { ...valid, components: [{ ...valid.components[0], id: 'Office_1' }] }],
    ['missing version', { ...valid, components: [{ ...valid.components[0], version: undefined }] }],
    ['path in file', { ...valid, components: [{ ...valid.components[0], file: '../x' }] }],
    ['zero bytes', { ...valid, components: [{ ...valid.components[0], bytes: 0 }] }],
    ['an archive larger than the ceiling', { ...valid, components: [{ ...valid.components[0], bytes: 2 ** 40 }] }],
    ['short digest', { ...valid, components: [{ ...valid.components[0], sha256: 'abc' }] }],
    ['duplicate id', { ...valid, components: [valid.components[0], valid.components[0]] }],
  ])('rejects %s', (_name, value) => {
    expect(() => parseComponentsManifest(value)).toThrow(/invalid manifest|duplicate component/u)
  })
})

describe('tar extraction', () => {
  it('writes files, directories, and links below the destination', async () => {
    const root = scratch()
    const archive = join(root, 'a.tar')
    writeFileSync(archive, tarArchive([
      { name: 'runtime', type: 'dir' },
      { name: 'runtime/dependencies/python/bin/python3', type: 'file', content: '#!/bin/sh\n', mode: 0o755 },
      { name: 'runtime/link', type: 'symlink', link: 'dependencies/python/bin/python3' },
    ]))
    const destination = join(root, 'out')
    mkdirSync(destination)
    await extractTar(archive, destination)
    expect(readFileSync(join(destination, 'runtime/dependencies/python/bin/python3'), 'utf8')).toBe('#!/bin/sh\n')
    expect(lstatSync(join(destination, 'runtime/dependencies/python/bin/python3')).mode & 0o777).toBe(0o755)
    expect(lstatSync(join(destination, 'runtime/link')).isSymbolicLink()).toBe(true)
  })

  it('refuses an entry that escapes the destination', async () => {
    const root = scratch()
    const archive = join(root, 'a.tar')
    writeFileSync(archive, tarArchive([{ name: '../outside', type: 'file', content: 'x' }]))
    const destination = join(root, 'out')
    mkdirSync(destination)
    await expect(extractTar(archive, destination)).rejects.toThrow('escapes')
  })

  it('refuses a link that escapes the destination', async () => {
    const root = scratch()
    const archive = join(root, 'a.tar')
    writeFileSync(archive, tarArchive([{ name: 'link', type: 'symlink', link: '../../outside' }]))
    const destination = join(root, 'out')
    mkdirSync(destination)
    await expect(extractTar(archive, destination)).rejects.toThrow('escapes')
  })

  it('refuses an archive that exceeds its entry budget', async () => {
    const root = scratch()
    const archive = join(root, 'a.tar')
    writeFileSync(archive, tarArchive([
      { name: 'one', type: 'file', content: 'x' },
      { name: 'two', type: 'file', content: 'x' },
    ]))
    const destination = join(root, 'out')
    mkdirSync(destination)
    await expect(extractTar(archive, destination, { totalBytes: 1024, entries: 1, depth: 8, fileBytes: 1024 }))
      .rejects.toThrow('too many entries')
  })

  it('refuses an archive that exceeds its size budget', async () => {
    const root = scratch()
    const archive = join(root, 'a.tar')
    writeFileSync(archive, tarArchive([{ name: 'big', type: 'file', content: 'x'.repeat(64) }]))
    const destination = join(root, 'out')
    mkdirSync(destination)
    await expect(extractTar(archive, destination, { totalBytes: 16, entries: 8, depth: 8, fileBytes: 1024 }))
      .rejects.toThrow('size budget')
  })

  it('refuses an entry nested deeper than the budget allows', async () => {
    const root = scratch()
    const archive = join(root, 'a.tar')
    writeFileSync(archive, tarArchive([{ name: 'a/b/c/d', type: 'file', content: 'x' }]))
    const destination = join(root, 'out')
    mkdirSync(destination)
    await expect(extractTar(archive, destination, { totalBytes: 1024, entries: 8, depth: 2, fileBytes: 1024 }))
      .rejects.toThrow('nested too deeply')
  })

  it('refuses a compressed archive that expands past the budget', async () => {
    const root = scratch()
    const archive = join(root, 'bomb.tar.zst')
    // A small download that expands to far more than the budget allows.
    writeFileSync(archive, zstdCompressSync(tarArchive([
      { name: 'big', type: 'file', content: 'x'.repeat(64 * 1024) },
    ])))
    const destination = join(root, 'out')
    mkdirSync(destination)
    const scratchFile = join(root, 'bomb.tar')
    await expect(extractZstdTar(archive, destination, scratchFile, {
      totalBytes: 1024, entries: 8, depth: 8, fileBytes: 1024,
    })).rejects.toThrow('size budget')
    // The intermediate tar never outlives the attempt.
    expect(existsSync(scratchFile)).toBe(false)
  })
})

describe.skipIf(process.platform === 'win32')('office component install', () => {
  function fixture(): { primaryRuntime: string; directory: string } {
    const root = scratch()
    const primaryRuntime = join(root, 'primary-runtime')
    mkdirSync(join(primaryRuntime, 'dependencies', 'node', 'bin'), { recursive: true })
    mkdirSync(join(primaryRuntime, 'dependencies', 'pnpm'), { recursive: true })
    writeFileSync(join(primaryRuntime, 'dependencies', 'node', 'bin', 'node'), '')
    writeFileSync(join(primaryRuntime, 'runtime.json'), JSON.stringify({
      desktopVersion: '0.2.1-rc.2', platform: 'linux', arch: 'x64', python: '3.12.0', node: '24.0.0', pnpm: '10.0.0',
    }))
    return { primaryRuntime, directory: join(root, 'components') }
  }

  function archive(): Buffer {
    return zstdCompressSync(tarArchive([
      { name: 'runtime', type: 'dir' },
      { name: 'runtime/dependencies', type: 'dir' },
      { name: 'runtime/dependencies/python', type: 'dir' },
      { name: 'runtime/dependencies/python/bin', type: 'dir' },
      { name: 'runtime/dependencies/python/bin/python3', type: 'file', content: '#!/bin/sh\nexit 0\n', mode: 0o755 },
      { name: 'runtime/node_modules', type: 'dir' },
      { name: 'runtime/node_modules/@deepseek-ai', type: 'dir' },
      { name: 'runtime/node_modules/@deepseek-ai/libreoffice-kit', type: 'dir' },
      { name: 'runtime/node_modules/@deepseek-ai/libreoffice-kit/lib', type: 'dir' },
      {
        name: 'runtime/node_modules/@deepseek-ai/libreoffice-kit/lib/cli.js',
        type: 'file',
        content: 'export default {}\n',
      },
      { name: 'office-skills', type: 'dir' },
      { name: 'office-skills/SKILL.md', type: 'file', content: '# office' },
    ]))
  }

  function fetcher(payload: Buffer, digest: string): { fetch: typeof fetch; calls: string[] } {
    const calls: string[] = []
    const fetchImpl = (async (input: string | URL | Request) => {
      const url = input instanceof Request ? input.url : String(input)
      calls.push(url)
      if (url.endsWith('/components.json')) {
        return new Response(JSON.stringify({
          schemaVersion: 1,
          components: [{ id: 'office', version: '3.12.0', file: 'office.tar.zst', bytes: payload.length, sha256: digest }],
        }))
      }
      const bytes = new Uint8Array(payload.byteLength)
      bytes.set(payload)
      return new Response(bytes)
    })
    return { fetch: fetchImpl, calls }
  }

  it('installs, composes the runtime, and reports readiness', async () => {
    const payload = archive()
    const digest = createHash('sha256').update(payload).digest('hex')
    const { primaryRuntime, directory } = fixture()
    const { fetch, calls } = fetcher(payload, digest)
    const component = new DesktopOfficeComponent({ feed: 'http://feed.test/', directory, primaryRuntime, fetch })

    expect(component.presentation).toMatchObject({ installed: false, phase: 'idle' })
    expect(component.installed()).toBeUndefined()

    const settled = await component.install()
    expect(settled).toMatchObject({ installed: true, phase: 'ready', version: '3.12.0', active: false })
    const installed = component.installed()
    expect(installed?.version).toBe('3.12.0')
    expect(readFileSync(join(installed?.runtime ?? '', 'runtime.json'), 'utf8')).toContain('3.12.0')
    expect(lstatSync(join(installed?.runtime ?? '', 'dependencies', 'node')).isSymbolicLink()).toBe(true)
    expect(readFileSync(join(directory, 'versions', '3.12.0', 'office-skills', 'SKILL.md'), 'utf8')).toBe('# office')

    component.markActive()
    expect(component.presentation.active).toBe(true)

    const requests = calls.length
    await component.install()
    expect(calls.length).toBe(requests)
  })

  it('rejects an archive whose digest does not match', async () => {
    const payload = archive()
    const { primaryRuntime, directory } = fixture()
    const { fetch } = fetcher(payload, 'b'.repeat(64))
    const component = new DesktopOfficeComponent({ feed: 'http://feed.test', directory, primaryRuntime, fetch })

    const settled = await component.install()
    expect(settled).toMatchObject({ installed: false, phase: 'error', failure: 'integrity' })
    expect(component.installed()).toBeUndefined()
  })
})
