// SPDX-License-Identifier: GPL-3.0-or-later

//! FITS (M.11b, second of M.11's two formats).
//!
//! Re-derived from `plug-ins/file-fits/fits.c` (GPL-3.0-or-later), pinned in
//! `docs/upstream-sources.toml`.
//!
//! # A provenance split worth being explicit about
//!
//! **Upstream delegates the parsing to CFITSIO** — it calls `fits_get_img_param` and never touches
//! a card itself. So this module has two kinds of fact in it and they are not equally sourced:
//!
//! * **Re-derived from upstream**: the registered magic, the `naxis < 2` refusal, the `BITPIX`
//!   table, the axis-shift rule, and which `naxisn` entries become width and height.
//! * **From the FITS specification, NOT from upstream's code**: the 80-column card layout, the
//!   `END` keyword, the 2880-byte block padding, and that the data is big-endian. Upstream's source
//!   cannot show these because CFITSIO does that work, so they are not re-derivations and are
//!   labelled rather than presented as though they were.
//!
//! No pure-Rust FITS codec was adopted: `fits 0.0.0` is an empty placeholder, `fits-header` reads
//! only headers, and `seiza-fits` is an astrophotography pipeline. The format's header is ASCII and
//! its data is raw, so it is written here — the same call M.7's four and M.11a made, and it keeps
//! the fourth consecutive cycle free of a notices regeneration.
//!
//! # `BITPIX` IS SIGNED, AND THE SIGN IS THE TYPE TAG
//!
//! | `BITPIX` | sample            | upstream's precision        |
//! |----------|-------------------|-----------------------------|
//! | `8`      | unsigned byte     | `U8_LINEAR`                 |
//! | `16`     | signed short      | `U16_NON_LINEAR`            |
//! | `32`     | signed int        | `U32_LINEAR`                |
//! | `-32`    | **IEEE float**    | `FLOAT_LINEAR`              |
//! | `-64`    | **IEEE double**   | `DOUBLE_LINEAR`             |
//!
//! A negative value means floating point. Reading `BITPIX` as unsigned loses that distinction
//! entirely, which is why it is parsed as `i32` and tested at `-32`.
//!
//! **An upstream inconsistency, recorded rather than copied**: 8 and 32 map to *LINEAR* precisions
//! while 16 maps to *NON_LINEAR*. Nothing in the file explains why 16 is the odd one out. It does
//! not affect this product, whose `Precision` has no transfer-curve axis at all — the gap M.6 filed
//! — but a later cycle that gains one should not inherit the inconsistency by accident.
//!
//! # The axis shift: RGB can be stored either way round
//!
//! `naxis == 2` is greyscale. With a third axis it is RGB — and **if the FIRST axis is 3, the
//! dimensions shift**:
//!
//! ```text
//! else if (hdu.naxisn[2]) {
//!     if (hdu.naxisn[0] == 3) { width = hdu.naxisn[1]; height = hdu.naxisn[2]; }
//!     channels = 3;
//! ```
//!
//! So `(w, h, 3)` and `(3, w, h)` are both RGB, and in the second the width and height are the
//! SECOND and THIRD axes. Missing that reads a 3-wide image of the wrong shape.

use crate::{FormatError, Result};

/// A FITS header card is exactly 80 bytes of ASCII. **Specification, not upstream's code.**
const CARD: usize = 80;

/// The header is padded to a multiple of this, and the data begins at the next boundary.
/// **Specification, not upstream's code.**
const BLOCK: usize = 2880;

pub(crate) struct Fits {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Upstream registers `0,string,SIMPLE`.
pub(crate) fn looks_like_fits(bytes: &[u8]) -> bool {
    bytes.starts_with(b"SIMPLE")
}

/// Read one keyword's value from the 80-column cards.
///
/// A card is `KEYWORD = value / comment`, space padded. The keyword occupies the first 8 columns,
/// so the search is anchored there rather than scanning anywhere in the card — otherwise `NAXIS`
/// would also match inside `NAXIS1`.
fn card_value(bytes: &[u8], keyword: &str) -> Option<i64> {
    let mut at = 0usize;
    while at + CARD <= bytes.len() {
        let card = &bytes[at..at + CARD];
        if card.starts_with(b"END") {
            return None;
        }
        let name = std::str::from_utf8(&card[..8]).ok()?.trim_end();
        if name == keyword {
            let rest = std::str::from_utf8(&card[8..]).ok()?;
            let value = rest.trim_start().strip_prefix('=')?;
            // Stop at the comment separator, then take the bare number.
            let value = value.split('/').next()?.trim();
            return value.parse::<i64>().ok();
        }
        at += CARD;
    }
    None
}

/// Where the data begins: past the `END` card, rounded up to the next 2880-byte block.
fn data_offset(bytes: &[u8]) -> Option<usize> {
    let mut at = 0usize;
    while at + CARD <= bytes.len() {
        if bytes[at..at + CARD].starts_with(b"END") {
            let consumed = at + CARD;
            return Some(consumed.div_ceil(BLOCK) * BLOCK);
        }
        at += CARD;
    }
    None
}

pub(crate) fn decode(bytes: &[u8]) -> Result<Fits> {
    if !looks_like_fits(bytes) {
        return Err(FormatError::UnsupportedFeature("not a FITS image").into());
    }
    // Parsed as SIGNED, because the sign is the type tag.
    let bitpix = card_value(bytes, "BITPIX")
        .ok_or(FormatError::UnsupportedFeature("FITS declares no BITPIX"))?;
    let naxis = card_value(bytes, "NAXIS")
        .ok_or(FormatError::UnsupportedFeature("FITS declares no NAXIS"))?;

    // Upstream skips any HDU with fewer than two axes rather than guessing at a 1-D table.
    if naxis < 2 {
        return Err(FormatError::UnsupportedFeature("FITS has fewer than two axes").into());
    }
    let axis1 = card_value(bytes, "NAXIS1").unwrap_or(0);
    let axis2 = card_value(bytes, "NAXIS2").unwrap_or(0);
    let axis3 = if naxis > 2 {
        card_value(bytes, "NAXIS3").unwrap_or(0)
    } else {
        0
    };

    // The axis shift. `(w, h, 3)` and `(3, w, h)` are both RGB, and the second moves the
    // dimensions to the second and third axes.
    let (width, height, channels) = if naxis == 2 {
        (axis1, axis2, 1usize)
    } else if axis3 != 0 {
        if axis1 == 3 {
            (axis2, axis3, 3usize)
        } else {
            (axis1, axis2, 3usize)
        }
    } else {
        (axis1, axis2, 1usize)
    };

    if width <= 0 || height <= 0 {
        return Err(FormatError::UnsupportedFeature("FITS declares an empty image").into());
    }
    let (width, height) = (width as u32, height as u32);
    crate::document::pixel_count(width, height)?;

    // Only BITPIX 8 is decoded. The others are refused BY NAME rather than scaled by guesswork:
    // 16 and 32 are SIGNED integers whose display range comes from the `BZERO`/`BSCALE` cards, and
    // -32/-64 are physical floating-point measurements with no inherent display range at all.
    // Inventing a normalisation would make every astronomical image look plausible and be wrong,
    // which is the same reasoning `crate::pdf` records for not half-building an interpreter.
    if bitpix != 8 {
        return Err(FormatError::UnsupportedFeature(
            "FITS BITPIX other than 8 needs BZERO/BSCALE scaling",
        )
        .into());
    }

    let at = data_offset(bytes).ok_or(FormatError::UnsupportedFeature("FITS has no END card"))?;
    let needed = (width as usize) * (height as usize) * channels;
    let body = bytes
        .get(at..at + needed)
        .ok_or(FormatError::UnsupportedFeature(
            "FITS pixel data is truncated",
        ))?;

    let mut rgba = Vec::with_capacity((width as usize) * (height as usize) * 4);
    if channels == 1 {
        for &sample in body {
            rgba.extend_from_slice(&[sample, sample, sample, 255]);
        }
    } else {
        // Three axes means three PLANES, not interleaved triples: the third axis is the slowest.
        let plane = (width as usize) * (height as usize);
        for index in 0..plane {
            rgba.extend_from_slice(&[
                body[index],
                body[plane + index],
                body[2 * plane + index],
                255,
            ]);
        }
    }

    Ok(Fits {
        width,
        height,
        rgba,
    })
}
