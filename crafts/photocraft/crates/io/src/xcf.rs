//! XCF, the layered native image format, read and written from its public specification (the
//! XCF file format document). Read: versions 0 to 12 (big-endian, 32-bit
//! offsets before version 11 and 64-bit from it), RGB / grey / indexed images at 8, 16 and 32-bit
//! integer and 16/32/64-bit float precision, uncompressed, RLE and zlib tiles, layer groups
//! (item paths), layer masks, channels, offsets, opacity, visibility, blend modes (legacy and
//! default sets), locks, the colour map, resolution, guides, parasites (comment, ICC profile) and
//! vector paths. Write: version 11 (RLE, 64-bit offsets) with the same model, so a file written
//! here opens with every layer, group, mask, channel and path.

use std::sync::Arc;

use redrob_photo_color::{BlendMode, ColorMode, PixelFormat, SampleType};
use redrob_photo_doc::vector::{Knot, Path, Subpath};
use redrob_photo_doc::{AlphaChannel, Document, Layer, LayerContent, LayerMask, NamedPath};
use redrob_photo_geom::{Point, Rect, Size};
use redrob_photo_raster::Surface;

use crate::{ExportResult, ImportResult, IoError};

const MAGIC: &[u8; 9] = b"gimp xcf ";
const TILE: u32 = 64;
const MAX_SIDE: u32 = 262_144;
const MAX_PIXEL_BYTES: u64 = 1 << 30;
const MAX_LAYERS: usize = 65_536;

// Image and item properties.
const PROP_END: u32 = 0;
const PROP_COLORMAP: u32 = 1;
const PROP_ACTIVE_LAYER: u32 = 2;
const PROP_OPACITY: u32 = 6;
const PROP_MODE: u32 = 7;
const PROP_VISIBLE: u32 = 8;
const PROP_LINKED: u32 = 9;
const PROP_LOCK_ALPHA: u32 = 10;
const PROP_APPLY_MASK: u32 = 11;
const PROP_EDIT_MASK: u32 = 12;
const PROP_SHOW_MASK: u32 = 13;
const PROP_SELECTION: u32 = 14;
const PROP_OFFSETS: u32 = 15;
const PROP_COLOR: u32 = 16;
const PROP_COMPRESSION: u32 = 17;
const PROP_GUIDES: u32 = 18;
const PROP_RESOLUTION: u32 = 19;
const PROP_PARASITES: u32 = 21;
const PROP_VECTORS: u32 = 25;
const PROP_LOCK_CONTENT: u32 = 28;
const PROP_GROUP_ITEM: u32 = 29;
const PROP_ITEM_PATH: u32 = 30;
const PROP_GROUP_ITEM_FLAGS: u32 = 31;
const PROP_LOCK_POSITION: u32 = 32;
const PROP_FLOAT_OPACITY: u32 = 33;
const PROP_FLOAT_COLOR: u32 = 38;

fn bad(msg: impl std::fmt::Display) -> IoError {
    IoError::Unsupported(format!("XCF: {msg}"))
}

/// Is `bytes` an XCF file?
pub fn is_xcf(bytes: &[u8]) -> bool {
    bytes.starts_with(MAGIC)
}

// ---------------------------------------------------------------------------------------------
// Reading

struct Rd<'a> {
    b: &'a [u8],
    at: usize,
    /// 64-bit offsets (version 11 and later).
    wide: bool,
}

impl<'a> Rd<'a> {
    fn u8(&mut self) -> Result<u8, IoError> {
        let v = *self.b.get(self.at).ok_or_else(|| bad("file ends early"))?;
        self.at += 1;
        Ok(v)
    }
    fn u32(&mut self) -> Result<u32, IoError> {
        let s = self.b.get(self.at..self.at + 4).ok_or_else(|| bad("file ends early"))?;
        self.at += 4;
        Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn i32(&mut self) -> Result<i32, IoError> {
        self.u32().map(|v| v as i32)
    }
    fn f32(&mut self) -> Result<f32, IoError> {
        self.u32().map(f32::from_bits)
    }
    fn u64(&mut self) -> Result<u64, IoError> {
        let s = self.b.get(self.at..self.at + 8).ok_or_else(|| bad("file ends early"))?;
        self.at += 8;
        Ok(u64::from_be_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]))
    }
    fn ptr(&mut self) -> Result<usize, IoError> {
        let v = if self.wide { self.u64()? } else { u64::from(self.u32()?) };
        usize::try_from(v).map_err(|_| bad("offset out of range"))
    }
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], IoError> {
        let s = self.b.get(self.at..self.at.checked_add(n).ok_or_else(|| bad("length overflow"))?).ok_or_else(|| bad("file ends early"))?;
        self.at += n;
        Ok(s)
    }
    /// A string: u32 length including the NUL, then the bytes.
    fn string(&mut self) -> Result<String, IoError> {
        let n = self.u32()? as usize;
        if n == 0 {
            return Ok(String::new());
        }
        if n > 1 << 20 {
            return Err(bad("string too long"));
        }
        let s = self.bytes(n)?;
        let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
        Ok(String::from_utf8_lossy(&s[..end]).into_owned())
    }
    fn seek(&self, at: usize) -> Result<Rd<'a>, IoError> {
        if at >= self.b.len() {
            return Err(bad("offset past the end of the file"));
        }
        Ok(Rd { b: self.b, at, wide: self.wide })
    }
}

/// Sample storage of the image.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Prec {
    U8,
    U16,
    U32,
    F16,
    F32,
    F64,
}

impl Prec {
    fn bytes(self) -> usize {
        match self {
            Prec::U8 => 1,
            Prec::U16 | Prec::F16 => 2,
            Prec::U32 | Prec::F32 => 4,
            Prec::F64 => 8,
        }
    }
    /// A sample at `at` of a big-endian buffer, normalised to 0..1 (floats as stored).
    fn read(self, b: &[u8], at: usize) -> f32 {
        let g = |i: usize| b.get(at + i).copied().unwrap_or(0);
        match self {
            Prec::U8 => f32::from(g(0)) / 255.0,
            Prec::U16 => f32::from(u16::from_be_bytes([g(0), g(1)])) / 65535.0,
            Prec::U32 => (u32::from_be_bytes([g(0), g(1), g(2), g(3)]) as f64 / 4_294_967_295.0) as f32,
            Prec::F16 => f32::from(redrob_photo_codecs::f16::from_bits(u16::from_be_bytes([g(0), g(1)]))),
            Prec::F32 => f32::from_bits(u32::from_be_bytes([g(0), g(1), g(2), g(3)])),
            Prec::F64 => f64::from_be_bytes([g(0), g(1), g(2), g(3), g(4), g(5), g(6), g(7)]) as f32,
        }
    }
}

fn precision(version: u32, code: u32) -> Result<(Prec, bool), IoError> {
    // (precision, linear). Versions 4..=6 used a short enumeration; 7 and later the 100-step one.
    Ok(match (version, code) {
        (0..=3, _) => (Prec::U8, false),
        (4, 0) | (5 | 6, 0 | 1) => (Prec::U8, code == 0 && version >= 5),
        (4, 1) | (5 | 6, 2 | 3) => (Prec::U16, code == 2),
        (4, 2) | (5 | 6, 4 | 5) => (Prec::U32, code == 4),
        (4, 3) | (5 | 6, 6 | 7) => (Prec::F16, code == 6),
        (4, 4) | (5 | 6, 8 | 9) => (Prec::F32, code == 8),
        (_, 100 | 150) => (Prec::U8, code == 100),
        (_, 200 | 250) => (Prec::U16, code == 200),
        (_, 300 | 350) => (Prec::U32, code == 300),
        (_, 500 | 550) => (Prec::F16, code == 500),
        (_, 600 | 650) => (Prec::F32, code == 600),
        (_, 700 | 750) => (Prec::F64, code == 700),
        (_, c) => return Err(bad(format!("unknown precision {c}"))),
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Compression {
    None,
    Rle,
    Zlib,
}

struct Image {
    prec: Prec,
    compression: Compression,
    colormap: Vec<[u8; 3]>,
    dpi: Option<(f32, f32)>,
    comment: Option<String>,
    icc: Option<Vec<u8>>,
    guides: Vec<(i32, bool)>,
    paths: Vec<NamedPath>,
    warnings: Vec<String>,
}

struct Props {
    opacity: f32,
    mode: Option<u32>,
    visible: bool,
    offsets: (i32, i32),
    lock_alpha: bool,
    lock_content: bool,
    lock_position: bool,
    apply_mask: bool,
    group: bool,
    item_path: Option<Vec<u32>>,
    color: Option<[f32; 3]>,
    selection: bool,
    active: bool,
}

impl Default for Props {
    fn default() -> Self {
        Props {
            opacity: 1.0,
            mode: None,
            visible: true,
            offsets: (0, 0),
            lock_alpha: false,
            lock_content: false,
            lock_position: false,
            apply_mask: true,
            group: false,
            item_path: None,
            color: None,
            selection: false,
            active: false,
        }
    }
}

/// Reads an item's property list (layers and channels share the vocabulary).
fn item_props(r: &mut Rd) -> Result<Props, IoError> {
    let mut p = Props::default();
    loop {
        let ty = r.u32()?;
        let len = r.u32()? as usize;
        let start = r.at;
        match ty {
            PROP_END => break,
            PROP_ACTIVE_LAYER => p.active = true,
            PROP_OPACITY => p.opacity = r.u32()? as f32 / 255.0,
            PROP_FLOAT_OPACITY => p.opacity = r.f32()?,
            PROP_MODE => p.mode = Some(r.u32()?),
            PROP_VISIBLE => p.visible = r.u32()? != 0,
            PROP_OFFSETS => p.offsets = (r.i32()?, r.i32()?),
            PROP_LOCK_ALPHA => p.lock_alpha = r.u32()? != 0,
            PROP_LOCK_CONTENT => p.lock_content = r.u32()? != 0,
            PROP_LOCK_POSITION => p.lock_position = r.u32()? != 0,
            PROP_APPLY_MASK => p.apply_mask = r.u32()? != 0,
            PROP_GROUP_ITEM => p.group = true,
            PROP_ITEM_PATH => {
                let n = len / 4;
                if n > 64 {
                    return Err(bad("item path too deep"));
                }
                p.item_path = Some((0..n).map(|_| r.u32()).collect::<Result<_, _>>()?);
            }
            PROP_COLOR => {
                let c = r.bytes(3)?;
                p.color = Some([f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0]);
            }
            PROP_FLOAT_COLOR => p.color = Some([r.f32()?, r.f32()?, r.f32()?]),
            PROP_SELECTION => p.selection = true,
            PROP_LINKED | PROP_EDIT_MASK | PROP_SHOW_MASK | PROP_GROUP_ITEM_FLAGS => {}
            _ => {}
        }
        // Properties are length-delimited: skip what the arm did not consume.
        if len > 1 << 26 {
            return Err(bad("property too large"));
        }
        r.at = start.checked_add(len).ok_or_else(|| bad("property length overflow"))?;
    }
    Ok(p)
}

fn image_props(r: &mut Rd, img: &mut Image, version: u32) -> Result<(), IoError> {
    loop {
        let ty = r.u32()?;
        let len = r.u32()? as usize;
        let start = r.at;
        if len > 1 << 28 {
            return Err(bad("property too large"));
        }
        match ty {
            PROP_END => break,
            PROP_COLORMAP => {
                let n = r.u32()? as usize;
                if n > 256 {
                    return Err(bad("colour map has more than 256 entries"));
                }
                if version == 0 {
                    // Version 0 stored indices instead of colours: a grey ramp stands in.
                    img.colormap = (0..n).map(|i| [i as u8; 3]).collect();
                    img.warnings.push("version 0 colour map was replaced by a grey ramp".into());
                } else {
                    let c = r.bytes(n * 3)?;
                    img.colormap = c.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
                }
            }
            PROP_COMPRESSION => {
                img.compression = match r.u8()? {
                    0 => Compression::None,
                    1 => Compression::Rle,
                    2 => Compression::Zlib,
                    c => return Err(bad(format!("compression {c} is not supported"))),
                }
            }
            PROP_RESOLUTION => {
                let x = r.f32()?;
                let y = r.f32()?;
                if x.is_finite() && y.is_finite() && x > 0.0 && y > 0.0 {
                    img.dpi = Some((x, y));
                }
            }
            PROP_GUIDES => {
                let n = len / 5;
                for _ in 0..n.min(4096) {
                    let c = r.i32()?;
                    let o = r.u8()?;
                    // 1 = horizontal, 2 = vertical.
                    img.guides.push((c, o == 2));
                }
            }
            PROP_PARASITES => {
                let end = start + len;
                while r.at + 12 <= end {
                    let name = r.string()?;
                    let _flags = r.u32()?;
                    let size = r.u32()? as usize;
                    let data = r.bytes(size)?;
                    match name.as_str() {
                        "gimp-comment" => img.comment = Some(String::from_utf8_lossy(data).trim_end_matches('\0').to_string()),
                        "icc-profile" => img.icc = Some(data.to_vec()),
                        _ => {}
                    }
                }
            }
            PROP_VECTORS => {
                let mut sub = Rd { b: r.b, at: r.at, wide: r.wide };
                match read_vectors(&mut sub, start + len) {
                    Ok(paths) => img.paths = paths,
                    Err(e) => img.warnings.push(format!("paths were not read: {e}")),
                }
            }
            _ => {}
        }
        r.at = start.checked_add(len).ok_or_else(|| bad("property length overflow"))?;
    }
    Ok(())
}

/// PROP_VECTORS: the document's paths (bezier strokes only).
fn read_vectors(r: &mut Rd, end: usize) -> Result<Vec<NamedPath>, IoError> {
    let version = r.u32()?;
    if version != 1 {
        return Err(bad(format!("vectors version {version}")));
    }
    let _active = r.u32()?;
    let n = r.u32()? as usize;
    if n > 4096 {
        return Err(bad("too many paths"));
    }
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        if r.at >= end {
            break;
        }
        let name = r.string()?;
        let _tattoo = r.u32()?;
        let _visible = r.u32()?;
        let _linked = r.u32()?;
        let parasites = r.u32()? as usize;
        let strokes = r.u32()? as usize;
        for _ in 0..parasites.min(256) {
            let _pname = r.string()?;
            let _flags = r.u32()?;
            let size = r.u32()? as usize;
            r.bytes(size)?;
        }
        let mut path = Path::new(Vec::new());
        for _ in 0..strokes.min(65_536) {
            let ty = r.u32()?;
            let closed = r.u32()? != 0;
            let axes = r.u32()? as usize;
            let points = r.u32()? as usize;
            if !(2..=6).contains(&axes) || points > 1 << 20 {
                return Err(bad("bad stroke"));
            }
            let mut pts: Vec<(u32, f64, f64)> = Vec::with_capacity(points);
            for _ in 0..points {
                let kind = r.u32()?;
                let x = f64::from(r.f32()?);
                let y = f64::from(r.f32()?);
                for _ in 2..axes {
                    r.f32()?;
                }
                pts.push((kind, x, y));
            }
            if ty != 1 {
                continue;
            }
            path.subpaths.push(bezier_stroke(&pts, closed));
        }
        if !path.subpaths.is_empty() {
            out.push(NamedPath { name, path, psd_raw: None });
        }
    }
    Ok(out)
}

/// A bezier stroke's control points (0 = anchor, 1 = control) as knots: each anchor takes the
/// control before it as its in handle and the one after as its out handle.
fn bezier_stroke(pts: &[(u32, f64, f64)], closed: bool) -> Subpath {
    let anchors: Vec<usize> = pts.iter().enumerate().filter(|(_, p)| p.0 == 0).map(|(i, _)| i).collect();
    let mut knots = Vec::with_capacity(anchors.len());
    for (k, &i) in anchors.iter().enumerate() {
        let a = Point::new(pts[i].1, pts[i].2);
        let before = if i > 0 && pts[i - 1].0 == 1 {
            Some(i - 1)
        } else if k == 0 && closed && pts.last().is_some_and(|p| p.0 == 1) {
            Some(pts.len() - 1)
        } else {
            None
        };
        let after = if i + 1 < pts.len() && pts[i + 1].0 == 1 { Some(i + 1) } else { None };
        let in_ctrl = before.map_or(a, |j| Point::new(pts[j].1, pts[j].2));
        let out_ctrl = after.map_or(a, |j| Point::new(pts[j].1, pts[j].2));
        knots.push(Knot { anchor: a, in_ctrl, out_ctrl, smooth: false });
    }
    Subpath { knots, closed, ..Default::default() }
}

/// RLE-decodes one byte stream of `n` bytes.
fn rle(src: &[u8], at: &mut usize, n: usize, out: &mut Vec<u8>) -> Result<(), IoError> {
    let start = out.len();
    while out.len() - start < n {
        let op = *src.get(*at).ok_or_else(|| bad("tile data ends early"))?;
        *at += 1;
        let remaining = n - (out.len() - start);
        match op {
            0..=126 => {
                let len = op as usize + 1;
                let v = *src.get(*at).ok_or_else(|| bad("tile data ends early"))?;
                *at += 1;
                out.extend(std::iter::repeat_n(v, len.min(remaining)));
            }
            127 => {
                let hi = *src.get(*at).ok_or_else(|| bad("tile data ends early"))?;
                let lo = *src.get(*at + 1).ok_or_else(|| bad("tile data ends early"))?;
                let v = *src.get(*at + 2).ok_or_else(|| bad("tile data ends early"))?;
                *at += 3;
                let len = usize::from(u16::from_be_bytes([hi, lo]));
                out.extend(std::iter::repeat_n(v, len.min(remaining)));
            }
            128 => {
                let hi = *src.get(*at).ok_or_else(|| bad("tile data ends early"))?;
                let lo = *src.get(*at + 1).ok_or_else(|| bad("tile data ends early"))?;
                *at += 2;
                let len = usize::from(u16::from_be_bytes([hi, lo]));
                let s = src.get(*at..*at + len).ok_or_else(|| bad("tile data ends early"))?;
                *at += len;
                out.extend_from_slice(&s[..len.min(remaining)]);
            }
            _ => {
                let len = 256 - op as usize;
                let s = src.get(*at..*at + len).ok_or_else(|| bad("tile data ends early"))?;
                *at += len;
                out.extend_from_slice(&s[..len.min(remaining)]);
            }
        }
    }
    Ok(())
}

/// Decodes one tile into pixel-interleaved bytes (`bpp` bytes per pixel, `n` pixels).
fn tile_pixels(data: &[u8], compression: Compression, bpp: usize, n: usize) -> Result<Vec<u8>, IoError> {
    match compression {
        Compression::None => {
            let s = data.get(..n * bpp).ok_or_else(|| bad("tile data ends early"))?;
            Ok(s.to_vec())
        }
        Compression::Rle => {
            // Each byte of the pixel is its own stream, in byte order.
            let mut planes = Vec::with_capacity(n * bpp);
            let mut at = 0usize;
            for _ in 0..bpp {
                rle(data, &mut at, n, &mut planes)?;
            }
            let mut out = vec![0u8; n * bpp];
            for b in 0..bpp {
                for i in 0..n {
                    out[i * bpp + b] = planes[b * n + i];
                }
            }
            Ok(out)
        }
        Compression::Zlib => {
            use std::io::Read;
            let mut out = Vec::with_capacity(n * bpp);
            let mut dec = flate2::read::ZlibDecoder::new(data).take((n * bpp) as u64);
            dec.read_to_end(&mut out).map_err(|e| bad(format!("zlib tile: {e}")))?;
            if out.len() < n * bpp {
                return Err(bad("zlib tile is short"));
            }
            Ok(out)
        }
    }
}

/// Pixels of a hierarchy: `(width, height, bytes per pixel, interleaved big-endian samples)`.
fn read_hierarchy(r: &Rd, at: usize, compression: Compression) -> Result<(u32, u32, usize, Vec<u8>), IoError> {
    let mut h = r.seek(at)?;
    let w = h.u32()?;
    let hgt = h.u32()?;
    let bpp = h.u32()? as usize;
    if w == 0 || hgt == 0 || w > MAX_SIDE || hgt > MAX_SIDE || bpp == 0 || bpp > 32 {
        return Err(bad(format!("bad hierarchy {w} × {hgt} at {bpp} bytes per pixel")));
    }
    let total = u64::from(w) * u64::from(hgt) * bpp as u64;
    if total > MAX_PIXEL_BYTES {
        return Err(bad("drawable exceeds the 1 GiB pixel limit"));
    }
    let level_at = h.ptr()?;
    let mut lv = r.seek(level_at)?;
    let lw = lv.u32()?;
    let lh = lv.u32()?;
    if (lw, lh) != (w, hgt) {
        return Err(bad("level size differs from the hierarchy"));
    }
    let tiles_x = w.div_ceil(TILE) as usize;
    let tiles_y = hgt.div_ceil(TILE) as usize;
    let mut offsets = Vec::with_capacity(tiles_x * tiles_y);
    for _ in 0..tiles_x * tiles_y {
        let o = lv.ptr()?;
        if o == 0 {
            return Err(bad("tile list ends early"));
        }
        offsets.push(o);
    }
    let mut out = crate::flat::try_buffer((w as usize) * (hgt as usize), bpp)?;
    for (ti, &o) in offsets.iter().enumerate() {
        let tx = (ti % tiles_x) as u32;
        let ty = (ti / tiles_x) as u32;
        let tw = (w - tx * TILE).min(TILE) as usize;
        let th = (hgt - ty * TILE).min(TILE) as usize;
        // Tile data runs to the next tile's offset (or the end of the file).
        let next = offsets.get(ti + 1).copied().filter(|&n| n > o).unwrap_or(r.b.len());
        let data = r.b.get(o..next.min(r.b.len())).ok_or_else(|| bad("tile offset past the end of the file"))?;
        let px = tile_pixels(data, compression, bpp, tw * th)?;
        for y in 0..th {
            let dst = (((ty as usize) * TILE as usize + y) * w as usize + (tx as usize) * TILE as usize) * bpp;
            out[dst..dst + tw * bpp].copy_from_slice(&px[y * tw * bpp..(y + 1) * tw * bpp]);
        }
    }
    Ok((w, hgt, bpp, out))
}

/// Blend mode of an XCF layer mode (legacy 0..=27 and default 28..=63 sets).
fn blend_of(mode: u32) -> (BlendMode, bool) {
    let m = match mode {
        0 | 28 => BlendMode::Normal,
        1 | 29 => BlendMode::Dissolve,
        3 | 31 => BlendMode::Multiply,
        4 | 32 => BlendMode::Screen,
        5 | 23 | 33 => BlendMode::Overlay,
        6 | 34 => BlendMode::Difference,
        7 | 35 => BlendMode::LinearDodge,
        8 | 36 => BlendMode::Subtract,
        9 | 37 => BlendMode::Darken,
        10 | 38 => BlendMode::Lighten,
        11 | 39 => BlendMode::HsvHue,
        12 | 40 => BlendMode::HsvSaturation,
        13 | 41 => BlendMode::Color,
        14 | 42 => BlendMode::HsvValue,
        15 | 43 => BlendMode::Divide,
        16 | 44 => BlendMode::ColorDodge,
        17 | 45 => BlendMode::ColorBurn,
        18 | 46 => BlendMode::HardLight,
        19 | 47 => BlendMode::SoftLight,
        50 => BlendMode::VividLight,
        51 => BlendMode::PinLight,
        52 => BlendMode::LinearLight,
        53 => BlendMode::HardMix,
        54 => BlendMode::Exclusion,
        55 => BlendMode::LinearBurn,
        56 => BlendMode::DarkerColor,
        57 => BlendMode::LighterColor,
        58 => BlendMode::Luminance,
        24 => BlendMode::Hue,
        25 => BlendMode::Saturation,
        26 => BlendMode::Color,
        27 => BlendMode::Luminosity,
        63 => BlendMode::PassThrough,
        _ => return (BlendMode::Normal, true),
    };
    (m, false)
}

/// XCF mode of a blend mode (default set).
fn mode_of(b: BlendMode) -> Option<u32> {
    Some(match b {
        BlendMode::Normal => 28,
        BlendMode::Dissolve => 29,
        BlendMode::Multiply => 31,
        BlendMode::Screen => 32,
        BlendMode::Overlay => 33,
        BlendMode::Difference => 34,
        BlendMode::LinearDodge => 35,
        BlendMode::Subtract => 36,
        BlendMode::Darken => 37,
        BlendMode::Lighten => 38,
        BlendMode::HsvHue => 39,
        BlendMode::HsvSaturation => 40,
        BlendMode::Color => 41,
        BlendMode::HsvValue => 42,
        BlendMode::Divide => 43,
        BlendMode::ColorDodge => 44,
        BlendMode::ColorBurn => 45,
        BlendMode::HardLight => 46,
        BlendMode::SoftLight => 47,
        BlendMode::VividLight => 50,
        BlendMode::PinLight => 51,
        BlendMode::LinearLight => 52,
        BlendMode::HardMix => 53,
        BlendMode::Exclusion => 54,
        BlendMode::LinearBurn => 55,
        BlendMode::DarkerColor => 56,
        BlendMode::LighterColor => 57,
        BlendMode::Luminance => 58,
        BlendMode::Hue => 24,
        BlendMode::Saturation => 25,
        BlendMode::Luminosity => 27,
        BlendMode::PassThrough => 63,
        _ => return None,
    })
}

struct ReadLayer {
    layer: Layer,
    path: Option<Vec<u32>>,
    group: bool,
}

/// Document sample depth for a precision.
fn depth_of(p: Prec) -> SampleType {
    match p {
        Prec::U8 => SampleType::U8,
        Prec::U16 => SampleType::U16,
        _ => SampleType::F32,
    }
}

fn put_sample(out: &mut Vec<u8>, v: f32, depth: SampleType) {
    match depth {
        SampleType::U8 => out.push((v.clamp(0.0, 1.0) * 255.0).round() as u8),
        SampleType::U16 => out.extend(((v.clamp(0.0, 1.0) * 65535.0).round() as u16).to_ne_bytes()),
        _ => out.extend(v.to_ne_bytes()),
    }
}

/// Converts a drawable's big-endian samples (`bpp` bytes per pixel of `prec`) into the document
/// format's interleaved native bytes. `xcf_type` is the layer type (0..=5) or `None` for a
/// channel (one grey sample).
fn convert_pixels(data: &[u8], bpp: usize, prec: Prec, xcf_type: Option<u32>, colormap: &[[u8; 3]], fmt: PixelFormat) -> Result<Vec<u8>, IoError> {
    let sb = prec.bytes();
    let n = data.len() / bpp.max(1);
    let channels = fmt.channels();
    let mut out = Vec::with_capacity(n * fmt.bytes_per_pixel());
    let gray_doc = fmt.mode == ColorMode::Grayscale;
    for i in 0..n {
        let at = i * bpp;
        let s = |k: usize| prec.read(data, at + k * sb);
        let (r, g, b, a) = match xcf_type {
            Some(0) => (s(0), s(1), s(2), 1.0),
            Some(1) => (s(0), s(1), s(2), s(3)),
            Some(2) => (s(0), s(0), s(0), 1.0),
            Some(3) => (s(0), s(0), s(0), s(1)),
            Some(4 | 5) => {
                let idx = data.get(at).copied().unwrap_or(0) as usize;
                let c = colormap.get(idx).copied().unwrap_or([0, 0, 0]);
                let a = if xcf_type == Some(5) { prec.read(data, at + sb) } else { 1.0 };
                (f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0, a)
            }
            Some(t) => return Err(bad(format!("layer type {t}"))),
            None => (s(0), s(0), s(0), 1.0),
        };
        if gray_doc {
            let l = if xcf_type.is_some_and(|t| t <= 1) { 0.299 * r + 0.587 * g + 0.114 * b } else { r };
            put_sample(&mut out, l, fmt.sample);
            if channels == 2 {
                put_sample(&mut out, a, fmt.sample);
            }
        } else {
            put_sample(&mut out, r, fmt.sample);
            put_sample(&mut out, g, fmt.sample);
            put_sample(&mut out, b, fmt.sample);
            if channels == 4 {
                put_sample(&mut out, a, fmt.sample);
            }
        }
    }
    Ok(out)
}

/// A channel hierarchy as an 8-bit grey surface placed at `(x, y)`.
fn gray_surface(r: &Rd, at: usize, img: &Image, x: i32, y: i32) -> Result<Surface, IoError> {
    let (w, h, bpp, data) = read_hierarchy(r, at, img.compression)?;
    let mut s = Surface::new(PixelFormat::GRAY8);
    let mut bytes = Vec::with_capacity((w as usize) * (h as usize));
    for i in 0..(w as usize) * (h as usize) {
        bytes.push((img.prec.read(&data, i * bpp).clamp(0.0, 1.0) * 255.0).round() as u8);
    }
    s.write_interleaved(Rect::new(x, y, x.saturating_add(w as i32), y.saturating_add(h as i32)), &bytes);
    s.prune();
    Ok(s)
}

fn read_layer(r: &Rd, at: usize, img: &mut Image, fmt: PixelFormat) -> Result<ReadLayer, IoError> {
    let mut l = r.seek(at)?;
    let w = l.u32()?;
    let h = l.u32()?;
    let ty = l.u32()?;
    let name = l.string()?;
    let p = item_props(&mut l)?;
    let hier = l.ptr()?;
    let mask_at = l.ptr()?;
    let mut layer = if p.group {
        Layer::group(name, Vec::new())
    } else {
        let mut lay = Layer::raster(name, fmt);
        if w > 0 && h > 0 && hier != 0 {
            let (hw, hh, bpp, data) = read_hierarchy(r, hier, img.compression)?;
            if (hw, hh) != (w, h) {
                return Err(bad("layer pixel size differs from its header"));
            }
            let bytes = convert_pixels(&data, bpp, img.prec, Some(ty), &img.colormap, fmt)?;
            let mut s = Surface::new(fmt);
            let rect = Rect::new(p.offsets.0, p.offsets.1, p.offsets.0.saturating_add(w as i32), p.offsets.1.saturating_add(h as i32));
            if rect.width() == w && rect.height() == h {
                s.write_interleaved(rect, &bytes);
                s.prune();
            }
            lay.content = LayerContent::Raster(s);
        }
        lay
    };
    layer.visible = p.visible;
    layer.opacity = p.opacity.clamp(0.0, 1.0);
    if let Some(m) = p.mode {
        let (b, approx) = blend_of(m);
        if approx {
            img.warnings.push(format!("layer \"{}\": blend mode {m} has no equivalent and was opened as Normal", layer.name));
        }
        if p.group || b != BlendMode::PassThrough {
            layer.blend = b;
        }
    } else if p.group {
        layer.blend = BlendMode::PassThrough;
    }
    layer.locks.transparency = p.lock_alpha;
    layer.locks.pixels = p.lock_content;
    layer.locks.position = p.lock_position;
    if mask_at != 0 {
        let mut c = r.seek(mask_at)?;
        let _mw = c.u32()?;
        let _mh = c.u32()?;
        let _mname = c.string()?;
        let mp = item_props(&mut c)?;
        let mhier = c.ptr()?;
        if mhier != 0 {
            let surface = gray_surface(r, mhier, img, p.offsets.0, p.offsets.1)?;
            layer.mask = Some(LayerMask { surface, enabled: p.apply_mask && mp.visible, linked: true, density: 1.0, feather: 0.0 });
        }
    }
    Ok(ReadLayer { layer, path: p.item_path, group: p.group })
}

/// Places `layers` (file order: top first, with item paths) into a tree, bottom-up.
fn build_tree(layers: Vec<ReadLayer>) -> Vec<Layer> {
    // Without item paths the list is flat.
    if layers.iter().all(|l| l.path.is_none()) || !layers.iter().any(|l| l.group) {
        return layers.into_iter().rev().map(|l| l.layer).collect();
    }
    // Item paths are indices from the top; sort by path, then nest by prefix.
    struct Node {
        layer: Layer,
        children: Vec<Node>,
    }
    fn insert(nodes: &mut Vec<Node>, path: &[u32], layer: Layer) {
        if path.len() <= 1 {
            let idx = path.first().copied().unwrap_or(nodes.len() as u32) as usize;
            let at = idx.min(nodes.len());
            nodes.insert(at, Node { layer, children: Vec::new() });
            return;
        }
        let idx = (path[0] as usize).min(nodes.len().saturating_sub(1));
        match nodes.get_mut(idx) {
            Some(parent) => insert(&mut parent.children, &path[1..], layer),
            None => nodes.push(Node { layer, children: Vec::new() }),
        }
    }
    fn finish(nodes: Vec<Node>) -> Vec<Layer> {
        // The file lists the top first; the document stores the bottom first.
        nodes
            .into_iter()
            .rev()
            .map(|n| {
                let mut l = n.layer;
                if let Some(c) = l.children_mut() {
                    *c = finish(n.children);
                }
                l
            })
            .collect()
    }
    let mut items: Vec<(Vec<u32>, Layer)> = layers.into_iter().enumerate().map(|(i, l)| (l.path.unwrap_or_else(|| vec![i as u32]), l.layer)).collect();
    // Parents before children, siblings in order.
    items.sort_by(|a, b| a.0.len().cmp(&b.0.len()).then_with(|| a.0.cmp(&b.0)));
    let mut root: Vec<Node> = Vec::new();
    for (path, layer) in items {
        insert(&mut root, &path, layer);
    }
    finish(root)
}

pub(crate) fn import(name: &str, bytes: &[u8]) -> Result<ImportResult, IoError> {
    if !is_xcf(bytes) || bytes.len() < 14 {
        return Err(bad("not an XCF file"));
    }
    let tag = &bytes[9..14];
    let version = if tag == b"file\0" {
        0
    } else if tag[0] == b'v' && tag[4] == 0 {
        std::str::from_utf8(&tag[1..4]).ok().and_then(|s| s.parse::<u32>().ok()).ok_or_else(|| bad("bad version tag"))?
    } else {
        return Err(bad("bad version tag"));
    };
    if version > 12 {
        return Err(bad(format!("version {version} is newer than this reader (up to 12)")));
    }
    let mut r = Rd { b: bytes, at: 14, wide: version >= 11 };
    let width = r.u32()?;
    let height = r.u32()?;
    let base_type = r.u32()?;
    let prec_code = if version >= 4 { r.u32()? } else { 150 };
    let (prec, linear) = precision(version, prec_code)?;
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE || u64::from(width) * u64::from(height) * 8 > MAX_PIXEL_BYTES {
        return Err(bad(format!("canvas {width} × {height} is out of range")));
    }
    let mut img = Image {
        prec,
        compression: Compression::None,
        colormap: Vec::new(),
        dpi: None,
        comment: None,
        icc: None,
        guides: Vec::new(),
        paths: Vec::new(),
        warnings: Vec::new(),
    };
    image_props(&mut r, &mut img, version)?;
    let depth = depth_of(prec);
    if matches!(prec, Prec::U32 | Prec::F16 | Prec::F64) {
        img.warnings.push(format!("{prec:?} samples open as {}", if depth == SampleType::F32 { "32-bit float" } else { "16-bit" }));
    }
    if linear {
        img.warnings.push("linear-light pixel values are opened as stored; tag the document with a linear profile if it looks dark".into());
    }
    let mode = match base_type {
        0 => ColorMode::Rgb,
        1 => ColorMode::Grayscale,
        2 => ColorMode::Rgb,
        t => return Err(bad(format!("base type {t}"))),
    };
    let fmt = PixelFormat::new(mode, depth, true);
    // Layer and channel pointer lists.
    let mut layer_ptrs = Vec::new();
    loop {
        let p = r.ptr()?;
        if p == 0 {
            break;
        }
        if layer_ptrs.len() >= MAX_LAYERS {
            return Err(bad("too many layers"));
        }
        layer_ptrs.push(p);
    }
    let mut channel_ptrs = Vec::new();
    loop {
        let p = r.ptr()?;
        if p == 0 {
            break;
        }
        if channel_ptrs.len() >= MAX_LAYERS {
            return Err(bad("too many channels"));
        }
        channel_ptrs.push(p);
    }
    let mut layers = Vec::with_capacity(layer_ptrs.len());
    for &p in &layer_ptrs {
        layers.push(read_layer(&r, p, &mut img, fmt)?);
    }
    let mut doc = Document::new(name, Size::new(width, height), mode, depth);
    doc.layers = build_tree(layers);
    if base_type == 2 && !img.colormap.is_empty() {
        doc.mode = ColorMode::Indexed;
        doc.color_table = Some(redrob_photo_doc::ColorTable { colors: img.colormap.clone(), transparent: None });
    }
    for &p in &channel_ptrs {
        let mut c = r.seek(p)?;
        let _cw = c.u32()?;
        let _ch = c.u32()?;
        let cname = c.string()?;
        let cp = item_props(&mut c)?;
        let hier = c.ptr()?;
        if cp.selection {
            if hier != 0 {
                doc.selection = Some(gray_surface(&r, hier, &img, 0, 0)?);
            }
            continue;
        }
        if hier == 0 {
            continue;
        }
        let surface = gray_surface(&r, hier, &img, 0, 0)?;
        let mut ch = AlphaChannel::new(cname, surface);
        if let Some(c) = cp.color {
            ch.color = redrob_photo_doc::Color::rgb(c[0], c[1], c[2]);
        }
        ch.opacity = cp.opacity.clamp(0.0, 1.0);
        doc.channels.push(ch);
    }
    if let Some((x, y)) = img.dpi {
        doc.resolution_dpi = x;
        img.warnings.extend(crate::unequal_resolution_warning(f64::from(x), f64::from(y)));
    }
    if let Some(c) = img.comment.take().filter(|c| !c.is_empty()) {
        doc.metadata.text.push(("Comment".into(), c));
    }
    if let Some(icc) = img.icc.take() {
        doc.icc_profile = Some(Arc::new(icc));
    }
    for (coord, vertical) in &img.guides {
        if *vertical {
            doc.guides.vertical.push(f64::from(*coord));
        } else {
            doc.guides.horizontal.push(f64::from(*coord));
        }
    }
    doc.paths = std::mem::take(&mut img.paths);
    if doc.layers.is_empty() {
        img.warnings.push("the file has no layers".into());
    }
    Ok(ImportResult { document: doc, warnings: img.warnings, source_read_only: false, preview_only: false })
}

// ---------------------------------------------------------------------------------------------
// Writing

struct Wr {
    b: Vec<u8>,
}

impl Wr {
    fn u8(&mut self, v: u8) {
        self.b.push(v);
    }
    fn u32(&mut self, v: u32) {
        self.b.extend(v.to_be_bytes());
    }
    #[allow(dead_code)]
    fn i32(&mut self, v: i32) {
        self.b.extend(v.to_be_bytes());
    }
    fn f32(&mut self, v: f32) {
        self.b.extend(v.to_bits().to_be_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.b.extend(v.to_be_bytes());
    }
    /// Reserves a 64-bit pointer; returns where to patch it.
    fn ptr_slot(&mut self) -> usize {
        let at = self.b.len();
        self.u64(0);
        at
    }
    fn patch(&mut self, slot: usize, v: usize) {
        self.b[slot..slot + 8].copy_from_slice(&(v as u64).to_be_bytes());
    }
    fn string(&mut self, s: &str) {
        let bytes: Vec<u8> = s.bytes().filter(|&b| b != 0).collect();
        if bytes.is_empty() {
            self.u32(0);
            return;
        }
        self.u32(bytes.len() as u32 + 1);
        self.b.extend(bytes);
        self.u8(0);
    }
    fn prop(&mut self, ty: u32, payload: &[u8]) {
        self.u32(ty);
        self.u32(payload.len() as u32);
        self.b.extend_from_slice(payload);
    }
}

fn be(v: u32) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}

/// RLE-encodes one byte stream.
fn rle_encode(src: &[u8], out: &mut Vec<u8>) {
    let mut i = 0;
    while i < src.len() {
        let mut run = 1;
        while i + run < src.len() && src[i + run] == src[i] && run < 65_535 {
            run += 1;
        }
        if run >= 2 {
            if run <= 127 {
                out.push((run - 1) as u8);
            } else {
                out.push(127);
                out.extend((run as u16).to_be_bytes());
            }
            out.push(src[i]);
            i += run;
            continue;
        }
        // Literal stretch up to the next run of three.
        let start = i;
        let mut j = i;
        while j < src.len() && j - start < 65_535 {
            if j + 2 < src.len() && src[j] == src[j + 1] && src[j] == src[j + 2] {
                break;
            }
            j += 1;
        }
        if j == start {
            j += 1;
        }
        let len = j - start;
        if len <= 127 {
            out.push((256 - len) as u8);
        } else {
            out.push(128);
            out.extend((len as u16).to_be_bytes());
        }
        out.extend_from_slice(&src[start..j]);
        i = j;
    }
}

/// Writes a hierarchy (one level, RLE tiles) for `w` × `h` pixels of `bpp` big-endian bytes.
fn write_hierarchy(w: &mut Wr, width: u32, height: u32, bpp: usize, pixels: &[u8]) -> usize {
    let at = w.b.len();
    w.u32(width);
    w.u32(height);
    w.u32(bpp as u32);
    let level_slot = w.ptr_slot();
    w.u64(0);
    let level_at = w.b.len();
    w.patch(level_slot, level_at);
    w.u32(width);
    w.u32(height);
    let tiles_x = width.div_ceil(TILE) as usize;
    let tiles_y = height.div_ceil(TILE) as usize;
    let slots: Vec<usize> = (0..tiles_x * tiles_y).map(|_| w.ptr_slot()).collect();
    w.u64(0);
    let mut plane = Vec::new();
    for (ti, slot) in slots.into_iter().enumerate() {
        let tx = (ti % tiles_x) as u32;
        let ty = (ti / tiles_x) as u32;
        let tw = (width - tx * TILE).min(TILE) as usize;
        let th = (height - ty * TILE).min(TILE) as usize;
        let tile_at = w.b.len();
        w.patch(slot, tile_at);
        for b in 0..bpp {
            plane.clear();
            plane.reserve(tw * th);
            for y in 0..th {
                let row = ((ty as usize) * TILE as usize + y) * width as usize + (tx as usize) * TILE as usize;
                for x in 0..tw {
                    plane.push(pixels[(row + x) * bpp + b]);
                }
            }
            rle_encode(&plane, &mut w.b);
        }
    }
    at
}

/// Big-endian sample bytes of a normalised value at the document depth.
fn sample_be(out: &mut Vec<u8>, v: f32, depth: SampleType) {
    match depth {
        SampleType::U8 => out.push((v.clamp(0.0, 1.0) * 255.0).round() as u8),
        SampleType::U16 => out.extend(((v.clamp(0.0, 1.0) * 65535.0).round() as u16).to_be_bytes()),
        _ => out.extend(v.to_bits().to_be_bytes()),
    }
}

/// A layer's own pixels (straight RGBA) and bounds, mask and fill opacity left alone.
fn layer_pixels(layer: &Layer, canvas: Rect) -> (Rect, Vec<[f32; 4]>) {
    match &layer.content {
        LayerContent::Raster(s) => {
            let r = s.content_bounds();
            (r, redrob_photo_compose::surface_to_buffer(s, r).px)
        }
        _ => {
            let mut solo = layer.clone();
            solo.visible = true;
            solo.opacity = 1.0;
            solo.fill_opacity = 1.0;
            solo.blend = BlendMode::Normal;
            solo.clipped = false;
            solo.mask = None;
            solo.effects = Default::default();
            let r = redrob_photo_compose::composite_bounds(&solo, canvas).unwrap_or(canvas).intersect(&canvas);
            (r, redrob_photo_compose::render_layer(&solo, r).px)
        }
    }
}

struct Writer<'a> {
    doc: &'a Document,
    w: Wr,
    depth: SampleType,
    gray: bool,
    warnings: Vec<String>,
    /// (layer struct offset slot in the pointer table) in file order.
    layer_slots: Vec<usize>,
}

impl Writer<'_> {
    fn item_props(&mut self, layer: &Layer, path: &[u32], group: bool) {
        let w = &mut self.w;
        if group {
            w.prop(PROP_GROUP_ITEM, &[]);
            w.prop(PROP_GROUP_ITEM_FLAGS, &be(1));
        }
        if path.len() > 1 {
            let mut p = Vec::with_capacity(path.len() * 4);
            for &i in path {
                p.extend(i.to_be_bytes());
            }
            w.prop(PROP_ITEM_PATH, &p);
        }
        w.prop(PROP_OPACITY, &be((layer.opacity.clamp(0.0, 1.0) * 255.0).round() as u32));
        w.prop(PROP_FLOAT_OPACITY, &layer.opacity.clamp(0.0, 1.0).to_bits().to_be_bytes());
        w.prop(PROP_VISIBLE, &be(u32::from(layer.visible)));
        let mode = match mode_of(layer.blend) {
            Some(m) => m,
            None => {
                self.warnings.push(format!("layer \"{}\": the {} blend mode has no XCF equivalent; saved as Normal", layer.name, layer.blend.label()));
                28
            }
        };
        let w = &mut self.w;
        w.prop(PROP_MODE, &be(mode));
        w.prop(PROP_LOCK_ALPHA, &be(u32::from(layer.locks.transparency)));
        w.prop(PROP_LOCK_CONTENT, &be(u32::from(layer.locks.pixels || layer.locks.all)));
        w.prop(PROP_LOCK_POSITION, &be(u32::from(layer.locks.position || layer.locks.all)));
        if layer.mask.is_some() {
            w.prop(PROP_APPLY_MASK, &be(u32::from(layer.mask.as_ref().is_some_and(|m| m.enabled))));
        }
    }

    /// Writes one layer (and, for a group, its children after it) at the current position.
    fn layer(&mut self, layer: &Layer, path: &[u32]) -> Result<(), IoError> {
        let canvas = self.doc.bounds();
        let group = matches!(layer.content, LayerContent::Group(_));
        let (rect, px) = if group { (canvas, Vec::new()) } else { layer_pixels(layer, canvas) };
        let rect = if rect.is_empty() { Rect::new(0, 0, 1, 1) } else { rect };
        let (lw, lh) = (rect.width(), rect.height());
        if lw > MAX_SIDE || lh > MAX_SIDE {
            return Err(bad(format!("layer \"{}\" is too large", layer.name)));
        }
        let at = self.w.b.len();
        let slot = self.layer_slots.pop().ok_or_else(|| bad("layer slot bookkeeping"))?;
        self.w.patch(slot, at);
        self.w.u32(lw);
        self.w.u32(lh);
        self.w.u32(if self.gray { 3 } else { 1 });
        self.w.string(&layer.name);
        if !matches!(layer.content, LayerContent::Raster(_) | LayerContent::Group(_)) {
            self.warnings.push(format!("layer \"{}\": {} layers are saved to XCF as pixels", layer.name, layer.content.kind_name().to_lowercase()));
        }
        if layer.fill_opacity < 1.0 || (layer.effects.enabled && !layer.effects.items.is_empty()) {
            self.warnings.push(format!("layer \"{}\": fill opacity and layer effects are not kept in XCF", layer.name));
        }
        self.item_props(layer, path, group);
        self.w.prop(PROP_OFFSETS, &[rect.x0.to_be_bytes(), rect.y0.to_be_bytes()].concat());
        self.w.prop(PROP_END, &[]);
        let hier_slot = self.w.ptr_slot();
        let mask_slot = self.w.ptr_slot();
        // Pixels: RGBA or grey + alpha, big-endian samples at the document depth.
        let n = (lw as usize) * (lh as usize);
        let channels = if self.gray { 2 } else { 4 };
        let sb = match self.depth {
            SampleType::U8 => 1,
            SampleType::U16 => 2,
            _ => 4,
        };
        let mut data = Vec::with_capacity(n * channels * sb);
        for i in 0..n {
            let p = px.get(i).copied().unwrap_or([0.0; 4]);
            if self.gray {
                sample_be(&mut data, p[0], self.depth);
            } else {
                sample_be(&mut data, p[0], self.depth);
                sample_be(&mut data, p[1], self.depth);
                sample_be(&mut data, p[2], self.depth);
            }
            sample_be(&mut data, p[3], self.depth);
        }
        let hier_at = write_hierarchy(&mut self.w, lw, lh, channels * sb, &data);
        self.w.patch(hier_slot, hier_at);
        if let Some(mask) = layer.mask.as_ref() {
            let mut values = Vec::new();
            mask.values_into(rect, &mut values);
            let mask_at = self.channel(&format!("{} mask", layer.name), lw, lh, &values, None, 1.0, mask.enabled, false)?;
            self.w.patch(mask_slot, mask_at);
            if mask.feather > 0.0 || mask.density < 1.0 {
                self.warnings.push(format!("layer \"{}\": mask density and feather are not kept in XCF", layer.name));
            }
        }
        Ok(())
    }

    /// Writes a channel structure; returns its offset.
    #[allow(clippy::too_many_arguments)]
    fn channel(
        &mut self,
        name: &str,
        cw: u32,
        ch: u32,
        values: &[f32],
        color: Option<[f32; 3]>,
        opacity: f32,
        visible: bool,
        selection: bool,
    ) -> Result<usize, IoError> {
        let at = self.w.b.len();
        self.w.u32(cw);
        self.w.u32(ch);
        self.w.string(name);
        if selection {
            self.w.prop(PROP_SELECTION, &[]);
        }
        self.w.prop(PROP_OPACITY, &be((opacity.clamp(0.0, 1.0) * 255.0).round() as u32));
        self.w.prop(PROP_FLOAT_OPACITY, &opacity.clamp(0.0, 1.0).to_bits().to_be_bytes());
        self.w.prop(PROP_VISIBLE, &be(u32::from(visible)));
        if let Some(c) = color {
            let bytes = [(c[0] * 255.0).round() as u8, (c[1] * 255.0).round() as u8, (c[2] * 255.0).round() as u8];
            self.w.prop(PROP_COLOR, &bytes);
            let mut f = Vec::with_capacity(12);
            for v in c {
                f.extend(v.to_bits().to_be_bytes());
            }
            self.w.prop(PROP_FLOAT_COLOR, &f);
        }
        self.w.prop(PROP_END, &[]);
        let hier_slot = self.w.ptr_slot();
        let sb = match self.depth {
            SampleType::U8 => 1,
            SampleType::U16 => 2,
            _ => 4,
        };
        let n = (cw as usize) * (ch as usize);
        let mut data = Vec::with_capacity(n * sb);
        for i in 0..n {
            sample_be(&mut data, values.get(i).copied().unwrap_or(0.0), self.depth);
        }
        let hier_at = write_hierarchy(&mut self.w, cw, ch, sb, &data);
        self.w.patch(hier_slot, hier_at);
        Ok(at)
    }
}

/// Flattens the layer tree into file order (top first) with item paths.
fn file_order<'a>(layers: &'a [Layer], prefix: &[u32], out: &mut Vec<(&'a Layer, Vec<u32>)>) {
    for (i, l) in layers.iter().rev().enumerate() {
        let mut path = prefix.to_vec();
        path.push(i as u32);
        out.push((l, path.clone()));
        if let LayerContent::Group(g) = &l.content {
            file_order(&g.children, &path, out);
        }
    }
}

fn write_vectors(w: &mut Wr, paths: &[NamedPath]) {
    let mut v = Wr { b: Vec::new() };
    v.u32(1);
    v.u32(0);
    v.u32(paths.len() as u32);
    for p in paths {
        v.string(&p.name);
        v.u32(0); // tattoo
        v.u32(1); // visible
        v.u32(0); // linked
        v.u32(0); // parasites
        v.u32(p.path.subpaths.len() as u32);
        for sp in &p.path.subpaths {
            v.u32(1); // bezier
            v.u32(u32::from(sp.closed));
            v.u32(2); // axes
            // Control points: in handle, anchor, out handle per knot.
            let mut pts: Vec<(u32, f64, f64)> = Vec::with_capacity(sp.knots.len() * 3);
            for k in &sp.knots {
                pts.push((1, k.in_ctrl.x, k.in_ctrl.y));
                pts.push((0, k.anchor.x, k.anchor.y));
                pts.push((1, k.out_ctrl.x, k.out_ctrl.y));
            }
            v.u32(pts.len() as u32);
            for (t, x, y) in pts {
                v.u32(t);
                v.f32(x as f32);
                v.f32(y as f32);
            }
        }
    }
    w.prop(PROP_VECTORS, &v.b);
}

/// What saving `doc` as XCF leaves out (empty when the file holds the whole document): the
/// layer kinds that become pixels, fill opacity, effects, mask density and feather, spot
/// channels, and colour modes other than RGB, grey and indexed.
pub fn losses(doc: &Document) -> Vec<String> {
    fn walk(layers: &[Layer], out: &mut Vec<String>) {
        for l in layers {
            match &l.content {
                LayerContent::Raster(_) => {}
                LayerContent::Group(g) => walk(&g.children, out),
                other => out.push(format!("layer \"{}\": {} layers are saved to XCF as pixels", l.name, other.kind_name().to_lowercase())),
            }
            if l.fill_opacity < 1.0 || (l.effects.enabled && !l.effects.items.is_empty()) {
                out.push(format!("layer \"{}\": fill opacity and layer effects are not kept in XCF", l.name));
            }
            if l.mask.as_ref().is_some_and(|m| m.feather > 0.0 || m.density < 1.0) {
                out.push(format!("layer \"{}\": mask density and feather are not kept in XCF", l.name));
            }
            if mode_of(l.blend).is_none() {
                out.push(format!("layer \"{}\": the {} blend mode has no XCF equivalent; saved as Normal", l.name, l.blend.label()));
            }
        }
    }
    let mut out = Vec::new();
    if !matches!(doc.mode, ColorMode::Rgb | ColorMode::Grayscale | ColorMode::Indexed) {
        out.push(format!("{:?} documents are saved to XCF as RGB", doc.mode));
    }
    walk(&doc.layers, &mut out);
    for ch in &doc.channels {
        if ch.spot.is_some() {
            out.push(format!("channel \"{}\": spot colour is saved as a plain channel", ch.name));
        }
    }
    out
}

pub(crate) fn export(doc: &Document) -> Result<ExportResult, IoError> {
    let depth = match doc.depth {
        SampleType::U8 => SampleType::U8,
        SampleType::U16 => SampleType::U16,
        _ => SampleType::F32,
    };
    let gray = matches!(doc.mode, ColorMode::Grayscale | ColorMode::Bitmap | ColorMode::Duotone);
    let mut wr = Writer { doc, w: Wr { b: Vec::new() }, depth, gray, warnings: Vec::new(), layer_slots: Vec::new() };
    if !matches!(doc.mode, ColorMode::Rgb | ColorMode::Grayscale | ColorMode::Indexed) {
        wr.warnings.push(format!("{:?} documents are saved to XCF as RGB", doc.mode));
    }
    let indexed = doc.mode == ColorMode::Indexed && doc.color_table.as_ref().is_some_and(|t| !t.colors.is_empty());
    let w = &mut wr.w;
    w.b.extend_from_slice(MAGIC);
    w.b.extend_from_slice(b"v011\0");
    w.u32(doc.size.width);
    w.u32(doc.size.height);
    // Indexed documents keep their pixels expanded here, so the colour map is kept as metadata
    // and the layers are written as RGB (base type RGB).
    w.u32(if gray { 1 } else { 0 });
    w.u32(match depth {
        SampleType::U8 => 150,
        SampleType::U16 => 250,
        _ => 650,
    });
    if indexed {
        wr.warnings.push("the indexed colour table is kept, but the layers are written as RGB".into());
    }
    w.prop(PROP_COMPRESSION, &[1]);
    let mut res = Vec::with_capacity(8);
    res.extend(doc.resolution_dpi.to_bits().to_be_bytes());
    res.extend(doc.resolution_dpi.to_bits().to_be_bytes());
    w.prop(PROP_RESOLUTION, &res);
    if !doc.guides.horizontal.is_empty() || !doc.guides.vertical.is_empty() {
        let mut g = Vec::new();
        for &h in &doc.guides.horizontal {
            g.extend((h.round() as i32).to_be_bytes());
            g.push(1);
        }
        for &v in &doc.guides.vertical {
            g.extend((v.round() as i32).to_be_bytes());
            g.push(2);
        }
        w.prop(PROP_GUIDES, &g);
    }
    // Parasites: the comment and the ICC profile.
    let comment = doc.metadata.text.iter().find(|(k, _)| k.eq_ignore_ascii_case("Comment")).map(|(_, v)| v.clone());
    if comment.is_some() || doc.icc_profile.is_some() {
        let mut p = Wr { b: Vec::new() };
        if let Some(c) = &comment {
            p.string("gimp-comment");
            p.u32(1); // persistent
            let mut bytes = c.as_bytes().to_vec();
            bytes.push(0);
            p.u32(bytes.len() as u32);
            p.b.extend(bytes);
        }
        if let Some(icc) = &doc.icc_profile {
            p.string("icc-profile");
            p.u32(1);
            p.u32(icc.len() as u32);
            p.b.extend_from_slice(icc);
        }
        w.prop(PROP_PARASITES, &p.b);
    }
    if !doc.paths.is_empty() {
        write_vectors(w, &doc.paths);
    }
    w.prop(PROP_END, &[]);
    // Pointer tables.
    let mut order = Vec::new();
    file_order(&doc.layers, &[], &mut order);
    if order.len() > MAX_LAYERS {
        return Err(bad("too many layers"));
    }
    let mut slots: Vec<usize> = (0..order.len()).map(|_| w.ptr_slot()).collect();
    w.u64(0);
    let channel_slots: Vec<usize> = (0..doc.channels.len() + usize::from(doc.selection.is_some())).map(|_| w.ptr_slot()).collect();
    w.u64(0);
    slots.reverse();
    wr.layer_slots = slots;
    for (layer, path) in &order {
        wr.layer(layer, path)?;
    }
    let canvas = doc.bounds();
    let mut ci = channel_slots.into_iter();
    for ch in &doc.channels {
        let values = ch.surface.read_region(canvas);
        let color = Some(ch.color.to_rgb());
        let at = wr.channel(&ch.name, canvas.width(), canvas.height(), &values, color, ch.opacity, true, false)?;
        if let Some(slot) = ci.next() {
            wr.w.patch(slot, at);
        }
        if ch.spot.is_some() {
            wr.warnings.push(format!("channel \"{}\": spot colour is saved as a plain channel", ch.name));
        }
    }
    if let Some(sel) = &doc.selection {
        let values = sel.read_region(canvas);
        let at = wr.channel("Selection Mask", canvas.width(), canvas.height(), &values, None, 0.5, false, true)?;
        if let Some(slot) = ci.next() {
            wr.w.patch(slot, at);
        }
    }
    Ok(ExportResult { bytes: wr.w.b, warnings: wr.warnings })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rle_round_trips() {
        let src: Vec<u8> = (0..300u32).map(|i| if i % 50 < 30 { 7 } else { (i % 256) as u8 }).collect();
        let mut enc = Vec::new();
        rle_encode(&src, &mut enc);
        let mut out = Vec::new();
        let mut at = 0;
        rle(&enc, &mut at, src.len(), &mut out).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(out, src);
        assert_eq!(at, enc.len());
    }

    #[test]
    fn layered_document_round_trips() {
        let mut doc = Document::new("t.xcf", Size::new(70, 40), ColorMode::Rgb, SampleType::U8);
        let fmt = PixelFormat::RGBA8;
        let mut a = Layer::raster("Bottom", fmt);
        if let LayerContent::Raster(s) = &mut a.content {
            let r = Rect::new(0, 0, 70, 40);
            let px: Vec<u8> = (0..70 * 40).flat_map(|i| [(i % 256) as u8, 20, 200, 255]).collect();
            s.write_interleaved(r, &px);
        }
        let mut b = Layer::raster("Top (half)", fmt);
        b.opacity = 0.5;
        b.blend = BlendMode::Multiply;
        if let LayerContent::Raster(s) = &mut b.content {
            let r = Rect::new(10, 5, 30, 25);
            let px: Vec<u8> = (0..20 * 20).flat_map(|_| [255, 0, 0, 128]).collect();
            s.write_interleaved(r, &px);
        }
        b.mask = Some(LayerMask::hide_all());
        let group = Layer::group("Group", vec![b]);
        doc.layers = vec![a, group];
        doc.channels.push(AlphaChannel::new("Spare", Surface::with_default(PixelFormat::GRAY8, &[0.25])));
        doc.paths.push(NamedPath { name: "Tri".into(), path: Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)])]), psd_raw: None });
        let out = export(&doc).unwrap_or_else(|e| panic!("{e}"));
        assert!(is_xcf(&out.bytes));
        let back = import("t.xcf", &out.bytes).unwrap_or_else(|e| panic!("{e}"));
        let d = back.document;
        assert_eq!((d.size.width, d.size.height), (70, 40));
        assert_eq!(d.layers.len(), 2);
        assert_eq!(d.layers[0].name, "Bottom");
        assert_eq!(d.layers[1].name, "Group");
        let LayerContent::Group(g) = &d.layers[1].content else { panic!("group") };
        assert_eq!(g.children.len(), 1);
        let top = &g.children[0];
        assert_eq!(top.name, "Top (half)");
        assert!((top.opacity - 0.5).abs() < 0.01);
        assert_eq!(top.blend, BlendMode::Multiply);
        assert!(top.mask.is_some());
        if let LayerContent::Raster(s) = &top.content {
            assert_eq!(s.content_bounds(), Rect::new(10, 5, 30, 25));
        } else {
            panic!("raster");
        }
        assert_eq!(d.channels.len(), 1);
        assert_eq!(d.paths.len(), 1);
        assert_eq!(d.paths[0].path.subpaths[0].knots.len(), 3);
    }
}
