//! Container metadata tests over bytes the image pipeline sees.

use crate::image_meta::{
    exif_with_orientation, extract_metadata, write_jpeg_metadata, write_webp_metadata, MetadataBlocks,
};
use image::{ImageEncoder, ImageFormat};

/// One pixel of every channel: red, green, blue.
const PIXELS: [u8; 3] = [1, 2, 3];

/// The EXIF block that declares one orientation.
fn exif_block(orientation: u16) -> Vec<u8> {
    let field = exif::Field {
        tag: exif::Tag::Orientation,
        ifd_num: exif::In::PRIMARY,
        value: exif::Value::Short(vec![orientation]),
    };
    let mut writer = exif::experimental::Writer::new();
    writer.push_field(&field);
    let mut written = std::io::Cursor::new(Vec::new());
    writer.write(&mut written, true).expect("the EXIF block writes");
    written.into_inner()
}

#[test]
fn a_jpeg_keeps_the_exif_block_it_carries() {
    let exif = exif_block(6);
    let mut jpeg = vec![0xff, 0xd8];
    let mut payload = Vec::from(b"Exif\0\0".as_slice());
    payload.extend_from_slice(&exif);
    jpeg.extend_from_slice(&[0xff, 0xe1]);
    jpeg.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    jpeg.extend_from_slice(&payload);
    jpeg.extend_from_slice(&[0xff, 0xd9]);

    let blocks = extract_metadata(&jpeg, ImageFormat::Jpeg);
    assert_eq!(blocks.exif.as_deref(), Some(exif.as_slice()));
    assert_eq!(
        exif::Reader::new()
            .read_raw(blocks.exif.expect("the block"))
            .expect("the block reads")
            .get_field(exif::Tag::Orientation, exif::In::PRIMARY)
            .expect("the orientation is there")
            .value
            .get_uint(0),
        Some(6)
    );
}

#[test]
fn a_jpeg_gains_the_blocks_it_is_given() {
    let exif = exif_block(3);
    let blocks = MetadataBlocks {
        exif: Some(exif.clone()),
        icc: Some(vec![9, 8, 7]),
        ..MetadataBlocks::default()
    };
    let jpeg = write_jpeg_metadata(&[0xff, 0xd8, 0xff, 0xd9], &blocks);
    let written = extract_metadata(&jpeg, ImageFormat::Jpeg);
    assert_eq!(written.exif.as_deref(), Some(exif.as_slice()));
    assert_eq!(written.icc.as_deref(), Some([9, 8, 7].as_slice()));
}

#[test]
fn a_webp_gains_the_blocks_it_is_given() {
    let exif = exif_block(8);
    let blocks = MetadataBlocks {
        exif: Some(exif.clone()),
        ..MetadataBlocks::default()
    };
    // A RIFF container with an empty VP8L chunk.
    let mut webp = Vec::from(b"RIFF".as_slice());
    webp.extend_from_slice(&20u32.to_le_bytes());
    webp.extend_from_slice(b"WEBP");
    webp.extend_from_slice(b"VP8L");
    webp.extend_from_slice(&4u32.to_le_bytes());
    webp.extend_from_slice(&[0, 0, 0, 0]);
    let written = write_webp_metadata(&webp, &blocks);
    let read = extract_metadata(&written, ImageFormat::WebP);
    assert_eq!(read.exif.as_deref(), Some(exif.as_slice()));
    // The RIFF size covers the chunk that was added.
    let size = u32::from_le_bytes([written[4], written[5], written[6], written[7]]) as usize;
    assert_eq!(size, written.len() - 8);
}

#[test]
fn an_orientation_replaces_the_one_the_source_declares() {
    let source = exif_block(3);
    let replaced = exif_with_orientation(Some(&source), 6).expect("the block is written");
    let orientation = exif::Reader::new()
        .read_raw(replaced)
        .expect("the block reads")
        .get_field(exif::Tag::Orientation, exif::In::PRIMARY)
        .expect("the orientation is there")
        .value
        .get_uint(0);
    assert_eq!(orientation, Some(6));
}

#[test]
fn an_orientation_of_one_keeps_the_source_block() {
    let source = exif_block(3);
    let kept = exif_with_orientation(Some(&source), 1).expect("the block is kept");
    assert_eq!(kept, source);
}

#[test]
fn a_source_without_metadata_gains_an_orientation() {
    let written = exif_with_orientation(None, 6).expect("the block is written");
    let orientation = exif::Reader::new()
        .read_raw(written)
        .expect("the block reads")
        .get_field(exif::Tag::Orientation, exif::In::PRIMARY)
        .expect("the orientation is there")
        .value
        .get_uint(0);
    assert_eq!(orientation, Some(6));
}

#[test]
fn a_png_round_trips_through_the_encoder() {
    let mut encoded = Vec::new();
    image::codecs::png::PngEncoder::new(&mut encoded)
        .write_image(&PIXELS, 1, 1, image::ExtendedColorType::Rgb8)
        .expect("the PNG writes");
    let decoded = image::load_from_memory_with_format(&encoded, ImageFormat::Png)
        .expect("the PNG reads")
        .to_rgb8();
    assert_eq!(decoded.as_raw(), &PIXELS);
}
