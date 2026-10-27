//! Helpers shared by the live path effects of [`crate::lpe_geo`], [`crate::lpe_draw`] and
//! [`crate::lpe_link`]: arc-length measured subpaths and the pieces cut from them, similarity
//! transforms between two segments, and a small line font that draws labels as geometry (the
//! effects produce paths only, so a measurement reads as strokes, not type).

use redrob_vector_geom::kurbo::ParamCurveArclen;
use redrob_vector_geom::{Affine, Anchor, AnchorKind, CubicBez, ParamCurve, PathData, Point, SubPath, Vec2};
use serde_json::Value;

use crate::util::num;

/// Arc-length accuracy of the measured pieces (document points).
pub(crate) const ARCLEN: f64 = 1e-5;

/// Every anchor point of `path`, subpath by subpath.
pub(crate) fn points(path: &PathData) -> Vec<Point> {
    path.subpaths.iter().flat_map(|sp| sp.anchors.iter().map(|a| a.p)).collect()
}

/// `v` at unit length (`None` for a vector without length).
pub(crate) fn unit(v: Vec2) -> Option<Vec2> {
    let l = v.hypot();
    (l > 1e-12 && l.is_finite()).then(|| v / l)
}

/// The left normal of `v` (y down: the left of the direction of travel), at unit length.
pub(crate) fn left(v: Vec2) -> Option<Vec2> {
    unit(v).map(|u| Vec2::new(u.y, -u.x))
}

/// An open straight subpath from `a` to `b`.
pub(crate) fn line(a: Point, b: Point) -> SubPath {
    SubPath::polyline(&[a, b], false)
}

/// The similarity (rotation, uniform scale and translation) taking segment `a0 → a1` onto
/// `b0 → b1`; the identity when `a0 == a1`.
pub(crate) fn similarity(a0: Point, a1: Point, b0: Point, b1: Point) -> Affine {
    let (u, v) = (a1 - a0, b1 - b0);
    let lu = u.hypot2();
    if lu <= 1e-18 {
        return Affine::translate(b0 - a0);
    }
    // v = (s e^{iθ}) u: the complex ratio v / u.
    let re = (v.x * u.x + v.y * u.y) / lu;
    let im = (v.y * u.x - v.x * u.y) / lu;
    let lin = Affine::new([re, im, -im, re, 0.0, 0.0]);
    Affine::translate(b0.to_vec2()) * lin * Affine::translate(-a0.to_vec2())
}

/// The affine map taking `s0, s1, s2` onto `t0, t1, t2` (`None` when the sources are collinear).
pub(crate) fn affine_3(s: [Point; 3], t: [Point; 3]) -> Option<Affine> {
    let (u, v) = (s[1] - s[0], s[2] - s[0]);
    let det = u.x * v.y - u.y * v.x;
    if det.abs() <= 1e-12 || !det.is_finite() {
        return None;
    }
    let (uu, vv) = (t[1] - t[0], t[2] - t[0]);
    // L [u v] = [U V]  ⇒  L = [U V] [u v]⁻¹.
    let inv = [v.y / det, -u.y / det, -v.x / det, u.x / det];
    let a = uu.x * inv[0] + vv.x * inv[1];
    let c = uu.x * inv[2] + vv.x * inv[3];
    let b = uu.y * inv[0] + vv.y * inv[1];
    let d = uu.y * inv[2] + vv.y * inv[3];
    let lin = Affine::new([a, b, c, d, 0.0, 0.0]);
    let moved = lin * s[0];
    Some(Affine::translate(t[0] - moved) * lin)
}

/// A subpath as cubics with the distance along it where each starts.
#[derive(Clone, Debug, Default)]
pub(crate) struct Measured {
    pub cubics: Vec<CubicBez>,
    pub starts: Vec<f64>,
    pub lens: Vec<f64>,
    pub total: f64,
    pub closed: bool,
}

impl Measured {
    pub fn new(sp: &SubPath) -> Self {
        let mut m = Self { closed: sp.closed, ..Default::default() };
        for i in 0..sp.segment_count() {
            let c = sp.segment(i);
            let l = c.arclen(ARCLEN);
            if !l.is_finite() || l <= 1e-9 {
                continue;
            }
            m.starts.push(m.total);
            m.lens.push(l);
            m.cubics.push(c);
            m.total += l;
        }
        m
    }

    pub fn is_empty(&self) -> bool {
        self.cubics.is_empty()
    }

    /// Which cubic distance `s` falls in, and the parameter there.
    fn locate(&self, s: f64) -> Option<(usize, f64)> {
        if self.is_empty() {
            return None;
        }
        let s = if self.closed { s.rem_euclid(self.total.max(1e-12)) } else { s.clamp(0.0, self.total) };
        let i = self.starts.partition_point(|st| *st <= s).saturating_sub(1).min(self.cubics.len() - 1);
        let t = self.cubics[i].inv_arclen((s - self.starts[i]).clamp(0.0, self.lens[i]), ARCLEN).clamp(0.0, 1.0);
        Some((i, t))
    }

    /// The point and unit tangent at distance `s` along the subpath (held at an open one's ends,
    /// round and round a closed one).
    pub fn at(&self, s: f64) -> (Point, Vec2) {
        let Some((i, t)) = self.locate(s) else { return (Point::ZERO, Vec2::new(1.0, 0.0)) };
        let c = &self.cubics[i];
        let (t0, t1) = ((t - 1e-4).max(0.0), (t + 1e-4).min(1.0));
        let d = c.eval(t1) - c.eval(t0);
        (c.eval(t), unit(d).unwrap_or(Vec2::new(1.0, 0.0)))
    }

    /// The piece of the subpath from distance `a` to distance `b` (`a < b`, both within the
    /// length; a closed subpath wraps), as an open subpath.
    pub fn slice(&self, a: f64, b: f64) -> Option<SubPath> {
        if self.is_empty() || b <= a + 1e-9 || !(a.is_finite() && b.is_finite()) {
            return None;
        }
        let (a, b) = if self.closed && self.total > 0.0 {
            let a0 = a.rem_euclid(self.total);
            (a0, a0 + (b - a).min(self.total))
        } else {
            (a.clamp(0.0, self.total), b.clamp(0.0, self.total))
        };
        let mut pieces = Vec::new();
        let n = self.cubics.len();
        let (i0, _) = self.locate(a.min(self.total - 1e-12).max(0.0))?;
        let mut i = i0;
        let mut rounds = 0;
        loop {
            let start = self.starts[i] + (rounds as f64) * self.total;
            let len = self.lens[i];
            let lo = (a - start).max(0.0);
            let hi = (b - start).min(len);
            if hi > lo + 1e-9 {
                let ta = self.cubics[i].inv_arclen(lo, ARCLEN).clamp(0.0, 1.0);
                let tb = self.cubics[i].inv_arclen(hi, ARCLEN).clamp(0.0, 1.0);
                if tb > ta + 1e-9 {
                    pieces.push(self.cubics[i].subsegment(ta..tb));
                }
            }
            if start + len >= b {
                break;
            }
            i += 1;
            if i == n {
                if !self.closed {
                    break;
                }
                i = 0;
                rounds += 1;
                if rounds > 1 {
                    break;
                }
            }
        }
        (!pieces.is_empty()).then(|| chain(&pieces, false))
    }
}

/// Cubics joined end to start as one subpath (the end of each meets the start of the next).
pub(crate) fn chain(pieces: &[CubicBez], closed: bool) -> SubPath {
    let mut anchors: Vec<Anchor> = Vec::with_capacity(pieces.len() + 1);
    for (k, c) in pieces.iter().enumerate() {
        if k == 0 {
            anchors.push(Anchor { p: c.p0, h_in: c.p0, h_out: c.p1, kind: AnchorKind::Corner });
        } else if let Some(last) = anchors.last_mut() {
            last.h_out = c.p1;
        }
        anchors.push(Anchor { p: c.p3, h_in: c.p2, h_out: c.p3, kind: AnchorKind::Corner });
    }
    if closed && anchors.len() > 1 {
        let last = anchors.pop().unwrap_or_else(|| Anchor::corner(Point::ZERO));
        if let Some(first) = anchors.first_mut() {
            first.h_in = last.h_in;
        }
    }
    SubPath::new(anchors, closed)
}

/// The subpath flattened to a polyline (`tol` document points), with the distance along the
/// subpath of each vertex.
pub(crate) fn flatten(m: &Measured, tol: f64) -> Vec<(Point, f64)> {
    let mut out = Vec::new();
    for (k, c) in m.cubics.iter().enumerate() {
        let steps = ((m.lens[k] / tol.max(1e-3)).sqrt() * 2.0).ceil().clamp(2.0, 256.0) as usize;
        let from = if k == 0 { 0 } else { 1 };
        for s in from..=steps {
            let t = s as f64 / steps as f64;
            let d = m.starts[k] + c.subsegment(0.0..t).arclen(ARCLEN * 10.0);
            out.push((c.eval(t), d));
        }
    }
    out
}

/// Where segments `a0a1` and `b0b1` cross strictly inside both: the parameters along each.
pub(crate) fn segment_cross(a0: Point, a1: Point, b0: Point, b1: Point) -> Option<(f64, f64)> {
    let (r, s) = (a1 - a0, b1 - b0);
    let den = r.cross(s);
    if den.abs() <= 1e-12 {
        return None;
    }
    let qp = b0 - a0;
    let t = qp.cross(s) / den;
    let u = qp.cross(r) / den;
    // Half-open on both segments, so a crossing that lands exactly on a polyline vertex is
    // counted once (by the segment that starts there) instead of being dropped twice.
    let on = -1e-9..1.0 - 1e-9;
    (on.contains(&t) && on.contains(&u)).then_some((t, u))
}

/// A number list parameter: `"a b c"`, `"a,b,c"` or `[a, b, c]`.
pub(crate) fn numbers(p: &Value, key: &str) -> Vec<f64> {
    match p.get(key) {
        Some(Value::String(s)) => s.split([' ', ',']).filter(|t| !t.is_empty()).filter_map(|t| t.trim().parse::<f64>().ok()).collect(),
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_f64).collect(),
        Some(Value::Number(n)) => n.as_f64().into_iter().collect(),
        _ => vec![],
    }
    .into_iter()
    .filter(|v| v.is_finite())
    .collect()
}

/// A point list parameter: `"x,y x,y …"` or `[[x, y], …]`.
pub(crate) fn point_list(p: &Value, key: &str) -> Vec<Point> {
    match p.get(key) {
        Some(Value::String(s)) => s
            .split_whitespace()
            .filter_map(|pair| {
                let (x, y) = pair.split_once(',')?;
                let (x, y) = (x.trim().parse::<f64>().ok()?, y.trim().parse::<f64>().ok()?);
                (x.is_finite() && y.is_finite()).then_some(Point::new(x, y))
            })
            .collect(),
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| {
                let a = v.as_array()?;
                let (x, y) = (a.first()?.as_f64()?, a.get(1)?.as_f64()?);
                (x.is_finite() && y.is_finite()).then_some(Point::new(x, y))
            })
            .collect(),
        _ => vec![],
    }
}

/// A point in box coordinates (0,0 the top left, 1,1 the bottom right of `b`) to the document.
pub(crate) fn from_box(b: redrob_vector_geom::Rect, q: Point) -> Point {
    Point::new(b.x0 + q.x * b.width(), b.y0 + q.y * b.height())
}

/// The unit a distance parameter is in, as points per unit: `pt`, `px`, `mm`, `cm`, `in`, `pc`.
pub(crate) fn unit_scale(p: &Value, key: &str) -> f64 {
    match p.get(key).and_then(Value::as_str).unwrap_or("pt").trim().to_ascii_lowercase().as_str() {
        "px" => 0.75,
        "mm" => 72.0 / 25.4,
        "cm" => 72.0 / 2.54,
        "in" | "inch" => 72.0,
        "pc" => 12.0,
        "m" => 72.0 / 0.0254,
        _ => 1.0,
    }
}

/// `n` formatted with `precision` decimals, trailing zeros trimmed (`12.50` → `12.5`, `3.00` → `3`).
pub(crate) fn format_number(n: f64, precision: usize) -> String {
    let s = format!("{n:.precision$}");
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s }
}

/// Deterministic noise in [-1, 1] from the effect's seed (`seed` parameter) and three indices.
pub(crate) fn noise(p: &Value, a: u64, b: u64, c: u64) -> f64 {
    crate::util::noise(num(p, "seed", 0.0).abs() as u64, a, b, c)
}

// ----- a line font for labels (strokes on a 3 × 4 grid, baseline y = 0, cap height 4) -----

/// The strokes of one glyph: polylines on a grid 2 wide and 4 high (y up); `None` for a glyph
/// the font lacks (drawn as a space).
fn glyph(ch: char) -> Option<&'static [&'static [(f64, f64)]]> {
    Some(match ch.to_ascii_lowercase() {
        '0' => &[&[(0.0, 0.0), (2.0, 0.0), (2.0, 4.0), (0.0, 4.0), (0.0, 0.0)]],
        '1' => &[&[(1.0, 0.0), (1.0, 4.0)], &[(0.0, 3.0), (1.0, 4.0)]],
        '2' => &[&[(0.0, 4.0), (2.0, 4.0), (2.0, 2.0), (0.0, 2.0), (0.0, 0.0), (2.0, 0.0)]],
        '3' => &[&[(0.0, 4.0), (2.0, 4.0), (2.0, 0.0), (0.0, 0.0)], &[(0.0, 2.0), (2.0, 2.0)]],
        '4' => &[&[(0.0, 4.0), (0.0, 2.0), (2.0, 2.0)], &[(2.0, 4.0), (2.0, 0.0)]],
        '5' => &[&[(2.0, 4.0), (0.0, 4.0), (0.0, 2.0), (2.0, 2.0), (2.0, 0.0), (0.0, 0.0)]],
        '6' => &[&[(2.0, 4.0), (0.0, 4.0), (0.0, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)]],
        '7' => &[&[(0.0, 4.0), (2.0, 4.0), (2.0, 0.0)]],
        '8' => &[&[(0.0, 0.0), (2.0, 0.0), (2.0, 4.0), (0.0, 4.0), (0.0, 0.0)], &[(0.0, 2.0), (2.0, 2.0)]],
        '9' => &[&[(0.0, 0.0), (2.0, 0.0), (2.0, 4.0), (0.0, 4.0), (0.0, 2.0), (2.0, 2.0)]],
        '.' | ',' => &[&[(1.0, 0.0), (1.0, 0.4)]],
        '-' => &[&[(0.0, 2.0), (2.0, 2.0)]],
        '+' => &[&[(0.0, 2.0), (2.0, 2.0)], &[(1.0, 1.0), (1.0, 3.0)]],
        '%' => &[
            &[(0.0, 0.0), (2.0, 4.0)],
            &[(0.0, 4.0), (0.6, 4.0), (0.6, 3.4), (0.0, 3.4), (0.0, 4.0)],
            &[(1.4, 0.6), (2.0, 0.6), (2.0, 0.0), (1.4, 0.0), (1.4, 0.6)],
        ],
        '°' => &[&[(0.4, 4.0), (1.2, 4.0), (1.2, 3.2), (0.4, 3.2), (0.4, 4.0)]],
        'p' => &[&[(0.0, -1.5), (0.0, 2.5), (2.0, 2.5), (2.0, 0.5), (0.0, 0.5)]],
        't' => &[&[(1.0, 3.5), (1.0, 0.0), (2.0, 0.0)], &[(0.0, 2.5), (2.0, 2.5)]],
        'm' => &[&[(0.0, 0.0), (0.0, 2.5), (1.0, 2.5), (1.0, 0.0)], &[(1.0, 2.5), (2.0, 2.5), (2.0, 0.0)]],
        'c' => &[&[(2.0, 2.5), (0.0, 2.5), (0.0, 0.0), (2.0, 0.0)]],
        'i' => &[&[(1.0, 0.0), (1.0, 2.5)], &[(1.0, 3.2), (1.0, 3.6)]],
        'n' => &[&[(0.0, 0.0), (0.0, 2.5), (2.0, 2.5), (2.0, 0.0)]],
        'x' => &[&[(0.0, 0.0), (2.0, 2.5)], &[(0.0, 2.5), (2.0, 0.0)]],
        'u' => &[&[(0.0, 2.5), (0.0, 0.0), (2.0, 0.0), (2.0, 2.5)]],
        'e' => &[&[(2.0, 0.0), (0.0, 0.0), (0.0, 2.5), (2.0, 2.5), (2.0, 1.25), (0.0, 1.25)]],
        'a' => &[&[(0.0, 2.5), (2.0, 2.5), (2.0, 0.0), (0.0, 0.0), (0.0, 1.25), (2.0, 1.25)]],
        'd' => &[&[(2.0, 4.0), (2.0, 0.0), (0.0, 0.0), (0.0, 2.5), (2.0, 2.5)]],
        'f' => &[&[(2.0, 4.0), (1.0, 4.0), (1.0, 0.0)], &[(0.0, 2.5), (2.0, 2.5)]],
        'l' => &[&[(1.0, 4.0), (1.0, 0.0)]],
        'o' => &[&[(0.0, 0.0), (2.0, 0.0), (2.0, 2.5), (0.0, 2.5), (0.0, 0.0)]],
        'r' => &[&[(0.0, 0.0), (0.0, 2.5), (2.0, 2.5)]],
        's' => &[&[(2.0, 2.5), (0.0, 2.5), (0.0, 1.25), (2.0, 1.25), (2.0, 0.0), (0.0, 0.0)]],
        ' ' => &[],
        _ => return None,
    })
}

/// `text` drawn in the line font along `dir` (unit, the baseline's direction) from `origin`
/// (the baseline's start), `size` the cap height; the glyphs are open strokes.
pub(crate) fn label(text: &str, origin: Point, dir: Vec2, size: f64) -> Vec<SubPath> {
    let dir = unit(dir).unwrap_or(Vec2::new(1.0, 0.0));
    // Up on screen (y down) is to the left of the baseline's direction.
    let up = Vec2::new(dir.y, -dir.x);
    let u = size / 4.0;
    let mut out = Vec::new();
    let mut pen = 0.0;
    for ch in text.chars() {
        let strokes = glyph(ch).unwrap_or(&[]);
        for s in strokes {
            let pts: Vec<Point> = s.iter().map(|(x, y)| origin + dir * ((pen + x) * u) + up * (y * u)).collect();
            if pts.len() >= 2 {
                out.push(SubPath::polyline(&pts, false));
            }
        }
        pen += 3.0;
    }
    out
}

/// The advance of `text` in the line font at cap height `size`.
pub(crate) fn label_width(text: &str, size: f64) -> f64 {
    text.chars().count() as f64 * 3.0 * size / 4.0 - size / 4.0
}
