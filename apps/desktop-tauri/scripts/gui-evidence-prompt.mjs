#!/usr/bin/env node
/** Drive one Desktop prompt through the authenticated RPC transport. */
import { randomUUID } from 'node:crypto'

const base = process.env.DSH_GUI_BASE
const launchUrl = process.env.DSH_GUI_LAUNCH_URL
const cwd = process.env.DSH_GUI_WORKSPACE
if (!base || !launchUrl || !cwd) throw new Error('missing DSH_GUI_BASE, DSH_GUI_LAUNCH_URL, or DSH_GUI_WORKSPACE')

const login = await fetch(launchUrl, { redirect: 'manual' })
if (login.status !== 303) throw new Error(`launch URL: HTTP ${login.status}`)
const setCookie = login.headers.get('set-cookie')
const cookie = setCookie?.split(';', 1)[0]
if (!cookie) throw new Error('launch URL did not set an authentication cookie')

/**
 * Call one Host RPC endpoint using the same envelope as createWebConnectionRpc.
 * @param {string} method - Channel-relative endpoint.
 * @param {unknown} args - Endpoint argument object.
 * @returns {Promise<unknown>} Decoded result value.
 */
async function call(method, args) {
  const rpcId = randomUUID()
  const response = await fetch(`${base}/api/${method}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', cookie },
    body: JSON.stringify({ type: 'client-request', rpcId, method, payload: { args } }),
  })
  if (!response.ok) throw new Error(`RPC ${method}: HTTP ${response.status}`)
  const envelope = await response.json()
  if (envelope?.type !== 'server-response' || envelope.rpcId !== rpcId) throw new Error(`RPC ${method}: invalid response envelope`)
  if (envelope.result?.ok !== true) throw new Error(`RPC ${method}: ${envelope.result?.error?.message ?? 'request failed'}`)
  return envelope.result.value
}

const created = await call('session/create', { request: { cwd } })
const sessionId = created?.sessionId
if (typeof sessionId !== 'string' || sessionId.length === 0) throw new Error('session/create returned no sessionId')
const requestId = randomUUID()
const prompt = await call('session/prompt', {
  request: {
    requestId,
    sessionId,
    mode: 'queue',
    content: [{ type: 'text', text: 'Reply with exactly GUI_EVIDENCE_OK' }],
  },
})
if (prompt?.accepted !== true) throw new Error('session/prompt was not accepted')
console.log(`RESULT model-turn=OK session=${sessionId} request=${requestId}`)
