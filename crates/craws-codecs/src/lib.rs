//! Format adapters. The engine never sees files or formats — ports (CLI, MCP,
//! app) decode here into plain sRGB RGBA8 and hand buffers over.
//!
//! Backed by the `image` crate (whose JPEG path is zune-jpeg — already the
//! fast lane); swapping individual formats for specialized decoders behind
//! this same API is the planned path, guided by the codec benches.

use std::io::Cursor;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
    /// Encoded losslessly (the `image` crate's webp encoder is lossless-only).
    WebP,
}

impl ImageFormat {
    /// Detect from a file extension (case-insensitive).
    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "png" => Some(Self::Png),
            "jpg" | "jpeg" => Some(Self::Jpeg),
            "webp" => Some(Self::WebP),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
            Self::WebP => "webp",
        }
    }
}

/// Decoded image: straight (non-premultiplied) sRGB RGBA8.
pub struct Decoded {
    pub width: u32,
    pub height: u32,
    pub rgba8: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("decode failed: {0}")]
    Decode(#[source] image::ImageError),
    #[error("encode failed: {0}")]
    Encode(#[source] image::ImageError),
    #[error("jpeg encode failed: {0}")]
    JpegEncode(String),
    #[error("jpeg dimensions {width}x{height} exceed the format's 65535 limit")]
    JpegTooLarge { width: u32, height: u32 },
    #[error("cannot tell the format of `{0}` — use a .png / .jpg / .webp extension")]
    UnknownFormat(String),
}

/// Decode any supported format (sniffed from the bytes, not the file name).
pub fn decode(bytes: &[u8]) -> Result<Decoded, CodecError> {
    let img = image::load_from_memory(bytes).map_err(CodecError::Decode)?;
    let rgba = img.into_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(Decoded { width, height, rgba8: rgba.into_raw() })
}

/// Encode straight sRGB RGBA8. `quality` applies to JPEG only (default 90).
/// JPEG has no alpha: translucent pixels are composited over white first.
pub fn encode(
    width: u32,
    height: u32,
    rgba8: &[u8],
    format: ImageFormat,
    quality: Option<u8>,
) -> Result<Vec<u8>, CodecError> {
    assert_eq!(rgba8.len() as u64, width as u64 * height as u64 * 4, "buffer size mismatch");
    use image::codecs::{png::PngEncoder, webp::WebPEncoder};
    use image::{ExtendedColorType, ImageEncoder};

    let mut out = Vec::new();
    match format {
        ImageFormat::Png => PngEncoder::new(Cursor::new(&mut out))
            .write_image(rgba8, width, height, ExtendedColorType::Rgba8)
            .map_err(CodecError::Encode)?,
        ImageFormat::WebP => WebPEncoder::new_lossless(Cursor::new(&mut out))
            .write_image(rgba8, width, height, ExtendedColorType::Rgba8)
            .map_err(CodecError::Encode)?,
        // jpeg-encoder (SIMD) instead of image's own encoder — ~7x faster here.
        ImageFormat::Jpeg => {
            let (w, h) = jpeg_dims(width, height)?;
            let rgb = flatten_over_white(rgba8);
            jpeg_encoder::Encoder::new(&mut out, quality.unwrap_or(90).clamp(1, 100))
                .encode(&rgb, w, h, jpeg_encoder::ColorType::Rgb)
                .map_err(|e| CodecError::JpegEncode(e.to_string()))?;
        }
    }
    Ok(out)
}

/// JPEG dimensions are 16-bit in the format; reject anything larger up front.
fn jpeg_dims(width: u32, height: u32) -> Result<(u16, u16), CodecError> {
    match (u16::try_from(width), u16::try_from(height)) {
        (Ok(w), Ok(h)) => Ok((w, h)),
        _ => Err(CodecError::JpegTooLarge { width, height }),
    }
}

/// `out = a·c + (1−a)·white`, in gamma space — the conventional export flatten.
fn flatten_over_white(rgba8: &[u8]) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(rgba8.len() / 4 * 3);
    for p in rgba8.chunks_exact(4) {
        let a = p[3] as u16;
        rgb.push(((p[0] as u16 * a + 255 * (255 - a)) / 255) as u8);
        rgb.push(((p[1] as u16 * a + 255 * (255 - a)) / 255) as u8);
        rgb.push(((p[2] as u16 * a + 255 * (255 - a)) / 255) as u8);
    }
    rgb
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checker(w: u32, h: u32) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let on = (x / 8 + y / 8) % 2 == 0;
                v.extend_from_slice(if on { &[200, 60, 30, 255] } else { &[20, 90, 160, 255] });
            }
        }
        v
    }

    #[test]
    fn png_roundtrip_exact() {
        let (w, h) = (100, 60);
        let px = checker(w, h);
        let bytes = encode(w, h, &px, ImageFormat::Png, None).unwrap();
        let d = decode(&bytes).unwrap();
        assert_eq!((d.width, d.height), (w, h));
        assert_eq!(d.rgba8, px);
    }

    #[test]
    fn webp_lossless_roundtrip_exact() {
        let (w, h) = (64, 48);
        let px = checker(w, h);
        let bytes = encode(w, h, &px, ImageFormat::WebP, None).unwrap();
        let d = decode(&bytes).unwrap();
        assert_eq!(d.rgba8, px);
    }

    #[test]
    fn jpeg_roundtrip_close_on_flat_color() {
        let (w, h) = (32, 32);
        let px: Vec<u8> = [120u8, 80, 200, 255].repeat((w * h) as usize);
        let bytes = encode(w, h, &px, ImageFormat::Jpeg, Some(95)).unwrap();
        let d = decode(&bytes).unwrap();
        for (got, want) in d.rgba8.chunks_exact(4).zip(px.chunks_exact(4)) {
            for i in 0..3 {
                assert!((got[i] as i16 - want[i] as i16).abs() <= 4, "{got:?} vs {want:?}");
            }
            assert_eq!(got[3], 255);
        }
    }

    #[test]
    fn jpeg_flattens_transparency_over_white() {
        let px = [0u8, 0, 0, 0]; // fully transparent black
        let bytes = encode(1, 1, &px, ImageFormat::Jpeg, Some(100)).unwrap();
        let d = decode(&bytes).unwrap();
        assert!(d.rgba8[0] > 250 && d.rgba8[1] > 250 && d.rgba8[2] > 250, "{:?}", &d.rgba8[..3]);
    }

    #[test]
    fn format_detection() {
        assert_eq!(ImageFormat::from_path(Path::new("a/b/photo.JPG")), Some(ImageFormat::Jpeg));
        assert_eq!(ImageFormat::from_path(Path::new("x.webp")), Some(ImageFormat::WebP));
        assert_eq!(ImageFormat::from_path(Path::new("x.png")), Some(ImageFormat::Png));
        assert_eq!(ImageFormat::from_path(Path::new("x.tiff")), None);
        assert_eq!(ImageFormat::from_path(Path::new("noext")), None);
    }
}
