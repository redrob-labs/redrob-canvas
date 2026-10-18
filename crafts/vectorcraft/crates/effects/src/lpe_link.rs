//! Live path effects that link to other objects (Effect › Path Effects): Clone Original, Attach
//! Path, Bounding Box, Fill Between Many and Fill Between Strokes. An effect names its linked
//! objects by id (`source`, `source2`, `sources`); the engine keeps a copy of each linked object's
//! geometry in the effect's parameters under [`SOURCES`] after every edit (so the effect follows
//! the linked object as it is edited), and the evaluation here reads that copy: the effects stay
//! pure functions of their parameters, like every other geometry effect.
//!
//! Power Clip ([`POWER_CLIP`]) lives here too: applied to a clip group's clipping path it inverts
//! the clip (everything but the clipping path shows) and hides the clipping path's own paint; the
//! renderer and the writers read it through [`crate::clip_outline`].

use redrob_vector_geom::shapes;
use redrob_vector_geom::{Anchor, AnchorKind, FillRule, PathData, Point, Rect, SubPath};
use serde_json::{Value, json};

use crate::lpe_util::*;
use crate::util::{flag, num, text};

pub const CLONE_ORIGINAL: &str = "path.cloneOriginal";
pub const ATTACH_PATH: &str = "path.attachPath";
pub const BOUNDING_BOX: &str = "path.boundingBox";
pub const FILL_BETWEEN_MANY: &str = "path.fillBetweenMany";
pub const FILL_BETWEEN_STROKES: &str = "path.fillBetweenStrokes";
pub const POWER_CLIP: &str = "path.powerClip";

/// The parameter the engine keeps the linked objects' geometry in: a list of
/// `{id, path: PathData, bounds: [x0, y0, x1, y1], visual: [x0, y0, x1, y1]}` in the order of
/// [`link_ids`]. Keys starting with `__` are kept out of effect dialogs.
pub const SOURCES: &str = "__sources";

/// The ends Attach Path attaches: (label, value).
pub const ATTACH_ENDS: [(&str, &str); 3] = [("Start", "start"), ("End", "end"), ("Both", "both")];

/// Is `id` an effect that links to other objects?
pub fn is_linked(id: &str) -> bool {
    matches!(id, CLONE_ORIGINAL | ATTACH_PATH | BOUNDING_BOX | FILL_BETWEEN_MANY | FILL_BETWEEN_STROKES)
}

/// The ids of the objects effect `id` with `params` links to, in the order their geometry is
/// kept: `source`, then `source2`, then `sources` (a list). Empty for an effect that links to
/// nothing.
pub fn link_ids(id: &str, params: &Value) -> Vec<u64> {
    if !is_linked(id) {
        return vec![];
    }
    let one = |key: &str| params.get(key).and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())));
    let mut out: Vec<u64> = one("source").into_iter().chain(one("source2")).collect();
    if let Some(a) = params.get("sources").and_then(Value::as_array) {
        out.extend(a.iter().filter_map(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))));
    }
    out
}

/// One linked object's geometry as the engine kept it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Linked {
    pub id: u64,
    pub path: PathData,
    pub bounds: Option<Rect>,
    pub visual: Option<Rect>,
}

impl Linked {
    /// The record as the engine writes it into [`SOURCES`].
    pub fn to_value(&self) -> Value {
        let rect = |r: Option<Rect>| r.map(|r| json!([r.x0, r.y0, r.x1, r.y1])).unwrap_or(Value::Null);
        json!({"id": self.id, "path": serde_json::to_value(&self.path).unwrap_or(Value::Null), "bounds": rect(self.bounds), "visual": rect(self.visual)})
    }

    fn from_value(v: &Value) -> Option<Self> {
        let rect = |key: &str| -> Option<Rect> {
            let a = v.get(key)?.as_array()?;
            if a.len() != 4 {
                return None;
            }
            let n: Vec<f64> = a.iter().filter_map(Value::as_f64).filter(|x| x.is_finite()).collect();
            (n.len() == 4).then(|| Rect::new(n[0], n[1], n[2], n[3]))
        };
        Some(Self {
            id: v.get("id")?.as_u64()?,
            path: v.get("path").and_then(|p| serde_json::from_value(p.clone()).ok()).unwrap_or_default(),
            bounds: rect("bounds"),
            visual: rect("visual"),
        })
    }
}

/// The linked objects' geometry kept in `params` ([`SOURCES`]).
pub fn sources(params: &Value) -> Vec<Linked> {
    params.get(SOURCES).and_then(Value::as_array).map(|a| a.iter().filter_map(Linked::from_value).collect()).unwrap_or_default()
}

/// The kept geometry of the object `id` links to.
fn source_by_id(params: &Value, id: u64) -> Option<Linked> {
    sources(params).into_iter().find(|l| l.id == id)
}

/// The first linked object's kept geometry (`source`).
fn first_source(effect: &str, params: &Value) -> Option<Linked> {
    let ids = link_ids(effect, params);
    ids.first().and_then(|id| source_by_id(params, *id))
}

/// Clone Original: the path takes the linked object's geometry (its style stays its own, unless
/// the engine copies the original's with `copyStyle`); edits to the original follow.
pub fn clone_original(path: &PathData, p: &Value) -> PathData {
    match first_source(CLONE_ORIGINAL, p) {
        Some(l) if !l.path.is_empty() => l.path,
        _ => path.clone(),
    }
}

/// Bounding Box: a rectangle round the linked object's geometric bounds (its visual bounds, the
/// strokes included, with `visual`), following it.
pub fn bounding_box(path: &PathData, p: &Value) -> PathData {
    let Some(l) = first_source(BOUNDING_BOX, p) else { return path.clone() };
    let r = if flag(p, "visual", false) { l.visual.or(l.bounds) } else { l.bounds.or(l.visual) };
    match r.or_else(|| l.path.bounds()) {
        Some(r) => shapes::rectangle(r),
        None => path.clone(),
    }
}

/// Attach Path: the path's start (`end: "start"`), end or both are moved onto the linked path at
/// `startPosition` and `endPosition` (0..1 of its length); with `curve`, the attached anchor's
/// handle follows the linked path's tangent there, `handleLength` long (a corner otherwise).
pub fn attach_path(path: &PathData, p: &Value) -> PathData {
    let Some(l) = first_source(ATTACH_PATH, p) else { return path.clone() };
    let Some(target) = l.path.subpaths.iter().find(|s| s.anchors.len() >= 2) else { return path.clone() };
    let m = Measured::new(target);
    if m.is_empty() {
        return path.clone();
    }
    let which = text(p, "end", "start").to_ascii_lowercase();
    let (do_start, do_end) = match which.as_str() {
        "end" => (false, true),
        "both" => (true, true),
        _ => (true, false),
    };
    let curve = flag(p, "curve", true);
    let handle = num(p, "handleLength", 20.0).clamp(0.0, 1e4);
    let mut out = path.clone();
    let Some(sp) = out.subpaths.iter_mut().find(|s| !s.closed && s.anchors.len() >= 2) else { return out };
    let n = sp.anchors.len();
    if do_start {
        let (q, t) = m.at(num(p, "startPosition", 0.0).clamp(0.0, 1.0) * m.total);
        // Leaving the attachment point along the linked path's tangent (either way: the way the
        // path already goes).
        let going = sp.anchors[1].p - q;
        let dir = if going.dot(t) >= 0.0 { t } else { -t };
        let a = &mut sp.anchors[0];
        let delta = q - a.p;
        if curve {
            *a = Anchor { p: q, h_in: q, h_out: q + dir * handle, kind: AnchorKind::Corner };
        } else {
            let had_out = a.h_out.distance(a.p) > 1e-9;
            a.p = q;
            a.h_in = q;
            a.h_out = if had_out { a.h_out + delta } else { q };
        }
    }
    if do_end {
        let (q, t) = m.at(num(p, "endPosition", 1.0).clamp(0.0, 1.0) * m.total);
        let coming = q - sp.anchors[n - 2].p;
        let dir = if coming.dot(t) >= 0.0 { t } else { -t };
        let a = &mut sp.anchors[n - 1];
        let delta = q - a.p;
        if curve {
            *a = Anchor { p: q, h_in: q - dir * handle, h_out: q, kind: AnchorKind::Corner };
        } else {
            let had_in = a.h_in.distance(a.p) > 1e-9;
            a.p = q;
            a.h_out = q;
            a.h_in = if had_in { a.h_in + delta } else { q };
        }
    }
    out
}

/// The subpaths of `paths` joined into one run: each next one reversed when its end is nearer
/// (`auto_reverse`), the gaps bridged by straight lines (`join`) or left as separate subpaths.
fn joined(paths: Vec<SubPath>, join: bool, auto_reverse: bool, close: bool) -> PathData {
    let mut parts: Vec<SubPath> = paths.into_iter().filter(|s| !s.anchors.is_empty()).collect();
    if parts.is_empty() {
        return PathData::default();
    }
    if auto_reverse {
        for k in 1..parts.len() {
            let end = parts[k - 1].anchors[parts[k - 1].anchors.len() - 1].p;
            let (f, l) = (parts[k].anchors[0].p, parts[k].anchors[parts[k].anchors.len() - 1].p);
            if end.distance(l) < end.distance(f) {
                parts[k].reverse();
            }
        }
    }
    if !join {
        if close {
            for s in &mut parts {
                s.closed = true;
            }
        }
        return PathData::new(parts);
    }
    let mut anchors: Vec<Anchor> = Vec::new();
    for s in parts {
        if let Some(last) = anchors.last_mut() {
            let first = s.anchors[0];
            if last.p.distance(first.p) <= 1e-6 {
                last.h_out = first.h_out;
                anchors.extend(s.anchors.iter().skip(1).copied());
                continue;
            }
            last.h_out = last.p;
            anchors.push(Anchor { h_in: first.p, ..first });
            anchors.extend(s.anchors.iter().skip(1).copied());
        } else {
            anchors.extend(s.anchors.iter().copied());
        }
    }
    PathData::single(SubPath::new(anchors, close))
}

/// Fill Between Many: the path becomes the region the linked paths (`sources`) enclose together:
/// their subpaths one after another, each reversed when that meets the previous one's end
/// (`autoReverse`), joined into one (`join`) and closed (`close`); the result follows every linked
/// path. `method`: `original` takes each linked path as it is, `bspline` and `spiro` take its
/// anchors through those splines first.
pub fn fill_between_many(path: &PathData, p: &Value) -> PathData {
    let ids = link_ids(FILL_BETWEEN_MANY, p);
    let all = sources(p);
    let method = text(p, "method", "original").to_ascii_lowercase();
    let mut subpaths: Vec<SubPath> = Vec::new();
    for id in ids {
        let Some(l) = all.iter().find(|l| l.id == id) else { continue };
        for sp in &l.path.subpaths {
            let pts: Vec<Point> = sp.anchors.iter().map(|a| a.p).collect();
            subpaths.push(match method.as_str() {
                "bspline" => redrob_vector_geom::bspline_to_bezier(&pts, sp.closed),
                "spiro" => redrob_vector_geom::spiro_approx_to_bezier(&pts, sp.closed),
                _ => {
                    let mut o = sp.clone();
                    o.closed = false;
                    o
                }
            });
        }
    }
    if subpaths.is_empty() {
        return path.clone();
    }
    joined(subpaths, flag(p, "join", true), flag(p, "autoReverse", true), flag(p, "close", true))
}

/// Fill Between Strokes: the region between this path (its first open subpath, or the linked
/// path `source2` with `source` as the other) and the linked path `source`: the first followed by
/// the second reversed (`reverseSecond`, on by default so two paths drawn the same way enclose a
/// band), joined and closed; it follows the linked paths.
pub fn fill_between_strokes(path: &PathData, p: &Value) -> PathData {
    let ids = link_ids(FILL_BETWEEN_STROKES, p);
    let all = sources(p);
    let linked = |k: usize| ids.get(k).and_then(|id| all.iter().find(|l| l.id == *id)).and_then(|l| l.path.subpaths.first().cloned());
    let (first, second) = match (linked(0), linked(1)) {
        (Some(a), Some(b)) => (b, a),
        (Some(a), None) => match path.subpaths.first() {
            Some(own) => (own.clone(), a),
            None => return path.clone(),
        },
        _ => return path.clone(),
    };
    let mut second = second;
    if flag(p, "reverseSecond", true) {
        second.reverse();
    }
    let mut first = first;
    first.closed = false;
    second.closed = false;
    joined(vec![first, second], flag(p, "join", true), false, flag(p, "close", true))
}

/// Does the clipping path `effects` (its object-level effects) carry Power Clip's inverse?
pub fn power_clip_inverse(effects: &[redrob_vector_doc::Effect]) -> bool {
    effects.iter().any(|e| e.visible && e.id == POWER_CLIP && flag(&e.params, "inverse", false))
}

/// Does the clipping path `effects` carry Power Clip's hide clip (its own paint isn't drawn)?
pub fn power_clip_hidden(effects: &[redrob_vector_doc::Effect]) -> bool {
    effects.iter().any(|e| e.visible && e.id == POWER_CLIP && flag(&e.params, "hideClip", false))
}

/// How far the inverse clip reaches past the clipping path's bounds (document points).
pub const INVERSE_REACH: f64 = 1.0e4;

/// The inverse of a clip region: everything within [`INVERSE_REACH`] of the clipping path's bounds
/// but the region itself.
pub fn inverse_region(bp: &redrob_vector_geom::BezPath, rule: FillRule) -> PathData {
    use redrob_vector_geom::Shape;
    let b = bp.bounding_box().inflate(INVERSE_REACH, INVERSE_REACH);
    let outer = shapes::rectangle(b);
    let inner = PathData::from_bezpath(bp);
    redrob_vector_pathops::boolean(&outer, FillRule::NonZero, &inner, rule, redrob_vector_pathops::BoolOp::Difference)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linked(id: u64, pts: &[(f64, f64)]) -> Value {
        let path = PathData::single(SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), false));
        let b = path.bounds();
        Linked { id, path, bounds: b, visual: b.map(|r| r.inflate(1.0, 1.0)) }.to_value()
    }

    fn own() -> PathData {
        PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0)], false))
    }

    #[test]
    fn linked_effects_read_the_kept_geometry() {
        let p = json!({"source": 7, SOURCES: [linked(7, &[(0.0, 0.0), (100.0, 0.0), (100.0, 50.0)])]});
        assert_eq!(link_ids(CLONE_ORIGINAL, &p), vec![7]);
        let c = clone_original(&own(), &p);
        assert_eq!(c.anchor_count(), 3);
        let bb = bounding_box(&own(), &json!({"source": 7, "visual": true, SOURCES: p[SOURCES].clone()}));
        assert_eq!(bb.bounds(), Some(Rect::new(-1.0, -1.0, 101.0, 51.0)));
        let at = attach_path(&own(), &json!({"source": 7, "end": "start", "startPosition": 0.5, "curve": false, SOURCES: p[SOURCES].clone()}));
        let a0 = at.subpaths[0].anchors[0].p;
        assert!((a0.x - 75.0).abs() < 1e-6 || (a0.x - 100.0).abs() < 1e-6, "{a0:?}");
    }

    #[test]
    fn fills_between_paths_close_a_band() {
        let p = json!({"sources": [1, 2], SOURCES: [linked(1, &[(0.0, 0.0), (100.0, 0.0)]), linked(2, &[(0.0, 50.0), (100.0, 50.0)])]});
        let out = fill_between_many(&own(), &p);
        assert_eq!(out.subpaths.len(), 1);
        assert!(out.subpaths[0].closed && out.subpaths[0].anchors.len() == 4);
        let s = fill_between_strokes(&own(), &json!({"source": 2, SOURCES: [linked(2, &[(0.0, 50.0), (10.0, 50.0)])]}));
        assert!(s.subpaths[0].closed);
        assert!((redrob_vector_pathops::area(&s, FillRule::NonZero) - 500.0).abs() < 1e-6);
    }
}
