//! Splines the Pen, the Pencil and the spline live effects draw through or along a run of points,
//! converted to the anchor model (cubic Béziers) so the result is an ordinary editable path.
//!
//! * [`bspline_to_bezier`]: a cubic uniform B-spline whose control polygon is the points. The
//!   curve is smooth (C2) and hugs the polygon without passing through its inner points; an open
//!   run is clamped so it starts and ends on its first and last point.
//! * [`spiro_approx_to_bezier`]: an interpolating spline through the points, as close to
//!   curvature-continuous (G2) as a cubic per span can get without solving the clothoid: tangents
//!   come from a Catmull-Rom style blend of the neighbouring chords, handle lengths start at the
//!   circular-arc fit of each span and are then relaxed a few rounds so the curvature on both
//!   sides of every point agrees. The Spiro mode itself solves the clothoid spline
//!   ([`crate::clothoid`]); this one is kept for the corner-aware spline live effect.
//! * [`paraxial`]: the next point of an axis-aligned (paraxial) polyline, every segment
//!   horizontal or vertical and perpendicular to the one before.

use kurbo::{Point, Vec2};

use crate::path::{Anchor, AnchorKind, SubPath};

/// How the Pen and the Pencil turn the points they are given into a path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SplineMode {
    /// Plain Bézier segments: corners and dragged-out handles as placed.
    #[default]
    Bezier,
    /// A clothoid spline through the points ([`crate::clothoid::clothoid_to_bezier`]).
    Spiro,
    /// [`bspline_to_bezier`] with the points as the control polygon.
    BSpline,
    /// Straight segments snapped to the axes ([`paraxial`]).
    Paraxial,
}

impl SplineMode {
    /// Every mode, in the order the control bar lists them.
    pub const ALL: [SplineMode; 4] = [SplineMode::Bezier, SplineMode::Spiro, SplineMode::BSpline, SplineMode::Paraxial];

    /// The stable id used in tool options and command params.
    pub fn id(self) -> &'static str {
        match self {
            SplineMode::Bezier => "regular",
            SplineMode::Spiro => "spiro",
            SplineMode::BSpline => "bspline",
            SplineMode::Paraxial => "paraxial",
        }
    }

    /// The mode with that id (`regular`|`bezier`, `spiro`, `bspline`, `paraxial`), ignoring case.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "regular" | "bezier" => SplineMode::Bezier,
            "spiro" => SplineMode::Spiro,
            "bspline" | "b-spline" => SplineMode::BSpline,
            "paraxial" => SplineMode::Paraxial,
            _ => return None,
        })
    }

    /// Does the mode rebuild the whole path from its points on every edit (the spline modes)?
    pub fn is_spline(self) -> bool {
        matches!(self, SplineMode::Spiro | SplineMode::BSpline)
    }

    /// The spline of this mode through (or along) `pts`; `None` for the modes that place
    /// segments one by one ([`SplineMode::Bezier`], [`SplineMode::Paraxial`]).
    pub fn through(self, pts: &[Point], closed: bool) -> Option<SubPath> {
        match self {
            SplineMode::Spiro => Some(crate::clothoid::clothoid_to_bezier(pts, closed)),
            SplineMode::BSpline => Some(bspline_to_bezier(pts, closed)),
            _ => None,
        }
    }
}

/// `pts` without consecutive duplicates (and, closed, without a last point equal to the first).
fn dedup(pts: &[Point], closed: bool) -> Vec<Point> {
    let mut out: Vec<Point> = Vec::with_capacity(pts.len());
    for &p in pts {
        if p.x.is_finite() && p.y.is_finite() && out.last().is_none_or(|l| l.distance(p) > 1e-9) {
            out.push(p);
        }
    }
    while closed && out.len() > 1 && out.first().zip(out.last()).is_some_and(|(f, l)| f.distance(*l) <= 1e-9) {
        out.pop();
    }
    out
}

/// A cubic uniform B-spline with `ctrl` as its control polygon, as cubic Bézier anchors. Open: the
/// curve is clamped to start at the first control point and end at the last one (the end spans
/// are the knot-insertion result of a triple end knot). Closed: the polygon wraps round and every
/// anchor is a knot point, so the curve has no start. Fewer than three points give a polyline.
pub fn bspline_to_bezier(ctrl: &[Point], closed: bool) -> SubPath {
    let c = dedup(ctrl, closed);
    let n = c.len();
    if n < 3 {
        return SubPath::polyline(&c, closed && n > 2);
    }
    // Knot point between control points a, b, c and the handles either side of it.
    let knot = |a: Point, b: Point, d: Point| Point::new((a.x + 4.0 * b.x + d.x) / 6.0, (a.y + 4.0 * b.y + d.y) / 6.0);
    let third = |a: Point, b: Point| Point::new((2.0 * a.x + b.x) / 3.0, (2.0 * a.y + b.y) / 3.0);
    let mut anchors = Vec::with_capacity(n);
    if closed {
        for k in 0..n {
            let (prev, cur, next) = (c[(k + n - 1) % n], c[k], c[(k + 1) % n]);
            anchors.push(Anchor { p: knot(prev, cur, next), h_in: third(cur, prev), h_out: third(cur, next), kind: AnchorKind::Smooth });
        }
        return SubPath::new(anchors, true);
    }
    anchors.push(Anchor { p: c[0], h_in: c[0], h_out: third(c[0], c[1]), kind: AnchorKind::Corner });
    for k in 1..n - 1 {
        let (prev, cur, next) = (c[k - 1], c[k], c[k + 1]);
        anchors.push(Anchor { p: knot(prev, cur, next), h_in: third(cur, prev), h_out: third(cur, next), kind: AnchorKind::Smooth });
    }
    anchors.push(Anchor { p: c[n - 1], h_in: third(c[n - 1], c[n - 2]), h_out: c[n - 1], kind: AnchorKind::Corner });
    SubPath::new(anchors, false)
}

/// Unit vector of `v`, or `None` when it has no length.
fn unit(v: Vec2) -> Option<Vec2> {
    let h = v.hypot();
    (h > 1e-12).then(|| v / h)
}

/// Handle length fitting a circular arc over a chord of length `d` whose tangent leaves the chord
/// at `phi` radians: `d / 3` for a straight span, longer as the arc bulges.
fn arc_handle(d: f64, phi: f64) -> f64 {
    let phi = phi.abs().min(80f64.to_radians());
    if phi < 1e-6 {
        return d / 3.0;
    }
    (2.0 / 3.0) * d * (phi / 2.0).tan() / phi.sin()
}

/// Signed curvature of the cubic (p0, p1, p2, p3) at its start.
fn curvature_start(p0: Point, p1: Point, p2: Point, _p3: Point) -> f64 {
    let a = p1 - p0;
    let l = a.hypot();
    if l < 1e-9 {
        return 0.0;
    }
    (2.0 / 3.0) * a.cross(p2 - p1) / (l * l * l)
}

/// Signed curvature of the cubic (p0, p1, p2, p3) at its end (same sign convention as the start).
fn curvature_end(p0: Point, p1: Point, p2: Point, p3: Point) -> f64 {
    // Reversed, the curve runs the other way: its curvature changes sign.
    -curvature_start(p3, p2, p1, p0)
}

/// Rounds of handle-length relaxation in [`spiro_approx_to_bezier`].
const RELAX_ROUNDS: usize = 8;

/// An interpolating spline through `pts`: every point is an anchor the curve passes through, with
/// handles in line (smooth), chosen so the curvature on both sides of each anchor is as equal as
/// cubics allow. An approximation of a clothoid spline, documented as such: tangents are the
/// bisector of the chords meeting at the point (an end's tangent mirrors its neighbour's, so an
/// open curve leaves its ends with little curvature), handles start at the circular-arc fit of
/// each span and are relaxed [`RELAX_ROUNDS`] times towards matching curvature. Fewer than three
/// points give a line.
pub fn spiro_approx_to_bezier(pts: &[Point], closed: bool) -> SubPath {
    let p = dedup(pts, closed);
    let n = p.len();
    if n < 3 {
        return SubPath::polyline(&p, false);
    }
    let closed = closed && n > 2;
    let segs = if closed { n } else { n - 1 };
    // Chord directions and lengths of each span i (from p[i] to p[(i + 1) % n]).
    let chord: Vec<(Vec2, f64)> = (0..segs)
        .map(|i| {
            let v = p[(i + 1) % n] - p[i];
            (unit(v).unwrap_or(Vec2::new(1.0, 0.0)), v.hypot())
        })
        .collect();
    // Tangents: bisector of the chords either side; at an open end, the neighbour's tangent
    // mirrored over the end chord.
    let mut tangent = vec![Vec2::ZERO; n];
    for i in 0..n {
        let interior = closed || (i > 0 && i + 1 < n);
        if interior {
            let (u_prev, _) = chord[(i + segs - 1) % segs];
            let (u_next, _) = chord[i % segs];
            tangent[i] = unit(u_prev + u_next).unwrap_or(u_next);
        }
    }
    if !closed {
        let (u0, _) = chord[0];
        tangent[0] = unit(u0 * 2.0 - tangent[1]).unwrap_or(u0);
        let (ul, _) = chord[segs - 1];
        tangent[n - 1] = unit(ul * 2.0 - tangent[n - 2]).unwrap_or(ul);
    }
    // Handle lengths per span: (out of its start, into its end), from the circular-arc fit.
    let mut len: Vec<(f64, f64)> = (0..segs)
        .map(|i| {
            let (u, d) = chord[i];
            let (a, b) = (i, (i + 1) % n);
            let phi_a = tangent[a].cross(u).atan2(tangent[a].dot(u));
            let phi_b = tangent[b].cross(u).atan2(tangent[b].dot(u));
            (arc_handle(d, phi_a), arc_handle(d, phi_b))
        })
        .collect();
    let cubic = |i: usize, len: &[(f64, f64)]| {
        let (a, b) = (i, (i + 1) % n);
        (p[a], p[a] + tangent[a] * len[i].0, p[b] - tangent[b] * len[i].1, p[b])
    };
    // Relax: at each joint, the curvature arriving (end of the span before) and leaving (start of
    // the span after) move towards their geometric mean. Curvature at a handle's end scales with
    // 1 / length², so the fourth root of the ratio equalises them in one step; a damped step and a
    // clamp keep the rounds stable.
    for _ in 0..RELAX_ROUNDS {
        let before = len.clone();
        for j in 0..n {
            let (into, out_of) = if closed {
                ((j + segs - 1) % segs, j % segs)
            } else if j > 0 && j + 1 < n {
                (j - 1, j)
            } else {
                continue;
            };
            let (a0, a1, a2, a3) = cubic(into, &before);
            let (b0, b1, b2, b3) = cubic(out_of, &before);
            let k_in = curvature_end(a0, a1, a2, a3);
            let k_out = curvature_start(b0, b1, b2, b3);
            if k_in.abs() < 1e-9 || k_out.abs() < 1e-9 || (k_in > 0.0) != (k_out > 0.0) {
                continue;
            }
            let ratio = (k_in.abs() / k_out.abs()).powf(0.125).clamp(0.75, 1.3333);
            let d_in = chord[into].1;
            let d_out = chord[out_of].1;
            len[into].1 = (len[into].1 * ratio).clamp(d_in * 0.05, d_in * 0.8);
            len[out_of].0 = (len[out_of].0 / ratio).clamp(d_out * 0.05, d_out * 0.8);
        }
    }
    let mut anchors = Vec::with_capacity(n);
    for i in 0..n {
        let h_in = if closed || i > 0 { p[i] - tangent[i] * len[(i + segs - 1) % segs].1 } else { p[i] };
        let h_out = if closed || i + 1 < n { p[i] + tangent[i] * len[i % segs].0 } else { p[i] };
        let kind = if (closed || (i > 0 && i + 1 < n)) && h_in.distance(p[i]) > 1e-9 && h_out.distance(p[i]) > 1e-9 {
            AnchorKind::Smooth
        } else {
            AnchorKind::Corner
        };
        anchors.push(Anchor { p: p[i], h_in, h_out, kind });
    }
    SubPath::new(anchors, closed)
}

/// Where a paraxial (axis-aligned) segment from `prev` towards `p` ends: on the horizontal or the
/// vertical through `prev`, perpendicular to the segment before it (`before` → `prev`) when there
/// is one, else along the axis `p` is further out on. `flip` swaps the axis (Shift held).
pub fn paraxial(prev: Point, before: Option<Point>, p: Point, flip: bool) -> Point {
    let d = p - prev;
    let horizontal = match before {
        Some(b) if b.distance(prev) > 1e-9 => {
            let v = prev - b;
            // The segment before ran along x: this one runs along y.
            v.x.abs() < v.y.abs()
        }
        _ => d.x.abs() >= d.y.abs(),
    } ^ flip;
    if horizontal { Point::new(p.x, prev.y) } else { Point::new(prev.x, p.y) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::{CubicBez, ParamCurve};

    fn pts(v: &[(f64, f64)]) -> Vec<Point> {
        v.iter().map(|&(x, y)| Point::new(x, y)).collect()
    }

    #[test]
    fn bspline_open_is_clamped_and_smooth() {
        let c = pts(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
        let sp = bspline_to_bezier(&c, false);
        assert_eq!(sp.anchors.len(), 4);
        assert_eq!(sp.anchors[0].p, c[0]);
        assert_eq!(sp.anchors[3].p, c[3]);
        assert!(!sp.closed);
        // Inner knots sit inside the hull, not on the control points, with collinear handles.
        let a = &sp.anchors[1];
        assert!(a.p.x < 100.0 && a.p.y > 0.0);
        assert_eq!(a.kind, AnchorKind::Smooth);
        let (vi, vo) = (a.p - a.h_in, a.h_out - a.p);
        assert!(vi.cross(vo).abs() < 1e-9 && vi.dot(vo) > 0.0);
    }

    #[test]
    fn bspline_closed_wraps() {
        let c = pts(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
        let sp = bspline_to_bezier(&c, true);
        assert_eq!(sp.anchors.len(), 4);
        assert!(sp.closed);
        assert!(sp.anchors.iter().all(|a| a.kind == AnchorKind::Smooth));
        // Symmetric input: a rounded square centred on (50, 50).
        let a = &sp.anchors[0];
        assert!((a.p.x - 100.0 / 6.0).abs() < 1e-9 && (a.p.y - 100.0 / 6.0).abs() < 1e-9);
    }

    #[test]
    fn bspline_short_runs_are_polylines() {
        let sp = bspline_to_bezier(&pts(&[(0.0, 0.0), (10.0, 0.0)]), false);
        assert_eq!(sp.anchors.len(), 2);
        assert!(sp.segment_is_line(0));
        assert_eq!(bspline_to_bezier(&pts(&[(1.0, 1.0), (1.0, 1.0)]), false).anchors.len(), 1);
    }

    #[test]
    fn spiro_passes_through_points_smoothly() {
        let p = pts(&[(0.0, 0.0), (50.0, 50.0), (100.0, 0.0), (150.0, 50.0)]);
        let sp = spiro_approx_to_bezier(&p, false);
        assert_eq!(sp.anchors.len(), 4);
        for (a, q) in sp.anchors.iter().zip(&p) {
            assert_eq!(a.p, *q);
        }
        assert_eq!(sp.anchors[1].kind, AnchorKind::Smooth);
        assert!(!sp.anchors[0].has_in() && !sp.anchors[3].has_out());
        // The middle tangent is the bisector of the chords: horizontal at the peak.
        assert!((sp.anchors[1].h_out.y - 50.0).abs() < 1e-9);
    }

    #[test]
    fn spiro_square_is_nearly_a_circle() {
        let r = 100.0;
        let p = pts(&[(r, 0.0), (0.0, r), (-r, 0.0), (0.0, -r)]);
        let sp = spiro_approx_to_bezier(&p, true);
        assert!(sp.closed);
        for i in 0..sp.segment_count() {
            let c: CubicBez = sp.segment(i);
            for k in 1..8 {
                let q = c.eval(k as f64 / 8.0);
                let d = q.to_vec2().hypot();
                assert!((d - r).abs() < r * 0.01, "segment {i}: radius {d}");
            }
        }
        // Curvature agrees on both sides of every anchor.
        for i in 0..4 {
            let a: CubicBez = sp.segment((i + 3) % 4);
            let b: CubicBez = sp.segment(i);
            let k_in = curvature_end(a.p0, a.p1, a.p2, a.p3);
            let k_out = curvature_start(b.p0, b.p1, b.p2, b.p3);
            assert!((k_in - k_out).abs() < 1e-6, "{k_in} vs {k_out}");
        }
    }

    #[test]
    fn spiro_relaxation_balances_curvature() {
        // Uneven spacing: the arc fit alone leaves a curvature jump; relaxation closes it.
        let p = pts(&[(0.0, 0.0), (30.0, 40.0), (150.0, 60.0), (200.0, 0.0)]);
        let sp = spiro_approx_to_bezier(&p, false);
        let a: CubicBez = sp.segment(0);
        let b: CubicBez = sp.segment(1);
        let k_in = curvature_end(a.p0, a.p1, a.p2, a.p3);
        let k_out = curvature_start(b.p0, b.p1, b.p2, b.p3);
        assert!((k_in - k_out).abs() <= 0.35 * k_in.abs().max(k_out.abs()) + 1e-9, "{k_in} vs {k_out}");
    }

    #[test]
    fn paraxial_alternates_axes() {
        let o = Point::new(0.0, 0.0);
        assert_eq!(paraxial(o, None, Point::new(50.0, 10.0), false), Point::new(50.0, 0.0));
        assert_eq!(paraxial(o, None, Point::new(10.0, 50.0), false), Point::new(0.0, 50.0));
        assert_eq!(paraxial(o, None, Point::new(50.0, 10.0), true), Point::new(0.0, 10.0));
        // After a horizontal segment the next is vertical whatever the pointer favours.
        let prev = Point::new(50.0, 0.0);
        assert_eq!(paraxial(prev, Some(o), Point::new(120.0, 20.0), false), Point::new(50.0, 20.0));
        assert_eq!(paraxial(prev, Some(o), Point::new(120.0, 20.0), true), Point::new(120.0, 0.0));
    }

    #[test]
    fn mode_ids_round_trip() {
        for m in SplineMode::ALL {
            assert_eq!(SplineMode::parse(m.id()), Some(m));
        }
        assert_eq!(SplineMode::parse("Bezier"), Some(SplineMode::Bezier));
        assert_eq!(SplineMode::parse("nope"), None);
        assert!(SplineMode::Spiro.through(&pts(&[(0.0, 0.0), (1.0, 2.0), (3.0, 0.0)]), false).is_some());
        assert!(SplineMode::Paraxial.through(&[], false).is_none());
    }
}
