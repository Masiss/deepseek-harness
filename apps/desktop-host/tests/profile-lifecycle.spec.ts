import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { APPLICATION_BUNDLES, desktopProfileBundles, prepareDesktopProfile } from '../src/profile-lifecycle.ts'

const BASE = '@deepseek-ai/dsh-base'
const WEB = '@deepseek-ai/dsh-web-app'
const FEATURE = '@masiss/dsh-load-probe'

let profile: string

function bundles(): string[] {
  const manifest = JSON.parse(readFileSync(join(profile, 'package.json'), 'utf8')) as {
    dsh?: { profile?: { bundles?: string[] } }
  }
  return manifest.dsh?.profile?.bundles ?? []
}

function writeManifest(value: unknown): void {
  writeFileSync(join(profile, 'package.json'), `${JSON.stringify(value, undefined, 2)}\n`)
}

beforeEach(() => {
  profile = mkdtempSync(join(tmpdir(), 'dsh-profile-lifecycle-'))
})

afterEach(() => {
  rmSync(profile, { recursive: true, force: true })
})

describe('prepareDesktopProfile', () => {
  it('creates a missing profile from the web template', () => {
    const result = prepareDesktopProfile(profile)
    expect(result).toEqual({ droppedBundles: [], restoredBundle: false })
    expect(bundles()).toEqual([...desktopProfileBundles()])
  })

  it('keeps a compatible profile and its third-party bundles untouched', () => {
    mkdirSync(profile, { recursive: true })
    writeManifest({
      name: 'user-profile',
      private: true,
      dependencies: { '@masiss/dsh-load-probe': '0.2.1-rc.2' },
      dsh: { profile: { bundles: [BASE, WEB, FEATURE] } },
    })
    const before = readFileSync(join(profile, 'package.json'), 'utf8')

    const result = prepareDesktopProfile(profile)

    expect(result).toEqual({ droppedBundles: [], restoredBundle: false })
    expect(readFileSync(join(profile, 'package.json'), 'utf8')).toBe(before)
  })

  it('drops a second application bundle and keeps feature bundles', () => {
    mkdirSync(profile, { recursive: true })
    writeManifest({
      name: 'user-profile',
      private: true,
      dependencies: { '@deepseek-ai/dsh-headless': '0.2.1-rc.2' },
      dsh: { profile: { bundles: [BASE, WEB, '@deepseek-ai/dsh-headless', FEATURE] } },
    })

    const result = prepareDesktopProfile(profile)

    expect(result).toEqual({ droppedBundles: ['@deepseek-ai/dsh-headless'], restoredBundle: false })
    expect(bundles()).toEqual([BASE, WEB, FEATURE])
    // Installed packages survive recovery; only the composition changes.
    expect(JSON.parse(readFileSync(join(profile, 'package.json'), 'utf8'))).toMatchObject({
      dependencies: { '@deepseek-ai/dsh-headless': '0.2.1-rc.2' },
    })
  })

  it('restores the web application bundle when another application replaced it', () => {
    mkdirSync(profile, { recursive: true })
    writeManifest({
      name: 'user-profile',
      private: true,
      dependencies: {},
      dsh: { profile: { bundles: [BASE, '@deepseek-ai/dsh-headless'] } },
    })

    const result = prepareDesktopProfile(profile)

    expect(result).toEqual({ droppedBundles: ['@deepseek-ai/dsh-headless'], restoredBundle: true })
    expect(bundles()).toEqual([BASE, WEB])
  })

  it('appends the web application bundle to an application-less profile', () => {
    mkdirSync(profile, { recursive: true })
    writeManifest({ name: 'user-profile', private: true, dependencies: {}, dsh: { profile: { bundles: [BASE, FEATURE] } } })

    expect(prepareDesktopProfile(profile)).toEqual({ droppedBundles: [], restoredBundle: true })
    expect(bundles()).toEqual([BASE, FEATURE, WEB])
  })

  it('lists the web application among the application bundles it arbitrates', () => {
    expect(APPLICATION_BUNDLES).toContain(WEB)
    expect(APPLICATION_BUNDLES).not.toContain(BASE)
  })
})
