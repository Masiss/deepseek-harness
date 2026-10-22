/**
 * Desktop bridge: the `window.dshDesktop` capability surface for shells that
 * are not Electron.
 *
 * Electron exposes the product API from its preload over `ipcRenderer`. A
 * Tauri shell owns no JavaScript runtime of its own, so the capability moves
 * to the Host, which already serves the authenticated page: this module
 * registers `/__dsh-desktop` on the webserver, gates every request through
 * Connection's Host/Origin fence and authenticated Peer, and contributes the
 * `<head>` script row that installs `window.dshDesktop` before any client
 * module reads it.
 *
 * The shortcut coordinator is the same `ShortcutPersistence` the Electron main
 * process runs, so both shells share one device-preference implementation and
 * one `keybindings.json` document shape.
 *
 * The bridge deliberately omits `browser` (no Tauri equivalent to
 * `WebContentsView`) and native menu input. Update presentation is served here
 * when the Host was given an update service; without one the surface reports
 * `idle` and opens nothing, which is what the client treats as "this carrier
 * cannot update".
 *
 * @module @deepseek-ai/dsh-desktop-host/desktop-bridge
 */

import type { IncomingMessage, ServerResponse } from 'node:http'
import { readFile } from 'node:fs/promises'
import { cpus, release, totalmem } from 'node:os'
import { join } from 'node:path'
import type { Context } from '@deepseek-ai/cordis'
import { writeFileAtomic } from '@deepseek-ai/dsh-atomic-write'
import {
  parseShortcutDefinitions,
  parseShortcutEdit,
  ShortcutPersistence,
} from '@deepseek-ai/dsh-client-shortcuts/protocol'
import type {
  ShortcutPlatform,
  ShortcutRevision,
} from '@deepseek-ai/dsh-client-shortcuts/protocol'
import { resolveDshHome } from '@deepseek-ai/dsh-home-paths'
import type {} from '@deepseek-ai/dsh-client-connection'
import type { IndexInjection } from '@deepseek-ai/dsh-host-webserver'
import type { DesktopUpdatePresentation } from './update/service.ts'
import type { DesktopOfficeStatus } from './components/office.ts'

/** Absolute pathname prefix owning every bridge request. */
export const BRIDGE_PATH = '/__dsh-desktop'

/** Device-preference document name inside the DSH home. */
export const KEYBINDINGS_FILE_NAME = 'keybindings.json'

/** Ceiling for one bridge request body; the catalog of shortcut definitions is the largest legitimate payload. */
const MAX_BODY_BYTES = 1024 * 1024

/** Construction inputs for the bridge; both are explicit so a deployment can relocate the device document. */
export interface DesktopBridgeOptions {
  /**
   * Directory holding `keybindings.json`.
   * @defaultValue the resolved DSH home.
   */
  readonly home?: string
  /**
   * Input platform published to the shortcut document.
   * @defaultValue the running platform.
   */
  readonly platform?: ShortcutPlatform
  /**
   * Update status published to the client.
   * @defaultValue absent, which reports `idle` and opens nothing.
   */
  readonly updates?: DesktopUpdateChannel
  /**
   * On-demand Office component published to the client.
   * @defaultValue absent, which reports an idle, uninstalled component.
   */
  readonly components?: DesktopOfficeChannel
}

/** The part of the update service the bridge publishes. */
export interface DesktopUpdateChannel {
  /** Latest published status. */
  readonly presentation: DesktopUpdatePresentation
  /**
   * Observe status changes.
   * @param listener - Called after every published change.
   * @returns Disposer removing this listener.
   */
  subscribe(listener: (state: DesktopUpdatePresentation) => void): () => void
  /**
   * Run the user-requested flow: check, ask, download, ask, install, relaunch.
   * @returns Nothing when the flow settles.
   */
  open(): Promise<void>
}

/**
 * The part of the Office component service the bridge publishes.
 */
export interface DesktopOfficeChannel {
  /** Latest published status. */
  readonly presentation: DesktopOfficeStatus
  /**
   * Observe status changes.
   * @param listener - Called after every published change.
   * @returns Disposer removing this listener.
   */
  subscribe(listener: (state: DesktopOfficeStatus) => void): () => void
  /**
   * Download, verify, and install the component.
   * @returns The settled status; failures are part of it, not exceptions.
   */
  install(): Promise<DesktopOfficeStatus>
  /**
   * Relaunch the application so an installed component becomes active.
   * @returns Resolves once the shell was asked.
   */
  restart?(): Promise<void>
}

/**
 * Install the desktop bridge on a booted Host context.
 *
 * @param ctx - Host context owning the webserver and Connection services.
 * @param options - Explicit home override and input platform.
 * @returns disposer removing the route and the injection row.
 */
export function installDesktopBridge(ctx: Context, options: DesktopBridgeOptions = {}): () => void {
  const home = options.home ?? resolveDshHome()
  const platform = options.platform ?? shortcutPlatform(process.platform)
  const keybindingsPath = join(home, KEYBINDINGS_FILE_NAME)
  const shortcuts = new ShortcutPersistence({
    read: async () => {
      try {
        return await readFile(keybindingsPath, 'utf8')
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code === 'ENOENT') return null
        throw error
      }
    },
    write: raw => writeFileAtomic(keybindingsPath, raw, { mode: 0o600, dirMode: 0o700 }),
  }, 'desktop', platform, false, () => {
    // No page is pushed to: reads and edits answer their own request, and the
    // client applies the snapshot each reply carries.
  })

  const route = ctx.webServer.register({
    kind: 'prefix',
    path: BRIDGE_PATH,
    handler: (req, res) => handle(ctx, shortcuts, options.updates, options.components, req, res),
  })
  const events = options.updates === undefined ? undefined : serveUpdates(ctx, options.updates)
  const componentChannel = options.components
  const componentEvents = componentChannel === undefined
    ? undefined
    : serveEvents(ctx, `${BRIDGE_PATH}/components-events`, () => componentChannel.presentation,
      listener => componentChannel.subscribe(listener))
  const removeInject = ctx.on('webserver/index-inject', (table: IndexInjection[]) => {
    table.push({ kind: 'script', placement: 'head', text: renderShim(platform) })
  })
  return () => {
    route()
    events?.()
    componentEvents?.()
    removeInject()
    shortcuts.dispose()
  }
}

/** Map a Node platform onto the shortcut document's input platform. */
function shortcutPlatform(platform: NodeJS.Platform): ShortcutPlatform {
  if (platform === 'darwin') return 'macos'
  if (platform === 'win32') return 'windows'
  return 'linux'
}

/** Apply the Host/Origin fence and authenticated Peer before any bridge work. */
async function handle(
  ctx: Context,
  shortcuts: ShortcutPersistence,
  updates: DesktopUpdateChannel | undefined,
  components: DesktopOfficeChannel | undefined,
  req: IncomingMessage,
  res: ServerResponse,
): Promise<void> {
  res.setHeader('cache-control', 'no-store')
  const rejection = ctx.connection.requestRejection(req)
  if (rejection !== undefined) {
    res.statusCode = rejection
    res.end(rejection === 401 ? 'unauthorized' : 'forbidden')
    return
  }
  if (req.method !== 'POST') {
    res.statusCode = 405
    res.setHeader('allow', 'POST')
    res.end()
    return
  }
  const path = new URL(req.url ?? '/', 'http://127.0.0.1').pathname.slice(BRIDGE_PATH.length)
  const body = await readBoundedBody(req, res)
  if (body === null) return
  let request: unknown
  try {
    request = JSON.parse(body)
  } catch {
    // A non-JSON body is a caller defect, not a bridge state: answer it as one.
    sendJson(res, 400, { error: 'invalid-json' })
    return
  }
  const answer = await dispatch(shortcuts, updates, components, path, request)
  if (answer === undefined) {
    sendJson(res, 404, { error: 'unknown-method' })
    return
  }
  sendJson(res, answer.status, answer.payload)
}

/** One dispatch outcome: HTTP status plus the JSON payload. */
interface DispatchAnswer {
  readonly status: number
  readonly payload: unknown
}

/** Run one bridge method, or answer `undefined` when the path names none. */
async function dispatch(
  shortcuts: ShortcutPersistence,
  updates: DesktopUpdateChannel | undefined,
  components: DesktopOfficeChannel | undefined,
  path: string,
  request: unknown,
): Promise<DispatchAnswer | undefined> {
  switch (path) {
    case '/device-info':
      return { status: 200, payload: { value: readDeviceInfo() } }
    case '/shortcuts-get': {
      const definitions = field(request, 'definitions')
      if (!Array.isArray(definitions)) return { status: 400, payload: { error: 'invalid-definitions' } }
      let catalog
      try {
        catalog = parseShortcutDefinitions(definitions)
      } catch {
        return { status: 400, payload: { error: 'invalid-definitions' } }
      }
      shortcuts.setDefinitions(catalog)
      return { status: 200, payload: await shortcuts.readCurrent() }
    }
    case '/shortcuts-edit': {
      const revision = field(request, 'revision')
      if (typeof revision !== 'string') return { status: 400, payload: { error: 'invalid-revision' } }
      let edit
      try {
        edit = parseShortcutEdit(field(request, 'edit'))
      } catch {
        return { status: 400, payload: { error: 'invalid-edit' } }
      }
      return {
        status: 200,
        payload: await shortcuts.edit(edit, revision as ShortcutRevision),
      }
    }
    case '/shortcuts-recording':
      // The bridge registers no native accelerator, so a recording session has
      // nothing to suppress.
      return { status: 204, payload: undefined }
    case '/shortcuts-close-window':
      // One window owns the bridge; its configuration cannot be stale elsewhere.
      return { status: 204, payload: undefined }
    case '/updates-status':
      return { status: 200, payload: updates?.presentation ?? { phase: 'idle' } }
    case '/updates-open':
      // The user asked to update. The reply waits for the whole flow so the
      // settings spinner covers it; progress reaches the page over events.
      if (updates !== undefined) await updates.open()
      return { status: 204, payload: undefined }
    case '/components-status':
      return {
        status: 200,
        payload: components?.presentation ?? { id: 'office', installed: false, active: false, phase: 'idle' },
      }
    case '/components-install':
      // Installing downloads and verifies a payload; the reply carries the
      // settled status so the caller need not poll for the outcome.
      if (components === undefined) return { status: 503, payload: { error: 'components-unavailable' } }
      return { status: 200, payload: await components.install() }
    case '/components-restart':
      // The user asked to finish an install now; the shell owns the relaunch.
      if (components?.restart === undefined) return { status: 503, payload: { error: 'components-unavailable' } }
      await components.restart()
      return { status: 204, payload: undefined }
    default:
      return undefined
  }
}

/** Read one field of a parsed request without asserting its type. */
function field(request: unknown, key: string): unknown {
  if (typeof request !== 'object' || request === null) return undefined
  return (request as Record<string, unknown>)[key]
}

/** Write one JSON response with an explicit status. */
function sendJson(res: ServerResponse, status: number, payload: unknown): void {
  if (status === 204) {
    res.statusCode = 204
    res.end()
    return
  }
  const text = JSON.stringify(payload)
  res.statusCode = status
  res.setHeader('content-type', 'application/json; charset=utf-8')
  res.end(text)
}

/**
 * Serve update status as server-sent events.
 *
 * The page cannot poll for readiness without a timer of its own, and the
 * settings row subscribes once; one long-lived response per page is the
 * smallest carrier for that contract.
 *
 * @param ctx - Host context owning the webserver and Connection fence.
 * @param updates - Service whose published states are forwarded.
 * @returns Disposer closing the route and every open response.
 */
function serveUpdates(ctx: Context, updates: DesktopUpdateChannel): () => void {
  return serveEvents(ctx, `${BRIDGE_PATH}/updates-events`, () => updates.presentation,
    listener => updates.subscribe(listener))
}

/**
 * Serve one status stream as server-sent events.
 *
 * @param ctx - Host context owning the webserver and Connection fence.
 * @param path - Absolute route below the bridge prefix.
 * @param current - Latest state, sent on connect.
 * @param subscribe - Status subscription for one client.
 * @returns Disposer closing the route and every open response.
 */
function serveEvents<T>(
  ctx: Context,
  path: string,
  current: () => T,
  subscribe: (listener: (state: T) => void) => () => void,
): () => void {
  const clients = new Set<ServerResponse>()
  const route = ctx.webServer.register({
    kind: 'exact',
    path,
    handler: (req, res) => {
      res.setHeader('cache-control', 'no-store')
      const rejection = ctx.connection.requestRejection(req)
      if (rejection !== undefined) {
        res.statusCode = rejection
        res.end()
        return
      }
      if (req.method !== 'GET') {
        res.statusCode = 405
        res.setHeader('allow', 'GET')
        res.end()
        return
      }
      res.writeHead(200, {
        'content-type': 'text/event-stream',
        connection: 'keep-alive',
        'x-accel-buffering': 'no',
      })
      res.write(`data: ${JSON.stringify(current())}\n\n`)
      clients.add(res)
      // The response owns the connection lifetime; the request completes as
      // soon as its empty body ends, which is not when the page disconnects.
      res.on('close', () => { clients.delete(res) })
    },
  })
  const unsubscribe = subscribe((state) => {
    const record = `data: ${JSON.stringify(state)}\n\n`
    for (const res of clients) res.write(record)
  })
  return () => {
    unsubscribe()
    route()
    for (const res of clients) res.end()
    clients.clear()
  }
}

/** Collect a bounded request body as UTF-8 text; null once the ceiling is passed (stream drained). */
async function readBoundedBody(req: IncomingMessage, res: ServerResponse): Promise<string | null> {
  const chunks: Buffer[] = []
  let size = 0
  // http server streams without setEncoding always yield Buffer chunks.
  for await (const chunk of req as AsyncIterable<Buffer>) {
    size += chunk.byteLength
    if (size > MAX_BODY_BYTES) {
      // Drain the remainder so the refusal is a readable response, not a socket cut.
      req.resume()
      sendJson(res, 413, { error: 'body-too-large' })
      return null
    }
    chunks.push(chunk)
  }
  return Buffer.concat(chunks, size).toString('utf8')
}

/**
 * Local machine description attached to a feedback submission, in the field
 * order the Electron shell publishes: platform, os, app_arch, cpu, memory_gib.
 *
 * @returns `name=value` fields separated by `; `, with unavailable fields omitted.
 */
function readDeviceInfo(): string {
  const fields = [`platform=${process.platform}`]
  collect(fields, 'os', () => release())
  fields.push(`app_arch=${process.arch}`)
  collect(fields, 'cpu', () => cpus()[0]?.model)
  collect(fields, 'memory_gib', () => (totalmem() / 1024 ** 3).toFixed(1))
  return fields.join('; ')
}

/** Append one optional device field, omitting it when its source is empty or fails. */
function collect(fields: string[], name: string, read: () => string | undefined): void {
  let value: string | undefined
  try {
    value = read()
  } catch {
    // A refused source omits only its own field.
    return
  }
  if (value !== undefined && value !== '') fields.push(`${name}=${value}`)
}

/**
 * Render the `<head>` script installing `window.dshDesktop`.
 *
 * @param platform - value published as `<html data-platform>`.
 * @returns classic-script source with no `</script` sequence.
 */
function renderShim(platform: ShortcutPlatform): string {
  return `(() => {
  const base = ${JSON.stringify(BRIDGE_PATH)}
  // The client reads the shortcut document once per command registration without
  // awaiting, so a request per call would open one connection per read and
  // exhaust the host's connection budget. One request at a time keeps the
  // replies ordered — which the revision checks want anyway — and stays inside it.
  let queue = Promise.resolve()
  const post = (method, body) => {
    const send = () => fetch(base + '/' + method, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body === undefined ? null : body),
      credentials: 'same-origin',
    }).then((response) => {
      if (!response.ok) throw new Error('desktop bridge ' + method + ' failed: ' + String(response.status))
      return response.status === 204 ? undefined : response.json()
    })
    const reply = queue.then(send, send)
    queue = reply.then(() => undefined, () => undefined)
    return reply
  }
  const never = () => () => {}
  globalThis.dshDesktop = {
    protocolVersion: 1,
    deviceInfo: () => post('device-info').then((reply) => reply.value),
    keyboard: {
      subscribe: never,
      closeWindow: (revision) => post('shortcuts-close-window', { revision: revision }).then(() => undefined),
    },
    shortcuts: {
      get: (definitions) => post('shortcuts-get', { definitions: definitions }),
      edit: (edit, revision) => post('shortcuts-edit', { edit: edit, revision: revision }),
      subscribe: never,
      recording: (active) => post('shortcuts-recording', { active: active }).then(() => undefined),
    },
    updates: {
      status: () => post('updates-status'),
      open: () => post('updates-open').then(() => undefined),
      subscribe: (listener) => {
        const source = new EventSource(base + '/updates-events')
        source.onmessage = (event) => {
          try { listener(JSON.parse(event.data)) } catch (error) { console.warn('desktop bridge: update event', error) }
        }
        return () => { source.close() }
      },
    },
    components: {
      status: () => post('components-status'),
      install: () => post('components-install'),
      restart: () => post('components-restart').then(() => undefined),
      subscribe: (listener) => {
        const source = new EventSource(base + '/components-events')
        source.onmessage = (event) => {
          try { listener(JSON.parse(event.data)) } catch (error) { console.warn('desktop bridge: component event', error) }
        }
        return () => { source.close() }
      },
    },
  }
  document.documentElement.dataset.platform = ${JSON.stringify(platform)}
})()`
}
