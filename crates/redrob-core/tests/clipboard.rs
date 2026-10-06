//! Edit > Copy / Paste (Ctrl+C / Ctrl+V), batch 4 item H5. Engine side: copy out as RGBA, paste in
//! as a new layer.

use redrob_core::{Command, CoreError, Document, Editor, LayerId, Pixel, Rect};

fn filled() -> Editor {
    let mut editor = Editor::new(Document::new(4, 4).unwrap()).unwrap();
    editor.execute(Command::Fill { color: Pixel::rgba(50, 60, 70, 255) }).unwrap();
    editor
}

#[test]
fn copy_without_a_selection_takes_the_whole_layer() {
    let editor = filled();
    let (rect, pixels) = editor.document().copy_active_rgba().unwrap();
    assert_eq!(rect, Rect { x: 0, y: 0, width: 4, height: 4 });
    assert!(pixels.chunks(4).all(|p| p == [50, 60, 70, 255]));
}

#[test]
fn copy_is_cut_to_the_selection_box() {
    let mut editor = filled();
    editor
        .execute(Command::SelectRectangle {
            rect: Rect { x: 1, y: 2, width: 2, height: 1 },
            mode: Default::default(),
        })
        .unwrap();
    let (rect, pixels) = editor.document().copy_active_rgba().unwrap();
    assert_eq!(rect, Rect { x: 1, y: 2, width: 2, height: 1 });
    assert_eq!(pixels.len(), 8);
}

#[test]
fn paste_adds_a_layer_above_with_the_pixels_clipped_to_the_canvas() {
    let mut editor = filled();
    let base = editor.document().active_layer_id();
    let id = LayerId::new();
    let block = [1_u8, 2, 3, 255].repeat(9);
    editor
        .execute(Command::PasteLayer {
            id,
            name: "Pasted".into(),
            rect: Rect { x: 2, y: 2, width: 3, height: 3 },
            pixels: block,
        })
        .unwrap();
    let doc = editor.document();
    assert_eq!(doc.active_layer_id(), id);
    let order: Vec<LayerId> = doc.nodes().iter().map(|n| n.id()).collect();
    assert_eq!(order, vec![base, id]);
    let pixels = doc.layer(id).unwrap().pixels();
    let at = |x: usize, y: usize| &pixels[(y * 4 + x) * 4..(y * 4 + x) * 4 + 4];
    assert_eq!(at(2, 2), &[1, 2, 3, 255]);
    assert_eq!(at(3, 3), &[1, 2, 3, 255]);
    assert_eq!(at(1, 1), &[0, 0, 0, 0]);
    editor.undo().unwrap();
    assert!(editor.document().layer(id).is_none(), "one undo step");
}

#[test]
fn a_wrong_pixel_length_is_refused() {
    let mut editor = filled();
    let result = editor.execute(Command::PasteLayer {
        id: LayerId::new(),
        name: "Bad".into(),
        rect: Rect { x: 0, y: 0, width: 2, height: 2 },
        pixels: vec![0; 15],
    });
    assert!(matches!(result, Err(CoreError::InvalidBufferLength { .. })));
}
