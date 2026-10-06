//! P11. Adjustment layers: a filter held as a node, run on the stack below it at render time,
//! never written into anyone's pixels.

use redrob_core::{
    Command, CoreError, Editor, ExportOptions, FileFormat, Filter, LayerId, NodeContent, NodeKind,
    Pixel, export_document, load_project, save_project,
};

#[path = "common/canvas.rs"]
mod canvas;

/// Four opaque pixels, so source-over of the filtered copy is exactly the filtered copy.
fn opaque() -> Editor {
    canvas::editor(
        2,
        2,
        &[
            Pixel::rgba(10, 20, 30, 255),
            Pixel::rgba(200, 100, 50, 255),
            Pixel::rgba(0, 0, 0, 255),
            Pixel::rgba(255, 255, 255, 255),
        ],
    )
}

fn render(editor: &Editor) -> Vec<u8> {
    editor.render_snapshot().unwrap().pixels().to_vec()
}

fn add_invert(editor: &mut Editor) -> LayerId {
    let id = LayerId::new();
    editor
        .execute(Command::AddAdjustmentNode {
            id,
            name: "Invert".into(),
            parent: None,
            sibling_index: 1,
            filter: Filter::Invert,
        })
        .unwrap();
    id
}

#[test]
fn an_adjustment_renders_like_the_destructive_filter() {
    let mut adjusted = opaque();
    add_invert(&mut adjusted);

    let mut baked = opaque();
    baked
        .execute(Command::ApplyFilter {
            filter: Filter::Invert,
        })
        .unwrap();

    assert_eq!(
        render(&adjusted),
        render(&baked),
        "an invert adjustment must look exactly like inverting the layer"
    );
}

#[test]
fn an_adjustment_leaves_the_pixels_below_untouched() {
    let mut editor = opaque();
    let before = editor.document().layers()[0].pixels().to_vec();
    add_invert(&mut editor);
    assert_eq!(
        editor.document().layers()[0].pixels(),
        before.as_slice(),
        "non-destructive means the raster layer still holds its own pixels"
    );
    let adjustment = &editor.document().layers()[1];
    assert_eq!(adjustment.kind(), NodeKind::Adjustment);
    assert_eq!(
        adjustment.content().adjustment_filter(),
        Some(&Filter::Invert)
    );
}

#[test]
fn hiding_zero_opacity_and_undo_each_restore_the_original_render() {
    let mut editor = opaque();
    let original = render(&editor);
    let id = add_invert(&mut editor);
    assert_ne!(
        render(&editor),
        original,
        "the adjustment must reach the pixels"
    );

    editor
        .execute(Command::SetLayerVisibility { id, visible: false })
        .unwrap();
    assert_eq!(
        render(&editor),
        original,
        "a hidden adjustment must do nothing"
    );
    editor
        .execute(Command::SetLayerVisibility { id, visible: true })
        .unwrap();

    editor
        .execute(Command::SetLayerOpacity { id, opacity: 0.0 })
        .unwrap();
    assert_eq!(render(&editor), original, "opacity 0 is no adjustment");
    editor.undo().unwrap();
    assert_ne!(render(&editor), original);

    // Back past SetLayerVisibility x2 and the add itself.
    editor.undo().unwrap();
    editor.undo().unwrap();
    editor.undo().unwrap();
    assert_eq!(
        editor.document().layers().len(),
        1,
        "undo must remove the node"
    );
    assert_eq!(render(&editor), original);
}

#[test]
fn the_filter_can_be_replaced_and_only_on_an_adjustment() {
    let mut editor = opaque();
    let original = render(&editor);
    let id = add_invert(&mut editor);
    editor
        .execute(Command::SetAdjustmentFilter {
            id,
            filter: Filter::InvertLinear,
        })
        .unwrap();
    assert_eq!(
        editor
            .document()
            .layer(id)
            .unwrap()
            .content()
            .adjustment_filter(),
        Some(&Filter::InvertLinear)
    );
    assert_ne!(render(&editor), original);

    let raster = editor.document().layers()[0].id();
    let refused = editor.execute(Command::SetAdjustmentFilter {
        id: raster,
        filter: Filter::Invert,
    });
    assert!(
        matches!(
            refused,
            Err(CoreError::UnsupportedNodeContent(NodeKind::Raster))
        ),
        "a raster layer is not an adjustment: {refused:?}"
    );
}

#[test]
fn a_precision_the_filter_cannot_run_at_is_refused_up_front() {
    // `color_enhance` is 8-bit only. Adding it is fine at 8-bit; moving the document to 16-bit
    // afterwards must be refused, not left to fail every later render.
    let mut editor = opaque();
    editor
        .execute(Command::AddAdjustmentNode {
            id: LayerId::new(),
            name: "Enhance".into(),
            parent: None,
            sibling_index: 1,
            filter: Filter::ColorEnhance,
        })
        .unwrap();
    let to_u16: Command = serde_json::from_value(serde_json::json!({
        "type": "set_document_precision",
        "precision": "u16"
    }))
    .unwrap();
    let refused = editor.execute(to_u16);
    assert!(
        matches!(refused, Err(CoreError::FilterPrecisionUnsupported(_))),
        "got {refused:?}"
    );
    assert!(
        editor.render_snapshot().is_ok(),
        "the document must still render"
    );
}

#[test]
fn an_adjustment_survives_the_project_file() {
    let mut editor = opaque();
    let id = add_invert(&mut editor);
    let loaded = load_project(&save_project(editor.document()).unwrap()).unwrap();
    let NodeContent::Adjustment { filter } = loaded.layer(id).unwrap().content() else {
        panic!("the adjustment changed kind after a round trip")
    };
    assert_eq!(**filter, Filter::Invert);
}

#[test]
fn layered_formats_refuse_an_adjustment_instead_of_dropping_it() {
    let mut editor = opaque();
    add_invert(&mut editor);
    for format in [
        FileFormat::Ora,
        FileFormat::Psd,
        FileFormat::Kra,
        FileFormat::Xcf,
        FileFormat::Svg,
    ] {
        let result = export_document(editor.document(), format, &ExportOptions::default());
        assert!(
            result.is_err(),
            "{format:?} exported an adjustment layer it cannot represent"
        );
    }
}

#[test]
fn a_flat_export_bakes_the_adjustment() {
    // PNG is the flattened render, so the adjustment is in it -- that is the picture the user sees.
    let mut adjusted = opaque();
    add_invert(&mut adjusted);
    let mut baked = opaque();
    baked
        .execute(Command::ApplyFilter {
            filter: Filter::Invert,
        })
        .unwrap();
    let options = ExportOptions::default();
    assert_eq!(
        export_document(adjusted.document(), FileFormat::Png, &options)
            .unwrap()
            .bytes(),
        export_document(baked.document(), FileFormat::Png, &options)
            .unwrap()
            .bytes(),
    );
}
