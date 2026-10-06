//! Layer locks (Photoshop's lock transparent / pixels / position / all), batch 4 item M2.

use redrob_core::{Affine2D, Command, CoreError, Document, Editor, LayerLocks, Pixel, Rect, SamplingMode};

fn half_painted() -> Editor {
    // Left half opaque blue, right half transparent.
    let mut editor = Editor::new(Document::new(4, 2).unwrap()).unwrap();
    editor
        .execute(Command::SelectRectangle { rect: Rect { x: 0, y: 0, width: 2, height: 2 }, mode: Default::default() })
        .unwrap();
    editor.execute(Command::Fill { color: Pixel::rgba(0, 0, 255, 255) }).unwrap();
    editor.execute(Command::ClearSelection).unwrap();
    editor
}

fn lock(editor: &mut Editor, locks: LayerLocks) {
    let id = editor.document().active_layer_id();
    editor.execute(Command::SetLayerLocks { id, locks }).unwrap();
}

#[test]
fn locked_pixels_refuse_paint() {
    let mut editor = half_painted();
    lock(&mut editor, LayerLocks { pixels: true, ..LayerLocks::default() });
    let result = editor.execute(Command::Fill { color: Pixel::rgba(255, 0, 0, 255) });
    assert!(matches!(result, Err(CoreError::LayerLocked { what: "pixels", .. })));
}

#[test]
fn locked_transparency_only_recolours_what_is_there() {
    let mut editor = half_painted();
    lock(&mut editor, LayerLocks { transparent: true, ..LayerLocks::default() });
    editor.execute(Command::Fill { color: Pixel::rgba(255, 0, 0, 255) }).unwrap();
    let doc = editor.document();
    let pixels = doc.layer(doc.active_layer_id()).unwrap().pixels();
    assert_eq!(&pixels[0..4], &[255, 0, 0, 255], "opaque pixel recoloured");
    assert_eq!(pixels[3 * 4 + 3], 0, "transparent pixel stays transparent");
}

#[test]
fn locked_position_refuses_transforms() {
    let mut editor = half_painted();
    lock(&mut editor, LayerLocks { position: true, ..LayerLocks::default() });
    let result = editor.execute(Command::TransformActive { transform: Affine2D::IDENTITY, sampling: SamplingMode::Nearest });
    assert!(matches!(result, Err(CoreError::LayerLocked { what: "position", .. })));
    assert!(editor.execute(Command::FlipActive { horizontal: true, vertical: false }).is_err());
    // Painting is still allowed under a position lock.
    editor.execute(Command::Fill { color: Pixel::rgba(1, 2, 3, 255) }).unwrap();
}

#[test]
fn a_locked_layer_has_no_live_stroke() {
    let mut editor = half_painted();
    lock(&mut editor, LayerLocks { transparent: true, ..LayerLocks::default() });
    let begin = editor.begin_live_stroke(
        Pixel::rgba(0, 0, 0, 255),
        3.0,
        1.0,
        redrob_core::BrushSettings::default(),
        None,
        Vec::new(),
    );
    assert!(matches!(begin, Err(CoreError::LiveStrokeUnavailable)), "it commits on release instead");
}

#[test]
fn locks_are_saved_only_when_set() {
    let mut editor = half_painted();
    assert!(!serde_json::to_string(editor.document()).unwrap().contains("\"locks\""));
    lock(&mut editor, LayerLocks { position: true, ..LayerLocks::default() });
    let json = serde_json::to_string(editor.document()).unwrap();
    assert!(json.contains("\"locks\":{\"position\":true}"), "{json}");
}
