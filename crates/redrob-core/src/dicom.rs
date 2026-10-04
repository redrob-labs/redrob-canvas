// SPDX-License-Identifier: GPL-3.0-or-later

//! DICOM (M.11a, first of M.11's two formats).
//!
//! Re-derived from `plug-ins/common/file-dicom.c` (GPL-3.0-or-later), pinned in
//! `docs/upstream-sources.toml`.
//!
//! # The signature is at offset 128
//!
//! Upstream registers `128,string,DICM`: a 128-byte preamble, then the four bytes. **That is the
//! third format in group M whose signature is not at the start** — M.2's TGA keeps its at the very
//! END, M.10's `.pat` at offset 20, this at 128. A sniff that assumes offset 0 finds nothing in any
//! of the three.
//!
//! # The data elements
//!
//! After the magic comes a stream of elements, each `group: u16`, `element: u16`, a two-character
//! **Value Representation**, and a length. Everything is LITTLE-ENDIAN by default — upstream
//! expresses that as `g_ntohs (GUINT16_SWAP_LE_BE (x))`, a double swap that is the identity on a
//! little-endian host.
//!
//! **The VR is recognised HEURISTICALLY, and upstream says so**: *"Check if the value rep looks
//! valid. There probably is a better way of checking this..."* Two uppercase letters means explicit
//! VR; anything else means the bytes were really the first half of a 4-byte length, so the encoding
//! is implicit. That heuristic is re-derived here rather than improved on, because a file that
//! upstream reads and this does not is a parity bug whichever of us is more principled.
//!
//! **The binary VRs `OB`, `OW`, `SQ` and `UN` carry a 2-byte RESERVED field before a 4-byte
//! length**; every other explicit VR has a 2-byte length. Pixel data is `OW` or `OB`, so getting
//! this wrong misreads the length of the one element that matters.
//!
//! # The tags read, and the two that bite
//!
//! | tag           | meaning                                                      |
//! |---------------|--------------------------------------------------------------|
//! | `0028,0002`   | samples per pixel                                            |
//! | `0028,0004`   | photometric interpretation                                   |
//! | `0028,0010`   | **ROWS — this is the HEIGHT**                                |
//! | `0028,0011`   | **COLUMNS — this is the WIDTH**                              |
//! | `0028,0100`   | bits allocated                                               |
//! | `0028,0101`   | bits stored                                                  |
//! | `0028,0102`   | high bit                                                     |
//! | `0028,0103`   | pixel representation — 1 means signed                        |
//! | `7FE0,0010`   | pixel data                                                   |
//!
//! **`0010` is ROWS and `0011` is COLUMNS**, so the LOWER-numbered element is the HEIGHT. The
//! natural assumption is the other way round, and getting it backwards transposes the image without
//! failing.
//!
//! **`MONOCHROME1` means the image is INVERTED** — its minimum sample is meant to display as
//! *white*. `MONOCHROME2` is the ordinary direction. A photometric flag that silently flips every
//! grey is exactly the kind that needs a test.
//!
//! # The 16-bit chain is TWO composed shifts
//!
//! Upstream shifts twice, and the second one is by `bits_stored`, not by `bits_allocated`:
//!
//! ```text
//! buf16[i] = read_le(buf16[i]) >> (high_bit + 1 - bits_stored);   /* align the LSB */
//! d        = buf16[i] >> (bits_stored - 8);                       /* narrow to 8 */
//! ```
//!
//! So a 16-bit-allocated, 12-bit-stored image narrows by **4**, not 8. Using `bits_allocated` there
//! gives an image 16 times too dark.
//!
//! Then, in order: invert if `MONOCHROME1`, and if the samples are signed fold them —
//! `if (d > 127) d = 0; else d <<= 1;` — so negatives become black and positives span 0..254.

use crate::{FormatError, Result};

/// The 128-byte preamble that precedes the magic.
const PREAMBLE: usize = 128;

pub(crate) struct Dicom {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Is this a DICOM? The magic is at offset 128, not 0.
pub(crate) fn looks_like_dicom(bytes: &[u8]) -> bool {
    bytes.len() >= PREAMBLE + 4 && &bytes[PREAMBLE..PREAMBLE + 4] == b"DICM"
}

fn le16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]))
}

fn le32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes([
        *bytes.get(at)?,
        *bytes.get(at + 1)?,
        *bytes.get(at + 2)?,
        *bytes.get(at + 3)?,
    ]))
}

/// What the header told us, before any of it is trusted.
#[derive(Default)]
struct Header {
    samples_per_pixel: u16,
    rows: u16,
    columns: u16,
    bits_allocated: u16,
    bits_stored: u16,
    high_bit: u16,
    is_signed: bool,
    inverted: bool,
    planar: bool,
    pixels_at: Option<(usize, usize)>,
}

pub(crate) fn decode(bytes: &[u8]) -> Result<Dicom> {
    if !looks_like_dicom(bytes) {
        return Err(FormatError::UnsupportedFeature("not a DICOM image").into());
    }
    let mut header = Header::default();
    let mut at = PREAMBLE + 4;

    while at + 8 <= bytes.len() {
        let group = le16(bytes, at).ok_or(FormatError::UnsupportedFeature("DICOM truncated"))?;
        let element =
            le16(bytes, at + 2).ok_or(FormatError::UnsupportedFeature("DICOM truncated"))?;
        let vr = &bytes[at + 4..at + 6];
        at += 6;

        // The heuristic, as upstream performs it: two uppercase letters is an explicit VR.
        let explicit = vr[0].is_ascii_uppercase() && vr[1].is_ascii_uppercase();
        let length = if !explicit {
            // Those two bytes were the low half of a 4-byte length.
            let value = u32::from_le_bytes([
                vr[0],
                vr[1],
                *bytes.get(at).unwrap_or(&0),
                *bytes.get(at + 1).unwrap_or(&0),
            ]) as usize;
            at += 2;
            value
        } else if matches!(vr, b"OB" | b"OW" | b"SQ" | b"UN") {
            // A 2-byte RESERVED field, then a 4-byte length.
            let value = le32(bytes, at + 2)
                .ok_or(FormatError::UnsupportedFeature("DICOM truncated"))?
                as usize;
            at += 6;
            value
        } else {
            let value =
                le16(bytes, at).ok_or(FormatError::UnsupportedFeature("DICOM truncated"))? as usize;
            at += 2;
            value
        };

        let value_at = at;
        let short = || le16(bytes, value_at).unwrap_or(0);

        if group == 0x0028 {
            match element {
                0x0002 => header.samples_per_pixel = short(),
                0x0004 => {
                    let text = bytes.get(value_at..value_at + length).unwrap_or_default();
                    // MONOCHROME1's minimum sample displays as WHITE, so the image is inverted.
                    // Checked with `starts_with` because the value is space-padded to an even
                    // length, exactly as upstream's `strncmp` ignores the tail.
                    header.inverted = text.starts_with(b"MONOCHROME1");
                }
                0x0006 => header.planar = short() == 1,
                0x0010 => header.rows = short(),
                0x0011 => header.columns = short(),
                0x0100 => header.bits_allocated = short(),
                0x0101 => header.bits_stored = short(),
                0x0102 => header.high_bit = short(),
                0x0103 => header.is_signed = short() != 0,
                _ => {}
            }
        } else if group == 0x7fe0 && element == 0x0010 {
            header.pixels_at = Some((value_at, length));
        }

        // An undefined length (0xFFFFFFFF) would run away; stop rather than guess at a sequence.
        if length == 0xffff_ffff {
            break;
        }
        at = at.saturating_add(length);
    }

    let width = header.columns as u32;
    let height = header.rows as u32;
    if width == 0 || height == 0 {
        return Err(FormatError::UnsupportedFeature("DICOM declares no dimensions").into());
    }
    crate::document::pixel_count(width, height)?;

    // Upstream's own ceiling: anything other than 8 or 16 is refused by name.
    if !matches!(header.bits_allocated, 8 | 16) {
        return Err(
            FormatError::UnsupportedFeature("DICOM bits-allocated is neither 8 nor 16").into(),
        );
    }
    if header.planar {
        return Err(FormatError::UnsupportedFeature("planar DICOM is not decoded yet").into());
    }
    let samples = match header.samples_per_pixel {
        0 | 1 => 1usize,
        3 => 3usize,
        other => {
            let _ = other;
            return Err(FormatError::UnsupportedFeature(
                "DICOM samples-per-pixel is neither 1 nor 3",
            )
            .into());
        }
    };

    let (pixels_at, pixels_len) = header
        .pixels_at
        .ok_or(FormatError::UnsupportedFeature("DICOM has no pixel data"))?;
    let body =
        bytes
            .get(pixels_at..pixels_at + pixels_len)
            .ok_or(FormatError::UnsupportedFeature(
                "DICOM pixel data is truncated",
            ))?;

    let count = (width as usize) * (height as usize) * samples;
    let mut narrowed = Vec::with_capacity(count);

    if header.bits_allocated == 8 {
        if body.len() < count {
            return Err(FormatError::UnsupportedFeature("DICOM pixel data is short").into());
        }
        narrowed.extend_from_slice(&body[..count]);
    } else {
        if body.len() < count * 2 {
            return Err(FormatError::UnsupportedFeature("DICOM pixel data is short").into());
        }
        // `bits_stored` can exceed nothing sensible; guard the two shifts rather than panic.
        let stored = header.bits_stored.max(8);
        let align = (header.high_bit + 1).saturating_sub(stored);
        let narrow = stored - 8;
        for sample in body[..count * 2].chunks_exact(2) {
            let value = u16::from_le_bytes([sample[0], sample[1]]) >> align;
            narrowed.push((value >> narrow) as u8);
        }
    }

    // Invert, then fold the signed range -- upstream's order, and it matters: inverting after the
    // fold would turn black into white rather than the other way about.
    for value in &mut narrowed {
        if header.inverted {
            *value = !*value;
        }
        if header.is_signed {
            *value = if *value > 127 { 0 } else { *value << 1 };
        }
    }

    let mut rgba = Vec::with_capacity((width as usize) * (height as usize) * 4);
    for pixel in narrowed.chunks_exact(samples) {
        let [r, g, b] = if samples == 1 {
            [pixel[0], pixel[0], pixel[0]]
        } else {
            [pixel[0], pixel[1], pixel[2]]
        };
        rgba.extend_from_slice(&[r, g, b, 255]);
    }

    Ok(Dicom {
        width,
        height,
        rgba,
    })
}
