/**
 * Shared launch for the two Desktop Host entries.
 *
 * The Electron entry (`index.ts`) and the Tauri entry (`tauri.ts`) boot the
 * same `desktop` profile through the same application, and they used to
 * carry that launch body twice. Two copies of the same sequence drift: a
 * change to one of them leaves the other behind, and the carriers then behave
 * differently for no reason a user could name.
 *
 * This module owns the sequence both entries share — loading the profile,
 * taking the lease on the state a desktop carrier owns, booting the profile,
 * answering the shell's shutdown and quit-inspection requests, and releasing
 * the lease — and leaves each entry with what is genuinely its own: the
 * Electron application bundle and its Office plugin, the Tauri shell's update
 * service, Office component, and desktop bridge.
 *
 * @module @deepseek-ai/dsh-desktop-host/desktop-launch
 */

import { join } from 'node:path'
import type { Context } from '@deepseek-ai/cordis'
import { loadLayeredEnv, loadProfileDirectory, reportSkippedBundles } from '@deepseek-ai/dsh-app-boot'
import { runProfile } from '@deepseek-ai/dsh/profile-boot'
import { resolveDshHome } from '@deepseek-ai/dsh-home-paths'
import { acquireDesktopLease } from './desktop-lease.ts'
import type { HostTransport } from './transport.ts'
import type { DesktopQuitInspection } from './quit-inspection.ts'

/** How the booted profile invokes a package manager. */
export interface DesktopPackageManager {
  /** Executable to run. */
  readonly command: string
  /** Arguments after the executable. */
  readonly args: readonly string[]
  /** Environment for that run. */
  readonly env: Readonly<Record<string, string>>
}

/** What the launch needs from its carrier. */
export interface DesktopLaunchOptions {
  /** Control channel the shell reads. */
  readonly transport: HostTransport
  /** Runtime directory the Host was launched with. */
  readonly runtimeDir: string
  /** Profile directory the Host was launched with. */
  readonly projectDir: string
  /** Package manager the booted profile uses; absent when the carrier has none. */
  readonly packageManager?: DesktopPackageManager
  /**
   * Prepare the profile after this Host owns the shared state.
   *
   * A carrier that repairs the profile (the Tauri shell rewrites its bundle
   * list) must not do so while another carrier may be running on the same
   * harness home, so the callback runs after the lease is taken and before
   * the profile is loaded.
   */
  readonly prepareProfile?: (projectDir: string) => void
  /** Carrier-specific cleanup that runs before the profile shuts down. */
  readonly onStopping?: () => void | Promise<void>
  /** Carrier-specific composition that runs once the profile booted. */
  readonly compose?: (ctx: Context) => Promise<void>
  /** Carrier-specific work that runs after the shell learned the profile's URL. */
  readonly onReady?: () => Promise<void>
  /** Quit inspection the shell asks for, resolved when the request arrives. */
  readonly quitInspection?: () => Promise<DesktopQuitInspection> | undefined
}

/** What the launch hands back to its carrier. */
export interface DesktopLaunch {
  /** The booted profile's context. */
  readonly ctx: Context
  /** Stop the profile and release the lease. */
  readonly stop: () => Promise<void>
  /** Whether the profile is already shutting down. */
  readonly stopping: () => boolean
}

/**
 * Boot the Desktop profile for one carrier.
 *
 * @param options - The carrier's transport, directories, and composition.
 * @returns The booted context and the stop path.
 * @throws {DesktopLeaseError} when another desktop carrier owns the state.
 */
export async function launchDesktopProfile(options: DesktopLaunchOptions): Promise<DesktopLaunch> {
  const { transport } = options
  const installAnchor = join(options.runtimeDir, 'node_modules', '@deepseek-ai', 'dsh', 'package.json')
  // The Tauri shell and the Electron application can both be installed; only
  // one Host may own the state they share. The lease comes first: profile
  // preparation writes the same directory another carrier could be reading.
  const lease = acquireDesktopLease(resolveDshHome())
  let profile: ReturnType<typeof loadProfileDirectory>
  try {
    options.prepareProfile?.(options.projectDir)
    profile = loadProfileDirectory('dsh', options.projectDir, installAnchor)
    reportSkippedBundles('dsh', profile)
  } catch (error) {
    lease.release()
    throw error
  }
  const application = runProfile({
    environment: loadLayeredEnv('dsh'),
    profile: 'desktop',
    resolvedProfile: { profile, installAnchor },
    patchFiles: [],
    args: ['--no-open', '--host', '127.0.0.1', '--port', '0'],
    ...options.packageManager === undefined ? {} : { packageManager: options.packageManager },
  })

  let stopping: Promise<void> | undefined
  const stop = (): Promise<void> => stopping ??= (async () => {
    await options.onStopping?.()
    // Startup failure is reported by the entry; shutdown only owns a tree
    // that booted.
    const running = await application.catch(() => undefined)
    await running?.shutdown.shutdown(0)
    try { await transport.send({ type: 'shutdown-complete' }) } catch (error: unknown) { console.error(error) }
    transport.close()
    lease.release()
  })()

  transport.onMessage((message) => {
    if (message.type === 'shutdown') { void stop(); return }
    if (message.type !== 'quit-inspection') return
    if (!('requestId' in message) || !Number.isSafeInteger(message.requestId)) return
    const requestId = message.requestId
    void (async () => {
      try {
        const inspect = options.quitInspection?.()
        if (stopping !== undefined || inspect === undefined) throw new Error('desktop quit: Host is unavailable')
        const inspection = await inspect
        await transport.send({ type: 'quit-inspection', requestId, ...inspection })
      } catch (error) {
        await transport.send({ type: 'quit-inspection', requestId, activeTasks: true, scheduledTasks: false,
          error: error instanceof Error ? error.message : String(error) })
      }
    })().catch((error: unknown) => { console.error(error) })
  })
  transport.onDisconnect(() => { void stop() })

  const { ctx } = await application
  await options.compose?.(ctx)
  const url = ctx.connection.authenticatedUrl(`${ctx.webServer.protocol}//127.0.0.1:${String(ctx.webServer.port)}`)
  void transport.send({ type: 'ready', url, injections: ctx.webServer.collectIndexInjections() })
    .then(async () => { await options.onReady?.() })
    .catch((error: unknown) => { console.error(error) })
  return { ctx, stop, stopping: () => stopping !== undefined }
}
