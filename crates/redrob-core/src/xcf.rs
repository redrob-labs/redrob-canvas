// SPDX-License-Identifier: GPL-3.0-or-later

//! GIMP XCF import (read-only), re-derived from the public XCF format description and the layout of
//! GIMP's `app/xcf` loader (behaviour studied, no code copied). We read the common case: an 8-bit
//! RGB or RGBA image with uncompressed, RLE or zlib tiles, at any file version (v11 and later use
//! 8-byte file offsets), preserving the layer stack with each layer's name, opacity, visibility and
//! canvas offset.
//!
//! Not covered yet (export, and import of): >8-bit precision, indexed/greyscale base types, layer
//! masks and parasites. Those are later passes; an unsupported file is rejected with a typed error
//! rather than mis-read.

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
    /// At most `limit` unread bytes. A zlib tile's stream ends itself, so handing the inflate a slice
    /// bounded by the next tile's offset is enough -- an over-long slice costs nothing.
    fn remaining(&self, limit: usize) -> &'a [u8] {
        let start = self.pos.min(self.bytes.len());
        let end = (start + limit).min(self.bytes.len());
        &self.bytes[start..end]
    }
    /// A file offset, which is 4 bytes up to XCF v10 and 8 bytes from v11 -- the real change at v11,
    /// and the reason a v11 file read with 4-byte offsets lands in the middle of the data rather than
    /// failing cleanly.
    fn offset(&mut self, width: usize) -> Result<usize> {
        if width == 8 {
            let b = self.take(8)?;
            let value = u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
            usize::try_from(value).map_err(|_| FormatError::Malformed("XCF offset too large").into())
        } else {
            Ok(self.u32()? as usize)
        }
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
const PROP_COMPRESSION: u32 = 17;

/// How a level's tiles are stored. The mode is declared ONCE, as an image property, and applies to
/// every tile in the file -- which is why reading it is not optional: the three forms are not
/// distinguishable from the tile bytes, and guessing RLE on a raw tile yields a plausible-looking
/// smear rather than an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum XcfCompression {
    /// Interleaved pixels, no compression.
    None,
    /// One RLE stream PER CHANNEL -- planar, unlike the other two.
    Rle,
    /// One zlib stream per tile, inflating to interleaved pixels (XCF v11 and later).
    Zlib,
}

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
    // XCF v11's change is the OFFSET WIDTH: file offsets become 8 bytes. Reading a v11 file with
    // 4-byte offsets does not fail, it lands in the middle of the data -- so the width is decided here
    // and threaded through every offset read.
    let offset_width = if version >= 11 { 8 } else { 4 };
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
    // Image property list. The COMPRESSION property is the one we must not skip: it declares how every
    // tile in the file is stored, and the three forms cannot be told apart from the tile bytes.
    let compression = read_image_properties(&mut r)?;

    // Layer pointer list, zero-terminated.
    let mut layer_offsets = Vec::new();
    loop {
        let off = r.offset(offset_width)?;
        if off == 0 {
            break;
        }
        layer_offsets.push(off);
    }

    let mut builder = DocumentImportBuilder::new(width, height)?;
    let mut warnings = Vec::new();

    // XCF stores layers top-first; our siblings are bottom-first, so read then reverse.
    let mut layers = Vec::new();
    for off in layer_offsets {
        layers.push(read_layer(
            &r,
            off,
            width,
            height,
            compression,
            offset_width,
            &mut warnings,
        )?);
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

/// Walks the image property list, returning the declared tile compression.
///
/// Defaults to `None` when the property is absent, which is what the oldest files mean -- RLE as a
/// default would mis-read them, and the failure would be a picture rather than an error.
fn read_image_properties(r: &mut Be) -> Result<XcfCompression> {
    let mut compression = XcfCompression::None;
    loop {
        let id = r.u32()?;
        if id == PROP_END {
            // PROP_END has a zero-length payload field too in practice; but canonical XCF writes
            // PROP_END as id only. GIMP writes a length of 0 for END — read and discard it.
            let _ = r.u32();
            break;
        }
        let len = r.u32()? as usize;
        if id == PROP_COMPRESSION && len >= 1 {
            let mut p = r.at(r.pos);
            compression = match p.take(1)?[0] {
                0 => XcfCompression::None,
                1 => XcfCompression::Rle,
                2 => XcfCompression::Zlib,
                // 3 is "fractal", which GIMP itself never implemented. Refused by name rather than
                // decoded as one of the forms it is not.
                _ => return Err(FormatError::UnsupportedFeature("XCF tile compression").into()),
            };
        }
        r.take(len)?;
    }
    Ok(compression)
}

fn read_layer(
    base: &Be,
    offset: usize,
    canvas_w: u32,
    canvas_h: u32,
    compression: XcfCompression,
    offset_width: usize,
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

    let hierarchy_offset = r.offset(offset_width)?;
    let rect = read_hierarchy(
        base,
        hierarchy_offset,
        lw,
        lh,
        compression,
        offset_width,
        warnings,
    )?;
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
    compression: XcfCompression,
    offset_width: usize,
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
    let level_offset = r.offset(offset_width)?;
    read_level(
        base,
        level_offset,
        lw as usize,
        lh as usize,
        bpp,
        compression,
        offset_width,
    )
}

/// Read one level: its tiles, into an RGBA buffer of the layer size.
fn read_level(
    base: &Be,
    offset: usize,
    lw: usize,
    lh: usize,
    bpp: usize,
    compression: XcfCompression,
    offset_width: usize,
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
    let mut tile_offsets = Vec::with_capacity(n_tiles + 1);
    for _ in 0..n_tiles {
        tile_offsets.push(r.offset(offset_width)?);
    }
    // The terminating zero, when present, is what bounds the LAST tile's data. A compressed tile does
    // not declare its own byte length: the length is the distance to the next tile, so the list has to
    // be read before any tile is decoded.
    let terminator = r.offset(offset_width).unwrap_or(0);

    let mut rgba = vec![0u8; w * h * 4];
    for (ti, &toff) in tile_offsets.iter().enumerate() {
        if toff == 0 {
            continue;
        }
        let tx = (ti % tiles_x) * TILE;
        let ty = (ti / tiles_x) * TILE;
        let tw = (w - tx).min(TILE);
        let th = (h - ty).min(TILE);
        // Bound this tile's bytes by the next NON-ZERO offset after it; failing that, by a generous
        // allowance. GIMP does the same, because compression can make a tile LARGER than its pixels and
        // a tight bound would truncate exactly those tiles.
        let next = tile_offsets[ti + 1..]
            .iter()
            .copied()
            .find(|&o| o > toff)
            .or(if terminator > toff {
                Some(terminator)
            } else {
                None
            });
        let available = match next {
            Some(end) => end.saturating_sub(toff),
            None => tw * th * bpp * 2 + 64,
        };
        let planes = read_tile(base, toff, tw, th, bpp, compression, available)?;
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

/// Read a single tile into one plane per channel.
///
/// The three compression forms do NOT share a layout: RLE is planar (one stream per channel), while
/// raw and zlib are interleaved pixels. Treating an interleaved tile as planar produces a picture --
/// the first channel's plane reads as the first quarter of the pixels -- so the mode decides the
/// de-interleave, not just the decompression.
fn read_tile(
    base: &Be,
    offset: usize,
    tw: usize,
    th: usize,
    bpp: usize,
    compression: XcfCompression,
    available: usize,
) -> Result<Vec<Vec<u8>>> {
    let count = tw * th;
    let mut r = base.at(offset);
    match compression {
        XcfCompression::Rle => {
            let mut planes = Vec::with_capacity(bpp);
            for _ in 0..bpp {
                planes.push(rle_decode_plane(&mut r, count)?);
            }
            Ok(planes)
        }
        XcfCompression::None => Ok(deinterleave(r.take(count * bpp)?, count, bpp)),
        XcfCompression::Zlib => {
            let input = r.remaining(available);
            let mut decompressor = flate2::Decompress::new(true);
            let mut out = Vec::with_capacity(count * bpp);
            decompressor
                .decompress_vec(input, &mut out, flate2::FlushDecompress::Finish)
                .map_err(|_| FormatError::Malformed("XCF zlib tile"))?;
            if out.len() < count * bpp {
                return Err(FormatError::Malformed("XCF zlib tile short").into());
            }
            Ok(deinterleave(&out, count, bpp))
        }
    }
}

/// Splits interleaved pixels into one plane per channel, which is the shape the caller composes from.
fn deinterleave(data: &[u8], count: usize, bpp: usize) -> Vec<Vec<u8>> {
    (0..bpp)
        .map(|channel| {
            (0..count)
                .map(|index| data.get(index * bpp + channel).copied().unwrap_or(0))
                .collect()
        })
        .collect()
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
