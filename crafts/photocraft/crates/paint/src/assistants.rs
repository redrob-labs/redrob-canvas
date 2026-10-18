//! Painting assistants: rulers, vanishing points, perspective grids, ellipses, splines and
//! curvilinear grids that stroke input snaps to. Pure geometry on document coordinates; the
//! engine keeps a document's assistants and the canvas feeds pointer positions through
//! [`Assistant::snap`].
//!
//! The straight assistants turn a stroke into a line: the Ruler onto its own segment's line, the
//! Infinite Ruler onto the same line drawn across the canvas, the Parallel Ruler onto the line
//! through the stroke's start in the ruler's direction, the Vanishing Point onto the line through
//! the start and the point, the Perspective Grid onto whichever of its two receding directions the
//! stroke follows more closely, and the 2-Point Perspective onto a line to either of its points or
//! the vertical. The curved assistants turn a stroke into a curve: the Ellipse onto its rim, the
//! Concentric Ellipse onto the rim scaled through the stroke's start, the Perspective Ellipse onto
//! a circle drawn inside a quadrilateral in perspective, the Spline onto a cubic Bézier, the
//! Fish-Eye Point onto the ellipse through its two horizon ends and the stroke's start, and the
//! Curvilinear grid onto circles through its horizon or its vertical ends.

use serde::{Deserialize, Serialize};

pub const MAX_COORDINATE: f64 = 1_000_000.0;
/// Most assistants one document holds.
pub const MAX_ASSISTANTS: usize = 64;
/// Most major subdivisions a ruler shows.
pub const MAX_SUBDIVISIONS: u32 = 64;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Assistant {
    /// Strokes land on the infinite line through `a` and `b`; drawn as the segment `a → b` with
    /// `subdivisions` major notches, each split into `minor` minor ones.
    Ruler {
        a: [f64; 2],
        b: [f64; 2],
        #[serde(default)]
        subdivisions: u32,
        #[serde(default)]
        minor: u32,
    },
    /// The Ruler's line drawn across the whole canvas, with the same notches along `a → b`.
    InfiniteRuler {
        a: [f64; 2],
        b: [f64; 2],
        #[serde(default)]
        subdivisions: u32,
        #[serde(default)]
        minor: u32,
    },
    /// Strokes stay parallel to `a → b`, wherever they start.
    ParallelRuler { a: [f64; 2], b: [f64; 2] },
    /// Strokes head towards (or away from) `p`.
    VanishingPoint { p: [f64; 2] },
    /// A plane in perspective: the four corners of a quadrilateral, clockwise from the top left;
    /// strokes follow its two receding directions.
    PerspectiveGrid { corners: [[f64; 2]; 4] },
    /// An ellipse: `a → b` is the major axis, `c`'s distance from that axis the minor radius.
    /// Strokes follow the rim.
    Ellipse { a: [f64; 2], b: [f64; 2], c: [f64; 2] },
    /// The same ellipse, but a stroke follows the rim of the ellipse with the same centre, angle
    /// and proportions that passes through the stroke's start.
    ConcentricEllipse { a: [f64; 2], b: [f64; 2], c: [f64; 2] },
    /// A circle drawn inside the quadrilateral `corners` (clockwise from the top left) as a circle
    /// in perspective; strokes follow its rim.
    PerspectiveEllipse { corners: [[f64; 2]; 4] },
    /// A cubic Bézier: `p[0]` and `p[3]` are its ends, `p[1]` and `p[2]` its control handles.
    Spline { p: [[f64; 2]; 4] },
    /// Two vanishing points `a` and `b` on a horizon, plus `c`, which sets where the vertical
    /// ruler stands and how dense the drawn rays are (its distance from the horizon). Strokes head
    /// to either point or run perpendicular to the horizon.
    TwoPointPerspective {
        a: [f64; 2],
        b: [f64; 2],
        c: [f64; 2],
        #[serde(default = "default_density")]
        density: f64,
        #[serde(default = "default_true")]
        vertical: bool,
    },
    /// A fish-eye (wide-angle) point: `a → b` is the horizon of the view, `c` sets the bulge.
    /// Strokes follow the ellipse through `a`, `b` and the stroke's start.
    FishEye { a: [f64; 2], b: [f64; 2], c: [f64; 2] },
    /// Five-point curvilinear perspective: the circle on the diameter `a → b`. Strokes follow the
    /// circle through the horizon's ends (`a`, `b`) or through the vertical's ends that passes
    /// through the stroke's start.
    Curvilinear { a: [f64; 2], b: [f64; 2] },
}

fn default_density() -> f64 {
    1.0
}
fn default_true() -> bool {
    true
}

/// How far a point may be snapped (in document pixels) before the assistant is left alone.
pub const MAX_SNAP_DISTANCE: f64 = 1.0e6;

/// The result of snapping `p` with an assistant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Snapped {
    /// Where the point lands.
    pub point: [f64; 2],
    /// How far it moved.
    pub distance: f64,
}

/// Intersection of lines `a → b` and `c → d`, or `None` when they are (nearly) parallel.
pub fn intersect(a: [f64; 2], b: [f64; 2], c: [f64; 2], d: [f64; 2]) -> Option<[f64; 2]> {
    let (r, s) = ([b[0] - a[0], b[1] - a[1]], [d[0] - c[0], d[1] - c[1]]);
    let den = r[0] * s[1] - r[1] * s[0];
    let len = (r[0].hypot(r[1]) * s[0].hypot(s[1])).max(1e-12);
    if (den / len).abs() < 1e-6 {
        return None;
    }
    let t = ((c[0] - a[0]) * s[1] - (c[1] - a[1]) * s[0]) / den;
    let p = [a[0] + t * r[0], a[1] + t * r[1]];
    p.iter().all(|v| v.is_finite() && v.abs() <= MAX_COORDINATE * 1e3).then_some(p)
}

/// `p` projected onto the infinite line through `origin` in direction `dir` (unit length not
/// required). A zero direction returns `p` unchanged.
fn project(origin: [f64; 2], dir: [f64; 2], p: [f64; 2]) -> [f64; 2] {
    let len2 = dir[0] * dir[0] + dir[1] * dir[1];
    if len2 <= 1e-18 {
        return p;
    }
    let t = ((p[0] - origin[0]) * dir[0] + (p[1] - origin[1]) * dir[1]) / len2;
    [origin[0] + t * dir[0], origin[1] + t * dir[1]]
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn finite(p: [f64; 2]) -> bool {
    p.iter().all(|v| v.is_finite() && v.abs() <= MAX_COORDINATE)
}

fn lerp(a: [f64; 2], b: [f64; 2], t: f64) -> [f64; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

fn mid(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    lerp(a, b, 0.5)
}

/// Unit direction of `a → b`, or `None` for coincident points.
fn unit(a: [f64; 2], b: [f64; 2]) -> Option<[f64; 2]> {
    let d = [b[0] - a[0], b[1] - a[1]];
    let l = d[0].hypot(d[1]);
    (l >= 1e-9).then(|| [d[0] / l, d[1] / l])
}

/// Signed distance of `p` from the line through `a` and `b` (positive to the left of `a → b`
/// in y-down coordinates).
fn line_offset(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> f64 {
    let u = unit(a, b).unwrap_or([1.0, 0.0]);
    (p[0] - a[0]) * u[1] - (p[1] - a[1]) * u[0]
}

// ---------------------------------------------------------------------------------------------
// Ellipses
// ---------------------------------------------------------------------------------------------

/// An ellipse as centre, two semi-axes and the angle of the major axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EllipseGeom {
    pub center: [f64; 2],
    /// Semi-axis along the major direction.
    pub rx: f64,
    /// Semi-axis across it.
    pub ry: f64,
    /// Angle of the major axis in radians.
    pub angle: f64,
}

impl EllipseGeom {
    /// The ellipse the three handles `a`, `b` (major axis ends) and `c` (minor radius) define.
    pub fn from_handles(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> Option<Self> {
        let rx = dist(a, b) / 2.0;
        let ry = line_offset(a, b, c).abs();
        if rx < 1e-6 || ry < 1e-6 {
            return None;
        }
        Some(Self { center: mid(a, b), rx, ry, angle: (b[1] - a[1]).atan2(b[0] - a[0]) })
    }

    /// `p` in the ellipse's own frame (major axis along x).
    pub fn to_local(&self, p: [f64; 2]) -> [f64; 2] {
        let (s, c) = self.angle.sin_cos();
        let d = [p[0] - self.center[0], p[1] - self.center[1]];
        [d[0] * c + d[1] * s, -d[0] * s + d[1] * c]
    }

    pub fn to_world(&self, l: [f64; 2]) -> [f64; 2] {
        let (s, c) = self.angle.sin_cos();
        [self.center[0] + l[0] * c - l[1] * s, self.center[1] + l[0] * s + l[1] * c]
    }

    /// The point of the rim at parameter `t`.
    pub fn at(&self, t: f64) -> [f64; 2] {
        self.to_world([self.rx * t.cos(), self.ry * t.sin()])
    }

    /// The ellipse with the same centre, angle and proportions whose rim passes through `p`
    /// (`None` for `p` at the centre).
    pub fn through(&self, p: [f64; 2]) -> Option<Self> {
        let l = self.to_local(p);
        let k = ((l[0] / self.rx).powi(2) + (l[1] / self.ry).powi(2)).sqrt();
        (k > 1e-6).then(|| Self { rx: self.rx * k, ry: self.ry * k, ..*self })
    }

    /// The point of the rim nearest to `p` (Newton's method on the rim parameter).
    pub fn nearest(&self, p: [f64; 2]) -> [f64; 2] {
        let l = self.to_local(p);
        let (a, b) = (self.rx, self.ry);
        let mut t = (l[1] / b).atan2(l[0] / a);
        for _ in 0..12 {
            let (s, c) = t.sin_cos();
            // d/dt of the squared distance, and its derivative.
            let f = (a * c - l[0]) * (-a * s) + (b * s - l[1]) * (b * c);
            let df = (a * s) * (a * s) - (a * c - l[0]) * (a * c) + (b * c) * (b * c) - (b * s - l[1]) * (b * s);
            if df.abs() < 1e-12 {
                break;
            }
            let step = f / df;
            t -= step;
            if step.abs() < 1e-10 {
                break;
            }
        }
        self.at(t)
    }

    /// The rim as `n` segments.
    pub fn polyline(&self, n: usize) -> Vec<[[f64; 2]; 2]> {
        let n = n.max(8);
        (0..n)
            .map(|i| {
                let t0 = i as f64 / n as f64 * std::f64::consts::TAU;
                let t1 = (i + 1) as f64 / n as f64 * std::f64::consts::TAU;
                [self.at(t0), self.at(t1)]
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------------------------
// Homographies (perspective grid and perspective ellipse)
// ---------------------------------------------------------------------------------------------

/// The projective map from the unit square to a quadrilateral, `(u, v) → (x, y)`; `corners` run
/// clockwise from the top left: `(0,0) → tl`, `(1,0) → tr`, `(1,1) → br`, `(0,1) → bl`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Homography {
    /// `[a, b, c, d, e, f, g, h]`: `x = (a u + b v + c) / (g u + h v + 1)`,
    /// `y = (d u + e v + f) / (g u + h v + 1)`.
    pub m: [f64; 8],
}

impl Homography {
    pub fn from_quad(corners: [[f64; 2]; 4]) -> Option<Self> {
        let [[x0, y0], [x1, y1], [x2, y2], [x3, y3]] = corners;
        let (dx1, dx2, dx3) = (x1 - x2, x3 - x2, x0 - x1 + x2 - x3);
        let (dy1, dy2, dy3) = (y1 - y2, y3 - y2, y0 - y1 + y2 - y3);
        let m = if dx3.abs() < 1e-9 && dy3.abs() < 1e-9 {
            [x1 - x0, x2 - x1, x0, y1 - y0, y2 - y1, y0, 0.0, 0.0]
        } else {
            let den = dx1 * dy2 - dx2 * dy1;
            if den.abs() < 1e-12 {
                return None;
            }
            let g = (dx3 * dy2 - dx2 * dy3) / den;
            let h = (dx1 * dy3 - dx3 * dy1) / den;
            [x1 - x0 + g * x1, x3 - x0 + h * x3, x0, y1 - y0 + g * y1, y3 - y0 + h * y3, y0, g, h]
        };
        m.iter().all(|v| v.is_finite()).then_some(Self { m })
    }

    pub fn map(&self, u: f64, v: f64) -> [f64; 2] {
        let [a, b, c, d, e, f, g, h] = self.m;
        let w = g * u + h * v + 1.0;
        if w.abs() < 1e-12 {
            return [f64::NAN, f64::NAN];
        }
        [(a * u + b * v + c) / w, (d * u + e * v + f) / w]
    }

    /// `(x, y) → (u, v)`, the inverse map; `None` where the map is singular.
    pub fn unmap(&self, p: [f64; 2]) -> Option<[f64; 2]> {
        let [a, b, c, d, e, f, g, h] = self.m;
        // Solve the 2×2 system (a - g x) u + (b - h x) v = x - c, (d - g y) u + (e - h y) v = y - f.
        let (a11, a12, b1) = (a - g * p[0], b - h * p[0], p[0] - c);
        let (a21, a22, b2) = (d - g * p[1], e - h * p[1], p[1] - f);
        let det = a11 * a22 - a12 * a21;
        if det.abs() < 1e-12 {
            return None;
        }
        let u = (b1 * a22 - a12 * b2) / det;
        let v = (a11 * b2 - b1 * a21) / det;
        (u.is_finite() && v.is_finite()).then_some([u, v])
    }

    /// The local area scale of the map at `(u, v)`: the Jacobian determinant.
    pub fn jacobian(&self, u: f64, v: f64) -> f64 {
        let [a, b, _, d, e, _, g, h] = self.m;
        let w = g * u + h * v + 1.0;
        if w.abs() < 1e-12 {
            return 0.0;
        }
        let [x, y] = self.map(u, v);
        ((a - g * x) * (e - h * y) - (b - h * x) * (d - g * y)) / (w * w)
    }
}

// ---------------------------------------------------------------------------------------------
// Curves: Béziers and circles
// ---------------------------------------------------------------------------------------------

fn bezier(p: &[[f64; 2]; 4], t: f64) -> [f64; 2] {
    let s = 1.0 - t;
    let (b0, b1, b2, b3) = (s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t);
    [p[0][0] * b0 + p[1][0] * b1 + p[2][0] * b2 + p[3][0] * b3, p[0][1] * b0 + p[1][1] * b1 + p[2][1] * b2 + p[3][1] * b3]
}

/// The point of the Bézier nearest to `q`: a coarse scan, then a ternary refinement.
fn bezier_nearest(p: &[[f64; 2]; 4], q: [f64; 2]) -> [f64; 2] {
    const N: usize = 64;
    let mut best = (0usize, f64::INFINITY);
    for i in 0..=N {
        let d = dist(bezier(p, i as f64 / N as f64), q);
        if d < best.1 {
            best = (i, d);
        }
    }
    let mut lo = (best.0.saturating_sub(1)) as f64 / N as f64;
    let mut hi = ((best.0 + 1).min(N)) as f64 / N as f64;
    for _ in 0..40 {
        let m1 = lo + (hi - lo) / 3.0;
        let m2 = hi - (hi - lo) / 3.0;
        if dist(bezier(p, m1), q) < dist(bezier(p, m2), q) {
            hi = m2;
        } else {
            lo = m1;
        }
    }
    bezier(p, (lo + hi) / 2.0)
}

/// The circle through three points as (centre, radius); `None` when they are collinear.
fn circle_through(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> Option<([f64; 2], f64)> {
    let d = 2.0 * (a[0] * (b[1] - c[1]) + b[0] * (c[1] - a[1]) + c[0] * (a[1] - b[1]));
    let scale = dist(a, b).max(dist(b, c)).max(dist(a, c)).max(1e-9);
    if (d / (scale * scale)).abs() < 1e-7 {
        return None;
    }
    let (a2, b2, c2) = (a[0] * a[0] + a[1] * a[1], b[0] * b[0] + b[1] * b[1], c[0] * c[0] + c[1] * c[1]);
    let ux = (a2 * (b[1] - c[1]) + b2 * (c[1] - a[1]) + c2 * (a[1] - b[1])) / d;
    let uy = (a2 * (c[0] - b[0]) + b2 * (a[0] - c[0]) + c2 * (b[0] - a[0])) / d;
    let center = [ux, uy];
    let r = dist(center, a);
    (center.iter().all(|v| v.is_finite()) && r.is_finite() && r <= MAX_COORDINATE * 1e3).then_some((center, r))
}

/// `q` projected onto the circle (or, with `None`, onto the line through `a` and `b`).
fn circle_nearest(circle: Option<([f64; 2], f64)>, a: [f64; 2], b: [f64; 2], q: [f64; 2]) -> [f64; 2] {
    match circle {
        Some((c, r)) => match unit(c, q) {
            Some(u) => [c[0] + u[0] * r, c[1] + u[1] * r],
            None => q,
        },
        None => project(a, [b[0] - a[0], b[1] - a[1]], q),
    }
}

fn circle_polyline(center: [f64; 2], r: f64, n: usize) -> Vec<[[f64; 2]; 2]> {
    let n = n.max(8);
    let at = |i: usize| {
        let t = i as f64 / n as f64 * std::f64::consts::TAU;
        [center[0] + r * t.cos(), center[1] + r * t.sin()]
    };
    (0..n).map(|i| [at(i), at(i + 1)]).collect()
}

// ---------------------------------------------------------------------------------------------
// The assistant
// ---------------------------------------------------------------------------------------------

impl Assistant {
    pub const KINDS: [&'static str; 12] = [
        "ruler",
        "infiniteRuler",
        "parallelRuler",
        "vanishingPoint",
        "perspectiveGrid",
        "ellipse",
        "concentricEllipse",
        "perspectiveEllipse",
        "spline",
        "twoPointPerspective",
        "fishEye",
        "curvilinear",
    ];

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Ruler { .. } => "ruler",
            Self::InfiniteRuler { .. } => "infiniteRuler",
            Self::ParallelRuler { .. } => "parallelRuler",
            Self::VanishingPoint { .. } => "vanishingPoint",
            Self::PerspectiveGrid { .. } => "perspectiveGrid",
            Self::Ellipse { .. } => "ellipse",
            Self::ConcentricEllipse { .. } => "concentricEllipse",
            Self::PerspectiveEllipse { .. } => "perspectiveEllipse",
            Self::Spline { .. } => "spline",
            Self::TwoPointPerspective { .. } => "twoPointPerspective",
            Self::FishEye { .. } => "fishEye",
            Self::Curvilinear { .. } => "curvilinear",
        }
    }

    /// How many handles a `kind` has (what [`Self::from_points`] needs).
    pub fn handle_count(kind: &str) -> Option<usize> {
        Some(match kind {
            "ruler" | "infiniteRuler" | "parallelRuler" | "curvilinear" => 2,
            "vanishingPoint" => 1,
            "perspectiveGrid" | "perspectiveEllipse" | "spline" => 4,
            "ellipse" | "concentricEllipse" | "twoPointPerspective" | "fishEye" => 3,
            _ => return None,
        })
    }

    /// Does a `kind` snap strokes onto curves rather than straight lines?
    pub fn is_curved(&self) -> bool {
        matches!(
            self,
            Self::Ellipse { .. }
                | Self::ConcentricEllipse { .. }
                | Self::PerspectiveEllipse { .. }
                | Self::Spline { .. }
                | Self::FishEye { .. }
                | Self::Curvilinear { .. }
        )
    }

    /// Build an assistant of `kind` from its handles (see [`Self::handle_count`]). Coincident
    /// ruler ends or a degenerate grid are refused.
    pub fn from_points(kind: &str, points: &[[f64; 2]]) -> Option<Self> {
        if points.len() != Self::handle_count(kind)? || !points.iter().all(|p| finite(*p)) {
            return None;
        }
        let quad = || [points[0], points[1], points[2], points[3]];
        let a = match kind {
            "ruler" => Self::Ruler { a: points[0], b: points[1], subdivisions: 0, minor: 0 },
            "infiniteRuler" => Self::InfiniteRuler { a: points[0], b: points[1], subdivisions: 0, minor: 0 },
            "parallelRuler" => Self::ParallelRuler { a: points[0], b: points[1] },
            "vanishingPoint" => Self::VanishingPoint { p: points[0] },
            "perspectiveGrid" => Self::PerspectiveGrid { corners: quad() },
            "ellipse" => Self::Ellipse { a: points[0], b: points[1], c: points[2] },
            "concentricEllipse" => Self::ConcentricEllipse { a: points[0], b: points[1], c: points[2] },
            "perspectiveEllipse" => Self::PerspectiveEllipse { corners: quad() },
            "spline" => Self::Spline { p: quad() },
            "twoPointPerspective" => Self::TwoPointPerspective { a: points[0], b: points[1], c: points[2], density: 1.0, vertical: true },
            "fishEye" => Self::FishEye { a: points[0], b: points[1], c: points[2] },
            "curvilinear" => Self::Curvilinear { a: points[0], b: points[1] },
            _ => return None,
        };
        a.is_valid().then_some(a)
    }

    /// Rulers need two distinct ends; a grid needs four distinct corners; an ellipse a minor
    /// radius.
    pub fn is_valid(&self) -> bool {
        let distinct = |pts: &[[f64; 2]]| pts.iter().all(|c| finite(*c)) && (0..pts.len()).all(|i| (i + 1..pts.len()).all(|j| dist(pts[i], pts[j]) >= 1e-6));
        match self {
            Self::Ruler { a, b, .. } | Self::InfiniteRuler { a, b, .. } | Self::ParallelRuler { a, b } | Self::Curvilinear { a, b } => {
                finite(*a) && finite(*b) && dist(*a, *b) >= 1e-6
            }
            Self::VanishingPoint { p } => finite(*p),
            Self::PerspectiveGrid { corners } => distinct(corners) && Homography::from_quad(*corners).is_some(),
            Self::PerspectiveEllipse { corners } => distinct(corners) && Homography::from_quad(*corners).is_some(),
            Self::Ellipse { a, b, c } | Self::ConcentricEllipse { a, b, c } | Self::FishEye { a, b, c } => {
                finite(*c) && EllipseGeom::from_handles(*a, *b, *c).is_some()
            }
            Self::Spline { p } => p.iter().all(|q| finite(*q)) && dist(p[0], p[3]) >= 1e-6,
            Self::TwoPointPerspective { a, b, c, .. } => finite(*a) && finite(*b) && finite(*c) && dist(*a, *b) >= 1e-6,
        }
    }

    /// The draggable points, in [`Self::from_points`] order.
    pub fn handles(&self) -> Vec<[f64; 2]> {
        match self {
            Self::Ruler { a, b, .. } | Self::InfiniteRuler { a, b, .. } | Self::ParallelRuler { a, b } | Self::Curvilinear { a, b } => vec![*a, *b],
            Self::VanishingPoint { p } => vec![*p],
            Self::PerspectiveGrid { corners } | Self::PerspectiveEllipse { corners } => corners.to_vec(),
            Self::Ellipse { a, b, c } | Self::ConcentricEllipse { a, b, c } | Self::FishEye { a, b, c } | Self::TwoPointPerspective { a, b, c, .. } => {
                vec![*a, *b, *c]
            }
            Self::Spline { p } => p.to_vec(),
        }
    }

    /// Move handle `i` to `to`; false when there is no such handle or the result is degenerate.
    /// An ellipse's third handle snaps to a circle when it is within 8 % of one.
    pub fn set_handle(&mut self, i: usize, to: [f64; 2]) -> bool {
        if !finite(to) {
            return false;
        }
        let mut handles = self.handles();
        if i >= handles.len() {
            return false;
        }
        handles[i] = to;
        if let (Self::Ellipse { .. } | Self::ConcentricEllipse { .. }, 2) = (&*self, i) {
            let (a, b) = (handles[0], handles[1]);
            let rx = dist(a, b) / 2.0;
            let off = line_offset(a, b, to);
            if rx > 1e-6 && (off.abs() - rx).abs() <= rx * 0.08 {
                // Snap the minor radius to the major one: a circle.
                let u = unit(a, b).unwrap_or([1.0, 0.0]);
                let n = [u[1], -u[0]];
                let foot = project(a, [b[0] - a[0], b[1] - a[1]], to);
                let s = off.signum();
                handles[2] = [foot[0] + n[0] * rx * s, foot[1] + n[1] * rx * s];
            }
        }
        match Self::from_points(self.kind(), &handles) {
            Some(mut next) => {
                next.copy_settings(self);
                *self = next;
                true
            }
            None => false,
        }
    }

    /// Carry the non-geometric settings (subdivisions, density) of `from` over.
    fn copy_settings(&mut self, from: &Self) {
        match (self, from) {
            (Self::Ruler { subdivisions, minor, .. }, Self::Ruler { subdivisions: s, minor: m, .. })
            | (Self::InfiniteRuler { subdivisions, minor, .. }, Self::InfiniteRuler { subdivisions: s, minor: m, .. }) => {
                *subdivisions = *s;
                *minor = *m;
            }
            (Self::TwoPointPerspective { density, vertical, .. }, Self::TwoPointPerspective { density: d, vertical: v, .. }) => {
                *density = *d;
                *vertical = *v;
            }
            _ => {}
        }
    }

    /// Move the whole assistant by `d`.
    pub fn translate(&mut self, d: [f64; 2]) -> bool {
        let moved: Vec<[f64; 2]> = self.handles().into_iter().map(|p| [p[0] + d[0], p[1] + d[1]]).collect();
        match Self::from_points(self.kind(), &moved) {
            Some(mut a) => {
                a.copy_settings(self);
                *self = a;
                true
            }
            None => false,
        }
    }

    /// The centre of the handles: where a move handle sits.
    pub fn center(&self) -> [f64; 2] {
        let h = self.handles();
        let n = h.len() as f64;
        [h.iter().map(|p| p[0]).sum::<f64>() / n, h.iter().map(|p| p[1]).sum::<f64>() / n]
    }

    /// The ruler notches: `(subdivisions, minor)`; `None` for an assistant without them.
    pub fn subdivisions(&self) -> Option<(u32, u32)> {
        match self {
            Self::Ruler { subdivisions, minor, .. } | Self::InfiniteRuler { subdivisions, minor, .. } => Some((*subdivisions, *minor)),
            _ => None,
        }
    }

    /// Set a ruler's notches: `subdivisions` major ones (0 = none), each split into `minor`.
    pub fn set_subdivisions(&mut self, major: u32, minor_count: u32) -> bool {
        match self {
            Self::Ruler { subdivisions, minor, .. } | Self::InfiniteRuler { subdivisions, minor, .. } => {
                *subdivisions = major.min(MAX_SUBDIVISIONS);
                *minor = minor_count.min(MAX_SUBDIVISIONS);
                true
            }
            _ => false,
        }
    }

    /// A 2-point perspective's ray density (0.25..=4) and whether its vertical ruler shows.
    pub fn set_two_point(&mut self, density_value: f64, vertical_ruler: bool) -> bool {
        match self {
            Self::TwoPointPerspective { density, vertical, .. } => {
                *density = if density_value.is_finite() { density_value.clamp(0.25, 4.0) } else { 1.0 };
                *vertical = vertical_ruler;
                true
            }
            _ => false,
        }
    }

    /// The horizon this assistant defines, as two points on it: a 2-point perspective's line
    /// through its points; `None` for the others.
    pub fn horizon(&self) -> Option<[[f64; 2]; 2]> {
        match self {
            Self::TwoPointPerspective { a, b, .. } => Some([*a, *b]),
            _ => None,
        }
    }

    /// A perspective grid's vanishing points: `Ok(point)` where opposite sides meet, `Err(dir)`
    /// (a direction) where they are parallel. First the horizontal pair (top and bottom sides),
    /// then the vertical pair (left and right sides).
    pub fn vanishing(&self) -> Option<[Result<[f64; 2], [f64; 2]>; 2]> {
        let Self::PerspectiveGrid { corners: [tl, tr, br, bl] } = self else { return None };
        let horizontal = intersect(*tl, *tr, *bl, *br).ok_or([tr[0] - tl[0], tr[1] - tl[1]]);
        let vertical = intersect(*tl, *bl, *tr, *br).ok_or([bl[0] - tl[0], bl[1] - tl[1]]);
        Some([horizontal, vertical])
    }

    /// The perspective sensor: how large a brush at `p` is relative to the grid's near edge
    /// (its bottom side's middle), 0..1 — things shrink as they recede. A perspective grid only;
    /// `None` for the others.
    pub fn perspective_scale(&self, p: [f64; 2]) -> Option<f64> {
        let Self::PerspectiveGrid { corners } = self else { return None };
        let h = Homography::from_quad(*corners)?;
        let [u, v] = h.unmap(p)?;
        let near = h.jacobian(0.5, 1.0).abs();
        let here = h.jacobian(u, v).abs();
        if near <= 1e-12 || !here.is_finite() {
            return None;
        }
        Some((here / near).sqrt().clamp(0.0, 1.0))
    }

    /// The ellipse an Ellipse, Concentric Ellipse or Fish-Eye assistant draws.
    pub fn ellipse(&self) -> Option<EllipseGeom> {
        match self {
            Self::Ellipse { a, b, c } | Self::ConcentricEllipse { a, b, c } | Self::FishEye { a, b, c } => EllipseGeom::from_handles(*a, *b, *c),
            _ => None,
        }
    }

    /// The perspective ellipse's rim as a polyline (`n` segments).
    fn perspective_rim(corners: &[[f64; 2]; 4], n: usize) -> Vec<[f64; 2]> {
        let Some(h) = Homography::from_quad(*corners) else { return Vec::new() };
        (0..n)
            .map(|i| {
                let t = i as f64 / n as f64 * std::f64::consts::TAU;
                h.map(0.5 + 0.5 * t.cos(), 0.5 + 0.5 * t.sin())
            })
            .filter(|p| p.iter().all(|v| v.is_finite()))
            .collect()
    }

    /// The nearest point of a closed polyline to `q`.
    fn polyline_nearest(poly: &[[f64; 2]], q: [f64; 2]) -> Option<[f64; 2]> {
        if poly.len() < 2 {
            return None;
        }
        let mut best: Option<([f64; 2], f64)> = None;
        for i in 0..poly.len() {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            let d = [b[0] - a[0], b[1] - a[1]];
            let len2 = d[0] * d[0] + d[1] * d[1];
            let t = if len2 <= 1e-18 { 0.0 } else { (((q[0] - a[0]) * d[0] + (q[1] - a[1]) * d[1]) / len2).clamp(0.0, 1.0) };
            let c = [a[0] + d[0] * t, a[1] + d[1] * t];
            let dd = dist(c, q);
            if best.is_none_or(|(_, bd)| dd < bd) {
                best = Some((c, dd));
            }
        }
        best.map(|(c, _)| c)
    }

    /// The lines a stroke from `start` may follow: `(origin, direction)` pairs. Empty for the
    /// curved assistants (see [`Self::snap`]).
    fn lines(&self, start: [f64; 2]) -> Vec<([f64; 2], [f64; 2])> {
        let towards = |p: [f64; 2]| {
            let d = [p[0] - start[0], p[1] - start[1]];
            (d[0].hypot(d[1]) >= 1e-9).then_some((start, d))
        };
        match self {
            Self::Ruler { a, b, .. } | Self::InfiniteRuler { a, b, .. } => vec![(*a, [b[0] - a[0], b[1] - a[1]])],
            Self::ParallelRuler { a, b } => vec![(start, [b[0] - a[0], b[1] - a[1]])],
            Self::VanishingPoint { p } => towards(*p).into_iter().collect(),
            Self::PerspectiveGrid { .. } => self
                .vanishing()
                .into_iter()
                .flatten()
                .filter_map(|v| match v {
                    Ok(vp) => towards(vp),
                    Err(dir) => Some((start, dir)),
                })
                .collect(),
            Self::TwoPointPerspective { a, b, .. } => {
                let mut out: Vec<_> = [towards(*a), towards(*b)].into_iter().flatten().collect();
                if let Some(u) = unit(*a, *b) {
                    out.push((start, [-u[1], u[0]]));
                }
                out
            }
            _ => Vec::new(),
        }
    }

    /// Snap `p`, a point of a stroke that began at `start`. `None` when the assistant has no
    /// line for this stroke (a vanishing point the stroke started on).
    pub fn snap(&self, start: [f64; 2], p: [f64; 2]) -> Option<Snapped> {
        let candidates: Vec<[f64; 2]> = match self {
            Self::Ellipse { .. } => vec![self.ellipse()?.nearest(p)],
            Self::ConcentricEllipse { .. } => vec![self.ellipse()?.through(start)?.nearest(p)],
            Self::PerspectiveEllipse { corners } => Self::polyline_nearest(&Self::perspective_rim(corners, 180), p).into_iter().collect(),
            Self::Spline { p: ctrl } => vec![bezier_nearest(ctrl, p)],
            Self::FishEye { a, b, c } => {
                let e = EllipseGeom::from_handles(*a, *b, *c)?;
                let l = e.to_local(start);
                let inner = 1.0 - (l[0] / e.rx).powi(2);
                if l[1].abs() < 1e-6 || inner <= 1e-6 {
                    // A stroke begun on the horizon (or past the view's ends) follows it.
                    vec![project(*a, [b[0] - a[0], b[1] - a[1]], p)]
                } else {
                    let ry = l[1].abs() / inner.sqrt();
                    vec![EllipseGeom { ry, ..e }.nearest(p)]
                }
            }
            Self::Curvilinear { a, b } => {
                let m = mid(*a, *b);
                let r = dist(*a, *b) / 2.0;
                let u = unit(*a, *b)?;
                let (top, bottom) = ([m[0] - u[1] * r, m[1] + u[0] * r], [m[0] + u[1] * r, m[1] - u[0] * r]);
                vec![circle_nearest(circle_through(*a, *b, start), *a, *b, p), circle_nearest(circle_through(top, bottom, start), top, bottom, p)]
            }
            _ => self.lines(start).into_iter().map(|(o, d)| project(o, d, p)).collect(),
        };
        candidates
            .into_iter()
            .filter(|q| q.iter().all(|v| v.is_finite()))
            .map(|q| Snapped { point: q, distance: dist(q, p) })
            .filter(|s| s.distance <= MAX_SNAP_DISTANCE)
            .min_by(|x, y| x.distance.total_cmp(&y.distance))
    }

    /// How well this assistant fits a stroke from `start` heading to `p`: the snap distance,
    /// plus for a Ruler, an ellipse or a spline how far the start sits from the curve (a stroke
    /// begun far from a ruler is not the ruler's). Lower is better; `None` when it cannot snap
    /// this stroke.
    pub fn fit(&self, start: [f64; 2], p: [f64; 2]) -> Option<f64> {
        let s = self.snap(start, p)?;
        let start_penalty = match self {
            Self::Ruler { a, b, .. } | Self::InfiniteRuler { a, b, .. } => dist(project(*a, [b[0] - a[0], b[1] - a[1]], start), start),
            Self::Ellipse { .. } | Self::PerspectiveEllipse { .. } | Self::Spline { .. } => self.snap(start, start).map_or(0.0, |q| q.distance),
            _ => 0.0,
        };
        Some(s.distance + start_penalty)
    }

    /// The segments drawn for the assistant on a `size` document (besides its handles): rulers
    /// across the canvas, a vanishing point's rays towards the corners, a grid's edges and inner
    /// lines, the curved assistants' curves.
    pub fn guides(&self, size: [f64; 2]) -> Vec<[[f64; 2]; 2]> {
        let reach = size[0].hypot(size[1]).max(1.0);
        let along = |o: [f64; 2], d: [f64; 2]| {
            let l = d[0].hypot(d[1]).max(1e-9);
            let u = [d[0] / l, d[1] / l];
            [[o[0] - reach * u[0], o[1] - reach * u[1]], [o[0] + reach * u[0], o[1] + reach * u[1]]]
        };
        let notches = |a: [f64; 2], b: [f64; 2], major: u32, minor: u32, out: &mut Vec<[[f64; 2]; 2]>| {
            if major == 0 {
                return;
            }
            let Some(u) = unit(a, b) else { return };
            let n = [-u[1], u[0]];
            let len = dist(a, b);
            let major_len = (len / 40.0).clamp(4.0, 24.0);
            let minor_len = major_len / 2.0;
            let steps = major * (minor + 1);
            for i in 0..=steps {
                let t = f64::from(i) / f64::from(steps);
                let p = lerp(a, b, t);
                let l = if i % (minor + 1) == 0 { major_len } else { minor_len };
                out.push([[p[0] - n[0] * l, p[1] - n[1] * l], [p[0] + n[0] * l, p[1] + n[1] * l]]);
            }
        };
        match self {
            Self::Ruler { a, b, subdivisions, minor } => {
                let mut out = vec![[*a, *b]];
                notches(*a, *b, *subdivisions, *minor, &mut out);
                out
            }
            Self::InfiniteRuler { a, b, subdivisions, minor } => {
                let mut out = vec![along(*a, [b[0] - a[0], b[1] - a[1]])];
                notches(*a, *b, *subdivisions, *minor, &mut out);
                out
            }
            Self::ParallelRuler { a, b } => {
                // The ruler itself, and a faint family of parallels across the canvas.
                let d = [b[0] - a[0], b[1] - a[1]];
                let l = d[0].hypot(d[1]).max(1e-9);
                let n = [-d[1] / l, d[0] / l];
                let step = (size[0].min(size[1]) / 6.0).max(8.0);
                (-6..=6).map(|k| along([a[0] + n[0] * step * f64::from(k), a[1] + n[1] * step * f64::from(k)], d)).collect()
            }
            Self::VanishingPoint { p } => [
                [0.0, 0.0],
                [size[0], 0.0],
                [size[0], size[1]],
                [0.0, size[1]],
                [size[0] / 2.0, 0.0],
                [size[0] / 2.0, size[1]],
                [0.0, size[1] / 2.0],
                [size[0], size[1] / 2.0],
            ]
            .into_iter()
            .filter(|c| dist(*c, *p) >= 1e-6)
            .map(|c| [*p, c])
            .collect(),
            Self::PerspectiveGrid { corners } => {
                let [tl, tr, br, bl] = *corners;
                let mut out = vec![[tl, tr], [tr, br], [br, bl], [bl, tl]];
                // Inner lines: the sides divided in perspective where the vanishing points exist,
                // evenly where they do not.
                const DIVISIONS: usize = 4;
                for i in 1..DIVISIONS {
                    let t = i as f64 / DIVISIONS as f64;
                    out.push([lerp(tl, tr, t), lerp(bl, br, t)]);
                    out.push([lerp(tl, bl, t), lerp(tr, br, t)]);
                }
                out
            }
            Self::Ellipse { a, b, c } | Self::ConcentricEllipse { a, b, c } => {
                let Some(e) = EllipseGeom::from_handles(*a, *b, *c) else { return Vec::new() };
                let mut out = e.polyline(96);
                out.push([*a, *b]);
                if matches!(self, Self::ConcentricEllipse { .. }) {
                    for k in [0.5, 1.5, 2.0] {
                        out.extend(EllipseGeom { rx: e.rx * k, ry: e.ry * k, ..e }.polyline(96));
                    }
                }
                out
            }
            Self::PerspectiveEllipse { corners } => {
                let [tl, tr, br, bl] = *corners;
                let mut out = vec![[tl, tr], [tr, br], [br, bl], [bl, tl]];
                let rim = Self::perspective_rim(corners, 96);
                for i in 0..rim.len() {
                    out.push([rim[i], rim[(i + 1) % rim.len()]]);
                }
                out
            }
            Self::Spline { p } => {
                const N: usize = 64;
                let mut out: Vec<[[f64; 2]; 2]> = (0..N).map(|i| [bezier(p, i as f64 / N as f64), bezier(p, (i + 1) as f64 / N as f64)]).collect();
                out.push([p[0], p[1]]);
                out.push([p[3], p[2]]);
                out
            }
            Self::TwoPointPerspective { a, b, c, density, vertical } => {
                let mut out = vec![along(*a, [b[0] - a[0], b[1] - a[1]])];
                let Some(u) = unit(*a, *b) else { return out };
                let n = [-u[1], u[0]];
                // Rays from each point through points spaced along the vertical through `c`.
                let spacing = (dist(*a, *b) / (8.0 * density.max(0.25))).max(4.0);
                let count = ((reach / spacing).ceil() as i64).clamp(1, 64);
                let foot = project(*a, [b[0] - a[0], b[1] - a[1]], *c);
                for k in -count..=count {
                    if k == 0 {
                        continue;
                    }
                    let q = [foot[0] + n[0] * spacing * k as f64, foot[1] + n[1] * spacing * k as f64];
                    out.push(along(*a, [q[0] - a[0], q[1] - a[1]]));
                    out.push(along(*b, [q[0] - b[0], q[1] - b[1]]));
                }
                if *vertical {
                    out.push(along(*c, n));
                }
                out
            }
            Self::FishEye { a, b, c } => {
                let Some(e) = EllipseGeom::from_handles(*a, *b, *c) else { return Vec::new() };
                let mut out = vec![[*a, *b]];
                for k in [0.33, 0.66, 1.0, 1.5] {
                    out.extend(EllipseGeom { ry: e.ry * k, ..e }.polyline(96));
                }
                out
            }
            Self::Curvilinear { a, b } => {
                let m = mid(*a, *b);
                let r = dist(*a, *b) / 2.0;
                let Some(u) = unit(*a, *b) else { return Vec::new() };
                let n = [-u[1], u[0]];
                let mut out = circle_polyline(m, r, 128);
                out.push([*a, *b]);
                out.push([[m[0] - n[0] * r, m[1] - n[1] * r], [m[0] + n[0] * r, m[1] + n[1] * r]]);
                // Circles through the horizon ends and through the vertical ends, bulging either way.
                for k in [-2.0, -1.0, -0.5, 0.5, 1.0, 2.0] {
                    let d = r * k;
                    let rr = (r * r + d * d).sqrt();
                    out.extend(circle_polyline([m[0] + n[0] * d, m[1] + n[1] * d], rr, 128));
                    out.extend(circle_polyline([m[0] + u[0] * d, m[1] + u[1] * d], rr, 128));
                }
                out
            }
        }
    }
}

/// Pick the assistant among `assistants` that best fits a stroke from `start` to `p`.
pub fn choose(assistants: &[Assistant], start: [f64; 2], p: [f64; 2]) -> Option<usize> {
    assistants.iter().enumerate().filter_map(|(i, a)| a.fit(start, p).map(|f| (i, f))).min_by(|x, y| x.1.total_cmp(&y.1)).map(|(i, _)| i)
}

/// [`choose`] over the assistants `allowed` admits (the ones whose area holds the stroke).
pub fn choose_where(assistants: &[Assistant], start: [f64; 2], p: [f64; 2], allowed: impl Fn(usize) -> bool) -> Option<usize> {
    assistants
        .iter()
        .enumerate()
        .filter(|(i, _)| allowed(*i))
        .filter_map(|(i, a)| a.fit(start, p).map(|f| (i, f)))
        .min_by(|x, y| x.1.total_cmp(&y.1))
        .map(|(i, _)| i)
}

/// A horizon for a vanishing point dragged with Alt: the y (or the line) the point stays on, from
/// the other assistants — a 2-point perspective's horizon, else the nearest other vanishing
/// point's height. `None` when nothing defines one.
pub fn horizon_for(assistants: &[Assistant], skip: usize, p: [f64; 2]) -> Option<[f64; 2]> {
    let mut best: Option<([f64; 2], f64)> = None;
    for (i, a) in assistants.iter().enumerate() {
        if i == skip {
            continue;
        }
        let q = match a {
            Assistant::VanishingPoint { p: vp } => [p[0], vp[1]],
            Assistant::TwoPointPerspective { a, b, .. } => project(*a, [b[0] - a[0], b[1] - a[1]], p),
            _ => continue,
        };
        let d = dist(q, p);
        if best.is_none_or(|(_, bd)| d < bd) {
            best = Some((q, d));
        }
    }
    best.map(|(q, _)| q)
}

/// How an assistant is drawn: its own colour and opacity (over the global ones), whether its
/// control handles show, and the area it is limited to (two corners), outside of which it neither
/// draws nor snaps.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AssistantStyle {
    /// sRGB 0..1; `None` uses the global colour.
    pub color: Option<[f32; 3]>,
    /// 0..1; `None` uses the global opacity.
    pub opacity: Option<f32>,
    pub show_handles: bool,
    /// Two opposite corners of the area, in document pixels.
    pub limit: Option<[[f64; 2]; 2]>,
}

impl Default for AssistantStyle {
    fn default() -> Self {
        Self { color: None, opacity: None, show_handles: true, limit: None }
    }
}

impl AssistantStyle {
    /// The limit as `(x0, y0, x1, y1)`, ordered.
    pub fn limit_rect(&self) -> Option<[f64; 4]> {
        let [a, b] = self.limit?;
        Some([a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])])
    }

    /// Is `p` inside the limit (always, without one)?
    pub fn admits(&self, p: [f64; 2]) -> bool {
        match self.limit_rect() {
            Some([x0, y0, x1, y1]) => p[0] >= x0 && p[0] <= x1 && p[1] >= y0 && p[1] <= y1,
            None => true,
        }
    }

    /// `segment` clipped to the limit; `None` when it lies outside (or the segment is empty).
    pub fn clip(&self, segment: [[f64; 2]; 2]) -> Option<[[f64; 2]; 2]> {
        let Some([x0, y0, x1, y1]) = self.limit_rect() else { return Some(segment) };
        // Liang–Barsky.
        let [p, q] = segment;
        let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
        let (mut t0, mut t1) = (0.0f64, 1.0f64);
        for (num, den) in [(x0 - p[0], dx), (p[0] - x1, -dx), (y0 - p[1], dy), (p[1] - y1, -dy)] {
            if den.abs() < 1e-12 {
                if num > 0.0 {
                    return None;
                }
                continue;
            }
            let t = num / den;
            if den < 0.0 {
                t0 = t0.max(t);
            } else {
                t1 = t1.min(t);
            }
            if t0 > t1 {
                return None;
            }
        }
        Some([[p[0] + dx * t0, p[1] + dy * t0], [p[0] + dx * t1, p[1] + dy * t1]])
    }
}

/// The global look of the assistants in a document.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AssistantDisplay {
    /// sRGB 0..1.
    pub color: [f32; 3],
    /// 0..1.
    pub opacity: f32,
    /// Draw the assistants with the painting tools too (View › Show Painting Assistants).
    pub show: bool,
    /// Draw the stroke preview (the snapped line) while painting (View › Show Assistant Previews).
    pub show_previews: bool,
}

impl Default for AssistantDisplay {
    fn default() -> Self {
        Self { color: [0.6, 0.3, 0.9], opacity: 0.7, show: true, show_previews: true }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f64; 2], b: [f64; 2]) -> bool {
        dist(a, b) < 1e-6
    }

    #[test]
    fn rulers_project_onto_their_lines() {
        let r = Assistant::from_points("ruler", &[[0.0, 10.0], [100.0, 10.0]]).unwrap();
        assert!(close(r.snap([5.0, 5.0], [40.0, 30.0]).unwrap().point, [40.0, 10.0]));
        let inf = Assistant::from_points("infiniteRuler", &[[0.0, 10.0], [100.0, 10.0]]).unwrap();
        assert!(close(inf.snap([5.0, 5.0], [400.0, 30.0]).unwrap().point, [400.0, 10.0]));
        assert!(inf.guides([50.0, 50.0])[0][1][0] > 100.0, "the infinite ruler runs past its handles");
        assert_eq!(r.guides([50.0, 50.0]), vec![[[0.0, 10.0], [100.0, 10.0]]], "the plain ruler is its segment");
        let p = Assistant::from_points("parallelRuler", &[[0.0, 0.0], [10.0, 10.0]]).unwrap();
        // Parallel to the diagonal, through the stroke's start.
        assert!(close(p.snap([50.0, 0.0], [60.0, 0.0]).unwrap().point, [55.0, 5.0]));
        assert!(Assistant::from_points("ruler", &[[1.0, 1.0], [1.0, 1.0]]).is_none());
        assert!(Assistant::from_points("ruler", &[[1.0, 1.0]]).is_none());
        assert!(Assistant::from_points("nope", &[[1.0, 1.0]]).is_none());
    }

    #[test]
    fn ruler_subdivisions_draw_notches_and_survive_moves() {
        let mut r = Assistant::from_points("ruler", &[[0.0, 0.0], [100.0, 0.0]]).unwrap();
        assert!(r.set_subdivisions(4, 1));
        assert_eq!(r.subdivisions(), Some((4, 1)));
        // The segment plus 9 notches (4 majors × 2 steps, both ends included).
        assert_eq!(r.guides([100.0, 100.0]).len(), 1 + 9);
        assert!(r.translate([5.0, 5.0]));
        assert_eq!(r.subdivisions(), Some((4, 1)), "moving keeps the notches");
        assert!(r.set_handle(1, [100.0, 50.0]));
        assert_eq!(r.subdivisions(), Some((4, 1)));
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"subdivisions\":4"));
        let back: Assistant = serde_json::from_str("{\"kind\":\"ruler\",\"a\":[0,0],\"b\":[1,0]}").unwrap();
        assert_eq!(back.subdivisions(), Some((0, 0)), "old files without notches still load");
    }

    #[test]
    fn vanishing_point_and_grid_snap_towards_their_points() {
        let vp = Assistant::from_points("vanishingPoint", &[[100.0, 0.0]]).unwrap();
        let s = vp.snap([0.0, 100.0], [40.0, 70.0]).unwrap();
        // On the line from (0,100) to (100,0): x + y = 100.
        assert!((s.point[0] + s.point[1] - 100.0).abs() < 1e-6);
        assert!(vp.snap([100.0, 0.0], [40.0, 70.0]).is_none(), "a stroke on the point has no direction");
        // A trapezoid: horizontal sides converge above, vertical sides are parallel.
        let g = Assistant::from_points("perspectiveGrid", &[[40.0, 0.0], [60.0, 0.0], [100.0, 100.0], [0.0, 100.0]]).unwrap();
        let [h, v] = g.vanishing().unwrap();
        assert!(h.is_err(), "top and bottom are parallel: {h:?}");
        let v = v.unwrap();
        assert!(close(v, [50.0, -25.0]), "{v:?}");
        // A near-horizontal stroke follows the horizontal direction; a near-vertical one heads to the point.
        assert!(close(g.snap([20.0, 50.0], [60.0, 52.0]).unwrap().point, [60.0, 50.0]));
        let up = g.snap([0.0, 100.0], [20.0, 20.0]).unwrap().point;
        let d = [v[0] - 0.0, v[1] - 100.0];
        let cross = (up[0] - 0.0) * d[1] - (up[1] - 100.0) * d[0];
        assert!(cross.abs() < 1e-6, "snapped point lies on the line to the vanishing point: {up:?}");
        assert_eq!(g.guides([100.0, 100.0]).len(), 4 + 6);
        // The perspective sensor: full size at the near edge, smaller towards the far one.
        let near = g.perspective_scale([50.0, 100.0]).unwrap();
        let far = g.perspective_scale([50.0, 0.0]).unwrap();
        assert!((near - 1.0).abs() < 1e-6, "{near}");
        assert!(far < near && far > 0.0, "{far}");
        assert!(vp.perspective_scale([0.0, 0.0]).is_none());
    }

    #[test]
    fn choice_handles_and_moves() {
        let mut list = vec![
            Assistant::from_points("ruler", &[[0.0, 0.0], [100.0, 0.0]]).unwrap(),
            Assistant::from_points("ruler", &[[0.0, 80.0], [100.0, 80.0]]).unwrap(),
            Assistant::from_points("vanishingPoint", &[[500.0, 500.0]]).unwrap(),
        ];
        assert_eq!(choose(&list, [10.0, 78.0], [40.0, 79.0]), Some(1));
        assert_eq!(choose(&list, [10.0, 2.0], [40.0, 1.0]), Some(0));
        assert_eq!(choose(&list, [10.0, 40.0], [20.0, 50.0]), Some(2));
        assert_eq!(choose_where(&list, [10.0, 2.0], [40.0, 1.0], |i| i != 0), Some(1));
        assert!(list[0].set_handle(1, [100.0, 50.0]));
        assert!(!list[0].set_handle(1, [0.0, 0.0]), "coincident ends are refused");
        assert!(!list[0].set_handle(2, [0.0, 0.0]));
        assert!(list[2].translate([1.0, 2.0]));
        assert_eq!(list[2].handles(), vec![[501.0, 502.0]]);
        assert_eq!(list[0].center(), [50.0, 25.0]);
        let json = serde_json::to_string(&list[0]).unwrap();
        assert!(json.contains("\"kind\":\"ruler\""));
        assert_eq!(serde_json::from_str::<Assistant>(&json).unwrap(), list[0]);
        // Alt keeps a dragged vanishing point on the other one's horizon.
        list.push(Assistant::from_points("vanishingPoint", &[[0.0, 300.0]]).unwrap());
        assert_eq!(horizon_for(&list, 3, [50.0, 310.0]), Some([50.0, 502.0]));
        assert_eq!(horizon_for(&list[..2], 0, [1.0, 1.0]), None);
    }

    #[test]
    fn ellipses_snap_onto_their_rims() {
        // Major axis 0..100 along x, minor radius 25.
        let e = Assistant::from_points("ellipse", &[[0.0, 50.0], [100.0, 50.0], [50.0, 25.0]]).unwrap();
        let g = e.ellipse().unwrap();
        assert!((g.rx - 50.0).abs() < 1e-9 && (g.ry - 25.0).abs() < 1e-9);
        let top = e.snap([50.0, 10.0], [50.0, 10.0]).unwrap().point;
        assert!(close(top, [50.0, 25.0]), "{top:?}");
        let right = e.snap([50.0, 10.0], [130.0, 50.0]).unwrap().point;
        assert!(close(right, [100.0, 50.0]), "{right:?}");
        // The third handle snaps to a circle when close to one.
        let mut c = e.clone();
        assert!(c.set_handle(2, [50.0, 2.0]));
        let g = c.ellipse().unwrap();
        assert!((g.ry - 50.0).abs() < 1e-9, "snapped to a circle: {g:?}");
        // Concentric: the rim through the stroke's start.
        let ce = Assistant::from_points("concentricEllipse", &[[0.0, 50.0], [100.0, 50.0], [50.0, 25.0]]).unwrap();
        let s = ce.snap([150.0, 50.0], [50.0, 120.0]).unwrap().point;
        // Scale 2: rx 100, ry 50 → bottom at (50, 100).
        assert!(close(s, [50.0, 100.0]), "{s:?}");
        assert!(Assistant::from_points("ellipse", &[[0.0, 50.0], [100.0, 50.0], [50.0, 50.0]]).is_none(), "a flat ellipse is refused");
        // A perspective ellipse in an axis-aligned square is the inscribed circle.
        let pe = Assistant::from_points("perspectiveEllipse", &[[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]]).unwrap();
        let q = pe.snap([50.0, 0.0], [50.0, -20.0]).unwrap().point;
        assert!(dist(q, [50.0, 0.0]) < 0.1, "{q:?}");
        assert!(pe.guides([100.0, 100.0]).len() > 4);
    }

    #[test]
    fn splines_fisheyes_and_curvilinear_grids_follow_curves() {
        let sp = Assistant::from_points("spline", &[[0.0, 0.0], [0.0, 100.0], [100.0, 100.0], [100.0, 0.0]]).unwrap();
        let q = sp.snap([0.0, 0.0], [50.0, 100.0]).unwrap().point;
        // The curve's midpoint is (50, 75).
        assert!(dist(q, [50.0, 75.0]) < 0.05, "{q:?}");
        assert_eq!(sp.guides([100.0, 100.0]).len(), 64 + 2);
        let fe = Assistant::from_points("fishEye", &[[0.0, 50.0], [100.0, 50.0], [50.0, 25.0]]).unwrap();
        // A stroke begun at (50, 30) follows the ellipse through the horizon ends with ry 20.
        let q = fe.snap([50.0, 30.0], [50.0, 0.0]).unwrap().point;
        assert!(close(q, [50.0, 30.0]), "{q:?}");
        let on_horizon = fe.snap([20.0, 50.0], [60.0, 70.0]).unwrap().point;
        assert!(close(on_horizon, [60.0, 50.0]), "{on_horizon:?}");
        let cv = Assistant::from_points("curvilinear", &[[0.0, 50.0], [100.0, 50.0]]).unwrap();
        // A stroke begun on the horizon stays on it.
        assert!(close(cv.snap([30.0, 50.0], [60.0, 60.0]).unwrap().point, [60.0, 50.0]));
        // One begun above it follows the circle through (0,50), (100,50) and the start.
        let q = cv.snap([50.0, 30.0], [20.0, 20.0]).unwrap().point;
        let (c, r) = circle_through([0.0, 50.0], [100.0, 50.0], [50.0, 30.0]).unwrap();
        assert!((dist(q, c) - r).abs() < 1e-6, "{q:?}");
        assert!(cv.guides([100.0, 100.0]).len() > 128);
        let tp = Assistant::from_points("twoPointPerspective", &[[0.0, 50.0], [200.0, 50.0], [100.0, 120.0]]).unwrap();
        // A near-vertical stroke runs perpendicular to the horizon.
        assert!(close(tp.snap([100.0, 100.0], [101.0, 150.0]).unwrap().point, [100.0, 150.0]));
        assert_eq!(tp.horizon(), Some([[0.0, 50.0], [200.0, 50.0]]));
        let mut tp2 = tp.clone();
        assert!(tp2.set_two_point(2.0, false));
        assert!(tp2.guides([200.0, 200.0]).len() > tp.guides([200.0, 200.0]).len() - 2);
        assert!(cv.is_curved() && !tp.is_curved());
    }

    #[test]
    fn styles_limit_where_assistants_draw_and_snap() {
        let mut st = AssistantStyle::default();
        assert!(st.admits([5.0, 5.0]));
        assert_eq!(st.clip([[0.0, 0.0], [10.0, 10.0]]), Some([[0.0, 0.0], [10.0, 10.0]]));
        st.limit = Some([[10.0, 10.0], [0.0, 0.0]]);
        assert_eq!(st.limit_rect(), Some([0.0, 0.0, 10.0, 10.0]));
        assert!(st.admits([5.0, 5.0]) && !st.admits([15.0, 5.0]));
        let c = st.clip([[-10.0, 5.0], [20.0, 5.0]]).unwrap();
        assert!(close(c[0], [0.0, 5.0]) && close(c[1], [10.0, 5.0]), "{c:?}");
        assert_eq!(st.clip([[20.0, 20.0], [30.0, 30.0]]), None);
        let d = AssistantDisplay::default();
        assert!(d.show && d.show_previews);
        let json = serde_json::to_string(&st).unwrap();
        assert_eq!(serde_json::from_str::<AssistantStyle>(&json).unwrap(), st);
    }
}
