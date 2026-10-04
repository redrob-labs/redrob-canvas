//! K.11, the user-supplied convolution matrix.
//!
//! Two sources make this contract complete: `gimppropgui-convolution-matrix.c` names the 25 kernel
//! cells as a literal `a1..e5` table plus `divisor` and `offset`, and
//! `plug-ins/common/convolution-matrix.c`'s po strings supply `N_ormalise`, `A_lpha-weighting`, the
//! `Border` frame (`E_xtend`, `_Wrap`, `Cro_p`), the `Channels` frame, and a read constraint --
//! `Convolution does not work on layers smaller than 3x3 pixels.`

use redrob_core::{Command, ConvolutionBorder, Editor, Filter, Pixel};

#[path = "common/canvas.rs"]
mod canvas;

/// Build a test canvas. Memoised on its content by `common/canvas.rs`.
fn image(width: u32, height: u32, colors: &[Pixel]) -> Editor {
    canvas::editor(width, height, colors)
}

fn pixels(editor: &Editor) -> Vec<u8> {
    editor.document().layers()[0].pixels().to_vec()
}

const SIZE: usize = 12;

/// The tile-seamless tests use their own 16-square canvas, because the arithmetic they pin is exact
/// at 16: a ramp of step 17 spans 0..255 in exactly 16 columns, so its edge discontinuity is the
/// worst 8 bits allow and its interior step is exactly 17. At 12 the same ramp tops out at 187 and
/// neither number means what the test says it does.
const SEAM: usize = 16;
const RGB: [bool; 4] = [true, true, true, false];

fn black() -> Pixel {
    Pixel {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    }
}

/// A black field with one coloured pixel, which is the only input that shows WHERE a kernel cell
/// reads from.
fn dot(at: usize, colour: Pixel) -> Vec<Pixel> {
    let mut field = vec![black(); SIZE * SIZE];
    field[at * SIZE + at] = colour;
    field
}

fn flat(level: u8) -> Vec<Pixel> {
    vec![
        Pixel {
            r: level,
            g: level,
            b: level,
            a: 255
        };
        SIZE * SIZE
    ]
}

fn identity_kernel() -> [f64; 25] {
    let mut kernel = [0.0; 25];
    kernel[12] = 1.0;
    kernel
}

#[allow(clippy::too_many_arguments)]
fn convolve(
    field: &[Pixel],
    matrix: [f64; 25],
    divisor: f64,
    offset: f64,
    normalise: bool,
    alpha_weighting: bool,
    border: ConvolutionBorder,
    channels: [bool; 4],
) -> Vec<u8> {
    let mut editor = image(SIZE as u32, SIZE as u32, field);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ConvolutionMatrix {
                matrix,
                divisor,
                offset,
                normalise,
                alpha_weighting,
                border,
                channels,
            },
        })
        .expect("filter");
    pixels(&editor)
}

fn plain(field: &[Pixel], matrix: [f64; 25], divisor: f64) -> Vec<u8> {
    convolve(
        field,
        matrix,
        divisor,
        0.0,
        false,
        false,
        ConvolutionBorder::Extend,
        RGB,
    )
}

fn flatten(colors: &[Pixel]) -> Vec<u8> {
    colors.iter().flat_map(|p| [p.r, p.g, p.b, p.a]).collect()
}

fn lit(out: &[u8]) -> Vec<(usize, usize)> {
    (0..SIZE * SIZE)
        .filter(|&i| out[i * 4] > 0)
        .map(|i| (i % SIZE, i / SIZE))
        .collect()
}

/// A horizontal ramp: column x carries `x * 17`, so the left edge is 0 and the right 255.
fn horizontal_ramp() -> Vec<Pixel> {
    (0..SEAM * SEAM)
        .map(|i| {
            let value = ((i % SEAM) * 17) as u8;
            Pixel {
                r: value,
                g: value,
                b: value,
                a: 255,
            }
        })
        .collect()
}

/// The same ramp turned through ninety degrees.
fn vertical_ramp() -> Vec<Pixel> {
    (0..SEAM * SEAM)
        .map(|i| {
            let value = ((i / SEAM) * 17) as u8;
            Pixel {
                r: value,
                g: value,
                b: value,
                a: 255,
            }
        })
        .collect()
}

/// The largest jump a viewer would see where two copies abut, horizontally and vertically.
fn seam(pixels: &[u8]) -> (u32, u32) {
    let across = (0..SEAM)
        .map(|y| {
            let left = i32::from(pixels[(y * SEAM) * 4]);
            let right = i32::from(pixels[(y * SEAM + SEAM - 1) * 4]);
            (left - right).unsigned_abs()
        })
        .max()
        .expect("non-empty");
    let down = (0..SEAM)
        .map(|x| {
            let top = i32::from(pixels[x * 4]);
            let bottom = i32::from(pixels[((SEAM - 1) * SEAM + x) * 4]);
            (top - bottom).unsigned_abs()
        })
        .max()
        .expect("non-empty");
    (across, down)
}

fn seamless(field: &[Pixel]) -> Vec<u8> {
    let mut editor = image(SEAM as u32, SEAM as u32, field);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::TileSeamless,
        })
        .expect("filter");
    pixels(&editor)
}

/// THE blurb's claim, quantified: `Alters edges to make the image seamlessly tileable`.
///
/// A ramp is the worst case for tiling, because its two edges are as far apart as 8 bits allow. The
/// filter must reduce that discontinuity to the magnitude of an ordinary interior gradient, which
/// for a 16-wide ramp of step 17 is exactly 17.
///
/// Measured: 255 before, **17** after -- the ramp's own per-column step. And the axis it says
/// nothing about is left alone, so a horizontal ramp's vertical seam stays 0.
#[test]
fn tile_seamless_collapses_the_edge_discontinuity_to_an_interior_gradient() {
    let field = horizontal_ramp();
    assert_eq!(
        seam(&flatten(&field)),
        (255, 0),
        "a horizontal ramp starts with the worst possible horizontal seam"
    );
    assert_eq!(
        seam(&seamless(&field)),
        (17, 0),
        "and ends with one step of the ramp -- an ordinary interior gradient"
    );
}

/// The two axes are treated the same way, which the weight being `max` of the two distances
/// requires. Measured: a vertical ramp goes 255 to 17 down its own axis while across stays 0.
#[test]
fn tile_seamless_treats_both_axes_alike() {
    let field = vertical_ramp();
    assert_eq!(seam(&flatten(&field)), (0, 255), "the mirror case");
    assert_eq!(
        seam(&seamless(&field)),
        (0, 17),
        "the same collapse, on the other axis"
    );
}

/// Blending a flat field with its own half-offset copy is the identity, whatever the weight: both
/// samples are equal everywhere. An image that already tiles is left exactly alone.
#[test]
fn tile_seamless_leaves_an_already_tiling_image_alone() {
    let field = vec![
        Pixel {
            r: 100,
            g: 100,
            b: 100,
            a: 255
        };
        SEAM * SEAM
    ];
    assert_eq!(
        seamless(&field),
        flatten(&field),
        "a flat field already tiles, so nothing may change"
    );
}

/// At the very corner the weight is 1, so the pixel takes the half-offset sample outright. That
/// pins the offset itself: on a 16-square canvas the corner reads (8, 8).
///
/// Measured on the ramp: the corner was 0 and becomes 136, which is the ramp's value at column 8.
#[test]
fn tile_seamless_corner_takes_the_half_offset_sample() {
    let field = horizontal_ramp();
    let out = seamless(&field);

    let expected = flatten(&field)[(8 * SEAM + 8) * 4];
    assert_eq!(expected, 136, "the ramp's value at column 8");
    assert_eq!(
        out[0], expected,
        "the corner has weight 1, so it is exactly the sample half a canvas away"
    );
}

/// The centre has the LOWEST weight, so it keeps most of its own value. On an even-sized canvas it
/// is not exactly zero -- `min(8, 15-8)` is 7, giving a weight of `1 - 14/15`, about 0.067 -- which
/// is why this asserts dominance rather than equality.
///
/// Measured: 136 becomes 127, with the offset sample at that point being 0. A filter that weighted
/// the centre heavily would land near 0 instead.
#[test]
fn tile_seamless_centre_keeps_most_of_its_own_value() {
    let field = horizontal_ramp();
    let out = seamless(&field);
    let centre = (8 * SEAM + 8) * 4;

    assert_eq!(flatten(&field)[centre], 136, "its own value");
    assert_eq!(
        out[centre], 127,
        "a small blend toward the offset sample of 0, not a large one"
    );
}

/// Parameterless, and deliberately so -- see the variant on the ellipsis that disagrees.
#[test]
fn tile_seamless_deserialises_with_no_parameters() {
    let filter: Filter = serde_json::from_str(r#"{"kind":"tile_seamless"}"#).expect("deserialise");
    assert!(
        matches!(filter, Filter::TileSeamless),
        "no fields to default, because none is readable"
    );
}

/// THE test of the propgui's literal table. It names the cells as
/// `{"a1","b1","c1","d1","e1"}, ... {"a5",...,"e5"}`, so the letter is the COLUMN and the number is
/// the ROW, and `a1..e1` is the first row.
///
/// That ordering decides what a saved matrix means, so it is pinned rather than trusted. A single 1
/// at `a1` makes every pixel read from (x-2, y-2), and at `e5` from (x+2, y+2) -- measured exactly
/// with a lone dot at (6,6) appearing at (8,8) and (4,4).
///
/// Transposing the table, or indexing it column-major, moves both of these.
#[test]
fn convolution_cell_order_is_row_major_with_the_letter_as_column() {
    let field = dot(
        6,
        Pixel {
            r: 200,
            g: 200,
            b: 200,
            a: 255,
        },
    );

    let mut first_cell = [0.0; 25];
    first_cell[0] = 1.0;
    assert_eq!(
        lit(&plain(&field, first_cell, 1.0)),
        vec![(8, 8)],
        "a1 is the top-left cell, so the output reads two back in both axes"
    );

    let mut last_cell = [0.0; 25];
    last_cell[24] = 1.0;
    assert_eq!(
        lit(&plain(&field, last_cell, 1.0)),
        vec![(4, 4)],
        "e5 is the bottom-right cell, so the output reads two forward"
    );

    // The assertion that actually distinguishes row-major from its transpose. `a1` and `e5` are
    // index 0 and 24, both FIXED POINTS of transposition, so the two above are true of either
    // indexing -- and a transposed-index injection duly passed all nine tests until this was added.
    // The same trap as cycle 89's diagonal flare: a symmetric probe for an asymmetric claim.
    //
    // `b1` is index 1, row 0 and column 1, so it reads (x-1, y-2). Transposed it would read
    // (x-2, y-1), and from a dot at (6,6) those are (7,8) against (8,7).
    let mut second_cell = [0.0; 25];
    second_cell[1] = 1.0;
    assert_eq!(
        lit(&plain(&field, second_cell, 1.0)),
        vec![(7, 8)],
        "b1 is row 0 column 1, so it reads (x-1, y-2) -- a transpose would give (8,7)"
    );
}

/// The identity kernel -- 1 at the centre, 0 elsewhere -- changes nothing. This is the variant's
/// default, and a default that alters the image would be the wrong one for a user-supplied matrix.
#[test]
fn convolution_identity_kernel_is_the_identity() {
    let field = dot(
        6,
        Pixel {
            r: 200,
            g: 200,
            b: 200,
            a: 255,
        },
    );
    assert_eq!(
        plain(&field, identity_kernel(), 1.0),
        flatten(&field),
        "a centre-only kernel with divisor 1 must change nothing"
    );
}

/// `D_ivisor:` and `O_ffset:` are arithmetic, so they are checked to the byte on a flat field where
/// an all-ones kernel sums to 25 times the level.
///
/// Measured on flat 100: divisor 25 gives 100, divisor 50 gives 50, and divisor 25 with offset 25
/// gives 125.
#[test]
fn convolution_divisor_and_offset_are_exact() {
    let field = flat(100);
    let ones = [1.0f64; 25];

    assert_eq!(
        plain(&field, ones, 25.0)[0],
        100,
        "a box blur of a flat field is the identity"
    );
    assert_eq!(
        plain(&field, ones, 50.0)[0],
        50,
        "doubling the divisor halves the result"
    );
    assert_eq!(
        convolve(
            &field,
            ones,
            25.0,
            25.0,
            false,
            false,
            ConvolutionBorder::Extend,
            RGB
        )[0],
        125,
        "and the offset is added after the division"
    );
}

/// `N_ormalise` divides by the kernel's OWN sum, so it must override the divisor entirely.
///
/// Measured: an all-ones kernel with a deliberately absurd divisor of 999 returns 100 on a flat 100
/// field, because normalising divides by 25 instead.
#[test]
fn convolution_normalise_overrides_the_divisor() {
    let field = flat(100);
    let ones = [1.0f64; 25];

    assert_eq!(
        convolve(
            &field,
            ones,
            999.0,
            0.0,
            true,
            false,
            ConvolutionBorder::Extend,
            RGB
        )[0],
        100,
        "normalising uses the kernel sum of 25, not the stated 999"
    );
    assert_eq!(
        plain(&field, ones, 999.0)[0],
        3,
        "and without it the stated divisor really does apply: 100 * 25 / 999 is 2.5, rounding to 3"
    );
}

/// The `Channels` mask leaves an unticked channel byte-identical.
///
/// # Why the field is coloured and not flat
///
/// The first probe used a flat grey field, where every channel blurs to the same value -- so red-off
/// and red-on both read 100 and the test could not discriminate. A coloured dot separates them, and
/// the arithmetic is exact: 250/25 = 10, 120/25 = 4.8 -> 5, 60/25 = 2.4 -> 2.
#[test]
fn convolution_channel_mask_exempts_a_channel_exactly() {
    let field = dot(
        6,
        Pixel {
            r: 250,
            g: 120,
            b: 60,
            a: 255,
        },
    );
    let ones = [1.0f64; 25];
    let centre = (6 * SIZE + 6) * 4;

    let all_on = plain(&field, ones, 25.0);
    assert_eq!(
        (all_on[centre], all_on[centre + 1], all_on[centre + 2]),
        (10, 5, 2),
        "each channel is divided by 25 in its own right"
    );

    let red_off = convolve(
        &field,
        ones,
        25.0,
        0.0,
        false,
        false,
        ConvolutionBorder::Extend,
        [false, true, true, false],
    );
    assert_eq!(
        (red_off[centre], red_off[centre + 1], red_off[centre + 2]),
        (250, 5, 2),
        "red is carried through untouched while green and blue convolve"
    );

    assert_eq!(
        all_on[centre + 3],
        255,
        "alpha is off by default, so it is untouched too"
    );
}

/// The three `Border` values are three behaviours, and `Cro_p` is the one with no `EdgePolicy`
/// equivalent: it declines to convolve the border at all, which is about the OUTPUT rather than
/// about reading.
///
/// Measured with the dot at the very corner (0,0) and an all-ones kernel: Extend lights 9 pixels
/// with the corner at 72, Wrap lights 25 with the corner at 8, and Crop lights 2 with the corner
/// still at its original 200 -- untouched, because it is a border pixel.
#[test]
fn convolution_three_border_rules_are_three_behaviours() {
    let field = dot(
        0,
        Pixel {
            r: 200,
            g: 200,
            b: 200,
            a: 255,
        },
    );
    let ones = [1.0f64; 25];

    let extend = plain(&field, ones, 25.0);
    let wrap = convolve(
        &field,
        ones,
        25.0,
        0.0,
        false,
        false,
        ConvolutionBorder::Wrap,
        RGB,
    );
    let crop = convolve(
        &field,
        ones,
        25.0,
        0.0,
        false,
        false,
        ConvolutionBorder::Crop,
        RGB,
    );

    assert_eq!(
        lit(&extend).len(),
        9,
        "extending repeats the corner sample into the kernel"
    );
    assert_eq!(
        lit(&wrap).len(),
        25,
        "wrapping reaches the opposite edges, so the whole kernel footprint lights"
    );
    assert_eq!(
        crop[0], 200,
        "cropping leaves the border pixel exactly as it was"
    );
    assert!(
        extend[0] != crop[0] && wrap[0] != crop[0] && extend[0] != wrap[0],
        "all three must differ at the corner"
    );
}

/// READ, not a convention of ours: `Convolution does not work on layers smaller than 3x3 pixels.`
/// is an error message, so upstream refuses rather than coping.
///
/// Measured against that exact wording: 1x1 and 2x2 are refused, 3x3 is accepted. Note the threshold
/// is 3x3 while the kernel is 5x5 -- upstream's own number, not one tidied to match the kernel.
#[test]
fn convolution_refuses_a_layer_smaller_than_three_by_three() {
    for size in [1u32, 2] {
        let field = vec![black(); (size * size) as usize];
        let mut editor = image(size, size, &field);
        assert!(
            editor
                .execute(Command::ApplyFilter {
                    filter: Filter::ConvolutionMatrix {
                        matrix: identity_kernel(),
                        divisor: 1.0,
                        offset: 0.0,
                        normalise: false,
                        alpha_weighting: false,
                        border: ConvolutionBorder::Extend,
                        channels: RGB,
                    },
                })
                .is_err(),
            "{size}x{size} is under the read threshold and must be refused"
        );
    }

    let field = vec![black(); 9];
    let mut editor = image(3, 3, &field);
    assert!(
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::ConvolutionMatrix {
                    matrix: identity_kernel(),
                    divisor: 1.0,
                    offset: 0.0,
                    normalise: false,
                    alpha_weighting: false,
                    border: ConvolutionBorder::Extend,
                    channels: RGB,
                },
            })
            .is_ok(),
        "3x3 is exactly the threshold the message names, so it must be accepted"
    );
}

/// A divisor of zero has no meaning, and normalising a kernel whose cells sum to zero has none
/// either -- so the explicit divisor stands in that case and must itself be usable.
#[test]
fn convolution_refuses_an_unusable_divisor() {
    let field = flat(100);
    for (matrix, divisor, normalise) in [
        (identity_kernel(), 0.0, false),
        (identity_kernel(), f64::NAN, false),
        ([0.0f64; 25], 0.0, true),
    ] {
        let mut editor = image(SIZE as u32, SIZE as u32, &field);
        assert!(
            editor
                .execute(Command::ApplyFilter {
                    filter: Filter::ConvolutionMatrix {
                        matrix,
                        divisor,
                        offset: 0.0,
                        normalise,
                        alpha_weighting: false,
                        border: ConvolutionBorder::Extend,
                        channels: RGB,
                    },
                })
                .is_err(),
            "a divisor that cannot divide must be refused"
        );
    }
}

/// Seven parameters, matching the two sources between them: the 25-cell matrix, divisor, offset, two
/// toggles, the border rule and the channel mask.
#[test]
fn convolution_deserialises_with_the_identity_kernel() {
    let filter: Filter =
        serde_json::from_str(r#"{"kind":"convolution_matrix"}"#).expect("deserialise");
    match filter {
        Filter::ConvolutionMatrix {
            matrix,
            divisor,
            offset,
            normalise,
            alpha_weighting,
            border,
            channels,
        } => {
            assert_eq!(matrix, identity_kernel(), "a default that changes nothing");
            assert!((divisor - 1.0).abs() < f64::EPSILON);
            assert!(offset.abs() < f64::EPSILON);
            assert!(!normalise && !alpha_weighting, "both toggles default off");
            assert_eq!(
                border,
                ConvolutionBorder::Extend,
                "`E_xtend` is the first of the three read border names"
            );
            assert_eq!(
                channels, RGB,
                "colour on, alpha off -- convolving alpha would alter the layer's shape"
            );
        }
        other => panic!("wrong variant: {other:?}"),
    }
}
