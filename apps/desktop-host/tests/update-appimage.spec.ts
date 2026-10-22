import { createHash } from 'node:crypto'
import { chmod, mkdtemp, readFile, rm, stat, writeFile, mkdir } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import {
  appImageTarget,
  createAppImageDriver,
  installAppImage,
  UpdateInstallError,
  verifyArtifact,
} from '../src/update/appimage.ts'

let root: string

const OLD_IMAGE = 'old image bytes'
const NEW_IMAGE = 'new image bytes with an executable payload'

/** Write one file and return its digest and size. */
async function artifact(name: string, content: string): Promise<{ path: string; sha256: string; size: number }> {
  const path = join(root, name)
  await writeFile(path, content)
  return { path, sha256: createHash('sha256').update(content).digest('hex'), size: Buffer.byteLength(content) }
}

beforeEach(async () => {
  root = await mkdtemp(join(tmpdir(), 'dsh-update-appimage-'))
})

afterEach(async () => {
  await rm(root, { recursive: true, force: true })
})

describe('installAppImage', () => {
  it('replaces the running image with verified bytes', async () => {
    const target = join(root, 'dsh.AppImage')
    await writeFile(target, OLD_IMAGE)
    const next = await artifact('next.AppImage', NEW_IMAGE)

    await installAppImage({ artifactPath: next.path, targetPath: target, sha256: next.sha256, size: next.size })

    expect(await readFile(target, 'utf8')).toBe(NEW_IMAGE)
    expect((await stat(target)).mode & 0o777).toBe(0o755)
    await expect(stat(`${target}.new`)).rejects.toMatchObject({ code: 'ENOENT' })
  })

  it('keeps the running image when the bytes do not match the manifest', async () => {
    const target = join(root, 'dsh.AppImage')
    await writeFile(target, OLD_IMAGE)
    const next = await artifact('next.AppImage', NEW_IMAGE)

    const run = installAppImage({
      artifactPath: next.path, targetPath: target, sha256: 'b'.repeat(64), size: next.size,
    })
    await expect(run).rejects.toMatchObject({ failure: 'digest-mismatch' })
    expect(await readFile(target, 'utf8')).toBe(OLD_IMAGE)
  })

  it('keeps the running image when the artifact size does not match', async () => {
    const target = join(root, 'dsh.AppImage')
    await writeFile(target, OLD_IMAGE)
    const next = await artifact('next.AppImage', NEW_IMAGE)

    const run = installAppImage({
      artifactPath: next.path, targetPath: target, sha256: next.sha256, size: next.size + 1,
    })
    await expect(run).rejects.toMatchObject({ failure: 'size-mismatch' })
    expect(await readFile(target, 'utf8')).toBe(OLD_IMAGE)
  })

  it('refuses a missing artifact or a missing target', async () => {
    const target = join(root, 'dsh.AppImage')
    await writeFile(target, OLD_IMAGE)
    const next = await artifact('next.AppImage', NEW_IMAGE)

    await expect(installAppImage({
      artifactPath: join(root, 'absent'), targetPath: target, sha256: next.sha256, size: next.size,
    })).rejects.toMatchObject({ failure: 'artifact-missing' })

    await expect(installAppImage({
      artifactPath: next.path, targetPath: join(root, 'absent.AppImage'), sha256: next.sha256, size: next.size,
    })).rejects.toMatchObject({ failure: 'target-missing' })
  })

  it('replaces a staged image left by an interrupted install', async () => {
    const target = join(root, 'dsh.AppImage')
    await writeFile(target, OLD_IMAGE)
    await writeFile(`${target}.new`, 'half written')
    const next = await artifact('next.AppImage', NEW_IMAGE)

    await installAppImage({ artifactPath: next.path, targetPath: target, sha256: next.sha256, size: next.size })

    expect(await readFile(target, 'utf8')).toBe(NEW_IMAGE)
  })
})

describe('verifyArtifact', () => {
  it('accepts the bytes the manifest describes', async () => {
    const next = await artifact('verified.AppImage', NEW_IMAGE)
    await expect(verifyArtifact(next.path, next.sha256, next.size)).resolves.toBeUndefined()
  })

  it('refuses a staged copy whose bytes changed after the source was checked', async () => {
    const next = await artifact('staged.AppImage', NEW_IMAGE)
    await writeFile(next.path, 'x'.repeat(Buffer.byteLength(NEW_IMAGE)))

    await expect(verifyArtifact(next.path, next.sha256, next.size)).rejects.toMatchObject({ failure: 'digest-mismatch' })
  })

  it('refuses a staged copy whose length changed', async () => {
    const next = await artifact('staged.AppImage', NEW_IMAGE)

    await expect(verifyArtifact(next.path, next.sha256, next.size + 1)).rejects.toMatchObject({ failure: 'size-mismatch' })
  })
})

describe('createAppImageDriver', () => {
  it('refuses an installation the AppImage runtime did not name', async () => {
    const driver = createAppImageDriver({})
    await expect(driver.install({
      version: '0.3.0', artifactPath: join(root, 'next'), sha256: 'a'.repeat(64), size: 1,
    })).rejects.toBeInstanceOf(UpdateInstallError)
  })

  it('installs into $APPIMAGE', async () => {
    const target = join(root, 'dsh.AppImage')
    await writeFile(target, OLD_IMAGE)
    await chmod(target, 0o755)
    const next = await artifact('next.AppImage', NEW_IMAGE)
    await mkdir(join(root, 'unused'), { recursive: true })

    const driver = createAppImageDriver({ APPIMAGE: target })
    await driver.install({ version: '0.3.0', artifactPath: next.path, sha256: next.sha256, size: next.size })

    expect(await readFile(target, 'utf8')).toBe(NEW_IMAGE)
  })
})

describe('appImageTarget', () => {
  it('treats an empty variable as absent', () => {
    expect(appImageTarget({ APPIMAGE: '' })).toBeUndefined()
    expect(appImageTarget({ APPIMAGE: '/opt/dsh.AppImage' })).toBe('/opt/dsh.AppImage')
  })
})
