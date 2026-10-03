// SPDX-License-Identifier: GPL-3.0-or-later

//! Krita's NATIVE tiled paint-device data, re-derived from the layout in Krita's
//! `libs/image/tiles3` (behaviour studied, no code copied).
//!
//! This is what a real `.kra` actually stores for a paint layer. Until this existed, a file authored in
//! Krita opened as its flattened preview, which loses every layer -- the layer stack was present in
//! `maindoc.xml` and unreadable.
//!
//! The format is a short text header followed by one record per tile:
//!
//! ```text
//! VERSION 2
//! TILEWIDTH 64
//! TILEHEIGHT 64
//! PIXELSIZE 4
//! DATA 12
//! <x>,<y>,LZF,<byte count>\n<bytes>
//! ```
//!
//! Three things in here are easy to get wrong and silent when wrong:
//!
//! 1. A tile's `x`,`y` are PIXEL offsets and may be NEGATIVE. A Krita layer is unbounded, so it
//!    genuinely holds tiles left of and above the canvas; clamping them to zero would stack unrelated
//!    tiles on top of each other at the origin.
//! 2. The first byte of a tile's data is a FLAG, not data: 0 means the rest is the raw tile, 1 means it
//!    is LZF-compressed. Krita writes raw whenever compression would not pay, so both occur in one file.
//! 3. The compressed form is of a BYTE-PLANARISED tile: all first bytes of every pixel, then all second
//!    bytes, and so on. Decompressing without un-planarising yields a picture that looks like a colour
//!    channel smeared across the tile -- plausible enough to mistake for a colour-space bug.
//!
//! Krita's 8-bit RGBA pixels are stored BGRA (blue first), which is its colour-space traits' own order,
//! not an endianness accident to undo.

use crate::{FormatError, Result};

const TILE_HEADER_LIMIT: usize = 64;

/// Krita's tile edge, used when WRITING. Reading takes the size from the file's own header instead, so a
/// document written with a different tile size still opens.
const TILE_SIDE: usize = 64;

/// One decoded tiled paint device, as a canvas-sized RGBA buffer.
pub(crate) fn decode_tiled_layer(
    bytes: &[u8],
    canvas_w: u32,
    canvas_h: u32,
    default_pixel: Option<[u8; 4]>,
) -> Result<Vec<u8>> {
    let mut cursor = 0usize;
    let mut tile_w = 0usize;
    let mut tile_h = 0usize;
    let mut pixel_size = 0usize;
    let mut tiles = 0usize;
    let mut found_data = false;

    // Header lines, in any order, terminated by DATA.
    for _ in 0..16 {
        let line = read_line(bytes, &mut cursor)
            .ok_or(FormatError::Malformed("KRA tile header truncated"))?;
        let mut parts = line.split_whitespace();
        let keyword = parts.next().unwrap_or("");
        let value: usize = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
        match keyword {
            "VERSION" => {
                if value > 2 {
                    return Err(FormatError::UnsupportedFeature("KRA tile version").into());
                }
            }
            "TILEWIDTH" => tile_w = value,
            "TILEHEIGHT" => tile_h = value,
            "PIXELSIZE" => pixel_size = value,
            "DATA" => {
                tiles = value;
                found_data = true;
            }
            _ => {}
        }
        if found_data {
            break;
        }
    }
    if !found_data || tile_w == 0 || tile_h == 0 {
        return Err(FormatError::Malformed("KRA tile header").into());
    }
    // 4 bytes is 8-bit RGBA, 8 bytes is 16-bit RGBA. Anything else is a colour space whose channels we
    // would be guessing at, and guessing produces a picture rather than an error.
    let channel_bytes = match pixel_size {
        4 => 1,
        8 => 2,
        _ => return Err(FormatError::UnsupportedFeature("KRA tile pixel size").into()),
    };

    let cw = canvas_w as usize;
    let ch = canvas_h as usize;
    // The default pixel fills everywhere no tile covers. Krita stores it separately from the tiles, and
    // it is not always transparent -- a layer filled with white keeps one white default pixel and no
    // tiles at all for the untouched region.
    let default = default_pixel.unwrap_or([0, 0, 0, 0]);
    let mut out = Vec::with_capacity(cw * ch * 4);
    for _ in 0..(cw * ch) {
        out.extend_from_slice(&default);
    }

    let tile_bytes = tile_w * tile_h * pixel_size;
    for _ in 0..tiles {
        let header = read_line_limited(bytes, &mut cursor, TILE_HEADER_LIMIT)
            .ok_or(FormatError::Malformed("KRA tile record header"))?;
        let mut fields = header.trim().split(',');
        let x: i64 = fields
            .next()
            .and_then(|v| v.trim().parse().ok())
            .ok_or(FormatError::Malformed("KRA tile x"))?;
        let y: i64 = fields
            .next()
            .and_then(|v| v.trim().parse().ok())
            .ok_or(FormatError::Malformed("KRA tile y"))?;
        let compression = fields.next().unwrap_or("").trim().to_owned();
        let size: usize = fields
            .next()
            .and_then(|v| v.trim().parse().ok())
            .ok_or(FormatError::Malformed("KRA tile size"))?;
        if compression != "LZF" {
            return Err(FormatError::UnsupportedFeature("KRA tile compression").into());
        }
        if cursor + size > bytes.len() || size == 0 {
            return Err(FormatError::Malformed("KRA tile data truncated").into());
        }
        let payload = &bytes[cursor..cursor + size];
        cursor += size;

        // Byte 0 is the flag, not data.
        let raw = if payload[0] == 0 {
            payload[1..].to_vec()
        } else {
            let planar = lzf_decompress(&payload[1..], tile_bytes)?;
            unplanarise(&planar, pixel_size)
        };
        if raw.len() < tile_bytes {
            return Err(FormatError::Malformed("KRA tile short").into());
        }

        for ty in 0..tile_h {
            let cy = y + ty as i64;
            if cy < 0 || cy as usize >= ch {
                continue;
            }
            for tx in 0..tile_w {
                let cx = x + tx as i64;
                if cx < 0 || cx as usize >= cw {
                    continue;
                }
                let source = (ty * tile_w + tx) * pixel_size;
                // BGRA in, RGBA out. At 16 bits per channel the high byte is the one that survives.
                let take = |channel: usize| raw[source + channel * channel_bytes];
                let destination = (cy as usize * cw + cx as usize) * 4;
                out[destination] = take(2);
                out[destination + 1] = take(1);
                out[destination + 2] = take(0);
                out[destination + 3] = take(3);
            }
        }
    }
    Ok(out)
}

/// Krita's `.defaultpixel` sidecar: the layer's fill outside every stored tile, as raw pixel bytes in
/// the device's own order (BGRA for 8-bit RGBA).
pub(crate) fn decode_default_pixel(bytes: &[u8]) -> Option<[u8; 4]> {
    if bytes.len() < 4 {
        return None;
    }
    Some([bytes[2], bytes[1], bytes[0], bytes[3]])
}

/// Writes a canvas-sized RGBA buffer as Krita's native tiled paint device (H.7).
///
/// Tiles cover the canvas from the origin, so the last row and column are PARTIAL in content but still
/// a whole 64x64 tile on disk -- a tile is always tile-sized, and the area past the canvas is written
/// transparent. Writing a short final tile instead would produce a file Krita rejects.
///
/// Each tile is written in whichever form is SMALLER, which is also what Krita does: the compressed
/// form of a flat or empty tile is bigger than the tile, so always compressing would inflate exactly
/// the files that compress worst.
pub(crate) fn encode_tiled_layer(rgba: &[u8], canvas_w: u32, canvas_h: u32) -> Vec<u8> {
    let cw = canvas_w as usize;
    let ch = canvas_h as usize;
    let cols = cw.div_ceil(TILE_SIDE);
    let rows = ch.div_ceil(TILE_SIDE);
    let tile_bytes = TILE_SIDE * TILE_SIDE * 4;

    let mut records: Vec<u8> = Vec::new();
    let mut count = 0usize;
    for row in 0..rows {
        for col in 0..cols {
            let origin_x = col * TILE_SIDE;
            let origin_y = row * TILE_SIDE;
            let mut tile = vec![0u8; tile_bytes];
            for ty in 0..TILE_SIDE {
                let sy = origin_y + ty;
                if sy >= ch {
                    break;
                }
                for tx in 0..TILE_SIDE {
                    let sx = origin_x + tx;
                    if sx >= cw {
                        break;
                    }
                    let source = (sy * cw + sx) * 4;
                    let destination = (ty * TILE_SIDE + tx) * 4;
                    // RGBA out, BGRA in: the device's own channel order.
                    tile[destination] = rgba[source + 2];
                    tile[destination + 1] = rgba[source + 1];
                    tile[destination + 2] = rgba[source];
                    tile[destination + 3] = rgba[source + 3];
                }
            }
            let compressed = lzf_compress(&planarise(&tile, 4));
            let payload = match compressed {
                Some(bytes) if bytes.len() < tile_bytes => {
                    let mut out = Vec::with_capacity(bytes.len() + 1);
                    out.push(1); // compressed
                    out.extend_from_slice(&bytes);
                    out
                }
                _ => {
                    let mut out = Vec::with_capacity(tile_bytes + 1);
                    out.push(0); // raw
                    out.extend_from_slice(&tile);
                    out
                }
            };
            records.extend_from_slice(
                format!("{origin_x},{origin_y},LZF,{}\n", payload.len()).as_bytes(),
            );
            records.extend_from_slice(&payload);
            count += 1;
        }
    }

    let mut out = Vec::with_capacity(records.len() + 64);
    out.extend_from_slice(b"VERSION 2\n");
    out.extend_from_slice(format!("TILEWIDTH {TILE_SIDE}\n").as_bytes());
    out.extend_from_slice(format!("TILEHEIGHT {TILE_SIDE}\n").as_bytes());
    out.extend_from_slice(b"PIXELSIZE 4\n");
    out.extend_from_slice(format!("DATA {count}\n").as_bytes());
    out.extend_from_slice(&records);
    out
}

/// The inverse of `unplanarise`: groups byte `b` of every pixel together, which is the arrangement
/// Krita compresses (it makes a tile's bytes far more repetitive, so LZF finds real matches).
fn planarise(interleaved: &[u8], pixel_size: usize) -> Vec<u8> {
    let stride = interleaved.len() / pixel_size.max(1);
    let mut out = vec![0u8; stride * pixel_size];
    for index in 0..stride {
        for byte in 0..pixel_size {
            out[byte * stride + index] = interleaved[index * pixel_size + byte];
        }
    }
    out
}

/// LZF compression, re-derived from the same format the decompressor undoes.
///
/// Returns `None` when the output would not be smaller than the input, so the caller can store the
/// tile raw instead of paying a byte to say "compressed" for nothing.
///
/// Two limits are the format's, not choices: a literal run carries at most 32 bytes (the control byte
/// holds `run - 1` in five bits) and a back reference reaches at most 8192 bytes (thirteen bits), so a
/// match further back than that has to be emitted as literals.
fn lzf_compress(input: &[u8]) -> Option<Vec<u8>> {
    const HASH_BITS: usize = 14;
    const MAX_OFFSET: usize = 1 << 13;
    const MAX_MATCH: usize = 264;
    const MAX_LITERAL: usize = 32;

    if input.len() < 4 {
        return None;
    }
    let hash = |a: u8, b: u8, c: u8| -> usize {
        let value = (usize::from(a) << 16) | (usize::from(b) << 8) | usize::from(c);
        (value.wrapping_mul(2654435761) >> (32 - HASH_BITS)) & ((1 << HASH_BITS) - 1)
    };

    let mut table = vec![usize::MAX; 1 << HASH_BITS];
    let mut out: Vec<u8> = Vec::with_capacity(input.len());
    let mut literals: Vec<u8> = Vec::with_capacity(MAX_LITERAL);
    let mut ip = 0usize;

    // A literal run is emitted as its control byte followed by the bytes themselves.
    fn flush(out: &mut Vec<u8>, literals: &mut Vec<u8>) {
        if literals.is_empty() {
            return;
        }
        out.push((literals.len() - 1) as u8);
        out.extend_from_slice(literals);
        literals.clear();
    }

    while ip + 2 < input.len() {
        let slot = hash(input[ip], input[ip + 1], input[ip + 2]);
        let candidate = table[slot];
        table[slot] = ip;
        let offset = if candidate == usize::MAX {
            usize::MAX
        } else {
            ip - candidate - 1
        };
        let matches = candidate != usize::MAX
            && offset < MAX_OFFSET
            && input[candidate] == input[ip]
            && input[candidate + 1] == input[ip + 1]
            && input[candidate + 2] == input[ip + 2];
        if matches {
            let mut length = 3usize;
            while ip + length < input.len()
                && length < MAX_MATCH
                && input[candidate + length] == input[ip + length]
            {
                length += 1;
            }
            flush(&mut out, &mut literals);
            // The decoder reads `total = (control >> 5) + 2`, extended by one byte when that reaches 7.
            let code = length - 2;
            if code < 7 {
                out.push(((code << 5) | (offset >> 8)) as u8);
            } else {
                out.push(((7 << 5) | (offset >> 8)) as u8);
                out.push((code - 7) as u8);
            }
            out.push((offset & 0xff) as u8);
            ip += length;
        } else {
            literals.push(input[ip]);
            if literals.len() == MAX_LITERAL {
                flush(&mut out, &mut literals);
            }
            ip += 1;
        }
        if out.len() >= input.len() {
            return None;
        }
    }
    while ip < input.len() {
        literals.push(input[ip]);
        if literals.len() == MAX_LITERAL {
            flush(&mut out, &mut literals);
        }
        ip += 1;
    }
    flush(&mut out, &mut literals);
    if out.len() >= input.len() {
        None
    } else {
        Some(out)
    }
}

fn read_line(bytes: &[u8], cursor: &mut usize) -> Option<String> {
    read_line_limited(bytes, cursor, 256)
}

fn read_line_limited(bytes: &[u8], cursor: &mut usize, limit: usize) -> Option<String> {
    let start = *cursor;
    let mut end = start;
    while end < bytes.len() && bytes[end] != b'\n' && end - start < limit {
        end += 1;
    }
    if end >= bytes.len() {
        return None;
    }
    let line = String::from_utf8_lossy(&bytes[start..end]).into_owned();
    *cursor = end + 1;
    Some(line)
}

/// Undoes the byte-planarisation Krita applies before compressing: plane `b` holds byte `b` of every
/// pixel in turn, so a pixel's bytes are `stride` apart rather than adjacent.
fn unplanarise(planar: &[u8], pixel_size: usize) -> Vec<u8> {
    let stride = planar.len() / pixel_size.max(1);
    let mut out = vec![0u8; stride * pixel_size];
    for index in 0..stride {
        for byte in 0..pixel_size {
            out[index * pixel_size + byte] = planar[byte * stride + index];
        }
    }
    out
}

/// LZF decompression, re-derived from the algorithm Krita's tile compressor uses.
///
/// A control byte below 32 means a literal run of `byte + 1` bytes. Otherwise the top three bits are a
/// match length and the low five bits are the high bits of a BACK offset, with the next byte supplying
/// the low bits -- and a length of 7 means one more byte extends it. The copy is byte-by-byte because
/// the match may OVERLAP its own output (that is how a run is encoded), so a block move would read
/// bytes it has not written yet.
fn lzf_decompress(input: &[u8], expected: usize) -> Result<Vec<u8>> {
    let mut out: Vec<u8> = Vec::with_capacity(expected);
    let mut ip = 0usize;
    while ip < input.len() {
        let control = input[ip] as usize;
        ip += 1;
        if control < 32 {
            let run = control + 1;
            if ip + run > input.len() || out.len() + run > expected {
                return Err(FormatError::Malformed("KRA LZF literal overrun").into());
            }
            out.extend_from_slice(&input[ip..ip + run]);
            ip += run;
        } else {
            let mut length = control >> 5;
            let mut offset = (control & 31) << 8;
            if length == 7 {
                if ip >= input.len() {
                    return Err(FormatError::Malformed("KRA LZF length overrun").into());
                }
                length += input[ip] as usize;
                ip += 1;
            }
            if ip >= input.len() {
                return Err(FormatError::Malformed("KRA LZF offset overrun").into());
            }
            offset += input[ip] as usize;
            ip += 1;
            let total = length + 2;
            // `offset` counts back from the byte AFTER the last written one.
            if offset + 1 > out.len() || out.len() + total > expected {
                return Err(FormatError::Malformed("KRA LZF back reference out of range").into());
            }
            let mut reference = out.len() - offset - 1;
            for _ in 0..total {
                let byte = out[reference];
                out.push(byte);
                reference += 1;
            }
        }
    }
    if out.len() != expected {
        return Err(FormatError::Malformed("KRA LZF short output").into());
    }
    Ok(out)
}
