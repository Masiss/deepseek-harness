/** Launch the Desktop profile through the Web application and report its URL to the shell. */

import { delimiter, join } from 'node:path'
import { inspect } from 'node:util'
import type {} from '@deepseek-ai/dsh-client-connection'
import type {} from '@deepseek-ai/dsh-host-webserver'
import type {} from '@deepseek-ai/dsh-deepseek-account'
import { resolveDshHome } from '@deepseek-ai/dsh-home-paths'
import * as desktopOffice from './office.ts'

import { installDesktopUpdateTaskControl } from './update-tasks.ts'
import { installDesktopQuitInspection } from './quit-inspection.ts'
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
  const runtimeDir = process.argv[2] as string
  const projectDir = process.argv[3] as string
  installOfficeEngineResolution(runtimeDir)
  const control: {
    updateTasks?: ReturnType<typeof installDesktopUpdateTaskControl>
    quitInspection?: ReturnType<typeof installDesktopQuitInspection>
  } = {}
  let stopping = false

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

  await launchDesktopProfile({
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
    onStopping: () => { stopping = true },
    compose: async (ctx) => {
      control.updateTasks = installDesktopUpdateTaskControl(ctx)
      control.quitInspection = installDesktopQuitInspection(ctx)
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

/** Upper bound of the startup diagnostic carried over the control channel; the head holds the message and stack. */
const MAX_FATAL_DIAGNOSTIC_CHARS = 64 * 1024

if (import.meta.main) {
  main().catch(async (error: unknown) => {
    const message = error instanceof Error ? error.message : String(error)
    // The shell receives the complete inspected error here, not through stderr:
    // stderr bytes and this control frame race, and the shell reports the first
    // failure it sees.
    const diagnostic = inspect(error, { depth: 4, maxArrayLength: 50 }).slice(0, MAX_FATAL_DIAGNOSTIC_CHARS)
    try { await transport.send({ type: 'fatal', message, diagnostic }) } catch (sendError: unknown) { console.error(sendError) }
    console.error(error)
    process.exitCode = 1
    transport.close()
  })
}
