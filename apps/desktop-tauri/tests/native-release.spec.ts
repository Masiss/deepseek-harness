/** Native and webview feeds bind different artifact bytes to the same channel key. */
import { createHash, generateKeyPairSync } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'
import { releaseManifest } from '../scripts/release-manifest.ts'
import { parseUpdateManifest, selectUpdateArtifact } from '../../desktop-host/src/update/manifest.ts'

const script = fileURLToPath(new URL('../scripts/appimage-feed-env.sh', import.meta.url))

describe('native release feeds', () => {
  it.each(['linux-x64', 'linux-arm64'] as const)('signs the exact native artifact for %s', (target) => {
    const keys = generateKeyPairSync('ed25519')
    const publicKey = keys.publicKey.export({ type: 'spki', format: 'pem' }).toString()
    const bytes = Buffer.from('native executable bytes')
    const feed: unknown = JSON.parse(releaseManifest({
      artifact: '/build/DeepSeek-Harness-native.AppImage', bytes,
      version: '2.0.0', minimumVersion: '1.0.0', target,
      releaseBase: 'https://example.com/release', key: keys.privateKey,
    }))
    const manifest = parseUpdateManifest(feed, publicKey)
    const artifact = selectUpdateArtifact(manifest, target)
    expect(artifact).toEqual({
      url: 'https://example.com/release/DeepSeek-Harness-native.AppImage',
      sha256: createHash('sha256').update(bytes).digest('hex'), size: bytes.length,
    })
    const tampered = { ...manifest, artifacts: { [target]: { ...artifact, url: 'https://example.com/release/webview.AppImage' } } }
    expect(() => parseUpdateManifest(tampered, publicKey)).toThrow()
  })

  it.skipIf(process.platform === 'win32')('bakes defaults without executing shell metacharacters and preserves runtime overrides', () => {
    const update = "https://example.com/manifest-native.json?quote='&literal=$(exit 91)`exit 92`"
    const component = 'https://example.com/components'
    const generated = spawnSync('sh', [script], {
      encoding: 'utf8', timeout: 10_000, env: { PATH: process.env.PATH, DSH_APPIMAGE_UPDATE_FEED: update, DSH_COMPONENT_FEED: component },
    })
    expect(generated.error).toBeUndefined()
    expect(generated.signal).toBeNull()
    expect(generated.status).toBe(0)
    for (const override of [undefined, 'https://override.example/manifest-native.json']) {
      const result = spawnSync('sh', ['-c', `${generated.stdout}\nprintf '%s\\n%s' "$DSH_DESKTOP_UPDATE_FEED" "$DSH_DESKTOP_COMPONENT_FEED"`], {
        encoding: 'utf8', timeout: 10_000, env: override === undefined ? {} : { DSH_DESKTOP_UPDATE_FEED: override },
      })
      expect(result.error).toBeUndefined()
      expect(result.signal).toBeNull()
      expect(result.status).toBe(0)
      expect(result.stdout).toBe(`${override ?? update}\n${component}`)
    }
  })
})
