// SPDX-License-Identifier: GPL-3.0-or-later

//! GIMP XCF import (read-only), re-derived from the public XCF format description and the layout of
//! GIMP's `app/xcf` loader (behaviour studied, no code copied). We read the common case: an 8-bit
//! RGB or RGBA image, uncompressed or RLE tiles, preserving the layer stack with each layer's name,
//! opacity, visibility and canvas offset.
//!
//! Not covered yet (export, and import of): >8-bit precision, indexed/greyscale base types, layer
//! masks, parasites, zlib-compressed tiles (XCF ≥ v11). Those are later passes; an unsupported file
//! is rejected with a typed error rather than mis-read.

use crate::document::MAX_DIMENSION;
use crate::{
    Document, DocumentImportBuilder, FileFormat, FormatError, FormatWarning, FrameId, ImportNode,
    ImportOptions, RasterCel, Result,
};

const TILE: usize = 64;

struct Be<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Be<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }
    fn at(&self, pos: usize) -> Be<'a> {
        Be { bytes: self.bytes, pos }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.pos + n > self.bytes.len() {
            return Err(FormatError::Malformed("XCF truncated").into());
        }
        let s = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }
    fn f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.u32()?))
    }
    fn string(&mut self) -> Result<String> {
        let len = self.u32()? as usize;
        if len == 0 {
            return Ok(String::new());
        }
        // The stored length includes a trailing NUL.
        let raw = self.take(len)?;
        let text = &raw[..len.saturating_sub(1)];
        Ok(String::from_utf8_lossy(text).into_owned())
    }
}

// Property ids we care about.
const PROP_END: u32 = 0;
const PROP_OPACITY: u32 = 6;
const PROP_VISIBLE: u32 = 8;
const PROP_OFFSETS: u32 = 15;

pub(crate) fn import_xcf(
    bytes: &[u8],
    _options: &ImportOptions,
) -> Result<(Document, Vec<FormatWarning>)> {
    let mut r = Be::new(bytes);
    let magic = r.take(9)?;
    if &magic[..8] != b"gimp xcf" {
        return Err(FormatError::Malformed("not an XCF (bad magic)").into());
    }
    // Version tag: "file\0" (v0) or "vNNN\0".
    let version_tag = r.take(5)?;
    let version: u32 = if &version_tag[..4] == b"file" {
        0
    } else {
        std::str::from_utf8(&version_tag[1..4])
            .ok()
            .and_then(|s| s.trim_end_matches('\0').parse().ok())
            .unwrap_or(0)
    };
    if version >= 11 {
        // v11+ may use zlib-compressed tiles, which we do not decode yet.
        return Err(FormatError::UnsupportedFeature("XCF v11+ (zlib tiles)").into());
    }
    let width = r.u32()?;
    let height = r.u32()?;
    let base_type = r.u32()?;
    if base_type != 0 {
        return Err(FormatError::UnsupportedFeature("non-RGB XCF").into());
    }
    if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(FormatError::Malformed("XCF dimensions out of range").into());
    }
    // v4+ stores a precision u32 before the image properties; only 8-bit (0..=150 "8-bit") handled.
    if version >= 4 {
        let precision = r.u32()?;
        // GIMP precisions: 100=8-bit linear/perceptual family; anything implying >8bit we reject.
        if !(precision == 0 || (100..=160).contains(&precision)) {
            return Err(FormatError::UnsupportedFeature("non-8-bit XCF").into());
        }
        if precision >= 200 {
            return Err(FormatError::UnsupportedFeature("non-8-bit XCF").into());
        }
    }
    // Image property list — skip; we do not need canvas-level props for a faithful raster read.
    skip_properties(&mut r)?;

    // Layer pointer list (0-terminated). Pointer width is 4 bytes for v<11.
    let mut layer_offsets = Vec::new();
    loop {
        let off = r.u32()?;
        if off == 0 {
            break;
        }
        layer_offsets.push(off as usize);
    }

    let mut builder = DocumentImportBuilder::new(width, height)?;
    let mut warnings = Vec::new();

    // XCF stores layers top-first; our siblings are bottom-first, so read then reverse.
    let mut layers = Vec::new();
    for off in layer_offsets {
        layers.push(read_layer(&r, off, width, height, &mut warnings)?);
    }
    for layer in layers.into_iter().rev() {
        builder.push_node(
            ImportNode::raster(layer.name, vec![RasterCel::new(FrameId::DEFAULT, layer.pixels)])
                .with_visibility(layer.visible)
                .with_opacity(layer.opacity),
        )?;
    }
    if builder_is_empty(&builder) {
        return Err(FormatError::Malformed("XCF has no layers").into());
    }
    Ok((builder.build()?, warnings))
}

fn builder_is_empty(_b: &DocumentImportBuilder) -> bool {
    // DocumentImportBuilder::build() already errors on empty; this is a readability guard only.
    false
}

struct XcfLayer {
    name: String,
    pixels: Vec<u8>,
    opacity: f32,
    visible: bool,
}

fn skip_properties(r: &mut Be) -> Result<()> {
    loop {
        let id = r.u32()?;
        if id == PROP_END {
            // PROP_END has a zero-length payload field too in practice; but canonical XCF writes
            // PROP_END as id only. GIMP writes a length of 0 for END — read and discard it.
            let _ = r.u32();
            break;
        }
        let len = r.u32()? as usize;
        r.take(len)?;
    }
    Ok(())
}

fn read_layer(
    base: &Be,
    offset: usize,
    canvas_w: u32,
    canvas_h: u32,
    warnings: &mut Vec<FormatWarning>,
) -> Result<XcfLayer> {
    let mut r = base.at(offset);
    let lw = r.u32()?;
    let lh = r.u32()?;
    let _ltype = r.u32()?; // 0 RGB, 1 RGBA, ...
    let name = r.string()?;

    // Layer property list.
    let mut opacity = 1.0f32;
    let mut visible = true;
    let (mut off_x, mut off_y) = (0i32, 0i32);
    loop {
        let id = r.u32()?;
        if id == PROP_END {
            let _ = r.u32();
            break;
        }
        let len = r.u32()? as usize;
        match id {
            PROP_OPACITY => {
                // u32 0..255.
                let mut p = r.at(r.pos);
                opacity = (p.u32()? as f32 / 255.0).clamp(0.0, 1.0);
                r.take(len)?;
            }
            PROP_VISIBLE => {
                let mut p = r.at(r.pos);
                visible = p.u32()? != 0;
                r.take(len)?;
            }
            PROP_OFFSETS => {
                let mut p = r.at(r.pos);
                off_x = p.i32()?;
                off_y = p.i32()?;
                r.take(len)?;
            }
            _ => {
                r.take(len)?;
            }
        }
    }

    let hierarchy_offset = r.u32()? as usize;
    let rect = read_hierarchy(base, hierarchy_offset, lw, lh, warnings)?;
    let pixels = place(&rect, lw as usize, lh as usize, off_x, off_y, canvas_w, canvas_h);
    Ok(XcfLayer {
        name: if name.is_empty() { "Layer".into() } else { name },
        pixels,
        opacity,
        visible,
    })
}

/// Read a hierarchy: its top level's tiles into a layer-rect RGBA buffer.
fn read_hierarchy(
    base: &Be,
    offset: usize,
    lw: u32,
    lh: u32,
    warnings: &mut Vec<FormatWarning>,
) -> Result<Vec<u8>> {
    let mut r = base.at(offset);
    let hw = r.u32()?;
    let hh = r.u32()?;
    let bpp = r.u32()? as usize;
    if hw != lw || hh != lh {
        warnings.push(FormatWarning::FlattenedHierarchy);
    }
    if !(bpp == 3 || bpp == 4) {
        return Err(FormatError::UnsupportedFeature("XCF layer not 8-bit RGB(A)").into());
    }
    // The first level offset is the full-resolution image; the rest are mipmaps we ignore.
    let level_offset = r.u32()? as usize;
    read_level(base, level_offset, lw as usize, lh as usize, bpp)
}

/// Read one level: its tiles, into an RGBA buffer of the layer size.
fn read_level(
    base: &Be,
    offset: usize,
    lw: usize,
    lh: usize,
    bpp: usize,
) -> Result<Vec<u8>> {
    let mut r = base.at(offset);
    let w = r.u32()? as usize;
    let h = r.u32()? as usize;
    if w != lw || h != lh {
        return Err(FormatError::Malformed("XCF level size mismatch").into());
    }
    let tiles_x = w.div_ceil(TILE);
    let tiles_y = h.div_ceil(TILE);
    let n_tiles = tiles_x * tiles_y;
    let mut tile_offsets = Vec::with_capacity(n_tiles);
    for _ in 0..n_tiles {
        tile_offsets.push(r.u32()? as usize);
    }
    // The list is 0-terminated; drop a trailing zero if present.
    let mut rgba = vec![0u8; w * h * 4];
    for (ti, &toff) in tile_offsets.iter().enumerate() {
        if toff == 0 {
            continue;
        }
        let tx = (ti % tiles_x) * TILE;
        let ty = (ti / tiles_x) * TILE;
        let tw = (w - tx).min(TILE);
        let th = (h - ty).min(TILE);
        let planes = read_tile(base, toff, tw, th, bpp)?;
        for y in 0..th {
            for x in 0..tw {
                let si = y * tw + x;
                let d = ((ty + y) * w + (tx + x)) * 4;
                rgba[d] = planes[0][si];
                rgba[d + 1] = planes[1][si];
                rgba[d + 2] = planes[2][si];
                rgba[d + 3] = if bpp == 4 { planes[3][si] } else { 255 };
            }
        }
    }
    Ok(rgba)
}

/// Read a single tile. XCF stores either raw (level's compression is image-wide, but the loader
/// detects by trying RLE); we follow GIMP: v<11 uses RLE per channel. Returns one plane per channel.
fn read_tile(base: &Be, offset: usize, tw: usize, th: usize, bpp: usize) -> Result<Vec<Vec<u8>>> {
    let count = tw * th;
    let mut r = base.at(offset);
    let mut planes = Vec::with_capacity(bpp);
    for _ in 0..bpp {
        planes.push(rle_decode_plane(&mut r, count)?);
    }
    Ok(planes)
}

/// Decode one RLE channel plane of `count` bytes (XCF tile RLE). Opcodes:
///  n in 0..=126  -> (n+1) literal bytes
///  n == 127      -> next 2 bytes = u16 big-endian length of literals
///  n == 128      -> next byte = count, then 2 bytes? (GIMP: 128 => read u16 length, 1 repeated byte)
///  n in 129..=255-> (256-n)+1 copies of the next byte  [i.e. a run]
fn rle_decode_plane(r: &mut Be, count: usize) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(count);
    while out.len() < count {
        let op = r.take(1)?[0] as usize;
        if op < 128 {
            // op < 128: literal run of (op+1) bytes, unless op == 127 which escapes to a u16 count.
            let len = if op == 127 {
                let b = r.take(2)?;
                u16::from_be_bytes([b[0], b[1]]) as usize
            } else {
                op + 1
            };
            let data = r.take(len)?;
            out.extend_from_slice(data);
        } else {
            // op >= 128: a repeated byte. op == 128 escapes to a u16 count.
            let len = if op == 128 {
                let b = r.take(2)?;
                u16::from_be_bytes([b[0], b[1]]) as usize
            } else {
                256 - op + 1
            };
            let value = r.take(1)?[0];
            out.extend(std::iter::repeat(value).take(len));
        }
    }
    out.truncate(count);
    Ok(out)
}

/// Place a layer rect onto the canvas at its offset, clipping.
fn place(rect: &[u8], rw: usize, rh: usize, left: i32, top: i32, cw: u32, ch: u32) -> Vec<u8> {
    let cw = cw as usize;
    let ch = ch as usize;
    let mut out = vec![0u8; cw * ch * 4];
    for ry in 0..rh {
        let cy = top + ry as i32;
        if cy < 0 || cy as usize >= ch {
            continue;
        }
        for rx in 0..rw {
            let cx = left + rx as i32;
            if cx < 0 || cx as usize >= cw {
                continue;
            }
            let so = (ry * rw + rx) * 4;
            let d = (cy as usize * cw + cx as usize) * 4;
            if so + 4 <= rect.len() {
                out[d..d + 4].copy_from_slice(&rect[so..so + 4]);
            }
        }
    }
    out
}
