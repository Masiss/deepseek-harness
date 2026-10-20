/**
 * One version-bound Desktop update: check a published manifest, download the
 * artifact for this exact target, verify it, then hand it to a driver.
 *
 * The policy is the one the Electron coordinator applies: a check never
 * downloads, a download needs the version the user was shown, an install needs
 * a prepared artifact and a separate authorization, and a target that is not
 * newer than the installed version is not an update. Failures are classified,
 * not passed through: artifact URLs, feed bodies, and error text stay in the
 * journal-free state the UI consumes.
 *
 * @module @deepseek-ai/dsh-desktop-host/update/service
 */

import { createHash } from 'node:crypto'
import { mkdir, open, rm } from 'node:fs/promises'
import { join } from 'node:path'
import { gt, valid } from 'semver'
import type { UpdateInstallerDriver, UpdateInstallPlan } from './appimage.ts'
import { DesktopUpdateJournal } from './journal.ts'
import {
  parseUpdateManifest,
  selectUpdateArtifact,
  UpdateManifestError,
  type UpdateArtifact,
  type UpdateTarget,
} from './manifest.ts'

/** Phases the settings UI renders. */
export type DesktopUpdatePhase = 'idle' | 'checking' | 'available' | 'downloading'
  | 'verifying' | 'installing' | 'ready' | 'error'

/** Classified failure copy selected by the client locale. */
export type DesktopUpdateFailureKind = 'check' | 'check-network' | 'download' | 'download-network'
  | 'install' | 'install-network' | 'tasks-changed' | 'tasks-unavailable'

/** The carrier-owned status the client may render. */
export interface DesktopUpdatePresentation {
  /** Current stage of the attempt. */
  readonly phase: DesktopUpdatePhase
  /** Target version, once a manifest named one. */
  readonly version?: string
  /** Whole percent while downloading. */
  readonly percent?: number
  /** Fixed failure class, only on `error`. */
  readonly failure?: DesktopUpdateFailureKind
}

/** Admission answer from the shell: install, wait for tasks, or refuse. */
export type DesktopUpdateAdmission = 'proceed' | 'tasks-active' | 'unavailable'

/** Construction inputs; every deployment-varying choice is explicit. */
export interface DesktopUpdateServiceOptions {
  /** Manifest URL the shell was built to trust. */
  readonly feed: string
  /** Installed version, owned by the shell that launched this Host. */
  readonly currentVersion: string
  /** Target artifact this shell can install. */
  readonly target: UpdateTarget
  /** Directory holding staged artifacts and the journal. */
  readonly directory: string
  /** Driver that replaces the running installation. */
  readonly driver: UpdateInstallerDriver
  /** Feed channel; `latest` refuses prereleases, as the Electron policy does. */
  readonly channel?: 'nightly' | 'latest'
  /** Task admission and shutdown authorization, performed immediately before an install. */
  readonly beforeInstall?: () => Promise<DesktopUpdateAdmission>
  /** Fetch implementation, replaced at the network test boundary. */
  readonly fetch?: (url: string, init: RequestInit) => Promise<Response>
  /** Feed request deadline in milliseconds. */
  readonly timeoutMs?: number
  /** Whole-artifact deadline in milliseconds; downloads are much larger than the manifest. */
  readonly artifactTimeoutMs?: number
}

/** Largest accepted manifest; the document is a handful of small records. */
const MAX_MANIFEST_BYTES = 64 * 1024

/** Path component safe for a semver string. */
function versionPath(version: string): string {
  return version.replace(/[^0-9A-Za-z.+-]/gu, '_')
}

/** Read at most `limit` bytes and parse them as JSON. */
async function readBoundedJson(response: Response, limit: number): Promise<unknown> {
  const body = response.body
  if (body === null) throw new Error('desktop update: feed returned no body')
  const reader = body.getReader()
  const chunks: Uint8Array[] = []
  let size = 0
  for (;;) {
    const { done, value } = await reader.read()
    if (done) break
    size += value.byteLength
    if (size > limit) {
      await reader.cancel()
      throw new Error('desktop update: feed document is too large')
    }
    chunks.push(value)
  }
  return JSON.parse(Buffer.concat(chunks).toString('utf8'))
}

/** Whether a rejected request looked like transport rather than an answer. */
function isTransportError(error: unknown): boolean {
  return error instanceof TypeError || (error instanceof Error && error.name === 'TimeoutError')
    || (error instanceof Error && error.name === 'AbortError')
}

/** Owns one update attempt at a time and publishes every observable change. */
export class DesktopUpdateService {
  private current: DesktopUpdatePresentation = { phase: 'idle' }
  private candidate: { readonly version: string; readonly artifact: UpdateArtifact } | undefined
  private prepared: UpdateInstallPlan | undefined
  private checkOperation: Promise<DesktopUpdatePresentation> | undefined
  private downloadOperation: Promise<DesktopUpdatePresentation> | undefined
  private installOperation: Promise<DesktopUpdatePresentation> | undefined
  private disposed = false
  private readonly listeners = new Set<(state: DesktopUpdatePresentation) => void>()
  private readonly journal: DesktopUpdateJournal
  private readonly fetchImpl: (url: string, init: RequestInit) => Promise<Response>
  private readonly timeoutMs: number
  private readonly artifactTimeoutMs: number
  private readonly channel: 'nightly' | 'latest'

  /**
   * @param options - Feed, installed version, target, staging directory, and driver.
   */
  constructor(private readonly options: DesktopUpdateServiceOptions) {
    this.fetchImpl = options.fetch ?? ((url, init) => fetch(url, init))
    this.timeoutMs = options.timeoutMs ?? 15_000
    this.artifactTimeoutMs = options.artifactTimeoutMs ?? 600_000
    this.channel = options.channel ?? 'nightly'
    this.journal = new DesktopUpdateJournal(join(options.directory, 'journal'), options.currentVersion)
  }

  /** Latest published status; safe to serialize to the client. */
  get presentation(): DesktopUpdatePresentation {
    return this.current
  }

  /**
   * Observe status changes.
   * @param listener - Called after every published change.
   * @returns Disposer removing this listener.
   */
  subscribe(listener: (state: DesktopUpdatePresentation) => void): () => void {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }

  /**
   * Compare the published manifest with the installed version without downloading.
   *
   * @param manual - Whether a failure must stay visible in the status indicator.
   * @returns The resulting status, including a silent automatic failure.
   */
  async check(manual = false): Promise<DesktopUpdatePresentation> {
    this.assertLive()
    if (this.downloadOperation !== undefined || this.installOperation !== undefined || this.prepared !== undefined) {
      return this.current
    }
    if (!manual && this.current.phase === 'error' && this.current.failure === 'download') return this.current
    this.checkOperation ??= this.doCheck(manual).finally(() => { this.checkOperation = undefined })
    return this.checkOperation
  }

  /**
   * Download the version the user was shown.
   *
   * @param version - Exact version from the confirmation, never a URL.
   * @returns `ready` with a verified artifact, or a classified failure.
   */
  async download(version: string): Promise<DesktopUpdatePresentation> {
    this.assertLive()
    if (this.prepared !== undefined || this.installOperation !== undefined) return this.current
    this.downloadOperation ??= this.doDownload(version).finally(() => { this.downloadOperation = undefined })
    return this.downloadOperation
  }

  /**
   * Install the prepared artifact after separate confirmation.
   *
   * @param version - Exact version from the confirmation.
   * @returns `installing` once the driver replaced the installation, or a classified failure.
   */
  async install(version: string): Promise<DesktopUpdatePresentation> {
    this.assertLive()
    const plan = this.prepared
    if (plan === undefined || plan.version !== version || this.downloadOperation !== undefined) {
      throw new Error('desktop update: confirmed target is not prepared')
    }
    this.installOperation ??= this.doInstall(plan).finally(() => { this.installOperation = undefined })
    return this.installOperation
  }

  /** Drop listeners; pending operations finish without publishing. */
  dispose(): void {
    this.disposed = true
    this.listeners.clear()
  }

  private assertLive(): void {
    if (this.disposed) throw new Error('desktop update: service is disposed')
  }

  private publish(state: DesktopUpdatePresentation): DesktopUpdatePresentation {
    if (this.disposed) return state
    // Progress and the explicit verify step can name the same state; one
    // notification per observable change keeps SSE and the UI honest.
    if (state.phase === this.current.phase && state.version === this.current.version
      && state.percent === this.current.percent && state.failure === this.current.failure) return state
    this.current = state
    for (const listener of this.listeners) listener(state)
    return state
  }

  private fail(failure: DesktopUpdateFailureKind, version?: string): DesktopUpdatePresentation {
    return { phase: 'error', failure, ...(version === undefined ? {} : { version }) }
  }

  private async doCheck(manual: boolean): Promise<DesktopUpdatePresentation> {
    if (manual) this.publish({ phase: 'checking' })
    this.journal.milestone('check')
    let version: string | undefined
    try {
      const response = await this.fetchImpl(this.options.feed, {
        headers: { accept: 'application/json' },
        signal: AbortSignal.timeout(this.timeoutMs),
      })
      if (!response.ok) throw new Error(`desktop update: feed answered ${String(response.status)}`)
      const manifest = parseUpdateManifest(await readBoundedJson(response, MAX_MANIFEST_BYTES))
      version = manifest.version
      if (valid(manifest.version) === null) throw new Error('desktop update: feed version is invalid')
      if (this.channel === 'latest' && manifest.version.includes('-')) {
        throw new Error('desktop update: latest feed contains a prerelease')
      }
      const artifact = selectUpdateArtifact(manifest, this.options.target)
      const newer = gt(manifest.version, this.options.currentVersion)
      this.candidate = newer ? { version: manifest.version, artifact } : undefined
      if (!newer) return this.publish({ phase: 'idle' })
      this.journal.milestone('available', { targetVersion: manifest.version })
      return this.publish({ phase: 'available', version: manifest.version })
    } catch (error) {
      this.candidate = undefined
      const failure: DesktopUpdateFailureKind = error instanceof UpdateManifestError
        ? 'check'
        : isTransportError(error) ? 'check-network' : 'check'
      this.journal.milestone('failed', { failure, ...(version === undefined ? {} : { targetVersion: version }) })
      const state = this.fail(failure, version)
      // An automatic check stays silent, as in the Electron coordinator; a
      // manual one must tell the user why it answered nothing.
      return manual ? this.publish(state) : state
    }
  }

  private async doDownload(version: string): Promise<DesktopUpdatePresentation> {
    const candidate = this.candidate
    let artifactPath: string | undefined
    try {
      if (candidate === undefined) throw new Error('desktop update: no checked update is available')
      if (candidate.version !== version) throw new Error('desktop update: download confirmation is stale')
      this.publish({ phase: 'downloading', version, percent: 0 })
      this.journal.milestone('download', { targetVersion: version, percent: 0 })

      artifactPath = join(this.options.directory, `${versionPath(version)}-${this.options.target}.artifact`)
      await mkdir(this.options.directory, { recursive: true, mode: 0o700 })
      const response = await this.fetchImpl(candidate.artifact.url, {
        signal: AbortSignal.timeout(this.artifactTimeoutMs),
      })
      if (!response.ok || response.body === null) throw new Error('desktop update: artifact answered without bytes')

      const hash = createHash('sha256')
      let written = 0
      let announced = 0
      const handle = await open(artifactPath, 'w', 0o600)
      try {
        const reader = response.body.getReader()
        for (;;) {
          const { done, value } = await reader.read()
          if (done) break
          written += value.byteLength
          if (written > candidate.artifact.size) {
            await reader.cancel()
            throw new Error('desktop update: artifact exceeds its published size')
          }
          hash.update(value)
          await handle.write(value)
          const percent = Math.min(100, Math.floor((written / candidate.artifact.size) * 100))
          if (percent !== announced) {
            announced = percent
            this.publish({ phase: percent >= 100 ? 'verifying' : 'downloading', version, percent })
          }
        }
        await handle.sync()
      } finally {
        await handle.close()
      }

      this.publish({ phase: 'verifying', version, percent: 100 })
      this.journal.milestone('verify', { targetVersion: version })
      if (written !== candidate.artifact.size) throw new Error('desktop update: artifact size does not match the manifest')
      if (hash.digest('hex') !== candidate.artifact.sha256) {
        throw new Error('desktop update: artifact digest does not match the manifest')
      }
      this.prepared = {
        version,
        artifactPath,
        sha256: candidate.artifact.sha256,
        size: candidate.artifact.size,
      }
      this.journal.milestone('ready', { targetVersion: version })
      return this.publish({ phase: 'ready', version })
    } catch (error) {
      this.prepared = undefined
      if (artifactPath !== undefined) await rm(artifactPath, { force: true })
      const failure: DesktopUpdateFailureKind = isTransportError(error) ? 'download-network' : 'download'
      this.journal.milestone('failed', { failure, targetVersion: version })
      return this.publish(this.fail(failure, version))
    }
  }

  private async doInstall(plan: UpdateInstallPlan): Promise<DesktopUpdatePresentation> {
    this.publish({ phase: 'installing', version: plan.version })
    this.journal.milestone('install', { targetVersion: plan.version })
    const admission = await this.options.beforeInstall?.() ?? 'proceed'
    if (admission === 'tasks-active') {
      return this.publish({ phase: 'ready', version: plan.version })
    }
    if (admission === 'unavailable') {
      this.journal.milestone('failed', { failure: 'tasks-unavailable', targetVersion: plan.version })
      return this.publish(this.fail('tasks-unavailable', plan.version))
    }
    try {
      await this.options.driver.install(plan)
      this.journal.milestone('installed', { targetVersion: plan.version })
      // The shell restarts the application after this answer; until it does,
      // the published phase stays `installing`.
      return this.current
    } catch {
      const failure: DesktopUpdateFailureKind = 'install'
      this.journal.milestone('failed', { failure, targetVersion: plan.version })
      // A failed replacement leaves the running image intact but the staged
      // artifact unusable, so the attempt ends here instead of retrying.
      this.prepared = undefined
      await rm(plan.artifactPath, { force: true })
      return this.publish(this.fail(failure, plan.version))
    }
  }
}
