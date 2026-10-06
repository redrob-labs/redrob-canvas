//! M13: Image size and canvas size keep 16- and 32-bit documents intact (they used to assume four
//! bytes a pixel, so a deep cel came out the wrong length).

use redrob_core::precision::Precision;
use redrob_core::{Command, Document, Editor, Pixel, Rect, SamplingMode};

fn deep(precision: Precision) -> Editor {
    let mut editor = Editor::new(Document::new(4, 4).unwrap()).unwrap();
    editor.execute(Command::Fill { color: Pixel::rgba(200, 100, 50, 255) }).unwrap();
    editor.execute(Command::SetDocumentPrecision { precision }).unwrap();
    editor
}

fn cel_len(editor: &Editor) -> usize {
    let doc = editor.document();
    doc.layer(doc.active_layer_id()).unwrap().pixels().len()
}

#[test]
fn resizing_a_deep_document_keeps_its_cels_the_right_length() {
    for precision in [Precision::U16, Precision::F32] {
        for sampling in [SamplingMode::Nearest, SamplingMode::Bilinear] {
            let mut editor = deep(precision);
            editor.execute(Command::ResizeCanvas { width: 7, height: 3, sampling }).unwrap();
            assert_eq!(cel_len(&editor), 7 * 3 * precision.bytes_per_pixel(), "{precision:?} {sampling:?}");
            // A flat colour stays that colour.
            let snapshot = editor.render_snapshot().unwrap();
            let bytes = snapshot.rgba8();
            assert_eq!(&bytes[0..4], &[200, 100, 50, 255], "{precision:?} {sampling:?}");
        }
    }
}

#[test]
fn cropping_a_deep_document_keeps_its_cels_the_right_length() {
    for precision in [Precision::U16, Precision::F32] {
        let mut editor = deep(precision);
        editor.execute(Command::CropCanvas { rect: Rect::new(-1, 1, 3, 5) }).unwrap();
        assert_eq!(cel_len(&editor), 3 * 5 * precision.bytes_per_pixel(), "{precision:?}");
    }
}
