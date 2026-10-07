// SPDX-License-Identifier: GPL-3.0-or-later

//! Animation export: animated GIF, APNG and animated WebP. Re-derived from the GIF89a, APNG and WebP
//! container specifications and the shape of GIMP's animation exporters (behaviour studied, no code
//! copied). Every timeline frame is rendered to a full composite and written as one animation frame,
//! using each frame's own `duration_ms`.

use std::io::{Cursor, Write};

use image::codecs::gif::{GifEncoder, Repeat};
use image::{Delay, Frame as ImgFrame, ImageEncoder, RgbaImage};

use crate::{Document, FormatError, FormatWarning, RenderSnapshot, Result};

/// Render every timeline frame to a full-canvas RGBA composite, paired with its duration in ms.
fn render_frames(document: &Document) -> Result<Vec<(RgbaImage, u32)>> {
    let width = document.width();
    let height = document.height();
    let mut frames = Vec::new();
    for frame in document.timeline().frames() {
        let snapshot = RenderSnapshot::try_render_frame(document, 0, frame.id())?;
        let image: RgbaImage =
            image::ImageBuffer::from_raw(width, height, snapshot.pixels().to_vec())
                .ok_or(FormatError::OutputTooLarge)?;
        frames.push((image, frame.duration_ms().max(10)));
    }
    if frames.is_empty() {
        return Err(FormatError::Malformed("no frames to animate").into());
    }
    Ok(frames)
}

pub(crate) fn export_animated_gif(document: &Document) -> Result<(Vec<u8>, Vec<FormatWarning>)> {
    let frames = render_frames(document)?;
    let mut bytes = Vec::new();
    {
        let mut encoder = GifEncoder::new(Cursor::new(&mut bytes));
        encoder
            .set_repeat(Repeat::Infinite)
            .map_err(|_| FormatError::OutputTooLarge)?;
        for (image, duration_ms) in frames {
            let delay = Delay::from_numer_denom_ms(duration_ms, 1);
            encoder
                .encode_frame(ImgFrame::from_parts(image, 0, 0, delay))
                .map_err(|_| FormatError::OutputTooLarge)?;
        }
    }
    if bytes.len() > crate::MAX_FORMAT_OUTPUT_BYTES {
        return Err(FormatError::OutputTooLarge.into());
    }
    // GIF is 256-colour per frame; flag the quantisation loss.
    Ok((
        bytes,
        vec![FormatWarning::FlattenedAlpha {
            matte: crate::Pixel::TRANSPARENT,
        }],
    ))
}

// ---- APNG ------------------------------------------------------------------

fn crc32(bytes: &[u8]) -> u32 {
    // Standard CRC-32 (IEEE), computed without a precomputed table (small inputs).
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// Encodes a palette PNG: colour type 3, with a `PLTE` chunk and a `tRNS` chunk when any entry is
/// not opaque (J.3).
///
/// Hand-written because the image crate's encoder has no indexed colour type, and the chunk writer
/// this file already needed for APNG is the same machinery. An indexed document exported as RGBA
/// would carry its palette nowhere, which is most of the point of the mode.
///
/// `indices` is one byte per pixel, as [`crate::color_mode::quantize`] returned them — not
/// recomputed here. A second nearest-colour search could disagree with the first, and then the
/// file's pixels would not be the ones on screen.
pub(crate) fn export_indexed_png(
    width: u32,
    height: u32,
    indices: &[u8],
    palette: &[crate::Pixel],
) -> Result<Vec<u8>> {
    if palette.is_empty() || palette.len() > crate::MAX_PALETTE_COLORS {
        return Err(crate::CoreError::InvalidPalette(palette.len()));
    }
    let mut out = Vec::new();
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);

    // IHDR: bit depth 8, colour type 3 (indexed). Depth 8 even for a two-colour palette: 1-, 2- and
    // 4-bit depths need their rows bit-packed, and the saving is not worth a second packer.
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 3, 0, 0, 0]);
    write_chunk(&mut out, b"IHDR", &ihdr);

    // PLTE: three bytes per entry, RGB only. It must come before IDAT, and a colour-type-3 image
    // without it is invalid rather than defaulted.
    let mut plte = Vec::with_capacity(palette.len() * 3);
    for entry in palette {
        plte.extend_from_slice(&[entry.r, entry.g, entry.b]);
    }
    write_chunk(&mut out, b"PLTE", &plte);

    // tRNS carries the palette's alpha, one byte per entry, and may be SHORTER than the palette --
    // entries past its end are opaque. Written only when something is actually transparent, because
    // an all-255 tRNS is bytes that say nothing.
    if palette.iter().any(|entry| entry.a != 255) {
        let last_transparent = palette
            .iter()
            .rposition(|entry| entry.a != 255)
            .expect("just checked one exists");
        let trns: Vec<u8> = palette[..=last_transparent]
            .iter()
            .map(|entry| entry.a)
            .collect();
        write_chunk(&mut out, b"tRNS", &trns);
    }

    // Each row is prefixed with its filter byte. Filter 0 (none) because the rows are palette
    // indices: a difference filter on index numbers compresses the ORDER of the palette rather than
    // the picture, and can easily make the file larger.
    let stride = width as usize;
    let mut raw = Vec::with_capacity((stride + 1) * height as usize);
    for row in 0..height as usize {
        raw.push(0);
        let start = row * stride;
        raw.extend_from_slice(&indices[start..start + stride]);
    }
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(&raw)
        .map_err(|error| crate::CoreError::MalformedProject(error.to_string()))?;
    let compressed = encoder
        .finish()
        .map_err(|error| crate::CoreError::MalformedProject(error.to_string()))?;
    write_chunk(&mut out, b"IDAT", &compressed);
    write_chunk(&mut out, b"IEND", &[]);
    Ok(out)
}

/// Encode an APNG by hand: a PNG stream whose IDAT is the first frame, plus acTL/fcTL/fdAT chunks for
/// the animation. Each frame is a full-canvas RGBA8 image compressed to one IDAT/fdAT.
pub(crate) fn export_apng(document: &Document) -> Result<(Vec<u8>, Vec<FormatWarning>)> {
    let frames = render_frames(document)?;
    let width = document.width();
    let height = document.height();

    let mut out = Vec::new();
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);

    // IHDR: width, height, bit depth 8, colour type 6 (RGBA), no interlace.
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    write_chunk(&mut out, b"IHDR", &ihdr);

    // acTL: number of frames, play count (0 = infinite).
    let mut actl = Vec::new();
    actl.extend_from_slice(&(frames.len() as u32).to_be_bytes());
    actl.extend_from_slice(&0u32.to_be_bytes());
    write_chunk(&mut out, b"acTL", &actl);

    let mut sequence = 0u32;
    for (index, (image, duration_ms)) in frames.iter().enumerate() {
        // fcTL for this frame.
        let mut fctl = Vec::new();
        fctl.extend_from_slice(&sequence.to_be_bytes());
        sequence += 1;
        fctl.extend_from_slice(&width.to_be_bytes());
        fctl.extend_from_slice(&height.to_be_bytes());
        fctl.extend_from_slice(&0u32.to_be_bytes()); // x offset
        fctl.extend_from_slice(&0u32.to_be_bytes()); // y offset
        fctl.extend_from_slice(&(*duration_ms as u16).to_be_bytes()); // delay numerator
        fctl.extend_from_slice(&1000u16.to_be_bytes()); // delay denominator (ms)
        fctl.push(0); // dispose_op = none
        fctl.push(0); // blend_op = source
        write_chunk(&mut out, b"fcTL", &fctl);

        let raw = filtered_scanlines(image.as_raw(), width, height);
        let compressed = zlib_compress(&raw);
        if index == 0 {
            write_chunk(&mut out, b"IDAT", &compressed);
        } else {
            // fdAT: a sequence number prefix then the frame data.
            let mut fdat = Vec::with_capacity(4 + compressed.len());
            fdat.extend_from_slice(&sequence.to_be_bytes());
            sequence += 1;
            fdat.extend_from_slice(&compressed);
            write_chunk(&mut out, b"fdAT", &fdat);
        }
    }
    write_chunk(&mut out, b"IEND", &[]);

    if out.len() > crate::MAX_FORMAT_OUTPUT_BYTES {
        return Err(FormatError::OutputTooLarge.into());
    }
    Ok((out, Vec::new()))
}

/// Prefix each row with filter byte 0 (none), as PNG requires before compression.
fn filtered_scanlines(rgba: &[u8], width: u32, height: u32) -> Vec<u8> {
    let stride = width as usize * 4;
    let mut out = Vec::with_capacity((stride + 1) * height as usize);
    for y in 0..height as usize {
        out.push(0);
        out.extend_from_slice(&rgba[y * stride..(y + 1) * stride]);
    }
    out
}

/// zlib-compress with the `image` crate's bundled deflate via the png encoder would re-wrap chunks,
/// so compress directly with flate2 (already a transitive dep through png). We call the png crate's
/// re-export of the compressor through a minimal stored/deflate path.
fn zlib_compress(data: &[u8]) -> Vec<u8> {
    use std::io::Write;
    // flate2 is pulled in by the png backend; use its zlib encoder.
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = encoder.write_all(data);
    encoder.finish().unwrap_or_default()
}

// ---- Animated WebP ---------------------------------------------------------

/// Writes an animated WebP by assembling the RIFF container itself (H.12).
///
/// Re-derived from the WebP container specification. The one thing NOT re-written here is the per-frame
/// lossless bitstream: each frame is encoded by the same lossless encoder this product already uses for
/// a still WebP, and its `VP8L` chunk is lifted out and placed inside an `ANMF`. Writing a second VP8L
/// encoder would be a worse file produced by less-tested code, and the container -- which is what was
/// actually missing -- is the part this module owns.
///
/// Three container details are load-bearing:
///
/// 1. An animation REQUIRES the extended header (`VP8X`) with its animation flag. A reader that finds a
///    bare `VP8L` treats the file as a single still image, so the frames after the first simply vanish.
/// 2. `VP8X` stores canvas size MINUS ONE, in 24 bits. Writing the true size yields a canvas one pixel
///    too large on every axis, which looks like a border rather than a header bug.
/// 3. Every RIFF chunk is padded to an EVEN length, and the pad byte is not counted in the chunk's own
///    size but IS part of the enclosing size. Getting that wrong shifts every chunk after the first odd
///    one.
pub(crate) fn export_animated_webp(document: &Document) -> Result<(Vec<u8>, Vec<FormatWarning>)> {
    let frames = render_frames(document)?;
    let width = document.width();
    let height = document.height();
    if width == 0 || height == 0 || width > 1 << 24 || height > 1 << 24 {
        return Err(FormatError::Malformed("WebP canvas size").into());
    }

    let has_alpha = frames
        .iter()
        .any(|(image, _)| image.as_raw().chunks_exact(4).any(|pixel| pixel[3] != 255));

    let mut body: Vec<u8> = Vec::new();

    // VP8X: the extended header. Bit 1 is animation, bit 4 is alpha.
    let mut vp8x = Vec::with_capacity(10);
    let mut flags = 0x02u8;
    if has_alpha {
        flags |= 0x10;
    }
    vp8x.push(flags);
    vp8x.extend_from_slice(&[0, 0, 0]); // reserved
    write_u24(&mut vp8x, width - 1);
    write_u24(&mut vp8x, height - 1);
    write_riff_chunk(&mut body, b"VP8X", &vp8x);

    // ANIM: background colour then loop count. Zero means loop forever, which is what every other
    // animation this product writes does.
    let mut anim = Vec::with_capacity(6);
    anim.extend_from_slice(&[0, 0, 0, 0]); // background: transparent
    anim.extend_from_slice(&0u16.to_le_bytes()); // loop count: infinite
    write_riff_chunk(&mut body, b"ANIM", &anim);

    for (image, duration_ms) in &frames {
        let bitstream = lossless_bitstream(image)?;
        let mut anmf = Vec::with_capacity(16 + bitstream.len());
        // Frame offsets are stored in units of TWO pixels, so an odd offset cannot be expressed. Ours
        // are always zero (every frame is a full-canvas composite), which sidesteps that entirely.
        write_u24(&mut anmf, 0); // x / 2
        write_u24(&mut anmf, 0); // y / 2
        write_u24(&mut anmf, width - 1);
        write_u24(&mut anmf, height - 1);
        write_u24(&mut anmf, (*duration_ms).min(0xff_ffff));
        // Blending and disposal: each frame is a complete composite, so the previous frame must be
        // REPLACED rather than blended under it. Bit 1 clears the canvas to the background first and
        // bit 0 disables alpha blending -- without both, a transparent area would show the frame before.
        anmf.push(0x03);
        write_riff_chunk(&mut anmf, b"VP8L", &bitstream);
        write_riff_chunk(&mut body, b"ANMF", &anmf);
    }

    let mut out = Vec::with_capacity(body.len() + 12);
    out.extend_from_slice(b"RIFF");
    // The RIFF size covers "WEBP" plus every chunk, and not the eight bytes of its own header.
    out.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
    out.extend_from_slice(b"WEBP");
    out.extend_from_slice(&body);
    if out.len() > crate::MAX_FORMAT_OUTPUT_BYTES {
        return Err(FormatError::OutputTooLarge.into());
    }
    Ok((out, Vec::new()))
}

/// Encodes one frame losslessly and returns just its `VP8L` payload.
///
/// The still encoder produces a complete one-chunk WebP file, so the chunk is located by walking the
/// RIFF rather than assumed to start at a fixed offset -- an encoder that also emitted an `ICCP` or
/// `EXIF` chunk would otherwise have its metadata read as image data.
fn lossless_bitstream(image: &RgbaImage) -> Result<Vec<u8>> {
    let mut still = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut still)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ColorType::Rgba8.into(),
        )
        .map_err(|_| FormatError::Malformed("WebP frame encode"))?;
    find_riff_chunk(&still, b"VP8L")
        .ok_or_else(|| FormatError::Malformed("WebP frame has no VP8L chunk").into())
}

/// Finds one chunk's payload in a RIFF file, honouring the even-length padding.
fn find_riff_chunk(bytes: &[u8], kind: &[u8; 4]) -> Option<Vec<u8>> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return None;
    }
    let mut at = 12usize;
    while at + 8 <= bytes.len() {
        let tag = &bytes[at..at + 4];
        let size = u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]])
            as usize;
        let start = at + 8;
        let end = start.checked_add(size)?;
        if end > bytes.len() {
            return None;
        }
        if tag == kind {
            return Some(bytes[start..end].to_vec());
        }
        at = end + (size % 2);
    }
    None
}

fn write_u24(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes()[..3]);
}

/// Writes a RIFF chunk: a four-byte tag, a little-endian size that EXCLUDES the pad, the payload, and a
/// pad byte when the payload length is odd.
fn write_riff_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(kind);
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
}
