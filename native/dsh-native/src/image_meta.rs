//! Metadata blocks read from, and written into, image containers.
//!
//! The Desktop runtime replaces the sharp package with this native module, so
//! the metadata a caller asks to keep has to survive the same way the pixels
//! do. A container carries its descriptive blocks in a format of its own: a
//! PNG holds them in chunks, a JPEG in application segments, a WebP in RIFF
//! chunks. This module reads those blocks out of a source and writes them into
//! an output container, so a caller that keeps metadata keeps the same bytes.
//!
//! The PNG and TIFF encoders carry a colour profile and an EXIF block
//! themselves; the JPEG and WebP containers take them as segments and chunks,
//! which this module writes.

use image::ImageFormat;

/// Metadata blocks one container carries.
#[derive(Default, Clone)]
pub struct MetadataBlocks {
    /// Raw EXIF block: a TIFF stream without the JPEG `Exif\0\0` prefix.
    pub exif: Option<Vec<u8>>,
    /// Colour profile bytes.
    pub icc: Option<Vec<u8>>,
    /// XMP packet bytes.
    pub xmp: Option<Vec<u8>>,
    /// IPTC block bytes.
    pub iptc: Option<Vec<u8>>,
}

impl MetadataBlocks {
    /// Whether the blocks carry anything at all.
    pub fn is_empty(&self) -> bool {
        self.exif.is_none() && self.icc.is_none() && self.xmp.is_none() && self.iptc.is_none()
    }
}

/// The bytes of the metadata blocks of one container.
pub fn extract_metadata(data: &[u8], format: ImageFormat) -> MetadataBlocks {
    match format {
        ImageFormat::Png => extract_png(data),
        ImageFormat::Jpeg => extract_jpeg(data),
        ImageFormat::WebP => extract_webp(data),
        _ => MetadataBlocks::default(),
    }
}

/// The PNG chunk blocks.
fn extract_png(data: &[u8]) -> MetadataBlocks {
    let mut blocks = MetadataBlocks::default();
    let mut offset = 8; // signature
    while offset + 8 <= data.len() {
        let length = u32::from_be_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]) as usize;
        let kind = &data[offset + 4..offset + 8];
        let start = offset + 8;
        let end = start + length;
        if end > data.len() {
            break;
        }
        let payload = &data[start..end];
        match kind {
            b"eXIf" => blocks.exif = Some(payload.to_vec()),
            b"iCCP" => {
                // The chunk is a name, a null, a compression method, and the
                // compressed profile.
                if let Some(separator) = payload.iter().position(|byte| *byte == 0) {
                    if payload.len() > separator + 2 {
                        blocks.icc = Some(payload[separator + 2..].to_vec());
                    }
                }
            }
            b"iTXt" => {
                if payload.starts_with(b"XML:com.adobe.xmp") {
                    if let Some(text) = xmp_text(payload) {
                        blocks.xmp = Some(text);
                    }
                }
            }
            _ => {}
        }
        offset = end + 4; // payload + CRC
    }
    blocks
}

/// The XMP text of a PNG international text chunk.
///
/// The chunk carries a keyword, three flags, a language tag, a translated
/// keyword, and the text itself; each of the first five ends with a null.
fn xmp_text(payload: &[u8]) -> Option<Vec<u8>> {
    let mut cursor = 0;
    for _ in 0..5 {
        let position = payload.iter().skip(cursor).position(|byte| *byte == 0)?;
        cursor += position + 1;
    }
    if cursor >= payload.len() {
        return None;
    }
    Some(payload[cursor..].to_vec())
}

/// The JPEG application segments.
fn extract_jpeg(data: &[u8]) -> MetadataBlocks {
    let mut blocks = MetadataBlocks::default();
    let mut icc = Vec::new();
    let mut offset = 2; // SOI
    while offset + 4 <= data.len() {
        if data[offset] != 0xff {
            offset += 1;
            continue;
        }
        let marker = data[offset + 1];
        if marker == 0xd8 || marker == 0xd9 || (0xd0..=0xd7).contains(&marker) {
            offset += 2;
            continue;
        }
        let length = u16::from_be_bytes([data[offset + 2], data[offset + 3]]) as usize;
        let start = offset + 4;
        let end = offset + 2 + length;
        if length < 2 || end > data.len() {
            break;
        }
        let payload = &data[start..end];
        if marker == 0xe1 {
            if payload.starts_with(b"Exif\0\0") {
                blocks.exif = Some(payload[6..].to_vec());
            } else if payload.starts_with(b"http://ns.adobe.com/xap/1.0/\0") {
                blocks.xmp = Some(payload[29..].to_vec());
            }
        } else if marker == 0xe2 && payload.starts_with(b"ICC_PROFILE\0") {
            // A profile longer than the segment limit is split into numbered
            // chunks; the sequence marker separates them.
            icc.extend_from_slice(&payload[14..]);
        } else if marker == 0xed && payload.starts_with(b"Photoshop 3.0\0") {
            blocks.iptc = Some(payload[14..].to_vec());
        }
        offset = end;
    }
    if !icc.is_empty() {
        blocks.icc = Some(icc);
    }
    blocks
}

/// The WebP RIFF chunks.
fn extract_webp(data: &[u8]) -> MetadataBlocks {
    let mut blocks = MetadataBlocks::default();
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WEBP" {
        return blocks;
    }
    let mut offset = 12;
    while offset + 8 <= data.len() {
        let kind = &data[offset..offset + 4];
        let length = u32::from_le_bytes([
            data[offset + 4],
            data[offset + 5],
            data[offset + 6],
            data[offset + 7],
        ]) as usize;
        let start = offset + 8;
        let end = start + length;
        if end > data.len() {
            break;
        }
        match kind {
            b"EXIF" => blocks.exif = Some(data[start..end].to_vec()),
            b"ICCP" => blocks.icc = Some(data[start..end].to_vec()),
            b"XMP " => blocks.xmp = Some(data[start..end].to_vec()),
            _ => {}
        }
        offset = end + (length % 2); // chunks are padded to an even length
    }
    blocks
}

/// The EXIF block that declares one orientation.
///
/// A source that already carries an EXIF block keeps it, with the orientation
/// the caller asked for; a source that carries none gets a block that holds
/// nothing but the orientation.
///
/// @param exif - The source's EXIF block, when it has one.
/// @param orientation - The orientation to declare, 1 through 8.
/// @returns The EXIF block to write, or none when the orientation is the default.
pub fn exif_with_orientation(exif: Option<&[u8]>, orientation: u32) -> Option<Vec<u8>> {
    if orientation <= 1 || orientation > 8 {
        return exif.map(|block| block.to_vec());
    }
    // A source that already carries an EXIF block keeps its other fields; a
    // source that carries none gets a block that holds only the orientation.
    let mut fields: Vec<exif::Field> = Vec::new();
    if let Some(block) = exif {
        if let Ok(source) = exif::Reader::new().read_raw(block.to_vec()) {
            for field in source.fields() {
                if field.tag != exif::Tag::Orientation {
                    fields.push(field.clone());
                }
            }
        }
    }
    fields.push(exif::Field {
        tag: exif::Tag::Orientation,
        ifd_num: exif::In::PRIMARY,
        value: exif::Value::Short(vec![orientation as u16]),
    });
    let mut writer = exif::experimental::Writer::new();
    for field in &fields {
        writer.push_field(field);
    }
    let mut written = std::io::Cursor::new(Vec::new());
    writer.write(&mut written, true).ok()?;
    Some(written.into_inner())
}

/// One PNG chunk: its length, kind, payload, and checksum.
fn png_chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut chunk = Vec::with_capacity(payload.len() + 12);
    chunk.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    chunk.extend_from_slice(kind);
    chunk.extend_from_slice(payload);
    let mut checksummed = Vec::with_capacity(chunk.len() + 4);
    checksummed.extend_from_slice(kind);
    checksummed.extend_from_slice(payload);
    chunk.extend_from_slice(&crc32fast::hash(&checksummed).to_be_bytes());
    chunk
}

/// Write the XMP packet into a PNG.
///
/// The PNG encoder carries the colour profile and the EXIF block itself; the
/// XMP packet is an international text chunk this module writes.
///
/// @param png - The encoded PNG.
/// @param xmp - The XMP packet to write.
/// @returns The PNG carrying the packet.
pub fn write_png_xmp(png: &[u8], xmp: &[u8]) -> Vec<u8> {
    if xmp.is_empty() {
        return png.to_vec();
    }
    let mut payload = Vec::with_capacity(xmp.len() + 32);
    payload.extend_from_slice(b"XML:com.adobe.xmp\0\0\0\0\0"); // keyword and flags
    payload.extend_from_slice(xmp);
    let chunk = png_chunk(b"iTXt", &payload);

    let mut written = Vec::with_capacity(png.len() + chunk.len());
    written.extend_from_slice(&png[..8]); // signature
    // The chunk belongs after the header, before the pixels.
    let mut offset = 8;
    let mut inserted = false;
    while offset + 8 <= png.len() {
        let length = u32::from_be_bytes([
            png[offset],
            png[offset + 1],
            png[offset + 2],
            png[offset + 3],
        ]) as usize;
        let kind = &png[offset + 4..offset + 8];
        let end = offset + 8 + length;
        if end + 4 > png.len() {
            break;
        }
        written.extend_from_slice(&png[offset..end + 4]);
        offset = end + 4;
        if !inserted && kind == b"IHDR" {
            written.extend_from_slice(&chunk);
            inserted = true;
        }
    }
    if offset < png.len() {
        written.extend_from_slice(&png[offset..]);
    }
    written
}

/// One JPEG application segment.
fn jpeg_segment(marker: u8, payload: &[u8]) -> Vec<u8> {
    let mut segment = Vec::with_capacity(payload.len() + 4);
    segment.push(0xff);
    segment.push(marker);
    segment.extend_from_slice(&((payload.len() + 2) as u16).to_be_bytes());
    segment.extend_from_slice(payload);
    segment
}

/// Write the metadata blocks into a JPEG.
///
/// @param jpeg - The encoded JPEG.
/// @param blocks - The blocks to write.
/// @returns The JPEG carrying the blocks.
pub fn write_jpeg_metadata(jpeg: &[u8], blocks: &MetadataBlocks) -> Vec<u8> {
    if blocks.is_empty() {
        return jpeg.to_vec();
    }
    let mut segments = Vec::new();
    if let Some(exif) = blocks.exif.as_ref() {
        let mut payload = Vec::with_capacity(exif.len() + 6);
        payload.extend_from_slice(b"Exif\0\0");
        payload.extend_from_slice(exif);
        segments.extend_from_slice(&jpeg_segment(0xe1, &payload));
    }
    if let Some(xmp) = blocks.xmp.as_ref() {
        let mut payload = Vec::with_capacity(xmp.len() + 29);
        payload.extend_from_slice(b"http://ns.adobe.com/xap/1.0/\0");
        payload.extend_from_slice(xmp);
        segments.extend_from_slice(&jpeg_segment(0xe1, &payload));
    }
    if let Some(icc) = blocks.icc.as_ref() {
        let mut payload = Vec::with_capacity(icc.len() + 14);
        payload.extend_from_slice(b"ICC_PROFILE\0");
        payload.extend_from_slice(&[1, 1]); // chunk number and total
        payload.extend_from_slice(icc);
        segments.extend_from_slice(&jpeg_segment(0xe2, &payload));
    }
    if let Some(iptc) = blocks.iptc.as_ref() {
        let mut payload = Vec::with_capacity(iptc.len() + 14);
        payload.extend_from_slice(b"Photoshop 3.0\0");
        payload.extend_from_slice(iptc);
        segments.extend_from_slice(&jpeg_segment(0xed, &payload));
    }
    if segments.is_empty() {
        return jpeg.to_vec();
    }
    let mut written = Vec::with_capacity(jpeg.len() + segments.len());
    written.extend_from_slice(&jpeg[..2.min(jpeg.len())]); // SOI
    written.extend_from_slice(&segments);
    written.extend_from_slice(&jpeg[2.min(jpeg.len())..]);
    written
}

/// Write the metadata blocks into a WebP.
///
/// @param webp - The encoded WebP.
/// @param blocks - The blocks to write.
/// @returns The WebP carrying the blocks.
pub fn write_webp_metadata(webp: &[u8], blocks: &MetadataBlocks) -> Vec<u8> {
    if blocks.is_empty() {
        return webp.to_vec();
    }
    let mut chunks = Vec::new();
    let mut payload = Vec::new();
    if let Some(exif) = blocks.exif.as_ref() {
        payload.clear();
        payload.extend_from_slice(exif);
        chunks.extend_from_slice(&riff_chunk(b"EXIF", &payload));
    }
    if let Some(icc) = blocks.icc.as_ref() {
        payload.clear();
        payload.extend_from_slice(icc);
        chunks.extend_from_slice(&riff_chunk(b"ICCP", &payload));
    }
    if let Some(xmp) = blocks.xmp.as_ref() {
        payload.clear();
        payload.extend_from_slice(xmp);
        chunks.extend_from_slice(&riff_chunk(b"XMP ", &payload));
    }
    if chunks.is_empty() {
        return webp.to_vec();
    }
    // The chunks belong before the image chunk, and the RIFF size covers
    // everything after the size field: the form type and the chunks.
    let mut body = Vec::with_capacity(webp.len() + chunks.len());
    body.extend_from_slice(&webp[12..]);
    body.splice(0..0, chunks);
    let mut written = Vec::with_capacity(webp.len() + 4);
    written.extend_from_slice(b"RIFF");
    written.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
    written.extend_from_slice(b"WEBP");
    written.extend_from_slice(&body);
    written
}

/// One RIFF chunk: its kind, payload, and padding.
fn riff_chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut chunk = Vec::with_capacity(payload.len() + 8);
    chunk.extend_from_slice(kind);
    chunk.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    chunk.extend_from_slice(payload);
    if payload.len() % 2 == 1 {
        chunk.push(0);
    }
    chunk
}
