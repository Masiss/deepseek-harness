#!/usr/bin/env node
// Minimal DeepSeek Messages provider used by the approval probe.
//
// First assistant turn requests a bash call that widens the sandbox, so the
// Host must ask the user; after the tool result arrives the second turn ends
// with plain text. The server prints `MOCK_MODEL_URL=<origin>` on stdout so the
// probe can point DEEPSEEK_BASE_URL at it, and logs each turn to stderr.

import { createServer } from 'node:http'

const MARKER = process.env.DSH_APPROVAL_PROBE_MARKER
if (typeof MARKER !== 'string' || MARKER.length === 0) {
  console.error('mock-model-server: DSH_APPROVAL_PROBE_MARKER is required')
  process.exit(2)
}

function frame(event) {
  return `event: ${event.type}\ndata: ${JSON.stringify(event)}\n\n`
}

function toolTurn() {
  const input = {
    command: `echo approval-ok > '${MARKER}'`,
    description: 'Write the approval probe marker',
    sandbox_permissions: 'danger-full-access',
    justification: 'The approval probe must prove an escalated command runs after consent.',
  }
  return [
    frame({
      type: 'message_start',
      message: {
        id: 'msg_mock_tool',
        type: 'message',
        role: 'assistant',
        model: 'deepseek-flash',
        content: [],
        usage: { input_tokens: 20, output_tokens: 1 },
      },
    }),
    frame({
      type: 'content_block_start',
      index: 0,
      content_block: { type: 'text', text: '' },
    }),
    frame({
      type: 'content_block_delta',
      index: 0,
      delta: { type: 'text_delta', text: 'Running the probe command.' },
    }),
    frame({ type: 'content_block_stop', index: 0 }),
    frame({
      type: 'content_block_start',
      index: 1,
      content_block: { type: 'tool_use', id: 'call_mock_1', name: 'bash', input },
    }),
    frame({ type: 'content_block_stop', index: 1 }),
    frame({
      type: 'message_delta',
      delta: { stop_reason: 'tool_use' },
      usage: { output_tokens: 12 },
    }),
    frame({ type: 'message_stop' }),
  ].join('')
}

function jobTurn() {
  const input = {
    command: 'sleep 0.4; echo job-output-ok',
    description: 'Emit the background job marker',
    run_in_background: true,
  }
  return [
    frame({
      type: 'message_start',
      message: {
        id: 'msg_mock_job',
        type: 'message',
        role: 'assistant',
        model: 'deepseek-flash',
        content: [],
        usage: { input_tokens: 24, output_tokens: 1 },
      },
    }),
    frame({
      type: 'content_block_start',
      index: 0,
      content_block: { type: 'text', text: '' },
    }),
    frame({
      type: 'content_block_delta',
      index: 0,
      delta: { type: 'text_delta', text: 'Starting the background job.' },
    }),
    frame({ type: 'content_block_stop', index: 0 }),
    frame({
      type: 'content_block_start',
      index: 1,
      content_block: { type: 'tool_use', id: 'call_mock_job', name: 'bash', input },
    }),
    frame({ type: 'content_block_stop', index: 1 }),
    frame({
      type: 'message_delta',
      delta: { stop_reason: 'tool_use' },
      usage: { output_tokens: 14 },
    }),
    frame({ type: 'message_stop' }),
  ].join('')
}

function questionTurn() {
  const input = {
    questions: [{
      id: 'probe',
      question: 'Continue the approval probe?',
      options: [{ label: 'Yes' }, { label: 'No' }],
    }],
  }
  return [
    frame({
      type: 'message_start',
      message: {
        id: 'msg_mock_question',
        type: 'message',
        role: 'assistant',
        model: 'deepseek-flash',
        content: [],
        usage: { input_tokens: 26, output_tokens: 1 },
      },
    }),
    frame({
      type: 'content_block_start',
      index: 0,
      content_block: { type: 'text', text: '' },
    }),
    frame({
      type: 'content_block_delta',
      index: 0,
      delta: { type: 'text_delta', text: 'I need one answer before continuing.' },
    }),
    frame({ type: 'content_block_stop', index: 0 }),
    frame({
      type: 'content_block_start',
      index: 1,
      content_block: { type: 'tool_use', id: 'call_mock_question', name: 'ask_user_question', input },
    }),
    frame({ type: 'content_block_stop', index: 1 }),
    frame({
      type: 'message_delta',
      delta: { stop_reason: 'tool_use' },
      usage: { output_tokens: 16 },
    }),
    frame({ type: 'message_stop' }),
  ].join('')
}

function textTurn() {
  return [
    frame({
      type: 'message_start',
      message: {
        id: 'msg_mock_text',
        type: 'message',
        role: 'assistant',
        model: 'deepseek-flash',
        content: [],
        usage: { input_tokens: 30, output_tokens: 1 },
      },
    }),
    frame({
      type: 'content_block_start',
      index: 0,
      content_block: { type: 'text', text: '' },
    }),
    frame({
      type: 'content_block_delta',
      index: 0,
      delta: { type: 'text_delta', text: 'Approval probe complete.' },
    }),
    frame({ type: 'content_block_stop', index: 0 }),
    frame({
      type: 'message_delta',
      delta: { stop_reason: 'end_turn' },
      usage: { output_tokens: 5 },
    }),
    frame({ type: 'message_stop' }),
  ].join('')
}

function toolResults(messages) {
  if (!Array.isArray(messages)) return 0
  return messages.reduce((total, message) =>
    total + (Array.isArray(message?.content)
      ? message.content.filter(block => block?.type === 'tool_result').length
      : 0), 0)
}

const server = createServer((request, response) => {
  if (request.method !== 'POST') {
    response.writeHead(404).end()
    return
  }
  const chunks = []
  request.on('data', chunk => chunks.push(chunk))
  request.on('end', () => {
    let body = {}
    try {
      body = JSON.parse(Buffer.concat(chunks).toString('utf8'))
    } catch (error) {
      console.error(`mock-model-server: invalid request JSON: ${error}`)
      response.writeHead(400).end()
      return
    }
    const results = toolResults(body.messages)
    const turn = results === 0 ? 'approval' : results === 1 ? 'job' : results === 2 ? 'question' : 'text'
    console.error(`mock-model-server: turn=${turn} results=${results} messages=${body.messages?.length ?? 0}`)
    response.writeHead(200, {
      'content-type': 'text/event-stream',
      'cache-control': 'no-cache',
      connection: 'keep-alive',
    })
    response.end(
      turn === 'approval' ? toolTurn()
      : turn === 'job' ? jobTurn()
      : turn === 'question' ? questionTurn()
      : textTurn(),
    )
  })
})

server.listen(0, '127.0.0.1', () => {
  const { port } = server.address()
  console.log(`MOCK_MODEL_URL=http://127.0.0.1:${port}`)
})

const shutdown = () => server.close(() => process.exit(0))
process.on('SIGTERM', shutdown)
process.on('SIGINT', shutdown)
