import { describe, expect, it } from 'vitest'
import {
  parseUpdateManifest,
  selectUpdateArtifact,
  updateTarget,
  UpdateManifestError,
  type UpdateManifest,
} from '../src/update/manifest.ts'

const DIGEST = 'a'.repeat(64)

/** One valid manifest with an artifact per platform this suite exercises. */
function manifest(overrides: Record<string, unknown> = {}): unknown {
  return {
    schemaVersion: 1,
    version: '0.3.0',
    artifacts: {
      'linux-x64': { url: 'https://example.test/app.AppImage', sha256: DIGEST, size: 10 },
      'darwin-arm64': { url: 'https://example.test/app.dmg', sha256: DIGEST, size: 20 },
    },
    ...overrides,
  }
}

describe('parseUpdateManifest', () => {
  it('accepts a published release and keeps known targets', () => {
    const parsed = parseUpdateManifest(manifest())
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
    }))
    expect(Object.keys(parsed.artifacts)).toEqual(['linux-x64'])
  })

  it('accepts loopback HTTP so a release can be rehearsed locally', () => {
    const parsed = parseUpdateManifest(manifest({
      artifacts: { 'linux-x64': { url: 'http://127.0.0.1:8080/app', sha256: DIGEST, size: 1 } },
    }))
    expect(parsed.artifacts['linux-x64']?.url).toBe('http://127.0.0.1:8080/app')
  })

  it.each([
    ['not-an-object', '$', null],
    ['not-an-object', '$', []],
    ['unsupported-schema', 'schemaVersion', { schemaVersion: 2, version: '0.3.0', artifacts: {} }],
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
    const run = (): UpdateManifest => parseUpdateManifest(input)
    expect(run).toThrow(UpdateManifestError)
    try {
      run()
    } catch (error) {
      expect(error).toMatchObject({ failure, detail })
    }
  })
})

describe('selectUpdateArtifact', () => {
  it('returns the artifact for the running target', () => {
    const parsed = parseUpdateManifest(manifest())
    expect(selectUpdateArtifact(parsed, 'darwin-arm64').size).toBe(20)
  })

  it('refuses a release that does not publish this target', () => {
    const parsed = parseUpdateManifest(manifest())
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
