/**
 * Release manifest for the Desktop updater.
 *
 * One JSON document lists a published version's artifacts keyed by the exact
 * target a shell runs, so a client can never pick an artifact for another
 * platform or architecture. The manifest names bytes and their digest; it does
 * not authorize an install and carries no user-facing copy.
 *
 * @module @deepseek-ai/dsh-desktop-host/update/manifest
 */

import { valid } from 'semver'
import { UpdateSignatureError, verifyUpdateFeedSignature } from './signing.ts'

/** Targets a Desktop artifact can be published for, named `platform-arch`. */
export const UPDATE_TARGETS = [
  'linux-x64', 'linux-arm64',
  'darwin-x64', 'darwin-arm64',
  'win32-x64', 'win32-arm64',
] as const

/** One publishable target. */
export type UpdateTarget = typeof UPDATE_TARGETS[number]

/** Downloadable bytes for one target. */
export interface UpdateArtifact {
  /** Absolute HTTPS URL; loopback HTTP is accepted so a release can be rehearsed locally. */
  readonly url: string
  /** Lowercase SHA-256 of the complete artifact. */
  readonly sha256: string
  /** Exact artifact length in bytes. */
  readonly size: number
}

/** One published release. */
export interface UpdateManifest {
  /** Documents this shape; a reader refuses anything it does not know. */
  readonly schemaVersion: 2
  /** Released version, bare semver including any prerelease. */
  readonly version: string
  /** Oldest client version allowed to install this release; absent means any. */
  readonly minimumVersion?: string
  /** Base64 Ed25519 signature over the manifest's signed fields. */
  readonly signature?: string
  /** Artifacts by target; a target omitted here is not published for this release. */
  readonly artifacts: Readonly<Partial<Record<UpdateTarget, UpdateArtifact>>>
}

/** Why a manifest was refused. */
export type UpdateManifestFailure =
  | 'not-an-object'
  | 'unsupported-schema'
  | 'invalid-version'
  | 'invalid-minimum-version'
  | 'missing-artifacts'
  | 'invalid-artifact'
  | 'unsupported-target'
  | 'missing-artifact'
  | 'unsigned'
  | 'invalid-signature'

/** A manifest that cannot be used, with a fixed classification and a locating detail. */
export class UpdateManifestError extends Error {
  /**
   * @param failure - Fixed classification the UI and journal may report.
   * @param detail - Field path or target that failed validation.
   */
  constructor(readonly failure: UpdateManifestFailure, readonly detail: string) {
    super(`desktop update manifest: ${failure} (${detail})`)
    this.name = 'UpdateManifestError'
  }
}

/** A JSON object with string keys, as far as a parser can tell. */
function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** Whether a URL is downloadable: HTTPS anywhere, HTTP only on the loopback interface. */
function isDownloadUrl(value: string): boolean {
  let url: URL
  try {
    url = new URL(value)
  } catch {
    return false
  }
  if (url.protocol === 'https:') return true
  if (url.protocol !== 'http:') return false
  return url.hostname === '127.0.0.1' || url.hostname === '::1' || url.hostname === 'localhost'
}

/** Whether a value names a known target. */
function isUpdateTarget(value: string): value is UpdateTarget {
  return (UPDATE_TARGETS as readonly string[]).includes(value)
}

/**
 * Parse one published manifest.
 *
 * Unknown target keys are ignored: a newer release may publish a target this
 * client does not run, and that must not make its own artifact unusable.
 *
 * @param input - Parsed JSON, not a string.
 * @param publicKey - The PEM public key to verify the signature against; the pinned one by default.
 * @returns The validated manifest.
 * @throws {UpdateManifestError} when the document or any known artifact is malformed.
 */
export function parseUpdateManifest(input: unknown, publicKey?: string): UpdateManifest {
  if (!isRecord(input)) throw new UpdateManifestError('not-an-object', '$')
  if (input.schemaVersion !== 2) throw new UpdateManifestError('unsupported-schema', 'schemaVersion')
  if (typeof input.version !== 'string' || valid(input.version) === null) {
    throw new UpdateManifestError('invalid-version', 'version')
  }
  if (input.minimumVersion !== undefined
    && (typeof input.minimumVersion !== 'string' || valid(input.minimumVersion) === null)) {
    throw new UpdateManifestError('invalid-minimum-version', 'minimumVersion')
  }
  if (!isRecord(input.artifacts)) throw new UpdateManifestError('missing-artifacts', 'artifacts')

  const artifacts: Partial<Record<UpdateTarget, UpdateArtifact>> = {}
  for (const [target, raw] of Object.entries(input.artifacts)) {
    if (!isUpdateTarget(target)) continue
    if (!isRecord(raw)) throw new UpdateManifestError('invalid-artifact', `artifacts.${target}`)
    if (typeof raw.url !== 'string' || !isDownloadUrl(raw.url)) {
      throw new UpdateManifestError('invalid-artifact', `artifacts.${target}.url`)
    }
    if (typeof raw.sha256 !== 'string' || !/^[0-9a-f]{64}$/u.test(raw.sha256)) {
      throw new UpdateManifestError('invalid-artifact', `artifacts.${target}.sha256`)
    }
    if (typeof raw.size !== 'number' || !Number.isSafeInteger(raw.size) || raw.size <= 0) {
      throw new UpdateManifestError('invalid-artifact', `artifacts.${target}.size`)
    }
    artifacts[target] = { url: raw.url, sha256: raw.sha256, size: raw.size }
  }
  if (Object.keys(artifacts).length === 0) throw new UpdateManifestError('missing-artifacts', 'artifacts')

  // The signature covers the documents that name the artifact, so it is
  // checked before the manifest is returned to a caller that may act on it.
  const document = {
    schemaVersion: 2,
    version: input.version,
    ...(input.minimumVersion === undefined ? {} : { minimumVersion: input.minimumVersion }),
    artifacts: input.artifacts,
  }
  try {
    verifyUpdateFeedSignature(document, typeof input.signature === 'string' ? input.signature : undefined, publicKey)
  } catch (error) {
    if (error instanceof UpdateSignatureError) {
      throw new UpdateManifestError(error.failure === 'missing-signature' ? 'unsigned' : 'invalid-signature', error.failure)
    }
    throw error
  }

  return {
    schemaVersion: 2,
    version: input.version,
    ...(typeof input.minimumVersion === 'string' ? { minimumVersion: input.minimumVersion } : {}),
    ...(typeof input.signature === 'string' ? { signature: input.signature } : {}),
    artifacts,
  }
}

/**
 * Pick the artifact this shell can install.
 *
 * @param manifest - Validated manifest.
 * @param target - Target the running shell reports.
 * @returns The artifact published for that target.
 * @throws {UpdateManifestError} when the release publishes no artifact for it.
 */
export function selectUpdateArtifact(manifest: UpdateManifest, target: UpdateTarget): UpdateArtifact {
  const artifact = manifest.artifacts[target]
  if (artifact === undefined) throw new UpdateManifestError('missing-artifact', target)
  return artifact
}

/**
 * Name the publishable target that matches a running process.
 *
 * @param platform - `process.platform`.
 * @param arch - `process.arch`.
 * @returns The matching target.
 * @throws {UpdateManifestError} for a platform or architecture with no Desktop artifact.
 */
export function updateTarget(platform: NodeJS.Platform, arch: string): UpdateTarget {
  const candidate = `${platform}-${arch}`
  if (!isUpdateTarget(candidate)) throw new UpdateManifestError('unsupported-target', candidate)
  return candidate
}
