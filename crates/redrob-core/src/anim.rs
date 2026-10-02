// SPDX-License-Identifier: GPL-3.0-or-later

//! Animation export: animated GIF, APNG and (flagged) animated WebP. Re-derived from the GIF89a and
//! APNG specifications and the shape of GIMP's animation exporters (behaviour studied, no code
//! copied). Every timeline frame is rendered to a full composite and written as one animation frame,
//! using each frame's own `duration_ms`.

use std::io::Cursor;

use image::codecs::gif::{GifEncoder, Repeat};
use image::{Delay, Frame as ImgFrame, RgbaImage};

use crate::{Document, FormatError, FormatWarning, Result, RenderSnapshot};

/// Render every timeline frame to a full-canvas RGBA composite, paired with its duration in ms.
fn render_frames(document: &Document) -> Result<Vec<(RgbaImage, u32)>> {
    let width = document.width();
    let height = document.height();
    let mut frames = Vec::new();
    for frame in document.timeline().frames() {
        let snapshot = RenderSnapshot::try_render_frame(document, 0, frame.id())?;
        let image: RgbaImage = image::ImageBuffer::from_raw(width, height, snapshot.pixels().to_vec())
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
    Ok((bytes, vec![FormatWarning::FlattenedAlpha { matte: crate::Pixel::TRANSPARENT }]))
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
    let mut encoder =
        flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = encoder.write_all(data);
    encoder.finish().unwrap_or_default()
}
