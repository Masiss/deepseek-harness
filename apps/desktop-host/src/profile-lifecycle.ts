/**
 * Profile lifecycle for the Tauri Desktop Host.
 *
 * The Electron shell prepares its profile in the main process before it starts
 * the Host (`DesktopProjectManager.applyRelease`) and can recover a profile
 * that no longer boots (`disableAllPlugins`). This module gives the Tauri Host
 * the same guarantees without a second owner:
 *
 * - ensure the profile exists and drop link projections left by development;
 * - keep exactly one application bundle, the Web one the Desktop Host composes
 *   its command line against. Installing a second application bundle (headless,
 *   ACP, SDK) replaces the other app's command surface, so the Host's
 *   `--no-open`/`--host` flags stop existing and the next start cannot compose.
 *
 * Feature bundles are never removed: they insert rows over the Web application
 * instead of replacing it, which is the supported way to extend a profile.
 *
 * Only one Host runs per profile: a carrier enforces single-instance startup
 * for itself, and the lease in `desktop-lease.ts` keeps the other carrier out
 * of the state they share, so this module does not take the Electron profile
 * lock.
 */

import {
  initProfile,
  PROFILE_TEMPLATES,
  readProfileManifest,
  removeLinkProjections,
  writeProfileBundles,
  type ProfileTemplate,
} from '@deepseek-ai/dsh-app-boot'

/** Bundles the Desktop Host boots the Web application with. */
export function desktopProfileBundles(): readonly string[] {
  const template: ProfileTemplate | undefined = PROFILE_TEMPLATES.web
  if (template === undefined) throw new Error('desktop profile: the web profile template is missing')
  return template.bundles
}

/**
 * Packages that replace the application surface instead of adding to it.
 *
 * `@deepseek-ai/dsh-base` is a layer, not an application, so it is absent here.
 */
export const APPLICATION_BUNDLES: readonly string[] = [
  '@deepseek-ai/dsh-acp-app',
  '@deepseek-ai/dsh-headless',
  '@deepseek-ai/dsh-sdk-app',
  '@deepseek-ai/dsh-sdk-minimal',
  '@deepseek-ai/dsh-web-app',
]

/** Application bundle the Desktop Host drives. */
const DESKTOP_APPLICATION_BUNDLE = '@deepseek-ai/dsh-web-app'

/** What preparing the profile changed, for diagnostics. */
export interface DesktopProfilePreparation {
  /** Application bundles dropped because the Desktop Host cannot compose them. */
  readonly droppedBundles: readonly string[]
  /** Whether the Web application bundle had to be restored. */
  readonly restoredBundle: boolean
}

/**
 * Create or repair the Desktop profile before the Host loads it.
 *
 * @param profileDir - Profile directory the Host owns.
 * @returns the bundles dropped or restored, both empty for a compatible profile.
 * @throws when the profile manifest exists but cannot be read.
 */
export function prepareDesktopProfile(profileDir: string): DesktopProfilePreparation {
  const bundles = desktopProfileBundles()
  initProfile(profileDir, bundles)
  removeLinkProjections(profileDir)

  const manifest = readProfileManifest('dsh', profileDir)
  const current = manifest.dsh?.profile?.bundles ?? []
  const applications = current.filter(name => APPLICATION_BUNDLES.includes(name))
  const droppedBundles = applications.filter(name => name !== DESKTOP_APPLICATION_BUNDLE)
  const restoredBundle = !applications.includes(DESKTOP_APPLICATION_BUNDLE)
  if (droppedBundles.length === 0 && !restoredBundle) return { droppedBundles, restoredBundle }

  // Keep every non-application bundle in place: they compose over the Web app.
  const kept = current.filter(name => !droppedBundles.includes(name))
  const repaired = restoredBundle ? [...kept, DESKTOP_APPLICATION_BUNDLE] : kept
  writeProfileBundles(profileDir, manifest, repaired)
  return { droppedBundles, restoredBundle }
}
