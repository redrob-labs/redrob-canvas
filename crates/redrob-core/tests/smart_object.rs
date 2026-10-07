//! Smart objects, batch 4 item L4: transforms re-render from the original pixels.

use redrob_core::{Affine2D, Command, CoreError, Document, Editor, Pixel, Rect, SamplingMode};

fn dotted() -> Editor {
    let mut editor = Editor::new(Document::new(16, 16).unwrap()).unwrap();
    editor
        .execute(Command::SelectRectangle {
            rect: Rect {
                x: 6,
                y: 6,
                width: 3,
                height: 3,
            },
            mode: Default::default(),
        })
        .unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(250, 40, 40, 255),
        })
        .unwrap();
    editor.execute(Command::ClearSelection).unwrap();
    editor
}

fn shift(dx: f32) -> Command {
    Command::TransformActive {
        transform: Affine2D::new(1.0, 0.0, 0.0, 1.0, dx, 0.0),
        sampling: SamplingMode::Bilinear,
    }
}

fn pixels(editor: &Editor) -> Vec<u8> {
    let doc = editor.document();
    doc.layer(doc.active_layer_id()).unwrap().pixels().to_vec()
}

#[test]
fn two_half_pixel_moves_equal_one_whole_pixel_move() {
    let mut smart = dotted();
    let id = smart.document().active_layer_id();
    smart.execute(Command::ConvertToSmartObject { id }).unwrap();
    smart.execute(shift(0.5)).unwrap();
    smart.execute(shift(0.5)).unwrap();

    let mut once = dotted();
    once.execute(shift(1.0)).unwrap();
    assert_eq!(
        pixels(&smart),
        pixels(&once),
        "no blur builds up from the two resamples"
    );

    let mut plain = dotted();
    plain.execute(shift(0.5)).unwrap();
    plain.execute(shift(0.5)).unwrap();
    assert_ne!(pixels(&plain), pixels(&once), "an ordinary layer does blur");
}

#[test]
fn painting_a_smart_object_needs_rasterize() {
    let mut editor = dotted();
    let id = editor.document().active_layer_id();
    editor
        .execute(Command::ConvertToSmartObject { id })
        .unwrap();
    assert!(matches!(
        editor.execute(Command::Fill {
            color: Pixel::rgba(0, 0, 0, 255)
        }),
        Err(CoreError::LayerLocked { .. })
    ));
    editor
        .execute(Command::RasterizeSmartObject { id })
        .unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(0, 0, 0, 255),
        })
        .unwrap();
    assert!(!editor.document().layer(id).unwrap().is_smart_object());
}

#[test]
fn undo_restores_the_earlier_transform() {
    let mut editor = dotted();
    let id = editor.document().active_layer_id();
    editor
        .execute(Command::ConvertToSmartObject { id })
        .unwrap();
    let before = pixels(&editor);
    editor.execute(shift(3.0)).unwrap();
    editor.undo().unwrap();
    assert_eq!(pixels(&editor), before);
    editor.execute(shift(1.0)).unwrap();
    let mut once = dotted();
    once.execute(shift(1.0)).unwrap();
    assert_eq!(
        pixels(&editor),
        pixels(&once),
        "the undone move is not part of the composition"
    );
}

// ---- U4: non-affine edits re-render from the source too ----

fn perspective() -> Command {
    Command::PerspectiveActive {
        corners: [(1.0, 0.0), (15.0, 2.0), (14.0, 15.0), (0.0, 13.0)],
        sampling: SamplingMode::Bilinear,
    }
}

#[test]
fn a_perspective_warp_is_allowed_on_a_smart_object_and_stays_editable() {
    let mut smart = dotted();
    let id = smart.document().active_layer_id();
    smart.execute(Command::ConvertToSmartObject { id }).unwrap();
    smart.execute(perspective()).unwrap();
    assert!(smart.document().layer(id).unwrap().is_smart_object());

    // Same result as the same warp on an ordinary copy: one render from the source.
    let mut plain = dotted();
    plain.execute(perspective()).unwrap();
    assert_eq!(pixels(&smart), pixels(&plain));
}

#[test]
fn a_move_after_a_warp_replays_the_warp_from_the_source() {
    // Warp, then two half-pixel moves. The moves fold into one transform after the warp, so the
    // result equals the warp and then ONE whole-pixel move on an ordinary copy: no blur builds up.
    let mut smart = dotted();
    let id = smart.document().active_layer_id();
    smart.execute(Command::ConvertToSmartObject { id }).unwrap();
    smart.execute(perspective()).unwrap();
    smart.execute(shift(0.5)).unwrap();
    smart.execute(shift(0.5)).unwrap();

    let mut replay = dotted();
    replay.execute(perspective()).unwrap();
    replay.execute(shift(1.0)).unwrap();
    assert_eq!(
        pixels(&smart),
        pixels(&replay),
        "source -> warp -> one composed move"
    );
}

#[test]
fn undo_of_a_smart_warp_restores_the_previous_render() {
    let mut smart = dotted();
    let id = smart.document().active_layer_id();
    smart.execute(Command::ConvertToSmartObject { id }).unwrap();
    let before = pixels(&smart);
    smart.execute(perspective()).unwrap();
    assert_ne!(pixels(&smart), before);
    smart.undo().unwrap();
    assert_eq!(pixels(&smart), before);
    // And the warp list went back too: a move now is the composed affine path again.
    smart.execute(shift(1.0)).unwrap();
    let mut once = dotted();
    once.execute(shift(1.0)).unwrap();
    assert_eq!(pixels(&smart), pixels(&once));
}

#[test]
fn a_smart_object_with_warps_round_trips_through_json() {
    let mut smart = dotted();
    let id = smart.document().active_layer_id();
    smart.execute(Command::ConvertToSmartObject { id }).unwrap();
    smart.execute(perspective()).unwrap();
    let json = serde_json::to_string(smart.document()).unwrap();
    assert!(json.contains("\"warps\""));
    let back: Document = serde_json::from_str(&json).unwrap();
    assert_eq!(&back, smart.document());

    // A smart object without warps writes no "warps" key, as before U4.
    let mut plain = dotted();
    let id = plain.document().active_layer_id();
    plain.execute(Command::ConvertToSmartObject { id }).unwrap();
    assert!(
        !serde_json::to_string(plain.document())
            .unwrap()
            .contains("\"warps\"")
    );
}

// ---- U5: smart filters ----

fn invert() -> Command {
    Command::ApplyFilter {
        filter: redrob_core::Filter::Invert,
    }
}

#[test]
fn a_filter_on_a_smart_object_becomes_a_smart_filter() {
    let mut smart = dotted();
    let id = smart.document().active_layer_id();
    smart.execute(Command::ConvertToSmartObject { id }).unwrap();
    smart.execute(invert()).unwrap();
    let filters = smart.document().smart_filters(id).unwrap();
    assert_eq!(filters.len(), 1);
    assert!(filters[0].visible);

    let mut plain = dotted();
    plain.execute(invert()).unwrap();
    assert_eq!(
        pixels(&smart),
        pixels(&plain),
        "rendered the same as a plain invert"
    );
}

#[test]
fn hiding_or_removing_a_smart_filter_brings_the_source_back() {
    let mut smart = dotted();
    let id = smart.document().active_layer_id();
    let original = pixels(&smart);
    smart.execute(Command::ConvertToSmartObject { id }).unwrap();
    smart.execute(invert()).unwrap();
    assert_ne!(pixels(&smart), original);

    let mut hidden = smart.document().smart_filters(id).unwrap().to_vec();
    hidden[0].visible = false;
    smart
        .execute(Command::SetSmartFilters { filters: hidden })
        .unwrap();
    assert_eq!(pixels(&smart), original, "a hidden filter is skipped");
    assert_eq!(
        smart.document().smart_filters(id).unwrap().len(),
        1,
        "but kept"
    );

    smart
        .execute(Command::SetSmartFilters {
            filters: Vec::new(),
        })
        .unwrap();
    assert_eq!(pixels(&smart), original);
    assert!(smart.document().smart_filters(id).unwrap().is_empty());
}

#[test]
fn a_move_after_a_smart_filter_moves_first_and_filters_after() {
    // Photoshop order: the transform applies to the source, the filters to the result.
    let mut smart = dotted();
    let id = smart.document().active_layer_id();
    smart.execute(Command::ConvertToSmartObject { id }).unwrap();
    smart.execute(invert()).unwrap();
    smart.execute(shift(1.0)).unwrap();

    let mut plain = dotted();
    plain.execute(shift(1.0)).unwrap();
    plain.execute(invert()).unwrap();
    assert_eq!(pixels(&smart), pixels(&plain));
    assert_eq!(smart.document().smart_filters(id).unwrap().len(), 1);
}

#[test]
fn smart_filters_are_refused_on_an_ordinary_layer() {
    let mut plain = dotted();
    assert!(
        plain
            .execute(Command::SetSmartFilters {
                filters: Vec::new()
            })
            .is_err()
    );
}
