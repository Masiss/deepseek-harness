#!/usr/bin/env bash
# Compare the Rust image provider with the sharp package it replaces.
#
# The probe runs the same operations on both providers: the intrinsic facts of
# a raster, the normalized 8-bit sRGB output, and the encoded sizes. It
# reports both results side by side and fails when the facts differ.
#
#   usage: image-parity-probe.sh [linux-x64|linux-arm64]
set -u

repo=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)
target=${1:-linux-x64}
tree="$repo/apps/desktop/.desktop-build/targets/$target"
shim="$tree/dsh/node_modules/@deepseek-ai/dsh-desktop-host/native/sharp.cjs"
[ -f "$shim" ] || { echo "RESULT shim=missing"; exit 1; }

script="$tree/dsh/probe/image-parity-probe.mjs"
work="$tree/probe/image-parity"
mkdir -p "$(dirname "$script")" "$work/home" "$work/tmp"

cat > "$script" <<'PROBE'
import { createRequire } from 'node:module'
import { mkdirSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

const require = createRequire(import.meta.url)
const shim = require(process.env.DSH_IMAGE_SHIM)
const sharp = require(process.env.DSH_IMAGE_SHARP)

const dir = join(tmpdir(), 'dsh-image-parity')
mkdirSync(dir, { recursive: true })

/** Deterministic noise: incompressible, so encoders show their real size. */
function noise(width, height, channels) {
  const pixels = Buffer.alloc(width * height * channels)
  let state = 0x2545f491
  for (let index = 0; index < pixels.length; index += 1) {
    state ^= state << 13
    state ^= state >>> 17
    state ^= state << 5
    pixels[index] = state & 0xff
  }
  return pixels
}

/** One raster in one format, built by the real sharp package. */
async function raster(format, width, height, channels) {
  const pixels = noise(width, height, channels)
  const data = await sharp(pixels, { raw: { width, height, channels } })
    .toFormat(format)
    .toBuffer()
  return Buffer.from(data)
}

const fixtures = [
  { name: 'png-rgb', format: 'png', width: 64, height: 48, channels: 3 },
  { name: 'png-rgba', format: 'png', width: 64, height: 48, channels: 4 },
  { name: 'jpeg-rgb', format: 'jpeg', width: 64, height: 48, channels: 3 },
  { name: 'webp-rgb', format: 'webp', width: 64, height: 48, channels: 3 },
  { name: 'gif-rgb', format: 'gif', width: 32, height: 32, channels: 3 },
]

let failures = 0
for (const fixture of fixtures) {
  const source = await raster(fixture.format, fixture.width, fixture.height, fixture.channels)
  const sharpFacts = await sharp(source).metadata()
  const rustFacts = await shim(source).metadata()
  const factsMatch = sharpFacts.width === rustFacts.width
    && sharpFacts.height === rustFacts.height
    && sharpFacts.format === rustFacts.format
    && sharpFacts.depth === rustFacts.depth
    && sharpFacts.space === rustFacts.space
    && sharpFacts.hasAlpha === rustFacts.hasAlpha
    && (sharpFacts.pages ?? 1) === (rustFacts.pages ?? 1)
  const sharpWebp = (await sharp(source).rotate().toColourspace('srgb').resize({ width: 32, height: 32, fit: 'inside', withoutEnlargement: true }).webp({ quality: 85, effort: 0 }).toBuffer()).length
  const rustWebp = (await shim(source).rotate().toColourspace('srgb').resize({ width: 32, height: 32, fit: 'inside', withoutEnlargement: true }).webp({ quality: 85, effort: 0 }).toBuffer()).length
  const ratio = rustWebp / sharpWebp
  const sizeClose = ratio > 0.6 && ratio < 1.7
  if (!factsMatch || !sizeClose) failures += 1
  console.log(
    `RESULT ${fixture.name} facts=${factsMatch ? 'same' : 'differ'} `
    + `sharp=${JSON.stringify({
      format: sharpFacts.format, width: sharpFacts.width, height: sharpFacts.height,
      depth: sharpFacts.depth, space: sharpFacts.space, hasAlpha: sharpFacts.hasAlpha,
      pages: sharpFacts.pages ?? 1,
    })} rust=${JSON.stringify({
      format: rustFacts.format, width: rustFacts.width, height: rustFacts.height,
      depth: rustFacts.depth, space: rustFacts.space, hasAlpha: rustFacts.hasAlpha,
      pages: rustFacts.pages ?? 1,
    })} webp_bytes=${sharpWebp}/${rustWebp} ratio=${ratio.toFixed(2)}`,
  )
}
console.log(`RESULT failures=${failures} delegated=${shim.delegatedCount()}`)
writeFileSync(join(dir, 'result.txt'), `${failures}\n`)
if (failures > 0) process.exitCode = 1
PROBE

DSH_ISOLATION_DIR="$work" "$repo/apps/desktop-tauri/scripts/isolated-run.sh" \
  env DSH_IMAGE_SHIM="$shim" \
  DSH_IMAGE_SHARP="$repo/packages/attachment/attachment-local/node_modules/sharp" \
  "$tree/runtime/bin/node" "$script"
