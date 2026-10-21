import { existsSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'

const here = dirname(fileURLToPath(import.meta.url))
const module = join(here, '..', 'native', 'dsh-native.node')
const require = createRequire(import.meta.url)

/** The intrinsic facts of a raster. */
interface ShimFacts {
  readonly format: string
  readonly width: number
  readonly height: number
  readonly depth: string
  readonly space: string
  readonly hasAlpha: boolean
  readonly orientation?: number
  readonly exif?: Buffer
}

/** One pipeline over a raster. */
interface ShimPipeline {
  metadata(): Promise<ShimFacts>
  png(options?: { quality?: number }): ShimPipeline
  jpeg(options?: { quality?: number }): ShimPipeline
  tiff(options?: { quality?: number }): ShimPipeline
  toFormat(format: string): ShimPipeline
  withMetadata(options?: { orientation?: number }): ShimPipeline
  resize(options: { width?: number; height?: number; fit?: string }): ShimPipeline
  toBuffer(): Promise<Buffer>
}

/** The `sharp`-shaped entry over the native module. */
interface ShimEntry {
  (input: unknown, options?: unknown): ShimPipeline
  /** How many pipelines fell back to the real sharp package; gone when none can. */
  delegatedCount?: () => number
}

const shim = require(join(here, '..', 'native', 'sharp.cjs')) as ShimEntry

/** Whether the native module was built for this checkout. */
const built = existsSync(module)

describe.skipIf(!built)('sharp shim', () => {
  it('encodes a raster as PNG without the sharp package', async () => {
    const data = await shim({
      create: { width: 4, height: 3, channels: 3, background: { r: 9, g: 8, b: 7 } },
    }).png().toBuffer()
    const facts = await shim(data).metadata()
    expect(facts).toMatchObject({ format: 'png', width: 4, height: 3, depth: 'uchar', space: 'srgb' })
  })

  it('encodes a raster as TIFF without the sharp package', async () => {
    const data = await shim({
      create: { width: 4, height: 3, channels: 3, background: { r: 9, g: 8, b: 7 } },
    }).tiff().toBuffer()
    const facts = await shim(data).metadata()
    expect(facts).toMatchObject({ format: 'tiff', width: 4, height: 3 })
  })

  it('encodes into another format without the sharp package', async () => {
    // A half-transparent background keeps the alpha channel in the output.
    const data = await shim({
      create: { width: 4, height: 3, channels: 4, background: { r: 9, g: 8, b: 7, alpha: 0.5 } },
    }).toFormat('webp').toBuffer()
    const facts = await shim(data).metadata()
    expect(facts).toMatchObject({ format: 'webp', width: 4, height: 3, hasAlpha: true })
  })

  it('keeps the orientation a caller asks for', async () => {
    const source = await shim({
      create: { width: 4, height: 2, channels: 3, background: { r: 1, g: 2, b: 3 } },
    }).jpeg().toBuffer()
    const written = await shim(source).withMetadata({ orientation: 6 }).jpeg().toBuffer()
    const facts = await shim(written).metadata()
    expect(facts.orientation).toBe(6)
    expect(facts.exif).toBeDefined()
  })

  it('resizes inside a box without the sharp package', async () => {
    const source = await shim({
      create: { width: 8, height: 6, channels: 3, background: { r: 1, g: 2, b: 3 } },
    }).png().toBuffer()
    const written = await shim(source).resize({ width: 4, height: 4, fit: 'inside' }).png().toBuffer()
    const facts = await shim(written).metadata()
    expect(facts).toMatchObject({ width: 4, height: 3 })
  })

  it('carries no path to the sharp package', () => {
    // A fallback would need the package the payload does not ship.
    expect(shim.delegatedCount).toBeUndefined()
  })
})
