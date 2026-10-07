//! U3: the Move tool snaps the active layer's opaque edges to guides, so the engine reports them.

use redrob_core::precision::Precision;
use redrob_core::{Command, Document, Editor, Pixel, Rect};

fn editor_with_box() -> Editor {
    let mut editor = Editor::new(Document::new(40, 30).unwrap()).unwrap();
    editor
        .execute(Command::SelectRectangle {
            rect: Rect {
                x: 5,
                y: 7,
                width: 10,
                height: 4,
            },
            mode: Default::default(),
        })
        .unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(255, 0, 0, 255),
        })
        .unwrap();
    editor.execute(Command::ClearSelection).unwrap();
    editor
}

#[test]
fn the_active_layers_opaque_box_is_reported_exclusive() {
    let editor = editor_with_box();
    assert_eq!(
        editor.document().active_opaque_bounds(),
        Some((5, 7, 15, 11))
    );
}

#[test]
fn a_transparent_layer_has_no_box() {
    let editor = Editor::new(Document::new(40, 30).unwrap()).unwrap();
    assert_eq!(editor.document().active_opaque_bounds(), None);
}

#[test]
fn a_deep_document_reports_no_box_rather_than_a_wrong_one() {
    let mut editor = editor_with_box();
    editor
        .execute(Command::SetDocumentPrecision {
            precision: Precision::U16,
        })
        .unwrap();
    assert_eq!(editor.document().active_opaque_bounds(), None);
}
