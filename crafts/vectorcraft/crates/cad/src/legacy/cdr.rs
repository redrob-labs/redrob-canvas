//! CorelDRAW files (`.cdr`, `.cmx`, `.ccx`, `.cdt`): RIFF containers whose form type names the
//! format and version (`CDRx`, `CMX1`, `CCXx`, `CDTx`). The drawing's own records are
//! undocumented; the container's `DISP` chunk is a documented device-independent bitmap of the
//! page (the preview every version writes), which is what opens: the page at the preview's size,
//! with a warning that the vectors need a converter. Version 14 and later (X4+) files are
//! compressed containers (`cmpr`), whose preview is read the same way when uncompressed.

use redrob_vector_geom::{Affine, Rect};

use super::{Builder, Bytes, Imported};

/// Does `bytes` start a CorelDRAW RIFF container?
pub fn is_cdr(bytes: &[u8]) -> bool {
    bytes.len() >= 12
        && &bytes[..4] == b"RIFF"
        && (&bytes[8..11] == b"CDR" || &bytes[8..11] == b"CMX" || &bytes[8..11] == b"CCX" || &bytes[8..11] == b"CDT")
}

/// The version a form type letter names (`CDR9` → 9, `CDRA` → 10 …).
fn version(form: &[u8]) -> Option<u32> {
    let c = *form.get(3)?;
    match c {
        b'1'..=b'9' => Some(u32::from(c - b'0')),
        b'A'..=b'Z' => Some(10 + u32::from(c - b'A')),
        _ => None,
    }
}

/// Walk the RIFF chunks (depth first) for the first `DISP` chunk that is a DIB (clipboard
/// format 8) → its BITMAPINFO and pixels.
fn find_disp(data: &[u8], depth: u32) -> Option<&[u8]> {
    if depth > 16 {
        return None;
    }
    let mut r = Bytes::new(data);
    while r.left() >= 8 {
        let id = r.take(4)?;
        let len = r.u32le()? as usize;
        let body = r.take(len.min(r.left()))?;
        if len % 2 == 1 {
            let _ = r.u8();
        }
        if id == b"DISP" && body.len() > 44 {
            let fmt = u32::from_le_bytes([body[0], body[1], body[2], body[3]]);
            if fmt == 8 {
                return Some(&body[4..]);
            }
        }
        if id == b"LIST"
            && body.len() >= 4
            && let Some(found) = find_disp(&body[4..], depth + 1)
        {
            return Some(found);
        }
    }
    None
}

/// A device-independent bitmap (BITMAPINFO + colour table + pixels) as a BMP file.
fn dib_to_bmp(dib: &[u8]) -> Option<Vec<u8>> {
    let header_len = u32::from_le_bytes([*dib.first()?, *dib.get(1)?, *dib.get(2)?, *dib.get(3)?]) as usize;
    if header_len < 40 || dib.len() < header_len {
        return None;
    }
    let bits = u16::from_le_bytes([dib[14], dib[15]]);
    let compression = u32::from_le_bytes([dib[16], dib[17], dib[18], dib[19]]);
    let mut colors = u32::from_le_bytes([dib[32], dib[33], dib[34], dib[35]]) as usize;
    if colors == 0 && bits <= 8 {
        colors = 1usize << bits;
    }
    let masks = if compression == 3 && header_len == 40 { 12 } else { 0 };
    let offset = 14 + header_len + colors * 4 + masks;
    let mut out = Vec::with_capacity(14 + dib.len());
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&((14 + dib.len()) as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(offset as u32).to_le_bytes());
    out.extend_from_slice(dib);
    Some(out)
}

/// Read a CorelDRAW container: its preview bitmap as the page.
pub fn import(bytes: &[u8], title: &str) -> Result<Imported, String> {
    if !is_cdr(bytes) {
        return Err("not a CorelDRAW file".into());
    }
    let form = &bytes[8..12];
    let kind = match &form[..3] {
        b"CMX" => "Corel Presentation Exchange",
        b"CCX" => "Corel clipart",
        b"CDT" => "CorelDRAW template",
        _ => "CorelDRAW",
    };
    let ver = version(form).map(|v| format!(" version {v}")).unwrap_or_default();
    if bytes.len() >= 24 && &bytes[12..16] == b"cmpr" {
        return Err(format!(
            "{kind}{ver} files are compressed containers whose drawing records aren't documented: export the drawing as SVG, PDF, EPS or EMF from CorelDRAW to open it"
        ));
    }
    let Some(dib) = find_disp(&bytes[12..], 0) else {
        return Err(format!(
            "{kind}{ver}: the drawing records aren't documented and the file has no preview bitmap: export the drawing as SVG, PDF, EPS or EMF to open it"
        ));
    };
    let bmp = dib_to_bmp(dib).ok_or("the preview bitmap can't be read")?;
    let img =
        image::load_from_memory_with_format(&bmp, image::ImageFormat::Bmp).map_err(|e| format!("the preview bitmap can't be read: {e}"))?.to_rgba8();
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return Err("the preview bitmap is empty".into());
    }
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img).write_to(&mut png, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    let mut b = Builder::new();
    b.image(png.into_inner(), w, h, Affine::IDENTITY);
    b.warn(&format!(
        "{kind}{ver}: the drawing records aren't documented, so the file opens as its {w} × {h} preview bitmap; export the drawing as SVG, PDF, EPS or EMF from CorelDRAW for its vectors"
    ));
    let board = Rect::new(0.0, 0.0, f64::from(w), f64::from(h));
    Ok(b.finish(Affine::IDENTITY, Some(board), title))
}
