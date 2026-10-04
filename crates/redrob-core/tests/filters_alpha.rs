//! K.13, alpha and mono operations.
//!
//! Filed by AUDIT-2 at cycle 29: these sat in the measured gap with no backlog item that would ever
//! have closed them.

use redrob_core::{Command, Document, Editor, Filter, Pixel};

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
        (227, 157, 142, 255),
        "200*0.502 + 255*0.498 and so on, becoming fully opaque"
    );
    assert_eq!(
        apply(warm(128), semi_flatten(BLACK)),
        (100, 30, 15, 255),
        "over black the same alpha just scales the source"
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
        (200, 61, 31, 255),
        "one short of opaque is still blended"
    );
    assert_eq!(
        apply(warm(1), semi_flatten(WHITE)),
        (255, 254, 254, 255),
        "and one above transparent is nearly all background"
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
