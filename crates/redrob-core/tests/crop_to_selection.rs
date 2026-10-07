//! Image > Crop to Selection and Edit > Clear outside selection, item A1.

use redrob_core::{Command, CoreError, Document, Editor, Pixel, Rect};

/// A 6x4 opaque red canvas with a 2x2 selection at (1, 1)..(3, 3).
fn selected() -> Editor {
    let mut editor = Editor::new(Document::new(6, 4).unwrap()).unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(200, 10, 10, 255),
        })
        .unwrap();
    editor
        .execute(Command::SelectRectangle {
            rect: Rect::new(1, 1, 2, 2),
            mode: Default::default(),
        })
        .unwrap();
    editor
}

fn alpha(editor: &Editor) -> Vec<u8> {
    let doc = editor.document();
    doc.layer(doc.active_layer_id())
        .unwrap()
        .pixels()
        .chunks_exact(4)
        .map(|p| p[3])
        .collect()
}

#[test]
fn bounds_cover_every_partly_selected_pixel() {
    let editor = selected();
    assert_eq!(
        editor.document().selection().bounds(),
        Some(Rect::new(1, 1, 2, 2))
    );
}

#[test]
fn bounds_are_none_without_a_selection() {
    let mut editor = selected();
    editor.execute(Command::ClearSelection).unwrap();
    assert_eq!(editor.document().selection().bounds(), None);
}

#[test]
fn crop_to_selection_crops_to_the_bounding_box_and_undoes() {
    let mut editor = selected();
    editor.execute(Command::CropToSelection).unwrap();
    assert_eq!(
        (editor.document().width(), editor.document().height()),
        (2, 2)
    );
    editor.undo().unwrap();
    assert_eq!(
        (editor.document().width(), editor.document().height()),
        (6, 4)
    );
}

#[test]
fn crop_to_selection_refuses_when_nothing_is_selected() {
    let mut editor = selected();
    editor.execute(Command::ClearSelection).unwrap();
    let err = editor.execute(Command::CropToSelection).unwrap_err();
    assert!(matches!(err, CoreError::NoSelection), "{err:?}");
    assert_eq!(editor.document().width(), 6);
}

#[test]
fn clear_outside_keeps_the_inside_and_the_selection() {
    let mut editor = selected();
    let mask_before = editor.document().selection().mask().to_vec();
    editor.execute(Command::ClearOutsideSelection).unwrap();
    let a = alpha(&editor);
    for y in 0..4 {
        for x in 0..6 {
            let inside = (1..3).contains(&x) && (1..3).contains(&y);
            assert_eq!(a[y * 6 + x], if inside { 255 } else { 0 }, "({x},{y})");
        }
    }
    assert_eq!(editor.document().selection().mask(), &mask_before[..]);
    editor.undo().unwrap();
    assert!(alpha(&editor).iter().all(|v| *v == 255));
}

#[test]
fn clear_outside_refuses_when_nothing_is_selected() {
    let mut editor = selected();
    editor.execute(Command::ClearSelection).unwrap();
    let err = editor.execute(Command::ClearOutsideSelection).unwrap_err();
    assert!(matches!(err, CoreError::NoSelection), "{err:?}");
    assert!(alpha(&editor).iter().all(|v| *v == 255));
}
