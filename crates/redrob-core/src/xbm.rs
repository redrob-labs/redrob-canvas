// SPDX-License-Identifier: GPL-3.0-or-later

//! X BitMap (M.7d, last of M.7's four formats).
//!
//! Re-derived from `plug-ins/common/file-xbm.c` (GPL-3.0-or-later), pinned in
//! `docs/upstream-sources.toml`.
//!
//! `xbm 0.3.0` on the registry is a real codec and was the one crate M.7's search found for any of
//! its four formats. It is deliberately not used: the three facts below are precisely the ones an
//! independent implementation could differ on, so adopting a crate would mean verifying all three
//! against it anyway — and a new dependency costs a notices regeneration, which this group has
//! paid three times already. Written here, each fact is a line of code with a test on it.
//!
//! An XBM is C source:
//!
//! ```text
//! #define sample_width 8
//! #define sample_height 2
//! static unsigned char sample_bits[] = { 0x03, 0xc0 };
//! ```
//!
//! **1. UPSTREAM REGISTERS NO MAGIC, and cannot: the `#define` prefix is the image's own name**, so
//! there is no fixed byte at any fixed offset. It reaches the loader by file extension instead.
//! That is the third format in group M with no registered signature, after TGA 1.0 and ICO.
//!
//! This product detects it anyway, from `#define` followed by a name ending `_width` and an
//! integer — which is what upstream's own loader keys on (`match (fp, "width")` after an
//! underscore). **This is a divergence and it is a different call from the one M.2 refused.** TGA
//! 1.0 was left undetectable because the only fixes were to weaken `validate_detected_format` for
//! all formats, or to guess from plausible-looking binary fields. Neither applies here: this is a
//! positive TEXT signature with no collision surface against a binary format, and XPM — in this
//! same group — already establishes that a textual signature is evidence, its magic being literally
//! the comment `/* XPM */`.
//!
//! **2. A SET BIT IS BLACK.** Upstream flips every word on read — `c ^= 0xffff;` — under its own
//! comment: *"Flip all the bits so that 1's become black and 0's become white."* Its colormap is
//! `{0x00,0x00,0x00 /* black */, 0xff,0xff,0xff /* white */}`, index 0 black.
//!
//! **3. BITS RUN LEAST-SIGNIFICANT FIRST.** `data[...] = c & 1; c >>= 1;` — so bit 0 of a byte is
//! the LEFTMOST pixel. **SUN raster's 1-bit path and PBM are both most-significant-first**, and
//! SUN is M.7b, one part of this same item. Getting it backwards mirrors every group of 8 pixels
//! without failing.
//!
//! The word size comes from the C type: `char` gives 8 bits per word, `short` gives 16, and
//! upstream errors if it saw neither. The optional `_x_hot` / `_y_hot` defines carry a cursor
//! hotspot, which upstream stores as a parasite; this product has nowhere to put it and says so.

use crate::{FormatError, Result};

pub(crate) struct Xbm {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// Present when the file declared `_x_hot` / `_y_hot`. Reported rather than dropped silently,
    /// because upstream keeps it and this product cannot.
    pub hotspot: Option<(u32, u32)>,
}

/// Does this look like an XBM?
///
/// Requires `#define`, a name ending in `_width`, and an integer after it. All three, because
/// `#define` alone is any C file and `_width` alone is any C file that happens to use the word.
pub(crate) fn looks_like_xbm(bytes: &[u8]) -> bool {
    // An XBM is small and its defines come first; a megabyte of preamble is not one.
    let head = &bytes[..bytes.len().min(4096)];
    let Ok(text) = std::str::from_utf8(head) else {
        return false;
    };
    find_define(text, "_width").is_some()
}

/// Find `#define <anything><suffix> <integer>` and return the integer.
fn find_define(text: &str, suffix: &str) -> Option<u32> {
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("#define") else {
            continue;
        };
        let mut parts = rest.split_whitespace();
        let name = parts.next()?;
        if !name.ends_with(suffix) {
            continue;
        }
        if let Some(value) = parts.next()
            && let Ok(parsed) = value.parse::<u32>()
        {
            return Some(parsed);
        }
    }
    None
}

pub(crate) fn decode(bytes: &[u8]) -> Result<Xbm> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| FormatError::UnsupportedFeature("XBM is not valid UTF-8"))?;

    let width = find_define(text, "_width")
        .ok_or(FormatError::UnsupportedFeature("XBM declares no width"))?;
    let height = find_define(text, "_height")
        .ok_or(FormatError::UnsupportedFeature("XBM declares no height"))?;
    if width == 0 || height == 0 {
        return Err(FormatError::UnsupportedFeature("XBM declares an empty image").into());
    }
    crate::document::pixel_count(width, height)?;

    let hotspot = match (find_define(text, "_x_hot"), find_define(text, "_y_hot")) {
        // Upstream clamps both into the image before using them.
        (Some(x), Some(y)) => Some((x.min(width), y.min(height))),
        _ => None,
    };

    // Fact: the word size is the C type. Upstream sets 8 for `char`, 16 for `short`, and refuses
    // the file if it saw neither -- so the declaration is load-bearing, not decoration.
    let body_start = text
        .find("_bits")
        .and_then(|at| text[at..].find('{').map(|brace| at + brace))
        .ok_or(FormatError::UnsupportedFeature("XBM has no bit array"))?;
    let declaration = &text[..body_start];
    let intbits = if declaration.contains("short") {
        16usize
    } else if declaration.contains("char") {
        8usize
    } else {
        return Err(FormatError::UnsupportedFeature("XBM declares neither char nor short").into());
    };

    let body = &text[body_start + 1..];
    let end = body.find('}').ok_or(FormatError::UnsupportedFeature(
        "XBM bit array is unterminated",
    ))?;
    let mut words = Vec::new();
    for token in body[..end].split(',') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        let digits = token
            .strip_prefix("0x")
            .or_else(|| token.strip_prefix("0X"));
        let value = match digits {
            Some(hex) => u32::from_str_radix(hex, 16),
            None => token.parse::<u32>(),
        }
        .map_err(|_| FormatError::UnsupportedFeature("XBM bit array holds a non-number"))?;
        words.push(value);
    }

    // Each ROW starts on a word boundary: upstream's `k % intbits == 0` resets within the row, so
    // the padding bits of a short row are never carried into the next one.
    let words_per_row = (width as usize).div_ceil(intbits);
    if words.len() < words_per_row * height as usize {
        return Err(FormatError::UnsupportedFeature("XBM bit array is short").into());
    }

    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height as usize {
        let row = &words[y * words_per_row..][..words_per_row];
        for x in 0..width as usize {
            let word = row[x / intbits];
            // Fact 2: upstream flips every word -- `c ^= 0xffff` -- so a SET bit is BLACK.
            // Fact 3: and reads bit 0 first, so bit 0 is the LEFTMOST pixel.
            let bit = (word >> (x % intbits)) & 1;
            let value = if bit == 1 { 0u8 } else { 255u8 };
            rgba.extend_from_slice(&[value, value, value, 255]);
        }
    }

    Ok(Xbm {
        width,
        height,
        rgba,
        hotspot,
    })
}

/// Encode an 8-bit-word XBM.
///
/// `char` rather than `short`: upstream can write either, and the 8-bit form is the one whose bytes
/// line up with the pixels for a reader checking by eye. Any pixel darker than mid-grey becomes a
/// set bit, which is the threshold the one-bit-per-pixel target forces.
pub(crate) fn encode(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    let (w, h) = (width as usize, height as usize);
    let words_per_row = w.div_ceil(8);

    let mut out = String::new();
    out.push_str(&format!("#define image_width {width}\n"));
    out.push_str(&format!("#define image_height {height}\n"));
    out.push_str("static unsigned char image_bits[] = {\n");

    let mut tokens = Vec::with_capacity(words_per_row * h);
    for y in 0..h {
        for word in 0..words_per_row {
            let mut value = 0u8;
            for bit in 0..8usize {
                let x = word * 8 + bit;
                if x >= w {
                    break;
                }
                let at = (y * w + x) * 4;
                // Luma is not needed for a threshold this coarse; the mean of the three channels
                // is what upstream's indexed conversion effectively lands on for black and white.
                let level =
                    (u32::from(rgba[at]) + u32::from(rgba[at + 1]) + u32::from(rgba[at + 2])) / 3;
                if level < 128 {
                    // Fact 2 and 3 together: dark means a SET bit, and bit 0 is leftmost.
                    value |= 1 << bit;
                }
            }
            tokens.push(format!("0x{value:02x}"));
        }
    }

    for (index, token) in tokens.iter().enumerate() {
        if index % 12 == 0 {
            out.push_str("  ");
        }
        out.push_str(token);
        if index + 1 < tokens.len() {
            out.push(',');
            out.push(if (index + 1) % 12 == 0 { '\n' } else { ' ' });
        }
    }
    out.push_str(" };\n");
    Ok(out.into_bytes())
}
