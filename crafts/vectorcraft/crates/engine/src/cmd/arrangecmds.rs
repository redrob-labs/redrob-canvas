//! Path and arrange operations from the vector editing reference: Cut Path (`path.cutPath`: the
//! bottom paths' strokes cut where the top path crosses them), Split Path (`path.splitNonOverlapping`:
//! break apart that keeps holes with the subpath holding them), and the Arrange commands that place
//! several selected objects (`object.arrange.grid`, `object.arrange.polar`,
//! `object.arrange.removeOverlaps`).

use kurbo::{CubicBez, ParamCurve, Shape as _};
use redrob_vector_color::{Color, Paint};
use redrob_vector_doc::{AppearanceItem, Document, Node, NodeId, NodeKind, Scaling, StrokeLayer};
use redrob_vector_geom::{Affine, Anchor, PathData, Point, Rect, SubPath, Vec2};
use serde_json::{Value, json};

use super::draw2::{ids_json, replace_with_pieces};
use super::edit::selected_roots;
use super::pathops::Beside;
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "path.cutPath",
            "Cut Path",
            ["Object", "Path"],
            None,
            "{ids?: [the paths; the last in paint order (the top one) cuts the others; default: the selection]} the strokes of the paths below are cut where the top path crosses them: each becomes open stroked pieces without a fill; the top path is removed (as the Pathfinder's cut ops consume it) → {ids: the pieces}",
            has_multi,
            cut_path
        ),
        cmd!(
            "path.splitNonOverlapping",
            "Split Path",
            ["Object", "Path"],
            None,
            "{ids?|id?} break the selected compound paths and multi-subpath paths apart into one object per subpath, keeping the subpaths that lie inside another (holes, the counters of letters) with the subpath that holds them → {ids}",
            has_selection,
            split_non_overlapping
        ),
        cmd!(
            "object.arrange.grid",
            "Arrange in Grid",
            ["Object", "Arrange"],
            None,
            "{rows?: n, columns?: n (one given sizes the other; default: a square-ish grid), equalWidth?: true (every column as wide as the widest object; false: each column fitted to its own objects), equalHeight?: true, spacingX?: 0, spacingY?: 0 (pt, between cells), alignX?: left|center|right (center: where an object sits in a wider cell), alignY?: top|center|bottom, order?: rows (fill row by row, default)|columns, origin?: [x, y] (the grid's top-left; default: the selection's)} lay the selected objects out in rows × columns, in their paint order → {rows, columns, changed}",
            has_multi,
            arrange_grid
        ),
        cmd!(
            "object.arrange.polar",
            "Arrange in Circle",
            ["Object", "Arrange"],
            None,
            "{center?: [x, y], radiusX?: r, radiusY?: r (an ellipse; default: a circle round the selection's centre through its corners), ellipseId?: id (take the circle or ellipse from this object's bounds instead; the object stays where it is), startAngle?: 0 (degrees, counter-clockwise from 3 o'clock), endAngle?: 360 (the objects spread evenly between the two; 360 leaves a gap after the last), rotate?: false (each object turned to face along the circle), anchor?: center (the object's point put on the circle)|top|bottom|left|right} place the selected objects evenly round a circle or ellipse → {changed}",
            has_multi,
            arrange_polar
        ),
        cmd!(
            "object.arrange.removeOverlaps",
            "Remove Overlaps",
            ["Object", "Arrange"],
            None,
            "{gapX?: 0, gapY?: 0 (pt, the least distance kept between bounding boxes), ids?} move the selected objects apart by the least amount so that their bounding boxes no longer overlap (nor come closer than the gaps) → {changed, moved: [{id, dx, dy}]}",
            has_multi,
            remove_overlaps
        ),
    ]
}

/// The paths among `ids` (paths and compound paths; groups contribute the paths inside them), in
/// paint order (bottom first).
fn paths_in_paint_order(d: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    let mut out = vec![];
    for id in ids {
        if let Some(n) = d.node(*id) {
            n.walk(&mut |c: &Node| {
                if matches!(c.kind, NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. }) && !out.contains(&c.id) {
                    out.push(c.id);
                }
            });
        }
    }
    // Compound members stand for their compound.
    out.retain(|id| !d.parent_of(*id).and_then(|p| d.node(p)).is_some_and(|p| matches!(p.kind, NodeKind::Compound { .. })));
    let paths = d.index_paths(out.iter().copied());
    out.sort_by(|a, b| paths.get(a).cmp(&paths.get(b)));
    out
}

/// The geometry of path or compound path `id`.
fn geometry(d: &Document, id: NodeId) -> Option<PathData> {
    match d.node(id).map(|n| &n.kind)? {
        NodeKind::Path { path, .. } => Some(path.clone()),
        NodeKind::Compound { children, .. } => {
            Some(PathData::new(children.iter().filter_map(|c| c.path_data()).flat_map(|p| p.subpaths.iter().cloned()).collect()))
        }
        _ => None,
    }
}

/// The parameters `(ta, tb)` where cubic `a` crosses cubic `b`, by recursive subdivision of their
/// control boxes (a point within `tol` counts as a crossing).
fn curve_crossings(a: &CubicBez, b: &CubicBez, tol: f64) -> Vec<(f64, f64)> {
    fn go(a: &CubicBez, (a0, a1): (f64, f64), b: &CubicBez, (b0, b1): (f64, f64), tol: f64, depth: u32, out: &mut Vec<(f64, f64)>) {
        let ba = a.bounding_box().inflate(tol, tol);
        let bb = b.bounding_box();
        if !(ba.x0 <= bb.x1 && bb.x0 <= ba.x1 && ba.y0 <= bb.y1 && bb.y0 <= ba.y1) {
            return;
        }
        if depth == 0 || (ba.width().max(ba.height()) <= tol && bb.width().max(bb.height()) <= tol) {
            // Close enough: the midpoints stand for the crossing.
            let ta = 0.5 * (a0 + a1);
            let tb = 0.5 * (b0 + b1);
            if a.eval(ta).distance(b.eval(tb)) <= tol * 4.0 {
                out.push((ta, tb));
            }
            return;
        }
        let (a_l, a_r) = a.subdivide();
        let (b_l, b_r) = b.subdivide();
        let am = 0.5 * (a0 + a1);
        let bm = 0.5 * (b0 + b1);
        for (pa, ra) in [(&a_l, (a0, am)), (&a_r, (am, a1))] {
            for (pb, rb) in [(&b_l, (b0, bm)), (&b_r, (bm, b1))] {
                go(pa, ra, pb, rb, tol, depth - 1, out);
            }
        }
    }
    let mut out = vec![];
    go(a, (0.0, 1.0), b, (0.0, 1.0), tol, 22, &mut out);
    // Subdivision reports a crossing from several neighbouring leaves: keep one per place.
    out.sort_by(|x, y| x.0.total_cmp(&y.0));
    out.dedup_by(|x, y| (x.0 - y.0).abs() < 1e-4 && (x.1 - y.1).abs() < 1e-4);
    out
}

/// `sp` cut at `cuts` (`(segment, t)`), as open subpaths. A closed subpath cut once becomes one open
/// subpath; no cuts leave it as it is.
fn split_subpath(sp: &SubPath, mut cuts: Vec<(usize, f64)>) -> Vec<SubPath> {
    cuts.retain(|(i, t)| *i < sp.segment_count() && t.is_finite());
    cuts.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    cuts.dedup_by(|a, b| a.0 == b.0 && (a.1 - b.1).abs() < 1e-6);
    if cuts.is_empty() {
        return vec![sp.clone()];
    }
    // Every segment as its pieces between cuts, in order; `true` marks a piece that starts a new run.
    let mut pieces: Vec<(CubicBez, bool, bool)> = vec![]; // (curve, is_line, starts_run)
    for i in 0..sp.segment_count() {
        let c = sp.segment(i);
        let line = sp.segment_is_line(i);
        let mut ts: Vec<f64> = vec![0.0];
        ts.extend(cuts.iter().filter(|(k, _)| *k == i).map(|(_, t)| t.clamp(0.0, 1.0)));
        ts.push(1.0);
        ts.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        let starts_at_cut = cuts.iter().any(|(k, t)| *k == i && *t <= 1e-9);
        for (k, w) in ts.windows(2).enumerate() {
            if w[1] - w[0] < 1e-9 {
                continue;
            }
            pieces.push((c.subsegment(w[0]..w[1]), line, if k == 0 { starts_at_cut } else { true }));
        }
    }
    let mut runs: Vec<Vec<(CubicBez, bool)>> = vec![];
    for (c, line, starts) in pieces {
        if starts || runs.is_empty() {
            runs.push(vec![]);
        }
        if let Some(r) = runs.last_mut() {
            r.push((c, line));
        }
    }
    // A closed subpath's first run continues its last one (no cut at anchor 0).
    if sp.closed && runs.len() >= 2 && !cuts.iter().any(|(k, t)| *k == 0 && *t <= 1e-9) {
        let first = runs.remove(0);
        if let Some(last) = runs.last_mut() {
            last.extend(first);
        }
    }
    runs.into_iter()
        .map(|r| {
            let mut anchors: Vec<Anchor> = vec![];
            for (c, line) in r {
                let (h1, h2) = if line { (c.p0, c.p3) } else { (c.p1, c.p2) };
                match anchors.last_mut() {
                    Some(a) => a.h_out = h1,
                    None => anchors.push(Anchor { h_out: h1, ..Anchor::corner(c.p0) }),
                }
                anchors.push(Anchor { p: c.p3, h_in: h2, h_out: c.p3, ..Anchor::corner(c.p3) });
            }
            for a in &mut anchors {
                *a = Anchor::with_handles(a.p, a.h_in, a.h_out);
            }
            SubPath::new(anchors, false)
        })
        .filter(|s| s.anchors.len() >= 2)
        .collect()
}

/// `pd`'s subpaths cut wherever `cutter` crosses them, as open subpaths (one object per piece).
fn cut_by(pd: &PathData, cutter: &PathData, tol: f64) -> Option<Vec<PathData>> {
    let cutter_segs: Vec<CubicBez> = cutter.subpaths.iter().flat_map(|sp| (0..sp.segment_count()).map(move |i| sp.segment(i))).collect();
    let mut any = false;
    let mut out = vec![];
    for sp in &pd.subpaths {
        let mut cuts = vec![];
        for i in 0..sp.segment_count() {
            let c = sp.segment(i);
            let cb = c.bounding_box().inflate(tol, tol);
            for k in &cutter_segs {
                let kb = k.bounding_box();
                if !(cb.x0 <= kb.x1 && kb.x0 <= cb.x1 && cb.y0 <= kb.y1 && kb.y0 <= cb.y1) {
                    continue;
                }
                cuts.extend(curve_crossings(&c, k, tol).into_iter().map(|(t, _)| (i, t)));
            }
        }
        if cuts.is_empty() {
            out.push(sp.clone());
            continue;
        }
        any = true;
        out.extend(split_subpath(sp, cuts));
    }
    any.then(|| out.into_iter().map(PathData::single).collect())
}

fn cut_path(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.cutPath";
    let ids = checked_ids_or_selection(s, p, C)?;
    let st = s.doc()?;
    let ordered = paths_in_paint_order(&st.doc, &ids);
    let Some((&top, below)) = ordered.split_last() else {
        return Err(bad(C, "select the paths to cut and, above them, the cutting path"));
    };
    if below.is_empty() {
        return Err(bad(C, "select at least one path below the cutting path"));
    }
    let cutter = geometry(&st.doc, top).ok_or_else(|| bad(C, "the top object is not a path"))?;
    let scale = cutter.control_bounds().map_or(1.0, |b| b.width().max(b.height()).max(1.0));
    let tol = scale * 1e-6;
    let mut plan: Vec<(NodeId, Vec<PathData>)> = vec![];
    for id in below {
        let Some(pd) = geometry(&st.doc, *id) else {
            continue;
        };
        if let Some(pieces) = cut_by(&pd, &cutter, tol) {
            plan.push((*id, pieces));
        }
    }
    if plan.is_empty() {
        return Ok(json!({ "ids": [] }));
    }
    let out = s.edit("Cut Path", |d, sel| {
        let mut all = vec![];
        let mut cut = Beside::replacing(d, &plan.iter().map(|(id, _)| *id).collect::<Vec<_>>());
        for (id, pieces) in plan {
            all.extend(replace_with_pieces(d, &mut cut, id, pieces)?);
        }
        cut.finish(d)?;
        // Open stroked pieces: the fill goes, a path without a stroke gets a thin black one so
        // the cut shows.
        for id in &all {
            if let Some(n) = d.node_mut(*id) {
                let a = &mut *n.appearance;
                a.items.retain(|i| !matches!(i, AppearanceItem::Fill(_)));
                if a.stroke().is_none() {
                    a.items.push(AppearanceItem::Stroke(StrokeLayer::new(Paint::solid(Color::BLACK), 1.0)));
                }
                if let NodeKind::Path { live, .. } = &mut n.kind {
                    *live = None;
                }
            }
        }
        d.remove(top)?;
        sel.set(all.iter().copied());
        Ok(all)
    })?;
    Ok(ids_json(&out))
}

/// Does closed subpath `outer` hold `inner` (by `inner`'s first anchor, outer's winding)?
fn holds(outer: &SubPath, inner: &SubPath) -> bool {
    let Some(q) = inner.anchors.first().map(|a| a.p) else {
        return false;
    };
    let mut bp = redrob_vector_geom::BezPath::new();
    outer.to_bezpath_into(&mut bp);
    bp.winding(q) != 0
}

/// `pd`'s subpaths grouped so that a subpath inside another stays with it (nesting is kept by
/// area order: the biggest contours first take what they hold).
fn group_subpaths(pd: &PathData) -> Vec<PathData> {
    let mut order: Vec<usize> = (0..pd.subpaths.len()).collect();
    order.sort_by(|a, b| pd.subpaths[*b].signed_area().abs().total_cmp(&pd.subpaths[*a].signed_area().abs()));
    let mut owner: Vec<Option<usize>> = vec![None; pd.subpaths.len()];
    for (k, &i) in order.iter().enumerate() {
        if !pd.subpaths[i].closed {
            continue;
        }
        for &j in &order[k + 1..] {
            if owner[j].is_none() && holds(&pd.subpaths[i], &pd.subpaths[j]) {
                owner[j] = Some(i);
            }
        }
    }
    // Follow owners up to the root contour of each subpath.
    let root = |mut i: usize| {
        while let Some(o) = owner[i] {
            i = o;
        }
        i
    };
    let mut groups: Vec<(usize, Vec<SubPath>)> = vec![];
    for (i, sp) in pd.subpaths.iter().enumerate() {
        let r = root(i);
        match groups.iter_mut().find(|(g, _)| *g == r) {
            Some((_, v)) => v.push(sp.clone()),
            None => groups.push((r, vec![sp.clone()])),
        }
    }
    groups.into_iter().map(|(_, v)| PathData::new(v)).collect()
}

fn split_non_overlapping(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.splitNonOverlapping";
    let ids = checked_ids_or_selection(s, p, C)?;
    let st = s.doc()?;
    let mut plan: Vec<(NodeId, Vec<PathData>)> = vec![];
    for id in paths_in_paint_order(&st.doc, &ids) {
        let Some(pd) = geometry(&st.doc, id) else {
            continue;
        };
        if pd.subpaths.len() < 2 {
            continue;
        }
        let groups = group_subpaths(&pd);
        if groups.len() >= 2 {
            plan.push((id, groups));
        }
    }
    if plan.is_empty() {
        return Ok(json!({ "ids": [] }));
    }
    let out = s.edit("Split Path", |d, sel| {
        let mut all = vec![];
        let mut cut = Beside::replacing(d, &plan.iter().map(|(id, _)| *id).collect::<Vec<_>>());
        for (id, pieces) in plan {
            // A compound path's pieces become plain paths (a piece with holes keeps its fill rule).
            let rule = match d.node(id).map(|n| &n.kind) {
                Some(NodeKind::Compound { rule, .. } | NodeKind::Path { rule, .. }) => *rule,
                _ => redrob_vector_geom::FillRule::NonZero,
            };
            let ids = replace_with_pieces(d, &mut cut, id, pieces)?;
            for pid in &ids {
                if let Some(NodeKind::Path { rule: r, .. }) = d.node_mut(*pid).map(|n| &mut n.kind) {
                    *r = rule;
                }
            }
            all.extend(ids);
        }
        cut.finish(d)?;
        sel.set(all.iter().copied());
        Ok(all)
    })?;
    Ok(ids_json(&out))
}

/// The selected top-level objects with their visible bounds, in paint order.
fn boxed(s: &Session, ids: &[NodeId]) -> Vec<(NodeId, Rect)> {
    let Ok(st) = s.doc() else { return vec![] };
    let symbols = st.doc.symbol_boxes();
    ids.iter().filter_map(|id| Some((*id, st.doc.node(*id)?.visual_bounds_with(&symbols)?))).collect()
}

/// Move each object of `moves` by its vector, as one undo step → the number moved.
fn move_each(s: &mut Session, label: &str, moves: Vec<(NodeId, Vec2)>) -> Result<usize> {
    let moves: Vec<(NodeId, Vec2)> = moves.into_iter().filter(|(_, v)| v.hypot() > 1e-9).collect();
    if moves.is_empty() {
        return Ok(0);
    }
    s.edit(label, |d, _| {
        let mut n = 0;
        for (id, v) in &moves {
            if let Some(node) = d.node_mut(*id) {
                node.transform(Affine::translate(*v), Scaling::default());
                n += 1;
            }
        }
        Ok(n)
    })
}

fn align_frac(v: Option<&str>, lo: &str, hi: &str, cmd: &str, key: &str) -> Result<f64> {
    match v {
        None | Some("center") | Some("centre") | Some("middle") => Ok(0.5),
        Some(x) if x == lo => Ok(0.0),
        Some(x) if x == hi => Ok(1.0),
        Some(x) => Err(bad(cmd, format!("`{key}` must be {lo}, center or {hi}, not {x:?}"))),
    }
}

fn arrange_grid(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.arrange.grid";
    let ids = if p.get("ids").is_some() { checked_ids_param(s, p, "ids", C)? } else { selected_roots(s)? };
    let items = boxed(s, &ids);
    let n = items.len();
    if n < 2 {
        return Err(bad(C, "select at least two objects"));
    }
    let count = |k: &str| -> Result<Option<usize>> {
        match p.get(k).filter(|v| !v.is_null()) {
            None => Ok(None),
            Some(v) => v.as_u64().filter(|c| *c >= 1).map(|c| Some(c as usize)).ok_or_else(|| bad(C, format!("`{k}` must be a positive integer"))),
        }
    };
    let (rows, cols) = match (count("rows")?, count("columns")?) {
        (Some(r), Some(c)) => (r, c),
        (Some(r), None) => (r, n.div_ceil(r)),
        (None, Some(c)) => (n.div_ceil(c), c),
        (None, None) => {
            let c = (n as f64).sqrt().ceil() as usize;
            (n.div_ceil(c), c)
        }
    };
    if rows * cols < n {
        return Err(bad(C, format!("{rows} × {cols} cells can't hold {n} objects")));
    }
    let equal_w = bool_or(p, "equalWidth", true);
    let equal_h = bool_or(p, "equalHeight", true);
    let (gx, gy) = (f64_or(p, "spacingX", 0.0), f64_or(p, "spacingY", 0.0));
    let ax = align_frac(str_param(p, "alignX"), "left", "right", C, "alignX")?;
    let ay = align_frac(str_param(p, "alignY"), "top", "bottom", C, "alignY")?;
    let by_columns = match str_param(p, "order") {
        None | Some("rows") => false,
        Some("columns") => true,
        Some(o) => {
            return Err(bad(C, format!("`order` must be rows or columns, not {o:?}")));
        }
    };
    let union = items.iter().skip(1).fold(items[0].1, |r, (_, b)| r.union(*b));
    let origin = point_param(p, "origin").unwrap_or_else(|| Point::new(union.x0, union.y0));
    // Each object's cell.
    let cell_of = |i: usize| {
        if by_columns { (i % rows, i / rows) } else { (i / cols, i % cols) }
    };
    let max_w = items.iter().map(|(_, b)| b.width()).fold(0.0, f64::max);
    let max_h = items.iter().map(|(_, b)| b.height()).fold(0.0, f64::max);
    let mut col_w = vec![if equal_w { max_w } else { 0.0 }; cols];
    let mut row_h = vec![if equal_h { max_h } else { 0.0 }; rows];
    if !equal_w || !equal_h {
        for (i, (_, b)) in items.iter().enumerate() {
            let (r, c) = cell_of(i);
            if !equal_w {
                col_w[c] = col_w[c].max(b.width());
            }
            if !equal_h {
                row_h[r] = row_h[r].max(b.height());
            }
        }
    }
    let mut col_x = vec![origin.x; cols];
    for c in 1..cols {
        col_x[c] = col_x[c - 1] + col_w[c - 1] + gx;
    }
    let mut row_y = vec![origin.y; rows];
    for r in 1..rows {
        row_y[r] = row_y[r - 1] + row_h[r - 1] + gy;
    }
    let moves: Vec<(NodeId, Vec2)> = items
        .iter()
        .enumerate()
        .map(|(i, (id, b))| {
            let (r, c) = cell_of(i);
            let x = col_x[c] + (col_w[c] - b.width()) * ax;
            let y = row_y[r] + (row_h[r] - b.height()) * ay;
            (*id, Vec2::new(x - b.x0, y - b.y0))
        })
        .collect();
    let changed = move_each(s, "Arrange in Grid", moves)?;
    Ok(json!({ "rows": rows, "columns": cols, "changed": changed }))
}

fn arrange_polar(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.arrange.polar";
    let mut ids = if p.get("ids").is_some() { checked_ids_param(s, p, "ids", C)? } else { selected_roots(s)? };
    let ellipse = id_param(p, "ellipseId");
    if let Some(e) = ellipse {
        ids.retain(|id| *id != e);
    }
    let items = boxed(s, &ids);
    let n = items.len();
    if n < 1 {
        return Err(bad(C, "select the objects to arrange"));
    }
    let union = items.iter().skip(1).fold(items[0].1, |r, (_, b)| r.union(*b));
    let (center, rx, ry) = match ellipse {
        Some(e) => {
            let b = s.doc()?.doc.node(e).and_then(|n| n.geometric_bounds()).ok_or_else(|| bad(C, "no such ellipse object"))?;
            (b.center(), b.width() / 2.0, b.height() / 2.0)
        }
        None => {
            let c = point_param(p, "center").unwrap_or_else(|| union.center());
            let r = union.width().hypot(union.height()) / 2.0;
            let rx = f64_or(p, "radiusX", r);
            (c, rx, f64_or(p, "radiusY", rx))
        }
    };
    let start = f64_or(p, "startAngle", 0.0);
    let end = f64_or(p, "endAngle", 360.0);
    let full = ((end - start).abs() - 360.0).abs() < 1e-9;
    let step = if n == 1 {
        0.0
    } else if full {
        (end - start) / n as f64
    } else {
        (end - start) / (n - 1) as f64
    };
    let rotate = bool_or(p, "rotate", false);
    let (fx, fy) = match str_param(p, "anchor") {
        None | Some("center") => (0.5, 0.5),
        Some("top") => (0.5, 0.0),
        Some("bottom") => (0.5, 1.0),
        Some("left") => (0.0, 0.5),
        Some("right") => (1.0, 0.5),
        Some(a) => {
            return Err(bad(C, format!("`anchor` must be center, top, bottom, left or right, not {a:?}")));
        }
    };
    let mut plan: Vec<(NodeId, Affine)> = vec![];
    for (i, (id, b)) in items.iter().enumerate() {
        let ang = (start + step * i as f64).to_radians();
        // Counter-clockwise on screen: y is down.
        let target = Point::new(center.x + rx * ang.cos(), center.y - ry * ang.sin());
        let anchor = Point::new(b.x0 + b.width() * fx, b.y0 + b.height() * fy);
        let mut xf = Affine::translate(target - anchor);
        if rotate {
            // Face along the circle: the object's up turns outward.
            let turn = -(ang - std::f64::consts::FRAC_PI_2);
            xf = Affine::translate(target.to_vec2()) * Affine::rotate(turn) * Affine::translate(-target.to_vec2()) * xf;
        }
        plan.push((*id, xf));
    }
    let changed = s.edit("Arrange in Circle", |d, _| {
        let mut k = 0;
        for (id, xf) in &plan {
            if let Some(node) = d.node_mut(*id) {
                node.transform(*xf, Scaling::default());
                k += 1;
            }
        }
        Ok(k)
    })?;
    Ok(json!({ "changed": changed }))
}

fn remove_overlaps(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.arrange.removeOverlaps";
    let ids = if p.get("ids").is_some() { checked_ids_param(s, p, "ids", C)? } else { selected_roots(s)? };
    let items = boxed(s, &ids);
    if items.len() < 2 {
        return Err(bad(C, "select at least two objects"));
    }
    let (gx, gy) = (f64_or(p, "gapX", 0.0).max(0.0), f64_or(p, "gapY", 0.0).max(0.0));
    let mut boxes: Vec<Rect> = items.iter().map(|(_, b)| *b).collect();
    let n = boxes.len();
    // Push overlapping pairs apart along the axis that needs the smaller move, until no pair
    // overlaps (a bounded number of passes: each resolves what it finds).
    for _ in 0..64 {
        let mut moved = false;
        for i in 0..n {
            for j in (i + 1)..n {
                let (a, b) = (boxes[i], boxes[j]);
                let ox = (a.x1 + gx).min(b.x1 + gx) - a.x0.max(b.x0);
                let oy = (a.y1 + gy).min(b.y1 + gy) - a.y0.max(b.y0);
                if ox <= 1e-9 || oy <= 1e-9 {
                    continue;
                }
                moved = true;
                // Half the overlap each way, along the shorter overlap.
                if ox <= oy {
                    let dir = if a.center().x <= b.center().x { 1.0 } else { -1.0 };
                    boxes[i] = a + Vec2::new(-dir * ox / 2.0, 0.0);
                    boxes[j] = b + Vec2::new(dir * ox / 2.0, 0.0);
                } else {
                    let dir = if a.center().y <= b.center().y { 1.0 } else { -1.0 };
                    boxes[i] = a + Vec2::new(0.0, -dir * oy / 2.0);
                    boxes[j] = b + Vec2::new(0.0, dir * oy / 2.0);
                }
            }
        }
        if !moved {
            break;
        }
    }
    let moves: Vec<(NodeId, Vec2)> = items.iter().zip(&boxes).map(|((id, b0), b1)| (*id, Vec2::new(b1.x0 - b0.x0, b1.y0 - b0.y0))).collect();
    let report: Vec<Value> = moves.iter().filter(|(_, v)| v.hypot() > 1e-9).map(|(id, v)| json!({ "id": id.0, "dx": v.x, "dy": v.y })).collect();
    let changed = move_each(s, "Remove Overlaps", moves)?;
    Ok(json!({ "changed": changed, "moved": report }))
}
