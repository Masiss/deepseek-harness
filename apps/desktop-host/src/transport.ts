/** Desktop Host control transport: Electron IPC when forked, NDJSON frames over stdio for a native shell. */

import type { Readable, Writable } from 'node:stream'

/** Marker that separates a control frame from unrelated output on the same stream. */
export const CONTROL_FRAME_PREFIX = '@@DSH@@'

/** Upper bound of one control frame; `fatal` diagnostics are capped below this. */
export const MAX_CONTROL_FRAME_CHARS = 1024 * 1024

/** One decoded control message; every frame carries a discriminating `type`. */
export interface HostControlMessage {
  readonly type: string
  readonly [key: string]: unknown
}

/** How the shell and the Host exchange control messages. */
export interface HostTransport {
  /** Which channel this transport owns; the shell uses the same name in its own logs. */
  readonly kind: 'ipc' | 'stdio'
  /**
   * Deliver one control message to the shell.
   * @param message - JSON-serializable control message carrying a string `type`.
   * @returns Resolves when the frame is handed to the channel.
   */
  send(message: object): Promise<void>
  /**
   * Receive control messages from the shell.
   * @param listener - Called for every well-formed message.
   */
  onMessage(listener: (message: HostControlMessage) => void): void
  /**
   * Observe the channel closing.
   * @param listener - Called once when the shell disconnects or closes stdin.
   */
  onDisconnect(listener: () => void): void
  /** Release the channel; a no-op for Electron IPC, which the parent owns. */
  close(): void
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/**
 * Render one control message as a framed line.
 * @param message - JSON-serializable control message.
 * @returns One newline-terminated frame.
 */
export function encodeControlFrame(message: object): string {
  return `${CONTROL_FRAME_PREFIX}${JSON.stringify(message)}\n`
}

/**
 * Decode one line from the control channel.
 * @param line - One line without its terminator.
 * @returns The message, or `undefined` for unrelated output, malformed JSON, or a frame without `type`.
 */
export function decodeControlFrame(line: string): HostControlMessage | undefined {
  const text = line.endsWith('\r') ? line.slice(0, -1) : line
  if (!text.startsWith(CONTROL_FRAME_PREFIX)) return undefined
  if (text.length > MAX_CONTROL_FRAME_CHARS) return undefined
  let value: unknown
  try {
    value = JSON.parse(text.slice(CONTROL_FRAME_PREFIX.length))
  } catch (error) {
    // Unrelated output can share the channel; a malformed frame is reported and skipped.
    console.error(`desktop host: ignored malformed control frame (${error instanceof Error ? error.message : String(error)})`)
    return undefined
  }
  if (!isRecord(value) || typeof value.type !== 'string' || value.type === '') return undefined
  return { ...value, type: value.type }
}

/**
 * Create the stdio transport used by a native shell that spawned this process directly.
 * @param input - Control stream carrying shell messages.
 * @param output - Stream receiving Host frames; unrelated output must stay on stderr.
 * @returns A framed JSON-lines transport.
 */
export function createStdioTransport(input: Readable, output: Writable): HostTransport {
  const listeners: ((message: HostControlMessage) => void)[] = []
  const disconnectListeners: (() => void)[] = []
  let pending = ''
  let closed = false

  input.setEncoding('utf8')
  input.on('data', (chunk: string) => {
    pending += chunk
    for (;;) {
      const boundary = pending.indexOf('\n')
      if (boundary < 0) break
      const line = pending.slice(0, boundary)
      pending = pending.slice(boundary + 1)
      const message = decodeControlFrame(line)
      if (message === undefined) continue
      for (const listener of listeners) listener(message)
    }
    if (pending.length > MAX_CONTROL_FRAME_CHARS) {
      console.error('desktop host: discarded an oversized control frame')
      pending = ''
    }
  })
  input.on('end', () => {
    if (closed) return
    closed = true
    for (const listener of disconnectListeners) listener()
  })

  return {
    kind: 'stdio',
    send: message => new Promise<void>((resolve, reject) => {
      if (closed) { resolve(); return }
      output.write(encodeControlFrame(message), (error) => {
        if (error === null || error === undefined) resolve()
        else reject(error)
      })
    }),
    onMessage: (listener) => { listeners.push(listener) },
    onDisconnect: (listener) => { disconnectListeners.push(listener) },
    close: () => {
      closed = true
      // A piped stdin holds the event loop open. The shell keeps the pipe for the
      // control channel, so the Host releases it once shutdown has been reported;
      // otherwise it would linger until the shell closed its end.
      const handle = input as { unref?: () => void }
      if (typeof handle.unref === 'function') handle.unref()
    },
  }
}

/**
 * Create the Electron IPC transport used when the shell forks this process with a channel.
 * @returns An IPC transport, or `undefined` when no parent channel exists.
 */
export function createIpcTransport(): HostTransport | undefined {
  if (typeof process.send !== 'function') return undefined
  const listeners: ((message: HostControlMessage) => void)[] = []
  const disconnectListeners: (() => void)[] = []
  let closed = false
  const disconnect = (): void => {
    if (closed) return
    closed = true
    for (const listener of disconnectListeners) listener()
  }
  process.on('message', (message: unknown) => {
    if (!isRecord(message) || typeof message.type !== 'string' || message.type === '') return
    const control: HostControlMessage = { ...message, type: message.type }
    for (const listener of listeners) listener(control)
  })
  process.once('disconnect', disconnect)
  return {
    kind: 'ipc',
    send: message => new Promise<void>((resolve, reject) => {
      if (!process.connected || process.send === undefined) { resolve(); return }
      process.send(message, (error) => {
        if (error === null) resolve()
        else reject(error)
      })
    }),
    onMessage: (listener) => { listeners.push(listener) },
    onDisconnect: (listener) => { disconnectListeners.push(listener) },
    close: () => { if (process.connected) process.disconnect() },
  }
}

/**
 * Choose the control channel for this process.
 *
 * A native shell spawns the Host without a Node IPC channel, so the Host claims
 * stdio and moves ordinary logging to stderr. Frames keep a prefix precisely
 * because unrelated output can still reach the same stream.
 * @returns The IPC transport when forked by Electron, otherwise the stdio transport.
 */
export function createHostTransport(): HostTransport {
  const ipc = createIpcTransport()
  if (ipc !== undefined) return ipc
  // Ordinary logging must not reach the frame stream; the decoder also rejects it.
  console.log = (...data: unknown[]): void => { console.error(...data) }
  console.info = (...data: unknown[]): void => { console.error(...data) }
  return createStdioTransport(process.stdin, process.stdout)
}
