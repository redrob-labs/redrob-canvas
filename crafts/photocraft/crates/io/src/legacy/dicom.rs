//! DICOM (`.dcm`): Part 10 files with a 128-byte preamble and `DICM`, an explicit-VR
//! little-endian meta group, and a data set in the implicit or explicit little-endian or explicit
//! big-endian transfer syntax. Uncompressed pixel data at 8 or 16 bits (MONOCHROME1/2, RGB,
//! PALETTE COLOR) is read, windowed with the file's Window Center/Width (or the sample range) and
//! rescaled to 8 bits; the first frame of a multi-frame file opens. Compressed (encapsulated)
//! transfer syntaxes are refused. Writes an 8-bit Secondary Capture image, explicit VR LE.

use super::{Rgba, bad, be16, be32, le16, le32};
use crate::IoError;

const EXPLICIT_LE: &str = "1.2.840.10008.1.2.1";
const IMPLICIT_LE: &str = "1.2.840.10008.1.2";
const EXPLICIT_BE: &str = "1.2.840.10008.1.2.2";
const SECONDARY_CAPTURE: &str = "1.2.840.10008.5.1.4.1.1.7";
const IMPLEMENTATION_UID: &str = "2.25.300417834920157286541339201752640613489";

#[derive(Default)]
struct Attrs {
    rows: u32,
    cols: u32,
    samples: u16,
    photometric: String,
    bits_allocated: u16,
    bits_stored: u16,
    pixel_rep: u16,
    planar: u16,
    frames: u32,
    window: Option<(f64, f64)>,
    rescale: (f64, f64),
    lut_desc: [Option<[u32; 3]>; 3],
    lut: [Option<Vec<u8>>; 3],
    pixel_data: Option<(usize, usize)>,
    transfer: String,
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
    explicit: bool,
    big: bool,
}

impl<'a> Reader<'a> {
    fn u16(&self, at: usize) -> Option<u16> {
        if self.big { be16(self.b, at) } else { le16(self.b, at) }
    }
    fn u32(&self, at: usize) -> Option<u32> {
        if self.big { be32(self.b, at) } else { le32(self.b, at) }
    }

    /// Reads one element header: (group, element, VR, value length, header length).
    fn header(&self) -> Option<(u16, u16, [u8; 2], u32, usize)> {
        let group = self.u16(self.at)?;
        let elem = self.u16(self.at + 2)?;
        if !self.explicit || group == 0xFFFE {
            return Some((group, elem, *b"--", self.u32(self.at + 4)?, 8));
        }
        let vr = [*self.b.get(self.at + 4)?, *self.b.get(self.at + 5)?];
        let long = matches!(&vr, b"OB" | b"OW" | b"OF" | b"OD" | b"OL" | b"OV" | b"SQ" | b"UT" | b"UN" | b"UC" | b"UR" | b"SV" | b"UV");
        if long { Some((group, elem, vr, self.u32(self.at + 8)?, 12)) } else { Some((group, elem, vr, u32::from(self.u16(self.at + 6)?), 8)) }
    }
}

fn text(v: &[u8]) -> String {
    String::from_utf8_lossy(v).trim_matches(|c: char| c == '\0' || c.is_whitespace()).to_string()
}

fn number(v: &[u8]) -> Option<f64> {
    text(v).split('\\').next().and_then(|s| s.trim().parse().ok())
}

fn us(r: &Reader, v: &[u8]) -> u16 {
    if r.big { be16(v, 0).unwrap_or(0) } else { le16(v, 0).unwrap_or(0) }
}

/// Walks a data set (or a sequence item) collecting the attributes we use.
fn walk(r: &mut Reader, end: usize, a: &mut Attrs, depth: usize) -> Result<(), IoError> {
    if depth > 16 {
        return Err(bad("DICOM", "sequences nested too deeply"));
    }
    while r.at + 8 <= end.min(r.b.len()) {
        let (group, elem, vr, len, hl) = r.header().ok_or_else(|| bad("DICOM", "element header ends early"))?;
        r.at += hl;
        let undefined = len == 0xFFFF_FFFF;
        if group == 0xFFFE {
            // Item / delimiters: descend into items, finish on an end delimiter.
            match elem {
                0xE000 => {
                    let item_end = if undefined { end } else { r.at.saturating_add(len as usize) };
                    walk(r, item_end.min(end), a, depth + 1)?;
                    continue;
                }
                _ => return Ok(()),
            }
        }
        if &vr == b"SQ" || (undefined && (group, elem) != (0x7FE0, 0x0010)) {
            let seq_end = if undefined { end } else { r.at.saturating_add(len as usize) };
            // Sequence contents are irrelevant here except to find their end.
            let mut dummy = Attrs::default();
            walk(r, seq_end.min(end), &mut dummy, depth + 1)?;
            if !undefined {
                r.at = seq_end;
            }
            continue;
        }
        let value_at = r.at;
        if (group, elem) == (0x7FE0, 0x0010) {
            if undefined {
                return Err(bad("DICOM", format!("encapsulated (compressed) pixel data is not supported (transfer syntax {})", a.transfer)));
            }
            a.pixel_data = Some((value_at, len as usize));
            return Ok(());
        }
        let v = r.b.get(value_at..value_at.saturating_add(len as usize)).ok_or_else(|| bad("DICOM", "element value ends early"))?;
        match (group, elem) {
            (0x0002, 0x0010) => a.transfer = text(v),
            (0x0028, 0x0010) => a.rows = u32::from(us(r, v)),
            (0x0028, 0x0011) => a.cols = u32::from(us(r, v)),
            (0x0028, 0x0002) => a.samples = us(r, v),
            (0x0028, 0x0004) => a.photometric = text(v).to_ascii_uppercase(),
            (0x0028, 0x0006) => a.planar = us(r, v),
            (0x0028, 0x0008) => a.frames = number(v).unwrap_or(1.0) as u32,
            (0x0028, 0x0100) => a.bits_allocated = us(r, v),
            (0x0028, 0x0101) => a.bits_stored = us(r, v),
            (0x0028, 0x0103) => a.pixel_rep = us(r, v),
            (0x0028, 0x1050) => {
                if let Some(c) = number(v) {
                    a.window = Some((c, a.window.map_or(0.0, |w| w.1)));
                }
            }
            (0x0028, 0x1051) => {
                if let Some(w) = number(v) {
                    a.window = Some((a.window.map_or(0.0, |c| c.0), w));
                }
            }
            (0x0028, 0x1052) => a.rescale.1 = number(v).unwrap_or(0.0),
            (0x0028, 0x1053) => a.rescale.0 = number(v).unwrap_or(1.0),
            (0x0028, 0x1101..=0x1103) => {
                let d = [u32::from(us(r, v)), u32::from(us(r, &v[2.min(v.len())..])), u32::from(us(r, &v[4.min(v.len())..]))];
                a.lut_desc[(elem - 0x1101) as usize] = Some(d);
            }
            (0x0028, 0x1201..=0x1203) => a.lut[(elem - 0x1201) as usize] = Some(v.to_vec()),
            _ => {}
        }
        r.at = value_at.saturating_add(len as usize);
    }
    Ok(())
}

pub fn decode(bytes: &[u8]) -> Result<Rgba, IoError> {
    if bytes.len() < 132 || &bytes[128..132] != b"DICM" {
        return Err(bad("DICOM", "no DICM marker (raw data sets without a Part 10 header are not supported)"));
    }
    let mut a = Attrs { rescale: (1.0, 0.0), samples: 1, frames: 1, bits_allocated: 8, ..Default::default() };
    a.bits_stored = 0;
    // Meta group: always explicit VR little endian; its length is element (0002,0000).
    let mut r = Reader { b: bytes, at: 132, explicit: true, big: false };
    let (g, e, _, len, hl) = r.header().ok_or_else(|| bad("DICOM", "meta header ends early"))?;
    let meta_end = if (g, e) == (0x0002, 0x0000) { 132 + hl + 4 + le32(bytes, 132 + hl).unwrap_or(0) as usize } else { bytes.len() };
    let _ = len;
    walk(&mut r, meta_end.min(bytes.len()), &mut a, 0)?;
    r.at = meta_end.min(bytes.len());
    match a.transfer.as_str() {
        EXPLICIT_LE | "" => {}
        IMPLICIT_LE => r.explicit = false,
        EXPLICIT_BE => r.big = true,
        other => return Err(bad("DICOM", format!("transfer syntax {other} is compressed or unsupported; only uncompressed little/big endian files open"))),
    }
    walk(&mut r, bytes.len(), &mut a, 0)?;
    let (px_at, px_len) = a.pixel_data.ok_or_else(|| bad("DICOM", "no pixel data"))?;
    let (w, h) = (a.cols, a.rows);
    let mut out = Rgba::new(w, h)?;
    let (wu, hu) = (w as usize, h as usize);
    let n = wu * hu;
    let bpsample = match a.bits_allocated {
        8 => 1,
        16 => 2,
        b => return Err(bad("DICOM", format!("{b} bits allocated is not supported"))),
    };
    let samples = a.samples.max(1) as usize;
    let data = bytes.get(px_at..px_at + px_len.min(bytes.len() - px_at)).unwrap_or(&[]);
    if data.len() < n * samples * bpsample {
        return Err(bad("DICOM", "pixel data ends early"));
    }
    let signed = a.pixel_rep == 1;
    let bits = if a.bits_stored == 0 { a.bits_allocated } else { a.bits_stored.min(a.bits_allocated) };
    let raw = |i: usize| -> f64 {
        if bpsample == 1 {
            let v = data[i];
            if signed { f64::from(v as i8) } else { f64::from(v) }
        } else {
            let v = if r.big { be16(data, i * 2).unwrap_or(0) } else { le16(data, i * 2).unwrap_or(0) };
            let v = v & (((1u32 << bits) - 1) as u16);
            if signed {
                let shift = 16 - bits;
                f64::from(((v << shift) as i16) >> shift)
            } else {
                f64::from(v)
            }
        }
    };
    if samples == 3 {
        for i in 0..n {
            let (r0, g0, b0) = if a.planar == 1 { (raw(i), raw(n + i), raw(2 * n + i)) } else { (raw(i * 3), raw(i * 3 + 1), raw(i * 3 + 2)) };
            let max = if bpsample == 1 { 255.0 } else { f64::from((1u32 << bits) - 1) };
            let c = |v: f64| (v / max * 255.0).round().clamp(0.0, 255.0) as u8;
            out.data[i * 4..i * 4 + 4].copy_from_slice(&[c(r0), c(g0), c(b0), 255]);
        }
        if a.photometric.starts_with("YBR") {
            out.warnings.push("YBR colour was read as RGB".into());
        }
    } else if a.photometric == "PALETTE COLOR" {
        let lut = |k: usize, v: f64| -> u8 {
            let Some(d) = a.lut_desc[k] else { return 0 };
            let Some(t) = a.lut[k].as_ref() else { return 0 };
            let entries = if d[0] == 0 { 65_536 } else { d[0] as usize };
            let first = d[1] as usize;
            let depth = d[2];
            let idx = ((v as isize) - first as isize).clamp(0, entries as isize - 1) as usize;
            if depth > 8 { (le16(t, idx * 2).unwrap_or(0) >> 8) as u8 } else { t.get(idx).copied().unwrap_or(0) }
        };
        for i in 0..n {
            let v = raw(i);
            out.data[i * 4..i * 4 + 4].copy_from_slice(&[lut(0, v), lut(1, v), lut(2, v), 255]);
        }
    } else {
        // Modality rescale, then the window (file's, else the frame's range).
        let (slope, intercept) = a.rescale;
        let mut values: Vec<f64> = (0..n).map(|i| raw(i) * slope + intercept).collect();
        let (center, width) = match a.window {
            Some((c, w)) if w > 0.0 => (c, w),
            // 8-bit samples without a window are shown as stored (centre 128, width 256).
            _ if bpsample == 1 && slope == 1.0 && intercept == 0.0 => (128.0, 256.0),
            _ => {
                let lo = values.iter().copied().fold(f64::INFINITY, f64::min);
                let hi = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                ((lo + hi) / 2.0, (hi - lo).max(1.0))
            }
        };
        let lo = center - 0.5 - (width - 1.0) / 2.0;
        let hi = center - 0.5 + (width - 1.0) / 2.0;
        let invert = a.photometric == "MONOCHROME1";
        for (i, v) in values.iter_mut().enumerate() {
            let mut g = if *v <= lo {
                0.0
            } else if *v > hi {
                255.0
            } else {
                ((*v - (center - 0.5)) / (width - 1.0) + 0.5) * 255.0
            };
            if invert {
                g = 255.0 - g;
            }
            let g = g.round().clamp(0.0, 255.0) as u8;
            out.data[i * 4..i * 4 + 4].copy_from_slice(&[g, g, g, 255]);
        }
        if bpsample == 2 {
            out.warnings.push(format!("{bits}-bit samples were windowed to 8 bits (center {center}, width {width})"));
        }
    }
    if a.frames > 1 {
        out.warnings.push(format!("multi-frame DICOM: {} frames; the first opened", a.frames));
    }
    Ok(out)
}

fn element(out: &mut Vec<u8>, group: u16, elem: u16, vr: &[u8; 2], mut value: Vec<u8>) {
    let pad = if matches!(vr, b"UI" | b"OB") { 0u8 } else { b' ' };
    if value.len() & 1 == 1 {
        value.push(pad);
    }
    out.extend(group.to_le_bytes());
    out.extend(elem.to_le_bytes());
    out.extend_from_slice(vr);
    if matches!(vr, b"OB" | b"OW" | b"SQ" | b"UN" | b"UT") {
        out.extend([0, 0]);
        out.extend((value.len() as u32).to_le_bytes());
    } else {
        out.extend((value.len() as u16).to_le_bytes());
    }
    out.extend(value);
}

fn uid(seed: &[u8]) -> String {
    // A UUID-derived UID (2.25.<decimal 128-bit>), hashed from the content.
    let h = blake3::hash(seed);
    let b = h.as_bytes();
    let v = u128::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]]);
    format!("2.25.{v}")
}

/// An 8-bit Secondary Capture image: MONOCHROME2 for `grey`, else RGB (interleaved).
pub fn encode(w: u32, h: u32, rgba: &[u8], grey: bool, warnings: &mut Vec<String>) -> Result<Vec<u8>, IoError> {
    if w > 65535 || h > 65535 {
        return Err(IoError::Unsupported("DICOM images are at most 65535 pixels on a side".into()));
    }
    if rgba.chunks_exact(4).any(|p| p[3] != 255) {
        warnings.push("DICOM has no alpha channel; transparency was dropped".into());
    }
    let pixels: Vec<u8> = if grey { rgba.chunks_exact(4).map(super::luma).collect() } else { rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect() };
    let instance = uid(&pixels);
    let study = uid(instance.as_bytes());
    let series = uid(study.as_bytes());
    let mut meta = Vec::new();
    element(&mut meta, 0x0002, 0x0001, b"OB", vec![0, 1]);
    element(&mut meta, 0x0002, 0x0002, b"UI", SECONDARY_CAPTURE.into());
    element(&mut meta, 0x0002, 0x0003, b"UI", instance.clone().into());
    element(&mut meta, 0x0002, 0x0010, b"UI", EXPLICIT_LE.into());
    element(&mut meta, 0x0002, 0x0012, b"UI", IMPLEMENTATION_UID.into());
    let mut out = vec![0u8; 128];
    out.extend_from_slice(b"DICM");
    element(&mut out, 0x0002, 0x0000, b"UL", (meta.len() as u32).to_le_bytes().to_vec());
    out.extend(meta);
    element(&mut out, 0x0008, 0x0016, b"UI", SECONDARY_CAPTURE.into());
    element(&mut out, 0x0008, 0x0018, b"UI", instance.into());
    element(&mut out, 0x0008, 0x0060, b"CS", b"OT".to_vec());
    element(&mut out, 0x0008, 0x0064, b"CS", b"WSD".to_vec());
    element(&mut out, 0x0010, 0x0010, b"PN", Vec::new());
    element(&mut out, 0x0010, 0x0020, b"LO", Vec::new());
    element(&mut out, 0x0020, 0x000D, b"UI", study.into());
    element(&mut out, 0x0020, 0x000E, b"UI", series.into());
    element(&mut out, 0x0020, 0x0011, b"IS", b"1".to_vec());
    element(&mut out, 0x0020, 0x0013, b"IS", b"1".to_vec());
    element(&mut out, 0x0028, 0x0002, b"US", (if grey { 1u16 } else { 3u16 }).to_le_bytes().to_vec());
    element(&mut out, 0x0028, 0x0004, b"CS", (if grey { "MONOCHROME2" } else { "RGB" }).into());
    if !grey {
        element(&mut out, 0x0028, 0x0006, b"US", 0u16.to_le_bytes().to_vec());
    }
    element(&mut out, 0x0028, 0x0010, b"US", (h as u16).to_le_bytes().to_vec());
    element(&mut out, 0x0028, 0x0011, b"US", (w as u16).to_le_bytes().to_vec());
    element(&mut out, 0x0028, 0x0100, b"US", 8u16.to_le_bytes().to_vec());
    element(&mut out, 0x0028, 0x0101, b"US", 8u16.to_le_bytes().to_vec());
    element(&mut out, 0x0028, 0x0102, b"US", 7u16.to_le_bytes().to_vec());
    element(&mut out, 0x0028, 0x0103, b"US", 0u16.to_le_bytes().to_vec());
    element(&mut out, 0x7FE0, 0x0010, b"OB", pixels);
    Ok(out)
}
