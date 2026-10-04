// SPDX-License-Identifier: GPL-3.0-or-later

//! Paint select (L.5).
//!
//! Re-derived from `app/tools/gimppaintselecttool.c` and `app/tools/gimppaintselectoptions.c`
//! (GPL-3.0-or-later), pinned in `docs/upstream-sources.toml`. The segmentation kernel is OURS —
//! `gegl:paint-select` is in the one upstream not vendored here — so these tests pin the READ
//! envelope: the operation-dependent label, the asymmetric mask cut, the stroke width and the
//! combination into the selection. They deliberately do not claim pixel agreement with upstream.

use redrob_core::{
    Command, CoreError, Document, Editor, PAINT_SELECT_ADD_MASK_CUT,
    PAINT_SELECT_DEFAULT_STROKE_WIDTH, PAINT_SELECT_MAX_STROKE_WIDTH, PAINT_SELECT_REMOVE_MASK_CUT,
    Pixel, Rect, SelectionMode,
};

const LIGHT: Pixel = Pixel {
    r: 200,
    g: 200,
    b: 200,
    a: 255,
};
const DARK: Pixel = Pixel {
    r: 20,
    g: 20,
    b: 20,
    a: 255,
};
/// A dark square that does NOT touch the canvas border, so the border background seed is genuinely
/// outside the object. Crossing its boundary costs `0.01 + (180/510)^2 * 8` ~= 1.007 against a flat
/// step's 0.01, which is what makes the watershed land on the drawn edge.
const SQUARE: Rect = Rect {
    x: 5,
    y: 5,
    width: 6,
    height: 6,
};

fn field() -> Editor {
    let mut editor = Editor::new(Document::new(16, 16).unwrap()).unwrap();
    editor.execute(Command::Fill { color: LIGHT }).unwrap();
    editor
        .execute(Command::SelectRectangle {
            rect: SQUARE,
            mode: SelectionMode::Replace,
        })
        .unwrap();
    editor.execute(Command::Fill { color: DARK }).unwrap();
    editor.execute(Command::ClearSelection).unwrap();
    editor
}

fn cover(editor: &Editor, x: u32, y: u32) -> u8 {
    editor.document().selection().coverage(x, y)
}

fn scribble(editor: &mut Editor, mode: SelectionMode) {
    editor
        .execute(Command::PaintSelect {
            scribbles: vec![(8, 8)],
            stroke_width: 1,
            mode,
        })
        .unwrap();
}

/// A growing stroke inside an enclosed object selects that object and stops at its edge.
///
/// This is the kernel doing the job the tool exists for, and the thing it must not do is leak: the
/// light surround stays unselected on every side. The two shaved corners are recorded rather than
/// hidden — at a diagonal corner the object and background fields reach the pixel equally, and the
/// tie goes to the background by construction.
#[test]
fn a_growing_stroke_selects_the_enclosed_object_and_stops_at_its_edge() {
    let mut editor = field();
    scribble(&mut editor, SelectionMode::Add);

    // Inside, including the boundary columns and rows.
    for &(x, y) in &[(8, 8), (7, 7), (5, 7), (10, 7), (7, 5), (7, 10)] {
        assert_eq!(cover(&editor, x, y), 255, "inside at ({x},{y})");
    }
    // Outside, one pixel past the edge on all four sides, and far away.
    for &(x, y) in &[(4, 7), (11, 7), (7, 4), (7, 11), (2, 2), (13, 13)] {
        assert_eq!(cover(&editor, x, y), 0, "outside at ({x},{y})");
    }
    // The diagonal corners are the tie, and the tie goes to the background.
    assert_eq!(cover(&editor, 5, 5), 0);
    assert_eq!(cover(&editor, 5, 10), 0);
}

/// The same stroke with the object already selected REMOVES it.
///
/// Upstream hands the operation's output to `gimp_channel_select_buffer` with the latched
/// `painting_op`, so the region is combined rather than replacing the mask — which is why one
/// stroke shape serves both directions.
#[test]
fn a_shrinking_stroke_removes_the_object_it_is_drawn_on() {
    let mut editor = field();
    editor
        .execute(Command::SelectRectangle {
            rect: SQUARE,
            mode: SelectionMode::Replace,
        })
        .unwrap();
    assert_eq!(cover(&editor, 7, 7), 255);

    scribble(&mut editor, SelectionMode::Subtract);
    assert_eq!(cover(&editor, 7, 7), 0);
    assert_eq!(cover(&editor, 8, 8), 0);
}

/// **The asymmetric mask cut, which is this item's load-bearing read.**
///
/// `gegl:threshold` is set to **0.99 for ADD and 0.01 for everything else**, so the same partially
/// covered pixel is read two different ways. A feathered square leaves its boundary column at
/// coverage 153; shrinking at the 0.01 cut counts that pixel as in-extent and can remove it, while
/// the 0.99 cut would rule it out and leave it standing.
///
/// One assertion about the pair: the 153 pixel goes to **0** and the pixel at 102, which is in the
/// light surround on the far side of the edge, is left **exactly as it was**. So the cut is what
/// admits the partial pixel and the kernel is what decides which partial pixels belong.
#[test]
fn the_mask_cut_is_asymmetric_between_growing_and_shrinking() {
    assert_eq!(PAINT_SELECT_ADD_MASK_CUT, 0.99);
    assert_eq!(PAINT_SELECT_REMOVE_MASK_CUT, 0.01);

    let mut editor = field();
    editor
        .execute(Command::SelectRectangle {
            rect: SQUARE,
            mode: SelectionMode::Replace,
        })
        .unwrap();
    editor
        .execute(Command::FeatherSelection { radius: 2 })
        .unwrap();
    // Partial coverage on both sides of the object's edge.
    assert_eq!(cover(&editor, 5, 7), 153);
    assert_eq!(cover(&editor, 4, 7), 102);

    scribble(&mut editor, SelectionMode::Subtract);
    assert_eq!(cover(&editor, 5, 7), 0, "in-extent partial pixel removed");
    assert_eq!(
        cover(&editor, 4, 7),
        102,
        "background-side partial pixel untouched"
    );
}

/// The label branch is `painting_op == GIMP_CHANNEL_OP_ADD ? 1.f : 0.f`, so only ADD grows.
///
/// With nothing selected, the thresholded mask is empty either way, and that is where the two
/// directions diverge completely. Growing has an empty SEED and falls back on the stroke alone, so
/// it finds the object. Not-growing has an empty BOUND, which rules every pixel out, so the region
/// collapses to the stroke's own dab and nothing is discovered at all.
///
/// One assertion about the pair, on one document with one stroke: `Add` returns the whole object
/// and `Replace` returns a single pixel. An implementation that used one cut for both, or that
/// ignored the bound, could not produce both.
#[test]
fn only_a_growing_stroke_discovers_an_object_from_an_empty_selection() {
    let mut growing = field();
    scribble(&mut growing, SelectionMode::Add);
    assert_eq!(cover(&growing, 7, 7), 255);

    let mut replacing = field();
    scribble(&mut replacing, SelectionMode::Replace);
    assert_eq!(cover(&replacing, 8, 8), 255, "the stroke's own dab");
    assert_eq!(cover(&replacing, 7, 7), 0, "nothing discovered around it");
}

/// `stroke_width` is the dab diameter, read as `1, 6000, 50` with `radius = size / 2` INTEGER
/// division, so a wider stroke seeds more pixels and the bounds are refused outside the range.
#[test]
fn the_stroke_width_is_bounded_and_defaults_to_the_tools_own_value() {
    assert_eq!(PAINT_SELECT_DEFAULT_STROKE_WIDTH, 50);
    assert_eq!(PAINT_SELECT_MAX_STROKE_WIDTH, 6000);

    let mut editor = field();
    for width in [0, PAINT_SELECT_MAX_STROKE_WIDTH + 1] {
        assert!(matches!(
            editor.execute(Command::PaintSelect {
                scribbles: vec![(8, 8)],
                stroke_width: width,
                mode: SelectionMode::Add,
            }),
            Err(CoreError::InvalidFilterParameter)
        ));
    }
    // A scribble off the canvas is refused rather than silently clamped.
    assert!(matches!(
        editor.execute(Command::PaintSelect {
            scribbles: vec![(16, 8)],
            stroke_width: 1,
            mode: SelectionMode::Add,
        }),
        Err(CoreError::InvalidFilterParameter)
    ));
    // No scribbles is a no-op, not an error: upstream's stroke handler simply has nothing to add.
    let before = editor.document().selection_mask_snapshot();
    editor
        .execute(Command::PaintSelect {
            scribbles: vec![],
            stroke_width: 1,
            mode: SelectionMode::Add,
        })
        .unwrap();
    assert_eq!(editor.document().selection_mask_snapshot(), before);
}

/// The command round-trips, and an omitted `stroke_width` reads as the tool's default rather than
/// as zero — which the range check would otherwise refuse.
#[test]
fn the_paint_select_command_round_trips_and_defaults_its_stroke_width() {
    let command = Command::PaintSelect {
        scribbles: vec![(1, 2), (3, 4)],
        stroke_width: 7,
        mode: SelectionMode::Subtract,
    };
    let json = serde_json::to_string(&command).unwrap();
    let decoded: Command = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, command);

    let without_width = r#"{"type":"paint_select","scribbles":[[8,8]],"mode":"add"}"#;
    let decoded: Command = serde_json::from_str(without_width).unwrap();
    assert!(matches!(
        decoded,
        Command::PaintSelect {
            stroke_width: PAINT_SELECT_DEFAULT_STROKE_WIDTH,
            ..
        }
    ));
}
