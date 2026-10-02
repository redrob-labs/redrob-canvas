// SPDX-License-Identifier: GPL-3.0-or-later

//! DirectDraw Surface (DDS) export with block compression, re-derived from the DDS header layout and
//! the BC1/BC3 encoders in GIMP's `plug-ins/file-dds` (behaviour studied, no code copied).
//!
//! Import goes through the `image` crate, which decodes DDS but cannot encode it — that is why this
//! module exists at all. A DDS reader is useless to a game artist who then cannot save one back.
//!
//! Block compression is LOSSY, and deliberately so: DDS exists because a GPU samples these blocks
//! directly. A 4x4 block keeps two endpoint colours and two bits per pixel, so a block holding more
//! than four distinct colours cannot come back exactly. The export reports that as a loss rather than
//! implying a round-trip is exact.
//!
//! Which form is written is decided by the IMAGE, not by an option: a fully opaque image is BC1 (half
//! the bytes, no alpha block), anything with transparency is BC3. Writing BC1 for an image with alpha
//! would silently discard it, and writing BC3 for an opaque one doubles the file for an alpha block
//! that is entirely 255.

use crate::{FormatError, Result};

const MAGIC: &[u8; 4] = b"DDS ";
const HEADER_SIZE: u32 = 124;
const PIXELFORMAT_SIZE: u32 = 32;

// Header flags: caps, height, width, pixel format, and the compressed size.
const DDSD_CAPS: u32 = 0x1;
const DDSD_HEIGHT: u32 = 0x2;
const DDSD_WIDTH: u32 = 0x4;
const DDSD_PIXELFORMAT: u32 = 0x1000;
const DDSD_LINEARSIZE: u32 = 0x0008_0000;
const DDPF_FOURCC: u32 = 0x4;
const DDSCAPS_TEXTURE: u32 = 0x1000;

/// Encodes an RGBA buffer as a DDS with BC1 or BC3 blocks, chosen from the image's own alpha.
pub(crate) fn encode_dds(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    if width == 0 || height == 0 {
        return Err(FormatError::Malformed("DDS dimensions").into());
    }
    let w = width as usize;
    let h = height as usize;
    if rgba.len() < w * h * 4 {
        return Err(FormatError::Malformed("DDS pixel buffer too short").into());
    }
    let has_alpha = rgba.chunks_exact(4).any(|pixel| pixel[3] != 255);

    let blocks_x = w.div_ceil(4);
    let blocks_y = h.div_ceil(4);
    let block_bytes = if has_alpha { 16 } else { 8 };
    let mut data = Vec::with_capacity(blocks_x * blocks_y * block_bytes);
    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            // A block always covers 4x4 even at the right and bottom edges. The pixels past the image
            // REPEAT the edge rather than being zero: a block is compressed as a unit, so a black pad
            // would drag the endpoints of every edge block toward black and darken the visible pixels
            // inside it.
            let mut block = [[0u8; 4]; 16];
            for y in 0..4 {
                for x in 0..4 {
                    let sx = (bx * 4 + x).min(w - 1);
                    let sy = (by * 4 + y).min(h - 1);
                    let at = (sy * w + sx) * 4;
                    block[y * 4 + x].copy_from_slice(&rgba[at..at + 4]);
                }
            }
            if has_alpha {
                data.extend_from_slice(&encode_alpha_block(&block));
            }
            data.extend_from_slice(&encode_color_block(&block));
        }
    }

    let mut out = Vec::with_capacity(128 + data.len());
    out.extend_from_slice(MAGIC);
    write_u32(&mut out, HEADER_SIZE);
    write_u32(
        &mut out,
        DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_PIXELFORMAT | DDSD_LINEARSIZE,
    );
    write_u32(&mut out, height);
    write_u32(&mut out, width);
    // For a block-compressed surface this field is the TOTAL byte size, not a row pitch.
    write_u32(&mut out, data.len() as u32);
    write_u32(&mut out, 0); // depth
    write_u32(&mut out, 0); // mip map count: none
    for _ in 0..11 {
        write_u32(&mut out, 0); // reserved
    }
    // Pixel format: a four-character code, which is how a block-compressed surface declares itself.
    write_u32(&mut out, PIXELFORMAT_SIZE);
    write_u32(&mut out, DDPF_FOURCC);
    out.extend_from_slice(if has_alpha { b"DXT5" } else { b"DXT1" });
    for _ in 0..5 {
        write_u32(&mut out, 0); // bit count and the four channel masks, unused with a FourCC
    }
    write_u32(&mut out, DDSCAPS_TEXTURE);
    write_u32(&mut out, 0); // caps2
    write_u32(&mut out, 0); // caps3
    write_u32(&mut out, 0); // caps4
    write_u32(&mut out, 0); // reserved2
    out.extend_from_slice(&data);
    Ok(out)
}

/// The four-character code this buffer will be written as, which is decided by its alpha alone.
pub(crate) fn fourcc_for(rgba: &[u8]) -> &'static str {
    if rgba.chunks_exact(4).any(|pixel| pixel[3] != 255) {
        "DXT5"
    } else {
        "DXT1"
    }
}

/// True when this image cannot survive block compression unchanged, which is almost always.
///
/// A 4x4 block keeps four colours on a line between two endpoints, so only a block whose pixels already
/// lie on such a line comes back exactly. Used to decide whether the export reports a loss.
pub(crate) fn is_lossy_for(width: u32, height: u32, rgba: &[u8]) -> bool {
    let w = width as usize;
    let h = height as usize;
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return true;
    }
    // A single colour everywhere is the one common case that is exact: both endpoints are that colour.
    let first = &rgba[0..4];
    !rgba.chunks_exact(4).all(|pixel| pixel == first)
}

fn write_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

/// Packs one RGB triple into the 5:6:5 form a colour endpoint uses.
const fn to_rgb565(r: u8, g: u8, b: u8) -> u16 {
    ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3)
}

/// Unpacks a 5:6:5 endpoint, replicating the high bits into the low ones.
///
/// The replication is not an approximation to tidy up later: it is how a decoder expands these, so an
/// encoder that compares against plain zero-filled bits picks endpoints for colours no decoder will
/// produce.
const fn from_rgb565(value: u16) -> [u8; 3] {
    let r = ((value >> 11) & 31) as u8;
    let g = ((value >> 5) & 63) as u8;
    let b = (value & 31) as u8;
    [
        (r << 3) | (r >> 2),
        (g << 2) | (g >> 4),
        (b << 3) | (b >> 2),
    ]
}

/// Encodes the 8-byte colour half of a block: two endpoints plus two bits per pixel.
///
/// The endpoints come from the block's bounding box in RGB, which is the cheap choice GIMP's encoder
/// also starts from.
fn encode_color_block(block: &[[u8; 4]; 16]) -> [u8; 8] {
    let mut low = [255u8; 3];
    let mut high = [0u8; 3];
    for pixel in block {
        for channel in 0..3 {
            low[channel] = low[channel].min(pixel[channel]);
            high[channel] = high[channel].max(pixel[channel]);
        }
    }
    let mut c0 = to_rgb565(high[0], high[1], high[2]);
    let mut c1 = to_rgb565(low[0], low[1], low[2]);
    // The four-colour interpolation is only selected when c0 > c1. When the block is flat the two
    // endpoints quantise to the same value and the order cannot be fixed by swapping -- that block is
    // written in the three-colour form, which still reproduces the colour exactly at index 0.
    if c0 < c1 {
        std::mem::swap(&mut c0, &mut c1);
    }
    let e0 = from_rgb565(c0);
    let e1 = from_rgb565(c1);
    // The palette a DECODER will build, which is what the indices must be chosen against.
    let palette = if c0 > c1 {
        [
            e0,
            e1,
            blend(e0, e1, 2, 1),
            blend(e0, e1, 1, 2),
        ]
    } else {
        [e0, e1, blend(e0, e1, 1, 1), [0, 0, 0]]
    };
    let usable = if c0 > c1 { 4 } else { 3 };

    let mut indices = 0u32;
    for (i, pixel) in block.iter().enumerate() {
        let mut best = 0usize;
        let mut best_error = u32::MAX;
        for (index, candidate) in palette.iter().enumerate().take(usable) {
            let error = (0..3)
                .map(|c| {
                    let d = i32::from(pixel[c]) - i32::from(candidate[c]);
                    (d * d) as u32
                })
                .sum();
            if error < best_error {
                best_error = error;
                best = index;
            }
        }
        indices |= (best as u32) << (i * 2);
    }

    let mut out = [0u8; 8];
    out[0..2].copy_from_slice(&c0.to_le_bytes());
    out[2..4].copy_from_slice(&c1.to_le_bytes());
    out[4..8].copy_from_slice(&indices.to_le_bytes());
    out
}

/// Weighted blend of two endpoints, matching the 2:1 and 1:2 mixes a decoder computes.
fn blend(a: [u8; 3], b: [u8; 3], wa: u32, wb: u32) -> [u8; 3] {
    let mut out = [0u8; 3];
    for channel in 0..3 {
        let value = (u32::from(a[channel]) * wa + u32::from(b[channel]) * wb + (wa + wb) / 2)
            / (wa + wb);
        out[channel] = value.min(255) as u8;
    }
    out
}

/// Encodes the 8-byte alpha half of a BC3 block: two 8-bit endpoints plus three bits per pixel.
///
/// With `a0 > a1` the six interpolated steps are used; equal endpoints fall into the other form, where
/// two of the eight slots are hard 0 and 255. A flat alpha block therefore reproduces exactly, which is
/// what keeps a fully opaque region of an otherwise transparent image from picking up a gradient.
fn encode_alpha_block(block: &[[u8; 4]; 16]) -> [u8; 8] {
    let mut min = 255u8;
    let mut max = 0u8;
    for pixel in block {
        min = min.min(pixel[3]);
        max = max.max(pixel[3]);
    }
    let (a0, a1) = (max, min);
    let palette: [u8; 8] = if a0 > a1 {
        let step = |numerator: u32, denominator: u32| {
            ((u32::from(a0) * numerator + u32::from(a1) * (denominator - numerator)
                + denominator / 2)
                / denominator) as u8
        };
        [
            a0,
            a1,
            step(6, 7),
            step(5, 7),
            step(4, 7),
            step(3, 7),
            step(2, 7),
            step(1, 7),
        ]
    } else {
        [a0, a1, a0, a0, a0, a0, 0, 255]
    };
    let usable = if a0 > a1 { 8 } else { 6 };

    let mut bits: u64 = 0;
    for (i, pixel) in block.iter().enumerate() {
        let mut best = 0usize;
        let mut best_error = u32::MAX;
        for (index, candidate) in palette.iter().enumerate().take(usable) {
            let difference = i32::from(pixel[3]) - i32::from(*candidate);
            let error = (difference * difference) as u32;
            if error < best_error {
                best_error = error;
                best = index;
            }
        }
        bits |= (best as u64) << (i * 3);
    }

    let mut out = [0u8; 8];
    out[0] = a0;
    out[1] = a1;
    // Three bits per pixel across sixteen pixels is 48 bits, written little-endian after the endpoints.
    for byte in 0..6 {
        out[2 + byte] = ((bits >> (byte * 8)) & 0xff) as u8;
    }
    out
}
