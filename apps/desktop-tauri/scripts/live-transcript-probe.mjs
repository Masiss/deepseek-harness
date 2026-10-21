/**
 * Prove that a live model's transcript replays identically.
 *
 * The Desktop lane must not change what the model sees and what the session
 * records. This probe runs one turn against a real model, records the session
 * it wrote, then replays the same turn from that recording through the replay
 * adapter. The two session logs are normalized the way the snapshot lane
 * normalizes them — volatile identities become tokens — and compared: a
 * difference means the harness itself changed the transcript.
 *
 *   usage: live-transcript-probe.mjs
 */
import { spawn } from 'node:child_process'
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, statSync } from 'node:fs'
import { homedir, tmpdir } from 'node:os'
import { join } from 'node:path'
import { zstdDecompressSync } from 'node:zlib'

// The normalizer the snapshot lane uses, imported from source so the probe
// does not need a built package.
const { normalizeSessionLog } = await import(
  new URL('../../../packages/test-support/session-snapshot/src/normalize.ts', import.meta.url).href
)

/** The task both runs ask the model. */
const task = 'Reply with exactly LIVE_OK'
/** The sentinel the live answer carries. */
const sentinel = 'LIVE_OK'

const repo = new URL('../../..', import.meta.url).pathname

/**
 * Run a turn and stop it once the answer arrives.
 * @param {string[]} args - The command arguments.
 * @param {Record<string, string>} env - The environment of the run.
 * @returns {Promise<{code: number, out: string, err: string}>} The run's result.
 */
function runTurn(args, env) {
  return new Promise((resolve) => {
    const child = spawn('node', ['--import', join(repo, 'node_modules/tsx/dist/esm/index.mjs'),
      join(repo, 'apps/cli/src/bin.ts'), ...args], { cwd: repo, env: { ...process.env, ...env }, stdio: ['ignore', 'pipe', 'pipe'] })
    let out = ''
    let err = ''
    let answered = false
    const finish = (code) => {
      clearTimeout(timer)
      child.kill('SIGKILL')
      resolve({ code, out, err })
    }
    const timer = setTimeout(() => finish(124), 240_000)
    child.stdout.on('data', chunk => {
      out += String(chunk)
      if (!answered && out.includes(sentinel)) {
        answered = true
        // The answer is in; the run is stopped so the session is flushed.
        setTimeout(() => finish(0), 500)
      }
    })
    child.stderr.on('data', chunk => { err += String(chunk) })
    child.on('close', code => { if (!answered) finish(code ?? 0) })
  })
}

/**
 * The session log a harness home holds, newest first.
 * @param {string} home - The harness home.
 * @returns {{path: string, log: string} | undefined} The newest session.
 */
function newestSession(home) {
  const sessions = join(home, 'sessions')
  let directories
  try {
    directories = readdirSync(sessions, { withFileTypes: true })
  } catch {
    return undefined
  }
  const found = []
  for (const directory of directories) {
    if (!directory.isDirectory()) continue
    const workspace = join(sessions, directory.name)
    for (const entry of readdirSync(workspace, { withFileTypes: true })) {
      if (!entry.isDirectory() || !entry.name.startsWith('session-')) continue
      const sessionDir = join(workspace, entry.name)
      for (const file of readdirSync(sessionDir)) {
        if (!file.startsWith('session.v')) continue
        const path = join(sessionDir, file)
        found.push({ path, mtime: statSync(path).mtimeMs })
      }
    }
  }
  if (found.length === 0) return undefined
  const newest = found.sort((left, right) => right.mtime - left.mtime)[0]
  const bytes = readFileSync(newest.path)
  const log = newest.path.endsWith('.zstd')
    ? zstdDecompressSync(bytes).toString('utf8')
    : bytes.toString('utf8')
  return { path: newest.path, log }
}

/**
 * The session ids a log declares.
 * @param {string} log - The session log.
 * @returns {string[]} The session ids.
 */
function sessionIds(log) {
  const ids = new Set()
  for (const line of log.split('\n')) {
    if (line.trim() === '') continue
    try {
      const record = JSON.parse(line)
      const id = record?.data?.sessionId ?? record?.sessionId
      if (typeof id === 'string') ids.add(id)
    } catch {
      // A line that is not JSON is not a session header.
    }
  }
  return [...ids]
}

const home = mkdtempSync(join(tmpdir(), 'dsh-live-transcript-'))
const liveHome = join(home, 'live')
const replayHome = join(home, 'replay')

// The live run reads its credential from the harness home, so the store the
// provider owns travels with the run.
const credentialStore = join(process.env.DSH_HOME ?? join(homedir(), '.dsh'), '.credentials.yaml')
if (!existsSync(credentialStore)) {
  console.log('RESULT live-transcript=FAIL reason=no-credential-store')
  process.exit(1)
}
mkdirSync(liveHome, { recursive: true })
copyFileSync(credentialStore, join(liveHome, '.credentials.yaml'))

// The live run.
const live = await runTurn(['--profile', 'headless',
  '--patch', join(repo, 'apps/desktop-tauri/scripts/codex-model.patch.yml'), task], { DSH_HOME: liveHome })
if (!live.out.includes(sentinel)) {
  console.log('RESULT live-transcript=FAIL reason=no-answer')
  console.log(live.err.slice(-500))
  process.exit(1)
}
const recorded = newestSession(liveHome)
if (recorded === undefined) {
  console.log('RESULT live-transcript=FAIL reason=no-session')
  process.exit(1)
}
console.log(`live_session=${recorded.path}`)

// The replay run: the same turn, answered from the recording.
const fixture = join(home, 'fixture.jsonl')
const { writeFileSync } = await import('node:fs')
writeFileSync(fixture, recorded.log)
const replay = await runTurn(['--profile', 'headless',
  '--patch', join(repo, 'apps/desktop-tauri/scripts/live-transcript.patch.yml'), task],
{ DSH_HOME: replayHome, DSH_SNAPSHOT_FILE: fixture })
if (!replay.out.includes(sentinel)) {
  console.log('RESULT live-transcript=FAIL reason=replay-failed')
  console.log(replay.err.slice(-800))
  process.exit(1)
}
const replayed = newestSession(replayHome)
if (replayed === undefined) {
  console.log('RESULT live-transcript=FAIL reason=no-replayed-session')
  process.exit(1)
}
console.log(`replayed_session=${replayed.path}`)

// The comparison: the same normalization the snapshot lane uses.
const cwd = process.cwd()
const normalizedLive = normalizeSessionLog(recorded.log, { sessionIds: sessionIds(recorded.log), cwd })
const normalizedReplay = normalizeSessionLog(replayed.log, { sessionIds: sessionIds(replayed.log), cwd })
if (normalizedLive !== normalizedReplay) {
  console.log('RESULT live-transcript=FAIL reason=diff')
  const liveLines = normalizedLive.split('\n')
  const replayLines = normalizedReplay.split('\n')
  for (let index = 0; index < Math.max(liveLines.length, replayLines.length); index += 1) {
    if (liveLines[index] === replayLines[index]) continue
    console.log(`  live   ${String(liveLines[index]).slice(0, 300)}`)
    console.log(`  replay ${String(replayLines[index]).slice(0, 300)}`)
    break
  }
  process.exit(1)
}

console.log('RESULT live-transcript=OK')
