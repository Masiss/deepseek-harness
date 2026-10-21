/**
 * Validation for the on-demand component manifest published beside a release.
 *
 * The Desktop bundle carries the Host, the dsh runtime, and a seed profile.
 * Components a user may never open (Office authoring) ship separately, so the
 * manifest is the only description of what the Host may download: it names the
 * file, its exact byte length, and the digest the Host verifies before touching
 * the filesystem.
 *
 * @module @deepseek-ai/dsh-desktop-host/components/manifest
 */

/** Component identifiers this Host knows how to install. */
export type DesktopComponentId = 'office'

/** One downloadable component artifact. */
export interface DesktopComponentArtifact {
  /** Component this artifact supplies. */
  readonly id: DesktopComponentId
  /** Component version the archive contains. */
  readonly version: string
  /** Archive file name, resolved against the feed directory. */
  readonly file: string
  /** Exact archive length in bytes. */
  readonly bytes: number
  /** Lowercase SHA-256 of the archive bytes. */
  readonly sha256: string
}

/** Manifest served at `<feed>/components.json`. */
export interface DesktopComponentsManifest {
  /** Format version; only 1 is accepted. */
  readonly schemaVersion: 1
  /** Every artifact this release publishes. */
  readonly components: readonly DesktopComponentArtifact[]
}

const ID = /^[a-z][a-z0-9-]*$/u
const VERSION = /^\d+\.\d+\.\d+(?:[-+][\w.-]+)?$/u
const DIGEST = /^[a-f0-9]{64}$/u

/**
 * Largest archive a manifest may declare.
 *
 * The Office component ships a Python runtime and the Office engine, so the
 * ceiling leaves room for it. The bound matters because the declared length is
 * what the download writes before any digest is compared, and because the
 * archive expands further than it arrives.
 */
export const MAX_COMPONENT_ARTIFACT_BYTES = 1024 * 1024 * 1024

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/**
 * Validate a decoded component manifest.
 *
 * @param value - Parsed JSON from the feed; untrusted.
 * @returns The manifest with exactly the fields the installer uses.
 * @throws When the format, an identifier, a version, a file name, or a digest is invalid.
 */
export function parseComponentsManifest(value: unknown): DesktopComponentsManifest {
  if (!isRecord(value) || value.schemaVersion !== 1 || !Array.isArray(value.components)) {
    throw new Error('desktop components: invalid manifest')
  }
  const seen = new Set<string>()
  const components = value.components.map((entry: unknown): DesktopComponentArtifact => {
    if (!isRecord(entry)) throw new Error('desktop components: invalid manifest')
    const { id, version, file, bytes, sha256 } = entry
    if (typeof id !== 'string' || !ID.test(id)
      || typeof version !== 'string' || !VERSION.test(version)
      || typeof file !== 'string' || file.length === 0 || file.includes('/') || file === '.' || file === '..'
      || typeof bytes !== 'number' || !Number.isSafeInteger(bytes) || bytes <= 0 || bytes > MAX_COMPONENT_ARTIFACT_BYTES
      || typeof sha256 !== 'string' || !DIGEST.test(sha256)) {
      throw new Error('desktop components: invalid manifest')
    }
    if (seen.has(id)) throw new Error(`desktop components: duplicate component ${id}`)
    seen.add(id)
    return { id: id as DesktopComponentId, version, file, bytes, sha256 }
  })
  return { schemaVersion: 1, components }
}
