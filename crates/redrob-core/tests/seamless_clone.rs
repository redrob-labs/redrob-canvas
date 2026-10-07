// SPDX-License-Identifier: GPL-3.0-or-later

//! Seamless clone (L.6).
//!
//! Re-derived from `app/tools/gimpseamlessclonetool.c` and
//! `app/tools/gimpseamlesscloneoptions.c` (GPL-3.0-or-later), pinned in
//! `docs/upstream-sources.toml`. The kernel is `gegl:seamless-clone` and GEGL is the one upstream
//! not vendored here, so the interpolation is OURS; these tests pin the read envelope and the
//! properties the construction guarantees, not agreement with upstream's pixels.

use redrob_core::{
    Command, CoreError, Editor, Pixel, Rect, SEAMLESS_CLONE_DEFAULT_REFINE_SCALE,
    SEAMLESS_CLONE_MAX_REFINE_SCALE, samples_per_edge,
};

#[path = "common/canvas.rs"]
mod canvas;

/// The patch, taken from the top-left of the canvas and dropped at (14, 14).
const SRC: Rect = Rect {
    x: 2,
    y: 2,
    width: 8,
    height: 8,
};
const DST_X: i32 = 14;
const DST_Y: i32 = 14;

fn grey(v: u8) -> Pixel {
    Pixel {
        r: v,
        g: v,
        b: v,
        a: 255,
    }
}

fn read(editor: &Editor, x: u32, y: u32) -> u8 {
    let layer = editor.document().active_layer_id();
    editor
        .document()
        .layer(layer)
        .unwrap()
        .pixel(editor.document().width(), x, y)
        .unwrap()
        .r
}

fn clone_at(colors: &[Pixel], max_refine_scale: u32) -> Editor {
    let mut editor = canvas::editor(24, 24, colors);
    editor
        .execute(Command::SeamlessClone {
            src: SRC,
            dst_x: DST_X,
            dst_y: DST_Y,
            max_refine_scale,
        })
        .unwrap();
    editor
}

/// `max_refine_scale` is a count of boundary samples per edge, and **0 is a legal coarsest mesh**
/// rather than "no mesh" — a region still needs a closed boundary, so zero gives the four corners.
#[test]
fn the_refine_scale_counts_boundary_samples_and_zero_is_the_coarsest_mesh() {
    assert_eq!(SEAMLESS_CLONE_DEFAULT_REFINE_SCALE, 5);
    assert_eq!(SEAMLESS_CLONE_MAX_REFINE_SCALE, 50);
    assert_eq!(samples_per_edge(0), 1);
    assert_eq!(samples_per_edge(5), 6);
    assert_eq!(samples_per_edge(50), 51);
}

/// The seam closes and the patch's own detail survives — the whole point of the operation.
///
/// A flat field of 100 with a flat patch of 200 carrying one black pixel. Every boundary sample
/// sees the same mismatch of `100 - 200 = -100`, mean-value weights sum to 1, so the interpolated
/// correction is -100 across the interior: the flat part of the region lands on **100** and is
/// invisible against the field.
///
/// **The 100 is what separates this from a plain copy**, which would leave 200 there — and the
/// source region is asserted to still BE 200, so the patch really was that colour and arrived as
/// this one. The black pixel goes to `0 - 100` clamped to **0**, so the detail is carried through
/// rather than flattened with the seam.
#[test]
fn the_seam_closes_while_the_patch_keeps_its_detail() {
    let mut colors = vec![grey(100); 24 * 24];
    for y in 2..10 {
        for x in 2..10 {
            colors[y * 24 + x] = grey(200);
        }
    }
    // One dark pixel three in from the patch's top-left corner.
    colors[5 * 24 + 5] = grey(0);

    let editor = clone_at(&colors, 5);

    // The patch is still its own colour where it came from: this was a clone, not a move.
    assert_eq!(read(&editor, 6, 6), 200);
    assert_eq!(read(&editor, 5, 5), 0);

    // The whole destination region matches the field, except the carried detail.
    for row in 0..8 {
        for col in 0..8 {
            let (x, y) = (DST_X as u32 + col, DST_Y as u32 + row);
            let expected = if (col, row) == (3, 3) { 0 } else { 100 };
            assert_eq!(read(&editor, x, y), expected, "at ({x},{y})");
        }
    }
    // And nothing outside it moved.
    assert_eq!(read(&editor, 13, 13), 100);
    assert_eq!(read(&editor, 22, 22), 100);
}

/// A region CORNER is a polygon vertex, so mean-value coordinates give it a one-hot weight and the
/// correction there is the exact mismatch: the corner comes out as the destination's own colour.
///
/// Asserted against a destination with a STEP in it, so the claim is not free. The step sits at
/// `x = 18`, which puts the top-right corner on 255 and the top-left on 0 — and the pixel one in
/// from the right corner is NOT 255, which is what shows the vertex is special rather than the
/// whole edge being copied.
#[test]
fn a_region_corner_lands_on_the_destinations_own_colour() {
    let mut colors = vec![grey(0); 24 * 24];
    for y in 0..24 {
        for x in 0..24 {
            colors[y * 24 + x] = grey(if x < 18 { 0 } else { 255 });
        }
    }
    for y in 2..10 {
        for x in 2..10 {
            colors[y * 24 + x] = grey(128);
        }
    }

    let editor = clone_at(&colors, 5);

    // The four corners of the destination region, each on its destination value.
    assert_eq!(read(&editor, 14, 14), 0);
    assert_eq!(read(&editor, 21, 14), 255);
    assert_eq!(read(&editor, 14, 21), 0);
    assert_eq!(read(&editor, 21, 21), 255);
    // One in from the top-right corner is not simply the destination.
    assert_ne!(read(&editor, 20, 15), 255);
}

/// The refinement scale changes the mesh and therefore the result.
///
/// **A linear destination cannot show this and the first probe was wrong for that reason**:
/// mean-value interpolation reproduces a linear field at any mesh density, so a gradient moved the
/// result by only 1-2 bytes and would have made a weak test. A STEP is what separates the meshes —
/// a corners-only mesh cannot see where it is and ramps smoothly across the region, while a finer
/// mesh locates it.
///
/// Measured at `(18, 15)`: **146 coarse against 200 fine**, a 54-byte difference.
#[test]
fn the_refine_scale_changes_the_interpolation() {
    let mut colors = vec![grey(0); 24 * 24];
    for y in 0..24 {
        for x in 0..24 {
            colors[y * 24 + x] = grey(if x < 18 { 0 } else { 255 });
        }
    }
    for y in 2..10 {
        for x in 2..10 {
            colors[y * 24 + x] = grey(128);
        }
    }

    let coarse = clone_at(&colors, 0);
    let fine = clone_at(&colors, 5);

    assert_eq!(read(&coarse, 18, 15), 146);
    assert_eq!(read(&fine, 18, 15), 200);

    // The corners agree whatever the mesh: they are vertices of every mesh, coarsest included.
    for &(x, y) in &[(14, 14), (21, 14), (14, 21), (21, 21)] {
        assert_eq!(read(&coarse, x, y), read(&fine, x, y), "corner ({x},{y})");
    }
}

/// The geometry and the parameter are both bounded, and a refused call leaves the layer alone.
#[test]
fn an_out_of_range_scale_or_region_is_refused() {
    let colors = vec![grey(100); 24 * 24];
    let mut editor = canvas::editor(24, 24, &colors);
    let layer = editor.document().active_layer_id();
    let before = editor.document().layer(layer).unwrap().pixels().to_vec();

    let refused = [
        // Past the top of upstream's range.
        (SRC, DST_X, DST_Y, SEAMLESS_CLONE_MAX_REFINE_SCALE + 1),
        // Two pixels across has no interior to correct: every pixel is boundary.
        (
            Rect {
                x: 2,
                y: 2,
                width: 2,
                height: 8,
            },
            DST_X,
            DST_Y,
            5,
        ),
        (
            Rect {
                x: 2,
                y: 2,
                width: 8,
                height: 2,
            },
            DST_X,
            DST_Y,
            5,
        ),
        // The source runs off the canvas.
        (
            Rect {
                x: 20,
                y: 2,
                width: 8,
                height: 8,
            },
            DST_X,
            DST_Y,
            5,
        ),
        // The destination runs off the canvas.
        (SRC, 20, DST_Y, 5),
        (SRC, DST_X, 20, 5),
        // A negative destination.
        (SRC, -1, DST_Y, 5),
    ];
    for (src, dst_x, dst_y, scale) in refused {
        assert!(
            matches!(
                editor.execute(Command::SeamlessClone {
                    src,
                    dst_x,
                    dst_y,
                    max_refine_scale: scale,
                }),
                Err(CoreError::InvalidFilterParameter)
            ),
            "{src:?} -> ({dst_x},{dst_y}) scale {scale}"
        );
        assert_eq!(
            editor.document().layer(layer).unwrap().pixels(),
            before.as_slice()
        );
    }
}

/// The command round-trips, and an omitted scale reads as the tool's default.
///
/// The default has to be explicit here precisely because **0 is a legal value**: without it an
/// omitted field and a deliberate coarsest mesh would be the same document.
#[test]
fn the_seamless_clone_command_round_trips_and_defaults_its_scale() {
    let command = Command::SeamlessClone {
        src: SRC,
        dst_x: DST_X,
        dst_y: DST_Y,
        max_refine_scale: 0,
    };
    let json = serde_json::to_string(&command).unwrap();
    let decoded: Command = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, command);

    let without_scale = r#"{"type":"seamless_clone","src":{"x":2,"y":2,"width":8,"height":8},"dst_x":14,"dst_y":14}"#;
    let decoded: Command = serde_json::from_str(without_scale).unwrap();
    assert!(matches!(
        decoded,
        Command::SeamlessClone {
            max_refine_scale: SEAMLESS_CLONE_DEFAULT_REFINE_SCALE,
            ..
        }
    ));
}
