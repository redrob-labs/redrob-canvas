//! A clothoid ("spiro") spline: an interpolating curve through a run of points made of Euler
//! spiral pieces, one per span, whose curvature is continuous (G2) across every point, converted
//! to one cubic Bézier per span for the anchor model.
//!
//! An Euler spiral (clothoid) is the curve whose curvature changes linearly with arc length. A
//! piece between two points with given end tangents has one degree of freedom left (how fast its
//! curvature changes), fixed here so the piece lands on its chord ([`fit_span`]). The tangent
//! angle at every point is then the unknown of the spline: it is solved so the curvature arriving
//! at a point equals the curvature leaving it ([`solve`]); an open curve's two ends are left
//! straight (zero curvature), as a spiro spline's free ends are. The maths is the classic
//! two-parameter clothoid interpolation; nothing here is derived from any other program's code.

use kurbo::{Point, Vec2};

use crate::path::{Anchor, AnchorKind, SubPath};

/// Quadrature points along a normalised span for the position integral.
const QUAD: usize = 24;
/// Rounds of the tangent-angle solve.
const SOLVE_ROUNDS: usize = 24;
/// Rounds of the cubic handle-length fit.
const HANDLE_ROUNDS: usize = 16;

/// One clothoid piece on the unit chord: the tangent angle relative to the chord is
/// `theta(t) = a0 + b t + c t²` for `t` in 0..1 (arc-length parameter), so the curvature is
/// linear; `len` is the piece's arc length for a chord of length 1.
#[derive(Clone, Copy, Debug)]
struct Span {
    a0: f64,
    b: f64,
    c: f64,
    len: f64,
}

impl Span {
    /// Curvature at the start and the end, for a chord of length `d`.
    fn curvatures(&self, d: f64) -> (f64, f64) {
        let l = (self.len * d).max(1e-12);
        (self.b / l, (self.b + 2.0 * self.c) / l)
    }
}

/// The end point of the normalised piece with tangent polynomial `a0 + b t + c t²`, by quadrature
/// (midpoint rule on `QUAD` cells: smooth integrand, and the bisection below only needs the sign
/// and rough size of its y part).
fn end_point(a0: f64, b: f64, c: f64) -> Vec2 {
    let mut x = 0.0;
    let mut y = 0.0;
    let h = 1.0 / QUAD as f64;
    for i in 0..QUAD {
        let t = (i as f64 + 0.5) * h;
        let th = a0 + b * t + c * t * t;
        x += th.cos() * h;
        y += th.sin() * h;
    }
    Vec2::new(x, y)
}

/// The clothoid piece from a point to the next along a chord of direction angle 0, leaving at
/// angle `alpha0` and arriving at angle `alpha1` (both relative to the chord): the curvature
/// variation `c` that makes the piece end on the chord, found by bisection on the end point's
/// y component (monotone in `c` over the angles a spline meets), then the scale that makes the
/// chord length 1.
fn fit_span(alpha0: f64, alpha1: f64) -> Span {
    // theta(1) = alpha1 fixes b + c; c is free.
    let total = alpha1 - alpha0;
    let y_of = |c: f64| end_point(alpha0, total - c, c).y;
    // Bracket the root: a straight-ish guess first, then widen.
    let (mut lo, mut hi) = (-1.0f64, 1.0f64);
    let (mut y_lo, mut y_hi) = (y_of(lo), y_of(hi));
    let mut widen = 0;
    while y_lo.signum() == y_hi.signum() && widen < 8 {
        lo *= 2.0;
        hi *= 2.0;
        y_lo = y_of(lo);
        y_hi = y_of(hi);
        widen += 1;
    }
    let c = if y_lo.signum() == y_hi.signum() {
        // No crossing (a doubled-back span): the symmetric piece is the sanest.
        0.0
    } else {
        for _ in 0..36 {
            let mid = 0.5 * (lo + hi);
            let y_mid = y_of(mid);
            if y_mid.signum() == y_lo.signum() {
                lo = mid;
                y_lo = y_mid;
            } else {
                hi = mid;
            }
        }
        0.5 * (lo + hi)
    };
    let b = total - c;
    let end = end_point(alpha0, b, c);
    // The piece's chord is `end.x` long for unit arc length: scale to a unit chord.
    let len = 1.0 / end.x.abs().max(1e-6);
    Span { a0: alpha0, b, c, len }
}

/// Angle of `v`.
fn angle(v: Vec2) -> f64 {
    v.y.atan2(v.x)
}

/// `a` wrapped into −π..π.
fn wrap(a: f64) -> f64 {
    (a + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
}

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

/// Span `i` of the spline for tangent angles `theta` (absolute, one per point): (span, chord
/// angle, chord length).
fn span_at(p: &[Point], theta: &[f64], i: usize) -> (Span, f64, f64) {
    let n = p.len();
    let (a, b) = (i, (i + 1) % n);
    let v = p[b] - p[a];
    let phi = angle(v);
    let alpha0 = wrap(theta[a] - phi);
    let alpha1 = wrap(theta[b] - phi);
    (fit_span(alpha0, alpha1), phi, v.hypot())
}

/// Every span of the spline.
fn spans(p: &[Point], theta: &[f64], closed: bool) -> Vec<(Span, f64, f64)> {
    let n = p.len();
    let segs = if closed { n } else { n - 1 };
    (0..segs).map(|i| span_at(p, theta, i)).collect()
}

/// Refit the spans that meet at point `j` after its tangent angle changed.
fn refit_around(sp: &mut [(Span, f64, f64)], p: &[Point], theta: &[f64], j: usize, closed: bool) {
    let n = p.len();
    let segs = sp.len();
    if closed || j > 0 {
        let into = (j + segs - 1) % segs;
        sp[into] = span_at(p, theta, into);
    }
    if closed || j + 1 < n {
        let out_of = j % segs;
        sp[out_of] = span_at(p, theta, out_of);
    }
}

/// The curvature mismatch at point `j`: the curvature arriving along the span before less the
/// curvature leaving along the span after. At an open end the condition is zero curvature there.
fn residual(sp: &[(Span, f64, f64)], j: usize, n: usize, closed: bool) -> f64 {
    let segs = sp.len();
    if closed {
        let (into, out_of) = ((j + segs - 1) % segs, j % segs);
        sp[into].0.curvatures(sp[into].2).1 - sp[out_of].0.curvatures(sp[out_of].2).0
    } else if j == 0 {
        -sp[0].0.curvatures(sp[0].2).0
    } else if j + 1 == n {
        sp[segs - 1].0.curvatures(sp[segs - 1].2).1
    } else {
        sp[j - 1].0.curvatures(sp[j - 1].2).1 - sp[j].0.curvatures(sp[j].2).0
    }
}

/// Solve the tangent angles so the curvature is continuous at every point (and zero at the open
/// ends): Gauss-Seidel sweeps, each point's angle corrected by a secant step on its own residual,
/// damped and bounded so a sharp polyline still converges to something sane.
fn solve(p: &[Point], closed: bool) -> Vec<f64> {
    let n = p.len();
    let segs = if closed { n } else { n - 1 };
    // Start from the chord bisectors, the ends along their chords.
    let chord_angle = |i: usize| angle(p[(i + 1) % n] - p[i]);
    let mut theta: Vec<f64> = (0..n)
        .map(|i| {
            let interior = closed || (i > 0 && i + 1 < n);
            if interior {
                let a = chord_angle((i + segs - 1) % segs);
                let b = chord_angle(i % segs);
                a + 0.5 * wrap(b - a)
            } else if i == 0 {
                chord_angle(0)
            } else {
                chord_angle(segs - 1)
            }
        })
        .collect();
    let base: Vec<f64> = theta.clone();
    let mut sp = spans(p, &theta, closed);
    for _ in 0..SOLVE_ROUNDS {
        let mut worst = 0.0f64;
        for j in 0..n {
            let r0 = residual(&sp, j, n, closed);
            if r0.abs() < 1e-9 {
                continue;
            }
            // Secant: how the residual moves with this angle.
            let h = 1e-3;
            let saved = theta[j];
            theta[j] = saved + h;
            refit_around(&mut sp, p, &theta, j, closed);
            let r1 = residual(&sp, j, n, closed);
            let slope = (r1 - r0) / h;
            if slope.abs() < 1e-9 {
                theta[j] = saved;
                refit_around(&mut sp, p, &theta, j, closed);
                continue;
            }
            let step = (-r0 / slope).clamp(-0.5, 0.5) * 0.7;
            // A tangent never swings more than 80° from the chord bisector it started at.
            let limit = 80f64.to_radians();
            theta[j] = base[j] + wrap(saved + step - base[j]).clamp(-limit, limit);
            refit_around(&mut sp, p, &theta, j, closed);
            worst = worst.max(r0.abs());
        }
        if worst < 1e-7 {
            break;
        }
    }
    theta
}

/// Signed curvature of a cubic (p0, p1, p2, p3) at its start.
fn cubic_curvature_start(p0: Point, p1: Point, p2: Point) -> f64 {
    let a = p1 - p0;
    let l = a.hypot();
    if l < 1e-9 {
        return 0.0;
    }
    (2.0 / 3.0) * a.cross(p2 - p1) / (l * l * l)
}

/// One cubic for the span from `pa` to `pb` with unit tangents `ta`, `tb` and target curvatures
/// `ka`, `kb` at its ends: the handle lengths are found by fixed-point iteration on the cubic's
/// own end curvatures (a handle's curvature falls with its length squared), starting from the
/// circular-arc lengths.
fn cubic_for(pa: Point, pb: Point, ta: Vec2, tb: Vec2, ka: f64, kb: f64) -> (Point, Point) {
    let d = (pb - pa).hypot().max(1e-9);
    let (mut la, mut lb) = (d / 3.0, d / 3.0);
    for _ in 0..HANDLE_ROUNDS {
        let p1 = pa + ta * la;
        let p2 = pb - tb * lb;
        let ca = cubic_curvature_start(pa, p1, p2);
        // Reversed, the curve runs the other way: the end curvature changes sign.
        let cb = -cubic_curvature_start(pb, p2, p1);
        let adjust = |l: f64, have: f64, want: f64| -> f64 {
            if want.abs() < 1e-9 {
                // A straight end (a free end of the spline): a cubic only gets there with a long
                // handle, since its end curvature falls with the handle's length squared.
                (l * 1.3).clamp(d * 0.05, d * 0.75)
            } else if have.abs() < 1e-12 || (have > 0.0) != (want > 0.0) {
                // The wrong way round: the handle settles at a third of the chord.
                l * 0.5 + (d / 3.0) * 0.5
            } else {
                (l * (have / want).abs().sqrt()).clamp(d * 0.05, d * 0.9)
            }
        };
        la = adjust(la, ca, ka);
        lb = adjust(lb, cb, kb);
    }
    (pa + ta * la, pb - tb * lb)
}

/// The clothoid spline through `pts` as a subpath of smooth anchors (one cubic per span). Fewer
/// than three points give a line; a closed run of three or more closes smoothly.
pub fn clothoid_to_bezier(pts: &[Point], closed: bool) -> SubPath {
    let p = dedup(pts, closed);
    let n = p.len();
    if n < 3 {
        return SubPath::polyline(&p, false);
    }
    let closed = closed && n > 2;
    let segs = if closed { n } else { n - 1 };
    let theta = solve(&p, closed);
    let sp = spans(&p, &theta, closed);
    let tangent = |i: usize| Vec2::new(theta[i].cos(), theta[i].sin());
    let mut h_out: Vec<Point> = p.clone();
    let mut h_in: Vec<Point> = p.clone();
    for i in 0..segs {
        let (a, b) = (i, (i + 1) % n);
        let (span, _, d) = sp[i];
        let (ka, kb) = span.curvatures(d);
        let (c1, c2) = cubic_for(p[a], p[b], tangent(a), tangent(b), ka, kb);
        h_out[a] = c1;
        h_in[b] = c2;
    }
    let anchors = (0..n)
        .map(|i| {
            let interior = closed || (i > 0 && i + 1 < n);
            let kind = if interior { AnchorKind::Smooth } else { AnchorKind::Corner };
            Anchor { p: p[i], h_in: h_in[i], h_out: h_out[i], kind }
        })
        .collect();
    SubPath::new(anchors, closed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::{CubicBez, ParamCurve, ParamCurveCurvature};

    fn pts(v: &[(f64, f64)]) -> Vec<Point> {
        v.iter().map(|&(x, y)| Point::new(x, y)).collect()
    }

    #[test]
    fn a_fitted_span_lands_on_its_chord() {
        for (a0, a1) in [(0.3, -0.3), (0.5, 0.5), (-0.8, 0.2), (0.0, 0.0)] {
            let s = fit_span(a0, a1);
            let e = end_point(s.a0, s.b, s.c);
            assert!(e.y.abs() < 1e-6, "{a0} {a1}: {e:?}");
            assert!((e.x * s.len - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn passes_through_points_with_continuous_curvature() {
        let p = pts(&[(0.0, 0.0), (100.0, 60.0), (200.0, 0.0), (300.0, 80.0), (400.0, 0.0)]);
        let sp = clothoid_to_bezier(&p, false);
        assert_eq!(sp.anchors.len(), 5);
        for (a, q) in sp.anchors.iter().zip(&p) {
            assert_eq!(a.p, *q);
        }
        // Curvature either side of each inner anchor agrees closely.
        for i in 1..4 {
            let (a, b, c) = (&sp.anchors[i - 1], &sp.anchors[i], &sp.anchors[i + 1]);
            let before = CubicBez::new(a.p, a.h_out, b.h_in, b.p);
            let after = CubicBez::new(b.p, b.h_out, c.h_in, c.p);
            let (k_in, k_out) = (before.curvature(1.0), after.curvature(0.0));
            assert!((k_in - k_out).abs() < 0.15 * k_in.abs().max(k_out.abs()).max(1e-3), "{i}: {k_in} vs {k_out}");
            // Smooth: handles in line.
            let (vi, vo) = (b.p - b.h_in, b.h_out - b.p);
            assert!(vi.cross(vo).abs() < 1e-6 * vi.hypot() * vo.hypot() + 1e-9 && vi.dot(vo) > 0.0);
        }
        // Free ends: much straighter than the span's middle.
        let first = CubicBez::new(sp.anchors[0].p, sp.anchors[0].h_out, sp.anchors[1].h_in, sp.anchors[1].p);
        assert!(first.curvature(0.0).abs() < 0.5 * first.curvature(0.5).abs(), "{} vs {}", first.curvature(0.0), first.curvature(0.5));
    }

    #[test]
    fn a_closed_square_is_nearly_a_circle() {
        let p = pts(&[(100.0, 0.0), (0.0, 100.0), (-100.0, 0.0), (0.0, -100.0)]);
        let sp = clothoid_to_bezier(&p, true);
        assert!(sp.closed);
        for i in 0..4 {
            let (a, b) = (&sp.anchors[i], &sp.anchors[(i + 1) % 4]);
            let c = CubicBez::new(a.p, a.h_out, b.h_in, b.p);
            for k in 0..=8 {
                let r = c.eval(f64::from(k) / 8.0).to_vec2().hypot();
                assert!((r - 100.0).abs() < 1.5, "{r}");
            }
        }
    }
}
