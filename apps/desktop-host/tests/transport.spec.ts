import { PassThrough } from 'node:stream'
import { describe, expect, it } from 'vitest'
import {
  CONTROL_FRAME_PREFIX,
  createStdioTransport,
  decodeControlFrame,
  encodeControlFrame,
} from '../src/transport.ts'

/** Collect every byte a transport writes. */
function collector(): { stream: PassThrough; text: () => string } {
  const stream = new PassThrough()
  let text = ''
  stream.setEncoding('utf8')
  stream.on('data', (chunk: string) => { text += chunk })
  return { stream, text: () => text }
}

/** Resolve after the next macrotask, so stream events settle. */
function settle(): Promise<void> {
  return new Promise((resolve) => { setTimeout(resolve, 0) })
}

describe('control frames', () => {
  it('round-trips one message with its prefix and terminator', () => {
    const frame = encodeControlFrame({ type: 'ready', url: 'http://127.0.0.1:1/?token=x' })
    expect(frame.startsWith(CONTROL_FRAME_PREFIX)).toBe(true)
    expect(frame.endsWith('\n')).toBe(true)
    expect(decodeControlFrame(frame.trimEnd())).toEqual({ type: 'ready', url: 'http://127.0.0.1:1/?token=x' })
  })

  it('ignores unrelated output, malformed JSON, and frames without a type', () => {
    expect(decodeControlFrame('the Host printed a line')).toBeUndefined()
    expect(decodeControlFrame(`${CONTROL_FRAME_PREFIX}{`)).toBeUndefined()
    expect(decodeControlFrame(`${CONTROL_FRAME_PREFIX}{"url":"x"}`)).toBeUndefined()
    expect(decodeControlFrame(`${CONTROL_FRAME_PREFIX}{"type":""}`)).toBeUndefined()
  })

  it('treats a carriage return from a Windows pipe as part of the terminator', () => {
    expect(decodeControlFrame(`${CONTROL_FRAME_PREFIX}{"type":"ready"}\r`)).toEqual({ type: 'ready' })
  })
})

describe('stdio transport', () => {
  it('writes one frame per message', async () => {
    const output = collector()
    const transport = createStdioTransport(new PassThrough(), output.stream)
    await transport.send({ type: 'ready' })
    await transport.send({ type: 'shutdown-complete' })
    expect(output.text()).toBe(
      `${CONTROL_FRAME_PREFIX}{"type":"ready"}\n${CONTROL_FRAME_PREFIX}{"type":"shutdown-complete"}\n`,
    )
  })

  it('reassembles frames split across chunks and skips interleaved output', async () => {
    const input = new PassThrough()
    const received: object[] = []
    const transport = createStdioTransport(input, new PassThrough())
    transport.onMessage((message) => { received.push(message) })
    input.write('unrelated log line\n')
    input.write(`${CONTROL_FRAME_PREFIX}{"type":"shut`)
    input.write(`down"}\n${CONTROL_FRAME_PREFIX}{"type":"quit-inspection","requestId":7}\n`)
    await settle()
    expect(received).toEqual([
      { type: 'shutdown' },
      { type: 'quit-inspection', requestId: 7 },
    ])
  })

  it('reports disconnect once when the shell closes stdin', async () => {
    const input = new PassThrough()
    const transport = createStdioTransport(input, new PassThrough())
    let disconnects = 0
    transport.onDisconnect(() => { disconnects += 1 })
    input.end()
    await settle()
    await settle()
    expect(disconnects).toBe(1)
  })

  it('stops writing after close', async () => {
    const output = collector()
    const transport = createStdioTransport(new PassThrough(), output.stream)
    transport.close()
    await transport.send({ type: 'ready' })
    expect(output.text()).toBe('')
  })
})
