'use strict'
/**
 * `sharp`-shaped entry over the Rust native module.
 *
 * The attachment service inspects a raster, normalizes it, and re-encodes it.
 * This wrapper serves the operations that service uses — decode and report the
 * intrinsic facts, apply the EXIF orientation, scale inside a box, and encode
 * as WebP or JPEG — from the Rust module.
 *
 * Everything outside that surface is delegated to the real sharp package when
 * the payload still carries it, so test fixtures and other consumers keep
 * their behavior. The Desktop payload drops sharp once this module is present.
 */

const { existsSync } = require('node:fs')
const { dirname, join } = require('node:path')
const native = require('./dsh-native.node')

/** How many pipelines fell back to the real sharp package. */
let delegated = 0

/** The real sharp package, when the payload still carries it. */
function fallbackSharp() {
  if (fallbackSharp.resolved !== undefined) return fallbackSharp.resolved
  const here = dirname(__filename)
  const candidates = [
    // The Desktop runtime: <runtime>/node_modules/sharp.
    join(here, '..', '..', '..', 'sharp'),
    // The repository: packages/attachment/attachment-local/node_modules/sharp.
    join(here, '..', '..', '..', 'packages', 'attachment', 'attachment-local', 'node_modules', 'sharp'),
  ]
  for (const candidate of candidates) {
    if (!existsSync(candidate)) continue
    try {
      fallbackSharp.resolved = require(candidate)
      return fallbackSharp.resolved
    } catch {
      // A candidate that does not load is not a usable fallback.
    }
  }
  fallbackSharp.resolved = null
  return null
}

/** A pipeline over one encoded raster. */
class Pipeline {
  constructor(input, options) {
    this.input = input
    this.options = options ?? {}
    this.operations = []
    this.format = null
    this.quality = undefined
    this.effort = undefined
  }

  /** The intrinsic facts of the source raster. */
  async metadata() {
    const decoded = native.decodeImage(Buffer.from(this.input))
    const facts = decoded.facts
    return {
      format: facts.format,
      width: facts.width,
      height: facts.height,
      pages: facts.pages,
      orientation: facts.orientation ?? undefined,
      depth: facts.depth,
      space: facts.space,
      hasAlpha: facts.hasAlpha,
      hasProfile: facts.hasIcc,
      exif: facts.hasExif ? Buffer.alloc(0) : undefined,
      xmp: facts.hasXmp ? Buffer.alloc(0) : undefined,
      iptc: facts.hasIptc ? Buffer.alloc(0) : undefined,
      icc: facts.hasIcc ? Buffer.alloc(0) : undefined,
      tifftagPhotoshop: facts.hasPhotoshop ? {} : undefined,
      comments: facts.hasComments ? [] : undefined,
    }
  }

  /** A pipeline that yields the processed pixels. */
  raw() {
    const pipeline = this
    return {
      async toBuffer() {
        const processed = await pipeline.process()
        return Buffer.from(processed.pixels)
      },
    }
  }

  /** The pixels and dimensions after the pipeline's operations. */
  async process() {
    const decoded = native.decodeImage(Buffer.from(this.input))
    let pixels = Buffer.from(decoded.pixels)
    let width = decoded.facts.width
    let height = decoded.facts.height
    const channels = decoded.channels
    for (const operation of this.operations) {
      if (operation.kind === 'rotate') {
        const orientation = operation.orientation ?? decoded.facts.orientation ?? 1
        const rotated = native.rotateImage(pixels, width, height, channels, orientation)
        pixels = Buffer.from(rotated.pixels)
        width = rotated.width
        height = rotated.height
      } else if (operation.kind === 'resize') {
        const resized = native.resizeImage(
          pixels, width, height, channels,
          operation.width, operation.height, operation.withoutEnlargement,
        )
        pixels = Buffer.from(resized.pixels)
        width = resized.width
        height = resized.height
      }
    }
    return { pixels, width, height, channels }
  }

  /** The encoded bytes of the processed raster. */
  async toBuffer(options = {}) {
    const processed = await this.process()
    const format = this.format ?? 'png'
    let data
    if (format === 'jpeg') {
      data = Buffer.from(native.encodeJpeg(
        processed.pixels, processed.width, processed.height, processed.channels,
        this.quality ?? 80,
      ))
    } else if (format === 'webp') {
      data = Buffer.from(native.encodeWebp(
        processed.pixels, processed.width, processed.height, processed.channels,
        this.quality ?? 80, this.effort ?? 4,
      ))
    } else {
      data = await this.delegate('png').toBuffer()
    }
    if (options.resolveWithObject !== true) return data
    return { data, info: { width: processed.width, height: processed.height, format } }
  }

  /** An independent copy of this pipeline. */
  clone() {
    const copy = new Pipeline(this.input, this.options)
    copy.operations = [...this.operations]
    copy.format = this.format
    copy.quality = this.quality
    copy.effort = this.effort
    return copy
  }

  /** Apply the source's EXIF orientation. */
  rotate() {
    this.operations.push({ kind: 'rotate' })
    return this
  }

  /** Convert to the sRGB colour space; the module works in 8-bit sRGB. */
  toColourspace() {
    return this
  }

  /** Alias of {@link toColourspace}. */
  toColorspace() {
    return this
  }

  /** Scale inside a box, keeping the aspect ratio. */
  resize(options = {}) {
    this.operations.push({
      kind: 'resize',
      width: options.width ?? 0,
      height: options.height ?? 0,
      withoutEnlargement: options.withoutEnlargement === true,
    })
    return this
  }

  /** Encode as WebP. */
  webp(options = {}) {
    this.format = 'webp'
    this.quality = options.quality
    this.effort = options.effort
    return this
  }

  /** Encode as JPEG. */
  jpeg(options = {}) {
    this.format = 'jpeg'
    this.quality = options.quality
    return this
  }

  /**
   * Hand the pipeline to the real sharp package.
   * @param operation - The operation the caller asked for.
   * @returns The real package's pipeline for the same input and operations.
   */
  delegate(operation) {
    const fallback = fallbackSharp()
    if (fallback === null) {
      throw new Error(`sharp shim: ${operation} needs the sharp package`)
    }
    delegated += 1
    let instance = fallback(Buffer.from(this.input))
    for (const applied of this.operations) {
      if (applied.kind === 'rotate') instance = instance.rotate()
      else if (applied.kind === 'resize') {
        instance = instance.resize({
          width: applied.width,
          height: applied.height,
          fit: 'inside',
          withoutEnlargement: applied.withoutEnlargement,
        })
      }
    }
    return new Delegating(instance)
  }

  /** Encode as PNG; the Rust module encodes WebP and JPEG only. */
  png() {
    return this.delegate('png')
  }

  /** Encode as TIFF; the Rust module encodes WebP and JPEG only. */
  tiff() {
    return this.delegate('tiff')
  }

  /** Encode as another format; the Rust module encodes WebP and JPEG only. */
  toFormat(format) {
    return this.delegate(`toFormat(${format})`)
  }

  /** Write descriptive metadata; the Rust module writes no metadata. */
  withMetadata() {
    return this.delegate('withMetadata')
  }

  /** Write a colour profile; the Rust module writes no metadata. */
  withIccProfile() {
    return this.delegate('withIccProfile')
  }
}

/** A pipeline that hands every operation to the real sharp package. */
class Delegating {
  constructor(instance) {
    this.instance = instance
  }

  /** Every operation belongs to the real sharp package. */
  metadata(...args) {
    delegated += 1
    return this.instance.metadata(...args)
  }

  raw(...args) {
    delegated += 1
    return this.instance.raw(...args)
  }

  toBuffer(...args) {
    delegated += 1
    return this.instance.toBuffer(...args)
  }

  clone() {
    delegated += 1
    return new Delegating(this.instance.clone())
  }

  rotate(...args) {
    delegated += 1
    this.instance.rotate(...args)
    return this
  }

  toColourspace(...args) {
    delegated += 1
    this.instance.toColourspace(...args)
    return this
  }

  toColorspace(...args) {
    delegated += 1
    this.instance.toColorspace(...args)
    return this
  }

  resize(...args) {
    delegated += 1
    this.instance.resize(...args)
    return this
  }

  webp(...args) {
    delegated += 1
    this.instance.webp(...args)
    return this
  }

  jpeg(...args) {
    delegated += 1
    this.instance.jpeg(...args)
    return this
  }

  png(...args) {
    delegated += 1
    this.instance.png(...args)
    return this
  }

  tiff(...args) {
    delegated += 1
    this.instance.tiff(...args)
    return this
  }

  toFormat(...args) {
    delegated += 1
    this.instance.toFormat(...args)
    return this
  }

  withMetadata(...args) {
    delegated += 1
    this.instance.withMetadata(...args)
    return this
  }

  withIccProfile(...args) {
    delegated += 1
    this.instance.withIccProfile(...args)
    return this
  }
}

/**
 * Open an encoded raster.
 * @param input - Encoded bytes, or a sharp input descriptor.
 * @param options - Decoder options.
 * @returns The pipeline for that input.
 */
function sharp(input, options) {
  const encoded = (Buffer.isBuffer(input) || input instanceof Uint8Array)
    && (options?.raw === undefined)
  if (!encoded) {
    const fallback = fallbackSharp()
    if (fallback === null) {
      throw new Error('sharp shim: this input needs the sharp package')
    }
    delegated += 1
    return new Delegating(fallback(input, options))
  }
  return new Pipeline(input, options)
}

module.exports = sharp
module.exports.default = sharp
module.exports.sharp = sharp
/** How many pipelines fell back to the real sharp package. */
module.exports.delegatedCount = () => delegated
