// SPDX-License-Identifier: GPL-3.0-or-later

//! Handle transform at the command surface (L.3).
//!
//! Re-derived from `app/tools/gimphandletransformtool.c`,
//! `app/display/gimptoolhandlegrid.c` and `app/core/gimp-transform-utils.c`
//! (GPL-3.0-or-later), pinned in `docs/upstream-sources.toml`. The matrix derivation itself is
//! unit-tested beside the code; these are the claims that need real pixels and a real document.

use redrob_core::{Command, CoreError, Document, Editor, LayerId, Pixel, SamplingMode};

#[path = "common/canvas.rs"]
mod canvas;

const RED: Pixel = Pixel {
    r: 255,
    g: 0,
    b: 0,
    a: 255,
};
const BLUE: Pixel = Pixel {
    r: 0,
    g: 0,
    b: 255,
    a: 255,
};
const CLEAR: Pixel = Pixel {
    r: 0,
    g: 0,
    b: 0,
    a: 0,
};

fn filled(width: u32, height: u32) -> Editor {
    let mut editor = Editor::new(Document::new(width, height).unwrap()).unwrap();
    editor.execute(Command::Fill { color: RED }).unwrap();
    editor
}

fn pixel(editor: &Editor, layer: LayerId, x: u32, y: u32) -> Pixel {
    editor
        .document()
        .layer(layer)
        .unwrap()
        .pixel(editor.document().width(), x, y)
        .unwrap()
}

/// One handle translates the layer, and the pixels move with it.
///
/// `(0,0) -> (1,0)` is a one-pixel shift right. Column 0 has nothing to read from and goes
/// transparent; every other column keeps its colour. This also pins the composition the
/// implementation rests on — the class map is applied to the layer corners and the corner quad is
/// re-solved into a homography — because a wrong composition would shift by the wrong amount or
/// not at all.
#[test]
fn one_handle_shifts_the_layer_by_the_handle_offset() {
    let mut editor = filled(4, 4);
    let layer = editor.document().active_layer_id();

    editor
        .execute(Command::HandleTransform {
            src: vec![(0.0, 0.0)],
            dst: vec![(1.0, 0.0)],
            sampling: SamplingMode::Nearest,
        })
        .unwrap();

    for y in 0..4 {
        assert_eq!(pixel(&editor, layer, 0, y), CLEAR, "column 0 at y={y}");
        for x in 1..4 {
            assert_eq!(pixel(&editor, layer, x, y), RED, "({x},{y})");
        }
    }
}

/// The handle count is the transform class, observed in pixels rather than in a matrix.
///
/// Both documents get the same first two correspondences — `(0,0)` pinned and `(4,0)` pinned — and
/// the three-handle one adds `(0,4) -> (0,8)`, a stretch of the vertical axis alone. A similarity
/// cannot express that, so the two-handle document is returned UNCHANGED while the three-handle one
/// is stretched. One assertion about the difference: an implementation that always solved the
/// affine would stretch both.
///
/// **The input has to be non-uniform, and the first version of this test was wrong for that
/// reason.** A flat fill is INVARIANT under a vertical stretch that still covers the canvas: every
/// destination pixel reads a source at `y / 2`, which is inside the layer, so the whole canvas
/// stays the fill colour and both handle counts look identical. A single blue row at `y = 1` moves
/// to `y = 2` under the stretch and stays at `y = 1` under the identity, which is what separates
/// them.
#[test]
fn a_third_handle_changes_the_result_with_the_first_two_unchanged() {
    let src = vec![(0.0, 0.0), (4.0, 0.0), (0.0, 4.0)];
    let dst = vec![(0.0, 0.0), (4.0, 0.0), (0.0, 8.0)];

    // Row 1 blue, everything else red: a feature whose y position is readable.
    let mut colors = vec![RED; 64];
    for x in 0..8 {
        colors[8 + x] = BLUE;
    }

    let mut two = canvas::editor(8, 8, &colors);
    let two_layer = two.document().active_layer_id();
    let before = two.document().layer(two_layer).unwrap().pixels().to_vec();
    two.execute(Command::HandleTransform {
        src: src[..2].to_vec(),
        dst: dst[..2].to_vec(),
        sampling: SamplingMode::Nearest,
    })
    .unwrap();
    assert_eq!(
        two.document().layer(two_layer).unwrap().pixels(),
        before.as_slice(),
        "two handles pinning two points is the identity"
    );

    let mut three = canvas::editor(8, 8, &colors);
    let three_layer = three.document().active_layer_id();
    three
        .execute(Command::HandleTransform {
            src,
            dst,
            sampling: SamplingMode::Nearest,
        })
        .unwrap();
    // The blue row has moved from y = 1 to y = 2, and y = 1 now reads row 0.
    assert_eq!(pixel(&three, three_layer, 4, 1), RED);
    assert_eq!(pixel(&three, three_layer, 4, 2), BLUE);
    // Under the identity it is still exactly where it started.
    assert_eq!(pixel(&two, two_layer, 4, 1), BLUE);
    assert_eq!(pixel(&two, two_layer, 4, 2), RED);
}

/// A degenerate handle set is refused and the layer is left exactly as it was.
///
/// Two handles on the same point define no direction; three in a line define no plane. Both are
/// configurations a user reaches by hand, so the failure has to be a refusal rather than a
/// division by a near-zero determinant that writes a ruined layer.
#[test]
fn degenerate_handles_are_refused_without_touching_the_layer() {
    let mut editor = filled(4, 4);
    let layer = editor.document().active_layer_id();
    let before = editor.document().layer(layer).unwrap().pixels().to_vec();

    for (src, dst) in [
        (vec![(1.0, 1.0), (1.0, 1.0)], vec![(0.0, 0.0), (3.0, 3.0)]),
        (
            vec![(0.0, 0.0), (2.0, 0.0), (4.0, 0.0)],
            vec![(0.0, 0.0), (2.0, 1.0), (4.0, 5.0)],
        ),
        // Four handles with THREE of them collinear: `(2,0)`, `(1,1)`, `(0,2)` all sit on
        // `x + y = 2`, so the projective solve is singular even though no two points coincide.
        // Found while fixing a corner test that was passing for exactly this reason, not for the
        // reason its name claimed.
        (
            vec![(0.0, 0.0), (2.0, 0.0), (0.0, 2.0), (1.0, 1.0)],
            vec![(0.0, 0.0), (2.5, 0.0), (0.0, 2.5), (1.25, 1.25)],
        ),
        (vec![], vec![]),
        (vec![(0.0, 0.0); 5], vec![(1.0, 1.0); 5]),
        // Mismatched lengths: there is no correspondence to solve.
        (vec![(0.0, 0.0)], vec![(1.0, 1.0), (2.0, 2.0)]),
        // A non-finite coordinate, which would otherwise propagate into every pixel.
        (vec![(f32::NAN, 0.0)], vec![(1.0, 1.0)]),
    ] {
        assert!(matches!(
            editor.execute(Command::HandleTransform {
                src,
                dst,
                sampling: SamplingMode::Nearest,
            }),
            Err(CoreError::InvalidTransform)
        ));
        assert_eq!(
            editor.document().layer(layer).unwrap().pixels(),
            before.as_slice()
        );
    }
}

/// A transform valid on every HANDLE can still send a canvas CORNER to infinity.
///
/// Upstream's validity test runs against the points the matrix was solved from — its handles — and
/// that is not enough here, because the class map is then applied to the layer's corners.
///
/// Constructed rather than stumbled on. The matrix `[1,0,0, 0,1,0, -1/16,-1/16, 1]` has
/// `w = 1 - (x + y) / 16`, exactly zero at `(8, 8)`. Handles on the small square `(0,0)`, `(2,0)`,
/// `(0,2)`, `(2,2)` have `w` of 1, 0.875, 0.875 and 0.75 — every one positive, so the handle test
/// passes — and their images are the destinations below.
///
/// **The handles have to be in GENERAL POSITION and the first version of this test was wrong for
/// that reason.** With a fourth handle at `(1,1)` the points `(2,0)`, `(1,1)` and `(0,2)` are
/// collinear on `x + y = 2`, three of four on one line, so the projective solve is singular. The
/// test passed, and it passed for a degenerate handle set rather than for the corner — the
/// cycle-71 shape. A square has no three collinear.
#[test]
fn a_transform_valid_on_the_handles_is_refused_when_a_corner_maps_to_infinity() {
    let mut editor = filled(8, 8);
    let layer = editor.document().active_layer_id();
    let before = editor.document().layer(layer).unwrap().pixels().to_vec();

    let edge = 2.0_f32 / 0.875;
    let far = 2.0_f32 / 0.75;
    assert!(matches!(
        editor.execute(Command::HandleTransform {
            src: vec![(0.0, 0.0), (2.0, 0.0), (0.0, 2.0), (2.0, 2.0)],
            dst: vec![(0.0, 0.0), (edge, 0.0), (0.0, edge), (far, far)],
            sampling: SamplingMode::Nearest,
        }),
        Err(CoreError::InvalidTransform)
    ));
    assert_eq!(
        editor.document().layer(layer).unwrap().pixels(),
        before.as_slice()
    );
}

/// The corner case the near-zero test CANNOT see: corners that straddle the camera plane.
///
/// Found by reverse-verification, not by reading. Removing the corner validity check passed every
/// other test, because `project` independently rejects a near-zero `w` and was doing that check's
/// job. What it cannot do is notice corners whose `w` differ in SIGN while every one is comfortably
/// far from zero — those project to perfectly finite coordinates describing a quad folded through
/// the camera plane. **This is the only test that covers the corner check on its own.**
///
/// `w = 1 - (x + y) / 10` gives the 8x8 corners `w` of 1, 0.2, **-0.6** and 0.2 — mixed signs,
/// nothing near the 1e-6 threshold. The handles sit on the small square where `w` is 1, 0.8, 0.8
/// and 0.6, all one sign, so the handle test passes.
#[test]
fn corners_straddling_the_camera_plane_are_refused() {
    let mut editor = filled(8, 8);
    let layer = editor.document().active_layer_id();
    let before = editor.document().layer(layer).unwrap().pixels().to_vec();

    let edge = 2.0_f32 / 0.8;
    let far = 2.0_f32 / 0.6;
    assert!(matches!(
        editor.execute(Command::HandleTransform {
            src: vec![(0.0, 0.0), (2.0, 0.0), (0.0, 2.0), (2.0, 2.0)],
            dst: vec![(0.0, 0.0), (edge, 0.0), (0.0, edge), (far, far)],
            sampling: SamplingMode::Nearest,
        }),
        Err(CoreError::InvalidTransform)
    ));
    assert_eq!(
        editor.document().layer(layer).unwrap().pixels(),
        before.as_slice()
    );
}

/// The command round-trips through its wire form, the house check for a command addition.
#[test]
fn the_handle_transform_command_round_trips_through_json() {
    let command = Command::HandleTransform {
        src: vec![(0.0, 0.0), (8.0, 0.0), (8.0, 8.0), (0.0, 8.0)],
        dst: vec![(2.0, 0.0), (6.0, 0.0), (8.0, 8.0), (0.0, 8.0)],
        sampling: SamplingMode::Bilinear,
    };
    let json = serde_json::to_string(&command).unwrap();
    let decoded: Command = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, command);
}
