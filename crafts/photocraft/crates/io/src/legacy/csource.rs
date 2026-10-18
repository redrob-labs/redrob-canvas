//! Export as C source (a `struct { width, height, bytes_per_pixel, pixel_data[] }` with the
//! pixels as a run-length string, the form compilers accept for large images) and as an HTML
//! table (one cell per pixel, each with its colour).

use crate::IoError;

fn ident(stem: &str) -> String {
    let mut s: String = stem.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    if s.is_empty() || s.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        s.insert(0, '_');
    }
    s
}

/// C source: `static const struct { unsigned int width, height, bytes_per_pixel; unsigned char
/// pixel_data[]; } <name> = { w, h, 4, "..." };` with one run-length encoded string literal.
pub fn encode_c(stem: &str, w: u32, h: u32, rgba: &[u8], warnings: &mut Vec<String>) -> Vec<u8> {
    let name = ident(stem);
    let alpha = rgba.chunks_exact(4).any(|p| p[3] != 255);
    let bpp = if alpha { 4 } else { 3 };
    if !alpha {
        warnings.push("opaque image: written as 3 bytes per pixel".into());
    }
    let mut s = String::new();
    s.push_str(&format!("/* {name}: {w} x {h} pixels, {bpp} bytes per pixel ({}), run-length encoded. */\n", if alpha { "RGBA" } else { "RGB" }));
    s.push_str(&format!("#define {}_RUN_LENGTH_DECODE(image_buf, rle_data, size, bpp) do \\\n", name.to_ascii_uppercase()));
    s.push_str("{ unsigned int __bpp; unsigned char *__ip; const unsigned char *__il, *__rd; \\\n");
    s.push_str("  __bpp = (bpp); __ip = (image_buf); __il = __ip + (size) * __bpp; \\\n");
    s.push_str("  __rd = (rle_data); \\\n");
    s.push_str("  while (__ip < __il) { unsigned int __l = *(__rd++); \\\n");
    s.push_str("    if (__l & 128) { __l = __l - 128; \\\n");
    s.push_str("      do { memcpy (__ip, __rd, __bpp); __ip += __bpp; } while (--__l); __rd += __bpp; \\\n");
    s.push_str("    } else { __l *= __bpp; memcpy (__ip, __rd, __l); __ip += __l; __rd += __l; } } \\\n");
    s.push_str("} while (0)\n");
    s.push_str(&format!("static const struct {{\n  unsigned int width;\n  unsigned int height;\n  unsigned int bytes_per_pixel; /* {bpp}: {} */\n  unsigned char rle_pixel_data[];\n}} {name} = {{\n  {w}, {h}, {bpp},\n", if alpha { "RGBA" } else { "RGB" }));
    // Run-length encode: runs (bit 7 set, count 1..=127) and literal stretches (count 1..=127).
    let px: Vec<&[u8]> = rgba.chunks_exact(4).map(|p| &p[..bpp]).collect();
    let mut rle: Vec<u8> = Vec::with_capacity(rgba.len() / 2);
    let mut i = 0;
    while i < px.len() {
        let mut run = 1;
        while i + run < px.len() && px[i + run] == px[i] && run < 127 {
            run += 1;
        }
        if run >= 2 {
            rle.push(128 + run as u8);
            rle.extend_from_slice(px[i]);
            i += run;
        } else {
            let start = i;
            let mut j = i;
            while j < px.len() && j - start < 127 && !(j + 1 < px.len() && px[j] == px[j + 1]) {
                j += 1;
            }
            if j == start {
                j += 1;
            }
            rle.push((j - start) as u8);
            for p in &px[start..j] {
                rle.extend_from_slice(p);
            }
            i = j;
        }
    }
    // One string literal per 64 bytes, octal escapes for anything that is not printable ASCII.
    s.push_str("  \"");
    let mut col = 0usize;
    for (k, &b) in rle.iter().enumerate() {
        let esc = match b {
            b'\\' => "\\\\".to_string(),
            b'"' => "\\\"".to_string(),
            b'?' => "\\?".to_string(),
            0x20..=0x7e => (b as char).to_string(),
            _ => format!("\\{b:o}"),
        };
        // An escape followed by a digit would be read as a longer escape: break the literal.
        let next_digit = rle.get(k + 1).is_some_and(|n| n.is_ascii_digit());
        s.push_str(&esc);
        col += 1;
        if (esc.starts_with('\\') && esc.len() > 2 && next_digit) || col >= 64 {
            s.push_str("\"\n  \"");
            col = 0;
        }
    }
    s.push_str("\",\n};\n");
    s.into_bytes()
}

/// HTML table: one `<td>` per pixel with a background colour; rows are `<tr>`.
pub fn encode_html(stem: &str, w: u32, h: u32, rgba: &[u8], warnings: &mut Vec<String>) -> Result<Vec<u8>, IoError> {
    const MAX_CELLS: u64 = 1 << 20;
    if u64::from(w) * u64::from(h) > MAX_CELLS {
        return Err(IoError::Unsupported(format!("{w} × {h} is too large for an HTML table (at most {MAX_CELLS} cells)")));
    }
    if rgba.chunks_exact(4).any(|p| p[3] != 255) {
        warnings.push("HTML tables have no alpha: transparent pixels were matted over white".into());
    }
    let title: String = stem
        .chars()
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            c => c.to_string(),
        })
        .collect();
    let mut s = String::with_capacity(rgba.len() * 10);
    s.push_str("<!DOCTYPE html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n");
    s.push_str(&format!("<title>{title}</title>\n"));
    s.push_str("<style>table{border-collapse:collapse}td{width:1px;height:1px;padding:0;border:0}</style>\n</head>\n<body>\n");
    s.push_str(&format!("<table width=\"{w}\" height=\"{h}\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\">\n"));
    let wu = w as usize;
    for y in 0..h as usize {
        s.push_str("<tr>");
        for x in 0..wu {
            let p = &rgba[(y * wu + x) * 4..][..4];
            let a = u32::from(p[3]);
            let m = |c: u8| ((u32::from(c) * a + 255 * (255 - a)) / 255) as u8;
            s.push_str(&format!("<td bgcolor=\"#{:02x}{:02x}{:02x}\"></td>", m(p[0]), m(p[1]), m(p[2])));
        }
        s.push_str("</tr>\n");
    }
    s.push_str("</table>\n</body>\n</html>\n");
    Ok(s.into_bytes())
}
