import { generateKeyPairSync, sign } from 'node:crypto'
import { describe, expect, it } from 'vitest'
import {
  parseUpdateManifest,
  selectUpdateArtifact,
  updateTarget,
  UpdateManifestError,
  type UpdateManifest,
} from '../src/update/manifest.ts'
import { signedFeedBytes } from '../src/update/signing.ts'

const DIGEST = 'a'.repeat(64)

// The suite signs with a key of its own, so a manifest it builds verifies
// against the canonicalization the client uses, and the pinned public key
// stays the one the release channel signs with.
const { privateKey, publicKey } = generateKeyPairSync('ed25519')
const PUBLIC_KEY = publicKey.export({ type: 'spki', format: 'pem' }).toString()

/** Sign the fields a manifest covers, the way the release channel does. */
function signDocument(document: Record<string, unknown>): string {
  const signed = {
    schemaVersion: document.schemaVersion,
    version: document.version,
    ...(document.minimumVersion === undefined ? {} : { minimumVersion: document.minimumVersion }),
    artifacts: document.artifacts,
  }
  return sign(null, signedFeedBytes(signed as never), privateKey).toString('base64')
}

/** One valid manifest with an artifact per platform this suite exercises. */
function manifest(overrides: Record<string, unknown> = {}): unknown {
  const document = {
    schemaVersion: 2,
    version: '0.3.0',
    artifacts: {
      'linux-x64': { url: 'https://example.test/app.AppImage', sha256: DIGEST, size: 10 },
      'darwin-arm64': { url: 'https://example.test/app.dmg', sha256: DIGEST, size: 20 },
    },
    ...overrides,
  }
  return { ...document, signature: signDocument(document) }
}

describe('parseUpdateManifest', () => {
  it('accepts a published release and keeps known targets', () => {
    const parsed = parseUpdateManifest(manifest(), PUBLIC_KEY)
    expect(parsed.version).toBe('0.3.0')
    expect(parsed.artifacts['linux-x64']).toEqual({
      url: 'https://example.test/app.AppImage', sha256: DIGEST, size: 10,
    })
  })

  it('ignores targets this client does not know', () => {
    const parsed = parseUpdateManifest(manifest({
      artifacts: {
        'linux-x64': { url: 'https://example.test/app.AppImage', sha256: DIGEST, size: 10 },
        'linux-riscv64': { url: 'https://example.test/app', sha256: 'nonsense', size: 0 },
      },
    }), PUBLIC_KEY)
    expect(Object.keys(parsed.artifacts)).toEqual(['linux-x64'])
  })

  it('accepts loopback HTTP so a release can be rehearsed locally', () => {
    const parsed = parseUpdateManifest(manifest({
      artifacts: { 'linux-x64': { url: 'http://127.0.0.1:8080/app', sha256: DIGEST, size: 1 } },
    }), PUBLIC_KEY)
    expect(parsed.artifacts['linux-x64']?.url).toBe('http://127.0.0.1:8080/app')
  })

  it.each([
    ['not-an-object', '$', null],
    ['not-an-object', '$', []],
    ['unsupported-schema', 'schemaVersion', { schemaVersion: 1, version: '0.3.0', artifacts: {} }],
    ['invalid-version', 'version', manifest({ version: 'latest' })],
    ['missing-artifacts', 'artifacts', manifest({ artifacts: [] })],
    ['missing-artifacts', 'artifacts', manifest({ artifacts: {} })],
    ['invalid-artifact', 'artifacts.linux-x64.url', manifest({
      artifacts: { 'linux-x64': { url: 'http://example.test/app', sha256: DIGEST, size: 10 } },
    })],
    ['invalid-artifact', 'artifacts.linux-x64.url', manifest({
      artifacts: { 'linux-x64': { url: 'ftp://example.test/app', sha256: DIGEST, size: 10 } },
    })],
    ['invalid-artifact', 'artifacts.linux-x64.sha256', manifest({
      artifacts: { 'linux-x64': { url: 'https://example.test/app', sha256: 'A'.repeat(64), size: 10 } },
    })],
    ['invalid-artifact', 'artifacts.linux-x64.size', manifest({
      artifacts: { 'linux-x64': { url: 'https://example.test/app', sha256: DIGEST, size: 0 } },
    })],
    ['invalid-artifact', 'artifacts.linux-x64.size', manifest({
      artifacts: { 'linux-x64': { url: 'https://example.test/app', sha256: DIGEST, size: 1.5 } },
    })],
  ])('refuses %s at %s', (failure, detail, input) => {
    const run = (): UpdateManifest => parseUpdateManifest(input, PUBLIC_KEY)
    expect(run).toThrow(UpdateManifestError)
    try {
      run()
    } catch (error) {
      expect(error).toMatchObject({ failure, detail })
    }
  })
})

describe('the feed signature', () => {
  it('refuses a manifest that carries no signature', () => {
    const document = manifest() as Record<string, unknown>
    delete document.signature
    expect(() => parseUpdateManifest(document, PUBLIC_KEY)).toThrow(/unsigned/u)
  })

  it('refuses a manifest whose artifact changed after signing', () => {
    const document = manifest() as { artifacts: Record<string, { size: number }> }
    document.artifacts['linux-x64']!.size = 11
    expect(() => parseUpdateManifest(document, PUBLIC_KEY)).toThrow(/invalid-signature/u)
  })

  it('refuses a manifest signed by another key', () => {
    const other = generateKeyPairSync('ed25519')
    const document = {
      schemaVersion: 2,
      version: '0.3.0',
      artifacts: { 'linux-x64': { url: 'https://example.test/app.AppImage', sha256: DIGEST, size: 10 } },
    }
    const signature = sign(null, signedFeedBytes(document), other.privateKey).toString('base64')
    expect(() => parseUpdateManifest({ ...document, signature }, PUBLIC_KEY)).toThrow(/invalid-signature/u)
  })

  it('accepts a manifest that names the oldest allowed client', () => {
    const parsed = parseUpdateManifest(manifest({ minimumVersion: '0.2.0' }), PUBLIC_KEY)
    expect(parsed.minimumVersion).toBe('0.2.0')
  })

  it('refuses a manifest whose minimum version is not a version', () => {
    expect(() => parseUpdateManifest(manifest({ minimumVersion: 'recent' }), PUBLIC_KEY)).toThrow(/invalid-minimum-version/u)
  })
})

describe('selectUpdateArtifact', () => {
  it('returns the artifact for the running target', () => {
    const parsed = parseUpdateManifest(manifest(), PUBLIC_KEY)
    expect(selectUpdateArtifact(parsed, 'darwin-arm64').size).toBe(20)
  })

  it('refuses a release that does not publish this target', () => {
    const parsed = parseUpdateManifest(manifest(), PUBLIC_KEY)
    expect(() => selectUpdateArtifact(parsed, 'win32-x64')).toThrow(/missing-artifact \(win32-x64\)/u)
  })
})

describe('updateTarget', () => {
  it.each([
    ['linux', 'x64', 'linux-x64'],
    ['linux', 'arm64', 'linux-arm64'],
    ['darwin', 'arm64', 'darwin-arm64'],
    ['win32', 'x64', 'win32-x64'],
  ] as const)('maps %s/%s to %s', (platform, arch, target) => {
    expect(updateTarget(platform, arch)).toBe(target)
  })

  it('refuses a target with no Desktop artifact', () => {
    expect(() => updateTarget('linux', 'mips')).toThrow(/unsupported-target \(linux-mips\)/u)
  })
})
