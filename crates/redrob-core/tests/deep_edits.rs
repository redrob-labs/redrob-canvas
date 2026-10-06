//! L12: 8-bit-only pixel edits on 16- and 32-bit documents. They run on an 8-bit copy of the
//! active cel; the pixels they do not change keep their deep bytes exactly.

use redrob_core::precision::Precision;
use redrob_core::{Command, Document, Editor, Pixel, Rect};

fn deep(precision: Precision) -> Editor {
    let mut editor = Editor::new(Document::new(8, 8).unwrap()).unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(200, 100, 50, 255),
        })
        .unwrap();
    editor
        .execute(Command::SetDocumentPrecision { precision })
        .unwrap();
    editor
}

fn cel(editor: &Editor) -> Vec<u8> {
    let doc = editor.document();
    doc.layer(doc.active_layer_id()).unwrap().pixels().to_vec()
}

#[test]
fn a_fill_in_a_selection_keeps_the_rest_of_a_deep_layer_exact() {
    for precision in [Precision::U16, Precision::F32] {
        let mut editor = deep(precision);
        let before = cel(&editor);
        editor
            .execute(Command::SelectRectangle {
                rect: Rect::new(0, 0, 4, 8),
                mode: Default::default(),
            })
            .unwrap();
        editor
            .execute(Command::Fill {
                color: Pixel::rgba(0, 0, 255, 255),
            })
            .unwrap();
        let after = cel(&editor);
        let bpp = precision.bytes_per_pixel();
        assert_eq!(after.len(), before.len(), "{precision:?}: still a deep cel");
        // Right half untouched, byte for byte.
        for y in 0..8 {
            for x in 4..8 {
                let at = (y * 8 + x) * bpp;
                assert_eq!(
                    after[at..at + bpp],
                    before[at..at + bpp],
                    "{precision:?} ({x},{y})"
                );
            }
        }
        assert_eq!(editor.document().precision(), precision);
        let rendered = editor.render_snapshot().unwrap();
        assert_eq!(
            &rendered.rgba8()[0..4],
            &[0, 0, 255, 255],
            "{precision:?}: the fill landed"
        );
        assert_eq!(&rendered.rgba8()[7 * 4..8 * 4], &[200, 100, 50, 255]);
    }
}

#[test]
fn a_failed_edit_on_a_deep_layer_leaves_it_as_it_was() {
    let mut editor = deep(Precision::U16);
    let before = cel(&editor);
    // A puppet warp with mismatched pins is refused.
    assert!(
        editor
            .execute(Command::PuppetWarp {
                src_pts: vec![(1.0, 1.0)],
                dst_pts: vec![],
                sampling: redrob_core::SamplingMode::Nearest,
            })
            .is_err()
    );
    assert_eq!(cel(&editor), before);
    assert_eq!(editor.document().precision(), Precision::U16);
}

#[test]
fn a_flip_of_a_deep_layer_keeps_its_length() {
    let mut editor = deep(Precision::F32);
    editor
        .execute(Command::FlipActive {
            horizontal: true,
            vertical: false,
        })
        .unwrap();
    assert_eq!(cel(&editor).len(), 8 * 8 * Precision::F32.bytes_per_pixel());
}
