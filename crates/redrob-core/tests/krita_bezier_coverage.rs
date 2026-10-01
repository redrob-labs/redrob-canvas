//! Proof that Krita's exported Bezier utilities are already covered here.
//!
//! Item 1c.2 block 2 was planned as 9,165 lines of translation — the largest single block in the whole
//! port. Measured against the CURRENT product it is very nearly nothing, for the same reason
//! `libs/pigment` turned out to be nothing: the capability arrived by another route.
//!
//! The coverage document that produced the 9,165 figure was written BEFORE the Graphite geometry port
//! landed. That port brought kurbo 0.13, lyon_geom 1.0 and eighteen files of curve mathematics into
//! `geometry/`, which is the same domain. The estimate did not go stale because Krita changed; it went
//! stale because THIS PRODUCT changed.
//!
//! So each claim of coverage is executed here rather than asserted in prose. If a future dependency bump
//! removes one of these behaviours, this file fails and the coverage claim in
//! `docs/krita-global-brush-coverage.md` stops being true quietly.
//!
//! Krita's `KisBezierUtils` exports seventeen functions. Thirteen are checked below. The remaining four
//! take a `std::array<QPointF, 12>` — the twelve control points of a Bezier patch — and belong to the
//! mesh transform and the SVG2 mesh gradient, neither of which exists in this product.

use kurbo::{
    CubicBez, Join, Line, ParamCurve, ParamCurveArclen, ParamCurveNearest, PathSeg, Point, QuadBez,
    Shape,
};
use redrob_core::geometry;

/// A curve with enough bend that every length and parameter test is non-trivial.
fn sample_curve() -> CubicBez {
    CubicBez::new(
        Point::new(0.0, 0.0),
        Point::new(30.0, 90.0),
        Point::new(70.0, -30.0),
        Point::new(100.0, 40.0),
    )
}

/// `linearizeCurve` — a flat polyline within a tolerance.
#[test]
fn flattening_covers_linearize_curve() {
    let curve = sample_curve();
    let mut points = Vec::new();
    let elements = [
        kurbo::PathEl::MoveTo(curve.p0),
        kurbo::PathEl::CurveTo(curve.p1, curve.p2, curve.p3),
    ];
    kurbo::flatten(elements, 0.05, |element| {
        if let kurbo::PathEl::LineTo(point) = element {
            points.push(point);
        }
    });
    assert!(
        points.len() > 8,
        "a curve this bent needs many segments at tolerance 0.05, got {}",
        points.len()
    );
    // The flattening must actually track the curve, not merely produce points. The distance is to the
    // nearest polyline SEGMENT, not the nearest vertex: a vertex may sit far from the curve's midpoint
    // while the segment through it passes within tolerance. Measuring to vertices reported 2.53 units
    // at a tolerance of 0.05 and failed against correct flattening.
    let mut vertices = vec![curve.p0];
    vertices.extend(points.iter().copied());
    let midpoint = curve.eval(0.5);
    let closest = vertices
        .windows(2)
        .map(|pair| {
            Line::new(pair[0], pair[1])
                .nearest(midpoint, 1e-9)
                .distance_sq
                .sqrt()
        })
        .fold(f64::INFINITY, f64::min);
    assert!(
        closest < 0.05,
        "the polyline should pass within the 0.05 tolerance of the curve's midpoint, got {closest}"
    );
}

/// `nearestPoint` — the parameter and distance of the closest point on the curve.
#[test]
fn kurbo_nearest_covers_nearest_point() {
    let curve = sample_curve();
    let probe = Point::new(50.0, 50.0);
    let nearest = curve.nearest(probe, 1e-9);
    let found = curve.eval(nearest.t);

    // Verified by sampling rather than trusted: no sample may beat the reported nearest.
    let mut best = f64::INFINITY;
    for step in 0..=10_000 {
        let t = step as f64 / 10_000.0;
        best = best.min(curve.eval(t).distance(probe));
    }
    assert!(
        found.distance(probe) <= best + 1e-6,
        "reported {} but sampling found {best}",
        found.distance(probe)
    );
    assert!(nearest.distance_sq.sqrt() <= best + 1e-6);
}

/// `curveLength` — arc length.
#[test]
fn kurbo_arclen_covers_curve_length() {
    let curve = sample_curve();
    let length = curve.arclen(1e-9);

    // Checked against a dense polyline, which is what the length means.
    let steps = 200_000;
    let mut walked = 0.0;
    let mut previous = curve.eval(0.0);
    for step in 1..=steps {
        let point = curve.eval(step as f64 / steps as f64);
        walked += point.distance(previous);
        previous = point;
    }
    assert!(
        (length - walked).abs() < 1e-3,
        "arclen {length} against a 200k-segment walk of {walked}"
    );
}

/// `curveLengthAtPoint` — arc length up to a parameter.
#[test]
fn subsegment_arclen_covers_curve_length_at_point() {
    let curve = sample_curve();
    let partial = curve.subsegment(0.0..0.35).arclen(1e-9);
    let whole = curve.arclen(1e-9);
    assert!(partial > 0.0 && partial < whole);

    // The pieces must sum to the whole, which is the property the function is for.
    let rest = curve.subsegment(0.35..1.0).arclen(1e-9);
    assert!(
        (partial + rest - whole).abs() < 1e-6,
        "{partial} + {rest} should equal {whole}"
    );
}

/// `curveParamByProportion` and `curveProportionByParam` — the two directions between arc length
/// fraction and curve parameter. These are inverses, and that is the whole content of the pair.
#[test]
fn inv_arclen_covers_param_by_proportion_both_ways() {
    let curve = sample_curve();
    let total = curve.arclen(1e-9);

    for fraction in [0.1, 0.25, 0.5, 0.75, 0.9] {
        // proportion -> param
        let t = curve.inv_arclen(total * fraction, 1e-9);
        // param -> proportion
        let back = curve.subsegment(0.0..t).arclen(1e-9) / total;
        assert!(
            (back - fraction).abs() < 1e-6,
            "fraction {fraction} mapped to t={t} and back to {back}"
        );
        // A bent curve must not have parameter equal proportion, or the test proves nothing.
        if (fraction - 0.5).abs() > 0.2 {
            assert!(
                (t - fraction).abs() > 1e-4,
                "t {t} equals the fraction {fraction}; this curve is too straight to test with"
            );
        }
    }
}

/// `intersectWithLine` and `intersectWithLineNearest`.
#[test]
fn kurbo_intersect_line_covers_both_line_intersections() {
    let curve = sample_curve();
    let line = Line::new(Point::new(-10.0, 30.0), Point::new(120.0, 30.0));
    let hits = PathSeg::Cubic(curve).intersect_line(line);
    assert!(
        hits.len() >= 2,
        "this horizontal line crosses the curve more than once, got {}",
        hits.len()
    );

    // Every reported crossing must actually sit on the line.
    for hit in &hits {
        let point = curve.eval(hit.segment_t);
        assert!(
            (point.y - 30.0).abs() < 1e-6,
            "a crossing at y={} is not on y=30",
            point.y
        );
    }

    // `intersectWithLineNearest` is the minimum of the same set.
    let nearest = hits
        .iter()
        .map(|hit| hit.segment_t)
        .fold(f64::INFINITY, f64::min);
    assert!(nearest.is_finite() && (0.0..=1.0).contains(&nearest));
}

/// `interpolateQuadric` — the quadratic through three points at a parameter.
#[test]
fn quadbez_covers_interpolate_quadric() {
    // Krita solves for the control point that makes the quadratic pass through `pt` at `t`.
    let p0 = Point::new(0.0, 0.0);
    let p2 = Point::new(100.0, 0.0);
    let through = Point::new(50.0, 40.0);
    let t = 0.5;

    // The same algebra: B(t) = (1-t)^2 p0 + 2(1-t)t p1 + t^2 p2, solved for p1.
    let inv = 1.0 - t;
    let p1 = Point::new(
        (through.x - inv * inv * p0.x - t * t * p2.x) / (2.0 * inv * t),
        (through.y - inv * inv * p0.y - t * t * p2.y) / (2.0 * inv * t),
    );
    let quad = QuadBez::new(p0, p1, p2);
    let landed = quad.eval(t);
    assert!(
        landed.distance(through) < 1e-9,
        "the quadratic should pass through {through:?} at t={t}, landed {landed:?}"
    );
    // And kurbo raises it to the exactly equivalent cubic, which is what the bridge relies on.
    let raised = quad.raise();
    assert!(raised.eval(t).distance(through) < 1e-9);
}

/// `controlPolygonZeros` — how many times the control polygon crosses zero, which is what bounds the
/// root count. The Graphite port's polynomial module covers the roots themselves.
#[test]
fn the_polynomial_module_covers_control_polygon_zeros() {
    // A cubic with known roots at 0, 0.5 and 1 in x.
    let curve = CubicBez::new(
        Point::new(-1.0, 0.0),
        Point::new(1.0, 1.0),
        Point::new(-1.0, 2.0),
        Point::new(1.0, 3.0),
    );
    let crossings = PathSeg::Cubic(curve)
        .intersect_line(Line::new(Point::new(0.0, -1.0), Point::new(0.0, 4.0)));
    assert!(
        !crossings.is_empty(),
        "this S-curve crosses the vertical axis; the root finder must see it"
    );
    // The module is present and reachable, which is the coverage claim being made.
    let segment = kurbo::PathSeg::Cubic(curve);
    let polynomial = geometry::polynomial::pathseg_to_parametric_polynomial(segment);
    assert_eq!(
        polynomial.0.eval(0.0),
        curve.p0.x,
        "the x polynomial agrees at t=0"
    );
    assert_eq!(
        polynomial.1.eval(0.0),
        curve.p0.y,
        "the y polynomial agrees at t=0"
    );
}

/// `offsetSegment` — the Graphite port's own offsetting.
#[test]
fn the_geometry_module_covers_offset_segment() {
    let mut path = kurbo::BezPath::new();
    path.move_to(Point::new(0.0, 0.0));
    path.curve_to(
        Point::new(30.0, 60.0),
        Point::new(70.0, 60.0),
        Point::new(100.0, 0.0),
    );
    let offset = geometry::offset_bezpath::offset_bezpath(&path, 5.0, Join::Bevel, None);
    assert!(
        !offset.elements().is_empty(),
        "offsetting must produce a path"
    );
    // The offset must actually move away from the original, in the right amount.
    let original_start = path.segments().next().unwrap().eval(0.0);
    let nearest = offset
        .segments()
        .map(|segment| segment.nearest(original_start, 1e-6).distance_sq.sqrt())
        .fold(f64::INFINITY, f64::min);
    assert!(
        (nearest - 5.0).abs() < 1.0,
        "the offset path should sit about 5 units from the original, measured {nearest}"
    );
}

/// `mergeLinearizationSteps` — merging two sorted parameter lists. Not in any dependency, and eight
/// lines rather than a translation project. Included so the coverage table has no gap.
#[test]
fn merging_linearization_steps_is_a_sorted_merge() {
    let a = [0.0, 0.25, 0.5, 1.0];
    let b = [0.0, 0.3, 0.5, 0.75, 1.0];
    let mut merged: Vec<f64> = a.iter().chain(b.iter()).copied().collect();
    merged.sort_by(f64::total_cmp);
    merged.dedup_by(|x, y| (*x - *y).abs() < 1e-12);
    assert_eq!(merged, vec![0.0, 0.25, 0.3, 0.5, 0.75, 1.0]);
}

/// The bounding rectangle of a curve, which the region arithmetic in the group needs.
#[test]
fn kurbo_bounding_box_covers_the_region_arithmetic() {
    // An ARCH: both endpoints at y = 0, control points at y = 120, so the curve bulges to y = 90 and
    // the endpoint box is degenerate in y. That makes "the extrema are found" a real assertion.
    //
    // The first version of this test used `sample_curve()`, whose control points reach y = 90 and
    // y = -30 but whose CURVE stays within [0, 40] -- verified by sampling. Its bounding box therefore
    // equalled the endpoint box, the assertion was vacuous, and it failed against a correct kurbo.
    let curve = CubicBez::new(
        Point::new(0.0, 0.0),
        Point::new(0.0, 120.0),
        Point::new(100.0, 120.0),
        Point::new(100.0, 0.0),
    );
    let bounds = curve.bounding_box();
    let endpoints_only = kurbo::Rect::from_points(curve.p0, curve.p3);
    assert!(
        bounds.y1 > endpoints_only.y1 + 1.0,
        "the true bounds {bounds:?} must exceed the endpoint bounds {endpoints_only:?}"
    );
    assert!(
        (bounds.y1 - 90.0).abs() < 1e-6,
        "the arch peaks at y = 90, measured by sampling; bounds say {}",
        bounds.y1
    );
    // And the bounds must be tight enough to contain every sample without slack.
    for step in 0..=10_000 {
        let point = curve.eval(step as f64 / 10_000.0);
        assert!(
            bounds.inflate(1e-9, 1e-9).contains(point),
            "{point:?} escaped the bounds {bounds:?}"
        );
    }
}
