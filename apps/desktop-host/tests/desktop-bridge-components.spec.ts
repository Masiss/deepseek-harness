/**
 * The Office-component surface the Desktop bridge publishes: status, the
 * user-requested install, the restart that finishes it, and the event stream
 * the settings row subscribes to once.
 */

import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import WebServer from '@deepseek-ai/dsh-host-webserver'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { BRIDGE_PATH, installDesktopBridge, type DesktopOfficeChannel } from '../src/desktop-bridge.ts'
import type { DesktopOfficeStatus } from '../src/components/office.ts'

/** A channel whose state the test drives directly. */
class StubOffice implements DesktopOfficeChannel {
  presentation: DesktopOfficeStatus = { id: 'office', installed: false, active: false, phase: 'idle' }
  installs = 0
  restarts = 0
  private readonly listeners = new Set<(state: DesktopOfficeStatus) => void>()

  subscribe(listener: (state: DesktopOfficeStatus) => void): () => void {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }

  async install(): Promise<DesktopOfficeStatus> {
    this.installs += 1
    this.emit({ id: 'office', installed: true, active: false, phase: 'ready', version: '1.2.3' })
    return this.presentation
  }

  async restart(): Promise<void> {
    this.restarts += 1
  }

  /** Publish one state to every subscriber. */
  emit(state: DesktopOfficeStatus): void {
    this.presentation = state
    for (const listener of this.listeners) listener(state)
  }
}

let context: Context | undefined
let home: string | undefined
let dispose: (() => void) | undefined
let channel: StubOffice

beforeEach(async () => {
  home = await mkdtemp(join(tmpdir(), 'dsh-desktop-bridge-components-'))
  channel = new StubOffice()
  context = new Context()
  context.provide('connection', { requestRejection: () => undefined } as never)
  await context.plugin(WebServer, { host: '127.0.0.1', port: 0 })
  dispose = installDesktopBridge(context, { home, platform: 'linux', components: channel })
})

afterEach(async () => {
  dispose?.()
  dispose = undefined
  await context?.fiber.dispose()
  context = undefined
  if (home !== undefined) await rm(home, { recursive: true, force: true })
  home = undefined
})

/** Base URL of the bridge's booted server. */
function base(): string {
  if (context === undefined) throw new Error('spec: server not booted')
  return `http://127.0.0.1:${String(context.webServer.port)}`
}

async function post(method: string): Promise<Response> {
  return fetch(`${base()}${BRIDGE_PATH}/${method}`, {
    method: 'POST', headers: { 'content-type': 'application/json' }, body: 'null',
  })
}

describe('desktop bridge components', () => {
  it('reports status, installs on request, and restarts on request', async () => {
    expect(await (await post('components-status')).json()).toEqual({
      id: 'office', installed: false, active: false, phase: 'idle',
    })
    expect(await (await post('components-install')).json()).toEqual({
      id: 'office', installed: true, active: false, phase: 'ready', version: '1.2.3',
    })
    expect(channel.installs).toBe(1)
    expect((await post('components-restart')).status).toBe(204)
    expect(channel.restarts).toBe(1)
    expect(await (await post('components-status')).json()).toMatchObject({ installed: true, phase: 'ready' })
  })

  it('reports the default status and refuses actions without a channel', async () => {
    dispose?.()
    dispose = installDesktopBridge(context!, { home: home!, platform: 'linux' })
    expect(await (await post('components-status')).json()).toEqual({
      id: 'office', installed: false, active: false, phase: 'idle',
    })
    expect((await post('components-install')).status).toBe(503)
    expect((await post('components-restart')).status).toBe(503)
  })

  it('forwards each published state over one event stream', async () => {
    const control = new AbortController()
    const response = await fetch(`${base()}${BRIDGE_PATH}/components-events`, { signal: control.signal })
    expect(response.headers.get('content-type')).toBe('text/event-stream')
    const reader = response.body?.getReader()
    if (reader === undefined) throw new Error('spec: event stream has no body')
    const decoder = new TextDecoder()
    let seen = ''
    // The stream opens with the current state; wait for it before publishing.
    while (!seen.includes('"idle"')) {
      const { value, done } = await reader.read()
      if (done) break
      seen += decoder.decode(value, { stream: true })
    }
    channel.emit({ id: 'office', installed: false, active: false, phase: 'downloading', percent: 42 })
    while (!seen.includes('downloading')) {
      const { value, done } = await reader.read()
      if (done) break
      seen += decoder.decode(value, { stream: true })
    }
    expect(seen).toContain('"percent":42')
    control.abort()
    await reader.cancel().catch(() => undefined)
  })
})
