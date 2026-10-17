//! Splines through control points, converted to the anchor model: the cubic uniform B-spline
//! ([`bspline_to_bezier`]) and a clean-room approximation of a Spiro-like G2 spline
//! ([`spiro_approx_to_bezier`]). Both serve the pen and pencil drawing modes and the live path
//! effects of the same names.
//!
//! The Spiro approximation is deliberately not a clothoid solver: each node takes the tangent of
//! the circle through it and its neighbours, every segment's handle lengths are those of the
//! circular arc fitting its two tangents, and a few relaxation passes then scale the handles so
//! the curvature either side of each node agrees. The result is G1 everywhere and G2 to within
//! a few percent on even input, which is what the drawing modes need; it is documented as an
//! approximation wherever it is shown.

use kurbo::{CubicBez, ParamCurveCurvature, Point, Vec2};

use crate::path::{Anchor, AnchorKind, SubPath};

/// The cubic uniform B-spline whose control polygon is `ctrl`, as Bezier anchors. The curve
/// never passes through the control points (except the ends of an open run, which it is clamped
/// to); `weight` in 0..=1 pulls it toward them: 1 is the B-spline itself, 0 the control polygon.
pub fn bspline_to_bezier(ctrl: &[Point], closed: bool, weight: f64) -> SubPath {
    let n = ctrl.len();
    if n < 3 {
        return SubPath::polyline(ctrl, closed);
    }
    let w = if weight.is_finite() { weight.clamp(0.0, 1.0) } else { 1.0 };
    // Control point `i` with the ends clamped (an open run repeats its end points).
    let at = |i: isize| -> Point { if closed { ctrl[(i.rem_euclid(n as isize)) as usize] } else { ctrl[i.clamp(0, n as isize - 1) as usize] } };
    let third = |a: Point, b: Point| Point::new((2.0 * a.x + b.x) / 3.0, (2.0 * a.y + b.y) / 3.0);
    // The junction of the spans on either side of control point `i` (where the curve is nearest
    // to it) and its two handles.
    let node = |i: isize| -> Anchor {
        let (p0, p1, p2) = (at(i - 1), at(i), at(i + 1));
        let j = Point::new((p0.x + 4.0 * p1.x + p2.x) / 6.0, (p0.y + 4.0 * p1.y + p2.y) / 6.0);
        let h_in = third(p1, p0);
        let h_out = third(p1, p2);
        let p = p1.lerp(j, w);
        let (h_in, h_out) = (p1.lerp(h_in, w), p1.lerp(h_out, w));
        if w <= 1e-9 { Anchor::corner(p) } else { Anchor { p, h_in, h_out, kind: AnchorKind::Smooth } }
    };
    let mut anchors: Vec<Anchor> = (0..n as isize).map(node).collect();
    if !closed {
        // Clamped ends: the curve starts and ends on the end control points.
        let (first, last) = (ctrl[0], ctrl[n - 1]);
        anchors[0] = Anchor { p: first, h_in: first, h_out: first.lerp(third(first, ctrl[1]), w), kind: AnchorKind::Corner };
        let l = n - 1;
        anchors[l] = Anchor { p: last, h_in: last.lerp(third(last, ctrl[l - 1]), w), h_out: last, kind: AnchorKind::Corner };
    }
    SubPath::new(anchors, closed)
}

/// A smooth spline through `pts`, approximating a Spiro (clothoid) spline: see the module notes.
/// `corner[i]` (when given) keeps point `i` a sharp corner with straight tangents from its
/// neighbours instead of a smooth node.
pub fn spiro_approx_to_bezier(pts: &[Point], closed: bool, corner: Option<&[bool]>) -> SubPath {
    let n = pts.len();
    if n < 2 || (n == 2 && !closed) {
        return SubPath::polyline(pts, closed);
    }
    let is_corner = |i: usize| corner.is_some_and(|c| c.get(i).copied().unwrap_or(false));
    let prev = |i: usize| -> Option<Point> {
        if i > 0 {
            Some(pts[i - 1])
        } else if closed {
            Some(pts[n - 1])
        } else {
            None
        }
    };
    let next = |i: usize| -> Option<Point> {
        if i + 1 < n {
            Some(pts[i + 1])
        } else if closed {
            Some(pts[0])
        } else {
            None
        }
    };
    // Unit tangent (direction of travel) at each node.
    let tangents: Vec<Vec2> = (0..n)
        .map(|i| match (prev(i), next(i)) {
            (Some(a), Some(b)) if !is_corner(i) => circle_tangent(a, pts[i], b),
            (Some(a), None) => unit(pts[i] - a),
            (None, Some(b)) => unit(b - pts[i]),
            (Some(a), Some(b)) => unit(b - a),
            (None, None) => Vec2::new(1.0, 0.0),
        })
        .collect();
    let seg_count = if closed { n } else { n - 1 };
    // Handle lengths per segment end: (out of node i, into node i+1).
    let mut lens: Vec<(f64, f64)> = (0..seg_count)
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            let ta = if is_corner(i) { unit(b - a) } else { tangents[i] };
            let tb = if is_corner((i + 1) % n) { unit(b - a) } else { tangents[(i + 1) % n] };
            arc_handles(a, ta, b, tb)
        })
        .collect();
    // Relax: at each smooth interior node, scale the two meeting handles so the curvatures agree.
    for _ in 0..4 {
        for i in 0..n {
            if is_corner(i) {
                continue;
            }
            let (Some(si), Some(so)) = (seg_before(i, n, closed), seg_after(i, n, closed)) else { continue };
            let cin = cubic_of(pts, &tangents, &lens, si, n, &is_corner);
            let cout = cubic_of(pts, &tangents, &lens, so, n, &is_corner);
            let (ka, kb) = (cin.curvature(1.0).abs(), cout.curvature(0.0).abs());
            if !(ka.is_finite() && kb.is_finite()) || ka + kb < 1e-12 {
                continue;
            }
            // Curvature at an end goes as 1/len²: scale each handle toward the mean curvature,
            // half way, which converges without overshooting on even input.
            let mean = (ka + kb) / 2.0;
            let s_in = ((ka / mean).sqrt() - 1.0) * 0.5 + 1.0;
            let s_out = ((kb / mean).sqrt() - 1.0) * 0.5 + 1.0;
            lens[si].1 = (lens[si].1 * s_in).clamp(1e-6, 1e6);
            lens[so].0 = (lens[so].0 * s_out).clamp(1e-6, 1e6);
        }
    }
    let anchors = (0..n)
        .map(|i| {
            let p = pts[i];
            let h_in = match seg_before(i, n, closed) {
                Some(s) => {
                    let t = if is_corner(i) { unit(p - pts[s]) } else { tangents[i] };
                    p - t * lens[s].1
                }
                None => p,
            };
            let h_out = match seg_after(i, n, closed) {
                Some(s) => {
                    let t = if is_corner(i) { unit(pts[(i + 1) % n] - p) } else { tangents[i] };
                    p + t * lens[s].0
                }
                None => p,
            };
            let kind = if is_corner(i) || seg_before(i, n, closed).is_none() || seg_after(i, n, closed).is_none() {
                AnchorKind::Corner
            } else {
                AnchorKind::Smooth
            };
            Anchor { p, h_in, h_out, kind }
        })
        .collect();
    SubPath::new(anchors, closed)
}

fn seg_before(i: usize, n: usize, closed: bool) -> Option<usize> {
    if i > 0 {
        Some(i - 1)
    } else if closed {
        Some(n - 1)
    } else {
        None
    }
}

fn seg_after(i: usize, n: usize, closed: bool) -> Option<usize> {
    let seg_count = if closed { n } else { n - 1 };
    (i < seg_count).then_some(i)
}

fn cubic_of(pts: &[Point], tangents: &[Vec2], lens: &[(f64, f64)], s: usize, n: usize, is_corner: impl Fn(usize) -> bool) -> CubicBez {
    let (i, j) = (s, (s + 1) % n);
    let (a, b) = (pts[i], pts[j]);
    let ta = if is_corner(i) { unit(b - a) } else { tangents[i] };
    let tb = if is_corner(j) { unit(b - a) } else { tangents[j] };
    CubicBez::new(a, a + ta * lens[s].0, b - tb * lens[s].1, b)
}

/// Unit tangent at `p` of the circle through `a`, `p`, `b` (the chord direction when they are
/// collinear), pointing from `a` towards `b`.
fn circle_tangent(a: Point, p: Point, b: Point) -> Vec2 {
    let (u, v) = (unit(p - a), unit(b - p));
    let s = u + v;
    if s.hypot() < 1e-9 {
        // A hairpin: the tangent is perpendicular to the chords.
        return Vec2::new(-u.y, u.x);
    }
    // The tangent of the circumcircle at `p` bisects the two chord directions.
    unit(s)
}

/// Handle lengths of the cubic from `a` (tangent `ta`) to `b` (tangent `tb`) that best follows a
/// circular arc through both ends: for a turn of `theta` between the tangents the arc has radius
/// `chord / (2 sin(theta/2))` and a cubic approximates it with handles `4/3 tan(theta/4) r`. When
/// the tangents disagree with the chord (an S shape) each end takes its own arc from its tangent.
fn arc_handles(a: Point, ta: Vec2, b: Point, tb: Vec2) -> (f64, f64) {
    let chord = b - a;
    let d = chord.hypot();
    if d < 1e-12 {
        return (0.0, 0.0);
    }
    let c = chord / d;
    let one = |t: Vec2| -> f64 {
        // Angle between the tangent and the chord; an arc through both ends turns twice that.
        let cos = t.dot(c).clamp(-1.0, 1.0);
        let half = cos.acos().min(std::f64::consts::FRAC_PI_2 * 0.98);
        if half < 1e-6 {
            return d / 3.0;
        }
        let r = d / (2.0 * half.sin());
        (4.0 / 3.0) * (half / 2.0).tan() * r
    };
    (one(ta).clamp(d * 0.05, d * 1.5), one(tb).clamp(d * 0.05, d * 1.5))
}

fn unit(v: Vec2) -> Vec2 {
    let h = v.hypot();
    if h > 1e-12 { v / h } else { Vec2::new(1.0, 0.0) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Vec<Point> {
        vec![Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 100.0), Point::new(0.0, 100.0)]
    }

    #[test]
    fn bspline_is_inside_the_control_polygon_and_clamped_to_open_ends() {
        let sp = bspline_to_bezier(&square(), true, 1.0);
        assert_eq!(sp.anchors.len(), 4);
        for a in &sp.anchors {
            assert!(a.p.x > 0.0 && a.p.x < 100.0 && a.p.y > 0.0 && a.p.y < 100.0, "{:?}", a.p);
            assert_eq!(a.kind, AnchorKind::Smooth);
        }
        let open = bspline_to_bezier(&square(), false, 1.0);
        assert_eq!(open.anchors[0].p, Point::new(0.0, 0.0));
        assert_eq!(open.anchors[3].p, Point::new(0.0, 100.0));
        // Weight 0 is the polygon itself.
        let poly = bspline_to_bezier(&square(), true, 0.0);
        assert_eq!(poly.anchors.iter().map(|a| a.p).collect::<Vec<_>>(), square());
    }

    #[test]
    fn spiro_through_a_square_is_a_smooth_closed_loop_through_the_points() {
        let sp = spiro_approx_to_bezier(&square(), true, None);
        assert!(sp.closed);
        for (a, p) in sp.anchors.iter().zip(square()) {
            assert_eq!(a.p, p);
            assert_eq!(a.kind, AnchorKind::Smooth);
            assert!(a.has_in() && a.has_out());
        }
        // Curvature agrees across the nodes to within a few percent (it is a circle, nearly).
        let n = sp.anchors.len();
        for i in 0..n {
            let before = sp.segment((i + n - 1) % n);
            let after = sp.segment(i);
            let (ka, kb) = (before.curvature(1.0).abs(), after.curvature(0.0).abs());
            assert!((ka - kb).abs() <= 0.05 * ka.max(kb), "node {i}: {ka} vs {kb}");
        }
    }

    #[test]
    fn spiro_corners_stay_sharp_and_ends_of_open_runs_are_corners() {
        let pts = square();
        let sp = spiro_approx_to_bezier(&pts, false, Some(&[false, true, false, false]));
        assert_eq!(sp.anchors[0].kind, AnchorKind::Corner);
        assert_eq!(sp.anchors[1].kind, AnchorKind::Corner);
        assert_eq!(sp.anchors[2].kind, AnchorKind::Smooth);
        assert_eq!(sp.anchors[3].kind, AnchorKind::Corner);
        // The corner's handles lie on its chords.
        let a = &sp.anchors[1];
        assert!((a.h_in.y - a.p.y).abs() < 1e-9 && (a.h_out.x - a.p.x).abs() < 1e-9);
    }
}
