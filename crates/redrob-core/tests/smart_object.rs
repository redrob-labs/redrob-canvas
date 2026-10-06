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
