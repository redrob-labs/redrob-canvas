//! KiSS cel (`.cel`): a 32-byte header (`KiSS`, 0x20, bits per pixel 4/8/32, size and offset)
//! then pixels: two 4-bit indices per byte, one 8-bit index per byte, or 32-bit BGRA. Index 0 is
//! transparent. The palette of an indexed cel lives in a separate `.kcf` the set ships, so
//! indexed cels open with a grey ramp. The old headerless 4-bit form (size only) is read too.

use redrob_photo_doc::Document;

use super::{Rgba, bad, le16};
use crate::IoError;

pub fn decode(bytes: &[u8]) -> Result<Rgba, IoError> {
    let (bpp, w, h, data) = if bytes.starts_with(b"KiSS") {
        if bytes.len() < 32 || bytes[4] != 0x20 {
            return Err(bad("CEL", "not a KiSS cel"));
        }
        let bpp = bytes[5];
        let w = u32::from(le16(bytes, 8).unwrap_or(0));
        let h = u32::from(le16(bytes, 10).unwrap_or(0));
        (bpp, w, h, &bytes[32..])
    } else {
        if bytes.len() < 4 {
            return Err(bad("CEL", "file too short"));
        }
        (4u8, u32::from(le16(bytes, 0).unwrap_or(0)), u32::from(le16(bytes, 2).unwrap_or(0)), &bytes[4..])
    };
    let mut out = Rgba::new(w, h)?;
    let (wu, hu) = (w as usize, h as usize);
    match bpp {
        4 => {
            let stride = wu.div_ceil(2);
            if data.len() < stride * hu {
                return Err(bad("CEL", "pixel data ends early"));
            }
            for y in 0..hu {
                for x in 0..wu {
                    let b = data[y * stride + x / 2];
                    let i = if x % 2 == 0 { b >> 4 } else { b & 0x0f };
                    let g = i * 17;
                    out.put(x as u32, y as u32, [g, g, g, if i == 0 { 0 } else { 255 }]);
                }
            }
            out.palette = Some((0..16u8).map(|i| [i * 17; 3]).collect());
            out.warnings.push("4-bit cel: the palette comes from the set's .kcf file, which is not read; shown as a grey ramp".into());
        }
        8 => {
            if data.len() < wu * hu {
                return Err(bad("CEL", "pixel data ends early"));
            }
            for y in 0..hu {
                for x in 0..wu {
                    let i = data[y * wu + x];
                    out.put(x as u32, y as u32, [i, i, i, if i == 0 { 0 } else { 255 }]);
                }
            }
            out.palette = Some((0..=255u8).map(|i| [i; 3]).collect());
            out.warnings.push("8-bit cel: the palette comes from the set's .kcf file, which is not read; shown as a grey ramp".into());
        }
        32 => {
            if data.len() < wu * hu * 4 {
                return Err(bad("CEL", "pixel data ends early"));
            }
            for (i, p) in data.chunks_exact(4).take(wu * hu).enumerate() {
                out.data[i * 4..i * 4 + 4].copy_from_slice(&[p[2], p[1], p[0], p[3]]);
            }
            out.palette = None;
        }
        b => return Err(bad("CEL", format!("{b} bits per pixel"))),
    }
    // A palette only matters when the pixels are opaque; transparent index 0 keeps the cel RGBA.
    if out.data.chunks_exact(4).any(|p| p[3] == 0) {
        out.palette = None;
    }
    Ok(out)
}

/// A 32-bit cel (BGRA), the only depth that keeps the colours without a `.kcf`.
pub fn encode(_doc: &Document, w: u32, h: u32, rgba: &[u8], _warnings: &mut [String]) -> Result<Vec<u8>, IoError> {
    if w > 65535 || h > 65535 {
        return Err(IoError::Unsupported("cels are at most 65535 pixels on a side".into()));
    }
    let mut out = vec![0u8; 32];
    out[0..4].copy_from_slice(b"KiSS");
    out[4] = 0x20;
    out[5] = 32;
    out[8..10].copy_from_slice(&(w as u16).to_le_bytes());
    out[10..12].copy_from_slice(&(h as u16).to_le_bytes());
    out.reserve(rgba.len());
    for p in rgba.chunks_exact(4) {
        out.extend([p[2], p[1], p[0], p[3]]);
    }
    Ok(out)
}
