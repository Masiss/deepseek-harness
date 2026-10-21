import { mkdtempSync, readFileSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'
import { acquireDesktopLease, DESKTOP_LEASE_FILENAME, DesktopLeaseError } from '../src/desktop-lease.ts'

/** A harness home that no other test shares. */
function home(): string {
  return mkdtempSync(join(tmpdir(), 'dsh-desktop-lease-'))
}

describe('desktop lease', () => {
  it('records the owning process and releases the file', () => {
    const dir = home()
    const lease = acquireDesktopLease(dir)
    expect(readFileSync(lease.path, 'utf8').trim()).toBe(String(process.pid))
    lease.release()
    expect(() => readFileSync(lease.path, 'utf8')).toThrow()
  })

  it('refuses state another live process owns', () => {
    const dir = home()
    // This process is alive, so a lease naming it blocks every other holder.
    writeFileSync(join(dir, DESKTOP_LEASE_FILENAME), `${String(process.pid)}\n`)
    expect(() => acquireDesktopLease(dir)).toThrow(DesktopLeaseError)
  })

  it('names the owner that holds the state', () => {
    const dir = home()
    writeFileSync(join(dir, DESKTOP_LEASE_FILENAME), `${String(process.pid)}\n`)
    try {
      acquireDesktopLease(dir)
      expect.unreachable('the lease should have been refused')
    } catch (error) {
      expect(error).toBeInstanceOf(DesktopLeaseError)
      expect((error as DesktopLeaseError).ownerPid).toBe(process.pid)
      expect((error as DesktopLeaseError).message).toContain(String(process.pid))
    }
  })

  it('takes over a lease whose owner is gone', () => {
    const dir = home()
    // A pid that cannot be running: the lease is stale, not held.
    writeFileSync(join(dir, DESKTOP_LEASE_FILENAME), '2147483646\n')
    const lease = acquireDesktopLease(dir)
    expect(readFileSync(lease.path, 'utf8').trim()).toBe(String(process.pid))
    lease.release()
  })

  it('leaves a lease another process took over alone', () => {
    const dir = home()
    const lease = acquireDesktopLease(dir)
    // A takeover replaces the file; releasing must not remove the new owner's
    // lease.
    writeFileSync(lease.path, `${String(process.pid + 1)}\n`)
    lease.release()
    expect(readFileSync(lease.path, 'utf8').trim()).toBe(String(process.pid + 1))
  })

  it('creates the harness home when the carrier is the first', () => {
    const dir = join(home(), 'nested', 'home')
    const lease = acquireDesktopLease(dir)
    expect(readFileSync(lease.path, 'utf8').trim()).toBe(String(process.pid))
    lease.release()
  })

  it('releases twice without complaint', () => {
    const dir = home()
    const lease = acquireDesktopLease(dir)
    lease.release()
    expect(() => { lease.release() }).not.toThrow()
  })
})
