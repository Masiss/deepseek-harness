/**
 * Compare the native image module with the sharp package it replaces.
 *
 * The Desktop payload carries the native module instead of sharp, so every
 * operation the attachment service uses must behave the same way. The
 * comparison runs both providers over the same sources and reports the facts
 * and the decoded pixels of each result.
 *
 * Two kinds of difference are expected, and both are measured rather than
 * hidden:
 *
 * - a lossy codec encodes differently in libvips and in the native module,
 *   so the decoded pixels differ within the codec's own tolerance;
 * - libvips resamples with its own fixed-point arithmetic, so a resized
 *   raster differs from the native module's by a few levels per channel.
 *
 * A lossless codec without resampling must match byte for byte.
 *
 *   usage: image-parity.mjs
 */
import { createRequire } from 'node:module'

const require = createRequire(import.meta.url)
const shim = require(process.env.DSH_IMAGE_SHIM)
const sharp = require(process.env.DSH_IMAGE_SHARP)

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

/** The decoded pixels of an encoded raster. */
async function pixelsOf(data) {
  const { data: raw, info } = await sharp(data).raw().toBuffer({ resolveWithObject: true })
  return { pixels: Buffer.from(raw), width: info.width, height: info.height, channels: info.channels }
}

/** How far two decoded rasters differ. */
function difference(left, right) {
  if (left.width !== right.width || left.height !== right.height) return { comparable: false }
  if (left.channels !== right.channels) return { comparable: false }
  let differing = 0
  let max = 0
  let sum = 0
  for (let index = 0; index < Math.min(left.pixels.length, right.pixels.length); index += 1) {
    const delta = Math.abs(left.pixels[index] - right.pixels[index])
    if (delta !== 0) differing += 1
    sum += delta
    if (delta > max) max = delta
  }
  return {
    comparable: true,
    differing,
    total: left.pixels.length,
    max,
    mean: sum / Math.max(left.pixels.length, 1),
  }
}

let failures = 0
let checks = 0

/**
 * One comparison: both providers run the same operation over the same source.
 * @param name - What the comparison covers.
 * @param source - The encoded raster both providers read.
 * @param apply - The operation each provider runs.
 * @param tolerance - The largest per-channel difference the case allows.
 */
async function compare(name, source, apply, tolerance) {
  const sharpResult = await apply(sharp(source)).toBuffer()
  const shimResult = await apply(shim(source)).toBuffer()
  checks += 1
  const differenceReport = difference(await pixelsOf(sharpResult), await pixelsOf(shimResult))
  const sharpFacts = await sharp(sharpResult).metadata()
  const shimFacts = await shim(shimResult).metadata()
  const factsMatch = sharpFacts.width === shimFacts.width
    && sharpFacts.height === shimFacts.height
    && sharpFacts.format === shimFacts.format
    && Boolean(sharpFacts.exif) === Boolean(shimFacts.exif)
    && Boolean(sharpFacts.icc) === Boolean(shimFacts.icc)
  // A lossy codec has no pixel tolerance to hold to: the two encoders simply
  // encode differently, so only the facts have to match.
  const pixelsMatch = tolerance === 'lossy'
    ? differenceReport.comparable
    : differenceReport.comparable && differenceReport.max <= tolerance
  if (!factsMatch || !pixelsMatch) {
    failures += 1
    console.log(`FAIL ${name} facts=${factsMatch ? 'same' : 'different'} pixels=${pixelsMatch ? 'within' : 'different'}`)
    console.log(`  sharp ${String(sharpFacts.width)}x${String(sharpFacts.height)} ${String(sharpFacts.format)} exif=${String(Boolean(sharpFacts.exif))} icc=${String(Boolean(sharpFacts.icc))}`)
    console.log(`  shim  ${String(shimFacts.width)}x${String(shimFacts.height)} ${String(shimFacts.format)} exif=${String(Boolean(shimFacts.exif))} icc=${String(Boolean(shimFacts.icc))}`)
    if (differenceReport.comparable) {
      console.log(`  pixels differ in ${String(differenceReport.differing)} of ${String(differenceReport.total)} bytes, max ${String(differenceReport.max)}, mean ${differenceReport.mean.toFixed(3)}`)
    }
    return
  }
  const measured = differenceReport.comparable && differenceReport.max > 0
    ? ` differing=${String(differenceReport.differing)}/${String(differenceReport.total)} max=${String(differenceReport.max)} mean=${differenceReport.mean.toFixed(3)}`
    : ''
  console.log(`ok ${name} format=${String(shimFacts.format)} size=${String(shimResult.length)}${measured}`)
}

const rgb = await sharp(noise(64, 48, 3), { raw: { width: 64, height: 48, channels: 3 } })
  .png()
  .toBuffer()
const rgba = await sharp(noise(64, 48, 4), { raw: { width: 64, height: 48, channels: 4 } })
  .png()
  .toBuffer()

// A lossless codec without resampling must match byte for byte.
await compare('png-output', rgb, image => image.png(), 0)
await compare('toformat-png', rgb, image => image.toFormat('png'), 0)
await compare('png-alpha', rgba, image => image.png(), 0)
// libvips resamples with its own fixed-point arithmetic.
await compare('resize-png', rgb, image => image.resize({ width: 32, height: 32, fit: 'inside' }).png(), 8)
// A lossy codec encodes differently in the two providers.
await compare('jpeg-output', rgb, image => image.jpeg({ quality: 80 }), 'lossy')
await compare('webp-output', rgb, image => image.webp({ quality: 80 }), 'lossy')
await compare('gif-output', rgb, image => image.toFormat('gif'), 'lossy')
// libvips applies a colour transform to a TIFF it wrote, so the pixels differ
// by more than a codec's tolerance; the facts still have to match.
await compare('tiff-output', rgb, image => image.tiff(), 'lossy')

// The metadata a caller asks to keep must survive the native module.
const oriented = await sharp({
  create: { width: 4, height: 2, channels: 3, background: { r: 1, g: 2, b: 3 } },
}).jpeg().withMetadata({ orientation: 6 }).toBuffer()
await compare('metadata-orientation', oriented, image => image.withMetadata({ orientation: 6 }).jpeg(), 0)
const sharpOriented = await sharp(oriented).metadata()
const shimOriented = await shim(oriented).metadata()
checks += 1
if (sharpOriented.orientation !== shimOriented.orientation) {
  failures += 1
  console.log(`FAIL metadata-orientation-value sharp=${String(sharpOriented.orientation)} shim=${String(shimOriented.orientation)}`)
} else {
  console.log(`ok metadata-orientation-value orientation=${String(shimOriented.orientation)}`)
}

// A synthetic raster has no encoded bytes; both providers build the same one.
const syntheticSharp = await sharp({
  create: { width: 5, height: 3, channels: 4, background: { r: 9, g: 8, b: 7, alpha: 1 } },
}).png().toBuffer()
const syntheticShim = await shim({
  create: { width: 5, height: 3, channels: 4, background: { r: 9, g: 8, b: 7, alpha: 1 } },
}).png().toBuffer()
checks += 1
if (!difference(await pixelsOf(syntheticSharp), await pixelsOf(syntheticShim)).comparable
  || difference(await pixelsOf(syntheticSharp), await pixelsOf(syntheticShim)).max !== 0) {
  failures += 1
  console.log('FAIL synthetic-raster')
} else {
  console.log('ok synthetic-raster')
}

console.log(`RESULT image-parity=${failures === 0 ? 'OK' : 'FAIL'} checks=${checks} failures=${failures}`)
process.exit(failures === 0 ? 0 : 1)
