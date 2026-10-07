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

// ---- U6: deep detail under faint paint ----

/// A 32x4 16-bit layer ramping from black to white: a 2-pixel black|white image converted to
/// 16 bits and resampled up, which leaves values between 8-bit steps.
fn deep_ramp() -> Editor {
    let mut editor = Editor::new(Document::new(2, 4).unwrap()).unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(0, 0, 0, 255),
        })
        .unwrap();
    editor
        .execute(Command::SelectRectangle {
            rect: Rect::new(1, 0, 1, 4),
            mode: Default::default(),
        })
        .unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(255, 255, 255, 255),
        })
        .unwrap();
    editor.execute(Command::ClearSelection).unwrap();
    editor
        .execute(Command::SetDocumentPrecision {
            precision: Precision::U16,
        })
        .unwrap();
    editor
        .execute(Command::ResizeCanvas {
            width: 32,
            height: 4,
            sampling: redrob_core::SamplingMode::Bilinear,
        })
        .unwrap();
    editor
}

fn red16(cel: &[u8], x: usize) -> u16 {
    let at = x * 8;
    u16::from_le_bytes([cel[at], cel[at + 1]])
}

fn faint_stroke(editor: &mut Editor) {
    let points = (0..32)
        .map(|x| redrob_core::BrushPoint::new(x as f32, 1.5, 1.0))
        .collect();
    editor
        .execute(Command::BrushStroke {
            points,
            color: Pixel::rgba(255, 0, 0, 255),
            size: 2.0,
            opacity: 0.05,
            tip: None,
            pipe: Vec::new(),
            settings: redrob_core::BrushSettings::default(),
        })
        .unwrap();
}

#[test]
fn a_faint_stroke_keeps_the_deep_ramp_under_it_smooth() {
    let mut editor = deep_ramp();
    let before = cel(&editor);
    // The blur really made between-step values; otherwise the test proves nothing.
    let between = (0..32)
        .filter(|&x| !red16(&before, x).is_multiple_of(257))
        .count();
    assert!(between > 4, "the ramp has deep values: {between}");

    faint_stroke(&mut editor);
    let after = cel(&editor);
    // Under the stroke (row 1) the green channel (not painted, only darkened slightly by
    // compositing over it) still holds between-step values instead of collapsing to 8-bit ones.
    let green = |cel: &[u8], x: usize| {
        let at = (32 + x) * 8 + 2;
        u16::from_le_bytes([cel[at], cel[at + 1]])
    };
    let kept = (0..32)
        .filter(|&x| !green(&after, x).is_multiple_of(257))
        .count();
    assert!(kept > 4, "the deep detail survived the stroke: {kept}");
    // And nothing strays more than half an 8-bit step from what the stroke painted.
    let rendered = editor.render_snapshot().unwrap();
    for x in 0..32 {
        let eight = f32::from(rendered.rgba8()[(32 + x) * 4 + 1]) / 255.0;
        let deep = f32::from(green(&after, x)) / 65535.0;
        assert!(
            (deep - eight).abs() <= 0.5 / 255.0 + 1e-4,
            "x {x}: {deep} vs {eight}"
        );
    }
}

#[test]
fn a_geometric_edit_does_not_carry_residue_to_another_pixel() {
    let mut editor = deep_ramp();
    editor
        .execute(Command::FlipActive {
            horizontal: true,
            vertical: false,
        })
        .unwrap();
    let after = cel(&editor);
    // A flipped pixel is the widened 8-bit value (no residue from the pixel that used to be
    // there), exactly as before U6.
    for x in 0..32 {
        let v = red16(&after, x);
        assert!(v.is_multiple_of(257), "x {x}: {v}");
    }
}

#[test]
fn a_deep_document_paints_on_release_not_live() {
    let mut editor = deep(Precision::U16);
    let begin = editor.begin_live_stroke(
        Pixel::rgba(0, 0, 0, 255),
        3.0,
        1.0,
        redrob_core::BrushSettings::default(),
        None,
        Vec::new(),
    );
    assert!(matches!(
        begin,
        Err(redrob_core::CoreError::LiveStrokeUnavailable)
    ));
}
