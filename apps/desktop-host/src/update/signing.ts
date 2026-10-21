/**
 * The trust anchor of the Desktop update feed.
 *
 * A feed says which artifact is current and where to fetch it. Without a
 * signature, anyone who can answer the feed URL decides what a user
 * installs. The feed therefore carries a signature over the documents that
 * name the artifact, and the client verifies it against the public key
 * pinned here before it trusts a word of the manifest.
 *
 * The private key lives in the release channel's secret store; it is never
 * in the repository. The public key below is the only trust the client
 * needs, and changing it is a deliberate act in this file.
 *
 * @module @deepseek-ai/dsh-desktop-host/update/signing
 */

import { createPublicKey, verify } from 'node:crypto'

/**
 * The Ed25519 public key that signs the feed.
 *
 * A release channel that signs with another key is not read by this
 * client: the pinned key is the contract.
 */
export const UPDATE_FEED_PUBLIC_KEY = `-----BEGIN PUBLIC KEY-----
MCowBQYDK2VwAyEA8nRQMATSaQtVJXuAN8aVNweEc2PkVcAh5jwuhYRJ2ss=
-----END PUBLIC KEY-----
`

/** The document a signature covers, as the signer and the verifier agree on it. */
export interface SignedFeedDocument {
  readonly schemaVersion: number
  readonly version: string
  readonly minimumVersion?: string
  readonly artifacts: Readonly<Record<string, unknown>>
}

/** Why a feed signature did not verify. */
export type UpdateSignatureFailure =
  | 'missing-signature'
  | 'malformed-signature'
  | 'unsupported-key'
  | 'invalid-signature'

/** A signature that did not verify, with the reason. */
export class UpdateSignatureError extends Error {
  /**
   * @param failure - Fixed classification of the failure.
   * @param detail - What the verifier saw.
   */
  constructor(readonly failure: UpdateSignatureFailure, detail: string) {
    super(`desktop update signature: ${failure} (${detail})`)
    this.name = 'UpdateSignatureError'
  }
}

/**
 * The exact bytes a signature covers.
 *
 * The signer and the verifier build the document the same way, so the
 * signature is over the manifest without its own signature field, with its
 * keys in a fixed order.
 * @param document - The manifest's signed fields.
 * @returns The canonical bytes to sign or verify.
 */
export function signedFeedBytes(document: SignedFeedDocument): Buffer {
  const canonical = {
    schemaVersion: document.schemaVersion,
    version: document.version,
    ...(document.minimumVersion === undefined ? {} : { minimumVersion: document.minimumVersion }),
    artifacts: Object.fromEntries(Object.entries(document.artifacts).sort(([left], [right]) => left.localeCompare(right))),
  }
  return Buffer.from(JSON.stringify(canonical), 'utf8')
}

/**
 * Verify a feed's signature against a public key.
 *
 * The key defaults to the pinned one, which is the trust anchor a client
 * uses. A caller may pass another key to check a document against it, which
 * is how the tests exercise the canonicalization without the channel's
 * private key.
 *
 * @param document - The manifest's signed fields.
 * @param signature - The base64 signature the manifest carries.
 * @param publicKey - The PEM public key to verify against.
 * @throws {UpdateSignatureError} when the signature is absent, malformed, or does not verify.
 */
export function verifyUpdateFeedSignature(
  document: SignedFeedDocument,
  signature: string | undefined,
  publicKey: string = UPDATE_FEED_PUBLIC_KEY,
): void {
  if (signature === undefined || signature === '') {
    throw new UpdateSignatureError('missing-signature', 'the manifest carries no signature')
  }
  let key: ReturnType<typeof createPublicKey>
  try {
    key = createPublicKey(publicKey)
  } catch (error) {
    throw new UpdateSignatureError('unsupported-key', error instanceof Error ? error.message : String(error))
  }
  let bytes: Buffer
  try {
    bytes = Buffer.from(signature, 'base64')
  } catch (error) {
    throw new UpdateSignatureError('malformed-signature', error instanceof Error ? error.message : String(error))
  }
  const verified = verify(null, signedFeedBytes(document), key, bytes)
  if (!verified) throw new UpdateSignatureError('invalid-signature', 'the signature does not match the manifest')
}
