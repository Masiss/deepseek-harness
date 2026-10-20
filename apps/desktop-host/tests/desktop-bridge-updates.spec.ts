/**
 * The update surface the Desktop bridge publishes when the Host has a service:
 * status, the user-requested check, and the event stream the settings row
 * subscribes to once.
 */

import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { Context } from '@deepseek-ai/cordis'
import WebServer from '@deepseek-ai/dsh-host-webserver'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { BRIDGE_PATH, installDesktopBridge, type DesktopUpdateChannel } from '../src/desktop-bridge.ts'
import type { DesktopUpdatePresentation } from '../src/update/service.ts'

/** A channel whose state the test drives directly. */
class StubChannel implements DesktopUpdateChannel {
  presentation: DesktopUpdatePresentation = { phase: 'idle' }
  opens = 0
  private readonly listeners = new Set<(state: DesktopUpdatePresentation) => void>()

  subscribe(listener: (state: DesktopUpdatePresentation) => void): () => void {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }

  async open(): Promise<void> {
    this.opens += 1
    this.emit({ phase: 'available', version: '0.3.0' })
  }

  /** Publish one state to every subscriber. */
  emit(state: DesktopUpdatePresentation): void {
    this.presentation = state
    for (const listener of this.listeners) listener(state)
  }
}

let context: Context | undefined
let home: string | undefined
let dispose: (() => void) | undefined
let channel: StubChannel

beforeEach(async () => {
  home = await mkdtemp(join(tmpdir(), 'dsh-desktop-bridge-updates-'))
  channel = new StubChannel()
  context = new Context()
  context.provide('connection', { requestRejection: () => undefined } as never)
  await context.plugin(WebServer, { host: '127.0.0.1', port: 0 })
  dispose = installDesktopBridge(context, { home, platform: 'linux', updates: channel })
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

describe('desktop bridge updates', () => {
  it('reports the service status and runs the requested flow', async () => {
    expect(await (await fetch(`${base()}${BRIDGE_PATH}/updates-status`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: 'null',
    })).json()).toEqual({ phase: 'idle' })

    const opened = await fetch(`${base()}${BRIDGE_PATH}/updates-open`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: 'null',
    })
    expect(opened.status).toBe(204)
    expect(channel.opens).toBe(1)

    expect(await (await fetch(`${base()}${BRIDGE_PATH}/updates-status`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: 'null',
    })).json()).toEqual({ phase: 'available', version: '0.3.0' })
  })

  it('forwards each published state over one event stream', async () => {
    const control = new AbortController()
    const response = await fetch(`${base()}${BRIDGE_PATH}/updates-events`, { signal: control.signal })
    expect(response.headers.get('content-type')).toBe('text/event-stream')
    const opened = response.body?.getReader()
    if (opened === undefined) throw new Error('spec: event stream has no body')
    const reader = opened

    const decoder = new TextDecoder()
    let seen = ''
    /** Read until the stream carried `count` events. */
    async function readEvents(count: number): Promise<DesktopUpdatePresentation[]> {
      while (seen.split('\n\n').filter(part => part !== '').length < count) {
        const { done, value } = await reader.read()
        if (done) throw new Error('spec: event stream ended early')
        seen += decoder.decode(value, { stream: true })
      }
      return seen.split('\n\n').filter(part => part !== '')
        .map(part => JSON.parse(part.replace(/^data: /u, '')) as DesktopUpdatePresentation)
    }

    expect(await readEvents(1)).toEqual([{ phase: 'idle' }])
    channel.emit({ phase: 'downloading', version: '0.3.0', percent: 42 })
    expect(await readEvents(2)).toEqual([{ phase: 'idle' }, { phase: 'downloading', version: '0.3.0', percent: 42 }])
    control.abort()
  })
})
