//! Puppet warp (rigid moving least squares), batch 4 item L7.

use redrob_core::{Command, Document, Editor, Pixel, Rect, SamplingMode};

fn bar() -> Editor {
    // A horizontal bar across the middle of a 32x32 layer.
    let mut editor = Editor::new(Document::new(32, 32).unwrap()).unwrap();
    editor
        .execute(Command::SelectRectangle {
            rect: Rect {
                x: 4,
                y: 14,
                width: 24,
                height: 4,
            },
            mode: Default::default(),
        })
        .unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(0, 0, 0, 255),
        })
        .unwrap();
    editor.execute(Command::ClearSelection).unwrap();
    editor
}

fn pixels(editor: &Editor) -> Vec<u8> {
    let doc = editor.document();
    doc.layer(doc.active_layer_id()).unwrap().pixels().to_vec()
}

fn warp(editor: &mut Editor, src: Vec<(f32, f32)>, dst: Vec<(f32, f32)>) {
    editor
        .execute(Command::PuppetWarp {
            src_pts: src,
            dst_pts: dst,
            sampling: SamplingMode::Nearest,
        })
        .unwrap();
}

#[test]
fn unmoved_pins_change_nothing() {
    let mut editor = bar();
    let before = pixels(&editor);
    let pins = vec![(6.0, 16.0), (16.0, 16.0), (26.0, 16.0)];
    warp(&mut editor, pins.clone(), pins);
    assert_eq!(pixels(&editor), before);
}

#[test]
fn one_pin_translates() {
    let mut editor = bar();
    warp(&mut editor, vec![(16.0, 16.0)], vec![(16.0, 20.0)]);
    let p = pixels(&editor);
    let alpha = |x: usize, y: usize| p[(y * 32 + x) * 4 + 3];
    assert_eq!(alpha(16, 19), 255, "the bar moved down 4");
    assert_eq!(alpha(16, 14), 0);
}

#[test]
fn a_moved_end_keeps_the_bar_thick_rather_than_stretching_it() {
    // Pin both ends and lift the right one: a rigid bend keeps the bar about 4 px thick near the
    // middle, where a rubber-sheet bend would thin or thicken it.
    let mut editor = bar();
    warp(
        &mut editor,
        vec![(6.0, 16.0), (26.0, 16.0)],
        vec![(6.0, 16.0), (26.0, 8.0)],
    );
    let p = pixels(&editor);
    let column: Vec<u8> = (0..32).map(|y| p[(y * 32 + 16) * 4 + 3]).collect();
    let thick = column.iter().filter(|a| **a > 0).count();
    assert!((3..=6).contains(&thick), "middle column thickness {thick}");
}

#[test]
fn mismatched_pins_are_refused() {
    let mut editor = bar();
    assert!(
        editor
            .execute(Command::PuppetWarp {
                src_pts: vec![(1.0, 1.0)],
                dst_pts: vec![],
                sampling: SamplingMode::Nearest
            })
            .is_err()
    );
}
