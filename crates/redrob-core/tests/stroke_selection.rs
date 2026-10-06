//! Edit > Stroke along the selection edge, batch 4 item M5.

use redrob_core::{Command, CoreError, Document, Editor, Pixel, Rect, StrokeLocation};

fn selected_square(location: StrokeLocation, width: f32) -> Vec<u8> {
    let mut editor = Editor::new(Document::new(20, 20).unwrap()).unwrap();
    editor
        .execute(Command::SelectRectangle { rect: Rect { x: 5, y: 5, width: 10, height: 10 }, mode: Default::default() })
        .unwrap();
    editor
        .execute(Command::StrokeSelection { width, color: Pixel::rgba(255, 0, 0, 255), location })
        .unwrap();
    let doc = editor.document();
    doc.layer(doc.active_layer_id()).unwrap().pixels().to_vec()
}

fn alpha(pixels: &[u8], x: usize, y: usize) -> u8 {
    pixels[(y * 20 + x) * 4 + 3]
}

#[test]
fn inside_paints_only_inside_the_edge() {
    let p = selected_square(StrokeLocation::Inside, 2.0);
    assert_eq!(alpha(&p, 5, 10), 255, "first pixel inside");
    assert_eq!(alpha(&p, 6, 10), 255, "second pixel inside");
    assert_eq!(alpha(&p, 10, 10), 0, "the middle stays empty");
    assert_eq!(alpha(&p, 4, 10), 0, "nothing outside");
}

#[test]
fn outside_paints_only_outside_the_edge() {
    let p = selected_square(StrokeLocation::Outside, 2.0);
    assert_eq!(alpha(&p, 4, 10), 255);
    assert_eq!(alpha(&p, 3, 10), 255);
    assert_eq!(alpha(&p, 5, 10), 0);
    assert_eq!(alpha(&p, 1, 10), 0);
}

#[test]
fn center_straddles_the_edge() {
    let p = selected_square(StrokeLocation::Center, 2.0);
    assert_eq!(alpha(&p, 4, 10), 255);
    assert_eq!(alpha(&p, 5, 10), 255);
    assert_eq!(alpha(&p, 10, 10), 0);
}

#[test]
fn no_selection_is_refused() {
    let mut editor = Editor::new(Document::new(8, 8).unwrap()).unwrap();
    let result = editor.execute(Command::StrokeSelection {
        width: 2.0,
        color: Pixel::rgba(0, 0, 0, 255),
        location: StrokeLocation::Center,
    });
    assert!(matches!(result, Err(CoreError::NoSelection)));
}
