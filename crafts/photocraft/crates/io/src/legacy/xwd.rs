//! X Window Dump (`.xwd`): a big-endian header of 25 words, the window name, a colormap of
//! 12-byte entries, then the pixmap. ZPixmap images at 8 bits (through the colormap), 16, 24 and
//! 32 bits (through the RGB masks) and 1-bit XY bitmaps are read; 32-bit TrueColor ZPixmaps are
//! written.

use super::{Rgba, bad, be16, be32};
use crate::IoError;

fn mask_shift(mask: u32) -> (u32, u32) {
    if mask == 0 {
        return (0, 0);
    }
    let shift = mask.trailing_zeros();
    let bits = (mask >> shift).count_ones();
    (shift, bits)
}

fn expand(v: u32, bits: u32) -> u8 {
    if bits == 0 {
        return 0;
    }
    if bits >= 8 {
        return (v >> (bits - 8)) as u8;
    }
    let max = (1u32 << bits) - 1;
    ((v * 255 + max / 2) / max) as u8
}

pub fn decode(bytes: &[u8]) -> Result<Rgba, IoError> {
    let word = |i: usize| be32(bytes, i * 4).ok_or_else(|| bad("XWD", "header ends early"));
    let header_size = word(0)? as usize;
    let version = word(1)?;
    if version != 7 || header_size < 100 {
        return Err(bad("XWD", "not an X11 window dump (version 7)"));
    }
    let format = word(2)?;
    let depth = word(3)?;
    let w = word(4)?;
    let h = word(5)?;
    let byte_order = word(7)?;
    let bit_order = word(9)?;
    let bits_per_pixel = word(11)?;
    let bytes_per_line = word(12)? as usize;
    let visual = word(13)?;
    let (rm, gm, bm) = (word(14)?, word(15)?, word(16)?);
    let ncolors = word(19)? as usize;
    let mut out = Rgba::new(w, h)?;
    let cmap_at = header_size;
    let pix_at = cmap_at.checked_add(ncolors.saturating_mul(12)).ok_or_else(|| bad("XWD", "colormap too large"))?;
    if ncolors > 65_536 || pix_at > bytes.len() {
        return Err(bad("XWD", "colormap runs past the file"));
    }
    let cmap: Vec<[u8; 3]> = (0..ncolors)
        .map(|i| {
            let at = cmap_at + i * 12;
            [(be16(bytes, at + 4).unwrap_or(0) >> 8) as u8, (be16(bytes, at + 6).unwrap_or(0) >> 8) as u8, (be16(bytes, at + 8).unwrap_or(0) >> 8) as u8]
        })
        .collect();
    let data = &bytes[pix_at..];
    let (wu, hu) = (w as usize, h as usize);
    if bytes_per_line.checked_mul(hu).is_none_or(|n| n > data.len()) {
        return Err(bad("XWD", "pixel data ends early"));
    }
    let le = byte_order == 0;
    let (rs, rb) = mask_shift(rm);
    let (gs, gb) = mask_shift(gm);
    let (bs, bb) = mask_shift(bm);
    let true_color = matches!(visual, 4 | 5) && rm != 0;
    match (format, bits_per_pixel) {
        (0 | 1, 1) | (2, 1) => {
            for y in 0..hu {
                let row = &data[y * bytes_per_line..];
                for x in 0..wu {
                    let b = row.get(x / 8).copied().unwrap_or(0);
                    let bit = if bit_order == 0 { (b >> (x % 8)) & 1 } else { (b >> (7 - x % 8)) & 1 };
                    let c = cmap.get(bit as usize).copied().unwrap_or(if bit == 1 { [0, 0, 0] } else { [255, 255, 255] });
                    out.put(x as u32, y as u32, [c[0], c[1], c[2], 255]);
                }
            }
        }
        (2, 8) => {
            for y in 0..hu {
                let row = &data[y * bytes_per_line..];
                for x in 0..wu {
                    let i = row.get(x).copied().unwrap_or(0) as usize;
                    let c = cmap.get(i).copied().unwrap_or([i as u8, i as u8, i as u8]);
                    out.put(x as u32, y as u32, [c[0], c[1], c[2], 255]);
                }
            }
            if !cmap.is_empty() && cmap.len() <= 256 {
                out.palette = Some(cmap.clone());
            }
        }
        (2, 16 | 24 | 32) => {
            let bpp = (bits_per_pixel / 8) as usize;
            let zero = [0u8; 4];
            for y in 0..hu {
                let row = &data[y * bytes_per_line..];
                for x in 0..wu {
                    let p = row.get(x * bpp..x * bpp + bpp).unwrap_or(&zero[..bpp]);
                    let mut v = 0u32;
                    for (k, &b) in p.iter().enumerate() {
                        v |= u32::from(b) << (if le { 8 * k } else { 8 * (bpp - 1 - k) });
                    }
                    let px = if true_color || rm != 0 {
                        [expand((v & rm) >> rs, rb), expand((v & gm) >> gs, gb), expand((v & bm) >> bs, bb), 255]
                    } else if bpp == 1 {
                        let c = cmap.get(v as usize).copied().unwrap_or([0, 0, 0]);
                        [c[0], c[1], c[2], 255]
                    } else {
                        [(v >> 16) as u8, (v >> 8) as u8, v as u8, 255]
                    };
                    out.put(x as u32, y as u32, px);
                }
            }
        }
        (f, b) => return Err(bad("XWD", format!("pixmap format {f} at {b} bits per pixel is not supported (depth {depth})"))),
    }
    Ok(out)
}

/// A 32-bit TrueColor ZPixmap, big-endian, with 0x00RRGGBB pixels.
pub fn encode(stem: &str, w: u32, h: u32, rgba: &[u8], warnings: &mut Vec<String>) -> Result<Vec<u8>, IoError> {
    if rgba.chunks_exact(4).any(|p| p[3] != 255) {
        warnings.push("XWD has no alpha channel; transparency was dropped".into());
    }
    let mut name: Vec<u8> = stem.bytes().take(255).collect();
    name.push(0);
    let header_size = 100 + name.len();
    let words: [u32; 25] = [
        header_size as u32,
        7,
        2, // ZPixmap
        24,
        w,
        h,
        0,
        1, // MSBFirst byte order
        32,
        1,
        32,
        32,
        w * 4,
        4, // TrueColor
        0x00ff_0000,
        0x0000_ff00,
        0x0000_00ff,
        8,
        256,
        0,
        w,
        h,
        0,
        0,
        0,
    ];
    let mut out = Vec::with_capacity(header_size + rgba.len());
    for v in words {
        out.extend(v.to_be_bytes());
    }
    out.extend(name);
    for p in rgba.chunks_exact(4) {
        out.extend([0, p[0], p[1], p[2]]);
    }
    Ok(out)
}
