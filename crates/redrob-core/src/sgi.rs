// SPDX-License-Identifier: GPL-3.0-or-later

//! Silicon Graphics image (M.7, first of four).
//!
//! Re-derived from `plug-ins/file-sgi/sgi-lib.c` and `plug-ins/file-sgi/sgi.c`
//! (GPL-3.0-or-later), pinned in `docs/upstream-sources.toml`. No pure-Rust SGI codec exists on
//! the registry — searched, and unlike M.4's ICNS or M.5's JPEG 2000 there is nothing to adopt —
//! so the codec is written here, which group M's policy allows once the search has been done.
//!
//! The 512-byte header, as `sgiOpen` reads it:
//!
//! | offset | field        | type    | notes                                              |
//! |--------|--------------|---------|----------------------------------------------------|
//! | 0      | `magic`      | u16 BE  | 474 decimal, `0x01DA` — declared in decimal        |
//! | 2      | `comp`       | u8      | 0 none, 1 RLE, 2 aggressive RLE                    |
//! | 3      | `bpp`        | u8      | **BYTES** per channel, 1 or 2; outside → reject    |
//! | 4      | `dimensions` | u16 BE  | read and discarded by upstream                     |
//! | 6      | `xsize`      | u16 BE  | width                                              |
//! | 8      | `ysize`      | u16 BE  | height                                             |
//! | 10     | `zsize`      | u16 BE  | channels; **> 4 is CLAMPED to 4, not rejected**    |
//! | 12     | `pixmin`     | i32 BE  | read and discarded                                 |
//! | 16     | `pixmax`     | i32 BE  | read and discarded                                 |
//! | 512    | data         |         | RLE offset/length tables live here when compressed |
//!
//! Three things about this format bite, and each has a test:
//!
//! 1. **The magic is accepted in EITHER byte order.** `sgiOpen` reads it big-endian and, on a
//!    mismatch, swaps the two bytes and tries again, setting `swapBytes` for the rest of the file.
//!    Upstream's *registered* magic is only `0,short,474`, so **its own content detection cannot
//!    find a little-endian SGI that its loader reads perfectly well.** That gap is not inherited.
//! 2. **`bpp` counts BYTES, not bits.** 1 or 2, and upstream rejects anything else outright.
//! 3. **The RLE control bit is INVERTED relative to TGA's**, which this product implemented four
//!    items ago. Here bit 7 SET means a literal run and CLEAR means a repeat; in TGA it is the
//!    other way round. Getting it backwards produces a picture, just the wrong one.
//!
//! Channels are stored as SEPARATE PLANES, not interleaved, and rows run BOTTOM-UP: upstream asks
//! for stored row `ysize - 1 - y` to fill output row `y`.

use crate::{FormatError, Result};

/// `SGI_MAGIC`, written in upstream's own decimal so the two can be compared by eye.
const SGI_MAGIC: u16 = 474;
/// Pixel data begins here; the RLE tables occupy the start of it when the file is compressed.
const HEADER_LEN: usize = 512;

/// A decoded SGI: dimensions plus 8-bit RGBA.
pub(crate) struct Sgi {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Is this an SGI, in either byte order?
///
/// The magic alone is two bytes, which is thin, so `comp` and `bpp` are checked as upstream's
/// loader checks them. `bpp` is the stronger of the two: upstream refuses anything outside 1..=2.
pub(crate) fn looks_like_sgi(bytes: &[u8]) -> bool {
    if bytes.len() < HEADER_LEN {
        return false;
    }
    let big = u16::from_be_bytes([bytes[0], bytes[1]]);
    let little = u16::from_le_bytes([bytes[0], bytes[1]]);
    if big != SGI_MAGIC && little != SGI_MAGIC {
        return false;
    }
    matches!(bytes[2], 0..=2) && matches!(bytes[3], 1..=2)
}

fn be16(bytes: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([bytes[at], bytes[at + 1]])
}

/// Decode an SGI to RGBA.
///
/// Only `bpp == 1` is decoded. A 16-bit SGI needs the deep path this product gained in J.1 and is
/// filed rather than silently narrowed to 8 bits, because quietly halving someone's precision is
/// worse than refusing.
pub(crate) fn decode(bytes: &[u8]) -> Result<Sgi> {
    if !looks_like_sgi(bytes) {
        return Err(FormatError::UnsupportedFeature("not an SGI image").into());
    }
    let comp = bytes[2];
    let bpp = bytes[3];
    if bpp != 1 {
        return Err(FormatError::UnsupportedFeature("16-bit SGI is not decoded yet").into());
    }
    if comp == 2 {
        return Err(
            FormatError::UnsupportedFeature("aggressive-RLE SGI is not decoded yet").into(),
        );
    }

    let width = be16(bytes, 6) as usize;
    let height = be16(bytes, 8) as usize;
    // Upstream CLAMPS a channel count above 4 rather than refusing the file, after warning. The
    // same leniency is kept: a file that overstates its channels still has four readable ones.
    let channels = (be16(bytes, 10) as usize).min(4);
    if width == 0 || height == 0 || channels == 0 {
        return Err(FormatError::UnsupportedFeature("SGI declares an empty image").into());
    }
    crate::document::pixel_count(width as u32, height as u32)?;

    // One plane per channel, each `height` rows of `width` samples.
    let mut planes = vec![vec![0u8; width * height]; channels];

    if comp == 0 {
        let needed = width * height * channels;
        let body = bytes
            .get(HEADER_LEN..HEADER_LEN + needed)
            .ok_or(FormatError::UnsupportedFeature("SGI pixel data is short"))?;
        // Verbatim: whole planes back to back, each plane's rows bottom-up.
        for (z, plane) in planes.iter_mut().enumerate() {
            for y in 0..height {
                let from = (z * height + y) * width;
                plane[(height - 1 - y) * width..][..width]
                    .copy_from_slice(&body[from..from + width]);
            }
        }
        // (plane index `z` is needed for the file offset, so the enumerate stays)
    } else {
        // RLE: two tables of `height * channels` big-endian u32s at offset 512 — the file offset of
        // each row, then its byte length. Indexed [channel][row].
        let count = height * channels;
        let tables_len = count * 8;
        if bytes.len() < HEADER_LEN + tables_len {
            return Err(FormatError::UnsupportedFeature("SGI RLE tables are short").into());
        }
        for (z, plane) in planes.iter_mut().enumerate() {
            for y in 0..height {
                let index = z * height + y;
                let at = HEADER_LEN + index * 4;
                let offset =
                    u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
                        as usize;
                let row = bytes.get(offset..).ok_or(FormatError::UnsupportedFeature(
                    "SGI row offset is past the end",
                ))?;
                let decoded = decode_rle_row(row, width)?;
                plane[(height - 1 - y) * width..][..width].copy_from_slice(&decoded);
            }
        }
    }

    // Interleave to RGBA. 1 channel is grey, 2 is grey+alpha, 3 is RGB, 4 is RGBA -- the same
    // layout ladder every other planar format here uses.
    // Scatter each plane into the interleaved buffer. Alpha starts opaque, so a layout with no
    // alpha plane needs no special case: 1 channel is grey, 2 is grey+alpha, 3 is RGB, 4 is RGBA,
    // the same ladder every other planar format here uses.
    let mut rgba = vec![255u8; width * height * 4];
    for (z, plane) in planes.iter().enumerate() {
        // Which interleaved slots this plane feeds. A single grey plane feeds all three colour
        // channels; otherwise plane `z` feeds channel `z`.
        let targets: &[usize] = match (channels, z) {
            (1, _) | (2, 0) => &[0, 1, 2],
            (2, _) => &[3],
            _ => match z {
                0 => &[0],
                1 => &[1],
                2 => &[2],
                _ => &[3],
            },
        };
        for &target in targets {
            for (slot, sample) in rgba
                .iter_mut()
                .skip(target)
                .step_by(4)
                .zip(plane.iter().copied())
            {
                *slot = sample;
            }
        }
    }

    Ok(Sgi {
        width: width as u32,
        height: height as u32,
        rgba,
    })
}

/// One RLE row.
///
/// **The control bit is inverted relative to TGA.** `count = ch & 127`; bit 7 SET means `count`
/// literal bytes follow, CLEAR means ONE byte follows and is repeated `count` times. A count of
/// zero ends the row, and upstream clamps each count to the width still outstanding, so a
/// malformed run cannot write past the row.
fn decode_rle_row(source: &[u8], width: usize) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(width);
    let mut at = 0usize;
    while out.len() < width {
        let control = *source
            .get(at)
            .ok_or(FormatError::UnsupportedFeature("SGI RLE row ended early"))?;
        at += 1;
        let count = usize::from(control & 127).min(width - out.len());
        if count == 0 {
            break;
        }
        if control & 128 != 0 {
            let run = source
                .get(at..at + count)
                .ok_or(FormatError::UnsupportedFeature("SGI literal run is short"))?;
            out.extend_from_slice(run);
            at += count;
        } else {
            let value = *source
                .get(at)
                .ok_or(FormatError::UnsupportedFeature("SGI repeat run is short"))?;
            at += 1;
            out.extend(std::iter::repeat_n(value, count));
        }
    }
    if out.len() != width {
        return Err(FormatError::UnsupportedFeature("SGI row is the wrong width").into());
    }
    Ok(out)
}

/// Encode an uncompressed 4-channel SGI.
///
/// `comp = 0` deliberately: upstream can write RLE, but an uncompressed writer is the one whose
/// output is checkable by eye against the header table above, and the RLE *reader* is exercised by
/// its own fixture rather than by our own encoder agreeing with itself.
pub(crate) fn encode(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    let (w, h) = (width as usize, height as usize);
    let mut bytes = vec![0u8; HEADER_LEN];
    bytes[0..2].copy_from_slice(&SGI_MAGIC.to_be_bytes());
    bytes[2] = 0; // no compression
    bytes[3] = 1; // one byte per channel
    bytes[4..6].copy_from_slice(&3u16.to_be_bytes()); // dimensions: width, height, channels
    bytes[6..8].copy_from_slice(&(width as u16).to_be_bytes());
    bytes[8..10].copy_from_slice(&(height as u16).to_be_bytes());
    bytes[10..12].copy_from_slice(&4u16.to_be_bytes()); // RGBA
    bytes[12..16].copy_from_slice(&0i32.to_be_bytes()); // pixmin
    bytes[16..20].copy_from_slice(&255i32.to_be_bytes()); // pixmax

    // Planar and bottom-up, matching what the reader above expects of a file.
    for z in 0..4 {
        for y in 0..h {
            let source_row = h - 1 - y;
            for x in 0..w {
                bytes.push(rgba[(source_row * w + x) * 4 + z]);
            }
        }
    }
    Ok(bytes)
}
