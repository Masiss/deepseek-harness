//! Container scan tests over bytes the image pipeline sees.

use crate::image_scan::scan_container;
use image::ImageFormat;

/// One GIF frame written by the sharp package: a 4x2 palette image.
const GIF: [u8; 44] = [
    0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x04, 0x00, 0x02, 0x00, 0x80, 0x00, 0x00, 0x4c, 0x69, 0x71,
    0x01, 0x02, 0x03, 0x21, 0xf9, 0x04, 0x05, 0x00, 0x00, 0x00, 0x00, 0x2c, 0x00, 0x00, 0x00, 0x00,
    0x04, 0x00, 0x02, 0x00, 0x00, 0x02, 0x03, 0x8c, 0x6f, 0x05, 0x00, 0x3b,
];

#[test]
fn a_single_frame_gif_reports_one_frame() {
    let facts = scan_container(&GIF, ImageFormat::Gif);
    assert_eq!(facts.pages, 1);
    assert!(facts.transparency);
    assert!(!facts.carries_metadata());
}

#[test]
fn a_jpeg_without_metadata_reports_none() {
    // A minimal JPEG: SOI, a quantisation table, a frame header, and the end.
    let jpeg: [u8; 20] = [
        0xff, 0xd8, 0xff, 0xdb, 0x00, 0x04, 0x00, 0x00, 0xff, 0xc0, 0x00, 0x02, 0x00, 0x00, 0x00,
        0x00, 0xff, 0xd9, 0x00, 0x00,
    ];
    let facts = scan_container(&jpeg, ImageFormat::Jpeg);
    assert_eq!(facts.pages, 1);
    assert!(!facts.carries_metadata());
    assert_eq!(facts.orientation, None);
}

/// A RIFF container with one image chunk and a trailing VP8X header whose
/// declared payload length is zero.
fn webp_with_empty_trailing_chunk() -> Vec<u8> {
    let mut webp: Vec<u8> = Vec::new();
    webp.extend_from_slice(b"RIFF");
    webp.extend_from_slice(&0u32.to_le_bytes());
    webp.extend_from_slice(b"WEBP");
    webp.extend_from_slice(b"VP8 ");
    webp.extend_from_slice(&4u32.to_le_bytes());
    webp.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
    webp.extend_from_slice(b"VP8X");
    webp.extend_from_slice(&0u32.to_le_bytes());
    let size = (webp.len() - 8) as u32;
    webp[4..8].copy_from_slice(&size.to_le_bytes());
    webp
}

#[test]
fn a_webp_with_an_empty_trailing_chunk_reports_no_metadata() {
    let facts = scan_container(&webp_with_empty_trailing_chunk(), ImageFormat::WebP);
    assert_eq!(facts.pages, 1);
    assert!(!facts.carries_metadata());
}

#[test]
fn a_webp_vp8x_chunk_still_reports_its_flags() {
    let mut webp = webp_with_empty_trailing_chunk();
    // Give the VP8X chunk a one-byte payload carrying the EXIF flag.
    webp[28..32].copy_from_slice(&1u32.to_le_bytes());
    webp.push(0x08);
    let size = (webp.len() - 8) as u32;
    webp[4..8].copy_from_slice(&size.to_le_bytes());
    let facts = scan_container(&webp, ImageFormat::WebP);
    assert!(facts.exif);
}
