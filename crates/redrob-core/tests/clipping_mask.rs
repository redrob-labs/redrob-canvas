//! Clipping masks (Photoshop's Ctrl+Alt+G), batch 4 item M1.

use redrob_core::{Command, Document, Editor, LayerId, Pixel, Rect};

/// A base layer with a 2x2 opaque square in the corner of a 4x4 canvas, and a full red layer
/// above it.
fn base_and_top() -> (Editor, LayerId, LayerId) {
    let mut editor = Editor::new(Document::new(4, 4).unwrap()).unwrap();
    let base = editor.document().active_layer_id();
    editor
        .execute(Command::SelectRectangle {
            rect: Rect { x: 0, y: 0, width: 2, height: 2 },
            mode: Default::default(),
        })
        .unwrap();
    editor.execute(Command::Fill { color: Pixel::rgba(0, 0, 255, 255) }).unwrap();
    editor.execute(Command::ClearSelection).unwrap();
    let top = LayerId::new();
    editor.execute(Command::AddLayer { id: top, name: "Top".into(), index: 1 }).unwrap();
    editor.execute(Command::Fill { color: Pixel::rgba(255, 0, 0, 255) }).unwrap();
    (editor, base, top)
}

fn pixel(editor: &Editor, x: usize, y: usize) -> [u8; 4] {
    let snapshot = editor.render_snapshot().unwrap();
    let p = snapshot.pixels();
    let at = (y * 4 + x) * 4;
    [p[at], p[at + 1], p[at + 2], p[at + 3]]
}

#[test]
fn a_clipped_layer_shows_only_over_its_base() {
    let (mut editor, _, top) = base_and_top();
    assert_eq!(pixel(&editor, 3, 3), [255, 0, 0, 255], "unclipped, red everywhere");
    editor.execute(Command::SetLayerClipped { id: top, clipped: true }).unwrap();
    assert_eq!(pixel(&editor, 0, 0), [255, 0, 0, 255], "red over the base");
    assert_eq!(pixel(&editor, 3, 3)[3], 0, "nothing where the base is empty");
}

#[test]
fn hiding_the_base_hides_what_is_clipped_to_it() {
    let (mut editor, base, top) = base_and_top();
    editor.execute(Command::SetLayerClipped { id: top, clipped: true }).unwrap();
    editor.execute(Command::SetLayerVisibility { id: base, visible: false }).unwrap();
    assert_eq!(pixel(&editor, 0, 0)[3], 0);
}

#[test]
fn releasing_and_undo_restore_the_plain_layer() {
    let (mut editor, _, top) = base_and_top();
    editor.execute(Command::SetLayerClipped { id: top, clipped: true }).unwrap();
    editor.undo().unwrap();
    assert!(!editor.document().layer(top).unwrap().is_clipped());
    assert_eq!(pixel(&editor, 3, 3), [255, 0, 0, 255]);
}

#[test]
fn the_flag_is_saved_only_when_set() {
    let (mut editor, _, top) = base_and_top();
    let plain = serde_json::to_string(editor.document()).unwrap();
    assert!(!plain.contains("\"clipped\""), "old documents stay byte-identical");
    editor.execute(Command::SetLayerClipped { id: top, clipped: true }).unwrap();
    let json = serde_json::to_string(editor.document()).unwrap();
    assert!(json.contains("\"clipped\":true"));
    let back: Document = serde_json::from_str(&json).unwrap();
    assert!(back.layer(top).unwrap().is_clipped());
}
