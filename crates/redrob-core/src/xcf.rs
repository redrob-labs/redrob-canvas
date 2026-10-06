// SPDX-License-Identifier: GPL-3.0-or-later

//! GIMP XCF import and export, re-derived from the public XCF format description and the layout of
//! GIMP's `app/xcf` loader and saver (behaviour studied, no code copied). Import reads 8-bit RGB,
//! greyscale and indexed images with uncompressed, RLE or zlib tiles, at any file version (v11 and
//! later use 8-byte file offsets), preserving the layer stack with each layer's name, opacity,
//! visibility, canvas offset and layer mask. Export writes v11 RGBA with zlib tiles.
//!
//! Not covered yet: >8-bit precision, writing indexed or greyscale base types, the image's own
//! saved-selection and spot channels (counted and reported on import, since this product has no home
//! for them), and parasites. An unsupported file is rejected with a typed error rather than mis-read.

use std::io::Write;

use crate::document::MAX_DIMENSION;
use crate::{
    Document, DocumentImportBuilder, ExportOptions, FormatError, FormatWarning, FrameId,
    ImportNode, ImportOptions, NodeKind, RasterCel, Result,
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
        Be {
            bytes: self.bytes,
            pos,
        }
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
            usize::try_from(value)
                .map_err(|_| FormatError::Malformed("XCF offset too large").into())
        } else {
            Ok(self.u32()? as usize)
        }
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
const PROP_COLORMAP: u32 = 1;
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

/// The image's base type, which decides what a layer's channels MEAN.
///
/// A layer's hierarchy declares only how MANY bytes a pixel has: 1 could be grey or a palette index,
/// 2 could be grey+alpha or index+alpha. Only the base type tells them apart, so an indexed image read
/// as greyscale comes out as a picture of its palette indices -- dark, banded, and not obviously wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum XcfBase {
    Rgb,
    Grayscale,
    Indexed,
}

/// Base type plus the palette an indexed image needs.
struct XcfColor {
    base: XcfBase,
    /// Interleaved RGB triples, as the colormap property stores them.
    palette: Vec<u8>,
}

impl XcfColor {
    /// Channels that carry colour, before any alpha channel.
    const fn color_channels(&self) -> usize {
        match self.base {
            XcfBase::Rgb => 3,
            XcfBase::Grayscale | XcfBase::Indexed => 1,
        }
    }

    fn to_rgb(&self, samples: &[u8]) -> (u8, u8, u8) {
        match self.base {
            XcfBase::Rgb => (samples[0], samples[1], samples[2]),
            XcfBase::Grayscale => (samples[0], samples[0], samples[0]),
            XcfBase::Indexed => {
                let at = samples[0] as usize * 3;
                match self.palette.get(at..at + 3) {
                    Some(rgb) => (rgb[0], rgb[1], rgb[2]),
                    // An index with no palette entry is the file's problem, not a colour to invent:
                    // it reads as black rather than as whatever happens to follow in memory.
                    None => (0, 0, 0),
                }
            }
        }
    }
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
    let base = match base_type {
        0 => XcfBase::Rgb,
        1 => XcfBase::Grayscale,
        2 => XcfBase::Indexed,
        _ => return Err(FormatError::Malformed("XCF base type").into()),
    };
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
    // Image property list. Two properties must not be skipped: COMPRESSION, which declares how every
    // tile in the file is stored, and COLORMAP, without which an indexed image is only indices.
    let (compression, palette) = read_image_properties(&mut r, version)?;
    if base == XcfBase::Indexed && palette.is_empty() {
        return Err(FormatError::Malformed("indexed XCF without a colormap").into());
    }
    let color = XcfColor { base, palette };

    // Layer pointer list, zero-terminated.
    let mut layer_offsets = Vec::new();
    loop {
        let off = r.offset(offset_width)?;
        if off == 0 {
            break;
        }
        layer_offsets.push(off);
    }
    // The image's own CHANNEL pointer list follows, also zero-terminated: saved selections and spot
    // channels. This product has no home for them, so they are counted and reported rather than
    // silently dropped -- and the list is read either way, since a reader that stops at the layers has
    // no idea what follows.
    let mut channel_count = 0usize;
    loop {
        match r.offset(offset_width) {
            Ok(0) | Err(_) => break,
            Ok(_) => channel_count += 1,
        }
    }

    let mut builder = DocumentImportBuilder::new(width, height)?;
    let mut warnings = Vec::new();
    if channel_count > 0 {
        warnings.push(FormatWarning::OmittedMetadata);
    }

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
            &color,
            &mut warnings,
        )?);
    }
    for layer in layers.into_iter().rev() {
        builder.push_node(
            ImportNode::raster(
                layer.name,
                vec![RasterCel::new(FrameId::DEFAULT, layer.pixels)],
            )
            .with_visibility(layer.visible)
            .with_opacity(layer.opacity)
            .with_mask(layer.mask),
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
    mask: Option<crate::ImportMask>,
}

/// Walks the image property list, returning the declared tile compression and the colormap.
///
/// Compression defaults to `None` when the property is absent, which is what the oldest files mean --
/// RLE as a default would mis-read them, and the failure would be a picture rather than an error.
///
/// Version 0 wrote indexed colormaps incorrectly, which GIMP itself warns about and substitutes a
/// greyscale ramp for. We do the same substitution rather than rendering a wrong palette.
fn read_image_properties(r: &mut Be, version: u32) -> Result<(XcfCompression, Vec<u8>)> {
    let mut compression = XcfCompression::None;
    let mut palette = Vec::new();
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
        if id == PROP_COLORMAP && len >= 4 {
            let mut p = r.at(r.pos);
            let colors = p.u32()? as usize;
            if colors > 256 {
                return Err(FormatError::Malformed("XCF colormap too large").into());
            }
            if version == 0 {
                // The v0 payload is one byte per entry and not a usable palette; a grey ramp is what
                // GIMP substitutes, and inventing colours instead would look deliberate.
                palette = (0..colors)
                    .flat_map(|index| {
                        let grey = (index * 255 / colors.max(1).saturating_sub(1).max(1)) as u8;
                        [grey, grey, grey]
                    })
                    .collect();
            } else {
                palette = p.take(colors * 3)?.to_vec();
            }
        }
        r.take(len)?;
    }
    Ok((compression, palette))
}

// An XCF layer is decoded against the file's header state (version, offset width, compression,
// base type, palette) plus the warning sink -- the file's parameters, as in the PSD reader.
#[allow(clippy::too_many_arguments)]
fn read_layer(
    base: &Be,
    offset: usize,
    canvas_w: u32,
    canvas_h: u32,
    compression: XcfCompression,
    offset_width: usize,
    color: &XcfColor,
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
    // The layer MASK pointer sits immediately after the hierarchy pointer; zero means no mask. Reading
    // it is also what keeps this record's parse aligned.
    let mask_offset = r.offset(offset_width)?;
    let planes = read_hierarchy(
        base,
        hierarchy_offset,
        lw,
        lh,
        compression,
        offset_width,
        warnings,
    )?;
    // A layer's channel count says how many bytes a pixel has; the image's base type says what they
    // mean. Alpha is the channel after the colour ones, when there is one.
    let color_channels = color.color_channels();
    if planes.len() < color_channels {
        return Err(FormatError::Malformed("XCF layer channel count").into());
    }
    let count = lw as usize * lh as usize;
    let mut rect = vec![0u8; count * 4];
    let mut samples = vec![0u8; color_channels];
    for index in 0..count {
        for (channel, slot) in samples.iter_mut().enumerate() {
            *slot = planes[channel].get(index).copied().unwrap_or(0);
        }
        let (red, green, blue) = color.to_rgb(&samples);
        rect[index * 4] = red;
        rect[index * 4 + 1] = green;
        rect[index * 4 + 2] = blue;
        rect[index * 4 + 3] = planes
            .get(color_channels)
            .and_then(|plane| plane.get(index).copied())
            .unwrap_or(255);
    }
    let pixels = place(
        &rect,
        lw as usize,
        lh as usize,
        off_x,
        off_y,
        canvas_w,
        canvas_h,
    );

    // A layer mask is a CHANNEL structure, not a layer: width, height, name, properties, hierarchy.
    let mask = if mask_offset == 0 {
        None
    } else {
        let plane = read_channel(base, mask_offset, compression, offset_width, warnings)?;
        // Placed at the LAYER's offset, with 255 outside: the mask only governs where the layer is, and
        // filling the rest with 0 would be indistinguishable from a mask that hides everything else.
        Some(crate::ImportMask::new(place_plane(
            &plane.pixels,
            plane.width,
            plane.height,
            off_x,
            off_y,
            canvas_w,
            canvas_h,
        )))
    };
    Ok(XcfLayer {
        name: if name.is_empty() {
            "Layer".into()
        } else {
            name
        },
        pixels,
        opacity,
        visible,
        mask,
    })
}

/// One channel structure (a layer mask, or one of the image's own channels): its own geometry, name,
/// properties and a single-channel hierarchy.
struct XcfChannel {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

fn read_channel(
    base: &Be,
    offset: usize,
    compression: XcfCompression,
    offset_width: usize,
    warnings: &mut Vec<FormatWarning>,
) -> Result<XcfChannel> {
    let mut r = base.at(offset);
    let cw = r.u32()?;
    let chh = r.u32()?;
    let _name = r.string()?;
    loop {
        let id = r.u32()?;
        if id == PROP_END {
            let _ = r.u32();
            break;
        }
        let len = r.u32()? as usize;
        r.take(len)?;
    }
    let hierarchy_offset = r.offset(offset_width)?;
    let planes = read_hierarchy(
        base,
        hierarchy_offset,
        cw,
        chh,
        compression,
        offset_width,
        warnings,
    )?;
    Ok(XcfChannel {
        width: cw as usize,
        height: chh as usize,
        pixels: planes.into_iter().next().unwrap_or_default(),
    })
}

/// Places a single-channel rect onto a canvas-sized plane, filling the rest with 255.
fn place_plane(
    rect: &[u8],
    rw: usize,
    rh: usize,
    left: i32,
    top: i32,
    cw: u32,
    ch: u32,
) -> Vec<u8> {
    let canvas_w = cw as usize;
    let canvas_h = ch as usize;
    let mut out = vec![255u8; canvas_w * canvas_h];
    for y in 0..rh {
        let cy = top + y as i32;
        if cy < 0 || cy as usize >= canvas_h {
            continue;
        }
        for x in 0..rw {
            let cx = left + x as i32;
            if cx < 0 || cx as usize >= canvas_w {
                continue;
            }
            out[cy as usize * canvas_w + cx as usize] =
                rect.get(y * rw + x).copied().unwrap_or(255);
        }
    }
    out
}

/// Read a hierarchy's top level as one plane per channel, at the declared geometry.
fn read_hierarchy(
    base: &Be,
    offset: usize,
    lw: u32,
    lh: u32,
    compression: XcfCompression,
    offset_width: usize,
    warnings: &mut Vec<FormatWarning>,
) -> Result<Vec<Vec<u8>>> {
    let mut r = base.at(offset);
    let hw = r.u32()?;
    let hh = r.u32()?;
    let bpp = r.u32()? as usize;
    if hw != lw || hh != lh {
        warnings.push(FormatWarning::FlattenedHierarchy);
    }
    // 1..=4 covers every 8-bit shape: a mask or grey or index (1), those plus alpha (2), RGB (3) and
    // RGBA (4). Which of the ambiguous ones it is comes from the image's base type, not from here.
    if !(1..=4).contains(&bpp) {
        return Err(FormatError::UnsupportedFeature("XCF layer not 8-bit").into());
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
) -> Result<Vec<Vec<u8>>> {
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
    // The list is zero-terminated, and a level may hold FEWER tiles than its size implies: GIMP
    // writes an untouched layer as an empty list (a lone 0) and its loader reads that as "this
    // level is empty", leaving the pixels zero. Reading n_tiles offsets regardless walked past the
    // terminator into the next structure -- GIMP's own 2.6 test file failed as "XCF truncated".
    for _ in 0..n_tiles {
        match r.offset(offset_width)? {
            0 => break,
            offset => tile_offsets.push(offset),
        }
    }
    // The terminating zero, when present, is what bounds the LAST tile's data. A compressed tile does
    // not declare its own byte length: the length is the distance to the next tile, so the list has to
    // be read before any tile is decoded.
    let terminator = if tile_offsets.len() == n_tiles {
        r.offset(offset_width).unwrap_or(0)
    } else {
        0
    };

    // One plane per channel, at the level's own geometry. The caller decides what the channels MEAN;
    // this function only undoes tiling and compression. A tile the file omits stays zero, which is how
    // GIMP treats an absent tile.
    let mut out = vec![vec![0u8; w * h]; bpp];
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
        for channel in 0..bpp {
            for y in 0..th {
                for x in 0..tw {
                    out[channel][(ty + y) * w + (tx + x)] = planes[channel][y * tw + x];
                }
            }
        }
    }
    Ok(out)
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
        // GIMP's xcf_load_tile_rle, by behaviour: a byte BELOW 128 starts a REPEAT of (op + 1) copies
        // of the next byte, 127 escaping to a u16 count; a byte of 128 or more starts a LITERAL run of
        // (256 - op) bytes, 128 escaping to a u16 count. This had the two cases swapped, and no test
        // decoded an RLE tile, so nothing noticed: GIMP's own 2.6 test file read a 1600-byte repeat
        // as a 1600-byte literal and ran off the end.
        if op < 128 {
            let len = if op == 127 {
                let b = r.take(2)?;
                u16::from_be_bytes([b[0], b[1]]) as usize
            } else {
                op + 1
            };
            let value = r.take(1)?[0];
            out.extend(std::iter::repeat_n(value, len));
        } else {
            let len = if op == 128 {
                let b = r.take(2)?;
                u16::from_be_bytes([b[0], b[1]]) as usize
            } else {
                256 - op
            };
            let data = r.take(len)?;
            out.extend_from_slice(data);
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

// ---- Export ----------------------------------------------------------------

/// Writes an XCF v11: 8-byte file offsets, RGB base type, zlib tiles, one layer per node.
///
/// v11 rather than the oldest version it could be, because v11 is where offsets became 8 bytes and the
/// alternative is a format that cannot address a large document at all. GIMP reads every version, so
/// there is nothing to gain by writing an older one.
///
/// Every pointer in XCF is an absolute file offset, so this builds the file with placeholders and
/// patches them once the targets are known -- a single forward pass cannot know where a layer's
/// hierarchy will land.
pub(crate) fn export_xcf(
    document: &Document,
    frame: FrameId,
    _options: &ExportOptions,
) -> Result<(Vec<u8>, Vec<FormatWarning>)> {
    let width = document.width();
    let height = document.height();
    let mut warnings = Vec::new();

    // Group nodes have no XCF equivalent here: GIMP's layer groups are a node type this writer does not
    // emit, so a group's children are written as plain layers and the grouping is reported lost.
    let mut layers: Vec<(&crate::Layer, Vec<u8>)> = Vec::new();
    for node in document.nodes() {
        if matches!(node.kind(), NodeKind::Group) {
            warnings.push(FormatWarning::FlattenedHierarchy);
            continue;
        }
        layers.push((node, source_pixels(document, node, frame)?));
    }

    let mut out: Vec<u8> = Vec::new();
    out.extend_from_slice(b"gimp xcf ");
    out.extend_from_slice(b"v011\0");
    write_u32(&mut out, width);
    write_u32(&mut out, height);
    write_u32(&mut out, 0); // base type: RGB
    write_u32(&mut out, 150); // precision: 8-bit non-linear, which is what our rasters are
    // Image properties: the compression mode, then END. The mode is not optional -- a reader that finds
    // no COMPRESSION property is entitled to assume uncompressed tiles.
    write_u32(&mut out, PROP_COMPRESSION);
    write_u32(&mut out, 1);
    out.push(2); // zlib
    write_u32(&mut out, PROP_END);
    write_u32(&mut out, 0);

    // XCF stores layers TOP-first, the reverse of our sibling order.
    let order: Vec<usize> = (0..layers.len()).rev().collect();
    let mut layer_pointer_slots = Vec::with_capacity(order.len());
    for _ in &order {
        layer_pointer_slots.push(out.len());
        write_u64(&mut out, 0);
    }
    write_u64(&mut out, 0); // layer list terminator
    write_u64(&mut out, 0); // channel list: none

    for (slot, &index) in layer_pointer_slots.iter().zip(order.iter()) {
        let (node, pixels) = &layers[index];
        let layer_offset = out.len();
        patch_u64(&mut out, *slot, layer_offset);

        write_u32(&mut out, width);
        write_u32(&mut out, height);
        write_u32(&mut out, 1); // layer type: RGBA
        write_string(&mut out, node.name());
        // Opacity, visibility and the layer's canvas offset. Written even at their defaults: a reader
        // that sees no OPACITY property has to guess, and our own reader's guess is not the file's.
        write_u32(&mut out, PROP_OPACITY);
        write_u32(&mut out, 4);
        write_u32(
            &mut out,
            (node.opacity() * 255.0).round().clamp(0.0, 255.0) as u32,
        );
        write_u32(&mut out, PROP_VISIBLE);
        write_u32(&mut out, 4);
        write_u32(&mut out, u32::from(node.is_visible()));
        write_u32(&mut out, PROP_OFFSETS);
        write_u32(&mut out, 8);
        write_u32(&mut out, 0);
        write_u32(&mut out, 0);
        write_u32(&mut out, PROP_END);
        write_u32(&mut out, 0);

        let hierarchy_slot = out.len();
        write_u64(&mut out, 0);
        // The mask pointer is written as zero rather than omitted: it is a fixed field of the record,
        // and a reader that expects it would otherwise take the hierarchy's first bytes for a pointer.
        // Our own masks are baked into the layer's alpha by `source_pixels`.
        write_u64(&mut out, 0);

        let hierarchy_offset = out.len();
        patch_u64(&mut out, hierarchy_slot, hierarchy_offset);
        write_u32(&mut out, width);
        write_u32(&mut out, height);
        write_u32(&mut out, 4); // bytes per pixel: RGBA
        let level_slot = out.len();
        write_u64(&mut out, 0);
        // No mipmap levels. GIMP writes a terminating zero here.
        write_u64(&mut out, 0);

        let level_offset = out.len();
        patch_u64(&mut out, level_slot, level_offset);
        write_u32(&mut out, width);
        write_u32(&mut out, height);
        let tiles_x = (width as usize).div_ceil(TILE);
        let tiles_y = (height as usize).div_ceil(TILE);
        let mut tile_slots = Vec::with_capacity(tiles_x * tiles_y);
        for _ in 0..(tiles_x * tiles_y) {
            tile_slots.push(out.len());
            write_u64(&mut out, 0);
        }
        write_u64(&mut out, 0); // tile list terminator

        for (ti, slot) in tile_slots.iter().enumerate() {
            let tx = (ti % tiles_x) * TILE;
            let ty = (ti / tiles_x) * TILE;
            let tw = (width as usize - tx).min(TILE);
            let th = (height as usize - ty).min(TILE);
            // A tile carries only its OWN rectangle, which makes the edge tiles narrower than 64 -- the
            // opposite of Krita's format, where a tile is always tile-sized. Writing a full tile here
            // would shift every row of the edge tiles.
            let mut tile = Vec::with_capacity(tw * th * 4);
            for y in 0..th {
                let row = ((ty + y) * width as usize + tx) * 4;
                tile.extend_from_slice(&pixels[row..row + tw * 4]);
            }
            let mut encoder =
                flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            encoder
                .write_all(&tile)
                .map_err(|_| FormatError::Malformed("XCF tile deflate"))?;
            let compressed = encoder
                .finish()
                .map_err(|_| FormatError::Malformed("XCF tile deflate"))?;
            let tile_offset = out.len();
            patch_u64(&mut out, *slot, tile_offset);
            out.extend_from_slice(&compressed);
        }
    }

    if out.len() > crate::MAX_FORMAT_OUTPUT_BYTES {
        return Err(FormatError::OutputTooLarge.into());
    }
    Ok((out, warnings))
}

fn write_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn write_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn patch_u64(out: &mut [u8], at: usize, value: usize) {
    out[at..at + 8].copy_from_slice(&(value as u64).to_be_bytes());
}

/// An XCF string: a byte count that INCLUDES the trailing NUL, then the bytes. An empty string is
/// written as a count of zero with no NUL, which is the one case where the rule does not apply.
fn write_string(out: &mut Vec<u8>, text: &str) {
    if text.is_empty() {
        write_u32(out, 0);
        return;
    }
    let bytes = text.as_bytes();
    write_u32(out, bytes.len() as u32 + 1);
    out.extend_from_slice(bytes);
    out.push(0);
}

/// The layer's pixels as RGBA, with a disabled-aware mask already multiplied into the alpha. XCF has
/// its own mask channel, but ours is baked here instead: a mask written as an XCF channel would have to
/// carry the enabled flag too, and a disabled mask baked in would silently become permanent.
fn source_pixels(document: &Document, node: &crate::Layer, frame: FrameId) -> Result<Vec<u8>> {
    let mut pixels = match node.kind() {
        NodeKind::Raster => node.raster_pixels(frame).map_or_else(
            |_| vec![0; document.width() as usize * document.height() as usize * 4],
            <[u8]>::to_vec,
        ),
        NodeKind::Text | NodeKind::Vector => {
            crate::semantic::rasterize(node.content(), document.width(), document.height())?
        }
        NodeKind::Group => unreachable!("groups are skipped before this point"),
    };
    if let Some(mask) = node.mask().filter(|mask| mask.is_enabled()) {
        for (pixel, coverage) in pixels.chunks_exact_mut(4).zip(mask.pixels()) {
            pixel[3] = ((u16::from(pixel[3]) * u16::from(*coverage) + 127) / 255) as u8;
        }
    }
    Ok(pixels)
}

#[cfg(test)]
mod tests {
    use super::{Be, rle_decode_plane};

    #[test]
    fn rle_follows_gimps_byte_meanings() {
        // Below 128 repeats the next byte (op + 1) times; 128 and above copies (256 - op) literal
        // bytes; 127 and 128 escape to a u16 count. Each case, from xcf_load_tile_rle's behaviour.
        let cases: [(&[u8], &[u8]); 4] = [
            (&[2, 7], &[7, 7, 7]),
            (&[0xFE, 1, 2], &[1, 2]),
            (&[127, 0, 5, 9], &[9, 9, 9, 9, 9]),
            (&[128, 0, 3, 4, 5, 6], &[4, 5, 6]),
        ];
        for (input, expected) in cases {
            let mut reader = Be::new(input);
            assert_eq!(
                rle_decode_plane(&mut reader, expected.len()).unwrap(),
                expected,
                "{input:?}"
            );
        }
    }
}
