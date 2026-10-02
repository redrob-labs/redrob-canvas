// SPDX-License-Identifier: GPL-3.0-or-later

//! Adobe Photoshop (PSD) import/export, re-derived from the PSD file-format spec and the layout of
//! GIMP's `plug-ins/common/file-psd` and Krita's `plugins/impex/psd` (behaviour studied, no code
//! copied). Import reads RGB(A) at 8, 16 and 32 bits per channel, with raw, PackBits (RLE) and ZIP
//! (with or without prediction) channel data, and keeps the layer records so a round-trip keeps the
//! layer stack. Export writes 8-bit, which is the depth this product's rasters hold.
//!
//! Deeper samples are narrowed on the way in and that narrowing is REPORTED
//! (`FormatWarning::NarrowedDepth`) rather than done silently -- a 16-bit gradient can band at 8-bit
//! and a 32-bit document's out-of-range values are clamped.
//!
//! Colour modes are converted to RGB on the way in (bitmap, greyscale, indexed, RGB, CMYK,
//! multichannel, duotone, Lab) and the conversion is reported
//! (`FormatWarning::ConvertedColorMode`), because a device space without its profile -- CMYK above all
//! -- converts approximately.
//!
//! Not yet covered: layer masks beyond alpha, adjustment layers, and
//! image resources. Those are later passes.

use crate::document::MAX_DIMENSION;
use crate::{
    Document, DocumentImportBuilder, ExportOptions, FileFormat, FormatError, FormatWarning, FrameId,
    ImportNode, ImportOptions, NodeKind, RasterCel, RenderSnapshot, Result,
};

const SIGNATURE: &[u8; 4] = b"8BPS";

/// A cursor-free big-endian reader over a byte slice.
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.pos + n > self.bytes.len() {
            return Err(FormatError::Malformed("PSD truncated").into());
        }
        let slice = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }
    fn i16(&mut self) -> Result<i16> {
        Ok(self.u16()? as i16)
    }
    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }
    fn skip(&mut self, n: usize) -> Result<()> {
        self.take(n)?;
        Ok(())
    }
    /// The unread tail. A ZIP-compressed plane does not declare its own byte length here, so the
    /// inflate is given the rest of the input and reports what it actually consumed.
    fn remaining(&self) -> &'a [u8] {
        &self.bytes[self.pos.min(self.bytes.len())..]
    }
}

/// PSD colour modes, by their header tag.
///
/// Every mode here is read by converting to RGB on the way in, because this product's rasters are
/// RGBA. The conversion is the whole content of this type: a mode is not "supported" by being
/// recognised, it is supported by knowing what its channel values MEAN.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ColorMode {
    /// 1 bit per pixel, and INVERTED against every other mode: a set bit is black.
    Bitmap,
    Grayscale,
    /// One channel of palette indices; the palette lives in the colour-mode data block.
    Indexed,
    Rgb,
    /// Four channels, stored INVERTED: 255 means no ink.
    Cmyk,
    /// Spot channels with no defined colour here. Read as grey from the first channel, which is what
    /// its stored data actually is.
    Multichannel,
    /// Greyscale data plus ink curves we do not apply; the stored plane IS the grey.
    Duotone,
    /// CIE Lab with L in 0..=255 for 0..=100 and a/b offset by 128.
    Lab,
}

impl ColorMode {
    const fn from_tag(tag: u16) -> Option<Self> {
        match tag {
            0 => Some(Self::Bitmap),
            1 => Some(Self::Grayscale),
            2 => Some(Self::Indexed),
            3 => Some(Self::Rgb),
            4 => Some(Self::Cmyk),
            7 => Some(Self::Multichannel),
            8 => Some(Self::Duotone),
            9 => Some(Self::Lab),
            _ => None,
        }
    }

    /// How many colour planes the mode reads before any alpha channel.
    const fn color_planes(self) -> usize {
        match self {
            Self::Bitmap | Self::Grayscale | Self::Indexed | Self::Multichannel | Self::Duotone => 1,
            Self::Rgb | Self::Lab => 3,
            Self::Cmyk => 4,
        }
    }

    /// The name used in the conversion warning, so a caller can tell which mode was approximated.
    const fn label(self) -> &'static str {
        match self {
            Self::Bitmap => "bitmap",
            Self::Grayscale => "grayscale",
            Self::Indexed => "indexed",
            Self::Rgb => "rgb",
            Self::Cmyk => "cmyk",
            Self::Multichannel => "multichannel",
            Self::Duotone => "duotone",
            Self::Lab => "lab",
        }
    }

    /// Converts one pixel's colour planes to RGB.
    ///
    /// CMYK is deliberately the multiplicative conversion rather than `255 - (c + k)`: the additive
    /// form clips to black wherever ink totals pass 100%, which is most of a real print document's
    /// shadows. Without an embedded profile this is still an approximation of a device space, which is
    /// why the import reports it rather than claiming the colours are right.
    fn to_rgb(self, planes: &[u8], palette: Option<&[u8]>) -> (u8, u8, u8) {
        let at = |index: usize| planes.get(index).copied().unwrap_or(0);
        match self {
            Self::Bitmap | Self::Grayscale | Self::Multichannel | Self::Duotone => {
                let grey = at(0);
                (grey, grey, grey)
            }
            Self::Indexed => {
                let index = at(0) as usize;
                match palette {
                    // The palette is PLANAR: 256 reds, then 256 greens, then 256 blues.
                    Some(table) if table.len() >= 768 => (
                        table[index],
                        table[256 + index],
                        table[512 + index],
                    ),
                    // No palette block: the index is all the information there is, so it is read as
                    // grey rather than invented as a colour.
                    _ => {
                        let grey = at(0);
                        (grey, grey, grey)
                    }
                }
            }
            Self::Rgb => (at(0), at(1), at(2)),
            Self::Cmyk => {
                let ink = |index: usize| f64::from(255 - at(index)) / 255.0;
                let (c, m, y, k) = (ink(0), ink(1), ink(2), ink(3));
                let channel = |v: f64| (255.0 * (1.0 - v) * (1.0 - k)).round().clamp(0.0, 255.0) as u8;
                (channel(c), channel(m), channel(y))
            }
            Self::Lab => {
                let lightness = f64::from(at(0)) * 100.0 / 255.0;
                let a = f64::from(at(1)) - 128.0;
                let b = f64::from(at(2)) - 128.0;
                crate::color::lab_to_srgb8(lightness, a, b)
            }
        }
    }
}

/// Bytes one sample occupies at this depth. 8, 16 and 32 are the depths a PSD stores channel data at
/// (1-bit bitmap mode is a colour mode, handled where colour modes are).
const fn sample_bytes(depth: u16) -> usize {
    match depth {
        16 => 2,
        32 => 4,
        _ => 1,
    }
}

/// Bytes one ROW of `cols` samples occupies. 1-bit rows are bit-packed and padded to a whole byte, so
/// the row stride is not `cols * sample_bytes` there -- getting this wrong shears a bitmap-mode image
/// diagonally rather than failing.
const fn row_bytes(cols: usize, depth: u16) -> usize {
    if depth == 1 {
        cols.div_ceil(8)
    } else {
        cols * sample_bytes(depth)
    }
}

/// Narrows one plane of `depth`-bit samples to the 8-bit samples this product's rasters hold.
///
/// 1-bit: bitmap mode, where a SET bit is black. The inversion is the mode's own convention, not a
/// mistake to correct later.
///
/// 16-bit: Photoshop stores 0..=32768 for 0..=1 rather than the full u16 range, so the scale is
/// against 32768 and values above it (which Photoshop does write) clamp instead of wrapping.
///
/// 32-bit: IEEE floats in LINEAR light, which is the point of a 32-bit document. Encoding them with
/// the sRGB transfer function is what keeps a 32-bit file from opening darker than the same picture
/// saved at 8-bit -- a plain `* 255` would do exactly that. Out-of-range values (a 32-bit document is
/// allowed to carry them) clamp to the displayable range, and that clamp is the loss we report.
fn narrow_samples(raw: &[u8], depth: u16, rows: usize, cols: usize) -> Vec<u8> {
    let count = rows * cols;
    let mut out = Vec::with_capacity(count);
    match depth {
        1 => {
            let stride = row_bytes(cols, 1);
            for row in 0..rows {
                for col in 0..cols {
                    let byte = raw.get(row * stride + col / 8).copied().unwrap_or(0);
                    let set = byte & (0x80 >> (col % 8)) != 0;
                    out.push(if set { 0 } else { 255 });
                }
            }
        }
        16 => {
            for i in 0..count {
                let hi = raw.get(i * 2).copied().unwrap_or(0);
                let lo = raw.get(i * 2 + 1).copied().unwrap_or(0);
                let value = u32::from(u16::from_be_bytes([hi, lo]));
                let scaled = (value * 255 + 16384) / 32768;
                out.push(scaled.min(255) as u8);
            }
        }
        32 => {
            for i in 0..count {
                let mut word = [0u8; 4];
                for (b, slot) in word.iter_mut().enumerate() {
                    *slot = raw.get(i * 4 + b).copied().unwrap_or(0);
                }
                let linear = f32::from_be_bytes(word);
                let linear = if linear.is_finite() {
                    f64::from(linear).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let encoded = crate::color::linear_to_srgb(linear);
                out.push((encoded * 255.0).round().clamp(0.0, 255.0) as u8);
            }
        }
        _ => {
            out.extend_from_slice(&raw[..count.min(raw.len())]);
            while out.len() < count {
                out.push(0);
            }
        }
    }
    out
}

/// Decode one channel plane to 8-bit samples.
///
/// `compression`: 0 raw, 1 PackBits/RLE, 2 ZIP, 3 ZIP with prediction. The two ZIP modes matter rather
/// than being exotic: Photoshop writes them for 16- and 32-bit documents, so a reader that only knows
/// raw and RLE opens almost no real deep file, which is what this item was about.
///
/// RLE and ZIP both work on the BYTE stream, so decoding always produces `count * sample_bytes` bytes
/// and narrows afterwards -- the depth never changes how the compression is undone.
fn decode_channel(
    reader: &mut Reader,
    compression: u16,
    rows: usize,
    cols: usize,
    depth: u16,
) -> Result<Vec<u8>> {
    let row_len = row_bytes(cols, depth);
    let raw_len = rows * row_len;
    match compression {
        0 => Ok(narrow_samples(reader.take(raw_len)?, depth, rows, cols)),
        1 => {
            // PackBits: a per-row byte-count table (u16 each), then the RLE streams.
            let mut row_lengths = Vec::with_capacity(rows);
            for _ in 0..rows {
                row_lengths.push(reader.u16()? as usize);
            }
            let mut out = Vec::with_capacity(raw_len);
            for &len in &row_lengths {
                let row = reader.take(len)?;
                unpack_bits(row, row_len, &mut out)?;
            }
            Ok(narrow_samples(&out, depth, rows, cols))
        }
        2 | 3 => {
            // The plane's remaining bytes are one zlib stream. A layer channel's length is known from
            // its record, but this reader is positional, so the stream is inflated from the rest of the
            // input and the inflate stops itself at the stream's end.
            let (raw, consumed) = inflate_plane(reader.remaining(), raw_len)?;
            reader.skip(consumed)?;
            let raw = if compression == 3 {
                undo_prediction(raw, rows, cols, depth)
            } else {
                raw
            };
            Ok(narrow_samples(&raw, depth, rows, cols))
        }
        _ => Err(FormatError::UnsupportedFeature("PSD compression").into()),
    }
}

/// Inflates one zlib stream, stopping at `limit` bytes of output, and reports how many input bytes it
/// consumed so the positional reader can be advanced past exactly this plane.
fn inflate_plane(input: &[u8], limit: usize) -> Result<(Vec<u8>, usize)> {
    let mut decompressor = flate2::Decompress::new(true);
    let mut out = Vec::with_capacity(limit);
    decompressor
        .decompress_vec(input, &mut out, flate2::FlushDecompress::Finish)
        .map_err(|_| FormatError::Malformed("PSD ZIP channel"))?;
    if out.len() < limit {
        return Err(FormatError::Malformed("PSD ZIP channel short").into());
    }
    out.truncate(limit);
    Ok((out, decompressor.total_in() as usize))
}

/// Undoes ZIP-with-prediction, which is a per-ROW delta so a row never predicts from its neighbour.
///
/// The delta's width follows the depth: bytes at 8-bit, big-endian 16-bit words at 16-bit. At 32-bit
/// the bytes of a row are also SHUFFLED into byte planes (every sample's first byte, then every
/// second byte, ...) before the byte-wise delta, so the un-shuffle has to happen after the sum.
fn undo_prediction(mut raw: Vec<u8>, rows: usize, cols: usize, depth: u16) -> Vec<u8> {
    let unit = sample_bytes(depth);
    let row_bytes = cols * unit;
    match depth {
        16 => {
            for row in 0..rows {
                let base = row * row_bytes;
                let mut previous = 0u16;
                for i in 0..cols {
                    let at = base + i * 2;
                    if at + 1 >= raw.len() {
                        break;
                    }
                    let delta = u16::from_be_bytes([raw[at], raw[at + 1]]);
                    let value = previous.wrapping_add(delta);
                    raw[at..at + 2].copy_from_slice(&value.to_be_bytes());
                    previous = value;
                }
            }
            raw
        }
        32 => {
            let mut out = vec![0u8; raw.len()];
            for row in 0..rows {
                let base = row * row_bytes;
                if base + row_bytes > raw.len() {
                    break;
                }
                // Byte-wise delta across the whole shuffled row.
                let mut previous = 0u8;
                for i in 0..row_bytes {
                    previous = previous.wrapping_add(raw[base + i]);
                    raw[base + i] = previous;
                }
                // Then un-shuffle: byte plane b of sample i sits at b * cols + i.
                for b in 0..unit {
                    for i in 0..cols {
                        out[base + i * unit + b] = raw[base + b * cols + i];
                    }
                }
            }
            out
        }
        _ => {
            for row in 0..rows {
                let base = row * row_bytes;
                let mut previous = 0u8;
                for i in 0..row_bytes {
                    if base + i >= raw.len() {
                        break;
                    }
                    previous = previous.wrapping_add(raw[base + i]);
                    raw[base + i] = previous;
                }
            }
            raw
        }
    }
}


/// PackBits decode of a single row into `out`, bounded to `cols` bytes.
fn unpack_bits(src: &[u8], cols: usize, out: &mut Vec<u8>) -> Result<()> {
    let target = out.len() + cols;
    let mut i = 0;
    while i < src.len() && out.len() < target {
        let n = src[i] as i8;
        i += 1;
        if n >= 0 {
            let run = n as usize + 1;
            if i + run > src.len() {
                return Err(FormatError::Malformed("PSD RLE overrun").into());
            }
            out.extend_from_slice(&src[i..i + run]);
            i += run;
        } else if n != -128 {
            let run = (1 - n as i32) as usize;
            if i >= src.len() {
                return Err(FormatError::Malformed("PSD RLE overrun").into());
            }
            let value = src[i];
            i += 1;
            out.extend(std::iter::repeat(value).take(run));
        }
    }
    out.truncate(target);
    while out.len() < target {
        out.push(0);
    }
    Ok(())
}

pub(crate) fn import_psd(
    bytes: &[u8],
    _options: &ImportOptions,
) -> Result<(Document, Vec<FormatWarning>)> {
    let mut r = Reader::new(bytes);
    if r.take(4)? != SIGNATURE {
        return Err(FormatError::Malformed("not a PSD (bad signature)").into());
    }
    let version = r.u16()?;
    if version != 1 {
        return Err(FormatError::UnsupportedFeature("PSB (large) files").into());
    }
    r.skip(6)?; // reserved
    let channels = r.u16()?;
    let height = r.u32()?;
    let width = r.u32()?;
    let depth = r.u16()?;
    let color_mode_tag = r.u16()?;
    let color_mode = ColorMode::from_tag(color_mode_tag)
        .ok_or(FormatError::UnsupportedFeature("PSD colour mode"))?;
    // 1 bit is bitmap mode's depth and ONLY bitmap mode's: a 1-bit RGB document does not exist, so
    // accepting the pair rather than the depth alone refuses a malformed header instead of shearing it.
    let depth_ok = match depth {
        1 => color_mode == ColorMode::Bitmap,
        8 | 16 | 32 => true,
        _ => false,
    };
    if !depth_ok {
        return Err(FormatError::UnsupportedFeature("PSD bit depth").into());
    }
    if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(FormatError::Malformed("PSD dimensions out of range").into());
    }
    // Colour mode data: for indexed mode this is the 768-byte planar palette, and without it an index
    // is only a number. Every other mode leaves it empty (duotone puts its ink curves here, which we
    // do not apply -- its stored plane is already the grey).
    let cmd_len = r.u32()? as usize;
    let palette = if color_mode == ColorMode::Indexed && cmd_len >= 768 {
        let table = r.take(768)?.to_vec();
        r.skip(cmd_len - 768)?;
        Some(table)
    } else {
        r.skip(cmd_len)?;
        None
    };
    let res_len = r.u32()? as usize;
    r.skip(res_len)?;

    let mut builder = DocumentImportBuilder::new(width, height)?;
    let mut warnings = Vec::new();
    if depth != 8 {
        warnings.push(FormatWarning::NarrowedDepth { source_bits: depth });
    }
    if color_mode != ColorMode::Rgb {
        warnings.push(FormatWarning::ConvertedColorMode {
            source: color_mode.label(),
        });
    }

    // Layer and mask information.
    let layer_mask_len = r.u32()? as usize;
    let mut had_layers = false;
    if layer_mask_len > 0 {
        let section_end = r.pos + layer_mask_len;
        let layer_info_len = r.u32()? as usize;
        if layer_info_len > 0 {
            let layer_info_end = r.pos + layer_info_len;
            let mut layer_count = r.i16()?;
            if layer_count < 0 {
                // Negative means the first alpha channel is the transparency of the merged result.
                layer_count = -layer_count;
            }
            had_layers = layer_count > 0;
            let layers = read_layers(
                &mut r,
                layer_count as usize,
                width,
                height,
                depth,
                color_mode,
                palette.as_deref(),
                &mut warnings,
            )?;
            // PSD layer records are bottom-first already, matching our sibling order.
            for layer in layers {
                builder.push_node(
                    ImportNode::raster(layer.name, vec![RasterCel::new(FrameId::DEFAULT, layer.pixels)])
                        .with_visibility(layer.visible)
                        .with_opacity(layer.opacity),
                )?;
            }
            r.pos = layer_info_end;
        }
        r.pos = section_end;
    }

    if !had_layers {
        // No layer section: decode the merged composite image as a single layer.
        let pixels = read_merged_image(
            &mut r,
            channels,
            width,
            height,
            depth,
            color_mode,
            palette.as_deref(),
        )?;
        builder.push_node(ImportNode::raster(
            "Background",
            vec![RasterCel::new(FrameId::DEFAULT, pixels)],
        ))?;
    }

    Ok((builder.build()?, warnings))
}

struct PsdLayer {
    name: String,
    pixels: Vec<u8>,
    opacity: f32,
    visible: bool,
}

fn read_layers(
    r: &mut Reader,
    count: usize,
    canvas_w: u32,
    canvas_h: u32,
    depth: u16,
    color_mode: ColorMode,
    palette: Option<&[u8]>,
    warnings: &mut Vec<FormatWarning>,
) -> Result<Vec<PsdLayer>> {
    // First pass: the records (geometry, channel list, blend info, name).
    struct Record {
        top: i32,
        left: i32,
        bottom: i32,
        right: i32,
        channels: Vec<(i16, usize)>, // (channel id, byte length)
        opacity: f32,
        visible: bool,
        name: String,
    }
    let mut records = Vec::with_capacity(count);
    for _ in 0..count {
        let top = r.i32()?;
        let left = r.i32()?;
        let bottom = r.i32()?;
        let right = r.i32()?;
        let nchannels = r.u16()? as usize;
        let mut channels = Vec::with_capacity(nchannels);
        for _ in 0..nchannels {
            let id = r.i16()?;
            let len = r.u32()? as usize;
            channels.push((id, len));
        }
        if r.take(4)? != b"8BIM" {
            return Err(FormatError::Malformed("PSD layer blend signature").into());
        }
        r.skip(4)?; // blend mode key
        let opacity = f32::from(r.u8()?) / 255.0;
        r.skip(1)?; // clipping
        let flags = r.u8()?;
        let visible = flags & 0x02 == 0; // bit 1 set = hidden
        r.skip(1)?; // filler
        let extra_len = r.u32()? as usize;
        let extra_end = r.pos + extra_len;
        // Layer mask data.
        let mask_len = r.u32()? as usize;
        r.skip(mask_len)?;
        // Blending ranges.
        let blend_len = r.u32()? as usize;
        r.skip(blend_len)?;
        // Pascal name, padded to a multiple of 4.
        let name_len = r.u8()? as usize;
        let raw_name = r.take(name_len)?;
        let name = String::from_utf8_lossy(raw_name).into_owned();
        let consumed = 1 + name_len;
        let pad = (4 - consumed % 4) % 4;
        r.skip(pad)?;
        r.pos = extra_end;
        records.push(Record {
            top,
            left,
            bottom,
            right,
            channels,
            opacity,
            visible,
            name: if name.is_empty() { "Layer".into() } else { name },
        });
    }
    // Second pass: the channel image data, in record order.
    let mut layers = Vec::with_capacity(count);
    for rec in records {
        let lw = (rec.right - rec.left).max(0) as usize;
        let lh = (rec.bottom - rec.top).max(0) as usize;
        let mut planes: std::collections::HashMap<i16, Vec<u8>> = std::collections::HashMap::new();
        for (id, _len) in &rec.channels {
            let compression = r.u16()?;
            let (rows, cols) = if *id == -2 {
                (lh, lw) // user mask — same geometry here
            } else {
                (lh, lw)
            };
            let plane = if rows == 0 || cols == 0 {
                Vec::new()
            } else {
                decode_channel(r, compression, rows, cols, depth)?
            };
            planes.insert(*id, plane);
        }
        // Compose into a full-canvas RGBA buffer is left to the caller; here produce the layer's own
        // rect packed RGBA, then place onto the canvas by top/left.
        //
        // Channel ids are POSITIONAL per colour mode: 0..n are that mode's own planes (grey, or
        // C/M/Y/K, or L/a/b), not red/green/blue. Reading id 0 as "red" is what made every non-RGB
        // file open as nonsense before this.
        let planes_count = color_mode.color_planes();
        let color: Vec<Vec<u8>> = (0..planes_count)
            .map(|index| planes.get(&(index as i16)).cloned().unwrap_or_default())
            .collect();
        let alpha = planes.get(&-1).cloned();
        let mut rect = vec![0u8; lw * lh * 4];
        let mut sample = vec![0u8; planes_count];
        for i in 0..(lw * lh) {
            for (plane, slot) in color.iter().zip(sample.iter_mut()) {
                *slot = plane.get(i).copied().unwrap_or(0);
            }
            let (r8, g8, b8) = color_mode.to_rgb(&sample, palette);
            rect[i * 4] = r8;
            rect[i * 4 + 1] = g8;
            rect[i * 4 + 2] = b8;
            rect[i * 4 + 3] = alpha.as_ref().map(|a| *a.get(i).unwrap_or(&255)).unwrap_or(255);
        }
        if alpha.is_none() {
            warnings.push(FormatWarning::FlattenedAlpha { matte: crate::Pixel::TRANSPARENT });
        }
        layers.push(PsdLayer {
            name: rec.name,
            pixels: place_rect(&rect, lw, lh, rec.left, rec.top, canvas_w, canvas_h),
            opacity: rec.opacity,
            visible: rec.visible,
        });
    }
    Ok(layers)
}

/// Place a layer's own rect buffer onto a full canvas-sized RGBA buffer at `(left, top)`, clipping to
/// the canvas. Pixels outside the rect stay transparent.
fn place_rect(
    rect: &[u8],
    rw: usize,
    rh: usize,
    left: i32,
    top: i32,
    canvas_w: u32,
    canvas_h: u32,
) -> Vec<u8> {
    let cw = canvas_w as usize;
    let ch = canvas_h as usize;
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
            out[d..d + 4].copy_from_slice(&rect[so..so + 4]);
        }
    }
    out
}

fn read_merged_image(
    r: &mut Reader,
    channels: u16,
    width: u32,
    height: u32,
    depth: u16,
    color_mode: ColorMode,
    palette: Option<&[u8]>,
) -> Result<Vec<u8>> {
    let w = width as usize;
    let h = height as usize;
    let compression = r.u16()?;
    let nchan = channels as usize;
    let stride = row_bytes(w, depth);
    
    let mut planes = Vec::with_capacity(nchan);
    match compression {
        1 => {
            // One big row-length table for ALL channels, then the RLE data.
            let total_rows = h * nchan;
            let mut row_lengths = Vec::with_capacity(total_rows);
            for _ in 0..total_rows {
                row_lengths.push(r.u16()? as usize);
            }
            let mut consumed_rows = 0;
            for _ in 0..nchan {
                let mut plane = Vec::with_capacity(stride * h);
                for _ in 0..h {
                    let len = row_lengths[consumed_rows];
                    consumed_rows += 1;
                    let row = r.take(len)?;
                    unpack_bits(row, stride, &mut plane)?;
                }
                planes.push(narrow_samples(&plane, depth, h, w));
            }
        }
        2 | 3 => {
            // The merged image's channels share ONE zlib stream here, unlike a layer channel, so the
            // whole composite is inflated once and then split per channel.
            let (raw, consumed) = inflate_plane(r.remaining(), stride * h * nchan)?;
            r.skip(consumed)?;
            for index in 0..nchan {
                let start = index * stride * h;
                let plane = raw[start..start + stride * h].to_vec();
                let plane = if compression == 3 {
                    undo_prediction(plane, h, w, depth)
                } else {
                    plane
                };
                planes.push(narrow_samples(&plane, depth, h, w));
            }
        }
        0 => {
            for _ in 0..nchan {
                let plane = r.take(stride * h)?.to_vec();
                planes.push(narrow_samples(&plane, depth, h, w));
            }
        }
        _ => return Err(FormatError::UnsupportedFeature("PSD compression").into()),
    }
    // The merged image's planes are POSITIONAL in the colour mode's own order, with any alpha after
    // them -- so a CMYK composite's fourth plane is black ink, not alpha, and reading it as alpha is
    // how a print document used to open mostly invisible.
    let color_planes = color_mode.color_planes();
    let mut rgba = vec![0u8; w * h * 4];
    let mut sample = vec![0u8; color_planes];
    for i in 0..(w * h) {
        for (index, slot) in sample.iter_mut().enumerate() {
            *slot = planes.get(index).and_then(|p| p.get(i)).copied().unwrap_or(0);
        }
        let (r8, g8, b8) = color_mode.to_rgb(&sample, palette);
        rgba[i * 4] = r8;
        rgba[i * 4 + 1] = g8;
        rgba[i * 4 + 2] = b8;
        rgba[i * 4 + 3] = if nchan > color_planes {
            planes
                .get(color_planes)
                .and_then(|p| p.get(i))
                .copied()
                .unwrap_or(255)
        } else {
            255
        };
    }
    Ok(rgba)
}

// ---- Export ----------------------------------------------------------------

fn write_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}
fn write_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}
fn write_i32(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_be_bytes());
}

pub(crate) fn export_psd(
    document: &Document,
    frame: FrameId,
    options: &ExportOptions,
) -> Result<(Vec<u8>, Vec<FormatWarning>)> {
    let mut warnings = Vec::new();
    let width = document.width();
    let height = document.height();
    let w = width as usize;
    let h = height as usize;

    // Collect raster layers (document order is bottom-first, matching PSD). Groups and vector/text
    // nodes are rasterized to their own canvas-sized buffer via source_pixels.
    let mut layers: Vec<(String, f32, bool, Vec<u8>)> = Vec::new();
    for node in document.nodes() {
        if matches!(node.kind(), NodeKind::Group) {
            warnings.push(FormatWarning::FlattenedHierarchy);
            continue;
        }
        let pixels = source_pixels(document, node, frame)?;
        layers.push((
            node.name().to_string(),
            node.opacity(),
            node.is_visible(),
            pixels,
        ));
    }
    if layers.is_empty() {
        return Err(FormatError::Malformed("PSD export needs at least one raster layer").into());
    }

    let mut out = Vec::new();
    // --- Header ---
    out.extend_from_slice(SIGNATURE);
    write_u16(&mut out, 1); // version
    out.extend_from_slice(&[0u8; 6]); // reserved
    write_u16(&mut out, 4); // channels in the composite (RGBA)
    write_u32(&mut out, height);
    write_u32(&mut out, width);
    write_u16(&mut out, 8); // depth
    write_u16(&mut out, 3); // RGB
    write_u32(&mut out, 0); // color mode data length
    write_u32(&mut out, 0); // image resources length

    // --- Layer and mask section ---
    let mut layer_section = Vec::new();
    // Layer info.
    let mut layer_info = Vec::new();
    write_u16(&mut layer_info, layers.len() as u16);
    // Per-layer: we write raw (uncompressed) channel data for R,G,B,A (ids 0,1,2,-1).
    let mut channel_blobs: Vec<Vec<Vec<u8>>> = Vec::new();
    for (name, opacity, visible, pixels) in &layers {
        write_i32(&mut layer_info, 0); // top
        write_i32(&mut layer_info, 0); // left
        write_i32(&mut layer_info, height as i32); // bottom
        write_i32(&mut layer_info, width as i32); // right
        write_u16(&mut layer_info, 4); // channel count
        // Each channel: id (i16) + data length (u32) = per-plane 2 bytes (compression) + w*h.
        let plane_len = 2 + w * h;
        for id in [0i16, 1, 2, -1] {
            layer_info.extend_from_slice(&id.to_be_bytes());
            write_u32(&mut layer_info, plane_len as u32);
        }
        layer_info.extend_from_slice(b"8BIM");
        layer_info.extend_from_slice(b"norm"); // blend mode: normal
        layer_info.push((opacity * 255.0).round().clamp(0.0, 255.0) as u8);
        layer_info.push(0); // clipping
        layer_info.push(if *visible { 0 } else { 0x02 }); // flags
        layer_info.push(0); // filler
        // Extra data: mask (0) + blending ranges (0) + name (pascal, padded to 4).
        let mut extra = Vec::new();
        write_u32(&mut extra, 0); // mask
        write_u32(&mut extra, 0); // blending ranges
        let name_bytes = name.as_bytes();
        let name_len = name_bytes.len().min(255);
        extra.push(name_len as u8);
        extra.extend_from_slice(&name_bytes[..name_len]);
        let consumed = 1 + name_len;
        let pad = (4 - consumed % 4) % 4;
        extra.extend(std::iter::repeat(0u8).take(pad));
        write_u32(&mut layer_info, extra.len() as u32);
        layer_info.extend_from_slice(&extra);

        // Build this layer's four planes (raw), stored for the channel-data phase.
        let mut planes = Vec::with_capacity(4);
        for c in 0..4 {
            let mut plane = vec![0u8; 2 + w * h]; // leading u16 compression = 0 (raw)
            for i in 0..(w * h) {
                plane[2 + i] = pixels[i * 4 + c];
            }
            planes.push(plane);
        }
        channel_blobs.push(planes);
    }
    // Channel image data, in layer then channel order.
    for planes in &channel_blobs {
        for plane in planes {
            layer_info.extend_from_slice(plane);
        }
    }
    // layer info is padded to even length.
    if layer_info.len() % 2 == 1 {
        layer_info.push(0);
    }
    write_u32(&mut layer_section, layer_info.len() as u32);
    layer_section.extend_from_slice(&layer_info);
    write_u32(&mut layer_section, 0); // global layer mask info length

    write_u32(&mut out, layer_section.len() as u32);
    out.extend_from_slice(&layer_section);

    // --- Merged composite image (raw planes R,G,B,A) ---
    let snapshot = RenderSnapshot::try_render_frame(document, 0, frame)?;
    let merged = snapshot.pixels();
    write_u16(&mut out, 0); // compression = raw
    for c in 0..4 {
        for i in 0..(w * h) {
            out.push(merged[i * 4 + c]);
        }
    }

    let _ = options;
    Ok((out, warnings))
}

/// Full-canvas RGBA for one node (raster taken directly; vector/text rasterized; mask baked).
fn source_pixels(document: &Document, node: &crate::Layer, frame: FrameId) -> Result<Vec<u8>> {
    let mut pixels = match node.kind() {
        NodeKind::Raster => node.raster_pixels(frame).map_or_else(
            |_| vec![0; document.width() as usize * document.height() as usize * 4],
            <[u8]>::to_vec,
        ),
        NodeKind::Text | NodeKind::Vector => {
            crate::semantic::rasterize(node.content(), document.width(), document.height())?
        }
        NodeKind::Group => unreachable!(),
    };
    if let Some(mask) = node.mask().filter(|mask| mask.is_enabled()) {
        for (pixel, coverage) in pixels.chunks_exact_mut(4).zip(mask.pixels()) {
            pixel[3] = ((u16::from(pixel[3]) * u16::from(*coverage) + 127) / 255) as u8;
        }
    }
    Ok(pixels)
}
