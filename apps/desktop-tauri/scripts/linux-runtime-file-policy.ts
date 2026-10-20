/**
 * Linux-only omissions from the installed production runtime.
 *
 * The Desktop runtime reaches the terminal, the session lease, and the
 * one-shot process bootstrap through the Rust provider in `native/dsh-native`,
 * so the payload needs no FFI library, no C++ addon, and no C flock addon. The
 * originals stay in the payload unless the Rust provider is present, so a
 * payload built without the module keeps the bundled implementations and the
 * loader hook keeps its fallback.
 *
 * This is a fork-owned step for the Linux lane; the Electron payload keeps its
 * full tree.
 */

import { existsSync } from 'node:fs'
import { join } from 'node:path'

/** Where the Rust provider lands in the installed runtime. */
export const RUST_PROVIDER_PATH = 'native/dsh-native.node'

/** Whether the installed runtime carries the Rust provider. */
export function rustProviderPresent(nodeModules: string): boolean {
  return existsSync(join(nodeModules, '@deepseek-ai', 'dsh-desktop-host', ...RUST_PROVIDER_PATH.split('/')))
}

/**
 * Identify files the Linux Desktop runtime omits.
 * @param path - Path relative to the production node_modules directory.
 * @param options - Target platform and whether the Rust provider is installed.
 * @returns Omission reason, or undefined when the entry must be copied.
 */
export function linuxRuntimeFileExclusion(
  path: string,
  options: { readonly platform: NodeJS.Platform; readonly rustProvider: boolean },
): string | undefined {
  if (options.platform !== 'linux') return undefined
  const parts = path.split(/[\\/]/u)
  const packageParts = parts.slice(parts.lastIndexOf('node_modules') + 1)
  const nameParts = packageParts[0]?.startsWith('@') ? 2 : 1
  const name = packageParts.slice(0, nameParts).join('/')
  const entry = packageParts.slice(nameParts).join('/')
  // The Linux one-shot process bootstrap binds libc through koffi, so the FFI
  // library stays in the payload until the Rust provider replaces it.
  if (name === 'koffi' && options.rustProvider) return 'Koffi FFI library replaced by the Rust provider'
  if (name === 'node-pty' && options.rustProvider) return 'node-pty addon replaced by the Rust provider'
  if (name.startsWith('@deepseek-ai/node-addon-system-') && /^bin\/(?:glibc|musl)\/system\.node$/u.test(entry)) {
    return 'C flock addon replaced by the Rust provider'
  }
  return undefined
}
