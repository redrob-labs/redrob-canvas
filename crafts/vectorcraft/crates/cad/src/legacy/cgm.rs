//! Computer Graphics Metafiles (ISO 8632): the binary encoding (part 3) and the clear-text
//! encoding (part 4). Read: the metafile and picture delimiters, the precisions and the VDC
//! extent, colour selection mode and colour table, and the primitives polyline, disjoint
//! polyline, polygon, polygon set, rectangle, circle, circular arcs, ellipse, elliptical arcs
//! and text, with line, fill, edge and text colours, line and edge widths, interior style and
//! edge visibility. VDC space is y up; the picture is fitted to its VDC extent.

use kurbo::BezPath;
use redrob_vector_geom::{Affine, Point, Rect};

use super::{Builder, Bytes, Imported, Stroke, arc_to, polyline, rect_path};

/// Does `bytes` start a CGM file (a BEGIN METAFILE element, binary or clear text)?
pub fn is_cgm(bytes: &[u8]) -> bool {
    if bytes.len() >= 2 {
        let h = u16::from_be_bytes([bytes[0], bytes[1]]);
        // Class 0, id 1: BEGIN METAFILE.
        if h >> 12 == 0 && (h >> 5) & 0x7f == 1 {
            return true;
        }
    }
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_ascii_uppercase();
    head.trim_start().starts_with("BEGMF") || head.contains("BEGMF ")
}

#[derive(Clone, Copy, PartialEq)]
enum Real {
    Float32,
    Float64,
    Fixed32,
    Fixed64,
}

struct State {
    b: Builder,
    vdc_real: bool,
    int_bits: u32,
    vdc_int_bits: u32,
    real: Real,
    vdc_real_prec: Real,
    color_bits: u32,
    index_bits: u32,
    direct_colors: bool,
    color_extent: ([f64; 3], [f64; 3]),
    table: Vec<[u8; 3]>,
    extent: Option<Rect>,
    line_color: [u8; 3],
    fill_color: [u8; 3],
    edge_color: [u8; 3],
    text_color: [u8; 3],
    line_width: f64,
    edge_width: f64,
    /// Line width is a fraction of the extent (scaled mode).
    line_scaled: bool,
    edge_scaled: bool,
    /// 0 hollow, 1 solid, 2 pattern, 3 hatch, 4 empty.
    interior: u32,
    edge_visible: bool,
    char_height: f64,
    line_dash: Vec<f64>,
    skipped: usize,
}

impl State {
    fn new() -> Self {
        let mut table = vec![[255, 255, 255], [0, 0, 0], [255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0], [255, 0, 255], [0, 255, 255]];
        table.resize(256, [0, 0, 0]);
        Self {
            b: Builder::new(),
            vdc_real: false,
            int_bits: 16,
            vdc_int_bits: 16,
            real: Real::Fixed32,
            vdc_real_prec: Real::Fixed32,
            color_bits: 8,
            index_bits: 8,
            direct_colors: false,
            color_extent: ([0.0; 3], [255.0; 3]),
            table,
            extent: None,
            line_color: [0, 0, 0],
            fill_color: [0, 0, 0],
            edge_color: [0, 0, 0],
            text_color: [0, 0, 0],
            line_width: 1.0,
            edge_width: 1.0,
            line_scaled: true,
            edge_scaled: true,
            interior: 0,
            edge_visible: false,
            char_height: 0.0,
            line_dash: vec![],
            skipped: 0,
        }
    }

    /// The width of a line in VDC units.
    fn width(&self, w: f64, scaled: bool) -> f64 {
        let base = self.extent.map(|e| e.width().abs().max(e.height().abs()) / 1000.0).unwrap_or(1.0);
        if scaled { (w * base).max(base * 0.5) } else { w.max(base * 0.25) }
    }

    fn stroke(&self) -> Stroke {
        Stroke { dash: self.line_dash.clone(), ..Stroke::new(self.line_color, self.width(self.line_width, self.line_scaled)) }
    }

    fn edge(&self) -> Option<Stroke> {
        self.edge_visible.then(|| Stroke::new(self.edge_color, self.width(self.edge_width, self.edge_scaled)))
    }

    fn fill(&self) -> Option<[u8; 3]> {
        match self.interior {
            1 | 2 | 3 => Some(self.fill_color),
            _ => None,
        }
    }

    fn filled(&mut self, bp: BezPath) {
        let (fill, edge) = (self.fill(), self.edge());
        if fill.is_none() && edge.is_none() {
            // A hollow interior draws its outline in the line colour.
            let st = self.stroke();
            self.b.path(bp, None, Some(st), false);
        } else {
            self.b.path(bp, fill, edge, false);
        }
    }

    // ---------- binary parameter readers ----------

    fn int(&self, r: &mut Bytes, bits: u32) -> Option<i64> {
        Some(match bits {
            8 => r.u8()? as i8 as i64,
            16 => r.i16be()? as i64,
            24 => {
                let b = r.take(3)?;
                let v = (i32::from(b[0] as i8) << 16) | (i32::from(b[1]) << 8) | i32::from(b[2]);
                v as i64
            }
            32 => r.i32be()? as i64,
            _ => r.i16be()? as i64,
        })
    }

    fn uint(&self, r: &mut Bytes, bits: u32) -> Option<u64> {
        Some(match bits {
            8 => u64::from(r.u8()?),
            16 => u64::from(r.u16be()?),
            24 => {
                let b = r.take(3)?;
                (u64::from(b[0]) << 16) | (u64::from(b[1]) << 8) | u64::from(b[2])
            }
            32 => u64::from(r.u32be()?),
            _ => u64::from(r.u8()?),
        })
    }

    fn real(&self, r: &mut Bytes, prec: Real) -> Option<f64> {
        Some(match prec {
            Real::Float32 => f64::from(f32::from_bits(r.u32be()?)),
            Real::Float64 => {
                let b = r.take(8)?;
                f64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
            }
            Real::Fixed32 => {
                let whole = r.i16be()?;
                let frac = r.u16be()?;
                f64::from(whole) + f64::from(frac) / 65536.0
            }
            Real::Fixed64 => {
                let whole = r.i32be()?;
                let frac = r.u32be()?;
                f64::from(whole) + f64::from(frac) / 4_294_967_296.0
            }
        })
    }

    fn vdc(&self, r: &mut Bytes) -> Option<f64> {
        if self.vdc_real { self.real(r, self.vdc_real_prec) } else { self.int(r, self.vdc_int_bits).map(|v| v as f64) }
    }

    fn point(&self, r: &mut Bytes) -> Option<Point> {
        let x = self.vdc(r)?;
        let y = self.vdc(r)?;
        Some(Point::new(x, y))
    }

    fn points(&self, r: &mut Bytes) -> Vec<Point> {
        let mut out = vec![];
        while let Some(p) = self.point(r) {
            out.push(p);
            if out.len() > 1_000_000 {
                break;
            }
        }
        out
    }

    fn color(&self, r: &mut Bytes) -> Option<[u8; 3]> {
        if self.direct_colors {
            let mut c = [0u8; 3];
            for (i, v) in c.iter_mut().enumerate() {
                let raw = self.uint(r, self.color_bits)? as f64;
                let (lo, hi) = (self.color_extent.0[i], self.color_extent.1[i]);
                let t = if hi > lo { (raw - lo) / (hi - lo) } else { raw / 255.0 };
                *v = (t.clamp(0.0, 1.0) * 255.0).round() as u8;
            }
            Some(c)
        } else {
            let i = self.uint(r, self.index_bits)? as usize;
            Some(self.table.get(i).copied().unwrap_or([0, 0, 0]))
        }
    }

    fn string(&self, r: &mut Bytes) -> Option<String> {
        let mut n = usize::from(r.u8()?);
        let mut out = vec![];
        loop {
            let more = if n == 255 {
                let l = r.u16be()?;
                n = usize::from(l & 0x7fff);
                l & 0x8000 != 0
            } else {
                false
            };
            out.extend_from_slice(r.take(n)?);
            if !more {
                break;
            }
            n = 255;
        }
        Some(String::from_utf8_lossy(&out).into_owned())
    }

    fn element(&mut self, class: u16, id: u16, data: &[u8]) {
        let mut r = Bytes::new(data);
        match (class, id) {
            (0, 1) | (0, 2) | (0, 3) | (0, 4) | (0, 5) => {}
            (1, 3) => self.vdc_real = self.int(&mut r, 16) == Some(1),
            (1, 4) => self.int_bits = self.int(&mut r, 16).unwrap_or(16) as u32,
            (1, 5) => {
                let form = self.int(&mut r, 16).unwrap_or(1);
                let exp = self.int(&mut r, 16).unwrap_or(16);
                let frac = self.int(&mut r, 16).unwrap_or(16);
                self.real = real_prec(form, exp, frac);
            }
            (1, 6) => self.index_bits = self.int(&mut r, 16).unwrap_or(16) as u32,
            (1, 7) => self.color_bits = self.int(&mut r, 16).unwrap_or(8) as u32,
            (1, 8) => self.index_bits = self.int(&mut r, 16).unwrap_or(8) as u32,
            (1, 10) => {
                let mut lo = [0.0; 3];
                let mut hi = [255.0; 3];
                for v in &mut lo {
                    *v = self.uint(&mut r, self.color_bits).unwrap_or(0) as f64;
                }
                for v in &mut hi {
                    *v = self.uint(&mut r, self.color_bits).unwrap_or(255) as f64;
                }
                self.color_extent = (lo, hi);
            }
            (2, 2) => self.direct_colors = self.int(&mut r, 16) == Some(1),
            (2, 3) => self.line_scaled = self.int(&mut r, 16) != Some(0),
            (2, 5) => self.edge_scaled = self.int(&mut r, 16) != Some(0),
            (2, 6) => {
                if let (Some(a), Some(b)) = (self.point(&mut r), self.point(&mut r)) {
                    self.extent = Some(Rect::from_points(a, b));
                }
            }
            (3, 1) => self.vdc_int_bits = self.int(&mut r, 16).unwrap_or(16) as u32,
            (3, 2) => {
                let form = self.int(&mut r, 16).unwrap_or(1);
                let exp = self.int(&mut r, 16).unwrap_or(16);
                let frac = self.int(&mut r, 16).unwrap_or(16);
                self.vdc_real_prec = real_prec(form, exp, frac);
            }
            (4, 1) => {
                let pts = self.points(&mut r);
                let st = self.stroke();
                self.b.path(polyline(&pts, false), None, Some(st), false);
            }
            (4, 2) => {
                let pts = self.points(&mut r);
                let st = self.stroke();
                for pair in pts.chunks(2) {
                    if pair.len() == 2 {
                        self.b.path(polyline(pair, false), None, Some(st.clone()), false);
                    }
                }
            }
            (4, 4) | (4, 5) => {
                if id == 5 {
                    // Restricted text: the box first.
                    let _ = self.vdc(&mut r);
                    let _ = self.vdc(&mut r);
                }
                let at = self.point(&mut r);
                let _final = self.int(&mut r, 16);
                if let (Some(at), Some(s)) = (at, self.string(&mut r)) {
                    let h = if self.char_height > 0.0 { self.char_height } else { self.extent.map(|e| e.height().abs() / 100.0).unwrap_or(10.0) };
                    self.b.text(at, &s, h, self.text_color, 0.0);
                }
            }
            (4, 7) => {
                let pts = self.points(&mut r);
                self.filled(polyline(&pts, true));
            }
            (4, 8) => {
                // Points each followed by an edge-out flag; a flag of 2 or 3 closes a subpath.
                let mut bp = BezPath::new();
                let mut open = false;
                while let Some(p) = self.point(&mut r) {
                    let Some(flag) = self.int(&mut r, 16) else { break };
                    if open {
                        bp.line_to(p);
                    } else {
                        bp.move_to(p);
                        open = true;
                    }
                    if flag >= 2 {
                        bp.close_path();
                        open = false;
                    }
                }
                if open {
                    bp.close_path();
                }
                self.filled(bp);
            }
            (4, 11) => {
                if let (Some(a), Some(b)) = (self.point(&mut r), self.point(&mut r)) {
                    self.filled(rect_path(a, b));
                }
            }
            (4, 12) => {
                if let (Some(c), Some(rad)) = (self.point(&mut r), self.vdc(&mut r)) {
                    self.filled(super::ellipse(c, rad, rad, 0.0));
                }
            }
            (4, 13) | (4, 14) => {
                let (a, b, c) = (self.point(&mut r), self.point(&mut r), self.point(&mut r));
                if let (Some(a), Some(b), Some(c)) = (a, b, c)
                    && let Some(bp) = arc_3point(a, b, c, id == 14 && self.int(&mut r, 16) == Some(0))
                {
                    if id == 14 {
                        self.filled(bp)
                    } else {
                        let st = self.stroke();
                        self.b.path(bp, None, Some(st), false);
                    }
                }
            }
            (4, 15) | (4, 16) => {
                let c = self.point(&mut r);
                let (dxs, dys, dxe, dye) = (self.vdc(&mut r), self.vdc(&mut r), self.vdc(&mut r), self.vdc(&mut r));
                let rad = self.vdc(&mut r);
                if let (Some(c), Some(dxs), Some(dys), Some(dxe), Some(dye), Some(rad)) = (c, dxs, dys, dxe, dye, rad) {
                    let start = dys.atan2(dxs);
                    let end = dye.atan2(dxe);
                    let sweep = (end - start).rem_euclid(std::f64::consts::TAU);
                    let mut bp = BezPath::new();
                    if id == 16 && self.int(&mut r, 16) == Some(0) {
                        bp.move_to(c);
                    }
                    arc_to(&mut bp, c, rad, rad, start, sweep, 0.0);
                    if id == 16 {
                        bp.close_path();
                        self.filled(bp);
                    } else {
                        let st = self.stroke();
                        self.b.path(bp, None, Some(st), false);
                    }
                }
            }
            (4, 17) | (4, 18) | (4, 19) => {
                let c = self.point(&mut r);
                let (p1, p2) = (self.point(&mut r), self.point(&mut r));
                if let (Some(c), Some(p1), Some(p2)) = (c, p1, p2) {
                    // Conjugate diameter end points: the ellipse axes and rotation.
                    let (v1, v2) = (p1 - c, p2 - c);
                    let (rx, ry, rot) = conjugate_to_axes(v1, v2);
                    if id == 17 {
                        self.filled(super::ellipse(c, rx, ry, rot));
                    } else {
                        let (dxs, dys, dxe, dye) = (self.vdc(&mut r), self.vdc(&mut r), self.vdc(&mut r), self.vdc(&mut r));
                        if let (Some(dxs), Some(dys), Some(dxe), Some(dye)) = (dxs, dys, dxe, dye) {
                            let to_param = |dx: f64, dy: f64| {
                                // The angle in the ellipse's own frame.
                                let (s, co) = rot.sin_cos();
                                let lx = dx * co + dy * s;
                                let ly = -dx * s + dy * co;
                                (ly / ry.max(1e-9)).atan2(lx / rx.max(1e-9))
                            };
                            let start = to_param(dxs, dys);
                            let sweep = (to_param(dxe, dye) - start).rem_euclid(std::f64::consts::TAU);
                            let mut bp = BezPath::new();
                            if id == 19 && self.int(&mut r, 16) == Some(0) {
                                bp.move_to(c);
                            }
                            arc_to(&mut bp, c, rx, ry, start, sweep, rot);
                            if id == 19 {
                                bp.close_path();
                                self.filled(bp);
                            } else {
                                let st = self.stroke();
                                self.b.path(bp, None, Some(st), false);
                            }
                        }
                    }
                }
            }
            (5, 2) => {
                self.line_dash = match self.int(&mut r, self.index_bits.max(16)).unwrap_or(1) {
                    2 => vec![4.0, 2.0],
                    3 => vec![1.0, 2.0],
                    4 => vec![4.0, 2.0, 1.0, 2.0],
                    5 => vec![4.0, 2.0, 1.0, 2.0, 1.0, 2.0],
                    _ => vec![],
                };
                let w = self.width(self.line_width, self.line_scaled);
                self.line_dash = self.line_dash.iter().map(|d| d * w).collect();
            }
            (5, 3) => {
                if let Some(w) = if self.line_scaled { self.real(&mut r, self.real) } else { self.vdc(&mut r) } {
                    self.line_width = w;
                }
            }
            (5, 4) => {
                if let Some(c) = self.color(&mut r) {
                    self.line_color = c;
                }
            }
            (5, 14) => {
                if let Some(c) = self.color(&mut r) {
                    self.text_color = c;
                }
            }
            (5, 15) => {
                if let Some(h) = self.vdc(&mut r) {
                    self.char_height = h;
                }
            }
            (5, 22) => self.interior = self.int(&mut r, 16).unwrap_or(0) as u32,
            (5, 23) => {
                if let Some(c) = self.color(&mut r) {
                    self.fill_color = c;
                }
            }
            (5, 28) => {
                if let Some(w) = if self.edge_scaled { self.real(&mut r, self.real) } else { self.vdc(&mut r) } {
                    self.edge_width = w;
                }
            }
            (5, 29) => {
                if let Some(c) = self.color(&mut r) {
                    self.edge_color = c;
                }
            }
            (5, 30) => self.edge_visible = self.int(&mut r, 16) == Some(1),
            (5, 34) => {
                let Some(start) = self.uint(&mut r, self.index_bits) else { return };
                let mut i = start as usize;
                let was = self.direct_colors;
                self.direct_colors = true;
                while let Some(c) = self.color(&mut r) {
                    if i < self.table.len() {
                        self.table[i] = c;
                    } else if i < 4096 {
                        self.table.resize(i + 1, [0, 0, 0]);
                        self.table[i] = c;
                    }
                    i += 1;
                }
                self.direct_colors = was;
            }
            (1, _) | (2, _) | (3, _) | (5, _) | (6, _) | (7, _) | (9, _) => {}
            (4, 3) | (4, 6) => {}
            (4, 9) => self.b.warn("cell arrays (raster cells) aren't read: they are left out"),
            _ => self.skipped += 1,
        }
    }
}

fn real_prec(form: i64, exp: i64, frac: i64) -> Real {
    match (form, exp + frac) {
        (0, b) if b <= 32 => Real::Float32,
        (0, _) => Real::Float64,
        (_, b) if b <= 32 => Real::Fixed32,
        _ => Real::Fixed64,
    }
}

/// The ellipse with conjugate diameter vectors `v1`, `v2` → (rx, ry, rotation).
fn conjugate_to_axes(v1: redrob_vector_geom::Vec2, v2: redrob_vector_geom::Vec2) -> (f64, f64, f64) {
    // The ellipse is the image of the unit circle under [v1 v2]: its axes are the singular values.
    let (a, b, c, d) = (v1.x, v2.x, v1.y, v2.y);
    let s1 = a * a + b * b + c * c + d * d;
    let s2 = ((a * a + b * b - c * c - d * d).powi(2) + 4.0 * (a * c + b * d).powi(2)).sqrt();
    let rx = ((s1 + s2) / 2.0).max(0.0).sqrt();
    let ry = ((s1 - s2) / 2.0).max(0.0).sqrt();
    let rot = 0.5 * (2.0 * (a * c + b * d)).atan2(a * a + b * b - c * c - d * d);
    (rx.max(1e-9), ry.max(1e-9), rot)
}

/// The arc through `a`, `b`, `c` (closed as a pie through the centre when `pie`).
fn arc_3point(a: Point, b: Point, c: Point, pie: bool) -> Option<BezPath> {
    let d = 2.0 * (a.x * (b.y - c.y) + b.x * (c.y - a.y) + c.x * (a.y - b.y));
    if d.abs() < 1e-12 {
        return Some(polyline(&[a, b, c], false));
    }
    let (a2, b2, c2) = (a.x * a.x + a.y * a.y, b.x * b.x + b.y * b.y, c.x * c.x + c.y * c.y);
    let ux = (a2 * (b.y - c.y) + b2 * (c.y - a.y) + c2 * (a.y - b.y)) / d;
    let uy = (a2 * (c.x - b.x) + b2 * (a.x - c.x) + c2 * (b.x - a.x)) / d;
    let center = Point::new(ux, uy);
    let r = (a - center).hypot();
    let (sa, sb, sc) = ((a - center).atan2(), (b - center).atan2(), (c - center).atan2());
    // Sweep from a through b to c.
    let ccw = (sb - sa).rem_euclid(std::f64::consts::TAU) < (sc - sa).rem_euclid(std::f64::consts::TAU);
    let sweep = if ccw { (sc - sa).rem_euclid(std::f64::consts::TAU) } else { -((sa - sc).rem_euclid(std::f64::consts::TAU)) };
    let mut bp = BezPath::new();
    if pie {
        bp.move_to(center);
    }
    arc_to(&mut bp, center, r, r, sa, sweep, 0.0);
    Some(bp)
}

/// Read a CGM file into a document.
pub fn import(bytes: &[u8], title: &str) -> Result<Imported, String> {
    let mut st = State::new();
    let head = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_ascii_uppercase();
    if head.trim_start().starts_with("BEGMF") || (!is_binary_start(bytes) && head.contains("BEGMF")) {
        clear_text(&mut st, &String::from_utf8_lossy(bytes));
    } else {
        binary(&mut st, bytes)?;
    }
    for _ in 0..st.skipped {
        st.b.skip();
    }
    let Some(fb) = st.b.bounds() else {
        return Err("the metafile draws nothing Redrob Canvas reads".into());
    };
    let extent = st.extent.map(|e| e.abs()).filter(|e| e.width() > 0.0 && e.height() > 0.0).unwrap_or(fb);
    // Fit the VDC extent to a letter-sized page, y up → y down.
    let scale = (612.0 / extent.width()).min(792.0 / extent.height());
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let map = Affine::scale(scale) * Affine::new([1.0, 0.0, 0.0, -1.0, -extent.x0, extent.y1]);
    let board = Rect::new(0.0, 0.0, extent.width() * scale, extent.height() * scale);
    Ok(st.b.finish(map, Some(board), title))
}

fn is_binary_start(bytes: &[u8]) -> bool {
    bytes.len() >= 2 && {
        let h = u16::from_be_bytes([bytes[0], bytes[1]]);
        h >> 12 == 0 && (h >> 5) & 0x7f == 1
    }
}

fn binary(st: &mut State, bytes: &[u8]) -> Result<(), String> {
    let mut r = Bytes::new(bytes);
    let mut any = false;
    while r.left() >= 2 {
        let Some(h) = r.u16be() else { break };
        let class = h >> 12;
        let id = (h >> 5) & 0x7f;
        let len = usize::from(h & 0x1f);
        let mut data: Vec<u8> = vec![];
        if len == 31 {
            // Long form: partitions, each with a continue flag.
            loop {
                let Some(l) = r.u16be() else { break };
                let n = usize::from(l & 0x7fff);
                let Some(part) = r.take(n) else { break };
                data.extend_from_slice(part);
                if n % 2 == 1 {
                    let _ = r.u8();
                }
                if l & 0x8000 == 0 {
                    break;
                }
            }
        } else {
            let Some(part) = r.take(len) else { break };
            data.extend_from_slice(part);
            if len % 2 == 1 {
                let _ = r.u8();
            }
        }
        any = true;
        st.element(class, id, &data);
        if class == 0 && id == 2 {
            break;
        }
    }
    if !any {
        return Err("not a CGM file".into());
    }
    Ok(())
}

// ---------- clear-text encoding ----------

fn clear_text(st: &mut State, text: &str) {
    // Elements end with `;` (or `/`); comments are quoted with `%`.
    let mut cleaned = String::new();
    let mut in_comment = false;
    let mut in_string: Option<char> = None;
    for c in text.chars() {
        if let Some(q) = in_string {
            cleaned.push(c);
            if c == q {
                in_string = None;
            }
            continue;
        }
        if in_comment {
            if c == '%' {
                in_comment = false;
            }
            continue;
        }
        match c {
            '%' => in_comment = true,
            '"' | '\'' => {
                in_string = Some(c);
                cleaned.push(c);
            }
            _ => cleaned.push(c),
        }
    }
    for el in cleaned.split([';', '/']) {
        let el = el.trim();
        if el.is_empty() {
            continue;
        }
        let (name, rest) = el.split_once(|c: char| c.is_whitespace()).unwrap_or((el, ""));
        let name = name.to_ascii_uppercase();
        let (nums, strings) = clear_params(rest);
        let pts = |nums: &[f64]| nums.chunks(2).filter(|c| c.len() == 2).map(|c| Point::new(c[0], c[1])).collect::<Vec<_>>();
        let color = |st: &State, nums: &[f64]| -> Option<[u8; 3]> {
            if st.direct_colors {
                if let [r, g, b, ..] = nums {
                    let q = |v: f64, i: usize| {
                        let (lo, hi) = (st.color_extent.0[i], st.color_extent.1[i]);
                        let t = if hi > lo { (v - lo) / (hi - lo) } else { v / 255.0 };
                        (t.clamp(0.0, 1.0) * 255.0).round() as u8
                    };
                    return Some([q(*r, 0), q(*g, 1), q(*b, 2)]);
                }
                None
            } else {
                nums.first().and_then(|i| st.table.get(*i as usize).copied())
            }
        };
        match name.as_str() {
            "BEGMF" | "ENDMF" | "BEGPIC" | "BEGPICBODY" | "ENDPIC" | "MFVERSION" | "MFDESC" | "MFELEMLIST" => {}
            "VDCEXT" => {
                let p = pts(&nums);
                if p.len() >= 2 {
                    st.extent = Some(Rect::from_points(p[0], p[1]));
                }
            }
            "COLRMODE" => st.direct_colors = rest.trim().eq_ignore_ascii_case("DIRECT"),
            "COLRVALUEEXT" => {
                if nums.len() >= 6 {
                    st.color_extent = ([nums[0], nums[1], nums[2]], [nums[3], nums[4], nums[5]]);
                }
            }
            "COLRTABLE" => {
                if let Some((first, rest)) = nums.split_first() {
                    let mut i = *first as usize;
                    for c in rest.chunks(3) {
                        if c.len() == 3 && i < 4096 {
                            if i >= st.table.len() {
                                st.table.resize(i + 1, [0, 0, 0]);
                            }
                            let q = |v: f64| v.clamp(0.0, 255.0).round() as u8;
                            st.table[i] = [q(c[0]), q(c[1]), q(c[2])];
                        }
                        i += 1;
                    }
                }
            }
            "LINEWIDTHMODE" => st.line_scaled = !rest.trim().eq_ignore_ascii_case("ABS") && !rest.trim().eq_ignore_ascii_case("ABSOLUTE"),
            "EDGEWIDTHMODE" => st.edge_scaled = !rest.trim().eq_ignore_ascii_case("ABS") && !rest.trim().eq_ignore_ascii_case("ABSOLUTE"),
            "LINE" | "INCRLINE" => {
                let mut p = pts(&nums);
                if name == "INCRLINE" {
                    accumulate(&mut p);
                }
                let s = st.stroke();
                st.b.path(polyline(&p, false), None, Some(s), false);
            }
            "DISJTLINE" => {
                let p = pts(&nums);
                let s = st.stroke();
                for pair in p.chunks(2) {
                    if pair.len() == 2 {
                        st.b.path(polyline(pair, false), None, Some(s.clone()), false);
                    }
                }
            }
            "POLYGON" | "INCRPOLYGON" => {
                let mut p = pts(&nums);
                if name == "INCRPOLYGON" {
                    accumulate(&mut p);
                }
                st.filled(polyline(&p, true));
            }
            "RECT" => {
                let p = pts(&nums);
                if p.len() >= 2 {
                    st.filled(rect_path(p[0], p[1]));
                }
            }
            "CIRCLE" => {
                if let [x, y, r, ..] = nums.as_slice() {
                    st.filled(super::ellipse(Point::new(*x, *y), *r, *r, 0.0));
                }
            }
            "ELLIPSE" => {
                if let [x, y, x1, y1, x2, y2, ..] = nums.as_slice() {
                    let c = Point::new(*x, *y);
                    let (rx, ry, rot) = conjugate_to_axes(Point::new(*x1, *y1) - c, Point::new(*x2, *y2) - c);
                    st.filled(super::ellipse(c, rx, ry, rot));
                }
            }
            "ARCCTR" | "ARCCTRCLOSE" => {
                if let [x, y, dxs, dys, dxe, dye, r, ..] = nums.as_slice() {
                    let c = Point::new(*x, *y);
                    let start = dys.atan2(*dxs);
                    let sweep = (dye.atan2(*dxe) - start).rem_euclid(std::f64::consts::TAU);
                    let mut bp = BezPath::new();
                    if name == "ARCCTRCLOSE" && rest.to_ascii_uppercase().contains("PIE") {
                        bp.move_to(c);
                    }
                    arc_to(&mut bp, c, *r, *r, start, sweep, 0.0);
                    if name == "ARCCTRCLOSE" {
                        bp.close_path();
                        st.filled(bp);
                    } else {
                        let s = st.stroke();
                        st.b.path(bp, None, Some(s), false);
                    }
                }
            }
            "ARC3PT" | "ARC3PTCLOSE" => {
                let p = pts(&nums);
                if p.len() >= 3
                    && let Some(bp) = arc_3point(p[0], p[1], p[2], name == "ARC3PTCLOSE" && rest.to_ascii_uppercase().contains("PIE"))
                {
                    if name == "ARC3PTCLOSE" {
                        st.filled(bp);
                    } else {
                        let s = st.stroke();
                        st.b.path(bp, None, Some(s), false);
                    }
                }
            }
            "TEXT" | "RESTRTEXT" => {
                let p = pts(&nums);
                if let (Some(at), Some(s)) = (p.last(), strings.first()) {
                    let h = if st.char_height > 0.0 { st.char_height } else { st.extent.map(|e| e.height().abs() / 100.0).unwrap_or(10.0) };
                    st.b.text(*at, s, h, st.text_color, 0.0);
                }
            }
            "LINECOLR" => {
                if let Some(c) = color(st, &nums) {
                    st.line_color = c;
                }
            }
            "FILLCOLR" => {
                if let Some(c) = color(st, &nums) {
                    st.fill_color = c;
                }
            }
            "EDGECOLR" => {
                if let Some(c) = color(st, &nums) {
                    st.edge_color = c;
                }
            }
            "TEXTCOLR" => {
                if let Some(c) = color(st, &nums) {
                    st.text_color = c;
                }
            }
            "LINEWIDTH" => {
                if let Some(w) = nums.first() {
                    st.line_width = *w;
                }
            }
            "EDGEWIDTH" => {
                if let Some(w) = nums.first() {
                    st.edge_width = *w;
                }
            }
            "CHARHEIGHT" => {
                if let Some(h) = nums.first() {
                    st.char_height = *h;
                }
            }
            "INTSTYLE" => {
                st.interior = match rest.trim().to_ascii_uppercase().as_str() {
                    "SOLID" => 1,
                    "PAT" | "PATTERN" => 2,
                    "HATCH" => 3,
                    "EMPTY" => 4,
                    _ => 0,
                };
            }
            "EDGEVIS" => st.edge_visible = rest.trim().eq_ignore_ascii_case("ON"),
            "LINETYPE" => {
                let w = st.width(st.line_width, st.line_scaled);
                st.line_dash = match nums.first().map(|n| *n as i64) {
                    Some(2) => vec![4.0 * w, 2.0 * w],
                    Some(3) => vec![w, 2.0 * w],
                    Some(4) => vec![4.0 * w, 2.0 * w, w, 2.0 * w],
                    Some(5) => vec![4.0 * w, 2.0 * w, w, 2.0 * w, w, 2.0 * w],
                    _ => vec![],
                };
            }
            "CELLARRAY" => st.b.warn("cell arrays (raster cells) aren't read: they are left out"),
            "VDCTYPE" | "INTEGERPREC" | "REALPREC" | "INDEXPREC" | "COLRPREC" | "COLRINDEXPREC" | "MAXCOLRINDEX" | "SCALEMODE" | "VDCINTEGERPREC"
            | "VDCREALPREC" | "BACKCOLR" | "CLIPRECT" | "CLIP" | "TRANSPARENCY" | "AUXCOLR" | "FONTLIST" | "CHARSETLIST" | "CHARCODING"
            | "MFDEFAULTS" | "BEGMFDEFAULTS" | "ENDMFDEFAULTS" | "TEXTFONTINDEX" | "TEXTPREC" | "CHAREXPAN" | "CHARSPACE" | "CHARORI"
            | "TEXTPATH" | "TEXTALIGN" | "CHARSETINDEX" | "ALTCHARSETINDEX" | "MARKERTYPE" | "MARKERSIZE" | "MARKERCOLR" | "HATCHINDEX"
            | "PATINDEX" | "EDGETYPE" | "FILLREFPT" | "PATTABLE" | "PATSIZE" | "ASF" | "LINECAP" | "LINEJOIN" | "EDGECAP" | "EDGEJOIN"
            | "MITRELIMIT" | "BEGAPS" | "ENDAPS" | "BEGAPSBODY" | "APSATTR" | "MARKER" | "POLYMARKER" | "APPLDATA" | "ESCAPE" | "MESSAGE" => {}
            _ => st.skipped += 1,
        }
    }
}

/// Relative points made absolute.
fn accumulate(p: &mut [Point]) {
    for i in 1..p.len() {
        p[i] = Point::new(p[i - 1].x + p[i].x, p[i - 1].y + p[i].y);
    }
}

/// The numbers and quoted strings of a clear-text element's parameters (`(x, y)` pairs and
/// keywords between them aside).
fn clear_params(rest: &str) -> (Vec<f64>, Vec<String>) {
    let mut nums = vec![];
    let mut strings = vec![];
    let mut chars = rest.chars().peekable();
    let mut token = String::new();
    let flush = |token: &mut String, nums: &mut Vec<f64>| {
        if !token.is_empty() {
            if let Ok(v) = token.parse::<f64>()
                && v.is_finite()
            {
                nums.push(v);
            }
            token.clear();
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '"' | '\'' => {
                flush(&mut token, &mut nums);
                let mut s = String::new();
                for d in chars.by_ref() {
                    if d == c {
                        break;
                    }
                    s.push(d);
                }
                strings.push(s);
            }
            '(' | ')' | ',' => flush(&mut token, &mut nums),
            c if c.is_whitespace() => flush(&mut token, &mut nums),
            c => token.push(c),
        }
    }
    flush(&mut token, &mut nums);
    (nums, strings)
}
