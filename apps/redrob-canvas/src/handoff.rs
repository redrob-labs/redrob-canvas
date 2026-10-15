// SPDX-License-Identifier: MIT OR Apache-2.0

//! Cross-mode hand-offs. The first: pixels from any mode traced into paths (`redrob_ai::vectorize`)
//! and inserted into the Vector document as a group named "Vectorized", switching the shell to
//! Vector mode. Pixel coordinates become document units 1:1, placed at the artboard's origin.

use std::sync::Arc;

use redrob_ai::vectorize::{TraceResult, TracedPath};
use vectorcraft_doc::{Appearance, Document, Node, NodeId, color::Color, color::Paint};
use vectorcraft_geom::Affine;

use crate::shell::{Mode, ShellApp};

/// The name of the group the traced paths go into.
pub const GROUP_NAME: &str = "Vectorized";

/// The appearance a traced path paints with: its colour as a fill, or as a stroke of the traced
/// width when the preset made strokes (`TracedPath::stroke`, crafts/vectorcraft/crates/trace/
/// src/lib.rs L452). `Appearance::basic(fill, stroke, width)` is
/// crafts/vectorcraft/crates/doc/src/appearance.rs L751.
pub fn appearance_for(path: &TracedPath) -> Appearance {
    let [r, g, b] = path.color;
    let color = Paint::solid(Color::rgb8(r, g, b));
    match path.stroke {
        Some(width) => Appearance::basic(Paint::None, color, width),
        None => Appearance::basic(color, Paint::None, 0.0),
    }
}

/// Builds one path node per traced path (`Node::path(id, PathData, Appearance)`,
/// crafts/vectorcraft/crates/doc/src/node.rs L864), moved by `origin`, inside a group named
/// [`GROUP_NAME`], and inserts the group at the top of `layer` (`Document::insert`,
/// crafts/vectorcraft/crates/doc/src/lib.rs L1002). Returns the group's id and the path ids,
/// bottom-most first, in the trace's paint order.
pub fn trace_to_nodes(
    doc: &mut Document,
    result: &TraceResult,
    origin: (f64, f64),
    layer: Option<NodeId>,
) -> Result<(NodeId, Vec<NodeId>), vectorcraft_doc::DocError> {
    let shift = Affine::translate(origin);
    let mut ids = Vec::with_capacity(result.paths.len());
    let mut children = Vec::with_capacity(result.paths.len());
    for traced in &result.paths {
        let id = doc.alloc_id();
        let path = traced.path.transformed(shift);
        children.push(Arc::new(Node::path(id, path, appearance_for(traced))));
        ids.push(id);
    }
    let group_id = doc.alloc_id();
    let mut group = Node::group(group_id, children);
    group.name = Some(GROUP_NAME.to_owned());
    doc.insert(layer, usize::MAX, group)?;
    Ok((group_id, ids))
}

/// Puts `result` into the Vector editor's active document (creating a `size` document when none
/// is open) and selects the new group. The shell switches to Vector mode.
pub fn deliver_trace(
    shell: &mut ShellApp,
    result: &TraceResult,
    size: (u32, u32),
) -> Result<usize, String> {
    let count = result.paths.len();
    let app = shell.vector_app();
    if app.session.active().is_none() {
        // `file.new {width, height}` (crafts/vectorcraft/crates/engine/src/cmd/docinfo.rs L388
        // uses it this way); `sync_views` aligns the views with the documents
        // (crafts/vectorcraft/crates/ui-egui/src/lib.rs L570).
        app.run(
            "file.new",
            serde_json::json!({ "width": f64::from(size.0), "height": f64::from(size.1) }),
        )?;
        app.sync_views();
    }
    let origin = app
        .session
        .active()
        .and_then(|st| {
            let index = app.view().map(|v| v.artboard).unwrap_or(0);
            st.doc
                .artboards
                .get(index)
                .or_else(|| st.doc.artboards.first())
                .map(|a| (a.rect.x0, a.rect.y0))
        })
        .unwrap_or((0.0, 0.0));
    app.session
        .edit("Vectorize", |doc, sel| {
            let layer = doc.default_layer();
            let (group, _) = trace_to_nodes(doc, result, origin, layer)?;
            sel.set([group]);
            Ok(())
        })
        .map_err(|e| e.to_string())?;
    shell.set_mode(Mode::Vector);
    Ok(count)
}

/// Snapshot the current mode's pixels, trace them with `preset` and hand the paths to Vector
/// mode. Synchronous (the panel's card runs the trace on the runtime instead and calls
/// [`deliver_trace`] with the result); the agent's `vectorize` tool and tests use this one.
pub fn vectorize_to_vector(shell: &mut ShellApp, preset: &str) -> Result<usize, String> {
    let image = shell
        .with_bridge(|bridge| bridge.snapshot())
        .ok_or_else(|| "open a document first".to_string())?;
    let size = (image.width, image.height);
    let result = redrob_ai::vectorize::vectorize(&image, preset).map_err(|e| e.to_string())?;
    deliver_trace(shell, &result, size)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use vectorcraft_geom::{PathData, Point, Rect, shapes};

    fn traced(x: f64, color: [u8; 3], stroke: Option<f64>) -> TracedPath {
        TracedPath {
            path: shapes::rectangle(Rect::new(x, 0.0, x + 10.0, 10.0)),
            color,
            pixels: 100,
            stroke,
            shape: None,
            gradient: None,
            square_caps: false,
        }
    }

    #[test]
    fn handoff_builds_one_node_per_traced_path_in_a_named_group() {
        let result = TraceResult {
            paths: vec![
                traced(0.0, [255, 0, 0], None),
                traced(20.0, [0, 255, 0], None),
                traced(40.0, [0, 0, 255], Some(2.0)),
            ],
            palette: vec![[255, 0, 0], [0, 255, 0], [0, 0, 255]],
            parents: Vec::new(),
        };
        let mut doc = Document::new(100.0, 100.0);
        let layer = doc.default_layer();
        let (group, ids) = trace_to_nodes(&mut doc, &result, (5.0, 7.0), layer).unwrap();
        assert_eq!(ids.len(), 3);
        let group_node = doc.node(group).unwrap();
        assert_eq!(group_node.name.as_deref(), Some(GROUP_NAME));
        let children = group_node.children().unwrap();
        assert_eq!(children.len(), 3);
        for (child, id) in children.iter().zip(&ids) {
            assert_eq!(child.id, *id);
        }
        // Coordinates are shifted by the origin, 1 px = 1 unit.
        let first: &PathData = children[0].path_data().unwrap();
        let bounds = first.control_bounds().unwrap();
        assert_eq!(bounds.origin(), Point::new(5.0, 7.0));
        assert_eq!(bounds.width(), 10.0);
        // Fills carry the colour; a stroked path has no fill.
        assert!(children[0].appearance.fill().is_some());
        let stroked = &children[2].appearance;
        assert!(matches!(stroked.fill_paint(), Paint::None));
    }

    #[test]
    fn an_empty_trace_still_makes_the_group() {
        let mut doc = Document::new(10.0, 10.0);
        let (group, ids) =
            trace_to_nodes(&mut doc, &TraceResult::default(), (0.0, 0.0), None).unwrap();
        assert!(ids.is_empty());
        assert!(doc.node(group).is_some());
    }

    #[test]
    fn appearance_follows_stroke_or_fill() {
        let filled = appearance_for(&traced(0.0, [1, 2, 3], None));
        assert!(filled.fill().is_some());
        let stroked = appearance_for(&traced(0.0, [1, 2, 3], Some(3.0)));
        assert!(matches!(stroked.fill_paint(), Paint::None));
    }
}
