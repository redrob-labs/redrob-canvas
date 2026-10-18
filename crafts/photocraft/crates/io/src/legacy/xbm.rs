//! X BitMap: a C source fragment with `#define name_width`, `#define name_height` and a
//! `static unsigned char name_bits[]` array, one bit per pixel, LSB first, 1 = black.

use super::{Rgba, bad};
use crate::IoError;

const MAX_SOURCE: usize = 64 << 20;

/// Does `bytes` look like an XBM source?
pub fn is_xbm(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(4096)];
    let Ok(text) = std::str::from_utf8(head) else { return false };
    text.contains("_width") && text.contains("_height") && text.contains("#define")
}

fn define(text: &str, suffix: &str) -> Option<u32> {
    for line in text.lines() {
        let l = line.trim();
        let Some(rest) = l.strip_prefix("#define") else { continue };
        let mut it = rest.split_whitespace();
        let name = it.next()?;
        if name.ends_with(suffix) {
            return it.next().and_then(|v| v.parse().ok());
        }
    }
    None
}

pub fn decode(bytes: &[u8]) -> Result<Rgba, IoError> {
    if bytes.len() > MAX_SOURCE {
        return Err(bad("XBM", "source too large"));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| bad("XBM", "not UTF-8 text"))?;
    let w = define(text, "_width").ok_or_else(|| bad("XBM", "no _width define"))?;
    let h = define(text, "_height").ok_or_else(|| bad("XBM", "no _height define"))?;
    let hot_x = define(text, "_x_hot");
    let open = text.find('{').ok_or_else(|| bad("XBM", "no bits array"))?;
    let close = text[open..].find('}').map(|i| open + i).ok_or_else(|| bad("XBM", "unterminated bits array"))?;
    let body = &text[open + 1..close];
    let mut out = Rgba::new(w, h)?;
    let stride = (w as usize).div_ceil(8);
    let mut values = body.split(',').map(str::trim).filter(|s| !s.is_empty()).map(|s| {
        let s = s.trim_start_matches("0x").trim_start_matches("0X");
        u8::from_str_radix(s, 16).or_else(|_| s.parse::<u8>()).map_err(|_| bad("XBM", format!("bad byte `{s}`")))
    });
    for y in 0..h {
        for bx in 0..stride {
            let v = values.next().ok_or_else(|| bad("XBM", "bits array ends early"))??;
            for bit in 0..8 {
                let x = (bx * 8 + bit) as u32;
                if x < w {
                    let on = (v >> bit) & 1 == 1;
                    let c = if on { 0 } else { 255 };
                    out.put(x, y, [c, c, c, 255]);
                }
            }
        }
    }
    if hot_x.is_some() {
        out.warnings.push("the XBM hotspot is not kept".into());
    }
    Ok(out)
}

fn ident(stem: &str) -> String {
    let mut s: String = stem.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    if s.is_empty() || s.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        s.insert(0, '_');
    }
    s
}

/// Pixels darker than mid-grey (luma < 128) become set bits.
pub fn encode(stem: &str, w: u32, h: u32, rgba: &[u8], warnings: &mut Vec<String>) -> Result<Vec<u8>, IoError> {
    let name = ident(stem);
    let stride = (w as usize).div_ceil(8);
    let mut s = format!("#define {name}_width {w}\n#define {name}_height {h}\nstatic unsigned char {name}_bits[] = {{\n");
    let mut n = 0usize;
    let total = stride * h as usize;
    let mut partial = false;
    for y in 0..h as usize {
        for bx in 0..stride {
            let mut v = 0u8;
            for bit in 0..8 {
                let x = bx * 8 + bit;
                if x < w as usize {
                    let p = &rgba[(y * w as usize + x) * 4..][..4];
                    let l = super::luma(p);
                    if p[3] >= 128 && l < 128 {
                        v |= 1 << bit;
                    }
                    if p[3] >= 128 && !(l < 16 || l > 239) {
                        partial = true;
                    }
                }
            }
            if n % 12 == 0 {
                s.push_str("   ");
            }
            n += 1;
            s.push_str(&format!(" 0x{v:02x}"));
            if n < total {
                s.push(',');
            }
            if n % 12 == 0 || n == total {
                s.push('\n');
            }
        }
    }
    s.push_str("};\n");
    if partial {
        warnings.push("XBM is 1-bit: pixels were thresholded at 50 % grey".into());
    }
    Ok(s.into_bytes())
}
