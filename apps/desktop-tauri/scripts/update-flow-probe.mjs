#!/usr/bin/env node
/**
 * Drive the whole update flow against the packaged Host without a window.
 *
 * This script plays the shell: it serves a release feed, spawns the Host over
 * stdio, answers the `update-prompt` frames the native dialog would answer, and
 * checks that the artifact replaced `$APPIMAGE` and that `restart` arrived.
 *
 *   usage: node update-flow-probe.mjs [linux-x64|linux-arm64]
 */

import { createHash, generateKeyPairSync, sign } from 'node:crypto'
import { createServer } from 'node:http'
import { cp, mkdir, readFile, rm, stat, writeFile } from 'node:fs/promises'
import { spawn } from 'node:child_process'
import { join } from 'node:path'

const repo = new URL('../../..', import.meta.url).pathname
const target = process.argv[2] ?? 'linux-x64'
const tree = join(repo, 'apps/desktop/.desktop-build/targets', target)
// Under isolated-run.sh the tree is read-only, so the probe keeps its writable
// state (seed profile, fake image, DSH home) in the bound work directory.
const work = process.env.DSH_ISOLATION_DIR === undefined
  ? join(tree, 'appimage', 'flow-probe')
  : join(process.env.DSH_ISOLATION_DIR, 'flow')

const OLD_IMAGE = Buffer.from('the installed version')
const NEW_IMAGE = Buffer.from(`the published version ${Date.now()}\n`.repeat(64))
const digest = createHash('sha256').update(NEW_IMAGE).digest('hex')
const keys = generateKeyPairSync('ed25519')
const feedKey = keys.publicKey.export({ type: 'spki', format: 'pem' }).toString()

/** The canonical bytes the Host verifies, with this run's signature. */
function signedManifest() {
  const document = {
    schemaVersion: 2,
    version: '2.0.0',
    artifacts: {
      [target]: { url: `http://127.0.0.1:${feedPort}/app.bin`, sha256: digest, size: NEW_IMAGE.byteLength },
    },
  }
  return {
    ...document,
    signature: sign(null, Buffer.from(JSON.stringify(document), 'utf8'), keys.privateKey).toString('base64'),
  }
}

let feedPort = 0
const feed = createServer((request, response) => {
  if (request.url === '/app.bin') {
    response.end(NEW_IMAGE)
    return
  }
  response.setHeader('content-type', 'application/json')
  response.end(JSON.stringify(signedManifest()))
})

await rm(work, { recursive: true, force: true })
await mkdir(join(work, 'home'), { recursive: true })
// The Host repairs the profile it boots, so it needs its own copy.
await cp(join(tree, 'project'), join(work, 'project'), { recursive: true })
const image = join(work, 'dsh.AppImage')
await writeFile(image, OLD_IMAGE, { mode: 0o755 })

await new Promise((resolve) => { feed.listen(0, '127.0.0.1', resolve) })
feedPort = feed.address().port

const root = join(tree, 'runtime/primary-runtime')
const host = spawn(join(tree, 'bin/desktop-host'), [
  join(tree, 'dsh'), join(work, 'project'),
  root, join(root, 'dependencies/pnpm/bin/pnpm.mjs'), join(tree, 'runtime/bin'),
], {
  env: {
    ...process.env,
    DSH_HOME: join(work, 'home'),
    DSH_DESKTOP_UPDATE_FEED: `http://127.0.0.1:${feedPort}/manifest.json`,
    DSH_DESKTOP_UPDATE_PUBLIC_KEY: feedKey,
    DSH_DESKTOP_VERSION: '1.0.0',
    DSH_CLIENT_VERSION: '1.0.0',
    APPIMAGE: image,
  },
  stdio: ['pipe', 'pipe', 'inherit'],
})

const seen = { prompts: [], restart: false }
const send = (frame) => host.stdin.write(`@@DSH@@${JSON.stringify(frame)}\n`)

let ready
const readyPromise = new Promise((resolve) => { ready = resolve })

host.stdout.setEncoding('utf8')
const reader = (async () => {
  let buffer = ''
  for await (const chunk of host.stdout) {
    buffer += chunk
    let index
    while ((index = buffer.indexOf('\n')) >= 0) {
      const line = buffer.slice(0, index)
      buffer = buffer.slice(index + 1)
      if (!line.startsWith('@@DSH@@')) continue
      const frame = JSON.parse(line.slice('@@DSH@@'.length))
      if (frame.type === 'ready') ready(frame.url)
      if (frame.type === 'restart') seen.restart = true
      if (frame.type === 'update-prompt') {
        seen.prompts.push(`${frame.action}:${frame.version}`)
        send({ type: 'update-prompt', promptId: frame.promptId, accepted: true })
      }
    }
  }
})()

const url = await Promise.race([
  readyPromise,
  new Promise((_, reject) => { setTimeout(() => { reject(new Error('host never reported ready')) }, 120_000) }),
])

const handoff = await fetch(url, { redirect: 'manual' })
const cookie = handoff.headers.getSetCookie().map((value) => value.split(';')[0]).join('; ')
const base = new URL(url).origin

const opened = await fetch(`${base}/__dsh-desktop/updates-open`, {
  method: 'POST',
  headers: { 'content-type': 'application/json', cookie },
  body: 'null',
  signal: AbortSignal.timeout(120_000),
})

const status = await (await fetch(`${base}/__dsh-desktop/updates-status`, {
  method: 'POST',
  headers: { 'content-type': 'application/json', cookie },
  body: 'null',
})).json()

const installed = await readFile(image)
const mode = (await stat(image)).mode & 0o777

console.log(`RESULT ready=OK port=${new URL(url).port}`)
console.log(`prompts=${seen.prompts.join(',')}`)
console.log(`open_status=${opened.status} presentation=${JSON.stringify(status)}`)
console.log(`restart_frame=${seen.restart}`)
const replaced = installed.equals(NEW_IMAGE)
console.log(`image_replaced=${replaced} mode=${mode.toString(8)}`)

send({ type: 'shutdown' })
host.stdin.end()
await Promise.race([
  new Promise((resolve) => { host.once('exit', resolve) }),
  new Promise((resolve) => { setTimeout(resolve, 15_000) }),
])
await reader
feed.close()
await rm(work, { recursive: true, force: true })

const ok = opened.status === 204
  && status.phase === 'installing'
  && seen.prompts.includes('download:2.0.0')
  && seen.prompts.includes('install:2.0.0')
  && seen.restart
  && replaced
console.log(`RESULT update-flow=${ok ? 'OK' : 'FAIL'}`)
if (!ok) process.exitCode = 1
