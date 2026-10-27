//! Object › Clone: linked copies of any object (the SVG `<use>` element), see
//! [`redrob_vector_doc::CloneRef`]. A clone is an ordinary object whose kind is refreshed from its
//! original after every edit ([`refresh`], run by `Session::edit`), so the canvas, hit testing and
//! every writer treat it as the art it shows; the SVG writer writes `<use>`.

use redrob_vector_doc::{CloneRef, Document, Node, NodeId};
use redrob_vector_geom::Affine;
use serde_json::{Value, json};

use super::edit::{copy_above, selected_roots};
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "object.clone",
            "Create Clone",
            ["Object", "Clone"],
            Some("Alt+D"),
            "{ids?|id?} make a clone of each selected object (or of ids): a linked copy in place, just above the original, that follows every edit of the original (and moves with it) while taking transforms of its own; its paint follows the original's until the clone is painted itself. Deleting the original unlinks its clones → {ids: the clones}",
            has_selection,
            make
        ),
        cmd!(
            "object.unlinkClone",
            "Unlink Clone",
            ["Object", "Clone"],
            Some("Alt+Shift+D"),
            "{ids?|id?} make the selected clones (or ids) independent copies of what they show → {changed}",
            has_clone,
            unlink
        ),
        cmd!(
            "object.relinkClone",
            "Relink to Copied",
            ["Object", "Clone"],
            None,
            "{ids?|id?} link the selected clones (or ids) to the object last copied (Edit › Copy), which must still be in the document → {changed}",
            has_clone,
            relink
        ),
        cmd!(
            "object.selectOriginal",
            "Select Original",
            ["Object", "Clone"],
            Some("Cmd+Alt+D"),
            "{ids?|id?} select the originals of the selected clones (or ids) → {ids}",
            has_clone,
            select_original
        ),
    ]
}

/// A clone is selected.
fn has_clone(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    if st.selection.objects.iter().any(|id| st.doc.node(*id).is_some_and(|n| n.extras().clone_of.is_some())) {
        Ok(())
    } else {
        Err("select a clone".into())
    }
}

/// The objects `p` names, or the top-level selected objects.
fn root_targets(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    if p.get("ids").is_some_and(|v| !v.is_null()) || p.get("id").is_some_and(|v| !v.is_null()) { targets(s, p) } else { selected_roots(s) }
}

fn ids_json(ids: &[NodeId]) -> Value {
    json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() })
}

fn make(s: &mut Session, p: &Value) -> Result<Value> {
    let roots: Vec<NodeId> = {
        let d = &s.doc()?.doc;
        root_targets(s, p)?.into_iter().filter(|id| d.node(*id).is_some_and(|n| !n.is_layer())).collect()
    };
    if roots.is_empty() {
        return Err(EngineError::Other("nothing to clone".into()));
    }
    let ids = s.edit("Clone", |d, sel| {
        let out = copy_above(d, &roots, |_, original, c| {
            c.name = None;
            c.extras_mut().clone_of = Some(Box::new(CloneRef { original: original.id, xf: Affine::IDENTITY, own_paint: false }));
        })?;
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(ids_json(&ids))
}

/// The selected clones among `p`'s targets.
fn clones(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    let d = &s.doc()?.doc;
    Ok(targets(s, p)?.into_iter().filter(|id| d.node(*id).is_some_and(|n| n.extras().clone_of.is_some())).collect())
}

fn unlink(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = clones(s, p)?;
    if ids.is_empty() {
        return Err(EngineError::Other("select a clone".into()));
    }
    s.edit("Unlink Clone", |d, _| {
        for id in &ids {
            if let Some(n) = d.node_mut(*id) {
                n.extras_mut().clone_of = None;
            }
        }
        Ok(())
    })?;
    Ok(json!({ "changed": ids.len() }))
}

fn relink(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = clones(s, p)?;
    if ids.is_empty() {
        return Err(EngineError::Other("select a clone".into()));
    }
    let copied = s.clipboard.nodes.first().map(|n| n.id).ok_or_else(|| EngineError::Other("copy an object first (Edit › Copy)".into()))?;
    {
        let d = &s.doc()?.doc;
        if d.node(copied).is_none() {
            return Err(EngineError::Other("the copied object is no longer in this document".into()));
        }
        if ids.iter().any(|id| *id == copied || reaches(d, copied, *id)) {
            return Err(EngineError::Other("a clone can't be linked to itself".into()));
        }
    }
    s.edit("Relink Clone", |d, _| {
        for id in &ids {
            if let Some(n) = d.node_mut(*id)
                && let Some(c) = n.extras_if_any().and_then(|e| e.clone_of.as_deref_mut())
            {
                c.original = copied;
            }
        }
        Ok(())
    })?;
    Ok(json!({ "changed": ids.len() }))
}

fn select_original(s: &mut Session, p: &Value) -> Result<Value> {
    let originals: Vec<NodeId> = {
        let d = &s.doc()?.doc;
        clones(s, p)?
            .into_iter()
            .filter_map(|id| d.node(id)?.extras().clone_of.as_deref().map(|c| c.original))
            .filter(|o| d.node(*o).is_some())
            .collect()
    };
    if originals.is_empty() {
        return Err(EngineError::Other("select a clone".into()));
    }
    s.select(|_, sel| sel.set(originals.iter().copied()))?;
    Ok(ids_json(&originals))
}

/// Does following the clone links from `from` reach `target`?
fn reaches(d: &Document, from: NodeId, target: NodeId) -> bool {
    let mut cur = from;
    for _ in 0..64 {
        let Some(c) = d.node(cur).and_then(|n| n.extras().clone_of.as_deref()) else {
            return false;
        };
        if c.original == target {
            return true;
        }
        cur = c.original;
    }
    true
}

/// What a clone of `original` placed by `c` shows: the original mapped by the clone's transform
/// (strokes scale with it).
fn expected(d: &mut Document, original: &Node, c: &CloneRef) -> Node {
    // Fresh ids for a group's members: ids are unique in the document.
    let mut copy = d.reid(original);
    copy.transform(c.xf, true);
    copy
}

/// Clones follow their originals: after an edit, every clone whose original changed (or that is
/// new, or newly linked) takes the original's kind again, mapped by its own transform. A clone
/// painted while its original kept its paint keeps that paint from then on; a clone whose
/// original is gone (or that would link to itself) is unlinked.
pub(crate) fn refresh(before: &Document, d: &mut Document) {
    let mut clones: Vec<(NodeId, CloneRef)> = vec![];
    d.walk(|n| {
        if let Some(c) = n.extras().clone_of.as_deref() {
            clones.push((n.id, c.clone()));
        }
    });
    for (id, c) in clones {
        let original = d.node(c.original).cloned();
        let Some(original) = original.filter(|_| c.original != id && !reaches(d, c.original, id)) else {
            if let Some(n) = d.node_mut(id) {
                n.extras_mut().clone_of = None;
            }
            continue;
        };
        let Some(cur) = d.node(id).cloned() else {
            continue;
        };
        let before_original = before.node_uncached(c.original);
        let before_clone = before.node_uncached(id).filter(|b| b.extras().clone_of.as_deref() == Some(&c));
        let original_changed = before_original != Some(&original);
        let mut own = c.own_paint;
        if !own
            && let Some(b) = before_clone
            && b.appearance != cur.appearance
            && before_original.is_some_and(|o| o.appearance == original.appearance)
        {
            own = true;
        }
        if before_clone.is_some() && !original_changed && own == c.own_paint {
            continue;
        }
        let copy = expected(d, &original, &c);
        if let Some(n) = d.node_mut(id) {
            n.kind = copy.kind;
            n.bbox_angle = copy.bbox_angle;
            if !own {
                n.appearance = copy.appearance;
            }
            n.extras_mut().clone_of = Some(Box::new(CloneRef { own_paint: own, ..c }));
        }
    }
}
