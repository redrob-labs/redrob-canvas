//! Layers panel thumbnails: a layer's own content, scaled to fit, as straight 8-bit RGBA.

use redrob_core::{
    Command, Document, Editor, LayerId, MAX_LAYER_THUMBNAIL_SIDE, Pixel, render_layer_thumbnail,
};

fn filled(width: u32, height: u32) -> (Editor, LayerId) {
    let mut editor = Editor::new(Document::new(width, height).unwrap()).unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(200, 40, 10, 255),
        })
        .unwrap();
    let id = editor.document().active_layer_id();
    (editor, id)
}

#[test]
fn a_wide_canvas_fits_the_longest_side_and_keeps_its_aspect() {
    let (editor, id) = filled(400, 200);
    let (w, h, pixels) = render_layer_thumbnail(editor.document(), id, 40)
        .unwrap()
        .unwrap();
    assert_eq!((w, h), (40, 20));
    assert_eq!(pixels.len(), 40 * 20 * 4);
    assert!(pixels.chunks(4).all(|p| p == [200, 40, 10, 255]));
}

#[test]
fn a_small_canvas_is_not_scaled_up() {
    let (editor, id) = filled(8, 4);
    let (w, h, _) = render_layer_thumbnail(editor.document(), id, 40)
        .unwrap()
        .unwrap();
    assert_eq!((w, h), (8, 4));
}

#[test]
fn the_thumbnail_shows_content_even_when_the_layer_is_hidden_or_faded() {
    // As in Photoshop: the eye and the opacity change how the layer composites, not what it holds.
    let (mut editor, id) = filled(64, 64);
    editor
        .execute(Command::SetLayerVisibility { id, visible: false })
        .unwrap();
    editor
        .execute(Command::SetLayerOpacity { id, opacity: 0.1 })
        .unwrap();
    let (_, _, pixels) = render_layer_thumbnail(editor.document(), id, 16)
        .unwrap()
        .unwrap();
    assert!(pixels.chunks(4).all(|p| p == [200, 40, 10, 255]));
}

#[test]
fn an_unknown_id_has_no_thumbnail() {
    let (editor, _) = filled(16, 16);
    assert!(
        render_layer_thumbnail(editor.document(), LayerId::new(), 16)
            .unwrap()
            .is_none()
    );
}

#[test]
fn the_requested_size_is_capped() {
    let (editor, id) = filled(2000, 1000);
    let (w, _, _) = render_layer_thumbnail(editor.document(), id, 10_000)
        .unwrap()
        .unwrap();
    assert_eq!(w, MAX_LAYER_THUMBNAIL_SIDE);
}
