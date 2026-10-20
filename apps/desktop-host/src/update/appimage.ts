/**
 * Installing a verified AppImage over the one the application is running from.
 *
 * The replacement is crash-safe by construction: the new image is written
 * beside the running one, its bytes are flushed, and only then does one
 * `rename` swap the directory entry. A crash at any earlier point leaves the
 * running image untouched; a crash after it leaves the new image complete.
 *
 * @module @deepseek-ai/dsh-desktop-host/update/appimage
 */

import { createHash } from 'node:crypto'
import { createReadStream } from 'node:fs'
import { chmod, copyFile, open, rename, rm, stat, unlink } from 'node:fs/promises'
import { dirname } from 'node:path'

/** What a driver installs. */
export interface UpdateInstallPlan {
  /** Version the bytes carry; recorded by the journal, never used to name paths. */
  readonly version: string
  /** Verified artifact on disk. */
  readonly artifactPath: string
  /** Expected lowercase SHA-256 of the artifact. */
  readonly sha256: string
  /** Expected artifact length in bytes. */
  readonly size: number
}

/** Why an install was refused or failed. */
export type UpdateInstallFailure =
  | 'unsupported'
  | 'artifact-missing'
  | 'size-mismatch'
  | 'digest-mismatch'
  | 'target-missing'
  | 'replace-failed'

/** A refused or failed install with a fixed classification. */
export class UpdateInstallError extends Error {
  /**
   * @param failure - Fixed classification the service reports.
   * @param detail - Filesystem object involved, never artifact content.
   */
  constructor(readonly failure: UpdateInstallFailure, readonly detail: string) {
    super(`desktop update install: ${failure} (${detail})`)
    this.name = 'UpdateInstallError'
  }
}

/** Installs one verified artifact over the running installation. */
export interface UpdateInstallerDriver {
  /** Short driver name recorded in the journal. */
  readonly kind: string
  /**
   * Replace the installed application with the artifact.
   * @param plan - Verified artifact identity.
   * @returns Nothing once the running installation points at the new bytes.
   */
  install(plan: UpdateInstallPlan): Promise<void>
}

/** Streaming SHA-256 of one file. */
export async function fileDigest(path: string): Promise<string> {
  const hash = createHash('sha256')
  for await (const chunk of createReadStream(path)) hash.update(chunk as Buffer)
  return hash.digest('hex')
}

/** Flush one directory entry so a rename that returned survives a power loss. */
async function syncDirectory(directory: string): Promise<void> {
  const handle = await open(directory, 'r')
  try {
    await handle.sync()
  } catch (error) {
    // Filesystems that refuse directory fsync report EINVAL; the rename is
    // still ordered against the file's own flush above.
    if ((error as NodeJS.ErrnoException).code !== 'EINVAL') throw error
  } finally {
    await handle.close()
  }
}

/** Options for one AppImage replacement. */
export interface AppImageInstallOptions {
  /** Verified artifact path. */
  readonly artifactPath: string
  /** Image the shell is running from; `$APPIMAGE`. */
  readonly targetPath: string
  /** Expected lowercase SHA-256. */
  readonly sha256: string
  /** Expected artifact length in bytes. */
  readonly size: number
}

/**
 * Verify an artifact and atomically replace one AppImage with it.
 *
 * @param options - Artifact path, target image, and the expected identity.
 * @returns Nothing after the directory entry points at the verified bytes.
 * @throws {UpdateInstallError} when verification or replacement fails; the target is unchanged.
 */
export async function installAppImage(options: AppImageInstallOptions): Promise<void> {
  const { artifactPath, targetPath, sha256, size } = options

  let artifact: Awaited<ReturnType<typeof stat>>
  try {
    artifact = await stat(artifactPath)
  } catch {
    throw new UpdateInstallError('artifact-missing', artifactPath)
  }
  if (!artifact.isFile()) throw new UpdateInstallError('artifact-missing', artifactPath)
  if (artifact.size !== size) throw new UpdateInstallError('size-mismatch', artifactPath)
  if (await fileDigest(artifactPath) !== sha256) throw new UpdateInstallError('digest-mismatch', artifactPath)

  let target: Awaited<ReturnType<typeof stat>>
  try {
    target = await stat(targetPath)
  } catch {
    throw new UpdateInstallError('target-missing', targetPath)
  }
  if (!target.isFile()) throw new UpdateInstallError('target-missing', targetPath)

  // Same directory: the rename must not cross a filesystem, and the staged
  // name is recognisable if a crashed install leaves it behind.
  const stagedPath = `${targetPath}.new`
  try {
    await unlink(stagedPath).catch((error: unknown) => {
      if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error
    })
    await copyFile(artifactPath, stagedPath)
    await chmod(stagedPath, 0o755)
    const staged = await open(stagedPath, 'r+')
    try { await staged.sync() } finally { await staged.close() }
    await rename(stagedPath, targetPath)
  } catch (error) {
    await rm(stagedPath, { force: true })
    if (error instanceof UpdateInstallError) throw error
    throw new UpdateInstallError('replace-failed', targetPath)
  }
  await syncDirectory(dirname(targetPath))
}

/** Read the image path the AppImage runtime exports. */
export function appImageTarget(env: NodeJS.ProcessEnv): string | undefined {
  const value = env.APPIMAGE
  return value === undefined || value === '' ? undefined : value
}

/**
 * Build the driver for a shell packaged as an AppImage.
 *
 * @param env - Process environment supplying `$APPIMAGE`; replaced in tests.
 * @returns A driver that refuses any target the AppImage runtime did not name.
 */
export function createAppImageDriver(env: NodeJS.ProcessEnv = process.env): UpdateInstallerDriver {
  return {
    kind: 'appimage',
    async install(plan) {
      const targetPath = appImageTarget(env)
      // A .deb or a development checkout has no writable image to replace;
      // the package manager owns those installs.
      if (targetPath === undefined) throw new UpdateInstallError('unsupported', 'APPIMAGE')
      await installAppImage({
        artifactPath: plan.artifactPath,
        targetPath,
        sha256: plan.sha256,
        size: plan.size,
      })
    },
  }
}
