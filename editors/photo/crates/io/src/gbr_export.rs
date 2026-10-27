//! Resource file export: the image as a bitmap brush (`.gbr`), the layers as an animated image
//! hose (`.gih`, one cell per layer), or the image as a pattern (`.gpat`, also written for a
//! `.pat` when asked for the open brush-resource pattern format). The headers mirror the public
//! format descriptions the readers in [`crate::gbr`] follow: big-endian header size, version,
//! width, height, bytes per pixel, the four-byte magic, spacing (brushes), then the
//! NUL-terminated name and the pixels.

use redrob_photo_doc::Document;

use crate::{ExportResult, IoError};

/// How a brush's pixels are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrushDepth {
    /// One byte per pixel: the paint mask (255 paints), from darkness times alpha.
    Mask,
    /// Four bytes per pixel: a colour brush.
    Color,
}

fn mask_of(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4)
        .map(|p| {
            let lum = (299 * u32::from(p[0]) + 587 * u32::from(p[1]) + 114 * u32::from(p[2])) / 1000;
            ((255 - lum) * u32::from(p[3]) / 255) as u8
        })
        .collect()
}

fn header(version: u32, w: u32, h: u32, bpp: u32, magic: &[u8; 4], spacing: Option<u32>, name: &str) -> Vec<u8> {
    let name: Vec<u8> = name.bytes().filter(|&b| b != 0).take(255).collect();
    let fixed = if spacing.is_some() { 28 } else { 24 };
    let size = fixed + name.len() + 1;
    let mut out = Vec::with_capacity(size);
    out.extend((size as u32).to_be_bytes());
    out.extend(version.to_be_bytes());
    out.extend(w.to_be_bytes());
    out.extend(h.to_be_bytes());
    out.extend(bpp.to_be_bytes());
    out.extend_from_slice(magic);
    if let Some(s) = spacing {
        out.extend(s.to_be_bytes());
    }
    out.extend(name);
    out.push(0);
    out
}

/// One `.gbr` brush: version 2, spacing in percent of the diameter.
pub fn write_gbr(name: &str, w: u32, h: u32, rgba: &[u8], spacing_pct: u32, depth: BrushDepth) -> Result<Vec<u8>, IoError> {
    if w == 0 || h == 0 || w > crate::gbr::MAX_SIDE || h > crate::gbr::MAX_SIDE {
        return Err(IoError::Unsupported(format!("brush size {w} × {h} is out of range")));
    }
    if rgba.len() != (w as usize) * (h as usize) * 4 {
        return Err(IoError::Unsupported("brush pixel data does not match its size".into()));
    }
    let bpp = match depth {
        BrushDepth::Mask => 1,
        BrushDepth::Color => 4,
    };
    let mut out = header(2, w, h, bpp, b"GIMP", Some(spacing_pct.clamp(1, 1000)), name);
    match depth {
        BrushDepth::Mask => out.extend(mask_of(rgba)),
        BrushDepth::Color => out.extend_from_slice(rgba),
    }
    Ok(out)
}

/// The cell selection mode of a hose dimension, as the header spells it.
pub const GIH_SELECTIONS: &[&str] = &["incremental", "angular", "random", "velocity", "pressure", "xtilt", "ytilt"];

/// A `.gih` of `cells` (all `w` × `h`): the two text header lines, then one `.gbr` per cell. One
/// dimension of `cells.len()` ranks with `selection`; `step` is the spacing in percent.
pub fn write_gih(name: &str, w: u32, h: u32, cells: &[Vec<u8>], step_pct: u32, selection: &str, depth: BrushDepth) -> Result<Vec<u8>, IoError> {
    if cells.is_empty() || cells.len() > crate::gbr::MAX_CELLS {
        return Err(IoError::Unsupported(format!("an image hose needs 1 to {} cells", crate::gbr::MAX_CELLS)));
    }
    let selection = if GIH_SELECTIONS.contains(&selection) { selection } else { "incremental" };
    let n = cells.len();
    let clean: String = name.chars().filter(|c| *c != '\n' && *c != '\r').collect();
    let mut out = format!(
        "{clean}\n{n} ncells:{n} cellwidth:{w} cellheight:{h} step:{step} dim:1 cols:1 rows:1 placement:constant rank0:{n} sel0:{selection}\n",
        step = step_pct.clamp(1, 1000)
    )
    .into_bytes();
    for (i, cell) in cells.iter().enumerate() {
        out.extend(write_gbr(&format!("{clean} {}", i + 1), w, h, cell, step_pct, depth)?);
    }
    Ok(out)
}

/// A pattern file (`GPAT`, version 1): 3 bytes per pixel when opaque, 4 with alpha.
pub fn write_gpat(name: &str, w: u32, h: u32, rgba: &[u8]) -> Result<Vec<u8>, IoError> {
    if w == 0 || h == 0 || w > crate::gbr::MAX_SIDE || h > crate::gbr::MAX_SIDE {
        return Err(IoError::Unsupported(format!("pattern size {w} × {h} is out of range")));
    }
    let alpha = rgba.chunks_exact(4).any(|p| p[3] != 255);
    let bpp = if alpha { 4 } else { 3 };
    let mut out = header(1, w, h, bpp, b"GPAT", None, name);
    if alpha {
        out.extend_from_slice(rgba);
    } else {
        out.extend(rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]));
    }
    Ok(out)
}

/// Is `bytes` a `GPAT` pattern file?
pub fn is_gpat(bytes: &[u8]) -> bool {
    bytes.len() >= 24 && &bytes[20..24] == b"GPAT"
}

/// A `GPAT` pattern file as (name, width, height, RGBA pixels).
pub fn read_gpat(bytes: &[u8]) -> Result<(String, u32, u32, Vec<u8>), IoError> {
    let u = |at: usize| -> Result<u32, IoError> {
        let s = bytes.get(at..at + 4).ok_or_else(|| IoError::Unsupported("pattern header ends early".into()))?;
        Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    };
    if !is_gpat(bytes) {
        return Err(IoError::Unsupported("not a GPAT pattern file".into()));
    }
    let size = u(0)? as usize;
    let (w, h, bpp) = (u(8)?, u(12)?, u(16)?);
    if size < 25 || size > bytes.len() || w == 0 || h == 0 || w > crate::gbr::MAX_SIDE || h > crate::gbr::MAX_SIDE {
        return Err(IoError::Unsupported("bad pattern header".into()));
    }
    let name_bytes = &bytes[24..size];
    let end = name_bytes.iter().position(|&b| b == 0).unwrap_or(name_bytes.len());
    let name = String::from_utf8_lossy(&name_bytes[..end]).trim().to_string();
    let n = (w as usize) * (h as usize);
    let px = bytes.get(size..size + n * bpp as usize).ok_or_else(|| IoError::Unsupported("pattern pixels end early".into()))?;
    let rgba: Vec<u8> = match bpp {
        1 => px.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        2 => px.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        3 => px.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        4 => px.to_vec(),
        b => return Err(IoError::Unsupported(format!("{b} bytes per pixel"))),
    };
    Ok((name, w, h, rgba))
}

/// The document's flattened image as a `.gbr`.
pub fn export_gbr(doc: &Document, depth: BrushDepth, spacing_pct: u32) -> Result<ExportResult, IoError> {
    let mut warnings = Vec::new();
    let (w, h, rgba) = crate::legacy::flatten(doc, &mut warnings)?;
    let name = doc.name.rsplit_once('.').map_or(doc.name.as_str(), |(a, _)| a);
    let bytes = write_gbr(name, w, h, &rgba, spacing_pct, depth)?;
    Ok(ExportResult { bytes, warnings })
}

/// The document's layers as the cells of a `.gih` (one layer = one cell, bottom to top).
pub fn export_gih(doc: &Document, depth: BrushDepth, spacing_pct: u32, selection: &str) -> Result<ExportResult, IoError> {
    let mut warnings = Vec::new();
    // A hose cell is one layer alone, whatever the frame tags say.
    let (w, h) = (doc.size.width, doc.size.height);
    let visible: Vec<usize> = doc.layers.iter().enumerate().filter(|(_, l)| l.visible).map(|(i, _)| i).collect();
    let mut cells = Vec::with_capacity(visible.len().max(1));
    if visible.len() <= 1 {
        cells.push(crate::legacy::flatten(doc, &mut warnings)?.2);
    } else {
        for &i in &visible {
            let mut d = doc.clone();
            for (j, l) in d.layers.iter_mut().enumerate() {
                l.visible = j == i;
            }
            cells.push(crate::legacy::flatten(&d, &mut warnings)?.2);
        }
    }
    let name = doc.name.rsplit_once('.').map_or(doc.name.as_str(), |(a, _)| a);
    let bytes = write_gih(name, w, h, &cells, spacing_pct, selection, depth)?;
    Ok(ExportResult { bytes, warnings })
}

/// The document's flattened image as a `GPAT` pattern.
pub fn export_gpat(doc: &Document) -> Result<ExportResult, IoError> {
    let mut warnings = Vec::new();
    let (w, h, rgba) = crate::legacy::flatten(doc, &mut warnings)?;
    let name = doc.name.rsplit_once('.').map_or(doc.name.as_str(), |(a, _)| a);
    let bytes = write_gpat(name, w, h, &rgba)?;
    Ok(ExportResult { bytes, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gbr_round_trips_through_the_reader() {
        let rgba = vec![0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 128, 255, 0, 0, 255];
        let b = write_gbr("Dots", 2, 2, &rgba, 25, BrushDepth::Mask).unwrap_or_default();
        assert!(crate::gbr::looks_like_gbr(&b));
        let (brush, used) = crate::gbr::parse_gbr(&b).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(used, b.len());
        assert_eq!(brush.name, "Dots");
        assert_eq!((brush.width, brush.height), (2, 2));
        assert_eq!(brush.spacing, Some(0.25));
        assert!(!brush.color);
        let c = write_gbr("Dots", 2, 2, &rgba, 25, BrushDepth::Color).unwrap_or_default();
        let (brush, _) = crate::gbr::parse_gbr(&c).unwrap_or_else(|e| panic!("{e}"));
        assert!(brush.color);
    }

    #[test]
    fn gih_header_is_read_back() {
        let cell = vec![0, 0, 0, 255, 255, 255, 255, 255, 0, 0, 0, 128, 255, 0, 0, 255];
        let b = write_gih("Hose", 2, 2, &[cell.clone(), cell], 30, "random", BrushDepth::Mask).unwrap_or_default();
        let (h, _) = crate::gbr::parse_gih_header(&b).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(h.name, "Hose");
        assert_eq!(h.ncells, 2);
        let imp = crate::gbr::read_gih(&b, "g", "Hose").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(imp.presets.len(), 1);
    }

    #[test]
    fn gpat_round_trips() {
        let rgba = vec![1, 2, 3, 255, 4, 5, 6, 255];
        let b = write_gpat("Pat", 2, 1, &rgba).unwrap_or_default();
        let (name, w, h, back) = read_gpat(&b).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!((name.as_str(), w, h), ("Pat", 2, 1));
        assert_eq!(back, rgba);
    }
}
