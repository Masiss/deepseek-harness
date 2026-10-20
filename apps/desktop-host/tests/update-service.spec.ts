import { createHash } from 'node:crypto'
import { createServer, type Server } from 'node:http'
import { mkdtemp, readFile, readdir, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import type { AddressInfo } from 'node:net'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { UpdateInstallPlan } from '../src/update/appimage.ts'
import {
  DesktopUpdateService,
  type DesktopUpdateAdmission,
  type DesktopUpdatePresentation,
} from '../src/update/service.ts'

const ARTIFACT = Buffer.from('a complete AppImage payload')
const DIGEST = createHash('sha256').update(ARTIFACT).digest('hex')

let root: string
let server: Server
let base: string
let manifestBody: unknown
let artifactBody: Buffer
let artifactStatus: number
let installed: UpdateInstallPlan[]
let admission: DesktopUpdateAdmission

beforeEach(async () => {
  root = await mkdtemp(join(tmpdir(), 'dsh-update-service-'))
  installed = []
  admission = 'proceed'
  artifactBody = ARTIFACT
  artifactStatus = 200
  manifestBody = {
    schemaVersion: 1,
    version: '0.3.0',
    artifacts: {
      'linux-x64': { url: 'placeholder', sha256: DIGEST, size: ARTIFACT.byteLength },
    },
  }
  server = createServer((request, response) => {
    if (request.url === '/app.bin') {
      response.statusCode = artifactStatus
      response.end(artifactBody)
      return
    }
    response.setHeader('content-type', 'application/json')
    response.end(JSON.stringify(manifestBody))
  })
  await new Promise<void>((resolve) => { server.listen(0, '127.0.0.1', resolve) })
  base = `http://127.0.0.1:${String((server.address() as AddressInfo).port)}`
})

afterEach(async () => {
  await new Promise<void>((resolve) => { server.close(() => { resolve() }) })
  await rm(root, { recursive: true, force: true })
})

/** Serve the manifest with the artifact URL pointing at this server. */
function serveArtifact(): void {
  manifestBody = {
    schemaVersion: 1,
    version: '0.3.0',
    artifacts: {
      'linux-x64': { url: `${base}/app.bin`, sha256: DIGEST, size: ARTIFACT.byteLength },
    },
  }
}

/** One service bound to the test server. */
function service(overrides: Partial<ConstructorParameters<typeof DesktopUpdateService>[0]> = {}): DesktopUpdateService {
  return new DesktopUpdateService({
    feed: `${base}/manifest.json`,
    currentVersion: '0.2.1',
    target: 'linux-x64',
    directory: join(root, 'updates'),
    driver: { kind: 'test', install: async (plan) => { installed.push(plan) } },
    beforeInstall: async () => admission,
    ...overrides,
  })
}

/** Read every record the service journaled for this run. */
async function journal(): Promise<Array<Record<string, unknown>>> {
  const directory = join(root, 'updates', 'journal')
  const [file] = await readdir(directory)
  if (file === undefined) throw new Error('spec: journal file is missing')
  return (await readFile(join(directory, file), 'utf8')).trim().split('\n')
    .map(line => JSON.parse(line) as Record<string, unknown>)
}

describe('DesktopUpdateService', () => {
  it('checks, downloads, verifies, and installs one version-bound artifact', async () => {
    serveArtifact()
    const updates = service()
    const published: DesktopUpdatePresentation[] = []
    updates.subscribe((state) => { published.push(state) })

    await expect(updates.check(true)).resolves.toEqual({ phase: 'available', version: '0.3.0' })
    await expect(updates.download('0.3.0')).resolves.toEqual({ phase: 'ready', version: '0.3.0' })
    await expect(updates.install('0.3.0')).resolves.toEqual({ phase: 'installing', version: '0.3.0' })

    expect(installed).toHaveLength(1)
    expect(installed[0]).toMatchObject({ version: '0.3.0', sha256: DIGEST, size: ARTIFACT.byteLength })
    expect(await readFile(installed[0]?.artifactPath ?? '', 'utf8')).toBe(ARTIFACT.toString())
    expect(published.map(state => state.phase)).toEqual([
      'checking', 'available', 'downloading', 'verifying', 'ready', 'installing',
    ])

    const records = await journal()
    expect(records.map(record => record.milestone)).toEqual([
      'started', 'check', 'available', 'download', 'verify', 'ready', 'install', 'installed',
    ])
  })

  it('reports nothing for a version that is not newer', async () => {
    serveArtifact()
    manifestBody = { ...manifestBody as object, version: '0.2.1' }
    const updates = service()
    await expect(updates.check(true)).resolves.toEqual({ phase: 'idle' })
    await expect(updates.download('0.2.1')).resolves.toMatchObject({ phase: 'error', failure: 'download' })
    expect(installed).toHaveLength(0)
  })

  it('refuses a release without an artifact for this target', async () => {
    manifestBody = {
      schemaVersion: 1,
      version: '0.3.0',
      artifacts: { 'darwin-arm64': { url: `${base}/app.bin`, sha256: DIGEST, size: ARTIFACT.byteLength } },
    }
    const updates = service()
    await expect(updates.check(true)).resolves.toEqual({ phase: 'error', failure: 'check', version: '0.3.0' })
  })

  it('refuses a prerelease on the latest channel', async () => {
    serveArtifact()
    manifestBody = { ...manifestBody as object, version: '0.3.0-rc.1' }
    const updates = service({ channel: 'latest' })
    await expect(updates.check(true)).resolves.toMatchObject({ phase: 'error', failure: 'check' })
  })

  it('discards an artifact whose bytes do not match the manifest', async () => {
    serveArtifact()
    artifactBody = Buffer.from('tampered payload')
    const updates = service()

    await updates.check(true)
    await expect(updates.download('0.3.0')).resolves.toMatchObject({ phase: 'error', failure: 'download' })
    expect(installed).toHaveLength(0)
    const entries = await readdir(join(root, 'updates'))
    expect(entries.some(entry => entry.endsWith('.artifact'))).toBe(false)
  })

  it('treats a failed artifact response as a download failure', async () => {
    serveArtifact()
    artifactStatus = 404
    const updates = service()
    await updates.check(true)
    await expect(updates.download('0.3.0')).resolves.toMatchObject({ phase: 'error', failure: 'download' })
  })

  it('classifies an unreachable feed as a network failure', async () => {
    const updates = service({ feed: 'http://127.0.0.1:1/manifest.json' })
    await expect(updates.check(true)).resolves.toMatchObject({ phase: 'error', failure: 'check-network' })
  })

  it('keeps an automatic check failure silent', async () => {
    const updates = service({ feed: 'http://127.0.0.1:1/manifest.json' })
    await expect(updates.check(false)).resolves.toMatchObject({ phase: 'error' })
    expect(updates.presentation).toEqual({ phase: 'idle' })
  })

  it('waits for active tasks instead of installing', async () => {
    serveArtifact()
    admission = 'tasks-active'
    const updates = service()
    await updates.check(true)
    await updates.download('0.3.0')

    await expect(updates.install('0.3.0')).resolves.toEqual({ phase: 'ready', version: '0.3.0' })
    expect(installed).toHaveLength(0)
  })

  it('refuses an install when the task lock is unavailable', async () => {
    serveArtifact()
    admission = 'unavailable'
    const updates = service()
    await updates.check(true)
    await updates.download('0.3.0')

    await expect(updates.install('0.3.0')).resolves.toMatchObject({ phase: 'error', failure: 'tasks-unavailable' })
    expect(installed).toHaveLength(0)
  })

  it('refuses a confirmation for a version that was not prepared', async () => {
    serveArtifact()
    const updates = service()
    await expect(updates.install('0.3.0')).rejects.toThrow(/not prepared/u)
  })

  it('reports a driver failure and drops the artifact', async () => {
    serveArtifact()
    const updates = service({
      driver: { kind: 'test', install: async () => { throw new Error('driver refused') } },
    })
    await updates.check(true)
    await updates.download('0.3.0')
    const plan = updates.presentation
    expect(plan.phase).toBe('ready')

    await expect(updates.install('0.3.0')).resolves.toMatchObject({ phase: 'error', failure: 'install' })
  })

  it('runs check, download, install, and relaunch after both confirmations', async () => {
    serveArtifact()
    const prompts: string[] = []
    const restarted = vi.fn()
    const updates = service({
      confirm: async (prompt) => { prompts.push(`${prompt.action}:${prompt.version}`); return true },
      restart: restarted,
    })

    await updates.open()

    expect(prompts).toEqual(['download:0.3.0', 'install:0.3.0'])
    expect(installed).toHaveLength(1)
    expect(restarted).toHaveBeenCalledOnce()
    expect(updates.presentation).toEqual({ phase: 'installing', version: '0.3.0' })
  })

  it('stops before downloading when the user declines', async () => {
    serveArtifact()
    const restarted = vi.fn()
    const updates = service({
      confirm: async prompt => prompt.action !== 'download',
      restart: restarted,
    })

    await updates.open()

    expect(updates.presentation).toEqual({ phase: 'available', version: '0.3.0' })
    expect(installed).toHaveLength(0)
    expect(restarted).not.toHaveBeenCalled()
  })

  it('keeps a downloaded artifact and asks only for the install next time', async () => {
    serveArtifact()
    const asked: string[] = []
    let allowInstall = false
    const updates = service({
      confirm: async (prompt) => {
        asked.push(prompt.action)
        return prompt.action === 'download' || allowInstall
      },
    })

    await updates.open()
    expect(updates.presentation).toEqual({ phase: 'ready', version: '0.3.0' })
    expect(installed).toHaveLength(0)

    // A second request resumes at the install decision; the artifact stays.
    await updates.open()
    expect(asked).toEqual(['download', 'install', 'install'])
    expect(installed).toHaveLength(0)

    allowInstall = true
    await updates.open()
    expect(installed).toHaveLength(1)
  })

  it('does not relaunch when the download fails', async () => {
    serveArtifact()
    artifactBody = Buffer.from('tampered payload')
    const restarted = vi.fn()
    const updates = service({
      confirm: async () => true,
      restart: restarted,
    })

    await updates.open()

    expect(updates.presentation).toMatchObject({ phase: 'error', failure: 'download' })
    expect(installed).toHaveLength(0)
    expect(restarted).not.toHaveBeenCalled()
  })

  it('joins concurrent requests into one flow', async () => {
    serveArtifact()
    const confirm = vi.fn(async () => true)
    const updates = service({ confirm })

    await Promise.all([updates.open(), updates.open(), updates.open()])

    expect(confirm).toHaveBeenCalledTimes(2)
    expect(installed).toHaveLength(1)
  })

  it('stops publishing after disposal', async () => {
    serveArtifact()
    const updates = service()
    const listener = vi.fn()
    updates.subscribe(listener)
    updates.dispose()
    await expect(updates.check(true)).rejects.toThrow(/disposed/u)
    expect(listener).not.toHaveBeenCalled()
  })
})
