//! XML Editor commands (`xml.newNode`, `xml.indent`, `xml.outdent`, `xml.move`, `path.setSvgData`):
//! the tree edits the XML Editor panel makes that no other command covers. Nodes are created as
//! empty groups or paths beside the selected one, indented into the sibling before them or
//! outdented beside their parent, moved before another node by dragging, and a path's geometry is
//! edited as SVG path data (`d`), rounded to a number of decimals when asked.

use redrob_vector_doc::{Node, NodeId, NodeKind};
use redrob_vector_geom::{BezPath, FillRule, PathData};
use serde_json::{Value, json};

use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "xml.newNode",
            "New Node",
            [],
            None,
            "{kind?: \"group\"|\"path\" (group), after?: id (default: the first selected object; the node goes right after it, or at the top of the first layer with nothing selected), inside?: bool (true: the first child of `after` when it is a group or layer)} create an empty node, as the XML Editor's New Node does → {id}",
            has_doc,
            new_node
        ),
        cmd!(
            "xml.indent",
            "Indent Node",
            [],
            None,
            "{id? (default: the first selected object)} move the node into the group or layer just before it (its last child) → {id}",
            has_selection,
            indent
        ),
        cmd!(
            "xml.outdent",
            "Outdent Node",
            [],
            None,
            "{id? (default: the first selected object)} move the node out of its group, right after it (a layer's direct children stay) → {id}",
            has_selection,
            outdent
        ),
        cmd!(
            "xml.move",
            "Move Node",
            [],
            None,
            "{id, before?: id (the node goes right before this one), into?: id (the node becomes the last child of this group or layer)} reorder the tree as dragging a row in the XML Editor does → {id}",
            has_doc,
            move_node
        ),
        cmd!(
            "path.setSvgData",
            "Set Path Data",
            [],
            None,
            "{id? (default: the first selected path), d?: SVG path data (M, L, C, Q, A, Z…), round?: decimals (0..6; rounds every coordinate of the path, or of `d` when given)} give the path the geometry `d` describes (document coordinates, points; a live shape becomes a plain path) → {id, d: the path data written}",
            has_selection,
            set_svg_data
        ),
    ]
}

fn first_selected(s: &Session) -> Option<NodeId> {
    s.doc().ok()?.selection.objects.first().copied()
}

fn is_container(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Group { .. } | NodeKind::Layer { .. })
}

fn new_node(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "xml.newNode";
    let kind = str_param(p, "kind").unwrap_or("group");
    let make = |id: NodeId| -> Result<Node> {
        Ok(Node::new(
            id,
            match kind {
                "group" => NodeKind::Group { children: vec![], clip: false },
                "path" => NodeKind::Path { path: PathData::new(vec![]), rule: FillRule::NonZero, live: None, clipping: false, guide: false },
                other => return Err(bad(C, format!("kind must be group or path, not `{other}`"))),
            },
        ))
    };
    let after = match p.get("after") {
        Some(v) if !v.is_null() => Some(checked_id(s, v, C)?),
        _ => first_selected(s),
    };
    let inside = bool_or(p, "inside", false);
    let id = s.edit("New Node", |d, sel| {
        let id = d.alloc_id();
        let n = make(id)?;
        match after {
            Some(a) if inside && d.node(a).is_some_and(is_container) => {
                d.insert(Some(a), 0, n)?;
            }
            Some(a) => {
                let (par, idx, _) = d.position(a).ok_or(crate::EngineError::NoNode(a))?;
                d.insert(par, idx + 1, n)?;
            }
            None => {
                let layer = d.layers.first().map(|l| l.id).ok_or_else(|| bad(C, "the document has no layer"))?;
                d.insert(Some(layer), usize::MAX, n)?;
            }
        }
        sel.set([id]);
        Ok(id)
    })?;
    Ok(json!({ "id": id.0 }))
}

fn target(s: &Session, p: &Value, cmd: &str) -> Result<NodeId> {
    match p.get("id") {
        Some(v) if !v.is_null() => checked_id(s, v, cmd),
        _ => first_selected(s).ok_or_else(|| bad(cmd, "nothing selected")),
    }
}

fn indent(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "xml.indent";
    let id = target(s, p, C)?;
    let d = &s.doc()?.doc;
    let (par, idx, _) = d.position(id).ok_or(crate::EngineError::NoNode(id))?;
    if idx == 0 {
        return Err(bad(C, "no node before it to move into"));
    }
    let prev = d.children(par).and_then(|c| c.get(idx - 1)).map(|n| n.id).ok_or_else(|| bad(C, "no node before it"))?;
    if !d.node(prev).is_some_and(is_container) {
        return Err(bad(C, "the node before it is not a group or layer"));
    }
    s.edit("Indent Node", |d, sel| {
        d.move_node(id, Some(prev), usize::MAX)?;
        sel.set([id]);
        Ok(())
    })?;
    Ok(json!({ "id": id.0 }))
}

fn outdent(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "xml.outdent";
    let id = target(s, p, C)?;
    let d = &s.doc()?.doc;
    let (par, _, _) = d.position(id).ok_or(crate::EngineError::NoNode(id))?;
    let parent = par.ok_or_else(|| bad(C, "a layer stays at the root"))?;
    if d.node(parent).is_some_and(|n| n.is_layer()) {
        return Err(bad(C, "the node is already directly in its layer"));
    }
    let (grand, pidx, _) = d.position(parent).ok_or(crate::EngineError::NoNode(parent))?;
    s.edit("Outdent Node", |d, sel| {
        d.move_node(id, grand, pidx + 1)?;
        sel.set([id]);
        Ok(())
    })?;
    Ok(json!({ "id": id.0 }))
}

fn move_node(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "xml.move";
    let id = checked_id(s, p.get("id").ok_or_else(|| bad(C, "missing id"))?, C)?;
    let before = p.get("before").filter(|v| !v.is_null()).map(|v| checked_id(s, v, C)).transpose()?;
    let into = p.get("into").filter(|v| !v.is_null()).map(|v| checked_id(s, v, C)).transpose()?;
    let d = &s.doc()?.doc;
    let (parent, index) = match (before, into) {
        (Some(b), _) => {
            let (par, idx, _) = d.position(b).ok_or(crate::EngineError::NoNode(b))?;
            // Leaving its own parent first shifts the siblings after it up by one.
            let own = d.position(id).ok_or(crate::EngineError::NoNode(id))?;
            let idx = if own.0 == par && own.1 < idx { idx - 1 } else { idx };
            (par, idx)
        }
        (None, Some(g)) => {
            if !d.node(g).is_some_and(is_container) {
                return Err(bad(C, "`into` must be a group or layer"));
            }
            (Some(g), usize::MAX)
        }
        (None, None) => return Err(bad(C, "give `before` or `into`")),
    };
    if parent.is_none() && !d.node(id).is_some_and(|n| n.is_layer()) {
        return Err(bad(C, "only layers sit at the root"));
    }
    if parent.is_some() && d.node(id).is_some_and(|n| n.is_layer()) {
        return Err(bad(C, "a layer stays at the root"));
    }
    s.edit("Move Node", |d, sel| {
        d.move_node(id, parent, index)?;
        sel.set([id]);
        Ok(())
    })?;
    Ok(json!({ "id": id.0 }))
}

/// `v` rounded to `decimals` places.
fn round_to(v: f64, decimals: u32) -> f64 {
    let k = 10f64.powi(decimals as i32);
    (v * k).round() / k
}

fn set_svg_data(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "path.setSvgData";
    let id = target(s, p, C)?;
    let round = p.get("round").and_then(Value::as_u64).map(|r| r.min(6) as u32);
    let current = match &s.doc()?.doc.node(id).ok_or(crate::EngineError::NoNode(id))?.kind {
        NodeKind::Path { path, .. } => path.clone(),
        _ => return Err(bad(C, "the node is not a path")),
    };
    let mut pd = match str_param(p, "d") {
        Some(d) => PathData::from_bezpath(&BezPath::from_svg(d).map_err(|e| bad(C, format!("path data: {e}")))?),
        None => current,
    };
    if let Some(r) = round {
        for sp in &mut pd.subpaths {
            for a in &mut sp.anchors {
                for q in [&mut a.p, &mut a.h_in, &mut a.h_out] {
                    q.x = round_to(q.x, r);
                    q.y = round_to(q.y, r);
                }
            }
        }
    }
    let written = pd.to_bezpath().to_svg();
    s.edit("Set Path Data", |d, sel| {
        if let Some(n) = d.node_mut(id)
            && let NodeKind::Path { path, live, .. } = &mut n.kind
        {
            *path = pd;
            *live = None;
        }
        sel.set([id]);
        Ok(())
    })?;
    Ok(json!({ "id": id.0, "d": written }))
}
