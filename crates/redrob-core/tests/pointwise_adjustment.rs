//! M8: an adjustment layer whose filter is pointwise re-renders over the damaged box only, and the
//! result must be exactly the full render's.

use redrob_core::{BrushPoint, Command, Document, Editor, Filter, LayerId, Pixel};

fn levels() -> Filter {
    serde_json::from_value(serde_json::json!({
        "kind": "levels", "input_black": 20, "input_white": 230, "gamma": 1.4,
        "output_black": 0, "output_white": 255
    }))
    .expect("levels JSON matches the filter")
}

fn blur() -> Filter {
    serde_json::from_value(serde_json::json!({ "kind": "gaussian_blur", "sigma": 2.0 }))
        .expect("blur JSON matches the filter")
}

fn painted_under(filter: Filter) -> Editor {
    let mut editor = Editor::new(Document::new(48, 32).unwrap()).unwrap();
    editor.execute(Command::Fill { color: Pixel::rgba(90, 140, 200, 255) }).unwrap();
    let base = editor.document().active_layer_id();
    editor
        .execute(Command::AddAdjustmentNode {
            id: LayerId::new(),
            name: "Adj".into(),
            parent: None,
            sibling_index: 1,
            filter,
        })
        .unwrap();
    // Render once so the next render can reuse the projection and bound itself to the stroke.
    editor.render_snapshot().unwrap();
    editor.execute(Command::SetActiveLayer { id: base }).unwrap();
    editor
        .execute(Command::BrushStroke {
            points: vec![BrushPoint::new(10.0, 10.0, 1.0), BrushPoint::new(20.0, 14.0, 1.0)],
            color: Pixel::rgba(250, 30, 30, 255),
            size: 4.0,
            opacity: 1.0,
            settings: Default::default(),
            tip: None,
            pipe: Vec::new(),
        })
        .unwrap();
    editor
}

#[test]
fn a_bounded_render_under_levels_equals_a_full_one() {
    let editor = painted_under(levels());
    let bounded = editor.render_snapshot().unwrap().pixels().to_vec();
    let frame = editor.document().current_frame_id();
    let full = editor.render_frame_snapshot(frame).unwrap().pixels().to_vec();
    assert_eq!(bounded, full);
}

#[test]
fn a_blur_adjustment_still_renders_in_full() {
    let editor = painted_under(blur());
    let bounded = editor.render_snapshot().unwrap().pixels().to_vec();
    let frame = editor.document().current_frame_id();
    let full = editor.render_frame_snapshot(frame).unwrap().pixels().to_vec();
    assert_eq!(bounded, full);
}
