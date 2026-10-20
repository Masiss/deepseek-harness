/**
 * Minimal reader for the zstd-compressed tar archives the component lane
 * publishes.
 *
 * The archives are produced by this repository's own packaging step, so the
 * reader accepts exactly what GNU tar writes for them: regular files,
 * directories, symbolic links, and the PAX and GNU headers that carry long
 * names. Anything else fails the install instead of being approximated.
 *
 * @module @deepseek-ai/dsh-desktop-host/components/tar
 */

import { createReadStream, createWriteStream } from 'node:fs'
import { chmod, mkdir, open, rm, symlink } from 'node:fs/promises'
import type { FileHandle } from 'node:fs/promises'
import { dirname, isAbsolute, join, resolve, sep } from 'node:path'
import { pipeline } from 'node:stream/promises'
import { createZstdDecompress } from 'node:zlib'

const BLOCK = 512

/** Whether every path segment stays below the extraction root. */
function inside(root: string, path: string): boolean {
  const target = resolve(root, path)
  return target === root || target.startsWith(`${root}${sep}`)
}

/** Read a NUL- or space-terminated ASCII field. */
function text(header: Buffer, offset: number, length: number): string {
  return header.toString('ascii', offset, offset + length).replace(/\0.*$/su, '').trim()
}

/** Read an octal numeric field, where an empty field means zero. */
function octal(header: Buffer, offset: number, length: number): number {
  const value = text(header, offset, length)
  return value === '' ? 0 : Number.parseInt(value, 8)
}

/** Parse the `key=value` records of a PAX extended header. */
function paxRecords(payload: Buffer): Map<string, string> {
  const records = new Map<string, string>()
  let offset = 0
  while (offset < payload.length) {
    const space = payload.indexOf(0x20, offset)
    if (space < 0) break
    const length = Number.parseInt(payload.toString('ascii', offset, space), 10)
    if (!Number.isSafeInteger(length) || length <= 0 || offset + length > payload.length) break
    const record = payload.subarray(space + 1, offset + length - 1).toString('utf8')
    const equals = record.indexOf('=')
    if (equals > 0) records.set(record.slice(0, equals), record.slice(equals + 1))
    offset += length
  }
  return records
}

/** Read the complete payload of one header at `offset`, then its data blocks. */
async function readEntry(handle: FileHandle, offset: number, size: number): Promise<Buffer> {
  const buffer = Buffer.alloc(size)
  if (size > 0) await handle.read(buffer, 0, size, offset)
  return buffer
}

/**
 * Extract one tar archive into an empty directory.
 *
 * @param archive - Path to the uncompressed tar file.
 * @param destination - Existing directory the entries are written below.
 * @returns Resolves after every entry is written and its mode applied.
 * @throws When an entry escapes the destination, or its type is not supported.
 */
export async function extractTar(archive: string, destination: string): Promise<void> {
  const root = resolve(destination)
  const handle = await open(archive, 'r')
  try {
    let offset = 0
    let pendingPath: string | undefined
    let pendingLink: string | undefined
    for (;;) {
      const header = await readEntry(handle, offset, BLOCK)
      if (header.every(byte => byte === 0)) break
      const magic = text(header, 257, 6)
      if (magic !== 'ustar' && magic !== 'ustar\0') throw new Error('desktop components: unsupported tar header')
      const name = text(header, 0, 100)
      const prefix = text(header, 345, 155)
      const type = String.fromCharCode(header[156] ?? 0)
      const size = octal(header, 124, 12)
      const mode = octal(header, 100, 8) & 0o777
      const dataOffset = offset + BLOCK
      offset = dataOffset + Math.ceil(size / BLOCK) * BLOCK

      if (type === 'x') {
        const records = paxRecords(await readEntry(handle, dataOffset, size))
        pendingPath = records.get('path') ?? pendingPath
        pendingLink = records.get('linkpath') ?? pendingLink
        continue
      }
      if (type === 'g') continue
      if (type === 'L' || type === 'K') {
        const payload = (await readEntry(handle, dataOffset, size)).toString('utf8').replace(/\0.*$/su, '')
        if (type === 'L') pendingPath = payload
        else pendingLink = payload
        continue
      }

      const entryPath = pendingPath ?? (prefix === '' ? name : `${prefix}/${name}`)
      const linkTarget = pendingLink ?? text(header, 157, 100)
      pendingPath = undefined
      pendingLink = undefined
      if (entryPath === '' || entryPath === '.' || entryPath === './') continue
      if (!inside(root, entryPath)) throw new Error(`desktop components: archive entry escapes the destination: ${entryPath}`)
      const target = resolve(root, entryPath)

      if (type === '5') {
        await mkdir(target, { recursive: true, mode: mode === 0 ? 0o755 : mode })
        continue
      }
      if (type === '0' || type === '\0') {
        await mkdir(dirname(target), { recursive: true })
        if (size === 0) {
          await (await open(target, 'w', mode === 0 ? 0o644 : mode)).close()
          continue
        }
        const source = await open(archive, 'r')
        try {
          await pipeline(
            source.createReadStream({ start: dataOffset, end: dataOffset + size - 1 }),
            createWriteStream(target, { mode: mode === 0 ? 0o644 : mode }),
          )
        } finally {
          await source.close()
        }
        await chmod(target, mode === 0 ? 0o644 : mode)
        continue
      }
      if (type === '2') {
        if (isAbsolute(linkTarget) || !inside(root, join(dirname(entryPath), linkTarget))) {
          throw new Error(`desktop components: archive link escapes the destination: ${entryPath}`)
        }
        await mkdir(dirname(target), { recursive: true })
        await symlink(linkTarget, target)
        continue
      }
      throw new Error(`desktop components: unsupported tar entry type ${JSON.stringify(type)} at ${entryPath}`)
    }
  } finally {
    await handle.close()
  }
}

/**
 * Decompress a zstd archive and extract its tar contents.
 *
 * @param archive - Path to the `.tar.zst` file.
 * @param destination - Existing directory the entries are written below.
 * @param scratch - Path for the intermediate uncompressed tar, removed afterwards.
 * @returns Resolves after the archive is fully extracted.
 */
export async function extractZstdTar(archive: string, destination: string, scratch: string): Promise<void> {
  await pipeline(createReadStream(archive), createZstdDecompress(), createWriteStream(scratch))
  try {
    await extractTar(scratch, destination)
  } finally {
    await rm(scratch, { force: true })
  }
}
