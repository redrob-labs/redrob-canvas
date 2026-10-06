//! Edit > Content-Aware Fill (PatchMatch), batch 4 item M10.

use redrob_core::{Command, CoreError, Document, Editor, Pixel, Rect};

/// Vertical stripes, 3 px black / 3 px white, with a selected hole in the middle.
fn striped_with_hole() -> Editor {
    let mut editor = Editor::new(Document::new(48, 32).unwrap()).unwrap();
    editor.execute(Command::Fill { color: Pixel::rgba(255, 255, 255, 255) }).unwrap();
    for x in (0..48).step_by(6) {
        editor
            .execute(Command::SelectRectangle { rect: Rect { x, y: 0, width: 3, height: 32 }, mode: Default::default() })
            .unwrap();
        editor.execute(Command::Fill { color: Pixel::rgba(0, 0, 0, 255) }).unwrap();
    }
    editor
        .execute(Command::SelectRectangle { rect: Rect { x: 18, y: 10, width: 10, height: 10 }, mode: Default::default() })
        .unwrap();
    editor
}

fn hole_pixels(editor: &Editor) -> Vec<[u8; 4]> {
    let doc = editor.document();
    let p = doc.layer(doc.active_layer_id()).unwrap().pixels();
    let mut out = Vec::new();
    for y in 10..20 {
        for x in 18..28 {
            let at = (y * 48 + x) * 4;
            out.push([p[at], p[at + 1], p[at + 2], p[at + 3]]);
        }
    }
    out
}

#[test]
fn the_hole_gets_texture_not_a_grey_average() {
    let mut editor = striped_with_hole();
    editor.execute(Command::ContentAwareFill).unwrap();
    let pixels = hole_pixels(&editor);
    let crisp = pixels.iter().filter(|p| p[0] < 60 || p[0] > 195).count();
    assert!(crisp * 10 >= pixels.len() * 7, "{crisp} of {} pixels are near black or white", pixels.len());
}

#[test]
fn the_fill_is_deterministic_and_one_undo_step() {
    let mut a = striped_with_hole();
    let mut b = striped_with_hole();
    a.execute(Command::ContentAwareFill).unwrap();
    b.execute(Command::ContentAwareFill).unwrap();
    assert_eq!(hole_pixels(&a), hole_pixels(&b));
    let before = {
        let fresh = striped_with_hole();
        hole_pixels(&fresh)
    };
    a.undo().unwrap();
    assert_eq!(hole_pixels(&a), before);
}

#[test]
fn it_needs_a_selection() {
    let mut editor = Editor::new(Document::new(8, 8).unwrap()).unwrap();
    assert!(matches!(editor.execute(Command::ContentAwareFill), Err(CoreError::NoSelection)));
}
