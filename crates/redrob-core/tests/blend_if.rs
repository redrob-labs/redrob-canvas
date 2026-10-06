//! Blend If (Layer Style > Blending Options), batch 4 item L6.

use redrob_core::{BlendIf, BlendRange, Command, CoreError, Document, Editor, LayerId, Pixel, Rect};

/// Bottom: left half black, right half white. Top: solid red.
fn setup() -> (Editor, LayerId) {
    let mut editor = Editor::new(Document::new(4, 1).unwrap()).unwrap();
    editor.execute(Command::Fill { color: Pixel::rgba(255, 255, 255, 255) }).unwrap();
    editor
        .execute(Command::SelectRectangle { rect: Rect { x: 0, y: 0, width: 2, height: 1 }, mode: Default::default() })
        .unwrap();
    editor.execute(Command::Fill { color: Pixel::rgba(0, 0, 0, 255) }).unwrap();
    editor.execute(Command::ClearSelection).unwrap();
    let top = LayerId::new();
    editor.execute(Command::AddLayer { id: top, name: "Top".into(), index: 1 }).unwrap();
    editor.execute(Command::Fill { color: Pixel::rgba(255, 0, 0, 255) }).unwrap();
    (editor, top)
}

fn rendered(editor: &Editor) -> Vec<[u8; 4]> {
    editor.render_snapshot().unwrap().pixels().chunks(4).map(|p| [p[0], p[1], p[2], p[3]]).collect()
}

#[test]
fn underlying_range_shows_the_layer_only_over_bright_pixels() {
    let (mut editor, top) = setup();
    let bright_only = BlendRange { black_low: 128, black_high: 128, white_low: 255, white_high: 255 };
    editor
        .execute(Command::SetLayerBlendIf { id: top, blend_if: Some(BlendIf { this_layer: BlendRange::ALL, underlying: bright_only }) })
        .unwrap();
    let px = rendered(&editor);
    assert_eq!(px[0], [0, 0, 0, 255], "over black the red is hidden");
    assert_eq!(px[3], [255, 0, 0, 255], "over white it shows");
}

#[test]
fn ranges_that_hide_nothing_clear_the_setting() {
    let (mut editor, top) = setup();
    editor
        .execute(Command::SetLayerBlendIf { id: top, blend_if: Some(BlendIf { this_layer: BlendRange::ALL, underlying: BlendRange::ALL }) })
        .unwrap();
    assert!(editor.document().layer(top).unwrap().blend_if().is_none());
}

#[test]
fn a_falling_range_is_refused() {
    let (mut editor, top) = setup();
    let bad = BlendRange { black_low: 200, black_high: 100, white_low: 255, white_high: 255 };
    assert!(matches!(
        editor.execute(Command::SetLayerBlendIf { id: top, blend_if: Some(BlendIf { this_layer: bad, underlying: BlendRange::ALL }) }),
        Err(CoreError::InvalidSemanticStyle)
    ));
}
