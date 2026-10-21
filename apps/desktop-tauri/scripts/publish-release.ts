/**
 * Publish a prepared Linux build as a GitHub Release.
 *
 * The Desktop updater and the on-demand Office component both read their
 * inputs from a feed: the updater reads `manifest.json`, and the component
 * installer reads `components.json` beside the archives it names. This script
 * writes both documents from the prepared tree, then attaches the artifacts and
 * the feed to one GitHub Release, so a release is a complete, self-describing
 * download source.
 *
 * The release assets are the feed. A GitHub release download URL is stable for
 * a tag, so the feed URL baked into the artifacts points at the release this
 * script publishes, and a later release moves the feed forward by publishing a
 * new tag.
 *
 * The script never signs anything: the artifacts carry the digest the feed
 * names, and the trust anchor for the feed belongs to the release channel.
 *
 *   usage: publish-release.ts <linux-x64|linux-arm64> [--tag <tag>] [--dry-run]
 */

import { createHash } from 'node:crypto'
import { existsSync, readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs'
import { basename, dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { spawnSync } from 'node:child_process'

const repo = resolve(dirname(fileURLToPath(import.meta.url)), '../../..')
const target = process.argv[2] ?? 'linux-x64'
const tree = join(repo, 'apps/desktop/.desktop-build/targets', target)

interface Arguments {
  readonly tag: string
  readonly dryRun: boolean
}

function parseArguments(): Arguments {
  const tagIndex = process.argv.indexOf('--tag')
  const dryRun = process.argv.includes('--dry-run')
  const tag = tagIndex === -1 ? '' : (process.argv[tagIndex + 1] ?? '')
  if (tag === '' && !dryRun) throw new Error('publish-release: --tag <tag> is required')
  return { tag, dryRun }
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

/** The update manifest the updater reads. */
function updateManifest(version: string, appImage: string, downloadBase: string): string {
  return `${JSON.stringify({
    schemaVersion: 1,
    version,
    artifacts: {
      [target]: {
        url: `${downloadBase}/${basename(appImage)}`,
        sha256: digestOf(appImage),
        size: statSync(appImage).size,
      },
    },
  }, undefined, 2)}\n`
}

/** The component manifest the Office installer reads, beside the archives it names. */
function componentManifest(): string | undefined {
  const published = join(tree, 'components', 'components.json')
  if (!existsSync(published)) return undefined
  const manifest = JSON.parse(readFileSync(published, 'utf8')) as { components?: unknown }
  if (!Array.isArray(manifest.components) || manifest.components.length === 0) {
    throw new Error('publish-release: the component manifest names no archive')
  }
  return `${JSON.stringify(manifest, undefined, 2)}\n`
}

const { tag, dryRun } = parseArguments()
const version = artifactVersion()
const appImage = appImagePath(version)

// The release download base is the feed the artifacts point at. The release
// URL is stable for a tag, so the artifacts published with this tag read the
// feed this release publishes.
const repository = process.env.GITHUB_REPOSITORY ?? 'Masiss/deepseek-harness'
const downloadBase = `https://github.com/${repository}/releases/download/${tag}`

const feed = join(tree, 'release-feed')
spawnSync('mkdir', ['-p', feed], { stdio: 'inherit' })
writeFileSync(join(feed, 'manifest.json'), updateManifest(version, appImage, downloadBase))
const components = componentManifest()
if (components !== undefined) writeFileSync(join(feed, 'components.json'), components)

const componentArchives = existsSync(join(tree, 'components'))
  ? readdirSync(join(tree, 'components')).filter(name => name.endsWith('.tar.zst'))
  : []

console.log(`publish-release: version=${version} target=${target} tag=${tag}`)
console.log(`publish-release: appimage=${basename(appImage)} bytes=${statSync(appImage).size} sha256=${digestOf(appImage)}`)
for (const archive of componentArchives) {
  console.log(`publish-release: component=${archive} bytes=${statSync(join(tree, 'components', archive)).size}`)
}
console.log(`publish-release: feed=${feed} feed_url=${downloadBase}/manifest.json`)

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

const upload = spawnSync('gh', ['release', 'upload', tag, '--clobber', ...[
  appImage,
  join(feed, 'manifest.json'),
  ...(components === undefined ? [] : [join(feed, 'components.json')]),
  ...componentArchives.map(archive => join(tree, 'components', archive)),
]], { cwd: repo, stdio: 'inherit', env })
if (upload.status !== 0) throw new Error(`publish-release: gh release upload exited with ${String(upload.status)}`)

console.log('publish-release: uploaded the artifacts and the feed')
