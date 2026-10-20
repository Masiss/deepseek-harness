/**
 * Tauri-side Desktop Host entry.
 *
 * It runs the same `desktop` profile as the Electron entry (`index.ts`) but
 * omits the Office plugin: Office becomes an on-demand component (ADR-0008).
 *
 * The two entries deliberately duplicate the launch body instead of sharing a
 * refactored `index.ts`, which upstream owns. Apply behavior changes to both.
 */
import { delimiter, join } from 'node:path'
import { loadLayeredEnv, loadProfileDirectory, reportSkippedBundles } from '@deepseek-ai/dsh-app-boot'
import { runProfile } from '@deepseek-ai/dsh/profile-boot'
import { createHostTransport } from './transport.ts'
import { installDesktopBridge } from './desktop-bridge.ts'
import { installDesktopUpdateTaskControl } from './update-tasks.ts'
import { installDesktopQuitInspection } from './quit-inspection.ts'
import { installPlatformSessionPublisher } from './platform-session.ts'
import { prepareDesktopProfile } from './profile-lifecycle.ts'

/** Control channel chosen once: stdio when the Tauri shell spawned this executable. */
const transport = createHostTransport()

async function main(): Promise<void> {
  const runtimeDir = process.argv[2] as string
  const projectDir = process.argv[3] as string
  const installAnchor = join(runtimeDir, 'node_modules', '@deepseek-ai', 'dsh', 'package.json')
  const preparation = prepareDesktopProfile(projectDir)
  if (preparation.droppedBundles.length > 0) {
    console.error(`dsh: dropped incompatible application bundle(s) from the Desktop profile: ${preparation.droppedBundles.join(', ')}`)
  }
  if (preparation.restoredBundle) {
    console.error('dsh: restored the Web application bundle in the Desktop profile')
  }
  const profile = loadProfileDirectory('dsh', projectDir, installAnchor)
  reportSkippedBundles('dsh', profile)
  const application = runProfile({
    environment: loadLayeredEnv('dsh'),
    profile: 'desktop',
    resolvedProfile: { profile, installAnchor },
    patchFiles: [],
    args: ['--no-open', '--host', '127.0.0.1', '--port', '0'],
    ...(process.argv[5] === undefined ? {} : {
      packageManager: {
        command: process.execPath,
        args: ['--expose-internals', process.argv[5]],
        env: {
          DSH_DESKTOP_NODE_EXECUTABLE: process.execPath,
          PATH: `${process.argv[6] ?? ''}${delimiter}${process.env.PATH ?? ''}`,
        },
      },
    }),
  })
  let stopping: Promise<void> | undefined
  const control: {
    updateTasks?: ReturnType<typeof installDesktopUpdateTaskControl>
    quitInspection?: ReturnType<typeof installDesktopQuitInspection>
    desktopBridge?: () => void
  } = {}
  const stop = (): Promise<void> => stopping ??= (async () => {
    control.desktopBridge?.()
    const running = await application.catch(() => undefined)
    await running?.shutdown.shutdown(0)
    try { await transport.send({ type: 'shutdown-complete' }) } catch (error: unknown) { console.error(error) }
    transport.close()
  })()
  transport.onMessage((message) => {
    if (message.type === 'shutdown') { void stop(); return }
    if (message.type === 'quit-inspection') {
      if (!('requestId' in message) || !Number.isSafeInteger(message.requestId)) return
      const requestId = message.requestId
      void (async () => {
        try {
          if (stopping !== undefined || control.quitInspection === undefined) throw new Error('desktop quit: Host is unavailable')
          const inspection = await control.quitInspection()
          await transport.send({ type: 'quit-inspection', requestId, ...inspection })
        } catch (error) {
          await transport.send({ type: 'quit-inspection', requestId, activeTasks: true, scheduledTasks: false,
            error: error instanceof Error ? error.message : String(error) })
        }
      })().catch((error: unknown) => { console.error(error) })
      return
    }
    if (message.type !== 'update-tasks' || !('requestId' in message) || !Number.isSafeInteger(message.requestId)
      || !('action' in message) || !['inspect', 'lock', 'unlock'].includes(String(message.action))) return
    void (async () => {
      try {
        if (stopping !== undefined || control.updateTasks === undefined) throw new Error('desktop update: Host is unavailable')
        const active = await control.updateTasks(message.action as 'inspect' | 'lock' | 'unlock')
        await transport.send({ type: 'update-tasks', requestId: message.requestId, active })
      } catch (error) {
        await transport.send({ type: 'update-tasks', requestId: message.requestId, active: true,
          error: error instanceof Error ? error.message : String(error) })
      }
    })().catch((error: unknown) => { console.error(error) })
  })
  transport.onDisconnect(() => { void stop() })
  const { ctx } = await application
  control.updateTasks = installDesktopUpdateTaskControl(ctx)
  control.quitInspection = installDesktopQuitInspection(ctx)
  control.desktopBridge = installDesktopBridge(ctx)
  installPlatformSessionPublisher(ctx, (session) => {
    void transport.send({ type: 'platform-session', session }).catch((error: unknown) => { console.error(error) })
  })
  const url = ctx.connection.authenticatedUrl(`${ctx.webServer.protocol}//127.0.0.1:${String(ctx.webServer.port)}`)
  void transport.send({ type: 'ready', url, injections: ctx.webServer.collectIndexInjections() })
    .catch((error: unknown) => { console.error(error) })
}

if (import.meta.main) {
  main().catch(async (error: unknown) => {
    const message = error instanceof Error ? error.message : String(error)
    try { await transport.send({ type: 'fatal', message }) } catch (sendError: unknown) { console.error(sendError) }
    console.error(error)
    process.exitCode = 1
    transport.close()
  })
}
