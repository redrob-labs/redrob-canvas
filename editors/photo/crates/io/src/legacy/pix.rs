//! Alias/Wavefront PIX (`.pix`, `.als`): a 10-byte big-endian header (width, height, x and y
//! offset, bits per pixel: 24 for BGR, 8 for grey) followed by run-length pixels, each run a
//! count byte then one pixel, rows top-down and never crossing a row boundary.

use super::{Rgba, bad, be16};
use crate::IoError;

pub fn decode(bytes: &[u8]) -> Result<Rgba, IoError> {
    if bytes.len() < 10 {
        return Err(bad("PIX", "file too short"));
    }
    let w = u32::from(be16(bytes, 0).unwrap_or(0));
    let h = u32::from(be16(bytes, 2).unwrap_or(0));
    let bpp = be16(bytes, 8).unwrap_or(0);
    let bytes_per_px = match bpp {
        24 => 3,
        8 => 1,
        b => return Err(bad("PIX", format!("{b} bits per pixel"))),
    };
    let mut out = Rgba::new(w, h)?;
    let mut i = 10usize;
    for y in 0..h {
        let mut x = 0u32;
        while x < w {
            let count = u32::from(*bytes.get(i).ok_or_else(|| bad("PIX", "pixel data ends early"))?);
            let px = bytes.get(i + 1..i + 1 + bytes_per_px).ok_or_else(|| bad("PIX", "pixel data ends early"))?;
            i += 1 + bytes_per_px;
            let rgba = if bytes_per_px == 3 { [px[2], px[1], px[0], 255] } else { [px[0], px[0], px[0], 255] };
            for _ in 0..count.max(1) {
                out.put(x, y, rgba);
                x += 1;
                if x >= w {
                    break;
                }
            }
        }
    }
    Ok(out)
}

pub fn encode(w: u32, h: u32, rgba: &[u8], grey: bool, warnings: &mut Vec<String>) -> Result<Vec<u8>, IoError> {
    if w > 65535 || h > 65535 {
        return Err(IoError::Unsupported("PIX images are at most 65535 pixels on a side".into()));
    }
    let wu = w as usize;
    let mut out = Vec::with_capacity(10 + rgba.len());
    out.extend((w as u16).to_be_bytes());
    out.extend((h as u16).to_be_bytes());
    out.extend([0u8; 4]);
    out.extend((if grey { 8u16 } else { 24u16 }).to_be_bytes());
    if rgba.chunks_exact(4).any(|p| p[3] != 255) {
        warnings.push("PIX has no alpha channel; transparency was dropped".into());
    }
    for y in 0..h as usize {
        let row = &rgba[y * wu * 4..(y + 1) * wu * 4];
        let px = |x: usize| -> Vec<u8> {
            let p = &row[x * 4..x * 4 + 4];
            if grey { vec![super::luma(p)] } else { vec![p[2], p[1], p[0]] }
        };
        let mut x = 0;
        while x < wu {
            let cur = px(x);
            let mut n = 1;
            while x + n < wu && n < 255 && px(x + n) == cur {
                n += 1;
            }
            out.push(n as u8);
            out.extend(cur);
            x += n;
        }
    }
    Ok(out)
}
