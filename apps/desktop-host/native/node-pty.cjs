'use strict'
/**
 * `node-pty`-shaped wrapper over the Rust native module.
 *
 * The terminal service reads exactly this surface: `spawn`, `pid`, `write`,
 * `resize`, `pause`, `resume`, `kill`, `onData`, `onExit`. Output that arrives
 * before `onData` attaches is buffered here, so the spawn-then-subscribe order
 * the service uses loses nothing.
 */

const native = require('./dsh-native.node')

function spawn(file, args, options = {}) {
  const dataListeners = new Set()
  const exitListeners = new Set()
  let pendingData = []
  let pendingExit

  const handle = native.spawn(
    file,
    [...args],
    {
      name: options.name,
      cols: options.cols,
      rows: options.rows,
      cwd: options.cwd,
      env: options.env,
    },
    (error, data) => {
      if (error !== null) return
      if (dataListeners.size === 0) {
        pendingData.push(data)
        return
      }
      for (const listener of dataListeners) listener(data)
    },
    (error, exit) => {
      if (error !== null) return
      if (exitListeners.size === 0) {
        pendingExit = exit
        return
      }
      for (const listener of exitListeners) listener(exit)
    },
  )

  return {
    pid: handle.pid,
    write: (data) => { handle.write(String(data)) },
    resize: (cols, rows) => { handle.resize(cols, rows) },
    pause: () => { handle.pause() },
    resume: () => { handle.resume() },
    kill: (signal) => { handle.kill(signal ?? 'SIGHUP') },
    onData(listener) {
      dataListeners.add(listener)
      if (pendingData.length > 0) {
        const queued = pendingData
        pendingData = []
        for (const chunk of queued) listener(chunk)
      }
      return { dispose() { dataListeners.delete(listener) } }
    },
    onExit(listener) {
      exitListeners.add(listener)
      if (pendingExit !== undefined) {
        const exit = pendingExit
        queueMicrotask(() => listener(exit))
      }
      return { dispose() { exitListeners.delete(listener) } }
    },
  }
}

module.exports = { spawn }
