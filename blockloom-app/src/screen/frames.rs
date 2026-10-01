//! Turning what a device sends into what the editor shows: PNG framing off
//! a pipe, and a downscaled JPEG data URL out of any decoded picture.

use base64::Engine;
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ExtendedColorType, RgbImage};
use std::io::{self, Read};

const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
/// A chunk longer than this is a corrupt length, not a screenshot: a 4K
/// RGBA frame is under 40 MB even uncompressed.
const MAX_CHUNK: usize = 64 * 1024 * 1024;
/// JPEG quality of a streamed frame: the panel is small and the frames many.
const JPEG_QUALITY: u8 = 78;

/// Reads whole PNG files one after another off a pipe. Every file ends at
/// its `IEND` chunk, so a stream of them needs no length prefix, and bytes
/// that aren't a PNG (a shell banner, a warning) are skipped to the next
/// signature instead of ending the stream.
pub struct PngStream<R: Read> {
    reader: io::BufReader<R>,
}

impl<R: Read> PngStream<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader: io::BufReader::with_capacity(1 << 20, reader),
        }
    }

    /// The next PNG's bytes, or None at the end of the stream.
    pub fn next_png(&mut self) -> io::Result<Option<Vec<u8>>> {
        if !self.find_signature()? {
            return Ok(None);
        }
        let mut png = PNG_SIGNATURE.to_vec();
        loop {
            let mut head = [0u8; 8];
            self.reader.read_exact(&mut head)?;
            let len = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
            if len > MAX_CHUNK {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "corrupt PNG chunk",
                ));
            }
            let start = png.len();
            png.extend_from_slice(&head);
            // The data and its CRC.
            png.resize(start + 8 + len + 4, 0);
            self.reader.read_exact(&mut png[start + 8..])?;
            if &head[4..8] == b"IEND" {
                return Ok(Some(png));
            }
        }
    }

    /// Consumes up to and including the next PNG signature.
    fn find_signature(&mut self) -> io::Result<bool> {
        let mut matched = 0;
        let mut byte = [0u8; 1];
        loop {
            if self.reader.read(&mut byte)? == 0 {
                return Ok(false);
            }
            matched = if byte[0] == PNG_SIGNATURE[matched] {
                matched + 1
            } else if byte[0] == PNG_SIGNATURE[0] {
                1
            } else {
                0
            };
            if matched == PNG_SIGNATURE.len() {
                return Ok(true);
            }
        }
    }
}

/// The pixel size a PNG's header declares, without decoding it.
pub fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    // Signature, then IHDR: length, "IHDR", width, height.
    if png.len() < 24 || png[..8] != PNG_SIGNATURE || &png[12..16] != b"IHDR" {
        return None;
    }
    let word = |at: usize| u32::from_be_bytes([png[at], png[at + 1], png[at + 2], png[at + 3]]);
    Some((word(16), word(20)))
}

/// A picture fitted to `max_width` and packed as a JPEG data URL, with the
/// size it ended up at.
pub struct Packed {
    pub url: String,
    pub width: u32,
    pub height: u32,
}

/// Downscales `shot` to at most `max_width` wide (never up) and packs it.
pub fn pack(shot: &DynamicImage, max_width: u32) -> Result<Packed, String> {
    let width = max_width.min(shot.width()).max(1);
    let small = if width < shot.width() {
        shot.thumbnail(width, u32::MAX).to_rgb8()
    } else {
        shot.to_rgb8()
    };
    pack_rgb(&small)
}

/// Packs raw RGB pixels as a JPEG data URL.
pub fn pack_rgb(rgb: &RgbImage) -> Result<Packed, String> {
    let mut jpeg = Vec::with_capacity(32 * 1024);
    JpegEncoder::new_with_quality(&mut jpeg, JPEG_QUALITY)
        .encode(
            rgb.as_raw(),
            rgb.width(),
            rgb.height(),
            ExtendedColorType::Rgb8,
        )
        .map_err(|e| format!("Couldn't pack the frame: {e}"))?;
    Ok(Packed {
        url: format!(
            "data:image/jpeg;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&jpeg)
        ),
        width: rgb.width(),
        height: rgb.height(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32, shade: u8) -> Vec<u8> {
        let shot = image::RgbaImage::from_pixel(width, height, image::Rgba([shade, 40, 90, 255]));
        let mut bytes = Vec::new();
        DynamicImage::ImageRgba8(shot)
            .write_to(&mut io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn a_stream_of_pngs_splits_at_each_iend() {
        let (a, b) = (png(8, 4, 10), png(6, 6, 200));
        let mut wire = Vec::new();
        // Shell noise before and between files is skipped.
        wire.extend_from_slice(b"warning: \x89P stray\n");
        wire.extend_from_slice(&a);
        wire.extend_from_slice(b"\n");
        wire.extend_from_slice(&b);
        let mut stream = PngStream::new(io::Cursor::new(wire));
        assert_eq!(stream.next_png().unwrap().unwrap(), a);
        assert_eq!(stream.next_png().unwrap().unwrap(), b);
        assert!(stream.next_png().unwrap().is_none());
    }

    #[test]
    fn a_stream_cut_mid_file_is_an_error_not_a_frame() {
        let mut wire = png(8, 4, 10);
        wire.truncate(wire.len() - 6);
        let mut stream = PngStream::new(io::Cursor::new(wire));
        assert!(stream.next_png().is_err());
    }

    #[test]
    fn the_size_comes_from_the_header() {
        assert_eq!(png_size(&png(8, 4, 0)), Some((8, 4)));
        assert_eq!(png_size(b"not a png at all, not at all"), None);
    }

    #[test]
    fn frames_shrink_to_the_width_and_never_grow() {
        let shot = image::load_from_memory(&png(1080, 2400, 120)).unwrap();
        let small = pack(&shot, 360).unwrap();
        assert_eq!((small.width, small.height), (360, 800));
        assert!(small.url.starts_with("data:image/jpeg;base64,"));
        let same = pack(&shot, 4000).unwrap();
        assert_eq!((same.width, same.height), (1080, 2400));
    }
}
