/**
 * Exclusive lease on the state a Desktop carrier owns.
 *
 * Two Desktop carriers can be installed side by side — the Tauri shell and the
 * Electron application — and they keep their profile directories apart. They
 * still share the state under the harness home: the session store, the
 * settings document, the credential store. Two Hosts writing those at once
 * corrupt what the user has, and each shell only enforces single-instance
 * startup for *itself*, so a copy of the other carrier would start happily.
 *
 * The lease is what the harness home carries: one file, named by the owning
 * process, taken before the Host boots and released when it stops. A lease
 * whose owner is gone is stale and may be taken over, so a carrier that was
 * killed does not lock the state forever.
 *
 * The lease covers the desktop carriers only. The other Hosts — the web, the
 * headless, the SDK — are separate programs with their own profiles and are
 * not stopped by a desktop application being open.
 *
 * @module @deepseek-ai/dsh-desktop-host/desktop-lease
 */

import { closeSync, fsyncSync, lstatSync, mkdirSync, openSync, readFileSync, unlinkSync, writeSync } from 'node:fs'
import { join } from 'node:path'

/** Name of the lease file inside the harness home. */
export const DESKTOP_LEASE_FILENAME = 'desktop-host.lease'

/** State another Desktop carrier is already using. */
export class DesktopLeaseError extends Error {
  /** The harness home that is already leased. */
  readonly home: string
  /** Process id of the live owner, when it could be read. */
  readonly ownerPid: number | undefined

  /**
   * @param home - The harness home that is already leased.
   * @param ownerPid - Process id of the live owner, when it could be read.
   */
  constructor(home: string, ownerPid?: number) {
    super(ownerPid === undefined
      ? `desktop: ${home} is in use by another Desktop application`
      : `desktop: ${home} is in use by another Desktop application (pid ${String(ownerPid)})`)
    this.name = 'DesktopLeaseError'
    this.home = home
    this.ownerPid = ownerPid
  }
}

/** A held lease, released when the owning process stops. */
export interface DesktopLease {
  /** The file the lease lives in. */
  readonly path: string
  /** Release the lease; a lease already taken over by another process is left alone. */
  release(): void
}

/** Whether a process is still running. */
function isAlive(pid: number): boolean {
  try {
    process.kill(pid, 0)
    return true
  } catch (error) {
    // ESRCH: no such process. EINVAL: the id cannot name one. Either way the
    // owner is gone, and its lease is stale.
    const code = (error as NodeJS.ErrnoException).code
    return code !== 'ESRCH' && code !== 'EINVAL'
  }
}

/** The pid recorded in a lease file, when it holds a usable one. */
function readOwner(path: string): number | undefined {
  let content: string
  try {
    content = readFileSync(path, 'utf8')
  } catch {
    return undefined
  }
  const owner = Number.parseInt(content.trim(), 10)
  return Number.isSafeInteger(owner) && owner > 0 ? owner : undefined
}

/**
 * Take the lease on the state a Desktop carrier owns.
 *
 * The lease file is created exclusively, so two processes racing for the same
 * state cannot both win. A lease whose owner no longer runs is stale: it is
 * removed and taken over, because a carrier that was killed must not keep the
 * state locked. After taking over, the file is read back, so a process that
 * lost the race releases the lease instead of holding a file it does not own.
 *
 * @param home - Harness home whose desktop state is leased.
 * @returns The held lease.
 * @throws {DesktopLeaseError} when another live process owns the state.
 */
export function acquireDesktopLease(home: string): DesktopLease {
  const path = join(home, DESKTOP_LEASE_FILENAME)
  // A first run has no harness home yet; the carrier creates it.
  mkdirSync(home, { recursive: true, mode: 0o700 })
  let descriptor: number
  try {
    descriptor = openSync(path, 'wx', 0o600)
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'EEXIST') throw error
    const owner = readOwner(path)
    if (owner !== undefined && isAlive(owner)) throw new DesktopLeaseError(home, owner)
    try {
      const holder = lstatSync(path)
      if (!holder.isFile()) throw new DesktopLeaseError(home)
      unlinkSync(path)
    } catch (unlinkError) {
      if ((unlinkError as NodeJS.ErrnoException).code !== 'ENOENT') throw unlinkError
    }
    descriptor = openSync(path, 'wx', 0o600)
  }

  writeSync(descriptor, `${String(process.pid)}\n`)
  fsyncSync(descriptor)
  closeSync(descriptor)

  const owner = readOwner(path)
  if (owner !== process.pid) {
    // Another process took the file between the exclusive create and the
    // write; it owns the state now.
    throw new DesktopLeaseError(home, owner)
  }

  let released = false
  return {
    path,
    release(): void {
      if (released) return
      released = true
      // Only remove a lease this process still owns: a takeover that happened
      // after this process died must keep its own file.
      if (readOwner(path) !== process.pid) return
      try {
        unlinkSync(path)
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error
      }
    },
  }
}
