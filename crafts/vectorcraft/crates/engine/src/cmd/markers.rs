//! Object › Objects to Marker: the selected objects' outlines become a custom marker of this
//! document, offered beside the built-in arrowheads (`stroke.set {startMarker|midMarker|endMarker}`
//! and the Stroke panel's marker menus). `marker.list` names them, `marker.delete` removes one.

use redrob_vector_doc::{CustomMarker, NodeKind};
use redrob_vector_geom::{Affine, BezPath, PathData, Shape};
use serde_json::{Value, json};

use super::edit::selected_roots;
use super::path::paths_in;
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "object.objectsToMarker",
            "Objects to Marker",
            ["Object"],
            None,
            "{name?: the marker's name (default Marker 1, 2…), ids?, remove?: false (take the objects out of the document once they are a marker)} turn the selected objects' outlines into a custom marker of this document: their right edge is the tip, they are scaled so their longer side spans the head's box (4 head units), and the stroke's paint fills them. Use it with stroke.set startMarker|midMarker|endMarker → {name}",
            has_selection,
            objects_to_marker
        ),
        cmd!(query "marker.list", "Markers", [], None, "{} → {markers: [names]} the document's custom markers", has_doc, list),
        cmd!(
            "marker.delete",
            "Delete Marker",
            [],
            None,
            "{name} remove a custom marker; strokes using it keep a copy of its art → {removed}",
            has_doc,
            delete
        ),
    ]
}

fn objects_to_marker(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.objectsToMarker";
    let roots = if p.get("ids").is_some() { targets(s, p)? } else { selected_roots(s)? };
    if roots.is_empty() {
        return Err(bad(C, "select the objects to make a marker of"));
    }
    let (outline, bounds) = {
        let d = &s.doc()?.doc;
        let mut bp = BezPath::new();
        for id in paths_in(d, &roots) {
            let Some(n) = d.node(id) else { continue };
            let Some(path) = n.path_data() else { continue };
            if matches!(n.kind, NodeKind::Path { guide: true, .. }) {
                continue;
            }
            bp.extend(path.to_bezpath().elements().iter().copied());
        }
        if bp.elements().is_empty() {
            return Err(bad(C, "the selection has no paths (expand text, shapes and strokes first)"));
        }
        let b = bp.bounding_box();
        if !(b.width() > 0.0 || b.height() > 0.0) {
            return Err(bad(C, "the selection has no size"));
        }
        (bp, b)
    };
    // Into head units: the right edge at the tip (local x 0), x running back, y centred.
    let k = 4.0 / bounds.width().max(bounds.height()).max(1e-9);
    let cy = bounds.center().y;
    let local = Affine::new([-k, 0.0, 0.0, k, bounds.x1 * k, -cy * k]) * outline;
    let name = match str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => n.to_string(),
        None => {
            let d = &s.doc()?.doc;
            (1..).map(|i| format!("Marker {i}")).find(|n| !d.markers.iter().any(|m| m.name == *n)).unwrap_or_else(|| "Marker".into())
        }
    };
    let remove = bool_or(p, "remove", false);
    s.edit("Objects to Marker", |d, sel| {
        let marker = CustomMarker { name: name.clone(), path: PathData::from_bezpath(&local) };
        match d.markers.iter_mut().find(|m| m.name == name) {
            Some(m) => *m = marker,
            None => d.markers.push(marker),
        }
        if remove {
            for id in &roots {
                d.remove(*id)?;
            }
            sel.clear();
        }
        Ok(())
    })?;
    Ok(json!({ "name": name }))
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let d = &s.doc()?.doc;
    Ok(json!({ "markers": d.markers.iter().map(|m| m.name.clone()).collect::<Vec<_>>() }))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "marker.delete";
    let name = str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(C, "missing `name`"))?.to_string();
    if !s.doc()?.doc.markers.iter().any(|m| m.name == name) {
        return Err(bad(C, format!("no marker named `{name}`")));
    }
    s.edit("Delete Marker", |d, _| {
        d.markers.retain(|m| m.name != name);
        Ok(())
    })?;
    Ok(json!({ "removed": name }))
}
