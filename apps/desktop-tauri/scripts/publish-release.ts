/**
 * Publish a prepared Linux build as a GitHub Release and publish its feed.
 *
 * The Desktop updater and the on-demand Office component both read their
 * inputs from a feed: the updater reads `manifest.json`, and the component
 * installer reads `components.json` beside the archives it names.
 *
 * A release and a feed are two different things. The release holds the
 * artifacts a user downloads, one per target. The feed holds the documents
 * that say which artifact is current, and it moves: a new release overwrites
 * the feed's documents, so the feed URL baked into an installed application
 * keeps pointing at the newest release. Without that split, an application
 * would read the feed of the release it shipped with and never see a
 * successor.
 *
 * The feed is therefore per target, because the Office component archive
 * differs per target, while the release holds every target's artifacts.
 *
 *   usage: publish-release.ts <linux-x64|linux-arm64> --tag <tag> [--feed-tag <tag>] [--dry-run]
 */

import { createHash, createPrivateKey, sign } from 'node:crypto'
import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs'
import { basename, dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawnSync } from 'node:child_process'
import {
  signedComponentBytes,
  signedFeedBytes,
  type SignedComponentDocument,
  type SignedFeedDocument,
} from '../../desktop-host/src/update/signing.ts'

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '../../..')
const target = process.argv[2] ?? 'linux-x64'
const tree = join(repo, 'apps/desktop/.desktop-build/targets', target)

interface Arguments {
  /** Tag of the release holding the artifacts. */
  readonly tag: string
  /** Tag of the release holding the feed documents. */
  readonly feedTag: string
  /** Create the release as a draft, so a rehearsal never goes live. */
  readonly draft: boolean
  readonly dryRun: boolean
}

function parseArguments(): Arguments {
  const tagIndex = process.argv.indexOf('--tag')
  const feedIndex = process.argv.indexOf('--feed-tag')
  const dryRun = process.argv.includes('--dry-run')
  const tag = tagIndex === -1 ? '' : (process.argv[tagIndex + 1] ?? '')
  const feedTag = feedIndex === -1 ? '' : (process.argv[feedIndex + 1] ?? '')
  const draft = process.argv.includes('--draft')
  if (tag === '' && !dryRun) throw new Error('publish-release: --tag <tag> is required')
  return { tag, feedTag: feedTag === '' ? `desktop-feed-${target}` : feedTag, draft, dryRun }
}

/** The version the artifacts were built from. */
function artifactVersion(): string {
  const manifest = JSON.parse(readFileSync(join(repo, 'package.json'), 'utf8')) as { version?: unknown }
  if (typeof manifest.version !== 'string') throw new Error('publish-release: the repository declares no version')
  return manifest.version
}

/** Lowercase SHA-256 of one file. */
function digestOf(path: string): string {
  return createHash('sha256').update(readFileSync(path)).digest('hex')
}

/** The AppImage this release publishes for the target. */
function appImagePath(version: string): string {
  const arch = target.endsWith('-arm64') ? 'aarch64' : 'x86_64'
  const path = join(tree, 'appimage', `DeepSeek-Harness-${version}-${arch}.AppImage`)
  if (!existsSync(path)) throw new Error(`publish-release: missing ${path}; run pack-appimage.sh first`)
  return path
}

/** The update manifest the updater reads, naming this target's artifact. */
function updateManifest(version: string, appImage: string, releaseBase: string, minimumVersion: string | undefined): string {
  const artifacts = {
    [target]: {
      url: `${releaseBase}/${basename(appImage)}`,
      sha256: digestOf(appImage),
      size: statSync(appImage).size,
    },
  }
  const document = {
    schemaVersion: 2,
    version,
    ...(minimumVersion === undefined ? {} : { minimumVersion }),
    artifacts,
  }
  const signature = signFeedDocument(document)
  return `${JSON.stringify({ ...document, signature }, undefined, 2)}\n`
}

/** The component manifest the Office installer reads, beside the archives it names. */
function componentManifest(): string | undefined {
  const published = join(tree, 'components', 'components.json')
  if (!existsSync(published)) return undefined
  const manifest = JSON.parse(readFileSync(published, 'utf8')) as { components?: unknown }
  if (!Array.isArray(manifest.components) || manifest.components.length === 0) {
    throw new Error('publish-release: the component manifest names no archive')
  }
  const document: SignedComponentDocument = { schemaVersion: 1, components: manifest.components }
  const signature = signDocument(signedComponentBytes(document))
  return `${JSON.stringify({ ...document, signature }, undefined, 2)}\n`
}

/**
 * Sign the feed document with the release channel's private key.
 *
 * The key is the channel's secret; the client trusts the matching public
 * key, which the Desktop Host pins. A feed that cannot be signed is not
 * published, because a client that verifies signatures would refuse it.
 */
function signFeedDocument(document: SignedFeedDocument): string {
  // The bytes come from the client's own canonicalization, so the signer and
  // the verifier cannot drift apart.
  return signDocument(signedFeedBytes(document))
}

/** Sign canonical bytes with the release channel's private key. */
function signDocument(bytes: Buffer): string {
  const pem = process.env.DSH_RELEASE_SIGNING_KEY ?? ''
  if (pem === '') throw new Error('publish-release: DSH_RELEASE_SIGNING_KEY is missing')
  const key = createPrivateKey(pem)
  return sign(null, bytes, key).toString('base64')
}

/** The artifacts this target contributes to the shared release. */
function releaseArtifacts(version: string): string[] {
  const artifacts = [appImagePath(version)]
  for (const part of ['deb', 'rpm']) {
    const directory = join(tree, part)
    if (!existsSync(directory)) continue
    for (const name of readdirSync(directory)) {
      if (name.endsWith('.deb') || name.endsWith('.rpm')) artifacts.push(join(directory, name))
    }
  }
  return artifacts
}

/** The component archives this target publishes beside the feed documents. */
function componentArchives(): string[] {
  const directory = join(tree, 'components')
  if (!existsSync(directory)) return []
  return readdirSync(directory)
    .filter(name => name.endsWith('.tar.zst'))
    .map(name => join(directory, name))
}

const { tag, feedTag, draft, dryRun } = parseArguments()
const version = artifactVersion()
const appImage = appImagePath(version)

// The release holds the artifacts; the feed holds the documents that name
// the current one. Both are addressed by tag, and a release tag does not
// move.
const repository = process.env.GITHUB_REPOSITORY ?? 'Masiss/deepseek-harness'
const releaseBase = `https://github.com/${repository}/releases/download/${tag}`
const feedBase = `https://github.com/${repository}/releases/download/${feedTag}`

const feed = join(tree, 'release-feed')
spawnSync('mkdir', ['-p', feed], { stdio: 'inherit' })
const minimumVersion = process.env.DSH_RELEASE_MINIMUM_VERSION
writeFileSync(join(feed, 'manifest.json'), updateManifest(version, appImage, releaseBase, minimumVersion))
const components = componentManifest()
if (components !== undefined) writeFileSync(join(feed, 'components.json'), components)

const artifacts = releaseArtifacts(version)
const archives = componentArchives()

console.log(`publish-release: version=${version} target=${target} tag=${tag} feed_tag=${feedTag}`)
for (const artifact of artifacts) {
  console.log(`publish-release: artifact=${basename(artifact)} bytes=${statSync(artifact).size} sha256=${digestOf(artifact)}`)
}
for (const archive of archives) {
  console.log(`publish-release: component=${basename(archive)} bytes=${statSync(archive).size} sha256=${digestOf(archive)}`)
}
console.log(`publish-release: feed_url=${feedBase}/manifest.json`)

if (dryRun) {
  console.log('publish-release: dry run, nothing uploaded')
  process.exit(0)
}

// The GitHub CLI carries the token; the workflow grants `contents: write` so
// the release and its assets are created by this run. The token arrives
// under the CLI's own name, and the workflow's own name is accepted too.
const token = process.env.GH_TOKEN ?? process.env.GITHUB_TOKEN ?? ''
const env = { ...process.env, GH_TOKEN: token }
if (token === '') throw new Error('publish-release: GITHUB_TOKEN is missing')

/** Whether a release with the given tag exists. */
function releaseExists(releaseTag: string): boolean {
  return spawnSync('gh', ['release', 'view', releaseTag], { stdio: 'ignore', env }).status === 0
}

/** Create a release when it does not exist yet. */
function ensureRelease(releaseTag: string, title: string, notes: string): void {
  if (releaseExists(releaseTag)) return
  const args = ['release', 'create', releaseTag, '--title', title, '--notes', notes]
  if (draft) args.push('--draft')
  const created = spawnSync('gh', args, { stdio: 'inherit', env })
  // Two targets publish into the same release, so the second one finds it
  // already there.
  if (created.status !== 0 && !releaseExists(releaseTag)) {
    throw new Error(`publish-release: gh release create exited with ${String(created.status)}`)
  }
}

ensureRelease(
  tag,
  `DeepSeek Harness Desktop ${version} (${target})`,
  `Linux desktop build: the AppImage, the .deb, and the .rpm for ${target}.`,
)
ensureRelease(
  feedTag,
  `DeepSeek Harness Desktop ${version} feed (${target})`,
  'The update feed: the manifest naming the current artifact and the Office component archive.',
)
// The feed release outlives the release it describes: every publication
// replaces its documents, so its title has to name the version it now carries.
const edited = spawnSync('gh', [
  'release', 'edit', feedTag,
  '--title', `DeepSeek Harness Desktop ${version} feed (${target})`,
  '--notes', 'The update feed: the manifest naming the current artifact and the Office component archive.',
], { stdio: 'inherit', env })
if (edited.status !== 0) throw new Error(`publish-release: gh release edit exited with ${String(edited.status)}`)

// The release's artifacts are immutable once published, so a repeated run
// replaces them rather than failing.
const uploadRelease = spawnSync('gh', ['release', 'upload', tag, '--clobber', ...artifacts], { stdio: 'inherit', env })
if (uploadRelease.status !== 0) throw new Error(`publish-release: gh release upload exited with ${String(uploadRelease.status)}`)

// The feed's documents move with every release, so they are always replaced.
const uploadFeed = spawnSync('gh', [
  'release', 'upload', feedTag, '--clobber',
  join(feed, 'manifest.json'),
  ...(components === undefined ? [] : [join(feed, 'components.json')]),
  ...archives,
], { stdio: 'inherit', env })
if (uploadFeed.status !== 0) throw new Error(`publish-release: gh release upload exited with ${String(uploadFeed.status)}`)

console.log('publish-release: uploaded the artifacts and the feed')
