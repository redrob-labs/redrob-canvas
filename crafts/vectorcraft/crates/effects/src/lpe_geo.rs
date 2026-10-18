//! Geometric-construction live path effects (Effect › Path Effects): the path's anchors are the
//! construction's given points and the effect draws the figure through them, live. Circles and
//! ellipses are drawn as the affine image of a unit circle, so every variant shares one arc
//! builder; the lines take start and end lengths past their defining points.

use redrob_vector_geom::shapes::{self, ArcKind};
use redrob_vector_geom::{Affine, PathData, Point, Rect, SubPath, Vec2};
use serde_json::Value;

use crate::lpe_util::*;
use crate::util::{flag, num, text};

pub const ELLIPSE_5: &str = "path.ellipseByFivePoints";
pub const ELLIPSE_FROM_POINTS: &str = "path.ellipseFromPoints";
pub const CIRCLE_3: &str = "path.circleByThreePoints";
pub const CIRCLE_CENTER_RADIUS: &str = "path.circleByCenterAndRadius";
pub const LINE_SEGMENT: &str = "path.lineSegment";
pub const PARALLEL: &str = "path.parallel";
pub const PERPENDICULAR_BISECTOR: &str = "path.perpendicularBisector";
pub const ANGLE_BISECTOR: &str = "path.angleBisector";
pub const TANGENT_TO_CURVE: &str = "path.tangentToCurve";
pub const CONSTRUCT_GRID: &str = "path.constructGrid";
pub const TRANSFORM_2: &str = "path.transformByTwoPoints";

/// The ways Ellipse from Points builds its ellipse: (label, value).
pub const ELLIPSE_METHODS: [(&str, &str); 6] = [
    ("Auto", "auto"),
    ("Force Circle", "circle"),
    ("Isometric Circle", "isometric"),
    ("Steiner Ellipse", "steiner"),
    ("Steiner Inellipse", "steinerIn"),
    ("Five Points", "fivePoints"),
];

/// The kinds of line Line Segment draws: (label, value).
pub const LINE_KINDS: [(&str, &str); 4] =
    [("Closed", "closed"), ("Open at Start", "openStart"), ("Open at End", "openEnd"), ("Infinite", "infinite")];

/// The unit circle (radius 1 about the origin) as a rect.
const UNIT: Rect = Rect::new(-1.0, -1.0, 1.0, 1.0);

/// How far an "infinite" line reaches past its points (document points).
const FAR: f64 = 1.0e4;

/// The circle through `a`, `b` and `c` as (centre, radius); `None` when they are collinear.
fn circumcircle(a: Point, b: Point, c: Point) -> Option<(Point, f64)> {
    let d = 2.0 * (a.x * (b.y - c.y) + b.x * (c.y - a.y) + c.x * (a.y - b.y));
    if d.abs() <= 1e-12 || !d.is_finite() {
        return None;
    }
    let (a2, b2, c2) = (a.to_vec2().hypot2(), b.to_vec2().hypot2(), c.to_vec2().hypot2());
    let ux = (a2 * (b.y - c.y) + b2 * (c.y - a.y) + c2 * (a.y - b.y)) / d;
    let uy = (a2 * (c.x - b.x) + b2 * (a.x - c.x) + c2 * (b.x - a.x)) / d;
    let centre = Point::new(ux, uy);
    Some((centre, centre.distance(a)))
}

/// The map taking the unit circle to the circle of `radius` about `c`.
fn circle_map(c: Point, radius: f64) -> Affine {
    Affine::translate(c.to_vec2()) * Affine::scale(radius.max(1e-9))
}

/// The ellipse through five points as the map of the unit circle onto it: the conic
/// `Ax² + Bxy + Cy² + Dx + Ey + F = 0` through them (the null vector of the 5 × 6 system), read
/// as centre, axes and tilt. `None` when the conic isn't an ellipse.
fn conic_through(pts: &[Point]) -> Option<Affine> {
    if pts.len() < 5 {
        return None;
    }
    // Scale the points to about unit size first: the conic terms span x⁴ otherwise.
    let c = pts.iter().fold(Vec2::ZERO, |acc, p| acc + p.to_vec2()) / pts.len() as f64;
    let scale = pts.iter().map(|p| (p.to_vec2() - c).hypot()).fold(0.0, f64::max).max(1e-9);
    let q: Vec<Point> = pts.iter().take(5).map(|p| ((p.to_vec2() - c) / scale).to_point()).collect();
    let mut m: Vec<[f64; 6]> = q.iter().map(|p| [p.x * p.x, p.x * p.y, p.y * p.y, p.x, p.y, 1.0]).collect();
    // Gaussian elimination to row echelon form; the free column gives the null vector.
    let mut pivot_cols = Vec::new();
    let mut row = 0;
    for col in 0..6 {
        if row >= 5 {
            break;
        }
        let best = (row..5).max_by(|&i, &j| m[i][col].abs().total_cmp(&m[j][col].abs()))?;
        if m[best][col].abs() <= 1e-12 {
            continue;
        }
        m.swap(row, best);
        let pv = m[row][col];
        for v in m[row].iter_mut() {
            *v /= pv;
        }
        let pivot_row = m[row];
        for (r, other) in m.iter_mut().enumerate() {
            let f = other[col];
            if r != row && f != 0.0 {
                for (x, y) in other.iter_mut().zip(pivot_row.iter()) {
                    *x -= f * y;
                }
            }
        }
        pivot_cols.push(col);
        row += 1;
    }
    let free = (0..6).find(|col| !pivot_cols.contains(col))?;
    let mut coef = [0.0; 6];
    coef[free] = 1.0;
    for (r, &pc) in pivot_cols.iter().enumerate() {
        coef[pc] = -m[r][free];
    }
    let [a, b, cc, d, e, f] = coef;
    let disc = b * b - 4.0 * a * cc;
    if disc >= -1e-12 || !disc.is_finite() {
        return None;
    }
    let x0 = (2.0 * cc * d - b * e) / disc;
    let y0 = (2.0 * a * e - b * d) / disc;
    let common = 2.0 * (a * e * e + cc * d * d - b * d * e + disc * f);
    let root = ((a - cc) * (a - cc) + b * b).sqrt();
    let axis = |sign: f64| -> Option<f64> {
        let v = common * ((a + cc) + sign * root);
        let r = -(v.sqrt()) / disc;
        (r.is_finite() && r > 0.0).then_some(r)
    };
    let (ra, rb) = (axis(1.0)?, axis(-1.0)?);
    // The angle of the `ra` axis (the `+` root), robust when `b` is a rounding residue.
    let theta = (cc - a - root).atan2(b);
    let centre = c + Vec2::new(x0, y0) * scale;
    Some(Affine::translate(centre) * Affine::rotate(theta) * Affine::scale_non_uniform(ra * scale, rb * scale))
}

/// The least-squares circle through `pts` (Kåsa fit); the diameter circle of two points.
fn fit_circle(pts: &[Point]) -> Option<Affine> {
    match pts.len() {
        0 | 1 => None,
        2 => {
            let c = pts[0].midpoint(pts[1]);
            Some(circle_map(c, c.distance(pts[0])))
        }
        3 => circumcircle(pts[0], pts[1], pts[2]).map(|(c, r)| circle_map(c, r)),
        _ => {
            // Minimise Σ (x² + y² + D x + E y + F)²: a 3 × 3 normal system.
            let (mut sxx, mut sxy, mut syy, mut sx, mut sy, mut sxz, mut syz, mut sz) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
            let n = pts.len() as f64;
            for p in pts {
                let z = p.x * p.x + p.y * p.y;
                sxx += p.x * p.x;
                sxy += p.x * p.y;
                syy += p.y * p.y;
                sx += p.x;
                sy += p.y;
                sxz += p.x * z;
                syz += p.y * z;
                sz += z;
            }
            let m = [[sxx, sxy, sx], [sxy, syy, sy], [sx, sy, n]];
            let rhs = [-sxz, -syz, -sz];
            let det3 = |m: &[[f64; 3]; 3]| {
                m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
                    + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
            };
            let det = det3(&m);
            if det.abs() <= 1e-12 {
                return None;
            }
            let solve = |col: usize| {
                let mut mm = m;
                for (row, v) in mm.iter_mut().zip(rhs) {
                    row[col] = v;
                }
                det3(&mm) / det
            };
            let (d, e, f) = (solve(0), solve(1), solve(2));
            let c = Point::new(-d / 2.0, -e / 2.0);
            let r2 = c.to_vec2().hypot2() - f;
            (r2 > 0.0).then(|| circle_map(c, r2.sqrt()))
        }
    }
}

/// The ellipse inscribed in the parallelogram `p0, p1, p2, p0 + p2 − p1`: the unit circle mapped
/// by the map taking the unit square's corners onto it.
fn parallelogram_ellipse(p0: Point, p1: Point, p2: Point) -> Option<Affine> {
    affine_3([Point::new(-1.0, -1.0), Point::new(1.0, -1.0), Point::new(1.0, 1.0)], [p0, p1, p2])
}

/// The Steiner circumellipse of triangle `p0 p1 p2` (through its vertices, centred on its
/// centroid), or with `inner` the Steiner inellipse (touching the sides at their midpoints): the
/// circumcircle (incircle) of an equilateral triangle mapped onto this one.
fn steiner(p0: Point, p1: Point, p2: Point, inner: bool) -> Option<Affine> {
    let eq: [Point; 3] = [0, 1, 2].map(|k| {
        let a = std::f64::consts::TAU * k as f64 / 3.0 - std::f64::consts::FRAC_PI_2;
        Point::new(a.cos(), a.sin())
    });
    let m = affine_3(eq, [p0, p1, p2])?;
    Some(if inner { m * Affine::scale(0.5) } else { m })
}

/// The angle (degrees, counterclockwise on screen from 3 o'clock) at which the unit circle mapped
/// by `m` passes `p`.
fn angle_on(m: Affine, p: Point) -> f64 {
    let q = m.inverse() * p;
    (-q.y).atan2(q.x).to_degrees().rem_euclid(360.0)
}

/// The ellipse `m` maps the unit circle to: whole, or cut from the first given point to the last
/// (through the ones between) as an arc, a chord or a slice.
fn ellipse_art(m: Affine, pts: &[Point], arc: bool, kind: ArcKind) -> PathData {
    if !arc || pts.len() < 2 {
        return shapes::ellipse(UNIT).transformed(m);
    }
    let (first, last) = (pts[0], pts[pts.len() - 1]);
    let (mut start, mut end) = (angle_on(m, first), angle_on(m, last));
    // The arc runs counterclockwise from start to end; it goes the way that passes a middle point.
    if pts.len() > 2 {
        let mid = angle_on(m, pts[pts.len() / 2]);
        let inside = (mid - start).rem_euclid(360.0) <= (end - start).rem_euclid(360.0);
        if !inside {
            std::mem::swap(&mut start, &mut end);
        }
    }
    shapes::ellipse_arc(UNIT, start, end, kind).transformed(m)
}

/// The arc options of the ellipse effects: `arc` cuts the ellipse between the first and last
/// point, `slice` closes the cut through the centre (a chord otherwise, when `closeArc` is set).
fn arc_options(p: &Value) -> (bool, ArcKind) {
    let arc = flag(p, "arc", false);
    let kind = if flag(p, "slice", false) {
        ArcKind::Pie
    } else if flag(p, "closeArc", false) {
        ArcKind::Chord
    } else {
        ArcKind::Arc
    };
    (arc, kind)
}

/// The axes (two diameters) and the frame (the bounding parallelogram) of the ellipse `m` maps
/// the unit circle to, as open subpaths.
fn axes_and_frame(m: Affine, axes: bool, frame: bool) -> Vec<SubPath> {
    let mut out = Vec::new();
    if axes {
        out.push(line(m * Point::new(-1.0, 0.0), m * Point::new(1.0, 0.0)));
        out.push(line(m * Point::new(0.0, -1.0), m * Point::new(0.0, 1.0)));
    }
    if frame {
        let corners: Vec<Point> = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].iter().map(|&(x, y)| m * Point::new(x, y)).collect();
        out.push(SubPath::polyline(&corners, true));
    }
    out
}

/// Ellipse by 5 Points: the ellipse through the first five anchors (the path as it is with fewer,
/// or when no ellipse passes through them).
pub fn ellipse_by_five_points(path: &PathData, p: &Value) -> PathData {
    let pts = points(path);
    let Some(m) = conic_through(&pts) else { return path.clone() };
    let (arc, kind) = arc_options(p);
    let mut out = ellipse_art(m, &pts[..5], arc, kind);
    out.subpaths.extend(axes_and_frame(m, flag(p, "drawAxes", false), flag(p, "drawFrame", false)));
    out
}

/// Ellipse from Points: a circle or ellipse from the path's 2 to 5 anchors by `method` (see
/// [`ELLIPSE_METHODS`]); `auto` picks by the number of anchors: two give a circle on their
/// diameter, three the circle through them, four the ellipse inscribed in their parallelogram
/// and five the ellipse through them.
pub fn ellipse_from_points(path: &PathData, p: &Value) -> PathData {
    let pts = points(path);
    let n = pts.len();
    if n < 2 {
        return path.clone();
    }
    let method = text(p, "method", "auto").to_ascii_lowercase();
    let m = match method.as_str() {
        "circle" => fit_circle(&pts),
        "isometric" if n >= 3 => parallelogram_ellipse(pts[0], pts[1], pts[2]),
        "steiner" if n >= 3 => steiner(pts[0], pts[1], pts[2], false),
        "steinerin" if n >= 3 => steiner(pts[0], pts[1], pts[2], true),
        "fivepoints" if n >= 5 => conic_through(&pts),
        "auto" => match n {
            2 | 3 => fit_circle(&pts),
            4 => parallelogram_ellipse(pts[0], pts[1], pts[2]),
            _ => conic_through(&pts).or_else(|| fit_circle(&pts)),
        },
        _ => fit_circle(&pts),
    };
    let Some(m) = m else { return path.clone() };
    let (arc, kind) = arc_options(p);
    let mut out = ellipse_art(m, &pts[..n.min(5)], arc, kind);
    out.subpaths.extend(axes_and_frame(m, flag(p, "drawAxes", false), flag(p, "drawFrame", false)));
    out
}

/// Circle by 3 Points: the circle through the first three anchors.
pub fn circle_by_three_points(path: &PathData, p: &Value) -> PathData {
    let pts = points(path);
    if pts.len() < 3 {
        return path.clone();
    }
    let Some((c, r)) = circumcircle(pts[0], pts[1], pts[2]) else { return path.clone() };
    let (arc, kind) = arc_options(p);
    ellipse_art(circle_map(c, r), &pts[..3], arc, kind)
}

/// Circle by Centre and Radius: the first anchor is the centre, the second lies on the circle.
pub fn circle_by_center_and_radius(path: &PathData, _p: &Value) -> PathData {
    let pts = points(path);
    if pts.len() < 2 {
        return path.clone();
    }
    let r = pts[0].distance(pts[1]);
    if r <= 1e-9 {
        return path.clone();
    }
    shapes::ellipse(UNIT).transformed(circle_map(pts[0], r))
}

/// A straight line from `a` towards `b`, reaching `before` past `a` (backwards) and `after`
/// past `b`.
fn extended(a: Point, b: Point, before: f64, after: f64) -> Option<SubPath> {
    let d = unit(b - a)?;
    Some(line(a - d * before, b + d * after))
}

/// Line Segment: the straight line through the first two anchors; closed between them, open
/// (reaching far) at the start, the end, or both (infinite).
pub fn line_segment(path: &PathData, p: &Value) -> PathData {
    let pts = points(path);
    if pts.len() < 2 {
        return path.clone();
    }
    let kind = text(p, "type", "closed").to_ascii_lowercase();
    let (before, after) = match kind.as_str() {
        "openstart" => (FAR, 0.0),
        "openend" => (0.0, FAR),
        "infinite" | "openboth" => (FAR, FAR),
        _ => (0.0, 0.0),
    };
    extended(pts[0], pts[1], before, after).map_or_else(|| path.clone(), PathData::single)
}

/// Parallel: the line parallel to the segment between the first two anchors, `distance` to its
/// left (negative: right), or through the third anchor when there is one and `throughThird` is
/// set; `startLength` and `endLength` past the projected ends.
pub fn parallel(path: &PathData, p: &Value) -> PathData {
    let pts = points(path);
    if pts.len() < 2 {
        return path.clone();
    }
    let (a, b) = (pts[0], pts[1]);
    let Some(n) = left(b - a) else { return path.clone() };
    let shift = if flag(p, "throughThird", true) && pts.len() >= 3 { (pts[2] - a).dot(n) } else { num(p, "distance", 20.0) };
    let (a2, b2) = (a + n * shift, b + n * shift);
    let (s, e) = (num(p, "startLength", 0.0).max(-FAR), num(p, "endLength", 0.0).max(-FAR));
    extended(a2, b2, s, e).map_or_else(|| path.clone(), PathData::single)
}

/// Perpendicular Bisector: the line through the midpoint of the first two anchors at a right
/// angle to them, `length` long (centred on the midpoint).
pub fn perpendicular_bisector(path: &PathData, p: &Value) -> PathData {
    let pts = points(path);
    if pts.len() < 2 {
        return path.clone();
    }
    let (a, b) = (pts[0], pts[1]);
    let Some(n) = left(b - a) else { return path.clone() };
    let mid = a.midpoint(b);
    let half = num(p, "length", 100.0).clamp(0.0, FAR) / 2.0;
    PathData::single(line(mid - n * half, mid + n * half))
}

/// Angle Bisector: from a three-anchor path, the line bisecting the angle at the middle anchor,
/// `startLength` behind it and `endLength` ahead (into the angle).
pub fn angle_bisector(path: &PathData, p: &Value) -> PathData {
    let pts = points(path);
    if pts.len() < 3 {
        return path.clone();
    }
    let (a, b, c) = (pts[0], pts[1], pts[2]);
    let (Some(u), Some(v)) = (unit(a - b), unit(c - b)) else { return path.clone() };
    let bis = unit(u + v).or_else(|| left(u)).unwrap_or(Vec2::new(1.0, 0.0));
    let (s, e) = (num(p, "startLength", 0.0).clamp(0.0, FAR), num(p, "endLength", 100.0).clamp(0.0, FAR));
    PathData::single(line(b - bis * s, b + bis * e))
}

/// Tangent to Curve: the tangent line at `position` (0..1 of the first subpath's length), turned
/// by `angle` degrees, reaching `startLength` back and `endLength` forward from the point.
pub fn tangent_to_curve(path: &PathData, p: &Value) -> PathData {
    let Some(sp) = path.subpaths.iter().find(|s| s.anchors.len() >= 2) else { return path.clone() };
    let m = Measured::new(sp);
    if m.is_empty() {
        return path.clone();
    }
    let at = num(p, "position", 0.5).clamp(0.0, 1.0) * m.total;
    let (q, mut t) = m.at(at);
    let a = num(p, "angle", 0.0).to_radians();
    if a != 0.0 {
        t = Vec2::new(t.x * a.cos() - t.y * a.sin(), t.x * a.sin() + t.y * a.cos());
    }
    let (s, e) = (num(p, "startLength", 100.0).clamp(0.0, FAR), num(p, "endLength", 100.0).clamp(0.0, FAR));
    let mut out = PathData::single(line(q - t * s, q + t * e));
    if flag(p, "keepPath", false) {
        out.subpaths.extend(path.subpaths.iter().cloned());
    }
    out
}

/// Construct Grid: from the path's first three anchors (an origin and the ends of its two axes),
/// a grid of `rows` × `columns` cells along those (possibly skewed) axes, as open lines.
pub fn construct_grid(path: &PathData, p: &Value) -> PathData {
    let pts = points(path);
    if pts.len() < 3 {
        return path.clone();
    }
    let (o, a, b) = (pts[0], pts[1], pts[2]);
    let (u, v) = (a - o, b - o);
    let rows = num(p, "rows", 5.0).clamp(1.0, 500.0).round() as usize;
    let cols = num(p, "columns", 5.0).clamp(1.0, 500.0).round() as usize;
    let mut out = Vec::with_capacity(rows + cols + 2);
    for r in 0..=rows {
        let f = r as f64 / rows as f64;
        out.push(line(o + v * f, o + u + v * f));
    }
    for c in 0..=cols {
        let f = c as f64 / cols as f64;
        out.push(line(o + u * f, o + v + u * f));
    }
    PathData::new(out)
}

/// Transform by 2 Points: the two knots `from` (box coordinates of the object's bounds `b`) are
/// moved to `to`; the path goes with them: translated, turned and scaled so the first knot lands
/// on the first target and the second on the second. `elastic` stretches only along the knots'
/// axis instead of scaling uniformly; `lockLength` keeps the distance between the knots (turn
/// and move only) and `lockAngle` keeps their direction (scale and move only).
pub fn transform_by_two_points(path: &PathData, b: Rect, p: &Value) -> PathData {
    let from = point_list(p, "from");
    let to = point_list(p, "to");
    if from.len() < 2 || to.len() < 2 {
        return path.clone();
    }
    let (a0, a1) = (from_box(b, from[0]), from_box(b, from[1]));
    let (b0, mut b1) = (from_box(b, to[0]), from_box(b, to[1]));
    let (u, v) = (a1 - a0, b1 - b0);
    if u.hypot() <= 1e-9 {
        return path.clone();
    }
    if flag(p, "lockLength", false)
        && let Some(d) = unit(v)
    {
        b1 = b0 + d * u.hypot();
    }
    if flag(p, "lockAngle", false)
        && let Some(d) = unit(u)
    {
        b1 = b0 + d * (b1 - b0).dot(d);
    }
    let xf = if flag(p, "elastic", false) {
        // Stretch along the knots' axis only: the across direction keeps its size.
        let (Some(du), Some(dv)) = (unit(u), unit(b1 - b0)) else { return path.clone() };
        let s = (b1 - b0).hypot() / u.hypot();
        // Turn the knots' axis to x, stretch along x, turn to the targets' axis.
        let to_x = Affine::new([du.x, -du.y, du.y, du.x, 0.0, 0.0]);
        let from_x = Affine::new([dv.x, dv.y, -dv.y, dv.x, 0.0, 0.0]);
        Affine::translate(b0.to_vec2()) * from_x * Affine::scale_non_uniform(s, 1.0) * to_x * Affine::translate(-a0.to_vec2())
    } else {
        similarity(a0, a1, b0, b1)
    };
    path.transformed(xf)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn poly(pts: &[(f64, f64)]) -> PathData {
        PathData::single(SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), false))
    }

    #[test]
    fn circle_through_three_points_passes_them() {
        let out = circle_by_three_points(&poly(&[(0.0, 0.0), (10.0, 0.0), (0.0, 10.0)]), &json!({}));
        let b = out.bounds().unwrap();
        assert!((b.center().x - 5.0).abs() < 1e-6 && (b.center().y - 5.0).abs() < 1e-6, "{b:?}");
        assert!((b.width() - 10.0 * 2f64.sqrt()).abs() < 1e-3);
    }

    #[test]
    fn five_points_on_an_ellipse_give_it_back() {
        let pts: Vec<(f64, f64)> = [0.0, 1.0, 2.2, 3.5, 4.7].iter().map(|t: &f64| (50.0 + 30.0 * t.cos(), 20.0 + 10.0 * t.sin())).collect();
        let out = ellipse_by_five_points(&poly(&pts), &json!({}));
        let b = out.bounds().unwrap();
        assert!((b.width() - 60.0).abs() < 1e-3 && (b.height() - 20.0).abs() < 1e-3, "{b:?}");
        assert!((b.center().x - 50.0).abs() < 1e-3 && (b.center().y - 20.0).abs() < 1e-3);
    }

    #[test]
    fn construction_lines_have_the_right_direction() {
        let bis = angle_bisector(&poly(&[(10.0, 0.0), (0.0, 0.0), (0.0, 10.0)]), &json!({"startLength": 0, "endLength": 10}));
        let end = bis.subpaths[0].anchors[1].p;
        assert!((end.x - end.y).abs() < 1e-9 && end.x > 0.0, "{end:?}");
        let perp = perpendicular_bisector(&poly(&[(0.0, 0.0), (10.0, 0.0)]), &json!({"length": 20}));
        let a = &perp.subpaths[0].anchors;
        assert!((a[0].p.x - 5.0).abs() < 1e-9 && (a[1].p.x - 5.0).abs() < 1e-9 && ((a[0].p.y - a[1].p.y).abs() - 20.0).abs() < 1e-9);
        let grid = construct_grid(&poly(&[(0.0, 0.0), (10.0, 0.0), (0.0, 10.0)]), &json!({"rows": 2, "columns": 3}));
        assert_eq!(grid.subpaths.len(), 3 + 4);
        let par = parallel(&poly(&[(0.0, 0.0), (10.0, 0.0)]), &json!({"distance": 5, "throughThird": false}));
        assert!((par.subpaths[0].anchors[0].p.y + 5.0).abs() < 1e-9);
    }

    #[test]
    fn two_point_transform_moves_the_knots() {
        let sq = poly(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
        let b = Rect::new(0.0, 0.0, 100.0, 100.0);
        let out = transform_by_two_points(&sq, b, &json!({"from": "0,0.5 1,0.5", "to": "0,0.5 2,0.5"}));
        let bb = out.bounds().unwrap();
        assert!((bb.width() - 200.0).abs() < 1e-6 && (bb.height() - 200.0).abs() < 1e-6, "{bb:?}");
        let el = transform_by_two_points(&sq, b, &json!({"from": "0,0.5 1,0.5", "to": "0,0.5 2,0.5", "elastic": true}));
        let eb = el.bounds().unwrap();
        assert!((eb.width() - 200.0).abs() < 1e-6 && (eb.height() - 100.0).abs() < 1e-6, "{eb:?}");
    }
}
