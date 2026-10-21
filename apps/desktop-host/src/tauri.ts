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
import { resolveDshHome } from '@deepseek-ai/dsh-home-paths'
import { createHostTransport } from './transport.ts'
import { installDesktopBridge } from './desktop-bridge.ts'
import { installDesktopUpdateTaskControl } from './update-tasks.ts'
import { installDesktopQuitInspection } from './quit-inspection.ts'
import { installPlatformSessionPublisher } from './platform-session.ts'
import { prepareDesktopProfile } from './profile-lifecycle.ts'
import { DesktopOfficeComponent } from './components/office.ts'
import * as desktopOffice from './office.ts'
import { createAppImageDriver } from './update/appimage.ts'
import { updateTarget } from './update/manifest.ts'
import { DesktopUpdateService, type DesktopUpdateAdmission } from './update/service.ts'

/** Control channel chosen once: stdio when the Tauri shell spawned this executable. */
const transport = createHostTransport()

/** User decisions sent to the shell and awaited by request id. */
const promptWaiters = new Map<number, (accepted: boolean) => void>()
let nextPromptRequest = 1

/**
 * Deadline for one native dialog.
 *
 * A shell without a visible window (a headless probe, a session without a
 * display) must not leave the update flow waiting forever.
 *
 * @returns Validated delay in milliseconds.
 */
function updatePromptTimeout(): number {
  const raw = process.env.DSH_DESKTOP_UPDATE_PROMPT_TIMEOUT_MS
  const parsed = raw === undefined ? Number.NaN : Number(raw)
  return Number.isSafeInteger(parsed) && parsed > 0 ? parsed : 180_000
}

async function main(): Promise<void> {
  const runtimeDir = process.argv[2] as string
  const projectDir = process.argv[3] as string
  const primaryRuntime = process.argv[4] ?? join(runtimeDir, '..', 'runtime', 'primary-runtime')
  // Office ships as an on-demand component: the Host only knows where the
  // release feed is, and composes the plugin after the user installs it.
  const officeComponent = new DesktopOfficeComponent({
    feed: process.env.DSH_DESKTOP_COMPONENT_FEED ?? '',
    directory: join(resolveDshHome(), 'components'),
    primaryRuntime,
  })
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
    updates?: DesktopUpdateService | undefined
  } = {}
  const stop = (): Promise<void> => stopping ??= (async () => {
    control.desktopBridge?.()
    control.updates?.dispose()
    const running = await application.catch(() => undefined)
    await running?.shutdown.shutdown(0)
    try { await transport.send({ type: 'shutdown-complete' }) } catch (error: unknown) { console.error(error) }
    transport.close()
  })()
  transport.onMessage((message) => {
    if (message.type === 'shutdown') { void stop(); return }
    if (message.type === 'update-prompt') {
      if (typeof message.promptId !== 'number' || !Number.isSafeInteger(message.promptId)) return
      const waiter = promptWaiters.get(message.promptId)
      if (waiter === undefined) return
      promptWaiters.delete(message.promptId)
      waiter(message.accepted === true)
      return
    }
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
  control.updates = createDesktopUpdates(() => control.updateTasks)
  control.desktopBridge = installDesktopBridge(ctx, {
    ...(control.updates === undefined ? {} : { updates: control.updates }),
    components: {
      get presentation() { return officeComponent.presentation },
      subscribe: listener => officeComponent.subscribe(listener),
      install: () => officeComponent.install(),
      restart: () => restartOffice(false),
    },
  })
  installPlatformSessionPublisher(ctx, (session) => {
    void transport.send({ type: 'platform-session', session }).catch((error: unknown) => { console.error(error) })
  })
  /**
   * Ask the shell to relaunch.
   *
   * @param admit - Refuse while a turn runs; the automatic path uses it, the
   * user-requested restart does not.
   * @returns Resolves once the request settled.
   */
  const restartOffice = async (admit: boolean): Promise<void> => {
    if (admit) {
      const decide = control.updateTasks
      if (decide !== undefined && await decide('lock')) {
        await decide('unlock')
        return
      }
    }
    await transport.send({ type: 'restart' })
  }
  const installedOffice = officeComponent.installed()
  if (installedOffice !== undefined) {
    await ctx.plugin(desktopOffice, {
      runtimeDir,
      source: installedOffice.runtime,
      root: installedOffice.runtime,
    })
    officeComponent.markActive()
  } else {
    // A finished install becomes visible only after the Host composes the
    // plugin, so the shell is asked to relaunch — the same restart the plugin
    // installer asks for. An active turn keeps running; the client can ask.
    officeComponent.subscribe((state) => {
      if (!state.installed || state.phase !== 'ready' || state.active) return
      void restartOffice(true).catch((error: unknown) => { console.error(error) })
    })
  }
  const url = ctx.connection.authenticatedUrl(`${ctx.webServer.protocol}//127.0.0.1:${String(ctx.webServer.port)}`)
  void transport.send({ type: 'ready', url, injections: ctx.webServer.collectIndexInjections() })
    .then(async () => {
      // The ordinary check the Electron coordinator schedules at startup; the
      // feed and the installed version are the shell's, not the page's.
      if (control.updates !== undefined) await control.updates.check()
    })
    .catch((error: unknown) => { console.error(error) })
}

/**
 * Build the update service this shell was configured with.
 *
 * Both inputs come from the shell: a release without a feed URL has no update
 * source, and a release that names no version cannot compare one. A target
 * this build cannot publish for (or a platform without an installer driver)
 * disables updates instead of failing the Host.
 *
 * @param taskControl - Task admission owned by the Booted Host context.
 * @returns The service, or undefined when this installation cannot update itself.
 */
function createDesktopUpdates(
  taskControl: () => ReturnType<typeof installDesktopUpdateTaskControl> | undefined,
): DesktopUpdateService | undefined {
  const feed = process.env.DSH_DESKTOP_UPDATE_FEED
  const currentVersion = process.env.DSH_DESKTOP_VERSION
  if (feed === undefined || feed === '' || currentVersion === undefined || currentVersion === '') return undefined
  try {
    return new DesktopUpdateService({
      feed,
      // The pinned key is the trust anchor; an operator may point the Host
      // at another channel's key.
      ...(process.env.DSH_DESKTOP_UPDATE_PUBLIC_KEY === undefined
        || process.env.DSH_DESKTOP_UPDATE_PUBLIC_KEY === ''
        ? {}
        : { publicKey: process.env.DSH_DESKTOP_UPDATE_PUBLIC_KEY }),
      currentVersion,
      target: updateTarget(process.platform, process.arch),
      directory: join(resolveDshHome(), 'updates'),
      driver: createAppImageDriver(),
      confirm: prompt => new Promise<boolean>((resolve) => {
        const promptId = nextPromptRequest++
        const timer = setTimeout(() => {
          promptWaiters.delete(promptId)
          resolve(false)
        }, updatePromptTimeout())
        promptWaiters.set(promptId, (accepted) => {
          clearTimeout(timer)
          resolve(accepted)
        })
        void transport.send({ type: 'update-prompt', promptId, action: prompt.action, version: prompt.version })
          .catch((error: unknown) => {
            clearTimeout(timer)
            promptWaiters.delete(promptId)
            console.error(error)
            resolve(false)
          })
      }),
      restart: async () => { await transport.send({ type: 'restart' }) },
      beforeInstall: async (): Promise<DesktopUpdateAdmission> => {
        const decide = taskControl()
        if (decide === undefined) return 'unavailable'
        try {
          // Refusing new requests before the final check is what keeps an
          // install from cutting a running turn in half.
          const active = await decide('lock')
          if (!active) return 'proceed'
          await decide('unlock')
          return 'tasks-active'
        } catch (error: unknown) {
          console.error(error)
          return 'unavailable'
        }
      },
    })
  } catch (error: unknown) {
    console.error(error)
    return undefined
  }
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
