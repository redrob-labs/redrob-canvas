//! Live path effects on the path's own geometry (Effect › Path): Bend along a spine, B-Spline and
//! Spiro splines through the anchors, Dashed Stroke as real geometry.
//!
//! - [`bend`]: the object's box is laid along one cubic spine (four points in box coordinates):
//!   its long axis follows the spine's arc length, its other axis the spine's normal, scaled by
//!   `widthScale`. The original path stays as it is, the effect being live.
//! - [`bspline`] and [`spiro`]: the anchors of every subpath are the control points (B-Spline) or
//!   the through-points (Spiro) of a spline from `redrob_vector_geom::spline`.
//! - [`dashed_stroke`]: each segment becomes `dashes` dashes separated by holes `holeFactor` of a
//!   period wide, optionally unified to one dash length along the subpath, with half dashes at the
//!   ends and bare offsets, as open subpaths or as the outline of the stroke along them.

use redrob_vector_doc::StrokeLayer;
use redrob_vector_geom::kurbo::ParamCurveArclen;
use redrob_vector_geom::{Anchor, AnchorKind, CubicBez, ParamCurve, PathData, Point, Rect, SubPath, Vec2};
use serde_json::Value;

use crate::stroke::{OUTLINE_TOL, style};
use crate::util::*;

/// Arc-length accuracy of the dashes and the bend (document points).
const ARCLEN: f64 = 1e-5;

/// The effect ids this module evaluates.
pub const BEND: &str = "path.bend";
pub const BSPLINE: &str = "path.bspline";
pub const SPIRO: &str = "path.spiro";
pub const DASHED_STROKE: &str = "path.dashedStroke";

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

/// The dashes of one segment of length `len` as (from, to) arc lengths: `n` dashes of `d` each
/// (the first and last halved when asked) with holes `gap` between them, after `start` bare.
fn layout(start: f64, n: usize, d: f64, gap: f64, half_start: bool, half_end: bool) -> Vec<(f64, f64)> {
    let mut out = Vec::with_capacity(n);
    let mut at = start;
    for k in 0..n {
        let mut l = d;
        if k == 0 && half_start {
            l *= 0.5;
        }
        if k + 1 == n && half_end {
            l *= 0.5;
        }
        out.push((at, at + l));
        at += l + gap;
    }
    out
}

/// Dashed Stroke: every segment of `path` cut into dashes (see the module notes). `stroke` is the
/// stroke outlined when `strokeToPath` is set (a plain 1 pt stroke without one).
pub fn dashed_stroke(path: &PathData, p: &Value, stroke: Option<&StrokeLayer>) -> PathData {
    let n = num(p, "dashes", 3.0).clamp(1.0, 1000.0).round() as usize;
    let hole = num(p, "holeFactor", 0.5).clamp(0.0, 0.95);
    let r = hole / (1.0 - hole);
    let unify = flag(p, "unify", true);
    let (hs, he) = (flag(p, "halfStart", false), flag(p, "halfEnd", false));
    let so = num(p, "startOffset", 0.0).clamp(0.0, 45.0) / 100.0;
    let eo = num(p, "endOffset", 0.0).clamp(0.0, 45.0) / 100.0;
    let halves = [hs, he].iter().filter(|h| **h).count() as f64 * 0.5;
    let mut out = Vec::new();
    for sp in &path.subpaths {
        let segs = sp.segment_count();
        if segs == 0 {
            continue;
        }
        let cubics: Vec<CubicBez> = (0..segs).map(|i| sp.segment(i)).collect();
        let lens: Vec<f64> = cubics.iter().map(|c| c.arclen(ARCLEN)).collect();
        // One dash length for the whole subpath when unified: that of a segment of mean length.
        let unified = {
            let total: f64 = lens.iter().sum();
            let usable = total * (1.0 - so - eo) / segs as f64;
            usable / (n as f64 - halves + (n as f64 - 1.0) * r).max(0.5)
        };
        for (c, &len) in cubics.iter().zip(&lens) {
            if len <= 1e-9 {
                continue;
            }
            let usable = len * (1.0 - so - eo);
            let (count, d, gap) = if unify && unified > 1e-9 {
                let period = unified * (1.0 + r);
                let mut count = ((usable + unified * r) / period).round().max(1.0) as usize;
                while count > 1 && count as f64 * unified > usable {
                    count -= 1;
                }
                let gap = if count > 1 { (usable - count as f64 * unified) / (count - 1) as f64 } else { 0.0 };
                (count, unified.min(usable), gap)
            } else {
                let d = usable / (n as f64 - halves + (n as f64 - 1.0) * r).max(0.5);
                (n, d, d * r)
            };
            for (a, z) in layout(so * len, count, d, gap, hs, he) {
                let (ta, tz) = (c.inv_arclen(a.clamp(0.0, len), ARCLEN).clamp(0.0, 1.0), c.inv_arclen(z.clamp(0.0, len), ARCLEN).clamp(0.0, 1.0));
                if tz <= ta + 1e-9 {
                    continue;
                }
                let piece = c.subsegment(ta..tz);
                out.push(SubPath::new(
                    vec![Anchor::with_handles(piece.p0, piece.p0, piece.p1), Anchor::with_handles(piece.p3, piece.p2, piece.p3)],
                    false,
                ));
            }
        }
    }
    let dashes = PathData::new(out);
    if !flag(p, "strokeToPath", false) {
        return dashes;
    }
    let fallback;
    let st = match stroke {
        Some(s) if s.width > 0.0 && s.width.is_finite() => s,
        _ => {
            fallback = StrokeLayer::new(redrob_vector_doc::color::Paint::None, 1.0);
            &fallback
        }
    };
    let bp = dashes.to_bezpath();
    let outline = redrob_vector_geom::kurbo::stroke(bp.iter(), &style(st, st.width), &redrob_vector_geom::kurbo::StrokeOpts::default(), OUTLINE_TOL);
    redrob_vector_pathops::stroke_region(&[outline])
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn line(x0: f64, x1: f64) -> PathData {
        PathData::single(SubPath::polyline(&[Point::new(x0, 0.0), Point::new(x1, 0.0)], false))
    }

    #[test]
    fn dashes_count_and_cover_the_segment_ends() {
        let out = dashed_stroke(&line(0.0, 100.0), &json!({"dashes": 4, "holeFactor": 0.5, "unify": false}), None);
        assert_eq!(out.subpaths.len(), 4);
        let first = &out.subpaths[0];
        let last = &out.subpaths[3];
        assert!((first.anchors[0].p.x).abs() < 1e-6);
        assert!((last.anchors[1].p.x - 100.0).abs() < 1e-6);
        // Dash and gap are equal at a hole factor of 0.5: 7 pieces of 100/7.
        let d = first.anchors[1].p.x - first.anchors[0].p.x;
        assert!((d - 100.0 / 7.0).abs() < 1e-3, "{d}");
        // Half dashes at the ends shorten only the ends.
        let half = dashed_stroke(&line(0.0, 100.0), &json!({"dashes": 4, "halfStart": true, "halfEnd": true, "unify": false}), None);
        let d0 = half.subpaths[0].anchors[1].p.x - half.subpaths[0].anchors[0].p.x;
        let d1 = half.subpaths[1].anchors[1].p.x - half.subpaths[1].anchors[0].p.x;
        assert!((2.0 * d0 - d1).abs() < 1e-6, "{d0} {d1}");
    }

    #[test]
    fn unified_dashes_share_one_length_across_segments() {
        let pd = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 50.0)], false));
        let out = dashed_stroke(&pd, &json!({"dashes": 2, "holeFactor": 0.5, "unify": true}), None);
        let lengths: Vec<f64> = out.subpaths.iter().map(|s| s.anchors[0].p.distance(s.anchors[1].p)).collect();
        assert!(lengths.len() >= 3, "{lengths:?}");
        let d = lengths[0];
        assert!(lengths.iter().all(|l| (l - d).abs() < 1e-3), "{lengths:?}");
        // Stroke-to-path makes one filled region with area.
        let filled = dashed_stroke(&pd, &json!({"dashes": 2, "strokeToPath": true}), None);
        assert!(filled.subpaths.iter().all(|s| s.closed));
        assert!(filled.bounds().is_some());
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
        let sq = PathData::single(SubPath::polyline(
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
