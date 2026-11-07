/** Launch the Desktop profile through the Web application and report its URL to the shell. */

import { randomUUID } from 'node:crypto'
import { delimiter, join } from 'node:path'
import type { PluginOperationRunId } from '@deepseek-ai/dsh-plugin-manager/diagnostic'
import type {} from '@deepseek-ai/dsh-client-connection'
import type {} from '@deepseek-ai/dsh-host-webserver'
import type {} from '@deepseek-ai/dsh-deepseek-account'
import { resolveDshHome } from '@deepseek-ai/dsh-home-paths'
import * as desktopOffice from './office.ts'
import { formatFatalDiagnostic } from './fatal-diagnostic.ts'
import { localDragResource } from './file-drag.ts'

import { installDesktopUpdateTaskControl } from './update-tasks.ts'
import { installDesktopQuitInspection } from './quit-inspection.ts'
import { installOperationDiagnostics } from './operation-diagnostics.ts'
import { installPlatformSessionPublisher } from './platform-session.ts'
import { installOfficeEngineResolution } from './office-engine.ts'
import { createHostTransport } from './transport.ts'
import { launchDesktopProfile } from './desktop-launch.ts'

/**
 * Control channel chosen once: Electron IPC when forked with a channel, stdio
 * when a native shell spawned this executable directly.
 */
const transport = createHostTransport()

async function main(): Promise<void> {
  const runId = randomUUID() as PluginOperationRunId
  // Announce the Host lifetime before startup so management operations emitted
  // during composition carry the run the shell journals.
  try { await transport.send({ type: 'starting', runId }) } catch (error: unknown) { console.error(error) }
  const runtimeDir = process.argv[2] as string
  const projectDir = process.argv[3] as string
  installOfficeEngineResolution(runtimeDir)
  const control: {
    updateTasks?: ReturnType<typeof installDesktopUpdateTaskControl>
    quitInspection?: ReturnType<typeof installDesktopQuitInspection>
  } = {}

  let stopping = false
  const dragReads = new Map<number, AbortController>()
  const dragTasks = new Set<Promise<void>>()

  // The task control answers the shell's update hand-off; it is registered
  // before the profile boots, so a request that arrives during the boot gets
  // an answer instead of silence.
  transport.onMessage((message) => {
    if (message.type !== 'update-tasks' || !('requestId' in message) || !Number.isSafeInteger(message.requestId)
      || !('action' in message) || !['inspect', 'lock', 'unlock'].includes(String(message.action))) return
    void (async () => {
      try {
        if (stopping || control.updateTasks === undefined) throw new Error('desktop update: Host is unavailable')
        const active = await control.updateTasks(message.action as 'inspect' | 'lock' | 'unlock')
        await transport.send({ type: 'update-tasks', requestId: message.requestId, active })
      } catch (error) {
        await transport.send({ type: 'update-tasks', requestId: message.requestId, active: true,
          error: error instanceof Error ? error.message : String(error) })
      }
    })().catch((error: unknown) => { console.error(error) })
  })

  // File drag reads are answerable only once the profile composed its
  // filesystem service; a cancel aborts the read the shell abandoned.
  transport.onMessage((message) => {
    if (message.type === 'file-drag-cancel') {
      if ('requestId' in message && typeof message.requestId === 'number') dragReads.get(message.requestId)?.abort()
      return
    }
    if (message.type !== 'file-drag-resource') return
    if (!('requestId' in message) || typeof message.requestId !== 'number' || !Number.isSafeInteger(message.requestId)
      || !('address' in message) || typeof message.address !== 'string') return
    const { requestId, address } = message
    const abort = new AbortController()
    dragReads.set(requestId, abort)
    const task = (async () => {
      try {
        if (stopping) throw new Error('Host stopping')
        const resource = await localDragResource(host.ctx, address, abort.signal)
        await transport.send({ type: 'file-drag-resource', requestId, resource })
      } catch (error) {
        if (!abort.signal.aborted) console.error(error)
        await transport.send({ type: 'file-drag-resource', requestId, resource: null, error: 'File drag resource unavailable' })
      } finally { dragReads.delete(requestId) }
    })().catch((error: unknown) => { console.error(error) })
    dragTasks.add(task)
    void task.finally(() => { dragTasks.delete(task) })
  })

  const host = await launchDesktopProfile({
    transport,
    runtimeDir,
    projectDir,
    ...(process.argv[5] === undefined ? {} : {
      packageManager: {
        command: process.execPath,
        args: ['--expose-internals', process.argv[5]],
        env: {
          ELECTRON_RUN_AS_NODE: '1',
          DSH_DESKTOP_NODE_EXECUTABLE: process.execPath,
          PATH: `${process.argv[6] ?? ''}${delimiter}${process.env.PATH ?? ''}`,
        },
      },
    }),
    onStopping: async () => {
      stopping = true
      for (const abort of dragReads.values()) abort.abort()
      await Promise.allSettled(dragTasks)
    },
    compose: async (ctx) => {
      control.updateTasks = installDesktopUpdateTaskControl(ctx)
      control.quitInspection = installDesktopQuitInspection(ctx)
      installOperationDiagnostics(ctx, record => transport.send({ type: 'plugin-operation', record }), 32, runId)
      await ctx.plugin(desktopOffice, {
        runtimeDir,
        source: process.argv[4] ?? join(runtimeDir, '..', 'runtime', 'primary-runtime'),
        root: join(resolveDshHome(), 'dsh-runtimes', 'dsh-primary-runtime'),
      })
      installPlatformSessionPublisher(ctx, (session) => {
        void transport.send({ type: 'platform-session', session }).catch((error: unknown) => { console.error(error) })
      })
    },
    quitInspection: () => control.quitInspection?.(),
  })
}

if (import.meta.main) {
  main().catch(async (error: unknown) => {
    const message = error instanceof Error ? error.message : String(error)
    // The shell receives the complete inspected error here, not through stderr:
    // stderr bytes and this control frame race, and the shell reports the first
    // failure it sees.
    const diagnostic = formatFatalDiagnostic(error)
    try { await transport.send({ type: 'fatal', message, diagnostic }) } catch (sendError: unknown) { console.error(sendError) }
    console.error(error)
    process.exitCode = 1
    transport.close()
  })
}
