/** Signed update metadata for one shell's exact AppImage bytes. */
import { createHash, sign, type KeyObject } from 'node:crypto'
import { basename } from 'node:path'
import { signedFeedBytes } from '../../desktop-host/src/update/signing.ts'

/**
 * Build a signed schema-v2 feed without publishing it.
 * @param input - Artifact bytes, release URL, version, target, and channel key.
 * @returns JSON document binding the signature to the artifact URL, digest, and size.
 */
export function releaseManifest(input: {
  readonly artifact: string
  readonly bytes: Buffer
  readonly releaseBase: string
  readonly version: string
  readonly target: 'linux-x64' | 'linux-arm64'
  readonly minimumVersion?: string
  readonly key: KeyObject
}): string {
  const document = {
    schemaVersion: 2,
    version: input.version,
    ...(input.minimumVersion === undefined ? {} : { minimumVersion: input.minimumVersion }),
    artifacts: {
      [input.target]: {
        url: `${input.releaseBase}/${basename(input.artifact)}`,
        sha256: createHash('sha256').update(input.bytes).digest('hex'),
        size: input.bytes.length,
      },
    },
  }
  const signature = sign(null, signedFeedBytes(document), input.key).toString('base64')
  return `${JSON.stringify({ ...document, signature }, undefined, 2)}\n`
}
