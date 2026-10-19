/**
 * Desktop bridge over a real WebServer: the bridge registers one HTTP route on
 * the booted server, so the transport, status codes, and body limits here are
 * the production ones. The connection service is a controllable stub — its real
 * provider is the browser composition — and the filesystem is real, because
 * the device-preference round trip is the behavior under test.
 */

import { mkdtemp, readFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import WebServer from '@deepseek-ai/dsh-host-webserver'
import type { IndexInjection } from '@deepseek-ai/dsh-host-webserver'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { BRIDGE_PATH, KEYBINDINGS_FILE_NAME, installDesktopBridge } from '../src/desktop-bridge.ts'

let context: Context | undefined
let home: string | undefined
let dispose: (() => void) | undefined
/** Answer the connection stub gives every bridge request until a test changes it. */
const trust: { rejection: 401 | 403 | undefined } = { rejection: undefined }

const DEFINITIONS = [{ id: 'app.palette', defaults: {} }] as const

beforeEach(async () => {
  home = await mkdtemp(join(tmpdir(), 'dsh-desktop-bridge-'))
  context = new Context()
  context.provide('connection', { requestRejection: () => trust.rejection } as never)
  await context.plugin(WebServer, { host: '127.0.0.1', port: 0 })
  dispose = installDesktopBridge(context, { home: homeDir(), platform: 'linux' })
})

afterEach(async () => {
  dispose?.()
  dispose = undefined
  await context?.fiber.dispose()
  context = undefined
  trust.rejection = undefined
  if (home !== undefined) await rm(home, { recursive: true, force: true })
  home = undefined
})

/** The device directory created for this spec run. */
function homeDir(): string {
  if (home === undefined) throw new Error('spec: home not created')
  return home
}

/** Base URL of the bridge's booted server. */
function base(): string {
  if (context === undefined) throw new Error('spec: server not booted')
  return `http://127.0.0.1:${String(context.webServer.port)}`
}

/** Send one bridge request with a JSON body. */
async function post(path: string, body?: unknown): Promise<Response> {
  return fetch(`${base()}${BRIDGE_PATH}${path}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(body ?? null),
  })
}

describe('desktop bridge', () => {
  it.each([401, 403] as const)('rejects a request refused by the connection fence with %i', async (rejection) => {
    trust.rejection = rejection
    const response = await post('/device-info')

    expect(response.status).toBe(rejection)
    expect(await response.text()).toBe(rejection === 401 ? 'unauthorized' : 'forbidden')
  })

  it('answers only POST', async () => {
    const response = await fetch(`${base()}${BRIDGE_PATH}/device-info`)

    expect(response.status).toBe(405)
    expect(response.headers.get('allow')).toBe('POST')
  })

  it('reports the local machine description', async () => {
    const response = await post('/device-info')
    const body = await response.json() as { value: string }

    expect(response.status).toBe(200)
    expect(body.value).toContain('platform=linux')
    expect(body.value).toContain(`app_arch=${process.arch}`)
    expect(body.value).not.toContain('undefined')
  })

  it('stores shortcut preferences in the device document and reads them back', async () => {
    const first = await post('/shortcuts-get', { definitions: DEFINITIONS })
    const initial = await first.json() as { revision: string; document: unknown; status: string }
    expect(initial.status).toBe('ready')

    const saved = await post('/shortcuts-edit', {
      edit: { type: 'set', id: 'app.palette', binding: { code: 'KeyK', modifiers: ['control'] } },
      revision: initial.revision,
    })
    expect(saved.status).toBe(200)
    const result = await saved.json() as { status: string }
    expect(result.status).toBe('saved')

    const document = await readFile(join(homeDir(), KEYBINDINGS_FILE_NAME), 'utf8')
    expect(document).toContain('KeyK')

    // A fresh install over the same home is what the next application launch does.
    dispose?.()
    dispose = installDesktopBridge(context as Context, { home: homeDir(), platform: 'linux' })
    const reread: unknown = await (await post('/shortcuts-get', { definitions: DEFINITIONS })).json()
    expect(JSON.stringify(reread)).toContain('KeyK')
  })

  it('reports the update phase and accepts the open request', async () => {
    expect(await (await post('/updates-status')).json()).toEqual({ phase: 'idle' })
    expect((await post('/updates-open')).status).toBe(204)
  })

  it('accepts the recording and close-window requests', async () => {
    expect((await post('/shortcuts-recording', { active: true })).status).toBe(204)
    expect((await post('/shortcuts-close-window', { revision: 'r' })).status).toBe(204)
  })

  it.each([
    { name: 'an unknown method', path: '/absent', body: null, status: 404 },
    { name: 'a malformed body', path: '/device-info', body: 'not-json', status: 400 },
    { name: 'definitions that are not an array', path: '/shortcuts-get', body: { definitions: 1 }, status: 400 },
    { name: 'an edit without a revision', path: '/shortcuts-edit', body: { edit: { type: 'reset-all' } }, status: 400 },
  ])('refuses $name', async ({ path, body, status }) => {
    const response = await fetch(`${base()}${BRIDGE_PATH}${path}`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: typeof body === 'string' ? body : JSON.stringify(body),
    })

    expect(response.status).toBe(status)
  })

  it('refuses a body past the ceiling', async () => {
    const response = await post('/shortcuts-get', { definitions: 'x'.repeat(2 * 1024 * 1024) })

    expect(response.status).toBe(413)
  })

  it('contributes the bootstrap row that installs window.dshDesktop', () => {
    const table: IndexInjection[] = []
    context?.emit('webserver/index-inject', table)

    expect(table).toHaveLength(1)
    const row = table[0]
    expect(row?.kind).toBe('script')
    if (row?.kind !== 'script') throw new Error('spec: expected a script row')
    expect(row.placement).toBe('head')
    expect(row.text).toContain('globalThis.dshDesktop')
    expect(row.text).toContain('protocolVersion: 1')
    expect(row.text).toContain('dataset.platform = "linux"')
    expect(row.text).not.toContain('</script')
  })

  it('removes its route and its injection row when disposed', async () => {
    dispose?.()
    dispose = undefined
    const table: IndexInjection[] = []
    context?.emit('webserver/index-inject', table)

    expect(table).toEqual([])
    // The server keeps running: only the bridge's own route is gone.
    expect((await fetch(`${base()}${BRIDGE_PATH}/device-info`, { method: 'POST' })).status).toBe(404)
  })
})
