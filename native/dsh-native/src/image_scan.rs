//! Container facts a decoder cannot report: metadata presence, EXIF orientation, and frame count.
//!
//! The image pipeline distinguishes a clean raster from one carrying descriptive
//! metadata, and it reports the orientation a viewer perceives. Both facts come
//! from the container itself, so each format is scanned directly rather than
//! inferred from the decoded pixels.

use image::ImageFormat;

/// What the container says about a raster.
#[derive(Default)]
pub struct ContainerFacts {
    /// Number of frames; one for a still image.
    pub pages: u32,
    /// EXIF orientation, when the container carries one.
    pub orientation: Option<u32>,
    /// Whether the container carries an EXIF block.
    pub exif: bool,
    /// Whether the container carries an XMP block.
    pub xmp: bool,
    /// Whether the container carries an IPTC block.
    pub iptc: bool,
    /// Whether the container carries an ICC profile.
    pub icc: bool,
    /// Whether the container carries a Photoshop tag block.
    pub photoshop: bool,
    /// Whether the container carries comments.
    pub comments: bool,
    /// Whether a GIF frame declares a transparent palette index.
    pub transparency: bool,
    /// Whether the raster carries a CMYK colour space.
    pub cmyk: bool,
}

impl ContainerFacts {
    fn still() -> Self {
        Self { pages: 1, ..Self::default() }
    }

    /// Whether the container carries descriptive metadata, a colour profile, or orientation.
    pub fn carries_metadata(&self) -> bool {
        self.exif
            || self.xmp
            || self.iptc
            || self.icc
            || self.photoshop
            || self.comments
            || self.orientation.is_some()
    }
}

/// Read the container facts of an encoded raster.
/// @param data - Complete encoded image bytes.
/// @param format - The container format the bytes carry.
/// @returns The frames, orientation, and metadata presence of the container.
pub fn scan_container(data: &[u8], format: ImageFormat) -> ContainerFacts {
    match format {
        ImageFormat::Png => scan_png(data),
        ImageFormat::Jpeg => scan_jpeg(data),
        ImageFormat::WebP => scan_webp(data),
        ImageFormat::Gif => scan_gif(data),
        _ => ContainerFacts::still(),
    }
}

/// The EXIF orientation a blob declares, when it declares one.
fn exif_orientation(blob: &[u8]) -> Option<u32> {
    let reader = exif::Reader::new();
    let exif = reader.read_raw(blob.to_vec()).ok()?;
    let field = exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?;
    field.value.get_uint(0)
}

/// The PNG chunk facts.
fn scan_png(data: &[u8]) -> ContainerFacts {
    let mut facts = ContainerFacts::still();
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
            b"eXIf" => {
                facts.exif = true;
                facts.orientation = exif_orientation(payload);
            }
            b"iCCP" => facts.icc = true,
            b"iTXt" => {
                if payload.starts_with(b"XML:com.adobe.xmp") {
                    facts.xmp = true;
                } else {
                    facts.comments = true;
                }
            }
            b"tEXt" | b"zTXt" | b"tIME" => facts.comments = true,
            _ => {}
        }
        offset = end + 4; // payload + CRC
    }
    facts
}

/// The JPEG segment facts.
fn scan_jpeg(data: &[u8]) -> ContainerFacts {
    let mut facts = ContainerFacts::still();
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
        if length < 2 {
            break;
        }
        let payload = &data[offset + 4..(offset + 2 + length).min(data.len())];
        match marker {
            0xe1 => {
                if payload.starts_with(b"Exif\0\0") {
                    facts.exif = true;
                    facts.orientation = exif_orientation(&payload[6..]);
                } else if payload.starts_with(b"http://ns.adobe.com/xap/1.0/") {
                    facts.xmp = true;
                }
            }
            0xc0 | 0xc1 | 0xc2 | 0xc3 | 0xc5 | 0xc6 | 0xc7 | 0xc9 | 0xca | 0xcb | 0xcd | 0xce
            | 0xcf => {
                // A frame header names its component count: four means CMYK.
                if payload.len() > 5 && payload[5] == 4 {
                    facts.cmyk = true;
                }
            }
            0xe2 => facts.icc = true,
            0xed => facts.iptc = true,
            0xfe => facts.comments = true,
            _ => {}
        }
        if marker == 0xda {
            // Start of scan: entropy-coded data follows, no more segments.
            break;
        }
        offset += 2 + length;
    }
    facts
}

/// The WebP RIFF chunk facts.
fn scan_webp(data: &[u8]) -> ContainerFacts {
    let mut facts = ContainerFacts { pages: 0, ..ContainerFacts::still() };
    let mut offset = 12; // RIFF header + WEBP fourcc
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
            b"EXIF" => facts.exif = true,
            b"XMP " => facts.xmp = true,
            b"ICCP" => facts.icc = true,
            b"VP8X" => {
                let flags = data[start];
                if flags & 0x08 != 0 {
                    facts.exif = true;
                }
                if flags & 0x04 != 0 {
                    facts.xmp = true;
                }
                if flags & 0x02 != 0 {
                    facts.icc = true;
                }
            }
            b"ANMF" => facts.pages += 1,
            _ => {}
        }
        offset = end + (length & 1);
    }
    facts.pages = facts.pages.max(1);
    facts
}

/// The GIF block facts.
fn scan_gif(data: &[u8]) -> ContainerFacts {
    let mut facts = ContainerFacts { pages: 0, ..ContainerFacts::still() };
    // The logical screen descriptor carries the global colour table.
    let mut offset = 13;
    if data.len() >= 10 {
        let packed = data[10];
        if packed & 0x80 != 0 {
            offset += 3 * (1 << ((packed & 0x07) + 1));
        }
    }
    while offset < data.len() {
        match data[offset] {
            0x2c => {
                facts.pages += 1;
                let packed = data.get(offset + 9).copied().unwrap_or(0);
                offset += 10; // image descriptor
                if packed & 0x80 != 0 {
                    offset += 3 * (1 << ((packed & 0x07) + 1));
                }
                offset += 1; // LZW minimum code size
                offset = skip_sub_blocks(data, offset);
            }
            0x21 => {
                let label = data.get(offset + 1).copied().unwrap_or(0);
                if label == 0xf9 {
                    // The graphic control extension declares the transparent index.
                    if data.get(offset + 3).copied().unwrap_or(0) & 0x01 != 0 {
                        facts.transparency = true;
                    }
                } else if label == 0xfe {
                    facts.comments = true;
                } else if label == 0xff {
                    let identifier = data.get(offset + 3..offset + 14);
                    if identifier == Some(b"XMP DataXMP") {
                        facts.xmp = true;
                    }
                }
                offset = skip_sub_blocks(data, offset + 2);
            }
            0x3b => break,
            _ => offset += 1,
        }
    }
    facts.pages = facts.pages.max(1);
    facts
}

/// Skip a chain of GIF sub-blocks.
fn skip_sub_blocks(data: &[u8], mut offset: usize) -> usize {
    while offset < data.len() {
        let size = data[offset] as usize;
        offset += 1;
        if size == 0 {
            return offset;
        }
        offset += size;
    }
    offset
}
