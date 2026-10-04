// SPDX-License-Identifier: GPL-3.0-or-later

//! SUN raster (M.7b, second of M.7's four formats).
//!
//! Re-derived from `plug-ins/common/file-sunras.c` (GPL-3.0-or-later), pinned in
//! `docs/upstream-sources.toml`. No pure-Rust SUN raster codec exists on the registry — searched
//! in cycle 136 alongside the other three M.7 formats — so the codec is written here.
//!
//! The header is eight big-endian `u32`s, 32 bytes in all:
//!
//! | offset | field       | notes                                                        |
//! |--------|-------------|--------------------------------------------------------------|
//! | 0      | `magic`     | `0x59A66A95`                                                 |
//! | 4      | `width`     |                                                              |
//! | 8      | `height`    |                                                              |
//! | 12     | `depth`     | **BITS** per pixel: 1, 8, 24, 32                             |
//! | 16     | `length`    | length of the image data — upstream notes it **may be 0**    |
//! | 20     | `type`      | 1 std, 2 RLE; upstream rejects only `> 5`                     |
//! | 24     | `maptype`   | 1 means an RGB colormap follows                              |
//! | 28     | `maplength` | colormap bytes; upstream rejects `> 256 * 3`                 |
//!
//! Five things about this format bite, and each has a test.
//!
//! 1. **`depth` counts BITS.** M.7's previous format, SGI, spells the same concept `bpp` and counts
//!    **BYTES**. Two formats in one backlog item, one field name apart, different units.
//! 2. **`type` is overloaded: it declares the channel ORDER as well as the compression.** Upstream
//!    corrects BGR to RGB for every type *except* 3 — `if (l_ras_type == 3) /* RGB-format ? That is
//!    what GIMP wants */ ... else /* We have BGR format. Correct it */`. So the default is **BGR**,
//!    and that is why `type <= 5` is accepted rather than just 1 and 2.
//! 3. **The RLE is an ESCAPE scheme, and a third distinct one in this group.** TGA keys on a packet
//!    header's high bit (set = repeat); SGI keys on a control byte's high bit *inverted* (set =
//!    literal); SUN keys on the single byte `0x80`. And the count is **off by one on purpose**:
//!    `rle_fgetc` sets `rlebuf.n = runcnt` and *also* returns `runval` immediately, so `0x80 n v`
//!    yields **n + 1** copies. `0x80 0x00` is a literal `0x80`.
//! 4. **Rows are padded to an EVEN byte count.** Upstream computes the pad per depth —
//!    `(((width+7)/8) % 2)` at 1 bit, `(width % 2)` at 8, `((width*3) % 2)` at 24 — which is one
//!    rule: pad when the row's byte count is odd. Ignoring it shears the image progressively.
//! 5. **At 32 bits the unused byte comes FIRST.** `getc (ifp); /* Skip unused byte */` precedes the
//!    three samples, so the layout is pad-then-BGR, not BGR-then-pad.
//!
//! The colormap is **planar**: upstream indexes it `suncolmap[j]`, `suncolmap[j + ncols]`,
//! `suncolmap[j + 2 * ncols]` — every red, then every green, then every blue, not RGB triples.

use crate::{FormatError, Result};

const RAS_MAGIC: u32 = 0x59a6_6a95;
const HEADER_LEN: usize = 32;
/// Upstream's own ceiling: `maplength > (256 * 3)` is refused outright.
const MAX_MAP_LEN: u32 = 256 * 3;
/// Upstream refuses only `type > 5`, so 0..=5 are all accepted at the header check.
const MAX_TYPE: u32 = 5;
/// The one type whose samples are already RGB; every other type is BGR.
const RAS_TYPE_RGB: u32 = 3;
const RAS_TYPE_RLE: u32 = 2;

pub(crate) struct SunRaster {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

fn be32(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// Is this a SUN raster?
///
/// The magic is a full four bytes and strong on its own, but the two validations upstream performs
/// on top of it are cheap and are applied here too, so a file this product claims is one its
/// decoder will also accept.
pub(crate) fn looks_like_sun_raster(bytes: &[u8]) -> bool {
    bytes.len() >= HEADER_LEN
        && be32(bytes, 0) == RAS_MAGIC
        && be32(bytes, 20) <= MAX_TYPE
        && be32(bytes, 28) <= MAX_MAP_LEN
}

/// Decode a SUN raster to RGBA.
pub(crate) fn decode(bytes: &[u8]) -> Result<SunRaster> {
    if !looks_like_sun_raster(bytes) {
        return Err(FormatError::UnsupportedFeature("not a SUN raster image").into());
    }
    let width = be32(bytes, 4) as usize;
    let height = be32(bytes, 8) as usize;
    let depth = be32(bytes, 12);
    let kind = be32(bytes, 20);
    let maptype = be32(bytes, 24);
    let maplength = be32(bytes, 28) as usize;

    if width == 0 || height == 0 {
        return Err(FormatError::UnsupportedFeature("SUN raster declares an empty image").into());
    }
    crate::document::pixel_count(width as u32, height as u32)?;

    // The colormap sits immediately after the header, and is PLANAR: all reds, then all greens,
    // then all blues.
    let map =
        bytes
            .get(HEADER_LEN..HEADER_LEN + maplength)
            .ok_or(FormatError::UnsupportedFeature(
                "SUN raster colormap is short",
            ))?;
    let palette = if maptype == 1 && maplength > 0 {
        let entries = maplength / 3;
        Some(
            (0..entries)
                .map(|index| [map[index], map[index + entries], map[index + 2 * entries]])
                .collect::<Vec<_>>(),
        )
    } else {
        None
    };

    let body = &bytes[HEADER_LEN + maplength..];
    // One rule, not three: pad when the row's byte count is odd.
    let row_bytes = match depth {
        1 => width.div_ceil(8),
        8 => width,
        24 => width * 3,
        32 => width * 4,
        other => {
            let _ = other;
            return Err(
                FormatError::UnsupportedFeature("SUN raster depth is not 1, 8, 24 or 32").into(),
            );
        }
    };
    // At 32 bits each pixel is already four bytes, so the row is even and never padded.
    let pad = row_bytes % 2;

    let mut samples = if kind == RAS_TYPE_RLE {
        decode_rle(body, (row_bytes + pad) * height)?
    } else {
        body.to_vec()
    };
    samples.resize((row_bytes + pad) * height, 0);

    // BGR unless the type says otherwise -- the overload in fact 2 above.
    let bgr = kind != RAS_TYPE_RGB;

    let mut rgba = vec![255u8; width * height * 4];
    for y in 0..height {
        let row = &samples[y * (row_bytes + pad)..][..row_bytes];
        for x in 0..width {
            let (r, g, b, a) = match depth {
                1 => {
                    // One bit per pixel, most significant first. A SET bit is BLACK here, matching
                    // upstream's 1-bit path, which is the same polarity PBM uses and the opposite
                    // of an additive grey ramp.
                    let bit = (row[x / 8] >> (7 - (x % 8))) & 1;
                    let value = if bit == 1 { 0 } else { 255 };
                    (value, value, value, 255)
                }
                8 => match &palette {
                    Some(entries) => {
                        let entry = entries.get(usize::from(row[x])).ok_or(
                            FormatError::UnsupportedFeature(
                                "SUN raster index is outside its colormap",
                            ),
                        )?;
                        (entry[0], entry[1], entry[2], 255)
                    }
                    None => (row[x], row[x], row[x], 255),
                },
                24 => {
                    let at = x * 3;
                    if bgr {
                        (row[at + 2], row[at + 1], row[at], 255)
                    } else {
                        (row[at], row[at + 1], row[at + 2], 255)
                    }
                }
                _ => {
                    // The unused byte comes FIRST at 32 bits.
                    let at = x * 4;
                    if bgr {
                        (row[at + 3], row[at + 2], row[at + 1], 255)
                    } else {
                        (row[at + 1], row[at + 2], row[at + 3], 255)
                    }
                }
            };
            let out = (y * width + x) * 4;
            rgba[out] = r;
            rgba[out + 1] = g;
            rgba[out + 2] = b;
            rgba[out + 3] = a;
        }
    }

    Ok(SunRaster {
        width: width as u32,
        height: height as u32,
        rgba,
    })
}

/// The escape-based RLE.
///
/// A byte other than `0x80` is itself. `0x80 0x00` is a literal `0x80`. `0x80 n v` emits `v`
/// **n + 1** times — upstream buffers `n` more copies *after* returning one, and that off-by-one is
/// deliberate, not a reading error.
fn decode_rle(source: &[u8], expected: usize) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(expected);
    let mut at = 0usize;
    while out.len() < expected {
        let Some(&flag) = source.get(at) else { break };
        at += 1;
        if flag != 0x80 {
            out.push(flag);
            continue;
        }
        let count = *source.get(at).ok_or(FormatError::UnsupportedFeature(
            "SUN raster RLE ended mid-escape",
        ))?;
        at += 1;
        if count == 0 {
            // An escaped 0x80, not a run.
            out.push(0x80);
            continue;
        }
        let value = *source.get(at).ok_or(FormatError::UnsupportedFeature(
            "SUN raster RLE run has no value",
        ))?;
        at += 1;
        out.extend(std::iter::repeat_n(value, usize::from(count) + 1));
    }
    Ok(out)
}

/// Encode an uncompressed 24-bit SUN raster.
///
/// `type = 1`, so the samples are written **BGR** — the default this format declares by *not*
/// being type 3. Uncompressed because an uncompressed writer's output is checkable by eye against
/// the header table, and the RLE reader is exercised by its own hand-built fixture rather than by
/// our encoder agreeing with itself.
pub(crate) fn encode(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    let (w, h) = (width as usize, height as usize);
    let row_bytes = w * 3;
    let pad = row_bytes % 2;

    let mut bytes = Vec::with_capacity(HEADER_LEN + (row_bytes + pad) * h);
    bytes.extend_from_slice(&RAS_MAGIC.to_be_bytes());
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&24u32.to_be_bytes()); // depth in BITS
    bytes.extend_from_slice(&(((row_bytes + pad) * h) as u32).to_be_bytes());
    bytes.extend_from_slice(&1u32.to_be_bytes()); // standard, uncompressed -- and therefore BGR
    bytes.extend_from_slice(&0u32.to_be_bytes()); // no colormap
    bytes.extend_from_slice(&0u32.to_be_bytes());

    for y in 0..h {
        for x in 0..w {
            let at = (y * w + x) * 4;
            bytes.push(rgba[at + 2]);
            bytes.push(rgba[at + 1]);
            bytes.push(rgba[at]);
        }
        // Pad the row to an even byte count.
        bytes.extend(std::iter::repeat_n(0u8, pad));
    }
    Ok(bytes)
}
