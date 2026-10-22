/**
 * On-demand Office component.
 *
 * The Desktop bundle ships the Host, the dsh runtime, and a profile. Office
 * authoring needs a Python payload and the Office skill assets, which a user
 * who never opens a document should not download and should not pay for on
 * disk. This service fetches that archive from the release feed, verifies its
 * digest, extracts it below the Harness home, and proves the interpreter still
 * imports every module the Office skills document before publishing it.
 *
 * The payload is composed as a second runtime: `runtime.json` is copied from
 * the bundled primary runtime and Node and pnpm are linked to it, so the
 * `desktop-office` plugin sees the same contract it sees in a full bundle.
 *
 * @module @deepseek-ai/dsh-desktop-host/components/office
 */

import { createHash } from 'node:crypto'
import { execFile } from 'node:child_process'
import { readFileSync, readdirSync, statSync } from 'node:fs'
import { copyFile, mkdir, open, rename, rm, stat, symlink } from 'node:fs/promises'
import { join, resolve, sep } from 'node:path'
import { promisify } from 'node:util'
import { parseComponentsManifest, type DesktopComponentArtifact } from './manifest.ts'
import { extractZstdTar } from './tar.ts'

const run = promisify(execFile)

/** Stages the client renders. */
export type DesktopComponentPhase = 'idle' | 'downloading' | 'verifying' | 'installing' | 'ready' | 'error'

/** Fixed failure classes the client localizes. */
export type DesktopComponentFailureKind = 'feed' | 'feed-network' | 'download' | 'download-network'
  | 'integrity' | 'archive' | 'python' | 'install' | 'unavailable'

/** The carrier-owned status the client may render. */
export interface DesktopOfficeStatus {
  /** Component this status describes. */
  readonly id: 'office'
  /** Whether a complete installation exists on disk. */
  readonly installed: boolean
  /** Whether the running Host composed the Office plugin from it. */
  readonly active: boolean
  /** Current stage. */
  readonly phase: DesktopComponentPhase
  /** Installed or target version, once known. */
  readonly version?: string
  /** Whole percent while downloading. */
  readonly percent?: number
  /** Fixed failure class, only on `error`. */
  readonly failure?: DesktopComponentFailureKind
}

/** Construction inputs; every deployment-varying choice is explicit. */
export interface DesktopOfficeComponentOptions {
  /** Directory URL serving `components.json` and the archives. */
  readonly feed: string
  /** Root below the Harness home that holds installed versions. */
  readonly directory: string
  /** Bundled primary runtime supplying `runtime.json`, Node, and pnpm. */
  readonly primaryRuntime: string
  /**
   * PEM public key the component manifest's signature is verified against.
   *
   * The pinned update-feed key is the trust anchor; another key is only for
   * a caller that checks a document against a key of its own.
   */
  readonly publicKey?: string
  /** Fetch implementation, replaced at the network test boundary. */
  readonly fetch?: (url: string, init: RequestInit) => Promise<Response>
  /** Manifest request deadline in milliseconds. */
  readonly timeoutMs?: number
  /** Archive deadline in milliseconds; the payload is much larger than the manifest. */
  readonly artifactTimeoutMs?: number
}

/** Largest accepted manifest. */
const MAX_MANIFEST_BYTES = 64 * 1024

/** Modules every Office skill imports; the archive must resolve all of them. */
const REQUIRED_IMPORTS = 'import numpy, pandas, openpyxl, docx, pptx, PIL, lxml\n'

/** Platform-specific Python launcher and search path in a component runtime. */
export function officePythonRuntime(runtime: string, platform: NodeJS.Platform = process.platform): {
  readonly executable: string
  readonly path: string
} {
  if (platform === 'win32') {
    const bin = `${runtime}\\dependencies\\python\\bin`
    return { executable: `${bin}\\python.exe`, path: `${bin};${process.env.PATH ?? ''}` }
  }
  const bin = join(runtime, 'dependencies', 'python', 'bin')
  return { executable: join(bin, 'python3'), path: '/usr/bin:/bin' }
}

/** One classified install failure. */
class ComponentFailure extends Error {
  constructor(readonly kind: DesktopComponentFailureKind, cause?: unknown) {
    super(`desktop office component: ${kind}`, cause === undefined ? undefined : { cause })
  }
}

/** Random suffix for staging names. */
function suffix(): string {
  return `${String(process.pid)}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`
}

interface RenameJournal {
  readonly oldVersion?: string | undefined
  readonly version: string
  readonly target: string
  readonly backup: string
  readonly markerTemporary: string
  readonly staging: string
}

/** Persist a small rename record before changing directory entries. */
async function durableWrite(file: string, content: string): Promise<void> {
  const handle = await open(file, 'w')
  try {
    await handle.writeFile(content)
    await handle.sync()
  } finally {
    await handle.close()
  }
}

/** Make a directory-entry rename durable on filesystems that support fsync. */
async function syncDirectory(directory: string): Promise<void> {
  const handle = await open(directory, 'r')
  try {
    await handle.sync()
  } finally {
    await handle.close()
  }
}

/** Read a valid marker version without requiring the referenced tree to exist. */
function markerVersion(directory: string): string | undefined {
  try {
    const value: unknown = JSON.parse(readFileSync(join(directory, 'current.json'), 'utf8'))
    if (typeof value !== 'object' || value === null || !('version' in value)) return undefined
    const version = value.version
    return typeof version === 'string' && /^[\w.+-]+$/u.test(version) ? version : undefined
  } catch {
    return undefined
  }
}

/** Read a trusted, locally-written rename journal. */
function readRenameJournal(file: string): RenameJournal | undefined {
  try {
    const value: unknown = JSON.parse(readFileSync(file, 'utf8'))
    if (typeof value !== 'object' || value === null) return undefined
    if (!('version' in value) || !('target' in value) || !('backup' in value)
      || !('markerTemporary' in value) || !('staging' in value)) return undefined
    if (typeof value.version !== 'string' || typeof value.target !== 'string'
      || typeof value.backup !== 'string' || typeof value.markerTemporary !== 'string'
      || typeof value.staging !== 'string') return undefined
    const oldVersion = 'oldVersion' in value && typeof value.oldVersion === 'string' ? value.oldVersion : undefined
    return {
      oldVersion,
      version: value.version,
      target: value.target,
      backup: value.backup,
      markerTemporary: value.markerTemporary,
      staging: value.staging,
    }
  } catch {
    return undefined
  }
}

/** Read a response body with an upper bound. */
async function boundedText(response: Response, limit: number): Promise<string> {
  if (response.body === null) throw new ComponentFailure('feed')
  const reader = response.body.getReader()
  const chunks: Uint8Array[] = []
  let size = 0
  for (;;) {
    const { done, value } = await reader.read()
    if (done) break
    size += value.byteLength
    if (size > limit) {
      await reader.cancel()
      throw new ComponentFailure('feed')
    }
    chunks.push(value)
  }
  return Buffer.concat(chunks).toString('utf8')
}

/**
 * Download one archive while hashing it.
 *
 * @param url - Absolute artifact URL.
 * @param file - Destination path owned by the caller.
 * @param fetchImpl - Fetch implementation.
 * @param signal - Deadline shared with the request.
 * @param onProgress - Receives bytes received so far.
 * @param limit - Exact byte length the artifact must have.
 * @returns The lowercase SHA-256 of the received bytes.
 */
/** Whether a local write failed because the filesystem could not store it. */
function storageFailure(error: unknown): boolean {
  if (typeof error !== 'object' || error === null || !('code' in error)) return false
  const code = (error as { code?: unknown }).code
  return code === 'ENOSPC' || code === 'EDQUOT' || code === 'EFBIG' || code === 'EROFS'
}

async function download(
  url: string,
  file: string,
  fetchImpl: NonNullable<DesktopOfficeComponentOptions['fetch']>,
  signal: AbortSignal,
  onProgress: (received: number) => void,
  limit: number,
): Promise<string> {
  let response: Response
  try {
    response = await fetchImpl(url, { signal })
  } catch (error) {
    throw new ComponentFailure('download-network', error)
  }
  if (!response.ok || response.body === null) throw new ComponentFailure('download')
  const hash = createHash('sha256')
  const out = await open(file, 'w')
  let received = 0
  try {
    const reader = response.body.getReader()
    for (;;) {
      const { done, value } = await reader.read()
      if (done) break
      hash.update(value)
      received += value.byteLength
      // The artifact cannot be larger than the manifest says: stop before an
      // endless or oversized body fills the disk.
      if (received > limit) throw new ComponentFailure('integrity')
      let offset = 0
      while (offset < value.byteLength) {
        const { bytesWritten } = await out.write(value, offset, value.byteLength - offset)
        offset += bytesWritten
      }
      onProgress(received)
    }
  } catch (error) {
    if (error instanceof ComponentFailure) throw error
    // The transport answered; the local filesystem could not accept the bytes.
    throw new ComponentFailure(storageFailure(error) ? 'download' : 'download-network', error)
  } finally {
    await out.close()
  }
  if (received !== limit) throw new ComponentFailure('integrity')
  return hash.digest('hex')
}

/**
 * Read, verify, and install the Office component on request.
 *
 * Installs are serialized: a second request waits for the running one and
 * receives its status instead of starting a competing extraction.
 */
export class DesktopOfficeComponent {
  private readonly directory: string
  private readonly primaryRuntime: string
  private readonly feed: string
  private readonly publicKey: string | undefined
  private readonly fetchImpl: NonNullable<DesktopOfficeComponentOptions['fetch']>
  private readonly timeoutMs: number
  private readonly artifactTimeoutMs: number
  private status: DesktopOfficeStatus
  private installing: Promise<void> | undefined
  private readonly listeners = new Set<(state: DesktopOfficeStatus) => void>()

  /**
   * @param options - Feed, directories, and network substitutes.
   */
  constructor(options: DesktopOfficeComponentOptions) {
    this.directory = options.directory
    this.primaryRuntime = options.primaryRuntime
    this.feed = options.feed.replace(/\/+$/u, '')
    this.publicKey = options.publicKey
    this.fetchImpl = options.fetch ?? ((url, init) => fetch(url, init))
    this.timeoutMs = options.timeoutMs ?? 30_000
    this.artifactTimeoutMs = options.artifactTimeoutMs ?? 30 * 60_000
    const installed = this.installed()
    this.status = installed === undefined
      ? { id: 'office', installed: false, active: false, phase: 'idle' }
      : { id: 'office', installed: true, active: false, phase: 'ready', version: installed.version }
  }

  /** Latest status; every published change is a new immutable object. */
  get presentation(): DesktopOfficeStatus {
    return this.status
  }

  /**
   * Observe status changes.
   *
   * @param listener - Called after every published change.
   * @returns Disposer removing this listener.
   */
  subscribe(listener: (state: DesktopOfficeStatus) => void): () => void {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }

  /**
   * Remove what a Host killed mid-download, mid-extract, or mid-commit left.
   *
   * `install()` removes its own artifacts when it fails, but a SIGKILL skips
   * those handlers. A committed version is kept only when `current.json`
   * names a complete installation; a version renamed into place before the
   * marker was written is unreachable.
   *
   * @returns Nothing after the unreachable files are gone.
   */
  async recover(): Promise<void> {
    await this.recoverRename()
    let entries: string[]
    try {
      entries = readdirSync(this.directory)
    } catch {
      return
    }
    const current = markerVersion(this.directory)
    let versions: string[] = []
    try {
      versions = readdirSync(join(this.directory, 'versions'))
    } catch {
      // No version directory yet.
    }
    await Promise.all([
      ...entries
        .filter(name => name.startsWith('download-') || name.startsWith('staging-'))
        .map(name => rm(join(this.directory, name), { recursive: true, force: true })),
      ...versions
        .filter(name => name !== current)
        .map(name => rm(join(this.directory, 'versions', name), { recursive: true, force: true })),
    ])
  }

  /** Recover a commit interrupted between its directory-entry renames. */
  private async recoverRename(): Promise<void> {
    const journalFile = join(this.directory, 'rename-journal.json')
    const journal = readRenameJournal(journalFile)
    if (journal === undefined) {
      await rm(journalFile, { force: true }).catch(() => undefined)
      return
    }
    const root = resolve(this.directory) + sep
    const journalPaths = [journal.target, journal.backup, journal.markerTemporary, journal.staging]
    if (journalPaths.some(path => !resolve(path).startsWith(root))) {
      await rm(journalFile, { force: true }).catch(() => undefined)
      return
    }
    const current = markerVersion(this.directory)
    if (current !== journal.version) {
      if (await this.exists(journal.backup)) {
        await rm(journal.target, { recursive: true, force: true })
        await rename(journal.backup, journal.target)
      } else if (journal.oldVersion !== journal.version) {
        await rm(journal.target, { recursive: true, force: true })
      }
    } else {
      await rm(journal.backup, { recursive: true, force: true })
    }
    await Promise.all([
      rm(journal.staging, { recursive: true, force: true }),
      rm(journal.markerTemporary, { force: true }),
      rm(journalFile, { force: true }),
    ])
    await syncDirectory(this.directory).catch(() => undefined)
  }

  private async exists(path: string): Promise<boolean> {
    try {
      await stat(path)
      return true
    } catch {
      return false
    }
  }

  /**
   * Locate a complete installation without trusting the manifest.
   *
   * @returns Version and composed runtime directory, or undefined when absent or incomplete.
   */
  installed(): { readonly version: string; readonly runtime: string } | undefined {
    let version: unknown
    try {
      version = (JSON.parse(readFileSync(join(this.directory, 'current.json'), 'utf8')) as { version?: unknown }).version
    } catch {
      return undefined
    }
    if (typeof version !== 'string' || !/^[\w.+-]+$/u.test(version)) return undefined
    const runtime = join(this.directory, 'versions', version, 'runtime')
    try {
      // A dangling link means the bundle moved; treat it as not installed.
      statSync(join(runtime, 'runtime.json'))
      statSync(officePythonRuntime(runtime).executable)
      statSync(join(runtime, 'dependencies', 'node', 'bin', 'node'))
    } catch {
      return undefined
    }
    return { version, runtime }
  }

  /** Mark the running Host as having composed the Office plugin from the installation. */
  markActive(): void {
    if (this.status.installed) this.publish({ ...this.status, active: true })
  }

  /**
   * Install the component, or wait for the running install.
   *
   * @returns The settled status; failures are published, not thrown.
   */
  async install(): Promise<DesktopOfficeStatus> {
    if (this.installing !== undefined) {
      await this.installing
      return this.status
    }
    const attempt = this.attempt().finally(() => { this.installing = undefined })
    this.installing = attempt
    await attempt
    return this.status
  }

  private publish(next: DesktopOfficeStatus): void {
    this.status = next
    for (const listener of this.listeners) listener(next)
  }

  private async attempt(): Promise<void> {
    const existing = this.installed()
    if (existing !== undefined) {
      this.publish({ id: 'office', installed: true, active: this.status.active, phase: 'ready', version: existing.version })
      return
    }
    await mkdir(this.directory, { recursive: true })
    const staging = join(this.directory, `staging-${suffix()}`)
    const archive = join(this.directory, `download-${suffix()}`)
    try {
      const artifact = await this.readArtifact()
      this.publish({ id: 'office', installed: false, active: false, phase: 'downloading', version: artifact.version, percent: 0 })
      const signal = AbortSignal.timeout(this.artifactTimeoutMs)
      const digest = await download(`${this.feed}/${artifact.file}`, archive, this.fetchImpl, signal, (received) => {
        this.publish({
          id: 'office', installed: false, active: false, phase: 'downloading', version: artifact.version,
          percent: Math.max(0, Math.min(100, Math.floor((received / artifact.bytes) * 100))),
        })
      }, artifact.bytes)
      if (digest !== artifact.sha256) throw new ComponentFailure('integrity')
      this.publish({ id: 'office', installed: false, active: false, phase: 'verifying', version: artifact.version, percent: 100 })
      await mkdir(staging, { recursive: true })
      try {
        await extractZstdTar(archive, staging, `${archive}.tar`)
      } catch (error) {
        throw new ComponentFailure(error instanceof ComponentFailure ? error.kind : 'archive', error)
      }
      await this.compose(staging)
      this.publish({ id: 'office', installed: false, active: false, phase: 'installing', version: artifact.version })
      await this.commit(staging, artifact.version)
      this.publish({ id: 'office', installed: true, active: false, phase: 'ready', version: artifact.version })
    } catch (error) {
      const kind = error instanceof ComponentFailure ? error.kind : 'install'
      console.error(error)
      this.publish({ id: 'office', installed: false, active: false, phase: 'error', failure: kind })
    } finally {
      await rm(staging, { recursive: true, force: true }).catch(() => undefined)
      await rm(archive, { force: true }).catch(() => undefined)
      await rm(`${archive}.tar`, { force: true }).catch(() => undefined)
    }
  }

  private async readArtifact(): Promise<DesktopComponentArtifact> {
    this.assertFeedIsDownloadable()
    let response: Response
    try {
      response = await this.fetchImpl(`${this.feed}/components.json`, { signal: AbortSignal.timeout(this.timeoutMs) })
    } catch (error) {
      throw new ComponentFailure('feed-network', error)
    }
    if (!response.ok) throw new ComponentFailure('feed')
    let manifest
    try {
      manifest = parseComponentsManifest(JSON.parse(await boundedText(response, MAX_MANIFEST_BYTES)), this.publicKey)
    } catch (error) {
      throw new ComponentFailure(error instanceof ComponentFailure ? error.kind : 'feed', error)
    }
    const artifact = manifest.components.find((entry: { id: string }) => entry.id === 'office')
    if (artifact === undefined) throw new ComponentFailure('feed')
    return artifact
  }

  /**
   * Refuse a feed the network layer cannot authenticate.
   *
   * The component archive is executed, so the feed that names it must travel
   * over HTTPS; HTTP is accepted only on the loopback interface, where a local
   * rehearsal cannot be intercepted on the wire.
   */
  private assertFeedIsDownloadable(): void {
    let url: URL
    try {
      url = new URL(this.feed)
    } catch (error) {
      throw new ComponentFailure('feed', error)
    }
    if (url.protocol === 'https:') return
    const loopback = url.protocol === 'http:'
      && (url.hostname === '127.0.0.1' || url.hostname === '::1' || url.hostname === 'localhost')
    if (!loopback) throw new ComponentFailure('feed')
  }

  /**
   * Turn extracted files into a payload the `desktop-office` plugin accepts.
   *
   * @param root - Staging directory holding `runtime/` and `office-skills/`.
   */
  private async compose(root: string): Promise<void> {
    const runtime = join(root, 'runtime')
    await copyFile(join(this.primaryRuntime, 'runtime.json'), join(runtime, 'runtime.json'))
    for (const name of ['node', 'pnpm']) {
      const link = join(runtime, 'dependencies', name)
      await rm(link, { recursive: true, force: true })
      await symlink(join(this.primaryRuntime, 'dependencies', name), link, 'dir')
    }
    const python = officePythonRuntime(runtime)
    const engine = join(runtime, 'node_modules', '@deepseek-ai', 'libreoffice-kit', 'lib', 'cli.js')
    try {
      await stat(python.executable)
      await stat(join(root, 'office-skills'))
      await stat(engine)
    } catch (error) {
      throw new ComponentFailure('archive', error)
    }
    try {
      await run(python.executable, ['-c', REQUIRED_IMPORTS], { timeout: 120_000, env: { PATH: python.path } })
    } catch (error) {
      throw new ComponentFailure('python', error)
    }
    await mkdir(join(this.directory, 'versions'), { recursive: true })
  }

  private async commit(staging: string, version: string): Promise<void> {
    const target = join(this.directory, 'versions', version)
    const backup = join(this.directory, 'versions', `.backup-${suffix()}`)
    const marker = join(this.directory, 'current.json')
    const temporary = `${marker}.${suffix()}`
    const journalFile = join(this.directory, 'rename-journal.json')
    const journal: RenameJournal = {
      oldVersion: markerVersion(this.directory),
      version,
      target,
      backup,
      markerTemporary: temporary,
      staging,
    }
    await durableWrite(journalFile, `${JSON.stringify(journal)}\n`)
    try {
      if (await this.exists(target)) await rename(target, backup)
      await rename(staging, target)
      await syncDirectory(join(this.directory, 'versions'))
      await durableWrite(temporary, `${JSON.stringify({ version }, undefined, 2)}\n`)
      await rename(temporary, marker)
      await syncDirectory(this.directory)
      await rm(backup, { recursive: true, force: true })
      await rm(journalFile, { force: true })
      await syncDirectory(this.directory)
      // Previous versions are unreachable once the marker moved.
      for (const entry of readdirSync(join(this.directory, 'versions'))) {
        if (entry !== version) await rm(join(this.directory, 'versions', entry), { recursive: true, force: true })
      }
    } catch (error) {
      // Keep the journal: the next Host start can choose one complete tree.
      throw error
    }
  }
}
