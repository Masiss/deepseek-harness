//! Image inspection and encoding for the Desktop runtime.
//!
//! The attachment service admits a raster, reports its intrinsic facts, and
//! re-encodes it into a normalized 8-bit sRGB form. Those operations run here so
//! the Desktop payload carries one native module instead of the sharp package
//! and its libvips binaries.

use std::io::Cursor;

use image::{DynamicImage, ImageFormat, RgbaImage};
use napi::bindgen_prelude::*;
use napi::bindgen_prelude::Buffer;
use napi_derive::napi;

use crate::image_scan::scan_container;

/// The intrinsic facts of an encoded raster.
#[napi(object)]
pub struct ImageFacts {
    /// Container format: `png`, `jpeg`, `webp`, or `gif`.
    pub format: String,
    /// Intrinsic width of the stored raster.
    pub width: u32,
    /// Intrinsic height of the stored raster.
    pub height: u32,
    /// Number of frames in the container.
    pub pages: u32,
    /// EXIF orientation, when the container carries one.
    pub orientation: Option<u32>,
    /// Sample depth: `uchar`, `ushort`, or `float`.
    pub depth: String,
    /// Colour space: `srgb`, `rgb16`, or `b-w`.
    pub space: String,
    /// Whether the pixels carry an alpha channel.
    pub has_alpha: bool,
    /// Whether the container carries descriptive metadata, a colour profile, or orientation.
    pub carries_metadata: bool,
    /// Whether the container carries an EXIF block.
    pub has_exif: bool,
    /// Whether the container carries an XMP block.
    pub has_xmp: bool,
    /// Whether the container carries an IPTC block.
    pub has_iptc: bool,
    /// Whether the container carries an ICC profile.
    pub has_icc: bool,
    /// Whether the container carries a Photoshop tag block.
    pub has_photoshop: bool,
    /// Whether the container carries comments.
    pub has_comments: bool,
}

/// A decoded raster: its facts and its 8-bit pixels.
#[napi(object)]
pub struct DecodedImage {
    pub facts: ImageFacts,
    /// Interleaved 8-bit pixels, three or four channels.
    pub pixels: Buffer,
    /// Channels per pixel in `pixels`.
    pub channels: u32,
}

/// A raster after a geometry operation.
#[napi(object)]
pub struct TransformedImage {
    pub pixels: Buffer,
    pub width: u32,
    pub height: u32,
}

fn reason(error: impl std::fmt::Display) -> Error {
    Error::from_reason(error.to_string())
}

/// The container format name the pipeline reports.
fn format_name(format: ImageFormat) -> Option<&'static str> {
    match format {
        ImageFormat::Png => Some("png"),
        ImageFormat::Jpeg => Some("jpeg"),
        ImageFormat::WebP => Some("webp"),
        ImageFormat::Gif => Some("gif"),
        _ => None,
    }
}

/// The 8-bit pixels of a decoded raster, with the source's channel count.
fn to_pixels(image: &DynamicImage) -> (Vec<u8>, u32, &'static str, &'static str, bool) {
    match image {
        DynamicImage::ImageLuma8(buffer) => (
            DynamicImage::ImageLuma8(buffer.clone()).to_rgb8().into_raw(),
            3,
            "uchar",
            "b-w",
            false,
        ),
        DynamicImage::ImageLumaA8(buffer) => (
            DynamicImage::ImageLumaA8(buffer.clone()).to_rgba8().into_raw(),
            4,
            "uchar",
            "b-w",
            true,
        ),
        DynamicImage::ImageRgb8(buffer) => (buffer.as_raw().clone(), 3, "uchar", "srgb", false),
        DynamicImage::ImageRgba8(buffer) => (buffer.as_raw().clone(), 4, "uchar", "srgb", true),
        DynamicImage::ImageLuma16(buffer) => (
            DynamicImage::ImageLuma16(buffer.clone()).to_rgb8().into_raw(),
            3,
            "ushort",
            "rgb16",
            false,
        ),
        DynamicImage::ImageLumaA16(buffer) => (
            DynamicImage::ImageLumaA16(buffer.clone()).to_rgba8().into_raw(),
            4,
            "ushort",
            "rgb16",
            true,
        ),
        DynamicImage::ImageRgb16(buffer) => (
            DynamicImage::ImageRgb16(buffer.clone()).to_rgb8().into_raw(),
            3,
            "ushort",
            "rgb16",
            false,
        ),
        DynamicImage::ImageRgba16(buffer) => (
            DynamicImage::ImageRgba16(buffer.clone()).to_rgba8().into_raw(),
            4,
            "ushort",
            "rgb16",
            true,
        ),
        DynamicImage::ImageRgb32F(buffer) => (
            DynamicImage::ImageRgb32F(buffer.clone()).to_rgb8().into_raw(),
            3,
            "float",
            "srgb",
            false,
        ),
        DynamicImage::ImageRgba32F(buffer) => (
            DynamicImage::ImageRgba32F(buffer.clone()).to_rgba8().into_raw(),
            4,
            "float",
            "srgb",
            true,
        ),
        // A future variant the pipeline does not know: report it as an
        // opaque 8-bit sRGB raster rather than guessing a depth.
        _ => (Vec::new(), 4, "uchar", "srgb", true),
    }
}

/// Decode an encoded raster and report its intrinsic facts.
/// @param data - Complete encoded image bytes.
/// @returns The facts and the 8-bit pixels of the raster.
#[napi]
pub fn decode_image(data: Buffer) -> Result<DecodedImage> {
    let data = data.as_ref().to_vec();
    let format = image::guess_format(&data).map_err(reason)?;
    let name = format_name(format)
        .ok_or_else(|| Error::from_reason("unsupported or malformed image data"))?;
    let decoded = decode_raster(&data, format)?;
    let container = scan_container(&data, format);
    let (mut pixels, mut channels, depth, mut space, mut has_alpha) = to_pixels(&decoded);
    if container.cmyk {
        space = "cmyk";
    }
    if format == ImageFormat::Gif && channels == 4 && !container.transparency {
        // A GIF frame carries transparency through its palette, not through an
        // alpha channel: a frame without a transparent index is an RGB raster.
        pixels = opaque_rgb(&pixels, channels);
        channels = 3;
        has_alpha = false;
    }
    let (width, height) = (decoded.width(), decoded.height());
    Ok(DecodedImage {
        facts: ImageFacts {
            format: name.to_string(),
            width,
            height,
            pages: container.pages.max(1),
            orientation: container.orientation,
            depth: depth.to_string(),
            space: space.to_string(),
            has_alpha,
            carries_metadata: container.carries_metadata(),
            has_exif: container.exif,
            has_xmp: container.xmp,
            has_iptc: container.iptc,
            has_icc: container.icc,
            has_photoshop: container.photoshop,
            has_comments: container.comments,
        },
        pixels: Buffer::from(pixels),
        channels,
    })
}

/// Scale a raster to fit inside a box, keeping its aspect ratio.
/// @param pixels - Interleaved 8-bit pixels.
/// @param width - Current width.
/// @param height - Current height.
/// @param channels - Channels per pixel.
/// @param target_width - Box width.
/// @param target_height - Box height.
/// @param without_enlargement - Whether a smaller raster keeps its size.
/// @returns The scaled pixels and their dimensions.
#[napi]
pub fn resize_image(
    pixels: Buffer,
    width: u32,
    height: u32,
    channels: u32,
    target_width: u32,
    target_height: u32,
    without_enlargement: bool,
) -> Result<TransformedImage> {
    let pixels = pixels.as_ref().to_vec();
    let source = raster(&pixels, width, height, channels)?;
    // A missing side is unlimited, as in sharp: only the given side bounds the
    // scale.
    let width_scale = if target_width == 0 {
        f64::INFINITY
    } else {
        target_width as f64 / width as f64
    };
    let height_scale = if target_height == 0 {
        f64::INFINITY
    } else {
        target_height as f64 / height as f64
    };
    let scale = width_scale.min(height_scale);
    let mut scaled_width = ((width as f64 * scale).round() as u32).max(1);
    let mut scaled_height = ((height as f64 * scale).round() as u32).max(1);
    if without_enlargement && scaled_width >= width && scaled_height >= height {
        scaled_width = width;
        scaled_height = height;
    }
    if scaled_width == width && scaled_height == height {
        return Ok(TransformedImage { pixels: Buffer::from(pixels), width, height });
    }
    let (pixels, _) = resize(&source, scaled_width, scaled_height)?;
    Ok(TransformedImage {
        pixels: Buffer::from(pixels),
        width: scaled_width,
        height: scaled_height,
    })
}

/// Apply an EXIF orientation to a raster.
/// @param pixels - Interleaved 8-bit pixels.
/// @param width - Current width.
/// @param height - Current height.
/// @param channels - Channels per pixel.
/// @param orientation - EXIF orientation, 1 through 8.
/// @returns The oriented pixels and their dimensions.
#[napi]
pub fn rotate_image(
    pixels: Buffer,
    width: u32,
    height: u32,
    channels: u32,
    orientation: u32,
) -> Result<TransformedImage> {
    let pixels = pixels.as_ref().to_vec();
    if orientation <= 1 || orientation > 8 {
        return Ok(TransformedImage { pixels: Buffer::from(pixels), width, height });
    }
    let source = raster(&pixels, width, height, channels)?.to_rgba8();
    let rotated: RgbaImage = match orientation {
        2 => image::imageops::flip_horizontal(&source),
        3 => image::imageops::rotate180(&source),
        4 => image::imageops::flip_vertical(&source),
        5 => image::imageops::flip_horizontal(&image::imageops::rotate270(&source)),
        6 => image::imageops::rotate90(&source),
        7 => image::imageops::flip_horizontal(&image::imageops::rotate90(&source)),
        8 => image::imageops::rotate270(&source),
        _ => source,
    };
    Ok(TransformedImage {
        pixels: Buffer::from(pixels_of(&rotated, channels)),
        width: rotated.width(),
        height: rotated.height(),
    })
}

/// Encode a raster as JPEG.
/// @param pixels - Interleaved 8-bit pixels.
/// @param width - Raster width.
/// @param height - Raster height.
/// @param channels - Channels per pixel.
/// @param quality - Quality target, 1 through 100.
/// @returns The encoded bytes.
#[napi]
pub fn encode_jpeg(
    pixels: Buffer,
    width: u32,
    height: u32,
    channels: u32,
    quality: u32,
) -> Result<Buffer> {
    let pixels = pixels.as_ref().to_vec();
    let source = raster(&pixels, width, height, channels)?.to_rgb8();
    let mut encoded = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut encoded, quality.clamp(1, 100) as u8)
        .encode_image(&source)
        .map_err(reason)?;
    Ok(Buffer::from(encoded))
}

/// Encode a raster as WebP.
/// @param pixels - Interleaved 8-bit pixels.
/// @param width - Raster width.
/// @param height - Raster height.
/// @param channels - Channels per pixel.
/// @param quality - Quality target, 1 through 100.
/// @param effort - Compression effort, 0 through 6.
/// @returns The encoded bytes.
#[napi]
pub fn encode_webp(
    pixels: Buffer,
    width: u32,
    height: u32,
    channels: u32,
    quality: u32,
    effort: u32,
) -> Result<Buffer> {
    let pixels = pixels.as_ref().to_vec();
    let source = raster(&pixels, width, height, channels)?;
    let dynamic = if channels == 4 {
        DynamicImage::ImageRgba8(source.to_rgba8())
    } else {
        DynamicImage::ImageRgb8(source.to_rgb8())
    };
    let mut encoded = Vec::new();
    let encoder = webp::WebpEncoder::new(&mut encoded)
        .with_quality(quality.clamp(1, 100) as u8)
        .with_compression(effort.min(6) as u8);
    dynamic
        .write_with_encoder(encoder)
        .map_err(reason)?;
    Ok(Buffer::from(encoded))
}

/// A raster as an image buffer of the given channel count.
fn raster(
    pixels: &[u8],
    width: u32,
    height: u32,
    channels: u32,
) -> Result<DynamicImage> {
    let mismatch =
        || Error::from_reason("image pixels do not match their dimensions");
    Ok(match channels {
        1 => DynamicImage::ImageLuma8(
            image::ImageBuffer::<image::Luma<u8>, Vec<u8>>::from_raw(width, height, pixels.to_vec())
                .ok_or_else(mismatch)?,
        ),
        2 => DynamicImage::ImageLumaA8(
            image::ImageBuffer::<image::LumaA<u8>, Vec<u8>>::from_raw(width, height, pixels.to_vec())
                .ok_or_else(mismatch)?,
        ),
        4 => DynamicImage::ImageRgba8(
            image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(width, height, pixels.to_vec())
                .ok_or_else(mismatch)?,
        ),
        _ => DynamicImage::ImageRgb8(
            image::ImageBuffer::<image::Rgb<u8>, Vec<u8>>::from_raw(width, height, pixels.to_vec())
                .ok_or_else(mismatch)?,
        ),
    })
}

/// Decode a raster with the decoder each format is fastest in.
/// @param data - Complete encoded image bytes.
/// @param format - The container format the bytes carry.
/// @returns The decoded raster.
fn decode_raster(data: &[u8], format: ImageFormat) -> Result<DynamicImage> {
    match format {
        ImageFormat::Png => decode_png(data),
        ImageFormat::WebP => webp::decode(data)
            .map_err(|error| Error::from_reason(format!("unsupported or malformed image data: {error}"))),
        _ => image::ImageReader::new(Cursor::new(data))
            .with_guessed_format()
            .map_err(reason)?
            .decode()
            .map_err(reason),
    }
}

/// Decode a PNG with the SIMD decoder libspng gives sharp.
/// @param data - Complete encoded PNG bytes.
/// @returns The decoded raster.
fn decode_png(data: &[u8]) -> Result<DynamicImage> {
    use zune_core::colorspace::ColorSpace;
    use zune_core::result::DecodingResult;

    let mut decoder = zune_png::PngDecoder::new(data);
    decoder
        .decode_headers()
        .map_err(|error| Error::from_reason(format!("unsupported or malformed image data: {error:?}")))?;
    let colorspace = decoder
        .get_colorspace()
        .ok_or_else(|| Error::from_reason("unsupported or malformed image data"))?;
    let (width, height) = decoder
        .get_info()
        .map(|info| (info.width as u32, info.height as u32))
        .ok_or_else(|| Error::from_reason("unsupported or malformed image data"))?;
    let samples = decoder
        .decode()
        .map_err(|error| Error::from_reason(format!("unsupported or malformed image data: {error:?}")))?;
    let mismatch = || Error::from_reason("image pixels do not match their dimensions");
    match samples {
        DecodingResult::U8(bytes) => Ok(match colorspace {
            ColorSpace::RGB => DynamicImage::ImageRgb8(
                image::ImageBuffer::<image::Rgb<u8>, Vec<u8>>::from_raw(width, height, bytes)
                    .ok_or_else(mismatch)?,
            ),
            ColorSpace::RGBA => DynamicImage::ImageRgba8(
                image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(width, height, bytes)
                    .ok_or_else(mismatch)?,
            ),
            ColorSpace::Luma => DynamicImage::ImageLuma8(
                image::ImageBuffer::<image::Luma<u8>, Vec<u8>>::from_raw(width, height, bytes)
                    .ok_or_else(mismatch)?,
            ),
            _ => DynamicImage::ImageLumaA8(
                image::ImageBuffer::<image::LumaA<u8>, Vec<u8>>::from_raw(width, height, bytes)
                    .ok_or_else(mismatch)?,
            ),
        }),
        DecodingResult::U16(bytes) => Ok(match colorspace {
            ColorSpace::RGB => DynamicImage::ImageRgb16(
                image::ImageBuffer::<image::Rgb<u16>, Vec<u16>>::from_raw(width, height, bytes)
                    .ok_or_else(mismatch)?,
            ),
            ColorSpace::RGBA => DynamicImage::ImageRgba16(
                image::ImageBuffer::<image::Rgba<u16>, Vec<u16>>::from_raw(width, height, bytes)
                    .ok_or_else(mismatch)?,
            ),
            ColorSpace::Luma => DynamicImage::ImageLuma16(
                image::ImageBuffer::<image::Luma<u16>, Vec<u16>>::from_raw(width, height, bytes)
                    .ok_or_else(mismatch)?,
            ),
            _ => DynamicImage::ImageLumaA16(
                image::ImageBuffer::<image::LumaA<u16>, Vec<u16>>::from_raw(width, height, bytes)
                    .ok_or_else(mismatch)?,
            ),
        }),
        DecodingResult::F32(bytes) => {
            let narrow: Vec<u8> = bytes
                .iter()
                .map(|value| (value.clamp(0.0, 1.0) * 255.0).round() as u8)
                .collect();
            Ok(match colorspace {
                ColorSpace::RGB => DynamicImage::ImageRgb8(
                    image::ImageBuffer::<image::Rgb<u8>, Vec<u8>>::from_raw(width, height, narrow)
                        .ok_or_else(mismatch)?,
                ),
                ColorSpace::RGBA => DynamicImage::ImageRgba8(
                    image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_raw(width, height, narrow)
                        .ok_or_else(mismatch)?,
                ),
                ColorSpace::Luma => DynamicImage::ImageLuma8(
                    image::ImageBuffer::<image::Luma<u8>, Vec<u8>>::from_raw(width, height, narrow)
                        .ok_or_else(mismatch)?,
                ),
                _ => DynamicImage::ImageLumaA8(
                    image::ImageBuffer::<image::LumaA<u8>, Vec<u8>>::from_raw(width, height, narrow)
                        .ok_or_else(mismatch)?,
                ),
            })
        }
        _ => Err(Error::from_reason("unsupported or malformed image data")),
    }
}


/// Scale a raster with the SIMD resizer libvips uses behind sharp.
/// @param source - The raster to scale.
/// @param width - Target width.
/// @param height - Target height.
/// @returns The scaled pixels and their channel count.
fn resize(source: &DynamicImage, width: u32, height: u32) -> Result<(Vec<u8>, u32)> {
    use fast_image_resize::images::Image as FirImage;
    use fast_image_resize::pixels::PixelType;
    use fast_image_resize::{FilterType, ResizeAlg, ResizeOptions, Resizer};

    let (pixels, channels, _, _, _) = to_pixels(source);
    let pixel_type = if channels == 4 {
        PixelType::U8x4
    } else {
        PixelType::U8x3
    };
    let input = FirImage::from_vec_u8(source.width(), source.height(), pixels, pixel_type)
        .map_err(reason)?;
    let mut output = FirImage::new(width, height, pixel_type);
    let mut resizer = Resizer::new();
    resizer
        .resize(
            &input,
            &mut output,
            &ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3)),
        )
        .map_err(reason)?;
    let scaled = output.into_vec();
    if scaled.len() < (width as usize) * (height as usize) * channels as usize {
        return Err(Error::from_reason("the resizer returned an incomplete raster"));
    }
    Ok((scaled, channels))
}

/// The pixels of a raster without an alpha channel.
/// @param pixels - Interleaved 8-bit pixels.
/// @param channels - Channels per pixel.
/// @returns The three-channel pixels.
fn opaque_rgb(pixels: &[u8], channels: u32) -> Vec<u8> {
    let mut opaque = Vec::with_capacity(pixels.len() / channels as usize * 3);
    for pixel in pixels.chunks_exact(channels as usize) {
        opaque.extend_from_slice(&pixel[..3]);
    }
    opaque
}

/// The pixels of a buffer, keeping the channel count of the source.
fn pixels_of(buffer: &RgbaImage, channels: u32) -> Vec<u8> {
    if channels == 4 {
        return buffer.as_raw().clone();
    }
    let raw = buffer.as_raw();
    let mut opaque = Vec::with_capacity(raw.len() / 4 * 3);
    for pixel in raw.chunks_exact(4) {
        opaque.extend_from_slice(&pixel[..3]);
    }
    opaque
}
