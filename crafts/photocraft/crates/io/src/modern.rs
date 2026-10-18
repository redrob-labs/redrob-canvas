//! JPEG XL (`.jxl`) and JPEG 2000 (`.jp2`, `.j2k`, `.j2c`, `.jpf`, `.jpx`) through pure-Rust
//! codecs: `jxl-oxide` decodes JPEG XL (bare codestream or container; grey, RGB, alpha, 8/16-bit
//! and float sources at 16 bits), `zune-jpegxl` writes lossless JPEG XL, and `j2k` reads and
//! writes JPEG 2000 Part 1 and HTJ2K (lossless, or lossy with a quality factor). The browser
//! build has none of them: the formats are recognised and refused with a clear message there.

use redrob_photo_codecs::{ChannelLayout, Image};
use redrob_photo_doc::Document;

use crate::{ExportOptions, ExportResult, ImportResult, IoError};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Modern {
    Jxl,
    J2k,
}

const JXL_CONTAINER: [u8; 12] = [0x00, 0x00, 0x00, 0x0C, 0x4A, 0x58, 0x4C, 0x20, 0x0D, 0x0A, 0x87, 0x0A];
const JP2_CONTAINER: [u8; 12] = [0x00, 0x00, 0x00, 0x0C, 0x6A, 0x50, 0x20, 0x20, 0x0D, 0x0A, 0x87, 0x0A];

/// Extensions of these formats (lower-case).
pub const EXTS: &[&str] = &["jxl", "jp2", "j2k", "j2c", "jpc", "jpf", "jpx", "jph"];

/// Which format `bytes` (by signature) or `name` (by extension) is.
pub fn detect(name: &str, bytes: &[u8]) -> Option<Modern> {
    if bytes.starts_with(&JXL_CONTAINER) || bytes.starts_with(&[0xFF, 0x0A]) {
        return Some(Modern::Jxl);
    }
    if bytes.starts_with(&JP2_CONTAINER) || bytes.starts_with(&[0xFF, 0x4F, 0xFF, 0x51]) {
        return Some(Modern::J2k);
    }
    let ext = name.rsplit(['/', '\\']).next().and_then(|n| n.rsplit_once('.')).map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "jxl" => Some(Modern::Jxl),
        "jp2" | "j2k" | "j2c" | "jpc" | "jpf" | "jpx" | "jph" => Some(Modern::J2k),
        _ => None,
    }
}

/// Opens a JPEG XL or JPEG 2000 file.
pub fn import(name: &str, bytes: &[u8]) -> Result<ImportResult, IoError> {
    let kind = detect(name, bytes).ok_or_else(|| IoError::UnknownFormat(name.to_string()))?;
    let img = match kind {
        Modern::Jxl => decode_jxl(bytes)?,
        Modern::J2k => decode_j2k(bytes)?,
    };
    crate::flat::image_to_document(name, &img)
}

/// Writes the flattened document as `ext` (`jxl`, or a JPEG 2000 extension).
pub fn export(doc: &Document, ext: &str, opts: &ExportOptions) -> Result<ExportResult, IoError> {
    let mut warnings = Vec::new();
    let img = crate::flat::document_to_image(doc, &mut warnings)?;
    let bytes = match ext {
        "jxl" => encode_jxl(&img, opts, &mut warnings)?,
        e => encode_j2k(&img, e, opts, &mut warnings)?,
    };
    Ok(ExportResult { bytes, warnings })
}

#[cfg(target_arch = "wasm32")]
fn unsupported(what: &str) -> IoError {
    IoError::Unsupported(format!("{what} can't be opened or saved in the browser version of Redrob Canvas; use the desktop app"))
}

#[cfg(target_arch = "wasm32")]
fn decode_jxl(_bytes: &[u8]) -> Result<Image, IoError> {
    Err(unsupported("JPEG XL"))
}
#[cfg(target_arch = "wasm32")]
fn decode_j2k(_bytes: &[u8]) -> Result<Image, IoError> {
    Err(unsupported("JPEG 2000"))
}
#[cfg(target_arch = "wasm32")]
fn encode_jxl(_img: &Image, _opts: &ExportOptions, _w: &mut Vec<String>) -> Result<Vec<u8>, IoError> {
    Err(unsupported("JPEG XL"))
}
#[cfg(target_arch = "wasm32")]
fn encode_j2k(_img: &Image, _ext: &str, _opts: &ExportOptions, _w: &mut Vec<String>) -> Result<Vec<u8>, IoError> {
    Err(unsupported("JPEG 2000"))
}

#[cfg(not(target_arch = "wasm32"))]
fn guarded<T>(what: &str, f: impl FnOnce() -> Result<T, IoError>) -> Result<T, IoError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|_| Err(IoError::Unsupported(format!("the {what} codec failed on this file"))))
}

#[cfg(not(target_arch = "wasm32"))]
fn decode_jxl(bytes: &[u8]) -> Result<Image, IoError> {
    use jxl_oxide::{AllocTracker, JxlImage, PixelFormat};
    guarded("JPEG XL", || {
        let image =
            JxlImage::builder().alloc_tracker(AllocTracker::with_limit(2 << 30)).read(bytes).map_err(|e| IoError::Unsupported(format!("JPEG XL: {e}")))?;
        let (w, h) = (image.width(), image.height());
        if w == 0 || h == 0 || u64::from(w) * u64::from(h) > 1 << 28 {
            return Err(IoError::Unsupported(format!("JPEG XL: {w} × {h} is out of range")));
        }
        let layout = match image.pixel_format() {
            PixelFormat::Gray => ChannelLayout::Gray,
            PixelFormat::Graya => ChannelLayout::GrayA,
            PixelFormat::Rgb => ChannelLayout::Rgb,
            PixelFormat::Rgba => ChannelLayout::Rgba,
            PixelFormat::Cmyk => ChannelLayout::Cmyk,
            PixelFormat::Cmyka => ChannelLayout::CmykA,
        };
        let render = image.render_frame(0).map_err(|e| IoError::Unsupported(format!("JPEG XL: {e}")))?;
        let mut stream = render.stream();
        let channels = stream.channels() as usize;
        if channels != layout.channels() {
            return Err(IoError::Unsupported(format!("JPEG XL: {channels} channels for a {layout:?} image")));
        }
        let (sw, sh) = (stream.width(), stream.height());
        let n = (sw as usize) * (sh as usize) * channels;
        let bits = image.image_header().metadata.bit_depth.bits_per_sample();
        if bits <= 8 {
            let mut buf = vec![0u8; n];
            if stream.write_to_buffer(&mut buf) != n {
                return Err(IoError::Unsupported("JPEG XL: the image is truncated".into()));
            }
            Ok(Image::from_u8(sw, sh, layout, buf)?)
        } else {
            let mut buf = vec![0u16; n];
            if stream.write_to_buffer(&mut buf) != n {
                return Err(IoError::Unsupported("JPEG XL: the image is truncated".into()));
            }
            Ok(Image::from_u16(sw, sh, layout, &buf)?)
        }
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn encode_jxl(img: &Image, _opts: &ExportOptions, warnings: &mut Vec<String>) -> Result<Vec<u8>, IoError> {
    use redrob_photo_codecs::SampleType;
    use zune_core::{bit_depth::BitDepth, colorspace::ColorSpace, options::EncoderOptions};
    let (w, h) = img.dimensions();
    if w < 2 || h < 2 {
        return Err(IoError::Unsupported("JPEG XL images must be at least 2 × 2 pixels".into()));
    }
    let (layout, space) = match img.layout() {
        ChannelLayout::Gray => (ChannelLayout::Gray, ColorSpace::Luma),
        ChannelLayout::GrayA => (ChannelLayout::GrayA, ColorSpace::LumaA),
        ChannelLayout::Rgb => (ChannelLayout::Rgb, ColorSpace::RGB),
        ChannelLayout::Rgba => (ChannelLayout::Rgba, ColorSpace::RGBA),
        other => {
            warnings.push(format!("{other:?} pixels were converted to RGB for JPEG XL"));
            (if other.has_alpha() { ChannelLayout::Rgba } else { ChannelLayout::Rgb }, if other.has_alpha() { ColorSpace::RGBA } else { ColorSpace::RGB })
        }
    };
    let (sample, depth) = match img.sample_type() {
        SampleType::U8 => (SampleType::U8, BitDepth::Eight),
        _ => (SampleType::U16, BitDepth::Sixteen),
    };
    if matches!(img.sample_type(), SampleType::F16 | SampleType::F32) {
        warnings.push("float samples were stored as 16-bit integers in JPEG XL".into());
    }
    let converted = img.convert(layout, sample);
    let data = converted.data().to_vec();
    warnings.push("JPEG XL is written lossless (the encoder has no lossy mode)".into());
    guarded("JPEG XL", || {
        let mut out = Vec::new();
        zune_jpegxl::JxlSimpleEncoder::new(&data, EncoderOptions::new(w as usize, h as usize, space, depth))
            .encode(&mut out)
            .map_err(|e| IoError::Unsupported(format!("JPEG XL: {e:?}")))?;
        Ok(out)
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn decode_j2k(_bytes: &[u8]) -> Result<Image, IoError> {
    Err(IoError::Unsupported("JPEG 2000 isn't supported yet (the pure-Rust codec needs a newer compiler than this build uses)".into()))
}

#[cfg(not(target_arch = "wasm32"))]
fn encode_j2k(_img: &Image, _ext: &str, _opts: &ExportOptions, _w: &mut Vec<String>) -> Result<Vec<u8>, IoError> {
    Err(IoError::Unsupported("JPEG 2000 isn't supported yet (the pure-Rust codec needs a newer compiler than this build uses)".into()))
}
