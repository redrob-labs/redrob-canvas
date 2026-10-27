//! PostScript and Encapsulated PostScript (`.ps`, `.eps`, `.epsf`, `.epsi`, `.ai` legacy):
//! import of raster PostScript, export with preview and encapsulation options.
//!
//! Import reads what a raster EPS carries: the `%%BoundingBox`, `%ImageData:` and
//! `%%HiResBoundingBox` comments, and the `image` / `colorimage` operator's sample data in
//! ASCIIHex, ASCII85 or binary form (with `RunLengthDecode` when the filter chain names it), at
//! 1, 2, 4, 8 or 12 bits per sample in grey, RGB or CMYK. A DOS EPS binary header's TIFF or WMF
//! preview section opens when the PostScript itself holds no raster. General PostScript programs
//! (vector drawings, text) have no interpreter here: they are refused with a note that says so.
//!
//! Export writes Level 2 PostScript: an EPSF-3.0 file with a `%%BoundingBox` from the document
//! size and resolution, the flattened image through `image` (grey) or `colorimage` (RGB / CMYK),
//! ASCIIHex or ASCII85 or binary encoding, optional `showpage` (a plain `.ps` page), and an
//! optional TIFF preview in a DOS EPS binary header.

use redrob_photo_codecs::{ChannelLayout, EncodeOptions, Format, Image};
use redrob_photo_doc::Document;

use crate::{ExportResult, ImportResult, IoError};

const DOS_EPS_MAGIC: [u8; 4] = [0xC5, 0xD0, 0xD3, 0xC6];
const MAX_SOURCE: usize = 512 << 20;

fn bad(msg: impl std::fmt::Display) -> IoError {
    IoError::Unsupported(format!("PostScript: {msg}"))
}

/// Is `bytes` PostScript (plain or a DOS EPS binary)?
pub fn is_postscript(bytes: &[u8]) -> bool {
    bytes.starts_with(b"%!PS") || bytes.starts_with(&DOS_EPS_MAGIC) || bytes.starts_with(b"%!\n") || bytes.starts_with(b"%!\r")
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes([*b.get(at)?, *b.get(at + 1)?, *b.get(at + 2)?, *b.get(at + 3)?]))
}

/// A DOS EPS binary header: the PostScript section and the optional WMF and TIFF previews.
struct DosEps<'a> {
    ps: &'a [u8],
    wmf: Option<&'a [u8]>,
    tiff: Option<&'a [u8]>,
}

fn dos_eps(bytes: &[u8]) -> Result<DosEps<'_>, IoError> {
    let section = |at: usize| -> Option<&[u8]> {
        let off = le32(bytes, at)? as usize;
        let len = le32(bytes, at + 4)? as usize;
        if off == 0 || len == 0 {
            return None;
        }
        bytes.get(off..off.checked_add(len)?)
    };
    let ps = section(4).ok_or_else(|| bad("DOS EPS header has no PostScript section"))?;
    Ok(DosEps { ps, wmf: section(12), tiff: section(20) })
}

/// The whole-number or decimal values after a `%%Key:` comment.
fn comment_numbers(text: &str, key: &str) -> Option<Vec<f64>> {
    for line in text.lines() {
        if let Some(rest) = line.trim_end().strip_prefix(key) {
            let v: Vec<f64> = rest.split_whitespace().filter_map(|t| t.parse().ok()).collect();
            if !v.is_empty() && !rest.contains("(atend)") {
                return Some(v);
            }
        }
    }
    None
}

/// What the `image` / `colorimage` call describes.
struct Raster {
    width: u32,
    height: u32,
    bits: u32,
    components: u32,
    /// Position of the first data byte in the source.
    data_at: usize,
    ascii85: bool,
    run_length: bool,
    /// Binary data (format 2 of `%ImageData`).
    binary: bool,
}

/// `text` with every `[...]` and `{...}` group replaced by a space (the image matrix and the
/// data procedure), so the scalar operands can be read.
fn remove_groups(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0usize;
    for c in text.chars() {
        match c {
            '[' | '{' => {
                depth += 1;
                if depth == 1 {
                    out.push(' ');
                }
            }
            ']' | '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// Finds the raster through `%ImageData:` or the operands before `image` / `colorimage`.
fn find_raster(src: &[u8]) -> Result<Raster, IoError> {
    let text = String::from_utf8_lossy(&src[..src.len().min(1 << 20)]);
    let op_at = find_op(&text).ok_or_else(|| {
        bad("no image or colorimage operator; general PostScript drawings are not rasterised (open the file in a PostScript viewer and export a bitmap)")
    })?;
    let before = &text[..op_at];
    let is_color = text[op_at..].starts_with("colorimage");
    let ascii85 = before.contains("ASCII85Decode");
    let run_length = before.contains("RunLengthDecode");
    let data_at = data_start(src, &text).ok_or_else(|| bad("no image data after the image operator"))?;
    // `%ImageData: w h bps channels pad lines format "op"` as raster EPS writers emit it.
    if let Some(v) = comment_numbers(&text, "%ImageData:")
        && v.len() >= 7
    {
        let (w, h, bits, comps, format) = (v[0] as u32, v[1] as u32, v[2] as u32, v[3] as u32, v[6] as u32);
        let binary = format == 2;
        return Ok(Raster { width: w, height: h, bits, components: comps, data_at, ascii85, run_length, binary });
    }
    // The dictionary form: /Width /Height /BitsPerComponent.
    let dict = |key: &str| -> Option<u32> { before.rfind(key).and_then(|i| before[i + key.len()..].split_whitespace().next()).and_then(|t| t.parse().ok()) };
    let stripped = remove_groups(before);
    let toks: Vec<&str> = stripped.split_whitespace().collect();
    let num = |i: usize| -> Option<u32> { toks.len().checked_sub(i).and_then(|k| toks.get(k)).and_then(|t| t.parse().ok()) };
    let (w, h, bits, components) = match (dict("/Width"), dict("/Height"), dict("/BitsPerComponent")) {
        (Some(w), Some(h), Some(b)) => {
            let comps = if before.contains("/DeviceCMYK") {
                4
            } else if before.contains("/DeviceRGB") {
                3
            } else {
                1
            };
            (w, h, b, comps)
        }
        _ if is_color => {
            // `w h bits [matrix] {proc} false ncomp colorimage`
            let comps = num(1).unwrap_or(3);
            (num(5).unwrap_or(0), num(4).unwrap_or(0), num(3).unwrap_or(8), comps)
        }
        _ => (num(3).unwrap_or(0), num(2).unwrap_or(0), num(1).unwrap_or(8), 1),
    };
    if w == 0 || h == 0 {
        return Err(bad("could not read the image size before the image operator"));
    }
    Ok(Raster { width: w, height: h, bits, components, data_at, ascii85, run_length, binary: false })
}

fn find_op(text: &str) -> Option<usize> {
    // The first `image` or `colorimage` token that is an operator (not in a comment or a name).
    let mut best = None;
    for (i, _) in text.match_indices("image") {
        let line_start = text[..i].rfind('\n').map_or(0, |p| p + 1);
        if text[line_start..i].trim_start().starts_with('%') {
            continue;
        }
        let prev = text[..i].chars().next_back();
        if prev == Some('/') {
            continue;
        }
        let start = if text[..i].ends_with("color") { i - 5 } else { i };
        let after = text[i + 5..].chars().next();
        if after.is_some_and(|c| c.is_alphanumeric()) {
            continue;
        }
        if text[line_start..start].trim_start().starts_with('/') && !text[line_start..start].contains(' ') {
            continue;
        }
        best = Some(start);
        break;
    }
    best
}

/// The byte after the line holding the image operator.
fn data_start(src: &[u8], text: &str) -> Option<usize> {
    let op = find_op(text)?;
    let line_end = text[op..].find('\n').map(|i| op + i + 1)?;
    // `text` is a lossy copy of the same bytes: ASCII offsets agree up to the first non-ASCII byte.
    let bytes_at = text[..line_end].len();
    (bytes_at <= src.len()).then_some(bytes_at)
}

fn hex_decode(src: &[u8], n: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(n);
    let mut hi: Option<u8> = None;
    for &c in src {
        let v = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            b'>' => break,
            _ => continue,
        };
        match hi.take() {
            None => hi = Some(v),
            Some(h) => out.push(h << 4 | v),
        }
        if out.len() >= n {
            break;
        }
    }
    if let Some(h) = hi
        && out.len() < n
    {
        out.push(h << 4);
    }
    out
}

fn ascii85_decode(src: &[u8], n: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(n);
    let mut group = [0u8; 5];
    let mut k = 0;
    let mut i = 0;
    // Skip a leading `<~`.
    if src.starts_with(b"<~") {
        i = 2;
    }
    while i < src.len() && out.len() < n {
        let c = src[i];
        i += 1;
        match c {
            b'~' => break,
            b'z' if k == 0 => out.extend([0, 0, 0, 0]),
            b'!'..=b'u' => {
                group[k] = c - b'!';
                k += 1;
                if k == 5 {
                    let mut v: u32 = 0;
                    for g in group {
                        v = v.wrapping_mul(85).wrapping_add(u32::from(g));
                    }
                    out.extend(v.to_be_bytes());
                    k = 0;
                }
            }
            _ => {}
        }
    }
    if k > 1 {
        for g in group.iter_mut().skip(k) {
            *g = 84;
        }
        let mut v: u32 = 0;
        for g in group {
            v = v.wrapping_mul(85).wrapping_add(u32::from(g));
        }
        out.extend(&v.to_be_bytes()[..k - 1]);
    }
    out.truncate(n);
    out
}

fn run_length_decode(src: &[u8], n: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(n);
    let mut i = 0;
    while i < src.len() && out.len() < n {
        let l = src[i] as usize;
        i += 1;
        if l == 128 {
            break;
        } else if l < 128 {
            let end = (i + l + 1).min(src.len());
            out.extend_from_slice(&src[i..end]);
            i = end;
        } else {
            if let Some(&v) = src.get(i) {
                out.extend(std::iter::repeat_n(v, 257 - l));
            }
            i += 1;
        }
    }
    out.truncate(n);
    out
}

/// Expands packed samples (1, 2, 4, 8 or 12 bits) to 8-bit values, row by row.
fn unpack(data: &[u8], w: usize, h: usize, comps: usize, bits: u32) -> Vec<u8> {
    let per_row = w * comps;
    let row_bytes = (per_row * bits as usize).div_ceil(8);
    let mut out = Vec::with_capacity(per_row * h);
    for y in 0..h {
        let row = data.get(y * row_bytes..).unwrap_or(&[]);
        for i in 0..per_row {
            let v = match bits {
                8 => row.get(i).copied().unwrap_or(0),
                1 => (row.get(i / 8).copied().unwrap_or(0) >> (7 - i % 8) & 1) * 255,
                2 => (row.get(i / 4).copied().unwrap_or(0) >> (6 - 2 * (i % 4)) & 3) * 85,
                4 => (row.get(i / 2).copied().unwrap_or(0) >> (4 - 4 * (i % 2)) & 15) * 17,
                12 => {
                    let bit = i * 12;
                    let b0 = row.get(bit / 8).copied().unwrap_or(0);
                    let b1 = row.get(bit / 8 + 1).copied().unwrap_or(0);
                    let v = if bit % 8 == 0 { (u16::from(b0) << 4) | u16::from(b1 >> 4) } else { (u16::from(b0 & 0x0f) << 8) | u16::from(b1) };
                    (v >> 4) as u8
                }
                16 => row.get(i * 2).copied().unwrap_or(0),
                _ => 0,
            };
            out.push(v);
        }
    }
    out
}

fn raster_image(src: &[u8], r: &Raster) -> Result<Image, IoError> {
    if r.width == 0 || r.height == 0 || u64::from(r.width) * u64::from(r.height) > 1 << 28 {
        return Err(bad(format!("image size {} × {} is out of range", r.width, r.height)));
    }
    if !matches!(r.bits, 1 | 2 | 4 | 8 | 12 | 16) || !matches!(r.components, 1 | 3 | 4) {
        return Err(bad(format!("{} bits per sample with {} components is not supported", r.bits, r.components)));
    }
    let (w, h, comps) = (r.width as usize, r.height as usize, r.components as usize);
    let row_bytes = (w * comps * r.bits as usize).div_ceil(8);
    let n = row_bytes * h;
    let data = &src[r.data_at.min(src.len())..];
    let raw = if r.binary {
        data.get(..n).ok_or_else(|| bad("binary image data ends early"))?.to_vec()
    } else if r.ascii85 {
        ascii85_decode(data, if r.run_length { n * 2 } else { n })
    } else {
        hex_decode(data, if r.run_length { n * 2 } else { n })
    };
    let raw = if r.run_length { run_length_decode(&raw, n) } else { raw };
    if raw.len() < n {
        return Err(bad(format!("image data ends early ({} of {n} bytes)", raw.len())));
    }
    let samples = unpack(&raw, w, h, comps, r.bits);
    let layout = match comps {
        1 => ChannelLayout::Gray,
        3 => ChannelLayout::Rgb,
        _ => ChannelLayout::Cmyk,
    };
    // PostScript `image` with 1 bit: 1 = white (DeviceGray), so the unpacked values stand.
    Ok(Image::from_u8(r.width, r.height, layout, samples)?)
}

/// Opens PostScript / EPS.
pub fn import(name: &str, bytes: &[u8]) -> Result<ImportResult, IoError> {
    if bytes.len() > MAX_SOURCE {
        return Err(bad("file too large"));
    }
    let (ps, tiff, wmf) = if bytes.starts_with(&DOS_EPS_MAGIC) {
        let d = dos_eps(bytes)?;
        (d.ps, d.tiff, d.wmf)
    } else {
        (bytes, None, None)
    };
    let text_head = String::from_utf8_lossy(&ps[..ps.len().min(1 << 16)]).into_owned();
    let bbox = comment_numbers(&text_head, "%%HiResBoundingBox:").or_else(|| comment_numbers(&text_head, "%%BoundingBox:")).filter(|v| v.len() == 4);
    let mut warnings = Vec::new();
    let raster = find_raster(ps);
    let mut r = match raster {
        Ok(ras) => {
            let img = raster_image(ps, &ras)?;
            crate::flat::image_to_document(name, &img)?
        }
        Err(e) => {
            // No raster in the PostScript: the preview stands in for it.
            if let Some(t) = tiff {
                let img = redrob_photo_codecs::decode_as(Format::Tiff, t)?;
                let mut r = crate::flat::image_to_document(name, &img)?;
                r.warnings.push(format!("{e}; the file's TIFF preview was opened instead"));
                r.preview_only = true;
                r
            } else if let Some(w) = wmf {
                let mut r = crate::legacy::import(name, w).ok_or_else(|| bad("the metafile preview was not recognised"))??;
                r.warnings.push(format!("{e}; the file's metafile preview was opened instead"));
                r.preview_only = true;
                r
            } else {
                return Err(e);
            }
        }
    };
    // Resolution from the bounding box (points) against the pixel size.
    if let Some(b) = bbox {
        let wpt = (b[2] - b[0]).abs();
        if wpt > 0.0 {
            let dpi = f64::from(r.document.size.width) * 72.0 / wpt;
            if dpi.is_finite() && (1.0..=10_000.0).contains(&dpi) {
                r.document.resolution_dpi = dpi as f32;
            }
        }
    } else {
        warnings.push("no %%BoundingBox: the resolution was left at the default".into());
    }
    r.warnings.append(&mut warnings);
    r.source_read_only = true;
    Ok(r)
}

/// Export options.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EpsOptions {
    /// Encapsulated (EPSF-3.0, no `showpage`) or a plain PostScript page.
    pub encapsulated: bool,
    /// Sample data encoding.
    pub encoding: EpsEncoding,
    /// A TIFF preview in a DOS EPS binary header (desktop publishing applications show it).
    pub tiff_preview: bool,
    /// PostScript Level (2 or 3); Level 1 writes `colorimage` without filters, hex only.
    pub level: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EpsEncoding {
    Hex,
    Ascii85,
    Binary,
}

impl Default for EpsOptions {
    fn default() -> Self {
        EpsOptions { encapsulated: true, encoding: EpsEncoding::Ascii85, tiff_preview: false, level: 2 }
    }
}

fn ascii85_encode(data: &[u8], out: &mut String) {
    let mut col = 0;
    let put = |s: &str, out: &mut String, col: &mut usize| {
        out.push_str(s);
        *col += s.len();
        if *col >= 76 {
            out.push('\n');
            *col = 0;
        }
    };
    for chunk in data.chunks(4) {
        let mut v: u32 = 0;
        for i in 0..4 {
            v = (v << 8) | u32::from(chunk.get(i).copied().unwrap_or(0));
        }
        if chunk.len() == 4 && v == 0 {
            put("z", out, &mut col);
            continue;
        }
        let mut digits = [0u8; 5];
        for d in digits.iter_mut().rev() {
            *d = (v % 85) as u8 + b'!';
            v /= 85;
        }
        let s: String = digits[..chunk.len() + 1].iter().map(|&b| b as char).collect();
        put(&s, out, &mut col);
    }
    out.push_str("~>\n");
}

/// Writes `doc` as PostScript.
pub fn export(doc: &Document, opts: EpsOptions) -> Result<ExportResult, IoError> {
    let mut warnings = Vec::new();
    let img = crate::flat::document_to_image(doc, &mut warnings)?;
    let (w, h) = img.dimensions();
    let cmyk = matches!(img.layout(), ChannelLayout::Cmyk | ChannelLayout::CmykA);
    let gray = matches!(img.layout(), ChannelLayout::Gray | ChannelLayout::GrayA);
    let (layout, comps) = if cmyk {
        (ChannelLayout::Cmyk, 4)
    } else if gray {
        (ChannelLayout::Gray, 1)
    } else {
        (ChannelLayout::Rgb, 3)
    };
    if img.layout().has_alpha() {
        warnings.push("PostScript has no transparency: the image was matted over white".into());
    }
    let samples = matte_white(&img, layout);
    let dpi = f64::from(doc.resolution_dpi.max(1.0));
    let wpt = f64::from(w) * 72.0 / dpi;
    let hpt = f64::from(h) * 72.0 / dpi;
    let mut s = String::with_capacity(samples.len() * 2 + 1024);
    let stem = doc.name.rsplit_once('.').map_or(doc.name.as_str(), |(a, _)| a);
    let title: String = stem.chars().filter(|c| !c.is_control() && *c != '(' && *c != ')').collect();
    s.push_str(if opts.encapsulated { "%!PS-Adobe-3.0 EPSF-3.0\n" } else { "%!PS-Adobe-3.0\n" });
    s.push_str("%%Creator: Redrob Canvas\n");
    s.push_str(&format!("%%Title: {title}\n"));
    s.push_str(&format!("%%BoundingBox: 0 0 {} {}\n", wpt.ceil() as i64, hpt.ceil() as i64));
    s.push_str(&format!("%%HiResBoundingBox: 0 0 {wpt:.4} {hpt:.4}\n"));
    s.push_str(&format!("%%LanguageLevel: {}\n", opts.level.clamp(1, 3)));
    s.push_str("%%Pages: 1\n%%DocumentData: ");
    s.push_str(if opts.encoding == EpsEncoding::Binary { "Binary\n" } else { "Clean7Bit\n" });
    s.push_str("%%EndComments\n");
    let format_code = match opts.encoding {
        EpsEncoding::Binary if opts.level > 1 => 2,
        _ => 1,
    };
    let op = if comps == 1 { "image" } else { "colorimage" };
    s.push_str(&format!("%ImageData: {w} {h} 8 {comps} 0 {} {format_code} \"{op}\"\n", w as usize * comps));
    s.push_str("%%BeginProlog\n%%EndProlog\n%%Page: 1 1\n");
    s.push_str("gsave\n");
    s.push_str(&format!("0 0 translate\n{wpt:.4} {hpt:.4} scale\n"));
    let space = match comps {
        1 => "/DeviceGray",
        4 => "/DeviceCMYK",
        _ => "/DeviceRGB",
    };
    let level1 = opts.level <= 1;
    if level1 {
        // Level 1: `readhexstring` into a row buffer, hex data only.
        s.push_str(&format!("/picstr {} string def\n", w as usize * comps));
        s.push_str(&format!("{w} {h} 8 [{w} 0 0 -{h} 0 {h}]\n{{currentfile picstr readhexstring pop}}\n"));
        if comps == 1 {
            s.push_str("image\n");
        } else {
            s.push_str(&format!("false {comps} colorimage\n"));
        }
        for row in samples.chunks(w as usize * comps) {
            for b in row {
                s.push_str(&format!("{b:02x}"));
            }
            s.push('\n');
        }
    } else {
        s.push_str(&format!("{space} setcolorspace\n"));
        s.push_str("<<\n  /ImageType 1\n");
        s.push_str(&format!("  /Width {w}\n  /Height {h}\n  /BitsPerComponent 8\n"));
        let decode = match comps {
            1 => "[0 1]",
            4 => "[0 1 0 1 0 1 0 1]",
            _ => "[0 1 0 1 0 1]",
        };
        s.push_str(&format!("  /Decode {decode}\n"));
        s.push_str(&format!("  /ImageMatrix [{w} 0 0 -{h} 0 {h}]\n"));
        let filter = match opts.encoding {
            EpsEncoding::Hex => "currentfile /ASCIIHexDecode filter",
            EpsEncoding::Ascii85 => "currentfile /ASCII85Decode filter",
            EpsEncoding::Binary => "currentfile",
        };
        s.push_str(&format!("  /DataSource {filter}\n>>\nimage\n"));
    }
    let mut bytes = s.into_bytes();
    if !level1 {
        match opts.encoding {
            EpsEncoding::Hex => {
                let mut t = String::with_capacity(samples.len() * 2 + samples.len() / 36);
                for (i, b) in samples.iter().enumerate() {
                    t.push_str(&format!("{b:02x}"));
                    if i % 36 == 35 {
                        t.push('\n');
                    }
                }
                t.push_str(">\n");
                bytes.extend(t.into_bytes());
            }
            EpsEncoding::Ascii85 => {
                let mut t = String::with_capacity(samples.len() * 5 / 4 + 64);
                ascii85_encode(&samples, &mut t);
                bytes.extend(t.into_bytes());
            }
            EpsEncoding::Binary => {
                bytes.extend_from_slice(&samples);
                bytes.push(b'\n');
            }
        }
    }
    let mut tail = String::new();
    tail.push_str("grestore\n");
    if !opts.encapsulated {
        tail.push_str("showpage\n");
    }
    tail.push_str("%%Trailer\n%%EOF\n");
    bytes.extend(tail.into_bytes());
    if opts.tiff_preview {
        // DOS EPS binary: header (30 bytes), PostScript, TIFF preview (at most 256 px wide, RGB).
        let scale = 256.0 / f64::from(w.max(h)).max(256.0);
        let (pw, ph) = (((f64::from(w) * scale).round() as u32).max(1), ((f64::from(h) * scale).round() as u32).max(1));
        let preview = preview_image(&img, pw, ph);
        let tiff = redrob_photo_codecs::encode(&preview, Format::Tiff, &EncodeOptions::default())?;
        let mut out = Vec::with_capacity(30 + bytes.len() + tiff.len());
        out.extend(DOS_EPS_MAGIC);
        out.extend(30u32.to_le_bytes());
        out.extend((bytes.len() as u32).to_le_bytes());
        out.extend(0u32.to_le_bytes());
        out.extend(0u32.to_le_bytes());
        out.extend((30 + bytes.len() as u32).to_le_bytes());
        out.extend((tiff.len() as u32).to_le_bytes());
        out.extend(0xFFFFu16.to_le_bytes());
        out.extend(bytes);
        out.extend(tiff);
        bytes = out;
    }
    Ok(ExportResult { bytes, warnings })
}

/// The image's samples in `layout` with any alpha matted over white, 8 bits.
fn matte_white(img: &Image, layout: ChannelLayout) -> Vec<u8> {
    let rgba = img.to_rgba_f32();
    let n = img.pixel_count();
    let mut out = Vec::with_capacity(n * layout.channels());
    let cmyk = layout == ChannelLayout::Cmyk && matches!(img.layout(), ChannelLayout::Cmyk | ChannelLayout::CmykA);
    if cmyk {
        // Keep the native inks: alpha-matte towards no ink.
        let c = img.convert(ChannelLayout::CmykA, redrob_photo_codecs::SampleType::U8);
        for p in c.data().chunks_exact(5) {
            let a = u32::from(p[4]);
            for &v in &p[..4] {
                out.push((u32::from(v) * a / 255) as u8);
            }
        }
        return out;
    }
    for p in rgba.chunks_exact(4) {
        let a = p[3].clamp(0.0, 1.0);
        let m = |v: f32| ((v * a + (1.0 - a)).clamp(0.0, 1.0) * 255.0).round() as u8;
        match layout {
            ChannelLayout::Gray => out.push(m(0.299 * p[0] + 0.587 * p[1] + 0.114 * p[2])),
            _ => out.extend([m(p[0]), m(p[1]), m(p[2])]),
        }
    }
    out
}

/// A small RGB preview of `img` (nearest neighbour), matted over white.
fn preview_image(img: &Image, pw: u32, ph: u32) -> Image {
    let (w, h) = img.dimensions();
    let rgb = matte_white(img, ChannelLayout::Rgb);
    let mut out = Vec::with_capacity((pw * ph * 3) as usize);
    for y in 0..ph {
        let sy = (u64::from(y) * u64::from(h) / u64::from(ph)).min(u64::from(h) - 1) as usize;
        for x in 0..pw {
            let sx = (u64::from(x) * u64::from(w) / u64::from(pw)).min(u64::from(w) - 1) as usize;
            let i = (sy * w as usize + sx) * 3;
            out.extend_from_slice(&rgb[i..i + 3]);
        }
    }
    Image::from_u8(pw, ph, ChannelLayout::Rgb, out).unwrap_or_else(|_| img.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use redrob_photo_color::{ColorMode, SampleType};
    use redrob_photo_geom::Size;

    fn doc() -> Document {
        let mut d = Document::new("t.eps", Size::new(4, 3), ColorMode::Rgb, SampleType::U8);
        let mut l = redrob_photo_doc::Layer::raster("Background", redrob_photo_color::PixelFormat::RGBA8);
        if let redrob_photo_doc::LayerContent::Raster(s) = &mut l.content {
            let px: Vec<u8> = (0..12u8).flat_map(|i| [i * 20, 255 - i * 20, 7, 255]).collect();
            s.write_interleaved(redrob_photo_geom::Rect::new(0, 0, 4, 3), &px);
        }
        d.layers.push(l);
        d
    }

    #[test]
    fn eps_round_trips_in_every_encoding() {
        for enc in [EpsEncoding::Hex, EpsEncoding::Ascii85, EpsEncoding::Binary] {
            for level in [1u8, 2] {
                let out = export(&doc(), EpsOptions { encoding: enc, level, ..EpsOptions::default() }).unwrap_or_else(|e| panic!("{e}"));
                assert!(is_postscript(&out.bytes));
                let back = import("t.eps", &out.bytes).unwrap_or_else(|e| panic!("{enc:?} L{level}: {e}"));
                assert_eq!((back.document.size.width, back.document.size.height), (4, 3), "{enc:?} L{level}");
                let (_, _, got) = crate::legacy::flatten(&back.document, &mut Vec::new()).unwrap_or_else(|e| panic!("{e}"));
                let want: Vec<u8> = (0..12u8).flat_map(|i| [i * 20, 255 - i * 20, 7, 255]).collect();
                assert_eq!(got, want, "{enc:?} L{level}");
            }
        }
    }

    #[test]
    fn dos_eps_preview_is_written_and_read() {
        let out = export(&doc(), EpsOptions { tiff_preview: true, ..EpsOptions::default() }).unwrap_or_else(|e| panic!("{e}"));
        assert!(out.bytes.starts_with(&DOS_EPS_MAGIC));
        let d = dos_eps(&out.bytes).unwrap_or_else(|e| panic!("{e}"));
        assert!(d.tiff.is_some());
        assert!(d.ps.starts_with(b"%!PS-Adobe-3.0 EPSF-3.0"));
        let back = import("t.eps", &out.bytes).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(back.document.size.width, 4);
    }

    #[test]
    fn ascii85_round_trips() {
        let data: Vec<u8> = (0..1000u32).map(|i| (i * 7 % 251) as u8).collect();
        let mut s = String::new();
        ascii85_encode(&data, &mut s);
        assert_eq!(ascii85_decode(s.as_bytes(), data.len()), data);
    }
}
