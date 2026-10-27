//! Windows Metafile (`.wmf`) import: the 16-bit GDI record stream is interpreted and rasterised
//! with the vector renderer. Handled: the placeable header (size in metafile units per inch),
//! window origin and extent, pens and brushes (solid and null), polygon, polyline, polypolygon,
//! rectangle, rounded rectangle, ellipse, move/line to, and embedded DIBs (StretchDIBits,
//! DIBStretchBlt, DIBBitBlt). Text and arc records are skipped with a note. EMF (32-bit) files
//! are refused.

use resvg::tiny_skia;

use super::{Rgba, bad, le16, le32};
use crate::IoError;

const DEFAULT_DPI: f32 = 96.0;
const MAX_SIDE: u32 = 8192;

#[derive(Clone, Copy)]
enum Obj {
    Pen { width: f32, color: Option<[u8; 3]> },
    Brush { color: Option<[u8; 3]> },
    Other,
}

struct Gdi {
    objects: Vec<Option<Obj>>,
    pen: Obj,
    brush: Obj,
    org: (f32, f32),
    ext: (f32, f32),
    pos: (f32, f32),
    poly_even_odd: bool,
}

impl Gdi {
    fn add(&mut self, o: Obj) {
        match self.objects.iter().position(Option::is_none) {
            Some(i) => self.objects[i] = Some(o),
            None => self.objects.push(Some(o)),
        }
    }
}

fn colorref(v: u32) -> [u8; 3] {
    [v as u8, (v >> 8) as u8, (v >> 16) as u8]
}

fn paint(c: [u8; 3]) -> tiny_skia::Paint<'static> {
    let mut p = tiny_skia::Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], 255);
    p.anti_alias = true;
    p
}

pub fn decode(bytes: &[u8]) -> Result<Rgba, IoError> {
    if bytes.len() >= 44 && le32(bytes, 0) == Some(1) && le32(bytes, 40) == Some(0x464D_4520) {
        return Err(bad("WMF", "this is an EMF (32-bit) metafile; EMF is not supported"));
    }
    let mut at = 0usize;
    let mut placeable: Option<([i16; 4], u16)> = None;
    if le32(bytes, 0) == Some(0x9AC6_CDD7) {
        let b = |i: usize| le16(bytes, i).unwrap_or(0) as i16;
        placeable = Some(([b(6), b(8), b(10), b(12)], le16(bytes, 14).unwrap_or(0)));
        at = 22;
    }
    if bytes.len() < at + 18 {
        return Err(bad("WMF", "file too short"));
    }
    let header_words = le16(bytes, at + 2).unwrap_or(9) as usize;
    let num_objects = le16(bytes, at + 6).unwrap_or(0) as usize;
    at += header_words * 2;
    // First pass: find the window extent when there is no placeable header.
    let (mut org, mut ext) = ((0.0f32, 0.0f32), (0.0f32, 0.0f32));
    let mut scan = at;
    let mut records = 0usize;
    while scan + 6 <= bytes.len() && records < 1_000_000 {
        let size = le32(bytes, scan).unwrap_or(0) as usize * 2;
        let func = le16(bytes, scan + 4).unwrap_or(0);
        if size < 6 {
            break;
        }
        let p = |i: usize| le16(bytes, scan + 6 + i * 2).unwrap_or(0) as i16 as f32;
        match func {
            0x020B => org = (p(1), p(0)),
            0x020C if ext == (0.0, 0.0) => ext = (p(1).abs(), p(0).abs()),
            0 => break,
            _ => {}
        }
        scan += size;
        records += 1;
    }
    // Canvas size and the unit-to-pixel scale.
    let (w, h, scale, origin) = match placeable {
        Some(([l, t, r, b], inch)) if inch > 0 && r != l && b != t => {
            let units_w = f32::from(r) - f32::from(l);
            let units_h = f32::from(b) - f32::from(t);
            let k = DEFAULT_DPI / f32::from(inch);
            ((units_w.abs() * k).round().max(1.0) as u32, (units_h.abs() * k).round().max(1.0) as u32, k, (f32::from(l.min(r)), f32::from(t.min(b))))
        }
        _ => {
            if ext == (0.0, 0.0) {
                return Err(bad("WMF", "no placeable header and no window extent: the size is unknown"));
            }
            (ext.0.round().max(1.0) as u32, ext.1.round().max(1.0) as u32, 1.0, org)
        }
    };
    if w > MAX_SIDE || h > MAX_SIDE {
        return Err(bad("WMF", format!("{w} × {h} pixels is too large to rasterise")));
    }
    let mut pm = tiny_skia::Pixmap::new(w, h).ok_or_else(|| bad("WMF", "cannot allocate the canvas"))?;
    let mut g = Gdi {
        objects: vec![None; num_objects.min(4096)],
        pen: Obj::Pen { width: 1.0, color: Some([0, 0, 0]) },
        brush: Obj::Brush { color: Some([255, 255, 255]) },
        org: origin,
        ext,
        pos: (0.0, 0.0),
        poly_even_odd: true,
    };
    let _ = g.ext;
    let mut skipped = 0usize;
    let mut warnings = Vec::new();
    // Logical units -> pixels. The window origin maps to the canvas corner.
    let map = |g: &Gdi, x: f32, y: f32| -> (f32, f32) { ((x - g.org.0) * scale, (y - g.org.1) * scale) };
    records = 0;
    while at + 6 <= bytes.len() && records < 1_000_000 {
        let size = le32(bytes, at).unwrap_or(0) as usize * 2;
        let func = le16(bytes, at + 4).unwrap_or(0);
        if size < 6 || at + size > bytes.len() {
            break;
        }
        let params = &bytes[at + 6..at + size];
        let p = |i: usize| le16(params, i * 2).unwrap_or(0) as i16 as f32;
        let pu = |i: usize| le16(params, i * 2).unwrap_or(0);
        let n = params.len() / 2;
        match func {
            0x0000 => break,
            0x020B => g.org = (p(1), p(0)),
            0x020C => g.ext = (p(1), p(0)),
            0x0106 => g.poly_even_odd = pu(0) == 1,
            0x02FA => {
                let style = pu(0);
                let width = p(1).abs().max(1.0) * scale;
                let color = (style & 0xff != 5).then(|| colorref(le32(params, 6).unwrap_or(0)));
                g.add(Obj::Pen { width, color });
            }
            0x02FC => {
                let style = pu(0);
                let color = (style != 1).then(|| colorref(le32(params, 2).unwrap_or(0)));
                g.add(Obj::Brush { color });
            }
            0x02FB | 0x00F7 | 0x0142 | 0x01F9 | 0x06FF => g.add(Obj::Other),
            0x012D => {
                if let Some(Some(o)) = g.objects.get(pu(0) as usize) {
                    match o {
                        Obj::Pen { .. } => g.pen = *o,
                        Obj::Brush { .. } => g.brush = *o,
                        Obj::Other => {}
                    }
                }
            }
            0x01F0 => {
                if let Some(slot) = g.objects.get_mut(pu(0) as usize) {
                    *slot = None;
                }
            }
            0x0214 => g.pos = (p(1), p(0)),
            0x0213 => {
                let (x, y) = (p(1), p(0));
                let mut pb = tiny_skia::PathBuilder::new();
                let a = map(&g, g.pos.0, g.pos.1);
                let b = map(&g, x, y);
                pb.move_to(a.0, a.1);
                pb.line_to(b.0, b.1);
                if let Some(path) = pb.finish() {
                    stroke(&mut pm, &path, &g.pen);
                }
                g.pos = (x, y);
            }
            0x0324 | 0x0325 => {
                let count = pu(0) as usize;
                let mut pb = tiny_skia::PathBuilder::new();
                for i in 0..count.min(n.saturating_sub(1) / 2) {
                    let (x, y) = map(&g, p(1 + i * 2), p(2 + i * 2));
                    if i == 0 { pb.move_to(x, y) } else { pb.line_to(x, y) }
                }
                if func == 0x0324 {
                    pb.close();
                }
                if let Some(path) = pb.finish() {
                    if func == 0x0324 {
                        fill(&mut pm, &path, &g.brush, g.poly_even_odd);
                    }
                    stroke(&mut pm, &path, &g.pen);
                }
            }
            0x0538 => {
                let polys = pu(0) as usize;
                let counts: Vec<usize> = (0..polys.min(n)).map(|i| pu(1 + i) as usize).collect();
                let mut k = 1 + polys;
                let mut pb = tiny_skia::PathBuilder::new();
                for c in counts {
                    for i in 0..c {
                        if k + 1 >= n {
                            break;
                        }
                        let (x, y) = map(&g, p(k), p(k + 1));
                        if i == 0 {
                            pb.move_to(x, y)
                        } else {
                            pb.line_to(x, y)
                        }
                        k += 2;
                    }
                    pb.close();
                }
                if let Some(path) = pb.finish() {
                    fill(&mut pm, &path, &g.brush, g.poly_even_odd);
                    stroke(&mut pm, &path, &g.pen);
                }
            }
            0x041B | 0x0418 | 0x061C => {
                let o = if func == 0x061C { 2 } else { 0 };
                let (bottom, right, top, left) = (p(o), p(o + 1), p(o + 2), p(o + 3));
                let (x0, y0) = map(&g, left.min(right), top.min(bottom));
                let (x1, y1) = map(&g, left.max(right), top.max(bottom));
                let rect = tiny_skia::Rect::from_ltrb(x0, y0, x1.max(x0 + 0.01), y1.max(y0 + 0.01));
                let path = match (func, rect) {
                    (0x0418, Some(r)) => tiny_skia::PathBuilder::from_oval(r),
                    (0x061C, Some(r)) => {
                        let rx = (p(1) * scale / 2.0).abs().min(r.width() / 2.0);
                        let ry = (p(0) * scale / 2.0).abs().min(r.height() / 2.0);
                        Some(round_rect(r, rx, ry))
                    }
                    (_, Some(r)) => Some(tiny_skia::PathBuilder::from_rect(r)),
                    _ => None,
                };
                if let Some(path) = path {
                    fill(&mut pm, &path, &g.brush, g.poly_even_odd);
                    stroke(&mut pm, &path, &g.pen);
                }
            }
            0x0B41 | 0x0F43 | 0x0940 => {
                // DIB records: destination rectangle, then the DIB (BITMAPINFO + bits).
                let (dib_at, dst) = match func {
                    0x0F43 => (22, (p(10), p(9), p(8), p(7))), // StretchDIBits: dst y,x,h,w at params 7..10
                    0x0B41 => (20, (p(9), p(8), p(7), p(6))),  // DIBStretchBlt
                    _ => (16, (p(7), p(6), p(5), p(4))),       // DIBBitBlt
                };
                let (dx, dy, dh, dw) = dst;
                if let Some(dib) = params.get(dib_at..) {
                    match dib_to_rgba(dib) {
                        Ok((bw, bh, px)) => {
                            let (x0, y0) = map(&g, dx, dy);
                            let (sw, sh) = if dw == 0.0 || dh == 0.0 { (bw as f32, bh as f32) } else { ((dw * scale).abs(), (dh * scale).abs()) };
                            blit(&mut pm, &px, bw, bh, x0, y0, sw, sh);
                        }
                        Err(e) => warnings.push(format!("embedded bitmap skipped: {e}")),
                    }
                }
            }
            0x0521 | 0x0A32 | 0x0817 | 0x0830 | 0x081A => skipped += 1,
            _ => {}
        }
        at += size;
        records += 1;
    }
    if skipped > 0 {
        warnings.push(format!("{skipped} text and arc records were not drawn"));
    }
    let mut out = Rgba::new(w, h)?;
    for (i, px) in pm.pixels().iter().enumerate() {
        let c = px.demultiply();
        out.data[i * 4..i * 4 + 4].copy_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
    }
    out.dpi = Some(DEFAULT_DPI);
    out.warnings = warnings;
    Ok(out)
}

fn round_rect(r: tiny_skia::Rect, rx: f32, ry: f32) -> tiny_skia::Path {
    let mut pb = tiny_skia::PathBuilder::new();
    let (l, t, rr, b) = (r.left(), r.top(), r.right(), r.bottom());
    let k = 0.5523;
    pb.move_to(l + rx, t);
    pb.line_to(rr - rx, t);
    pb.cubic_to(rr - rx + rx * k, t, rr, t + ry - ry * k, rr, t + ry);
    pb.line_to(rr, b - ry);
    pb.cubic_to(rr, b - ry + ry * k, rr - rx + rx * k, b, rr - rx, b);
    pb.line_to(l + rx, b);
    pb.cubic_to(l + rx - rx * k, b, l, b - ry + ry * k, l, b - ry);
    pb.line_to(l, t + ry);
    pb.cubic_to(l, t + ry - ry * k, l + rx - rx * k, t, l + rx, t);
    pb.close();
    pb.finish().unwrap_or_else(|| tiny_skia::PathBuilder::from_rect(r))
}

fn fill(pm: &mut tiny_skia::Pixmap, path: &tiny_skia::Path, brush: &Obj, even_odd: bool) {
    if let Obj::Brush { color: Some(c) } = brush {
        let rule = if even_odd { tiny_skia::FillRule::EvenOdd } else { tiny_skia::FillRule::Winding };
        pm.fill_path(path, &paint(*c), rule, tiny_skia::Transform::identity(), None);
    }
}

fn stroke(pm: &mut tiny_skia::Pixmap, path: &tiny_skia::Path, pen: &Obj) {
    if let Obj::Pen { width, color: Some(c) } = pen {
        let s = tiny_skia::Stroke { width: width.max(0.5), ..Default::default() };
        pm.stroke_path(path, &paint(*c), &s, tiny_skia::Transform::identity(), None);
    }
}

/// A DIB (BITMAPINFOHEADER + colour table + bits) as straight RGBA, through the BMP codec.
fn dib_to_rgba(dib: &[u8]) -> Result<(u32, u32, Vec<u8>), IoError> {
    let header_size = le32(dib, 0).ok_or_else(|| bad("DIB", "header ends early"))? as usize;
    if header_size < 12 || header_size > dib.len() {
        return Err(bad("DIB", "bad header size"));
    }
    let bpp = le16(dib, 14).unwrap_or(24);
    let mut colors = le32(dib, 32).unwrap_or(0) as usize;
    if colors == 0 && bpp <= 8 {
        colors = 1 << bpp;
    }
    let offset = 14 + header_size + colors * 4;
    let mut bmp = Vec::with_capacity(14 + dib.len());
    bmp.extend_from_slice(b"BM");
    bmp.extend(((14 + dib.len()) as u32).to_le_bytes());
    bmp.extend([0u8; 4]);
    bmp.extend((offset as u32).to_le_bytes());
    bmp.extend_from_slice(dib);
    let img = redrob_photo_codecs::decode_as(redrob_photo_codecs::Format::Bmp, &bmp)?;
    let (w, h) = img.dimensions();
    Ok((w, h, img.to_rgba8()))
}

/// Nearest-neighbour blit of `px` (w × h) into the destination rectangle.
fn blit(pm: &mut tiny_skia::Pixmap, px: &[u8], w: u32, h: u32, x0: f32, y0: f32, sw: f32, sh: f32) {
    if w == 0 || h == 0 || sw <= 0.0 || sh <= 0.0 {
        return;
    }
    let (pw, ph) = (pm.width() as i64, pm.height() as i64);
    let (xs, ys) = (x0.round() as i64, y0.round() as i64);
    let (dw, dh) = (sw.round().max(1.0) as i64, sh.round().max(1.0) as i64);
    let data = pm.data_mut();
    for dy in 0..dh {
        let ty = ys + dy;
        if ty < 0 || ty >= ph {
            continue;
        }
        let sy = ((dy * i64::from(h)) / dh).clamp(0, i64::from(h) - 1) as usize;
        for dx in 0..dw {
            let tx = xs + dx;
            if tx < 0 || tx >= pw {
                continue;
            }
            let sx = ((dx * i64::from(w)) / dw).clamp(0, i64::from(w) - 1) as usize;
            let s = &px[(sy * w as usize + sx) * 4..][..4];
            let a = u32::from(s[3]);
            let d = &mut data[((ty * pw + tx) * 4) as usize..][..4];
            // Source over, premultiplied destination.
            for k in 0..3 {
                d[k] = ((u32::from(s[k]) * a + u32::from(d[k]) * (255 - a)) / 255) as u8;
            }
            d[3] = (a + u32::from(d[3]) * (255 - a) / 255) as u8;
        }
    }
}
