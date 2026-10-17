//! Node-tool operations on direct-selected anchors: the segment type (line / curve), walking the
//! selection from anchor to anchor (Tab), growing and shrinking it spatially (Page Up / Page
//! Down) and touch-selecting the anchors a freehand stroke passes over (Alt-drag).

use std::collections::BTreeSet;

use redrob_vector_doc::{AnchorRef, NodeId, NodeKind};
use redrob_vector_geom::{PathData, Point};
use serde_json::{Value, json};

use super::path::path_mut;
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "path.segmentType",
            "Segment Type",
            [],
            None,
            "{to: \"line\"|\"curve\"} the selected segments (or the segments between direct-selected anchors): line retracts the handles at both ends so the segment is straight; curve gives a straight segment handles a third of the way along it, so it keeps its shape and can be bent → {changed: segments}",
            has_anchors,
            segment_type
        ),
        cmd!(
            "select.anchorStep",
            "Next Anchor",
            [],
            None,
            "{dir?: 1|-1 (default 1)} select the anchor after (or before) the selected one along its path, wrapping round; a path selected whole or with several anchors selected starts from its first anchor (Tab / Shift+Tab in the node tool)",
            has_selection,
            anchor_step
        ),
        cmd!(
            "select.anchorGrow",
            "Grow Anchor Selection",
            [],
            None,
            "{grow?: bool (default true)} grow the direct selection by the unselected anchor nearest to it (over the selected paths), or shrink it by the selected anchor farthest from its centre (Page Up / Page Down in the node tool)",
            has_anchors,
            anchor_grow
        ),
        cmd!(
            "select.anchorsTouching",
            "Touch Select Anchors",
            [],
            None,
            "{points: [[x,y]…], tolerance?: pt (3), mode?: \"set\"|\"add\"|\"toggle\" (set), ids?: the paths to look at (default: the selected paths, else every editable path)} select the anchors the polyline passes within tolerance of (the node tool's Alt-drag) → {count}",
            has_doc,
            anchors_touching
        ),
    ]
}

/// Every anchor of `pd` in path order.
fn all_anchors(pd: &PathData) -> Vec<AnchorRef> {
    pd.anchors().map(|(si, ai, _)| (si, ai)).collect()
}

/// The segments to retype: the selected segments of each selected path, else the segments whose
/// both ends are direct-selected (every segment of a path selected whole).
fn target_segments(s: &Session) -> Result<Vec<(NodeId, Vec<(usize, usize)>)>> {
    let st = s.doc()?;
    let mut out = vec![];
    for id in &st.selection.objects {
        let Some(pd) = st.doc.node(*id).and_then(|n| n.path_data()) else { continue };
        if let Some(segs) = st.selection.segments.get(id).filter(|s| !s.is_empty()) {
            out.push((*id, segs.iter().copied().collect()));
            continue;
        }
        let picked: BTreeSet<AnchorRef> = match st.selection.anchors.get(id) {
            Some(set) => set.clone(),
            None => all_anchors(pd).into_iter().collect(),
        };
        let mut segs = vec![];
        for (si, sp) in pd.subpaths.iter().enumerate() {
            let n = sp.anchors.len();
            for seg in 0..sp.segment_count() {
                if picked.contains(&(si, seg)) && picked.contains(&(si, (seg + 1) % n)) {
                    segs.push((si, seg));
                }
            }
        }
        if !segs.is_empty() {
            out.push((*id, segs));
        }
    }
    Ok(out)
}

fn segment_type(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.segmentType";
    let to_line = match str_param(p, "to") {
        Some("line") => true,
        Some("curve") => false,
        other => return Err(bad(C, format!("to must be line|curve, got {other:?}"))),
    };
    let targets = target_segments(s)?;
    if targets.is_empty() {
        return Ok(json!({ "changed": 0 }));
    }
    let changed = s.edit(if to_line { "Make Segment Line" } else { "Make Segment Curve" }, |d, _| {
        let mut changed = 0;
        for (id, segs) in &targets {
            let path = path_mut(d, *id)?;
            for &(si, seg) in segs {
                let Some(sp) = path.subpaths.get_mut(si) else { continue };
                let n = sp.anchors.len();
                if seg >= sp.segment_count() {
                    continue;
                }
                let (ia, ib) = (seg % n, (seg + 1) % n);
                let (pa, pb) = (sp.anchors[ia].p, sp.anchors[ib].p);
                if to_line {
                    sp.anchors[ia].set_handle(true, pa, true);
                    sp.anchors[ib].set_handle(false, pb, true);
                } else if sp.segment_is_line(seg) {
                    // Handles a third of the way along the line: the same shape, now a curve.
                    let d = pb - pa;
                    sp.anchors[ia].set_handle(true, pa + d / 3.0, true);
                    sp.anchors[ib].set_handle(false, pb - d / 3.0, true);
                } else {
                    continue;
                }
                changed += 1;
            }
        }
        Ok(changed)
    })?;
    Ok(json!({ "changed": changed }))
}

fn anchor_step(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "select.anchorStep";
    let dir = match p.get("dir").filter(|v| !v.is_null()) {
        None => 1i64,
        Some(v) => v.as_i64().filter(|d| *d == 1 || *d == -1).ok_or_else(|| bad(C, "dir must be 1 or -1"))?,
    };
    let st = s.doc()?;
    // The path walked: the one with direct-selected anchors, else the first selected path.
    let id = st
        .selection
        .objects
        .iter()
        .copied()
        .find(|id| st.selection.anchors.get(id).is_some_and(|a| !a.is_empty()))
        .or_else(|| st.selection.objects.iter().copied().find(|id| st.doc.node(*id).is_some_and(|n| n.path_data().is_some())))
        .ok_or_else(|| bad(C, "no path selected"))?;
    let pd = st.doc.node(id).and_then(|n| n.path_data()).ok_or_else(|| bad(C, "no path selected"))?;
    let all = all_anchors(pd);
    if all.is_empty() {
        return Ok(json!({ "anchor": Value::Null }));
    }
    let current = st.selection.anchors.get(&id).filter(|a| a.len() == 1).and_then(|a| a.iter().next().copied());
    let next = match current.and_then(|c| all.iter().position(|a| *a == c)) {
        Some(i) => all[(i as i64 + dir).rem_euclid(all.len() as i64) as usize],
        None => all[0],
    };
    s.select(|_, sel| {
        sel.clear();
        sel.add(id);
        sel.anchors.insert(id, BTreeSet::from([next]));
    })?;
    Ok(json!({ "id": id.0, "anchor": [next.0, next.1] }))
}

fn anchor_grow(s: &mut Session, p: &Value) -> Result<Value> {
    let grow = bool_or(p, "grow", true);
    let st = s.doc()?;
    let mut selected: Vec<(NodeId, AnchorRef, Point)> = vec![];
    let mut candidates: Vec<(NodeId, AnchorRef, Point)> = vec![];
    for id in &st.selection.objects {
        let Some(pd) = st.doc.node(*id).and_then(|n| n.path_data()) else { continue };
        let set = st.selection.anchors.get(id);
        for (si, ai, a) in pd.anchors() {
            let picked = set.is_none_or(|set| set.contains(&(si, ai)));
            if picked {
                selected.push((*id, (si, ai), a.p));
            } else {
                candidates.push((*id, (si, ai), a.p));
            }
        }
    }
    if selected.is_empty() {
        return Ok(json!({ "changed": false }));
    }
    let change: Option<(NodeId, AnchorRef)> = if grow {
        // The unselected anchor nearest to any selected one.
        candidates
            .iter()
            .map(|(id, r, p)| (*id, *r, selected.iter().map(|(_, _, q)| q.distance(*p)).fold(f64::INFINITY, f64::min)))
            .min_by(|a, b| a.2.total_cmp(&b.2))
            .map(|(id, r, _)| (id, r))
    } else if selected.len() > 1 {
        // The selected anchor farthest from the selection's centre.
        let n = selected.len() as f64;
        let c = selected.iter().fold(Point::ZERO, |acc, (_, _, p)| Point::new(acc.x + p.x / n, acc.y + p.y / n));
        selected.iter().max_by(|a, b| a.2.distance(c).total_cmp(&b.2.distance(c))).map(|(id, r, _)| (*id, *r))
    } else {
        None
    };
    let Some((id, r)) = change else { return Ok(json!({ "changed": false })) };
    let all: BTreeSet<AnchorRef> = st.doc.node(id).and_then(|n| n.path_data()).map(all_anchors).unwrap_or_default().into_iter().collect();
    s.select(|_, sel| {
        sel.add(id);
        // A path selected whole has every anchor selected: shrinking starts from them all.
        let e = sel.anchors.entry(id).or_insert_with(|| all.clone());
        if grow {
            e.insert(r);
        } else {
            e.remove(&r);
            if e.is_empty() {
                sel.anchors.remove(&id);
                sel.remove(id);
            }
        }
    })?;
    Ok(json!({ "changed": true, "id": id.0, "anchor": [r.0, r.1] }))
}

/// Distance from `q` to the polyline `pts`.
fn dist_to_polyline(pts: &[Point], q: Point) -> f64 {
    if pts.len() == 1 {
        return pts[0].distance(q);
    }
    pts.windows(2)
        .map(|w| {
            let (a, b) = (w[0], w[1]);
            let ab = b - a;
            let l2 = ab.hypot2();
            let t = if l2 < 1e-12 { 0.0 } else { ((q - a).dot(ab) / l2).clamp(0.0, 1.0) };
            (a + ab * t).distance(q)
        })
        .fold(f64::INFINITY, f64::min)
}

fn anchors_touching(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "select.anchorsTouching";
    let pts: Vec<Point> = p
        .get("points")
        .and_then(Value::as_array)
        .ok_or_else(|| bad(C, "missing points"))?
        .iter()
        .map(|v| finite_numbers::<2>(v).map(|[x, y]| Point::new(x, y)).ok_or_else(|| bad(C, "each point must be [x, y]")))
        .collect::<Result<_>>()?;
    if pts.is_empty() {
        return Err(bad(C, "need at least one point"));
    }
    let tol = f64_or(p, "tolerance", 3.0).clamp(0.0, 1000.0);
    let mode = match str_param(p, "mode") {
        None | Some("set") => "set",
        Some("add") => "add",
        Some("toggle") => "toggle",
        Some(m) => return Err(bad(C, format!("unknown mode {m:?}"))),
    };
    let ids: Vec<NodeId> = if p.get("ids").is_some() {
        checked_ids_param(s, p, "ids", C)?
    } else {
        let st = s.doc()?;
        let selected: Vec<NodeId> =
            st.selection.objects.iter().copied().filter(|id| st.doc.node(*id).is_some_and(|n| n.path_data().is_some())).collect();
        if selected.is_empty() {
            let mut all = vec![];
            st.doc.walk(|n| {
                if matches!(n.kind, NodeKind::Path { guide: false, .. }) && st.doc.is_editable(n.id) {
                    all.push(n.id);
                }
            });
            all
        } else {
            selected
        }
    };
    let st = s.doc()?;
    let mut hits: Vec<(NodeId, BTreeSet<AnchorRef>)> = vec![];
    for id in ids {
        let Some(pd) = st.doc.node(id).and_then(|n| n.path_data()) else { continue };
        let set: BTreeSet<AnchorRef> = pd.anchors().filter(|(_, _, a)| dist_to_polyline(&pts, a.p) <= tol).map(|(si, ai, _)| (si, ai)).collect();
        if !set.is_empty() {
            hits.push((id, set));
        }
    }
    let count: usize = hits.iter().map(|(_, s)| s.len()).sum();
    s.select(|d, sel| {
        if mode == "set" {
            sel.clear();
        }
        for (id, set) in hits {
            let whole = sel.contains(id) && sel.partial(id).is_none();
            sel.add(id);
            sel.segments.remove(&id);
            let e = sel.anchors.entry(id).or_default();
            if whole && let Some(pd) = d.node(id).and_then(|n| n.path_data()) {
                e.extend(all_anchors(pd));
            }
            for r in set {
                if mode == "toggle" && e.contains(&r) {
                    e.remove(&r);
                } else {
                    e.insert(r);
                }
            }
            if e.is_empty() {
                sel.anchors.remove(&id);
                sel.remove(id);
            }
        }
    })?;
    Ok(json!({ "count": count }))
}
