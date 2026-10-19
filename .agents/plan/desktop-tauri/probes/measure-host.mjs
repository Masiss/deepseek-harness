/**
 * Measure one Desktop Host launch: cold start to the `ready` frame, resident
 * memory while idle, and clean shutdown over the stdio control channel.
 *
 * Usage: node measure-host.mjs <hostCommand> [args...]
 */

import { spawn } from 'node:child_process'
import { readFileSync } from 'node:fs'

const [command, ...args] = process.argv.slice(2)
if (command === undefined) {
  console.error('usage: node measure-host.mjs <command> [args...]')
  process.exit(2)
}

const started = process.hrtime.bigint()
const child = spawn(command, args, { stdio: ['pipe', 'pipe', 'inherit'] })
console.error(`[measure] spawned pid ${String(child.pid)}`)

let exited = false
child.on('exit', (code, signal) => {
  exited = true
  console.error(`[measure] host exited code=${String(code)} signal=${String(signal)}`)
})

let buffered = ''
let resolveReady
let rejectReady
const readyFrame = new Promise((resolve, reject) => {
  resolveReady = resolve
  rejectReady = reject
})

child.stdout.setEncoding('utf8')
child.stdout.on('data', (chunk) => {
  console.error(`[measure] stdout chunk ${String(chunk.length)} bytes`)
  buffered += chunk
  for (;;) {
    const boundary = buffered.indexOf('\n')
    if (boundary < 0) break
    const line = buffered.slice(0, boundary)
    buffered = buffered.slice(boundary + 1)
    if (!line.startsWith('@@DSH@@')) continue
    let frame
    try {
      frame = JSON.parse(line.slice(7))
    } catch {
      continue
    }
    console.error(`[measure] frame type=${String(frame.type)}`)
    if (frame.type === 'ready') resolveReady(frame)
    if (frame.type === 'fatal') rejectReady(new Error(String(frame.message)))
  }
})
child.stdout.on('error', (error) => { rejectReady(error) })

setTimeout(() => { rejectReady(new Error('timeout waiting for ready')) }, 120_000)

const ready = await readyFrame
const readyMs = Number(process.hrtime.bigint() - started) / 1e6

const readRss = () => {
  try {
    const status = readFileSync(`/proc/${String(child.pid)}/status`, 'utf8')
    const match = /VmRSS:\s+(\d+) kB/u.exec(status)
    return match === null ? undefined : Number(match[1]) * 1024
  } catch {
    return undefined
  }
}

// Let the Web server and session stores settle before sampling.
await new Promise((resolve) => { setTimeout(resolve, 5000) })
const rssIdle = readRss()

child.stdin.write('@@DSH@@{"type":"shutdown"}\n')
const exitCode = await new Promise((resolve) => {
  if (exited) { resolve('already-exited'); return }
  child.on('exit', (code) => { resolve(code) })
  setTimeout(() => { resolve('shutdown-timeout') }, 30_000)
})

console.log(JSON.stringify({
  command,
  readyMs: Math.round(readyMs),
  url: ready.url,
  injections: Array.isArray(ready.injections) ? ready.injections.length : 0,
  rssIdleBytes: rssIdle,
  rssIdleMiB: rssIdle === undefined ? undefined : Math.round(rssIdle / 1024 / 1024),
  shutdownExitCode: exitCode,
}, null, 2))
process.exit(0)
