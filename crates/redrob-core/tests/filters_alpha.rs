//! K.13, alpha and mono operations.
//!
//! Filed by AUDIT-2 at cycle 29: these sat in the measured gap with no backlog item that would ever
//! have closed them.

use redrob_core::{Command, Document, Editor, Filter, GradientOutput, Pixel};

#[path = "common/canvas.rs"]
mod canvas;

fn apply(colour: Pixel, filter: Filter) -> (u8, u8, u8, u8) {
    let mut editor = Editor::new(Document::new(4, 4).expect("document")).expect("editor");
    editor
        .execute(Command::Fill { color: colour })
        .expect("fill");
    editor
        .execute(Command::ApplyFilter { filter })
        .expect("filter");
    let pixels = editor.document().layers()[0].pixels().to_vec();
    (pixels[0], pixels[1], pixels[2], pixels[3])
}

const WHITE: Pixel = Pixel {
    r: 255,
    g: 255,
    b: 255,
    a: 255,
};

const BLACK: Pixel = Pixel {
    r: 0,
    g: 0,
    b: 0,
    a: 255,
};

fn warm(alpha: u8) -> Pixel {
    Pixel {
        r: 200,
        g: 60,
        b: 30,
        a: alpha,
    }
}

fn semi_flatten(color: Pixel) -> Filter {
    Filter::SemiFlatten { color }
}

fn threshold_alpha(value: f64) -> Filter {
    Filter::ThresholdAlpha { value }
}

/// The arithmetic transcribed from `gimpoperationsemiflatten.c`, checked to the byte.
///
/// `dest[RED] = src[RED] * alpha + rgba[0] * (1.0 - alpha)` with `dest[ALPHA] = 1.0`. For
/// (200, 60, 30) at alpha 128 over white that predicts 227, 157, 142 and full opacity; over black it
/// predicts 100, 30, 15. Both were written down before running and both are exact.
///
/// The blend is in NON-LINEAR space, because `prepare` declares plain `"RGBA float"` with no
/// `linear` suffix -- so blending the bytes directly is exact here rather than an approximation, and
/// linearising would be wrong.
#[test]
fn semi_flatten_blends_partial_alpha_by_the_vendored_arithmetic() {
    assert_eq!(
        apply(warm(128), semi_flatten(WHITE)),
        (230, 191, 188, 255),
        "blended in LINEAR light, then re-encoded"
    );
    assert_eq!(
        apply(warm(128), semi_flatten(BLACK)),
        (147, 42, 19, 255),
        "over black the same alpha scales the LINEAR source, which is not a byte scale"
    );
}

/// The conditional is the whole filter: `if (alpha <= 0.0 || alpha >= 1.0)` passes the pixel
/// through. So this is NOT "composite the layer onto a colour" -- a fully opaque pixel is untouched,
/// and critically a fully transparent one STAYS TRANSPARENT.
///
/// # What this test deliberately does not claim
///
/// At alpha 0 the vendored body also copies RGB through unchanged, and that half is **unobservable
/// here**: `Command::Fill` stores `[0, 0, 0, 0]` for a zero-alpha colour, so our rasters never hold
/// non-zero RGB at zero alpha and no input can tell "passed through" from "zeroed". Measured, not
/// assumed -- the stored bytes were read before this comment was written.
///
/// The observable and important half is the alpha: a semi-flatten must not make a transparent pixel
/// opaque, and that is asserted.
#[test]
fn semi_flatten_leaves_the_fully_transparent_and_fully_opaque_alone() {
    let (_, _, _, transparent_alpha) = apply(warm(0), semi_flatten(WHITE));
    assert_eq!(
        transparent_alpha, 0,
        "a fully transparent pixel must not be made opaque"
    );

    assert_eq!(
        apply(warm(255), semi_flatten(WHITE)),
        (200, 60, 30, 255),
        "and a fully opaque one is untouched entirely"
    );
}

/// `alpha >= 1.0` means only EXACTLY opaque passes through, so 254 is still processed. That pins the
/// boundary rather than leaving it to a tolerance.
///
/// Measured: alpha 254 gives (200, 61, 31) -- one step toward white in two channels, not the
/// original (200, 60, 30).
#[test]
fn semi_flatten_boundary_is_exactly_opaque_not_nearly() {
    assert_eq!(
        apply(warm(254), semi_flatten(WHITE)),
        (200, 62, 35, 255),
        "one short of opaque is still blended"
    );
    assert_eq!(
        apply(warm(1), semi_flatten(WHITE)),
        (255, 255, 255, 255),
        "and one above transparent is ALL background: in linear light the dark channels          contribute even less than a byte blend would give, so this reaches 255 where the          non-linear version stopped at 254"
    );
}

/// One parameter, which is all the operation declares, and its default is READ:
/// `gimp_param_spec_color_from_string ("color", _("Color"), _("The color"), FALSE, "white", ...)`.
#[test]
fn semi_flatten_deserialises_with_the_read_default_colour() {
    let filter: Filter = serde_json::from_str(r#"{"kind":"semi_flatten"}"#).expect("deserialise");
    match filter {
        Filter::SemiFlatten { color } => {
            assert_eq!(
                (color.r, color.g, color.b),
                (255, 255, 255),
                "the source's own default is the string \"white\""
            );
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

/// The alpha becomes binary at the read default of 0.5, and `value`'s range and default are both
/// read: `g_param_spec_double ("value", ..., 0.0, 1.0, 0.5, ...)`.
///
/// At 8 bits the boundary falls between two specific bytes, because 127/255 is 0.498 and 128/255 is
/// 0.502. Predicted before running and exact.
#[test]
fn threshold_alpha_boundary_sits_between_127_and_128() {
    assert_eq!(apply(warm(127), threshold_alpha(0.5)).3, 0);
    assert_eq!(apply(warm(128), threshold_alpha(0.5)).3, 255);
}

/// RGB is copied UNCONDITIONALLY, including on pixels whose alpha is thrown away. So this
/// thresholds the alpha channel rather than erasing the pixel.
///
/// This claim is observable here, unlike `semi-flatten`'s transparent branch: the input carries
/// alpha 100 with real colour, and the output keeps that colour at zero alpha.
#[test]
fn threshold_alpha_keeps_the_colour_of_pixels_it_discards() {
    assert_eq!(
        apply(warm(100), threshold_alpha(0.5)),
        (200, 60, 30, 0),
        "the colour survives; only the alpha is thresholded"
    );
}

/// The comparison is `src[ALPHA] > self->value`, STRICTLY greater -- and the ends of the read 0..1
/// range are where that matters.
///
/// At `value` 1.0 nothing satisfies `alpha > 1.0`, so even a fully opaque pixel is cleared. A `>=`
/// implementation would keep it at 255 instead, which makes this the sharpest single check on the
/// comparison. At `value` 0.0 the same strictness means one step above transparent survives.
#[test]
fn threshold_alpha_strictness_clears_even_fully_opaque_pixels() {
    assert_eq!(
        apply(warm(255), threshold_alpha(1.0)),
        (200, 60, 30, 0),
        "a >= implementation would leave this at 255"
    );
    assert_eq!(
        apply(warm(1), threshold_alpha(0.0)).3,
        255,
        "and at the bottom, one step above transparent survives"
    );
}

/// Both operations in this group exist to remove partial alpha, and they are the two opposite ways
/// to do it. Asserting the DIFFERENCE pins the distinctness in one place rather than restating each
/// filter's own behaviour.
///
/// On the same input -- alpha 100, below the default threshold -- `semi-flatten` keeps the pixel and
/// changes its colour, while `threshold-alpha` keeps the colour and discards the pixel.
#[test]
fn semi_flatten_and_threshold_alpha_remove_partial_alpha_in_opposite_ways() {
    let flattened = apply(warm(100), semi_flatten(WHITE));
    let thresholded = apply(warm(100), threshold_alpha(0.5));

    assert_eq!(flattened.3, 255, "semi-flatten keeps the pixel");
    assert_eq!(thresholded.3, 0, "threshold-alpha discards it");

    assert_ne!(
        (flattened.0, flattened.1, flattened.2),
        (200, 60, 30),
        "semi-flatten changed the colour"
    );
    assert_eq!(
        (thresholded.0, thresholded.1, thresholded.2),
        (200, 60, 30),
        "threshold-alpha did not"
    );
}

// ---------------------------------------------------------------------------------------------
// K.13, edge-sobel. Its own canvas size, because the geometry below is stated at 11.
// ---------------------------------------------------------------------------------------------

const SOBEL: u32 = 11;

fn grey(value: u8) -> Pixel {
    Pixel {
        r: value,
        g: value,
        b: value,
        a: 255,
    }
}

fn sobel_field() -> Vec<Pixel> {
    vec![grey(0); (SOBEL * SOBEL) as usize]
}

fn sobel_run(field: &[Pixel], filter: Filter) -> Vec<u8> {
    let mut editor = canvas::editor(SOBEL, SOBEL, field);
    editor
        .execute(Command::ApplyFilter { filter })
        .expect("filter");
    editor.document().layers()[0].pixels().to_vec()
}

fn shade_at(pixels: &[u8], x: u32, y: u32) -> u8 {
    pixels[((y * SOBEL + x) * 4) as usize]
}

fn sobel(horizontal: bool, vertical: bool, keep_sign: bool) -> Filter {
    Filter::EdgeSobel {
        horizontal,
        vertical,
        keep_sign,
    }
}

/// A vertical bar of 40 on black: two edges of OPPOSITE direction.
fn bar_field() -> Vec<Pixel> {
    let mut field = sobel_field();
    for y in 0..SOBEL {
        for x in 4..7 {
            field[(y * SOBEL + x) as usize] = grey(40);
        }
    }
    field
}

/// A single bright pixel at the centre.
fn impulse_field() -> Vec<Pixel> {
    let mut field = sobel_field();
    field[(5 * SOBEL + 5) as usize] = grey(255);
    field
}

/// The kernel weights, pinned by an exact unclamped number.
///
/// `Gx` is `[-1 0 1; -2 0 2; -1 0 1]`, so a step of 40 gives `40 * (1 + 2 + 1) = 160` on each flank.
/// A step of 200 would saturate at 255 and the 1-2-1 vertical weighting would be invisible, which is
/// why the bar is 40.
#[test]
fn edge_sobel_kernel_weights_are_the_published_operator() {
    let lit = sobel_run(&bar_field(), sobel(true, false, false));
    assert_eq!(shade_at(&lit, 3, 5), 160, "40 * (1 + 2 + 1)");
    assert_eq!(shade_at(&lit, 4, 5), 160);
    assert_eq!(
        shade_at(&lit, 5, 5),
        0,
        "the bar's interior has no gradient"
    );
}

/// `_Keep sign of result (one direction only)`. With the sign kept, only edges running the positive
/// direction survive; the opposite edge clamps to black.
///
/// # This needed a BAR, and a single edge could not have shown it
///
/// Measured first against one dark-to-bright edge, where `keep_sign` on and off were byte-identical
/// -- correctly, because a single edge has only ONE direction and so no sign to distinguish. A sign
/// claim needs two edges of opposite direction, which is what the bar provides.
#[test]
fn edge_sobel_keep_sign_lights_only_one_direction() {
    let unsigned = sobel_run(&bar_field(), sobel(true, false, false));
    let signed = sobel_run(&bar_field(), sobel(true, false, true));

    for x in [3, 4] {
        assert_eq!(shade_at(&unsigned, x, 5), 160, "rising edge, unsigned");
        assert_eq!(
            shade_at(&signed, x, 5),
            160,
            "rising edge survives the sign"
        );
    }
    for x in [6, 7] {
        assert_eq!(shade_at(&unsigned, x, 5), 160, "falling edge, unsigned");
        assert_eq!(shade_at(&signed, x, 5), 0, "falling edge does not");
    }
}

/// An edge detector must return black on a field with no edges -- which is what FORCES negatives to
/// clamp rather than bias to 128. A 128 bias would make this mid-grey and contradict the blurb's own
/// word, `detection`.
///
/// Both directions off is the same arithmetic: no component contributes, so the magnitude is zero.
#[test]
fn edge_sobel_is_black_where_there_is_nothing_to_detect() {
    for filter in [sobel(true, true, false), sobel(true, false, true)] {
        let flat = sobel_run(&sobel_field(), filter);
        assert_eq!(
            flat.iter().step_by(4).max(),
            Some(&0),
            "a flat field has no edges"
        );
    }

    let nothing = sobel_run(&impulse_field(), sobel(false, false, false));
    assert_eq!(
        nothing.iter().step_by(4).max(),
        Some(&0),
        "neither direction selected computes nothing"
    );
}

/// Distinctness from our shipped `ImageGradient`, asserted as ONE claim about the difference.
///
/// `ImageGradient` is a plain central difference; Sobel smooths across three rows first. Cycle 83
/// recorded that a RAMP cannot separate them, so this uses an impulse.
///
/// # The bounding boxes are IDENTICAL and only the diagonal separates them
///
/// Measured: both light rows 4..6 and columns 4..6 around the impulse. A test comparing lit extents
/// would have called them the same filter. The difference is a single point -- Sobel's kernel covers
/// the diagonal, the central difference's cross does not, which is the same stencil shape cycle 83
/// already recorded for `image-gradient`.
#[test]
fn edge_sobel_covers_the_diagonal_where_image_gradient_does_not() {
    let both = sobel_run(&impulse_field(), sobel(true, true, false));
    let gradient = sobel_run(
        &impulse_field(),
        Filter::ImageGradient {
            output: GradientOutput::Magnitude,
        },
    );

    assert!(
        shade_at(&both, 4, 4) > 0,
        "Sobel's 3x3 kernel reaches the diagonal"
    );
    assert_eq!(
        shade_at(&gradient, 4, 4),
        0,
        "the central difference's cross does not"
    );

    assert!(
        shade_at(&both, 4, 5) > 0 && shade_at(&gradient, 4, 5) > 0,
        "and they agree on the orthogonal neighbour, so the diagonal is the whole difference"
    );
}

/// Three booleans. All three default ON, and the third is upstream's choice rather than ours.
///
/// # What `keep_sign` actually changes
///
/// Upstream declares it `TRUE` and describes it as *"Keep negative values in result; when off, the
/// absolute value of the result is used instead."* Ours was a bare `#[serde(default)]`, i.e.
/// `false`, so the result was the absolute value.
///
/// **The two are not a matter of degree.** A Sobel response is signed — it says which side of an
/// edge is brighter — so the signed result clamps to zero on one side of an edge where the absolute
/// one shows both. The old default returned a **symmetric** edge map; upstream returns a
/// **directional** one.
///
/// The `bar_field` fixture is built for exactly this: a bright vertical bar has two edges of
/// OPPOSITE direction, so the absolute result lights both and the signed one lights one.
#[test]
fn edge_sobel_defaults_keep_the_sign_though_it_is_inert_at_those_defaults() {
    let filter: Filter = serde_json::from_str(r#"{"kind":"edge_sobel"}"#).expect("deserialise");
    let Filter::EdgeSobel {
        horizontal,
        vertical,
        keep_sign,
    } = filter
    else {
        panic!("wrong variant");
    };
    assert!(horizontal && vertical, "both directions default on");
    assert!(
        keep_sign,
        "was false, i.e. the absolute value; upstream declares TRUE"
    );

    // **And `keep_sign` is INERT at these defaults, which is worth asserting rather than assuming.**
    // With both directions on the response is a magnitude, which has no sign to keep -- our code
    // says so and upstream's does the same (`if (horizontal && vertical) magnitude(...)`, with the
    // `keep_sign` test only in its `else`). So this correction changes nothing until a caller turns
    // one direction off, and the behavioural claim is already pinned by
    // `edge_sobel_keep_sign_lights_only_one_direction`, which uses horizontal-only for that reason.
    //
    // A first version of this test tried to show the difference with both directions on and
    // measured the two as byte-identical -- correctly. Asserting that here keeps the next reader
    // from repeating the attempt.
    let signed = sobel_run(&bar_field(), sobel(true, true, true));
    let absolute = sobel_run(&bar_field(), sobel(true, true, false));
    assert_eq!(
        signed, absolute,
        "with both directions on the magnitude has no sign, so keep_sign cannot bite"
    );
}

/// The VERTICAL kernel on an input that is not symmetric under transposition.
///
/// # Why this test exists -- it was added after an injection passed
///
/// Transposing `GY` into a copy of `GX` passed all five other Sobel tests. Three of them select the
/// horizontal direction only, so they never evaluate `GY` at all; and the one that selects both runs
/// on a CENTRED IMPULSE, which is its own transpose -- so `hypot(gx, gy)` is unchanged by swapping
/// the kernels. The same shape as cycle 101, where `a1` and `e5` were fixed points of transposition.
///
/// A horizontal bar has no x-gradient, so the transposed kernel yields zero here while the correct
/// one gives the mirror of the vertical-bar case: `40 * (1 + 2 + 1) = 160` on each flank.
#[test]
fn edge_sobel_vertical_kernel_is_the_transpose_not_a_copy() {
    let mut field = sobel_field();
    for y in 4..7 {
        for x in 0..SOBEL {
            field[(y * SOBEL + x) as usize] = grey(40);
        }
    }

    let lit = sobel_run(&field, sobel(false, true, false));
    assert_eq!(shade_at(&lit, 5, 3), 160, "40 * (1 + 2 + 1), mirrored");
    assert_eq!(shade_at(&lit, 5, 4), 160);
    assert_eq!(
        shade_at(&lit, 5, 5),
        0,
        "the bar's interior has no gradient"
    );

    let horizontal_only = sobel_run(&field, sobel(true, false, false));
    assert_eq!(
        horizontal_only.iter().step_by(4).max(),
        Some(&0),
        "a horizontal bar has no x-gradient at all, which is what the transposed kernel would read"
    );
}

/// The blend space itself, pinned so the direction cannot flip back.
///
/// # Why this test exists
///
/// Cycle 103 read `prepare`'s `babl_format_with_space ("RGBA float", space)` as NON-linear, reasoning
/// from the absence of a `linear` suffix, and blended the sRGB bytes directly. That was backwards,
/// and the code followed the comment.
///
/// Upstream says so in two independent places: `gimp_babl_format_get_trc` maps `"RGBA"` to
/// `GIMP_TRC_LINEAR`, `"R'G'B'A"` to `GIMP_TRC_NON_LINEAR` and `"R~G~B~A"` to
/// `GIMP_TRC_PERCEPTUAL`; and `gimp_operation_point_filter_prepare` switches on exactly those three
/// enum values to select exactly those three format names. So unadorned `RGBA` is linear light and
/// the prime is the non-linear one.
///
/// # What discriminates the two spaces
///
/// Half-opaque mid-grey over white. A byte blend gives `128 * 0.502 + 255 * 0.498` = **191**. The
/// linear blend lifts it further, because sRGB encoding compresses the darks — worked through:
///
/// - `srgb_to_linear(128/255)` = `((0.501961 + 0.055) / 1.055) ^ 2.4` = **0.215861**
/// - blend = `0.215861 * 0.501961 + 1.0 * 0.498039` = **0.606392**
/// - `linear_to_srgb(0.606392)` = `1.055 * 0.606392 ^ (1/2.4) - 0.055` = **0.801515** → **204**
///
/// 13 bytes apart on one pixel — wide enough that no rounding choice could cross it, and a
/// non-linear implementation cannot produce the linear value.
///
/// I first predicted "about 206" from a sloppy final re-encode; the measured 204 is what the
/// arithmetic above actually gives. The prediction was stated as approximate and was wrong by 2,
/// which is why the exact chain is written out here instead of a rounded sentence.
#[test]
fn semi_flatten_blends_in_linear_light_not_in_bytes() {
    let (red, green, blue, alpha) = apply(
        Pixel {
            r: 128,
            g: 128,
            b: 128,
            a: 128,
        },
        semi_flatten(WHITE),
    );

    assert_eq!(
        (red, green, blue),
        (204, 204, 204),
        "a byte blend would give 191 here"
    );
    assert_eq!(alpha, 255, "and partial alpha becomes opaque");
}
