//! Live path effects on the path's own geometry (Effect › Path): Bend along a spine, B-Spline and
//! Spiro splines through the anchors.
//!
//! - [`bend`]: the object's box is laid along one cubic spine (four points in box coordinates):
//!   its long axis follows the spine's arc length, its other axis the spine's normal, scaled by
//!   `widthScale`. The original path stays as it is, the effect being live.
//! - [`bspline`] and [`spiro`]: the anchors of every subpath are the control points (B-Spline) or
//!   the through-points (Spiro) of a spline from `redrob_vector_geom::spline`.

use redrob_vector_geom::kurbo::ParamCurveArclen;
use redrob_vector_geom::{AnchorKind, CubicBez, ParamCurve, PathData, Point, Rect, Vec2};
use serde_json::Value;

use crate::util::*;

/// Arc-length accuracy of the bend (document points).
const ARCLEN: f64 = 1e-5;

/// The effect ids this module evaluates.
pub const BEND: &str = "path.bend";
pub const BSPLINE: &str = "path.bspline";
pub const SPIRO: &str = "path.spiro";

/// The default spine of Bend: a gentle arch from the box's left middle to its right middle.
pub const BEND_SPINE: &str = "0,0.5 0.3,0 0.7,0 1,0.5";

/// Four points `x,y x,y x,y x,y` (or `[[x, y], …]`) in box coordinates, else the default spine.
fn spine_points(p: &Value) -> [Point; 4] {
    let mut out = [Point::new(0.0, 0.5), Point::new(0.3, 0.0), Point::new(0.7, 0.0), Point::new(1.0, 0.5)];
    let parsed: Vec<Point> = match p.get("spine") {
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
    };
    if parsed.len() == 4 {
        out.copy_from_slice(&parsed);
    } else if parsed.len() == 2 {
        // Two points: a straight spine between them.
        out = [parsed[0], parsed[0].lerp(parsed[1], 1.0 / 3.0), parsed[0].lerp(parsed[1], 2.0 / 3.0), parsed[1]];
    }
    out
}

/// Bend: lay `path` (reference box `b`) along the spine in `p`.
pub fn bend(path: &PathData, b: Rect, p: &Value) -> PathData {
    let (w, h) = (b.width().max(1e-9), b.height().max(1e-9));
    let scale = num(p, "widthScale", 100.0).clamp(0.0, 1000.0) / 100.0;
    let vertical = flag(p, "vertical", false);
    let to_doc = |q: Point| Point::new(b.x0 + q.x * w, b.y0 + q.y * h);
    let s = spine_points(p).map(to_doc);
    let spine = CubicBez::new(s[0], s[1], s[2], s[3]);
    let len = spine.arclen(ARCLEN);
    if len <= 1e-9 {
        return path.clone();
    }
    let c = b.center();
    let f = |q: Point| {
        // Along the box's long axis as a fraction, and across it as a distance from the middle.
        let (u, across) = if vertical { ((q.y - b.y0) / h, q.x - c.x) } else { ((q.x - b.x0) / w, q.y - c.y) };
        let across = across * scale;
        let (at, tan) = if u <= 0.0 {
            let t = tangent(&spine, 0.0);
            (spine.p0 + t * (u * len), t)
        } else if u >= 1.0 {
            let t = tangent(&spine, 1.0);
            (spine.p3 + t * ((u - 1.0) * len), t)
        } else {
            let t = spine.inv_arclen(u * len, ARCLEN).clamp(0.0, 1.0);
            (spine.eval(t), tangent(&spine, t))
        };
        // The box's "down" (+y, or +x when vertical) goes to the right of the direction of travel.
        let right = Vec2::new(-tan.y, tan.x);
        at + right * across
    };
    let piece = len / 24.0;
    map_nonlinear(path, piece, f)
}

fn tangent(c: &CubicBez, t: f64) -> Vec2 {
    redrob_vector_geom::bez::tangent(&redrob_vector_geom::PathSeg::Cubic(*c), t)
}

/// B-Spline: the anchors of each subpath as the control polygon of a cubic B-spline.
pub fn bspline(path: &PathData, p: &Value) -> PathData {
    let weight = num(p, "weight", 100.0).clamp(0.0, 100.0) / 100.0;
    PathData::new(
        path.subpaths
            .iter()
            .map(|sp| {
                let pts: Vec<Point> = sp.anchors.iter().map(|a| a.p).collect();
                redrob_vector_geom::bspline_to_bezier(&pts, sp.closed, weight)
            })
            .collect(),
    )
}

/// Spiro: a smooth spline through the anchors of each subpath (see
/// [`redrob_vector_geom::spline`] for the approximation). With `cusps`, corner anchors without
/// handles stay sharp.
pub fn spiro(path: &PathData, p: &Value) -> PathData {
    let cusps = flag(p, "cusps", false);
    PathData::new(
        path.subpaths
            .iter()
            .map(|sp| {
                let pts: Vec<Point> = sp.anchors.iter().map(|a| a.p).collect();
                let corners: Vec<bool> = sp.anchors.iter().map(|a| cusps && a.kind == AnchorKind::Corner && !a.has_in() && !a.has_out()).collect();
                redrob_vector_geom::spiro_approx_to_bezier(&pts, sp.closed, Some(&corners))
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn line(x0: f64, x1: f64) -> PathData {
        PathData::single(redrob_vector_geom::SubPath::polyline(&[Point::new(x0, 0.0), Point::new(x1, 0.0)], false))
    }

    #[test]
    fn bend_follows_the_spine_and_splines_keep_their_subpaths() {
        let pd = line(0.0, 100.0);
        let b = Rect::new(0.0, -5.0, 100.0, 5.0);
        // A spine along the top edge of the box: the line moves up by half the box's height.
        let out = bend(&pd, b, &json!({"spine": "0,0 0.33,0 0.67,0 1,0", "widthScale": 100.0}));
        let bb = out.bounds().unwrap();
        assert!((bb.y0 + 5.0).abs() < 1e-6 && (bb.y1 + 5.0).abs() < 1e-6, "{bb:?}");
        assert!((bb.x0).abs() < 1e-6 && (bb.x1 - 100.0).abs() < 1e-6);
        let sq = PathData::single(redrob_vector_geom::SubPath::polyline(
            &[Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 100.0), Point::new(0.0, 100.0)],
            true,
        ));
        let bs = bspline(&sq, &json!({"weight": 100.0}));
        assert_eq!(bs.subpaths.len(), 1);
        assert!(bs.subpaths[0].closed && bs.subpaths[0].anchors.len() == 4);
        let sp = spiro(&sq, &json!({}));
        assert_eq!(sp.subpaths[0].anchors.iter().map(|a| a.p).collect::<Vec<_>>(), sq.subpaths[0].anchors.iter().map(|a| a.p).collect::<Vec<_>>());
    }
}
