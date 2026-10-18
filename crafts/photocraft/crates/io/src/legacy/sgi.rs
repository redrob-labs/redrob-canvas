//! SGI image file (`.sgi`, `.rgb`, `.rgba`, `.bw`): a 512-byte big-endian header, then either
//! uncompressed planes or per-scanline RLE with offset and length tables. 1 to 4 channels at 8 or
//! 16 bits per sample; rows are stored bottom-up.

use super::{Rgba, bad, be16, be32};
use crate::IoError;

const MAGIC: u16 = 474;

pub fn decode(bytes: &[u8]) -> Result<Rgba, IoError> {
    if bytes.len() < 512 || be16(bytes, 0) != Some(MAGIC) {
        return Err(bad("SGI", "not an SGI image"));
    }
    let storage = bytes[2];
    let bpc = bytes[3] as usize;
    let dim = be16(bytes, 4).unwrap_or(0);
    let w = u32::from(be16(bytes, 6).unwrap_or(0));
    let h = if dim >= 2 { u32::from(be16(bytes, 8).unwrap_or(1)) } else { 1 };
    let channels = if dim >= 3 { be16(bytes, 10).unwrap_or(1) as usize } else { 1 };
    let pixmin = be32(bytes, 12).unwrap_or(0);
    let pixmax = be32(bytes, 16).unwrap_or(255);
    if !(1..=4).contains(&channels) || !(1..=2).contains(&bpc) {
        return Err(bad("SGI", format!("{channels} channels at {bpc} bytes per sample")));
    }
    let mut out = Rgba::new(w, h)?;
    let (wu, hu) = (w as usize, h as usize);
    let plane = wu * hu;
    // Samples per channel, 8-bit, top-down.
    let mut planes: Vec<Vec<u8>> = Vec::with_capacity(channels);
    let range = (pixmax.saturating_sub(pixmin)).max(1) as f32;
    let scale = |v: u32| -> u8 {
        if bpc == 1 && pixmin == 0 && pixmax == 255 { v as u8 } else { ((v.saturating_sub(pixmin) as f32 / range) * 255.0).round().clamp(0.0, 255.0) as u8 }
    };
    let sample = |b: &[u8], i: usize| -> Option<u32> { if bpc == 1 { b.get(i).map(|&v| u32::from(v)) } else { be16(b, i * 2).map(u32::from) } };
    match storage {
        0 => {
            let data = &bytes[512..];
            let need = plane * channels * bpc;
            if data.len() < need {
                return Err(bad("SGI", "pixel data ends early"));
            }
            for c in 0..channels {
                let mut p = vec![0u8; plane];
                let base = c * plane;
                for y in 0..hu {
                    for x in 0..wu {
                        let v = sample(data, base + y * wu + x).unwrap_or(0);
                        p[(hu - 1 - y) * wu + x] = scale(v);
                    }
                }
                planes.push(p);
            }
        }
        1 => {
            let tables = hu * channels;
            let starts = &bytes[512..];
            if starts.len() < tables * 8 {
                return Err(bad("SGI", "RLE tables end early"));
            }
            for c in 0..channels {
                let mut p = vec![0u8; plane];
                for y in 0..hu {
                    let t = c * hu + y;
                    let start = be32(starts, t * 4).unwrap_or(0) as usize;
                    let len = be32(starts, tables * 4 + t * 4).unwrap_or(0) as usize;
                    let row = bytes.get(start..start.saturating_add(len)).ok_or_else(|| bad("SGI", "RLE row runs past the file"))?;
                    let mut x = 0usize;
                    let mut i = 0usize;
                    let dst = &mut p[(hu - 1 - y) * wu..][..wu];
                    while let Some(code) = sample(row, i) {
                        i += 1;
                        let count = (code & 0x7f) as usize;
                        if count == 0 {
                            break;
                        }
                        if code & 0x80 != 0 {
                            for _ in 0..count {
                                let v = sample(row, i).ok_or_else(|| bad("SGI", "RLE literal run ends early"))?;
                                i += 1;
                                if x < wu {
                                    dst[x] = scale(v);
                                }
                                x += 1;
                            }
                        } else {
                            let v = sample(row, i).ok_or_else(|| bad("SGI", "RLE repeat ends early"))?;
                            i += 1;
                            for _ in 0..count {
                                if x < wu {
                                    dst[x] = scale(v);
                                }
                                x += 1;
                            }
                        }
                    }
                }
                planes.push(p);
            }
        }
        s => return Err(bad("SGI", format!("unknown storage {s}"))),
    }
    #[allow(clippy::needless_range_loop)]
    for i in 0..plane {
        let px = match channels {
            1 => [planes[0][i], planes[0][i], planes[0][i], 255],
            2 => [planes[0][i], planes[0][i], planes[0][i], planes[1][i]],
            3 => [planes[0][i], planes[1][i], planes[2][i], 255],
            _ => [planes[0][i], planes[1][i], planes[2][i], planes[3][i]],
        };
        out.data[i * 4..i * 4 + 4].copy_from_slice(&px);
    }
    if bpc == 2 {
        out.warnings.push("16-bit SGI samples were reduced to 8 bits".into());
    }
    Ok(out)
}

fn rle_row(row: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(row.len() + row.len() / 64 + 2);
    let mut i = 0;
    while i < row.len() {
        // A run of equal bytes (3 or more pays off).
        let mut run = 1;
        while i + run < row.len() && row[i + run] == row[i] && run < 127 {
            run += 1;
        }
        if run >= 3 {
            out.push(run as u8);
            out.push(row[i]);
            i += run;
            continue;
        }
        // A literal stretch up to the next run of 3.
        let start = i;
        let mut j = i;
        while j < row.len() && j - start < 127 {
            if j + 2 < row.len() && row[j] == row[j + 1] && row[j] == row[j + 2] {
                break;
            }
            j += 1;
        }
        if j == start {
            j = start + 1;
        }
        out.push(0x80 | (j - start) as u8);
        out.extend_from_slice(&row[start..j]);
        i = j;
    }
    out.push(0);
    out
}

/// RLE SGI with 1 (grey), 2 (grey + alpha), 3 or 4 channels, 8 bits each.
pub fn encode(stem: &str, w: u32, h: u32, rgba: &[u8], grey: bool, warnings: &mut Vec<String>) -> Result<Vec<u8>, IoError> {
    if w > 65535 || h > 65535 {
        return Err(IoError::Unsupported("SGI images are at most 65535 pixels on a side".into()));
    }
    let (wu, hu) = (w as usize, h as usize);
    let alpha = rgba.chunks_exact(4).any(|p| p[3] != 255);
    let channels = match (grey, alpha) {
        (true, false) => 1,
        (true, true) => 2,
        (false, false) => 3,
        (false, true) => 4,
    };
    if grey {
        warnings.push("saved as a greyscale SGI".into());
    }
    let mut header = vec![0u8; 512];
    header[0..2].copy_from_slice(&MAGIC.to_be_bytes());
    header[2] = 1;
    header[3] = 1;
    header[4..6].copy_from_slice(&(if channels == 1 { 2u16 } else { 3u16 }).to_be_bytes());
    header[6..8].copy_from_slice(&(w as u16).to_be_bytes());
    header[8..10].copy_from_slice(&(h as u16).to_be_bytes());
    header[10..12].copy_from_slice(&(channels as u16).to_be_bytes());
    header[12..16].copy_from_slice(&0u32.to_be_bytes());
    header[16..20].copy_from_slice(&255u32.to_be_bytes());
    let name: Vec<u8> = stem.bytes().take(79).collect();
    header[24..24 + name.len()].copy_from_slice(&name);
    header[104..108].copy_from_slice(&0u32.to_be_bytes()); // colormap: normal
    let tables = hu * channels;
    let mut starts = Vec::with_capacity(tables);
    let mut lengths = Vec::with_capacity(tables);
    let mut data = Vec::new();
    let data_base = 512 + tables * 8;
    let mut row = vec![0u8; wu];
    for c in 0..channels {
        for y in 0..hu {
            let sy = hu - 1 - y;
            for x in 0..wu {
                let p = &rgba[(sy * wu + x) * 4..][..4];
                row[x] = match (channels, c) {
                    (1 | 2, 0) => super::luma(p),
                    (2, _) => p[3],
                    (_, 3) => p[3],
                    (_, k) => p[k],
                };
            }
            let enc = rle_row(&row);
            starts.push((data_base + data.len()) as u32);
            lengths.push(enc.len() as u32);
            data.extend(enc);
        }
    }
    let mut out = header;
    out.extend(starts.iter().flat_map(|v| v.to_be_bytes()));
    out.extend(lengths.iter().flat_map(|v| v.to_be_bytes()));
    out.extend(data);
    Ok(out)
}
