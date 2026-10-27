//! Linked live objects from the vector editing reference: Dynamic Offset (`path.dynamicOffset`: the
//! Offset Path live effect, whose distance the node tool's knot drags), Linked Offset
//! (`path.linkedOffset`: an offset object that follows its source path) and Fill Between Paths
//! (`path.fillBetween`: a filled path bounded by the selected paths that follows them). The links
//! live in the objects' own data ([`redrob_vector_tools::effectknots`]); [`refresh`] runs after
//! every edit and recomputes an object whose sources changed.

use redrob_vector_doc::{Appearance, AppearanceItem, Document, FillLayer, Node, NodeId, NodeKind};
use redrob_vector_geom::{Anchor, PathData, SubPath};
use redrob_vector_pathops::{Join, offset_path};
use redrob_vector_tools::effectknots::{FILL_BETWEEN, LINKED_OFFSET, fill_between_of, linked_offset_of};
use serde_json::{Value, json};

use super::draw2::ids_json;
use super::edit::selected_roots;
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "path.dynamicOffset",
            "Dynamic Offset",
            ["Object", "Path"],
            None,
            "{offset?: 10 (pt; negative insets), ids?|id?} give each selected object a live Offset Path effect whose distance the node tool's knot drags on the canvas (Effect › Path › Offset Path edits it numerically; removing the effect restores the original) → {ids}",
            has_selection,
            dynamic_offset
        ),
        cmd!(
            "path.linkedOffset",
            "Linked Offset",
            ["Object", "Path"],
            None,
            "{distance?: 10 (pt; negative insets), ids?|id?} make, above each selected path, an offset object that stays linked to it: editing the source re-makes the offset, and the node tool's knot on the offset object sets its distance (path.linkedOffset.set); path.unlinkLive makes it a plain path → {ids: the offset objects}",
            has_selection,
            linked_offset
        ),
        cmd!(
            "path.linkedOffset.set",
            "Linked Offset Distance",
            [],
            None,
            "{id, distance} set a linked offset object's distance (the knot drag) → {id, distance}",
            has_doc,
            linked_offset_set
        ),
        cmd!(
            "path.fillBetween",
            "Fill Between Paths",
            ["Object", "Path"],
            None,
            "{ids?: [two or more paths; default: the selection]} make a filled path over the region bounded by the paths (each path's run joined to the next one's nearest end, closed) that stays linked to them: editing a source re-makes the fill; path.unlinkLive makes it a plain path → {id}",
            has_multi,
            fill_between
        ),
        cmd!(
            "path.unlinkLive",
            "Unlink Live Object",
            ["Object", "Path"],
            None,
            "{ids?|id?} linked offset and fill-between objects among the selection keep their geometry and stop following their sources → {changed}",
            has_selection,
            unlink_live
        ),
    ]
}

fn dynamic_offset(s: &mut Session, p: &Value) -> Result<Value> {
    let offset = f64_or(p, "offset", 10.0);
    if !offset.is_finite() {
        return Err(bad("path.dynamicOffset", "offset must be a number"));
    }
    let ids = checked_ids_or_selection(s, p, "path.dynamicOffset")?;
    let out = s.execute(
        "effect.apply",
        &json!({ "effect": "path.offsetPath", "params": { "offset": offset }, "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "item": null }),
    )?;
    Ok(json!({ "ids": out.get("ids").cloned().unwrap_or(Value::Null) }))
}

/// The geometry of path or compound path `id`, as one path.
fn geometry(d: &Document, id: NodeId) -> Option<PathData> {
    match &d.node(id)?.kind {
        NodeKind::Path { path, .. } => Some(path.clone()),
        NodeKind::Compound { children, .. } => {
            Some(PathData::new(children.iter().filter_map(|c| c.path_data()).flat_map(|p| p.subpaths.iter().cloned()).collect()))
        }
        _ => None,
    }
}

/// The offset of `pd` by `distance` (round joins, as the Offset Path effect's default).
fn offset_geometry(pd: &PathData, distance: f64) -> PathData {
    if distance.abs() < 1e-9 { pd.clone() } else { offset_path(pd, distance, Join::Round, 4.0) }
}

/// A new path node above `source` with `pd`, painted like the source, linked by `key` = `value`.
fn linked_node(d: &mut Document, source: NodeId, pd: PathData, key: &str, value: String, name: &str) -> Result<NodeId> {
    let src = d.node(source).ok_or(EngineError::NoNode(source))?;
    let appearance: Appearance = (*src.appearance).clone();
    let (parent, idx, _) = d.position(source).ok_or(EngineError::NoNode(source))?;
    let mut n = Node::path(d.alloc_id(), pd, appearance);
    n.name = Some(name.to_string());
    n.edit_attrs(|a| a.data.push((key.to_string(), value)));
    let id = n.id;
    d.insert(parent, idx + 1, n)?;
    Ok(id)
}

fn linked_offset(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.linkedOffset";
    let distance = f64_or(p, "distance", 10.0);
    if !distance.is_finite() {
        return Err(bad(C, "distance must be a number"));
    }
    let ids = if p.get("ids").is_some() || p.get("id").is_some() { targets(s, p)? } else { selected_roots(s)? };
    let out = s.edit("Linked Offset", |d, sel| {
        let mut made = vec![];
        for id in &ids {
            let Some(pd) = geometry(d, *id) else { continue };
            let off = offset_geometry(&pd, distance);
            made.push(linked_node(d, *id, off, LINKED_OFFSET, format!("{} {}", id.0, distance), "Linked Offset")?);
        }
        if made.is_empty() {
            return Err(bad(C, "select a path"));
        }
        sel.set(made.iter().copied());
        Ok(made)
    })?;
    Ok(ids_json(&out))
}

fn linked_offset_set(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.linkedOffset.set";
    let id = id_param(p, "id").ok_or_else(|| bad(C, "missing id"))?;
    let distance = f64_req(p, "distance", C)?;
    if !distance.is_finite() {
        return Err(bad(C, "distance must be a number"));
    }
    let (source, _) = s.doc()?.doc.node(id).and_then(linked_offset_of).ok_or_else(|| bad(C, "not a linked offset object"))?;
    s.edit("Linked Offset Distance", |d, _| {
        let pd = geometry(d, source).ok_or(EngineError::NoNode(source))?;
        let off = offset_geometry(&pd, distance);
        let n = d.node_mut(id).ok_or(EngineError::NoNode(id))?;
        if let NodeKind::Path { path, live, .. } = &mut n.kind {
            *path = off;
            *live = None;
        }
        n.edit_attrs(|a| {
            a.data.retain(|(k, _)| k != LINKED_OFFSET);
            a.data.push((LINKED_OFFSET.to_string(), format!("{} {}", source.0, distance)));
        });
        Ok(())
    })?;
    Ok(json!({ "id": id.0, "distance": distance }))
}

/// The closed region bounded by `paths`: each path's anchors in turn, every next path joined at
/// the end nearest to where the run stands (reversed when its end is the nearer), then closed. A
/// closed source contributes its outline from its first anchor.
fn region_between(paths: &[PathData]) -> Option<PathData> {
    let mut anchors: Vec<Anchor> = vec![];
    for pd in paths {
        for sp in &pd.subpaths {
            if sp.anchors.is_empty() {
                continue;
            }
            let mut run: Vec<Anchor> = sp.anchors.clone();
            if sp.closed && run.len() > 1 {
                // Back to the start, so the outline is whole.
                run.push(run[0]);
            }
            if let Some(last) = anchors.last() {
                let (first, end) = (run[0].p, run[run.len() - 1].p);
                if last.p.distance(end) < last.p.distance(first) {
                    run.reverse();
                    for a in &mut run {
                        std::mem::swap(&mut a.h_in, &mut a.h_out);
                    }
                }
            }
            anchors.extend(run);
        }
    }
    if anchors.len() < 3 {
        return None;
    }
    // The joins between runs are straight: the handles at both ends of a join retract.
    Some(PathData::single(SubPath::new(anchors, true)))
}

fn fill_between(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.fillBetween";
    let ids = checked_ids_or_selection(s, p, C)?;
    let st = s.doc()?;
    let sources: Vec<NodeId> = ids.iter().copied().filter(|id| geometry(&st.doc, *id).is_some()).collect();
    if sources.len() < 2 {
        return Err(bad(C, "select at least two paths"));
    }
    let fill = s.paint.fill.clone();
    let out = s.edit("Fill Between Paths", |d, sel| {
        let paths: Vec<PathData> = sources.iter().filter_map(|id| geometry(d, *id)).collect();
        let region = region_between(&paths).ok_or_else(|| bad(C, "the paths bound no region"))?;
        let top = *sources.last().ok_or_else(|| bad(C, "select at least two paths"))?;
        let paint = if fill.is_none() { redrob_vector_color::Paint::solid(redrob_vector_color::Color::gray(0.5)) } else { fill.clone() };
        let appearance = Appearance { items: vec![AppearanceItem::Fill(FillLayer::new(paint))], ..Appearance::default() };
        let (parent, idx, _) = d.position(top).ok_or(EngineError::NoNode(top))?;
        let mut n = Node::path(d.alloc_id(), region, appearance);
        n.name = Some("Fill Between Paths".into());
        let value = sources.iter().map(|i| i.0.to_string()).collect::<Vec<_>>().join(" ");
        n.edit_attrs(|a| a.data.push((FILL_BETWEEN.to_string(), value)));
        let id = n.id;
        // Below the paths it fills between (the bottom-most source), so their strokes stay on top.
        let bottom = sources.iter().filter_map(|s| d.position(*s).filter(|(p, _, _)| *p == parent).map(|(_, i, _)| i)).min().unwrap_or(idx + 1);
        d.insert(parent, bottom, n)?;
        sel.set([id]);
        Ok(id)
    })?;
    Ok(json!({ "id": out.0 }))
}

fn unlink_live(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = if p.get("ids").is_some() || p.get("id").is_some() { targets(s, p)? } else { selected_roots(s)? };
    let changed = s.edit("Unlink Live Object", |d, _| {
        let mut n = 0;
        for id in &ids {
            if let Some(node) = d.node_mut(*id)
                && (linked_offset_of(node).is_some() || fill_between_of(node).is_some())
            {
                node.edit_attrs(|a| a.data.retain(|(k, _)| k != LINKED_OFFSET && k != FILL_BETWEEN));
                n += 1;
            }
        }
        Ok(n)
    })?;
    Ok(json!({ "changed": changed }))
}

/// Linked offset and fill-between objects follow their sources: after an edit, one whose sources
/// changed (or that is new) is re-made from them; one whose source is gone keeps its geometry and
/// stops following.
pub(crate) fn refresh(before: &Document, d: &mut Document) {
    let mut offsets: Vec<(NodeId, NodeId, f64)> = vec![];
    let mut fills: Vec<(NodeId, Vec<NodeId>)> = vec![];
    d.walk(|n| {
        if let Some((src, dist)) = linked_offset_of(n) {
            offsets.push((n.id, src, dist));
        }
        if let Some(srcs) = fill_between_of(n) {
            fills.push((n.id, srcs));
        }
    });
    let changed = |d: &Document, id: NodeId| before.node_uncached(id) != d.node(id);
    for (id, src, dist) in offsets {
        if src == id || d.node(src).is_none() {
            if let Some(n) = d.node_mut(id) {
                n.edit_attrs(|a| a.data.retain(|(k, _)| k != LINKED_OFFSET));
            }
            continue;
        }
        if before.node_uncached(id).is_some() && !changed(d, src) {
            continue;
        }
        let Some(pd) = geometry(d, src) else { continue };
        let off = offset_geometry(&pd, dist);
        if let Some(NodeKind::Path { path, .. }) = d.node_mut(id).map(|n| &mut n.kind) {
            *path = off;
        }
    }
    for (id, srcs) in fills {
        if srcs.iter().any(|s| *s == id || d.node(*s).is_none()) {
            if let Some(n) = d.node_mut(id) {
                n.edit_attrs(|a| a.data.retain(|(k, _)| k != FILL_BETWEEN));
            }
            continue;
        }
        if before.node_uncached(id).is_some() && !srcs.iter().any(|s| changed(d, *s)) {
            continue;
        }
        let paths: Vec<PathData> = srcs.iter().filter_map(|s| geometry(d, *s)).collect();
        let Some(region) = region_between(&paths) else { continue };
        if let Some(NodeKind::Path { path, .. }) = d.node_mut(id).map(|n| &mut n.kind) {
            *path = region;
        }
    }
}
