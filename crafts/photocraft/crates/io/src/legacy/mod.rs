//! The long tail of legacy and scientific raster formats, read and written here rather than in
//! the codec crate: X BitMap (`.xbm`), Windows cursors (`.cur`) and animated cursors (`.ani`),
//! SGI (`.sgi`/`.rgb`/`.bw`), Alias PIX (`.pix`/`.als`), KiSS cels (`.cel`), Autodesk animations
//! (`.fli`/`.flc`), X Window dumps (`.xwd`), DICOM (`.dcm`), Windows metafiles (`.wmf`, read
//! only), and the C source and HTML table exports. Every reader produces 8-bit pixels; every
//! writer takes the flattened document. All formats are implemented from their public
//! specifications.

use redrob_photo_codecs::{ChannelLayout, Image};
use redrob_photo_color::ColorMode;
use redrob_photo_doc::Document;

use crate::{ExportOptions, ExportResult, ImportResult, IoError};

mod cel;
mod csource;
mod cursor;
mod dicom;
mod fli;
mod pix;
mod sgi;
mod wmf;
mod xbm;
mod xwd;

pub use cursor::ani_frames;
pub use fli::fli_frames;

/// Pixels decoded by one of these readers: straight 8-bit RGBA, row-major.
pub(crate) struct Rgba {
    pub w: u32,
    pub h: u32,
    pub data: Vec<u8>,
    /// A palette the file carried (the document opens in Indexed Color with it).
    pub palette: Option<Vec<[u8; 3]>>,
    pub dpi: Option<f32>,
    pub warnings: Vec<String>,
}

impl Rgba {
    pub(crate) fn new(w: u32, h: u32) -> Result<Self, IoError> {
        if w == 0 || h == 0 {
            return Err(IoError::Unsupported("the image is empty".into()));
        }
        if (w as u64) * (h as u64) > MAX_PIXELS {
            return Err(IoError::Unsupported(format!("{w} × {h} is too large to open")));
        }
        let data = crate::flat::try_buffer((w as usize) * (h as usize), 4)?;
        Ok(Rgba { w, h, data, palette: None, dpi: None, warnings: Vec::new() })
    }

    pub(crate) fn put(&mut self, x: u32, y: u32, px: [u8; 4]) {
        if x < self.w && y < self.h {
            let i = ((y as usize) * (self.w as usize) + x as usize) * 4;
            self.data[i..i + 4].copy_from_slice(&px);
        }
    }
}

/// Decoders refuse anything bigger than this (pixels).
pub(crate) const MAX_PIXELS: u64 = 1 << 28;

fn bad(what: &str, msg: impl std::fmt::Display) -> IoError {
    IoError::Unsupported(format!("{what}: {msg}"))
}

fn ext(name: &str) -> String {
    name.rsplit(['/', '\\']).next().and_then(|n| n.rsplit_once('.')).map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

/// Extensions these readers open.
pub const IMPORT_EXTS: &[&str] =
    &["xbm", "cur", "ani", "sgi", "rgb", "rgba", "bw", "int", "inta", "pix", "als", "cel", "fli", "flc", "xwd", "dcm", "dicom", "wmf"];

/// Extensions these writers produce.
pub const EXPORT_EXTS: &[&str] = &["xbm", "cur", "ani", "sgi", "rgb", "bw", "pix", "als", "cel", "fli", "flc", "xwd", "dcm", "c", "h", "html", "htm"];

/// Does one of these readers claim `bytes` (by magic) or `name` (by extension)?
pub fn is_legacy_input(name: &str, bytes: &[u8]) -> bool {
    detect(name, bytes).is_some()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Xbm,
    Cur,
    Ani,
    Sgi,
    Pix,
    Cel,
    Fli,
    Xwd,
    Dicom,
    Wmf,
}

fn detect(name: &str, bytes: &[u8]) -> Option<Kind> {
    // Magic first: a wrong extension never hides a recognisable file.
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"ACON" {
        return Some(Kind::Ani);
    }
    if bytes.len() >= 6 && bytes[0..4] == [0, 0, 2, 0] && (bytes[4] != 0 || bytes[5] != 0) {
        return Some(Kind::Cur);
    }
    if bytes.starts_with(&[0x01, 0xDA]) {
        return Some(Kind::Sgi);
    }
    if bytes.len() >= 132 && &bytes[128..132] == b"DICM" {
        return Some(Kind::Dicom);
    }
    if bytes.len() >= 6 && bytes[4..6] == [0x11, 0xAF] || bytes.len() >= 6 && bytes[4..6] == [0x12, 0xAF] {
        return Some(Kind::Fli);
    }
    if bytes.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A]) || (bytes.len() >= 18 && bytes[0..2] == [1, 0] && bytes[2..4] == [9, 0]) {
        return Some(Kind::Wmf);
    }
    if bytes.starts_with(b"KiSS") {
        return Some(Kind::Cel);
    }
    if xbm::is_xbm(bytes) {
        return Some(Kind::Xbm);
    }
    match ext(name).as_str() {
        "xbm" => Some(Kind::Xbm),
        "cur" => Some(Kind::Cur),
        "ani" => Some(Kind::Ani),
        "sgi" | "rgb" | "rgba" | "bw" | "int" | "inta" => Some(Kind::Sgi),
        "pix" | "als" => Some(Kind::Pix),
        "cel" => Some(Kind::Cel),
        "fli" | "flc" => Some(Kind::Fli),
        "xwd" => Some(Kind::Xwd),
        "dcm" | "dicom" => Some(Kind::Dicom),
        "wmf" => Some(Kind::Wmf),
        _ => None,
    }
}

/// Opens `bytes` as one of these formats; `None` when none claims it.
pub fn import(name: &str, bytes: &[u8]) -> Option<Result<ImportResult, IoError>> {
    let kind = detect(name, bytes)?;
    Some(import_kind(kind, name, bytes))
}

fn import_kind(kind: Kind, name: &str, bytes: &[u8]) -> Result<ImportResult, IoError> {
    let rgba = match kind {
        Kind::Xbm => xbm::decode(bytes)?,
        Kind::Cur => cursor::decode_cur(bytes)?,
        Kind::Ani => cursor::decode_ani(bytes)?,
        Kind::Sgi => sgi::decode(bytes)?,
        Kind::Pix => pix::decode(bytes)?,
        Kind::Cel => cel::decode(bytes)?,
        Kind::Fli => fli::decode(bytes)?,
        Kind::Xwd => xwd::decode(bytes)?,
        Kind::Dicom => dicom::decode(bytes)?,
        Kind::Wmf => wmf::decode(bytes)?,
    };
    to_document(name, rgba)
}

pub(crate) fn to_document(name: &str, rgba: Rgba) -> Result<ImportResult, IoError> {
    let Rgba { w, h, data, palette, dpi, warnings } = rgba;
    let opaque = data.chunks_exact(4).all(|p| p[3] == 255);
    let img = if opaque {
        let rgb: Vec<u8> = data.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
        Image::from_u8(w, h, ChannelLayout::Rgb, rgb)?
    } else {
        Image::from_u8(w, h, ChannelLayout::Rgba, data)?
    };
    let mut r = crate::flat::image_to_document(name, &img)?;
    r.warnings.extend(warnings);
    if let Some(dpi) = dpi.filter(|d| d.is_finite() && *d > 0.0) {
        r.document.resolution_dpi = dpi;
    }
    if let Some(colors) = palette.filter(|c| !c.is_empty() && c.len() <= 256)
        && r.document.mode == ColorMode::Rgb
        && opaque
    {
        r.document.mode = ColorMode::Indexed;
        r.document.color_table = Some(redrob_photo_doc::ColorTable { colors, transparent: None });
    }
    r.source_read_only = matches!(kind_of_name(name), Some(Kind::Wmf));
    Ok(r)
}

fn kind_of_name(name: &str) -> Option<Kind> {
    detect(name, &[])
}

/// The flattened document as straight RGBA8 plus its size.
pub(crate) fn flatten(doc: &Document, warnings: &mut Vec<String>) -> Result<(u32, u32, Vec<u8>), IoError> {
    let img = crate::flat::document_to_image(doc, warnings)?;
    let (w, h) = img.dimensions();
    Ok((w, h, img.to_rgba8()))
}

/// A 256-colour palette and indices for `rgba` (an indexed document keeps its own table).
pub(crate) fn palettize(doc: &Document, w: u32, h: u32, rgba: &[u8], max: usize) -> Result<(Vec<[u8; 3]>, Vec<u8>), IoError> {
    use redrob_photo_algo::quantize::{self, Dither, Forced, PaletteKind};
    let mut px: Vec<[f32; 4]> = rgba.chunks_exact(4).map(|p| [f32::from(p[0]) / 255.0, f32::from(p[1]) / 255.0, f32::from(p[2]) / 255.0, 1.0]).collect();
    let pal = match doc.color_table.as_ref().filter(|t| doc.mode == ColorMode::Indexed && !t.colors.is_empty() && t.colors.len() <= max) {
        Some(t) => t.colors.clone(),
        None => quantize::build_palette(&px, PaletteKind::Adaptive, max.clamp(2, 256), Forced::None).map_err(IoError::Unsupported)?,
    };
    let idx = quantize::quantize(&mut px, w as usize, &pal, Dither::None, 1.0, None);
    debug_assert_eq!(idx.len(), (w as usize) * (h as usize));
    Ok((pal, idx))
}

/// Writes `doc` as the format `ext` names; `None` when none of these writers handles it.
pub fn export(doc: &Document, ext: &str, opts: &ExportOptions) -> Option<Result<ExportResult, IoError>> {
    let ext = ext.to_ascii_lowercase();
    if !EXPORT_EXTS.contains(&ext.as_str()) {
        return None;
    }
    Some(export_ext(doc, &ext, opts))
}

fn export_ext(doc: &Document, ext: &str, _opts: &ExportOptions) -> Result<ExportResult, IoError> {
    let mut warnings = Vec::new();
    let (w, h, rgba) = flatten(doc, &mut warnings)?;
    let stem = doc.name.rsplit_once('.').map_or(doc.name.as_str(), |(a, _)| a);
    let bytes = match ext {
        "xbm" => xbm::encode(stem, w, h, &rgba, &mut warnings)?,
        "cur" => cursor::encode_cur(w, h, &rgba, None, &mut warnings)?,
        "ani" => cursor::encode_ani(doc, w, h, &rgba, &mut warnings)?,
        "sgi" | "rgb" | "bw" => sgi::encode(stem, w, h, &rgba, ext == "bw" || doc.mode == ColorMode::Grayscale, &mut warnings)?,
        "pix" | "als" => pix::encode(w, h, &rgba, doc.mode == ColorMode::Grayscale, &mut warnings)?,
        "cel" => cel::encode(doc, w, h, &rgba, &mut warnings)?,
        "fli" | "flc" => fli::encode(doc, w, h, &rgba, ext == "flc", &mut warnings)?,
        "xwd" => xwd::encode(stem, w, h, &rgba, &mut warnings)?,
        "dcm" => dicom::encode(w, h, &rgba, doc.mode == ColorMode::Grayscale, &mut warnings)?,
        "c" | "h" => csource::encode_c(stem, w, h, &rgba, &mut warnings),
        "html" | "htm" => csource::encode_html(stem, w, h, &rgba, &mut warnings)?,
        _ => return Err(IoError::UnknownFormat(ext.to_string())),
    };
    Ok(ExportResult { bytes, warnings })
}

/// Little-endian helpers shared by the binary formats.
pub(crate) fn le16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(at)?, *b.get(at + 1)?]))
}
pub(crate) fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes([*b.get(at)?, *b.get(at + 1)?, *b.get(at + 2)?, *b.get(at + 3)?]))
}
pub(crate) fn be16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*b.get(at)?, *b.get(at + 1)?]))
}
pub(crate) fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes([*b.get(at)?, *b.get(at + 1)?, *b.get(at + 2)?, *b.get(at + 3)?]))
}

/// Grey value of a straight RGBA pixel (Rec. 601 luma).
pub(crate) fn luma(p: &[u8]) -> u8 {
    ((u32::from(p[0]) * 299 + u32::from(p[1]) * 587 + u32::from(p[2]) * 114) / 1000) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc_from(w: u32, h: u32, rgba: Vec<u8>) -> Document {
        let r = to_document("t.png", Rgba { w, h, data: rgba, palette: None, dpi: None, warnings: Vec::new() }).unwrap_or_else(|e| panic!("{e}"));
        r.document
    }

    fn checker(w: u32, h: u32) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let on = (x + y) % 2 == 0;
                v.extend(if on { [200, 30, 60, 255] } else { [10, 120, 240, 255] });
            }
        }
        v
    }

    #[test]
    fn every_writer_round_trips_through_its_reader() {
        let (w, h) = (9u32, 5u32);
        let px = checker(w, h);
        let doc = doc_from(w, h, px.clone());
        let opts = ExportOptions::default();
        for ext in ["cur", "ani", "sgi", "pix", "cel", "fli", "flc", "xwd", "dcm"] {
            let out = export(&doc, ext, &opts).unwrap_or_else(|| panic!("{ext}: no writer")).unwrap_or_else(|e| panic!("{ext}: {e}"));
            let back = import(&format!("t.{ext}"), &out.bytes).unwrap_or_else(|| panic!("{ext}: no reader")).unwrap_or_else(|e| panic!("{ext}: {e}"));
            assert_eq!((back.document.size.width, back.document.size.height), (w, h), "{ext}: size");
            let mut warnings = Vec::new();
            let (_, _, got) = flatten(&back.document, &mut warnings).unwrap_or_else(|e| panic!("{ext}: {e}"));
            assert_eq!(got, px, "{ext}: pixels");
        }
    }

    #[test]
    fn xbm_is_one_bit_and_round_trips_black_and_white() {
        let (w, h) = (10u32, 3u32);
        let mut px = Vec::new();
        for y in 0..h {
            for x in 0..w {
                px.extend(if (x * 7 + y) % 3 == 0 { [0, 0, 0, 255] } else { [255, 255, 255, 255] });
            }
        }
        let doc = doc_from(w, h, px.clone());
        let out = export(&doc, "xbm", &ExportOptions::default()).unwrap_or_else(|| panic!("no xbm writer")).unwrap_or_else(|e| panic!("{e}"));
        let text = String::from_utf8_lossy(&out.bytes);
        assert!(text.contains("#define t_width 10"), "{text}");
        let back = import("t.xbm", &out.bytes).unwrap_or_else(|| panic!("no reader")).unwrap_or_else(|e| panic!("{e}"));
        let (_, _, got) = flatten(&back.document, &mut Vec::new()).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(got, px);
    }

    #[test]
    fn c_source_and_html_describe_the_pixels() {
        let doc = doc_from(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]);
        let c = export(&doc, "c", &ExportOptions::default()).unwrap_or_else(|| panic!("no c writer")).unwrap_or_else(|e| panic!("{e}"));
        let c = String::from_utf8_lossy(&c.bytes);
        assert!(c.contains("width") && c.contains("\\377\\0\\0") || c.contains("0xff"), "{c}");
        let html = export(&doc, "html", &ExportOptions::default()).unwrap_or_else(|| panic!("no html writer")).unwrap_or_else(|e| panic!("{e}"));
        let html = String::from_utf8_lossy(&html.bytes);
        assert!(html.contains("<table") && html.contains("#ff0000") && html.contains("#0000ff"), "{html}");
    }
}
