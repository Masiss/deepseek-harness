'use strict'
/**
 * `sharp`-shaped entry over the Rust native module.
 *
 * The attachment service inspects a raster, normalizes it, and re-encodes it.
 * This wrapper serves that surface — decode and report the intrinsic facts,
 * apply the EXIF orientation, scale inside a box, keep the metadata a caller
 * asks for, and encode as PNG, JPEG, WebP, or TIFF — from the Rust module.
 *
 * Every operation the wrapper offers runs on the native module, so a payload
 * that carries this module needs no sharp package and no libvips binaries.
 */

const { readFileSync } = require('node:fs')
const native = require('./dsh-native.node')

/** Formats the native module encodes. */
const ENCODABLE_FORMATS = new Set(['png', 'jpeg', 'webp', 'tiff', 'gif'])

/** Whether a name is one of the built-in colour profiles libvips ships. */
const BUILTIN_PROFILES = new Set(['srgb', 'p3', 'cmyk'])

/** The pixels of one synthetic raster, as the native module expects them. */
function syntheticPixels(create) {
  const width = create.width
  const height = create.height
  const channels = create.channels
  const background = create.background ?? { r: 0, g: 0, b: 0, alpha: 1 }
  const pixels = Buffer.alloc(width * height * channels)
  for (let index = 0; index < width * height; index += 1) {
    const offset = index * channels
    pixels[offset] = background.r ?? 0
    if (channels > 1) pixels[offset + 1] = background.g ?? 0
    if (channels > 2) pixels[offset + 2] = background.b ?? 0
    // The background alpha is a fraction, as sharp takes it.
    if (channels > 3) pixels[offset + 3] = Math.round((background.alpha ?? 1) * 255)
  }
  return pixels
}

/** The metadata blocks a container carries. */
function readBlocks(data) {
  const blocks = native.readMetadata(Buffer.from(data))
  return {
    exif: blocks.exif === undefined || blocks.exif === null ? undefined : Buffer.from(blocks.exif),
    icc: blocks.icc === undefined || blocks.icc === null ? undefined : Buffer.from(blocks.icc),
    xmp: blocks.xmp === undefined || blocks.xmp === null ? undefined : Buffer.from(blocks.xmp),
    iptc: blocks.iptc === undefined || blocks.iptc === null ? undefined : Buffer.from(blocks.iptc),
  }
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
    this.blocks = undefined
    // A synthetic raster has no encoded bytes to decode; its pixels are known.
    this.synthetic = undefined
  }

  /** The intrinsic facts of the source raster. */
  async metadata() {
    if (this.synthetic !== undefined) {
      return {
        format: 'raw',
        width: this.synthetic.width,
        height: this.synthetic.height,
        pages: 1,
        orientation: undefined,
        depth: 'uchar',
        space: 'srgb',
        hasAlpha: this.synthetic.channels === 4,
        hasProfile: false,
        exif: undefined,
        xmp: undefined,
        iptc: undefined,
        icc: undefined,
        tifftagPhotoshop: undefined,
        comments: undefined,
      }
    }
    const decoded = native.decodeImage(Buffer.from(this.input))
    const facts = decoded.facts
    const blocks = readBlocks(this.input)
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
      exif: blocks.exif,
      xmp: blocks.xmp,
      iptc: blocks.iptc,
      icc: blocks.icc,
      tifftagPhotoshop: facts.hasPhotoshop ? {} : undefined,
      comments: facts.hasComments ? [] : undefined,
    }
  }

  /** A pipeline that yields the processed pixels. */
  raw() {
    const pipeline = this
    return {
      async toBuffer(options = {}) {
        const processed = await pipeline.process()
        const data = Buffer.from(processed.pixels)
        if (options.resolveWithObject !== true) return data
        return {
          data,
          info: {
            width: processed.width,
            height: processed.height,
            channels: processed.channels,
            size: processed.width * processed.height * processed.channels,
            format: 'raw',
          },
        }
      },
    }
  }

  /** The pixels and dimensions after the pipeline's operations. */
  async process() {
    let pixels
    let width
    let height
    let channels
    let orientation = 1
    if (this.synthetic !== undefined) {
      pixels = Buffer.from(this.input)
      width = this.synthetic.width
      height = this.synthetic.height
      channels = this.synthetic.channels
    } else {
      const decoded = native.decodeImage(Buffer.from(this.input))
      pixels = Buffer.from(decoded.pixels)
      width = decoded.facts.width
      height = decoded.facts.height
      channels = decoded.channels
      orientation = decoded.facts.orientation ?? 1
    }
    for (const operation of this.operations) {
      if (operation.kind === 'rotate') {
        const applied = operation.orientation ?? orientation
        const rotated = native.rotateImage(pixels, width, height, channels, applied)
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
    const blocks = this.blocks ?? {}
    const data = Buffer.from(native.encodeImage(
      processed.pixels, processed.width, processed.height, processed.channels,
      {
        format,
        ...(this.quality === undefined ? {} : { quality: this.quality }),
        ...(this.effort === undefined ? {} : { effort: this.effort }),
        ...(blocks.exif === undefined ? {} : { exif: blocks.exif }),
        ...(blocks.icc === undefined ? {} : { icc: blocks.icc }),
        ...(blocks.xmp === undefined ? {} : { xmp: blocks.xmp }),
        ...(blocks.iptc === undefined ? {} : { iptc: blocks.iptc }),
      },
    ))
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
    copy.blocks = this.blocks
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

  /** Encode as PNG. */
  png(options = {}) {
    this.format = 'png'
    this.quality = options.quality
    return this
  }

  /** Encode as TIFF. */
  tiff(options = {}) {
    this.format = 'tiff'
    this.quality = options.quality
    return this
  }

  /** Encode as another format. */
  toFormat(format) {
    if (!ENCODABLE_FORMATS.has(format)) {
      throw new Error(`sharp shim: the native module cannot encode ${format}`)
    }
    this.format = format
    return this
  }

  /** Keep the source's descriptive metadata, and the orientation asked for. */
  withMetadata(options = {}) {
    // A synthetic raster carries no metadata to keep.
    const source = this.synthetic === undefined ? readBlocks(this.input) : {}
    const orientation = options.orientation
    let exif = source.exif
    if (orientation !== undefined && this.synthetic === undefined) {
      const written = native.exifForOrientation(Buffer.from(this.input), orientation)
      exif = written === undefined || written === null ? undefined : Buffer.from(written)
    }
    this.blocks = {
      exif,
      icc: source.icc,
      xmp: source.xmp,
      iptc: source.iptc,
    }
    return this
  }

  /** Write a colour profile. */
  withIccProfile(profile) {
    if (typeof profile !== 'string') {
      throw new Error('sharp shim: a colour profile is named by a file path')
    }
    if (BUILTIN_PROFILES.has(profile)) {
      throw new Error(
        `sharp shim: the native module does not ship the built-in ${profile} profile;`
        + ' pass the profile file instead',
      )
    }
    this.blocks = { ...(this.blocks ?? {}), icc: readFileSync(profile) }
    return this
  }
}

/**
 * Open an encoded raster.
 * @param input - Encoded bytes, a raw buffer description, or a synthetic raster.
 * @param options - Decoder options.
 * @returns The pipeline for that input.
 */
function sharp(input, options) {
  // sharp names a raw raster by its descriptor: the pixels and their shape.
  if (input?.raw !== undefined) {
    const raw = input.raw
    const pixels = Buffer.alloc(raw.width * raw.height * raw.channels)
    Buffer.from(input.data ?? input.buffer ?? input).copy(pixels)
    const pipeline = new Pipeline(pixels, options)
    pipeline.synthetic = { width: raw.width, height: raw.height, channels: raw.channels }
    return pipeline
  }
  // sharp names a new raster by what it should contain.
  if (input?.create !== undefined) {
    const create = input.create
    const pipeline = new Pipeline(syntheticPixels(create), options)
    pipeline.synthetic = { width: create.width, height: create.height, channels: create.channels }
    return pipeline
  }
  if (Buffer.isBuffer(input) || input instanceof Uint8Array) {
    return new Pipeline(input, options)
  }
  throw new Error('sharp shim: this input needs the sharp package')
}

module.exports = sharp
module.exports.default = sharp
module.exports.sharp = sharp
