// SPDX-License-Identifier: GPL-3.0-or-later

//! Adobe Photoshop (PSD) import/export, re-derived from the PSD file-format spec and the layout of
//! GIMP's `plug-ins/common/file-psd` and Krita's `plugins/impex/psd` (behaviour studied, no code
//! copied). We support the common case: 8-bit RGB(A), raw or PackBits (RLE) channel data, with the
//! layer records preserved so a round-trip keeps the layer stack.
//!
//! Not yet covered (recorded as warnings on export, ignored on import): 16/32-bit depth, CMYK/Lab,
//! layer masks beyond alpha, adjustment layers, and image resources. Those are later passes.

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
}

/// Decode one channel plane: `compression` 0 = raw, 1 = PackBits/RLE. `count` is pixel count.
fn decode_channel(reader: &mut Reader, compression: u16, rows: usize, cols: usize) -> Result<Vec<u8>> {
    let count = rows * cols;
    match compression {
        0 => Ok(reader.take(count)?.to_vec()),
        1 => {
            // PackBits: a per-row byte-count table (u16 each), then the RLE streams.
            let mut row_lengths = Vec::with_capacity(rows);
            for _ in 0..rows {
                row_lengths.push(reader.u16()? as usize);
            }
            let mut out = Vec::with_capacity(count);
            for &len in &row_lengths {
                let row = reader.take(len)?;
                unpack_bits(row, cols, &mut out)?;
            }
            Ok(out)
        }
        _ => Err(FormatError::Unsupported("PSD compression").into()),
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
        return Err(FormatError::Unsupported("PSB (large) files").into());
    }
    r.skip(6)?; // reserved
    let channels = r.u16()?;
    let height = r.u32()?;
    let width = r.u32()?;
    let depth = r.u16()?;
    let color_mode = r.u16()?;
    if depth != 8 {
        return Err(FormatError::Unsupported("non-8-bit PSD").into());
    }
    if color_mode != 3 {
        return Err(FormatError::Unsupported("non-RGB PSD").into());
    }
    if width == 0 || height == 0 || width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(FormatError::Malformed("PSD dimensions out of range").into());
    }
    // Skip color mode data, image resources.
    let cmd_len = r.u32()? as usize;
    r.skip(cmd_len)?;
    let res_len = r.u32()? as usize;
    r.skip(res_len)?;

    let mut builder = DocumentImportBuilder::new(width, height)?;
    let mut warnings = Vec::new();

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
            let layers = read_layers(&mut r, layer_count as usize, width, height, &mut warnings)?;
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
        let pixels = read_merged_image(&mut r, channels, width, height)?;
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
                decode_channel(r, compression, rows, cols)?
            };
            planes.insert(*id, plane);
        }
        // Compose into a full-canvas RGBA buffer is left to the caller; here produce the layer's own
        // rect packed RGBA, then place onto the canvas by top/left.
        let red = planes.get(&0).cloned().unwrap_or_default();
        let green = planes.get(&1).cloned().unwrap_or_default();
        let blue = planes.get(&2).cloned().unwrap_or_default();
        let alpha = planes.get(&-1).cloned();
        let mut rect = vec![0u8; lw * lh * 4];
        for i in 0..(lw * lh) {
            rect[i * 4] = *red.get(i).unwrap_or(&0);
            rect[i * 4 + 1] = *green.get(i).unwrap_or(&0);
            rect[i * 4 + 2] = *blue.get(i).unwrap_or(&0);
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
) -> Result<Vec<u8>> {
    let w = width as usize;
    let h = height as usize;
    let compression = r.u16()?;
    let nchan = channels as usize;
    let mut planes = Vec::with_capacity(nchan);
    if compression == 1 {
        // One big row-length table for ALL channels, then the RLE data.
        let total_rows = h * nchan;
        let mut row_lengths = Vec::with_capacity(total_rows);
        for _ in 0..total_rows {
            row_lengths.push(r.u16()? as usize);
        }
        let mut consumed_rows = 0;
        for _ in 0..nchan {
            let mut plane = Vec::with_capacity(w * h);
            for _ in 0..h {
                let len = row_lengths[consumed_rows];
                consumed_rows += 1;
                let row = r.take(len)?;
                unpack_bits(row, w, &mut plane)?;
            }
            planes.push(plane);
        }
    } else {
        for _ in 0..nchan {
            planes.push(r.take(w * h)?.to_vec());
        }
    }
    let mut rgba = vec![0u8; w * h * 4];
    for i in 0..(w * h) {
        rgba[i * 4] = *planes.first().and_then(|p| p.get(i)).unwrap_or(&0);
        rgba[i * 4 + 1] = *planes.get(1).and_then(|p| p.get(i)).unwrap_or(&0);
        rgba[i * 4 + 2] = *planes.get(2).and_then(|p| p.get(i)).unwrap_or(&0);
        rgba[i * 4 + 3] = if nchan >= 4 {
            *planes.get(3).and_then(|p| p.get(i)).unwrap_or(&255)
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
