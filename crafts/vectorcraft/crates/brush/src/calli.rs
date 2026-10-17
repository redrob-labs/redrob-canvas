//! Calligraphic and bristle brushes: an elliptical nib swept along the path.
//!
//! The swept area of an ellipse moving along a curve is bounded by the ellipse's support points
//! in the directions normal to the path. For nib axes `u`, `v` with semi-axes `a`, `b` the
//! support point in direction `d` is `(a²(u·d)u + b²(v·d)v) / √(a²(u·d)² + b²(v·d)²)`, so the
//! stroke's local width across direction `n` is `2·√(a²(u·n)² + b²(v·n)²)`.
//!
//! A Calligraphic brush with Pressure variation changes its nib along the stroke with the pen
//! pressure recorded on it (`StrokeLayer::pressure`): the sweep is sampled densely and each
//! sample takes the nib for the pressure there.

use redrob_vector_color::Paint;
use redrob_vector_doc::{Node, PressureProfile, StrokeLayer};
use redrob_vector_geom::{BezPath, PathData, Point, SubPath, Vec2};

use crate::track::{Rng, Track, normal, seed_of, tracks};
use crate::{Bristle, BristleShape, Calligraphic, Variation, filled, tolerance};

/// The most pieces a varying nib cuts one segment of a subpath into, and about how many samples
/// it takes along a whole subpath.
const MAX_STEPS: f64 = 1024.0;

/// An elliptical nib.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Nib {
    u: Vec2,
    v: Vec2,
    a: f64,
    b: f64,
}

impl Nib {
    /// `angle` in degrees counter-clockwise on the page (y-down document space), `size` the
    /// major diameter, `roundness` the minor/major ratio (0..1].
    pub fn new(angle_deg: f64, roundness: f64, size: f64) -> Self {
        let t = angle_deg.to_radians();
        let u = Vec2::new(t.cos(), -t.sin());
        let v = Vec2::new(-u.y, u.x);
        let a = (size / 2.0).max(1e-4);
        Nib { u, v, a, b: (a * roundness.clamp(0.01, 1.0)).max(1e-4) }
    }
    pub fn support(&self, d: Vec2) -> Vec2 {
        let (du, dv) = (self.u.dot(d), self.v.dot(d));
        let den = ((self.a * du).powi(2) + (self.b * dv).powi(2)).sqrt().max(1e-12);
        (self.u * (self.a * self.a * du) + self.v * (self.b * self.b * dv)) / den
    }
}

/// Unit vector at `angle` radians from `base` towards its left normal.
fn rot(base: Vec2, angle: f64) -> Vec2 {
    base * angle.cos() + normal(base) * angle.sin()
}

/// Samples (point, tangent, fraction of the track's length) along a track, with extra tangents
/// fanned in at corners so the outer edge of the sweep follows the nib around the corner. `step`
/// (for a nib that varies along the track): segments are cut into pieces at most that long.
fn samples(t: &Track, step: Option<f64>) -> Vec<(Point, Vec2, f64)> {
    let mut out = vec![];
    let n = t.segments();
    let len = t.len().max(1e-12);
    for i in 0..=n {
        let p = t.verts[i];
        let f = t.cum[i] / len;
        if t.corner[i] {
            let (a, b) = t.in_out(i);
            let turn = a.cross(b).atan2(a.dot(b));
            let steps = (turn.abs() / 10f64.to_radians()).ceil().max(1.0) as usize;
            for k in 0..=steps {
                out.push((p, rot(a, turn * k as f64 / steps as f64), f));
            }
        } else {
            out.push((p, t.tangents[i], f));
        }
        if let Some(step) = step
            && i < n
        {
            let seg = t.cum[i + 1] - t.cum[i];
            let pieces = (seg / step).ceil().clamp(1.0, MAX_STEPS) as usize;
            for k in 1..pieces {
                let u = k as f64 / pieces as f64;
                out.push((p.lerp(t.verts[i + 1], u), t.dirs[i], (t.cum[i] + seg * u) / len));
            }
        }
    }
    out
}

/// The swept outline along `t` of the nib `nib(f)` (`f`: the fraction of the track's length) as
/// closed subpaths. `step`: how often a varying nib is sampled (None: a constant nib).
pub(crate) fn sweep(t: &Track, nib: &dyn Fn(f64) -> Nib, step: Option<f64>) -> Vec<SubPath> {
    let s = samples(t, step);
    let left: Vec<Point> = s.iter().map(|(p, tg, f)| *p + nib(*f).support(normal(*tg))).collect();
    let right: Vec<Point> = s.iter().map(|(p, tg, f)| *p + nib(*f).support(-normal(*tg))).collect();
    if t.closed {
        let mut r = right;
        r.reverse();
        return vec![SubPath::polyline(&left, true), SubPath::polyline(&r, true)];
    }
    let cap = |(p, tg, f): (Point, Vec2, f64), from: f64, to: f64, out: &mut Vec<Point>| {
        let (steps, nib) = (16, nib(f));
        for k in 1..steps {
            let ang = from + (to - from) * k as f64 / steps as f64;
            out.push(p + nib.support(rot(tg, ang)));
        }
    };
    let half = std::f64::consts::FRAC_PI_2;
    let mut poly = left.clone();
    let (Some(&end), Some(&start)) = (s.last(), s.first()) else { return Vec::new() };
    cap(end, half, -half, &mut poly);
    poly.extend(right.iter().rev());
    cap(start, -half, -3.0 * half, &mut poly);
    vec![SubPath::polyline(&poly, true)]
}

/// The stroke's angle, roundness and size: Random ones drawn for this stroke (from `rng`), the
/// others the brush's values.
fn randomized(c: &Calligraphic, rng: &mut Rng) -> [f64; 3] {
    let mut out = [c.angle, c.roundness, c.size];
    for ((v, var), mode) in out.iter_mut().zip(c.variation).zip(c.modes()) {
        // Every value draws, so a stroke's random values stay the same whatever the other modes.
        let r = rng.range(-var, var);
        if mode == Variation::Random {
            *v += r;
        }
    }
    let [angle, round, size] = out;
    [angle, round.clamp(1.0, 100.0), size.max(0.05)]
}

/// The nib's angle, roundness and size at pen pressure `p` (0..1): each Pressure value goes from
/// the brush's value less its variation (lightest) to the value plus the variation (heaviest),
/// kept within its range; the others are `base`'s.
fn pressured(c: &Calligraphic, base: [f64; 3], p: f64) -> [f64; 3] {
    let values = [c.angle, c.roundness, c.size];
    let mut out = base;
    for (i, (v, mode)) in out.iter_mut().zip(c.modes()).enumerate() {
        let (Some(&value), Some(&var), Some(&(lo, hi))) = (values.get(i), c.variation.get(i), Calligraphic::RANGES.get(i)) else { continue };
        if mode == Variation::Pressure {
            let x = value + var * (2.0 * p.clamp(0.0, 1.0) - 1.0);
            *v = if x.is_finite() { x.clamp(lo, hi) } else { value.clamp(lo, hi) };
        }
    }
    let [angle, round, size] = out;
    [angle, round.clamp(1.0, 100.0), size.max(0.05)]
}

/// How fast each part of `bp` was drawn, as a function of the fraction of its length: each
/// segment's length against the mean segment length (1 = average speed), read from how far apart
/// the anchors were put down. `None` for a path of one segment (nothing to compare).
fn speed_profile(bp: &BezPath) -> Option<impl Fn(f64) -> f64> {
    use redrob_vector_geom::kurbo::ParamCurveArclen;
    let lens: Vec<f64> = bp.segments().map(|s| s.arclen(0.1)).collect();
    if lens.len() < 2 {
        return None;
    }
    let total: f64 = lens.iter().sum();
    let mean = total / lens.len() as f64;
    if !(total > 0.0 && mean > 0.0) {
        return None;
    }
    let mut cum = vec![0.0];
    for l in &lens {
        cum.push(cum.last().copied().unwrap_or(0.0) + l);
    }
    Some(move |f: f64| {
        let at = f.clamp(0.0, 1.0) * total;
        let i = cum.iter().rposition(|c| *c <= at).unwrap_or(0).min(lens.len() - 1);
        (lens[i] / mean).clamp(0.1, 10.0)
    })
}

/// Smooth hash noise in −1..1 along a stroke: the value at `f` (0..1) of a wave `cycles` long.
fn smooth_noise(seed: u64, salt: u64, f: f64, cycles: f64) -> f64 {
    let x = f * cycles;
    let (i, u) = (x.floor(), x - x.floor());
    let v = |k: f64| {
        let mut r = Rng::new(seed ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (k as i64 as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9));
        r.range(-1.0, 1.0)
    };
    let s = u * u * (3.0 - 2.0 * u);
    v(i) * (1.0 - s) + v(i + 1.0) * s
}

/// Smooth a polyline's points by averaging neighbours `passes` times (Mass: the pen's inertia).
fn smooth_points(pts: &mut [Point], passes: usize, closed: bool) {
    for _ in 0..passes {
        let src = pts.to_vec();
        let n = src.len();
        if n < 3 {
            return;
        }
        for (i, p) in pts.iter_mut().enumerate() {
            let (prev, next) = if closed {
                (src[(i + n - 1) % n], src[(i + 1) % n])
            } else if i == 0 || i + 1 == n {
                continue;
            } else {
                (src[i - 1], src[i + 1])
            };
            *p = Point::new((prev.x + 2.0 * p.x + next.x) / 4.0, (prev.y + 2.0 * p.y + next.y) / 4.0);
        }
    }
}

/// Wiggle: the track's vertices wander off the path by up to `amp`, slowly along its length.
fn wiggle_track(t: &mut Track, seed: u64, amp: f64) {
    let len = t.len().max(1e-9);
    let n = t.verts.len();
    for (v, c) in t.verts.iter_mut().zip(&t.cum) {
        let f = c / len;
        *v += Vec2::new(smooth_noise(seed, 0x51, f, 6.0), smooth_noise(seed, 0x52, f, 6.0)) * amp;
    }
    if t.closed && n > 1 {
        t.verts[n - 1] = t.verts[0];
    }
}

pub(crate) fn calligraphic(c: &Calligraphic, bp: &BezPath, weight: f64, stroke: &StrokeLayer, name: &str) -> Vec<Node> {
    let mut rng = Rng::new(seed_of(bp, name));
    let base = randomized(c, &mut rng);
    let seed = seed_of(bp, name);
    let speed = if c.thinning != 0.0 { speed_profile(bp) } else { None };
    let thinning = (c.thinning / 100.0).clamp(-1.0, 1.0);
    let tremor = (c.tremor / 100.0).clamp(0.0, 1.0);
    let wiggle = (c.wiggle / 100.0).clamp(0.0, 1.0);
    let wiggled = |mut ts: Vec<Track>| {
        if wiggle > 0.0 {
            for t in &mut ts {
                wiggle_track(t, seed, wiggle * base[2] * weight);
            }
        }
        ts
    };
    let mut subs: Vec<SubPath> = if c.varies_along() {
        let pressure = stroke.pressure.as_ref();
        let nib = |f: f64| {
            let [angle, round, mut size] = pressured(c, base, pressure.map_or(PressureProfile::MID, |p| p.at(f)));
            if let Some(speed) = &speed {
                // Faster thins (positive thinning): twice the mean speed halves the width at 100 %.
                let k = 1.0 - thinning * (speed(f) - 1.0).clamp(-1.0, 1.0) * 0.5;
                size *= k.clamp(0.1, 2.0);
            }
            if tremor > 0.0 {
                size *= (1.0 + tremor * 0.6 * smooth_noise(seed, 0x7e3, f, 40.0)).max(0.05);
            }
            Nib::new(angle, round / 100.0, size * weight)
        };
        // Flattened for the largest nib the pressure gives.
        let size = pressured(c, base, 0.0)[2].max(pressured(c, base, 1.0)[2]) * weight;
        wiggled(tracks(bp, tolerance(size))).iter().flat_map(|t| sweep(t, &nib, Some((t.len() / MAX_STEPS).max(0.25)))).collect()
    } else {
        let [angle, round, size] = base;
        let nib = Nib::new(angle, round / 100.0, size * weight);
        wiggled(tracks(bp, tolerance(size * weight))).iter().flat_map(|t| sweep(t, &|_| nib, None)).collect()
    };
    if subs.is_empty() {
        return vec![];
    }
    if c.mass > 0.0 {
        // Inertia: the edges of the sweep are smoothed, more with more mass.
        let passes = (c.mass.clamp(0.0, 100.0) / 100.0 * 12.0).round() as usize;
        for sp in &mut subs {
            let closed = sp.closed;
            let mut pts: Vec<Point> = sp.anchors.iter().map(|a| a.p).collect();
            smooth_points(&mut pts, passes, closed);
            for (a, p) in sp.anchors.iter_mut().zip(pts) {
                (a.p, a.h_in, a.h_out) = (p, p, p);
            }
        }
    }
    vec![filled(PathData::new(subs), &stroke.paint)]
}

/// Offset a track sideways by `d` (a polyline following it).
fn offset_track(t: &Track, d: f64, trim0: f64, trim1: f64) -> Option<BezPath> {
    let l = t.len();
    let (s0, s1) = (trim0.clamp(0.0, l), (l - trim1).clamp(0.0, l));
    if s1 - s0 < 1e-6 {
        return None;
    }
    let mut bp = BezPath::new();
    let mut first = true;
    let mut push = |p: Point, bp: &mut BezPath| {
        if first {
            bp.move_to(p);
            first = false;
        } else {
            bp.line_to(p);
        }
    };
    push(t.map(s0, d), &mut bp);
    for (i, c) in t.cum.iter().enumerate() {
        if *c > s0 && *c < s1 {
            push(t.verts[i] + normal(t.tangents[i]) * d, &mut bp);
        }
    }
    push(t.map(s1, d), &mut bp);
    if t.closed && trim0 <= 0.0 && trim1 <= 0.0 {
        bp.close_path();
    }
    Some(bp)
}

pub(crate) fn bristle(b: &Bristle, bp: &BezPath, weight: f64, paint: &Paint, name: &str) -> Vec<Node> {
    let mut rng = Rng::new(seed_of(bp, name));
    let size = b.size.max(0.1) * weight;
    let strands = (3.0 + b.density.clamp(1.0, 100.0) / 100.0 * 13.0).round() as usize;
    let thick = (size * b.thickness.clamp(1.0, 100.0) / 100.0 * 0.45).max(0.15);
    let flat = matches!(
        b.shape,
        BristleShape::FlatPoint | BristleShape::FlatBlunt | BristleShape::FlatCurve | BristleShape::FlatAngle | BristleShape::FlatFan
    );
    let wobble = (100.0 - b.stiffness.clamp(1.0, 100.0)) / 100.0;
    let ragged = b.length.clamp(25.0, 300.0) / 100.0;
    let opacity = (b.opacity.clamp(1.0, 100.0) / 100.0) as f32;
    let mut out = vec![];
    for t in tracks(bp, tolerance(size)) {
        let l = t.len();
        for k in 0..strands {
            // Round tips concentrate strands near the centre; flat tips spread them evenly.
            let lane = (k as f64 + 0.5) / strands as f64 * 2.0 - 1.0;
            let off = if flat { lane } else { lane.signum() * lane.abs().powf(1.4) } * size / 2.0 + rng.range(-wobble, wobble) * size * 0.08;
            let edge = 1.0 - lane.abs();
            let trim = |r: &mut Rng| r.unit() * ragged * size * (1.2 - edge) * 0.8;
            let (t0, t1) = (trim(&mut rng), trim(&mut rng));
            let Some(sp) = offset_track(&t, off, t0.min(l * 0.3), t1.min(l * 0.3)) else { continue };
            let w = thick * (0.6 + 0.4 * edge) * rng.range(0.8, 1.2);
            let nib = Nib::new(0.0, 1.0, w);
            let subs: Vec<SubPath> = tracks(&sp, tolerance(w)).iter().flat_map(|tt| sweep(tt, &|_| nib, None)).collect();
            if subs.is_empty() {
                continue;
            }
            let mut n = filled(PathData::new(subs), paint);
            n.opacity = (opacity * rng.range(0.7, 1.0) as f32).clamp(0.0, 1.0);
            out.push(n);
        }
    }
    out
}
