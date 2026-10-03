// SPDX-License-Identifier: GPL-3.0-or-later

//! K.0. The shared neighbourhood reader, and the proof that porting a filter onto it changed
//! nothing.

use redrob_core::neighbourhood::{EdgePolicy, Neighbourhood};
use redrob_core::{Command, Document, Editor, Filter, Pixel};

/// A 4x3 image whose pixels are all distinct, so a wrong coordinate cannot coincide with a right
/// one. Red channel encodes `x`, green encodes `y`.
fn probe_image() -> (Vec<u8>, u32, u32) {
    let (width, height) = (4u32, 3u32);
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            pixels.extend_from_slice(&[(x * 10) as u8, (y * 10) as u8, 7, 255]);
        }
    }
    (pixels, width, height)
}

/// Each policy gives the documented answer for a sample off the edge.
///
/// Asserted per policy against a specific coordinate rather than "it does not panic": the whole
/// value of one place deciding this is that the four answers are DIFFERENT, and a test that only
/// checked for absence of a crash would pass with every policy collapsed into one.
#[test]
fn each_edge_policy_answers_outside_the_image_its_own_way() {
    let (pixels, width, height) = probe_image();

    // Clamp: x = −1 reads column 0, so the red channel is 0.
    let clamp = Neighbourhood::new(&pixels, width, height, EdgePolicy::Clamp);
    assert_eq!(clamp.channel(-1, 1, 0), Some(0.0));
    // And x = 9 (past the right edge) reads column 3, red = 30.
    assert_eq!(clamp.channel(9, 1, 0), Some(30.0));

    // Transparent black: outside is zero in every channel, including alpha.
    let none = Neighbourhood::new(&pixels, width, height, EdgePolicy::TransparentBlack);
    assert_eq!(none.channel(-1, 1, 0), Some(0.0));
    assert_eq!(
        none.channel(-1, 1, 3),
        Some(0.0),
        "alpha outside must be zero too — that is what makes it transparent rather than black"
    );
    // Inside, it reads the real pixel, so it is not simply zero everywhere.
    assert_eq!(none.channel(2, 1, 0), Some(20.0));

    // Wrap: x = −1 reads the LAST column, red = 30.
    let wrap = Neighbourhood::new(&pixels, width, height, EdgePolicy::Wrap);
    assert_eq!(wrap.channel(-1, 1, 0), Some(30.0));
    // And y = 3 wraps to row 0, green = 0.
    assert_eq!(wrap.channel(1, 3, 1), Some(0.0));
    // A coordinate several images away still resolves, not just one step out.
    assert_eq!(wrap.channel(-5, 1, 0), Some(30.0));

    // Normalise: the sample does not exist and says so.
    let normalise = Neighbourhood::new(&pixels, width, height, EdgePolicy::Normalise);
    assert_eq!(normalise.channel(-1, 1, 0), None);
    assert_eq!(normalise.channel(2, 1, 0), Some(20.0));
}

/// Only `Normalise` reports a short window; the other three always fill it.
///
/// This is the distinction the policy exists for. A blur dividing by the full window under
/// transparent black fades its own border, and under clamp biases it toward whichever pixel
/// happened to be on the edge — the count is what lets the caller avoid both.
#[test]
fn only_the_normalise_policy_reports_a_short_window() {
    let (pixels, width, height) = probe_image();
    // A radius-1 window at the top-left corner overhangs on two sides: 4 of 9 samples are real.
    for policy in [
        EdgePolicy::Clamp,
        EdgePolicy::TransparentBlack,
        EdgePolicy::Wrap,
    ] {
        let view = Neighbourhood::new(&pixels, width, height, policy);
        let (_, counted) = view.window_sum(0, 0, 1, 0);
        assert_eq!(counted, 9, "{policy:?} must fill the whole window");
    }
    let view = Neighbourhood::new(&pixels, width, height, EdgePolicy::Normalise);
    let (total, counted) = view.window_sum(0, 0, 1, 0);
    assert_eq!(counted, 4, "only the four real samples count");
    // Columns 0 and 1 of rows 0 and 1: red is 0, 10, 0, 10.
    assert_eq!(total, 20.0);
}

/// A kernel summing to zero returns its raw response instead of dividing by nothing.
///
/// Every difference operator — Laplacian, Sobel — has a zero-sum kernel. Normalising by the weight
/// would be a division by zero, and the `0/0` would come out NaN and quantise to a black pixel, so
/// an edge detector would output a black image. The raw response IS the answer for those.
#[test]
fn a_zero_sum_kernel_returns_its_raw_response() {
    let (pixels, width, height) = probe_image();
    let view = Neighbourhood::new(&pixels, width, height, EdgePolicy::Clamp);
    // A horizontal difference: right minus left. At x = 1 that is 20 − 0 = 20.
    const DIFFERENCE: [f64; 9] = [
        0.0, 0.0, 0.0, //
        -1.0, 0.0, 1.0, //
        0.0, 0.0, 0.0,
    ];
    let response = view.convolve(1, 1, &DIFFERENCE, 3, 0);
    assert!(
        response.is_finite(),
        "a zero-sum kernel must not produce NaN, got {response}"
    );
    assert_eq!(response, 20.0);
}

/// A kernel with a non-zero sum is normalised by the weight ACTUALLY applied.
///
/// Under `Normalise` a window overhanging the edge applies less than the kernel's full weight, so
/// dividing by the kernel's total would darken every border pixel. Dividing by what was applied is
/// what keeps a box kernel a true average.
#[test]
fn a_normalising_kernel_divides_by_the_weight_actually_applied() {
    let (pixels, width, height) = probe_image();
    let view = Neighbourhood::new(&pixels, width, height, EdgePolicy::Normalise);
    const BOX: [f64; 9] = [1.0; 9];
    // At the corner only four samples exist, with red 0, 10, 0, 10 — a true average of 5.
    let average = view.convolve(0, 0, &BOX, 3, 0);
    assert_eq!(average, 5.0);
    // Dividing by the kernel's total 9 instead would have given 20/9 ≈ 2.22.
    assert!(
        (average - 20.0 / 9.0).abs() > 1.0,
        "the border must not be darkened by dividing by the full kernel"
    );
}

/// K.0's acceptance: Laplace ported onto the shared reader produces BYTE-IDENTICAL pixels.
///
/// The old inline formula is recomputed here from the same source image and compared exactly. A
/// refactor of a filter is only safe if it is provably the same filter, and "the existing tests
/// still pass" does not establish that — the suite's Laplace coverage might not touch the border,
/// which is precisely where an edge policy differs.
///
/// Every pixel of a 5x4 image is checked, so the corners and edges are all included.
#[test]
fn laplace_ported_onto_the_shared_reader_is_byte_identical() {
    let (width, height) = (5u32, 4u32);
    let mut editor = Editor::new(Document::new(width, height).unwrap()).unwrap();
    // A pattern with real structure, so the Laplacian has something to respond to and a wrong
    // result cannot hide in a flat image.
    for y in 0..height {
        for x in 0..width {
            editor
                .execute(Command::SelectRectangle {
                    rect: redrob_core::Rect::new(x as i32, y as i32, 1, 1),
                    mode: redrob_core::SelectionMode::Replace,
                })
                .unwrap();
            editor
                .execute(Command::Fill {
                    color: Pixel::rgba(
                        (x * 50) as u8,
                        (y * 60) as u8,
                        if (x + y) % 2 == 0 { 200 } else { 30 },
                        255,
                    ),
                })
                .unwrap();
        }
    }
    editor.execute(Command::ClearSelection).unwrap();

    let source = editor.document().layers()[0].pixels().to_vec();
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Laplace,
        })
        .unwrap();
    let ported = editor.document().layers()[0].pixels().to_vec();

    // The formula exactly as it was written inline before K.0, clamped coordinates and all.
    let w = i64::from(width);
    let h = i64::from(height);
    let at = |x: i64, y: i64, c: usize| -> f64 {
        let cx = x.clamp(0, w - 1) as usize;
        let cy = y.clamp(0, h - 1) as usize;
        f64::from(source[(cy * width as usize + cx) * 4 + c])
    };
    let mut expected = source.clone();
    for y in 0..h {
        for x in 0..w {
            let o = (y as usize * width as usize + x as usize) * 4;
            for c in 0..3 {
                let lap = 8.0 * at(x, y, c)
                    - at(x - 1, y - 1, c)
                    - at(x, y - 1, c)
                    - at(x + 1, y - 1, c)
                    - at(x - 1, y, c)
                    - at(x + 1, y, c)
                    - at(x - 1, y + 1, c)
                    - at(x, y + 1, c)
                    - at(x + 1, y + 1, c);
                expected[o + c] = lap.abs().round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    assert_eq!(
        ported, expected,
        "the ported Laplace must be byte-identical to the inline version"
    );
    // And it actually did something, so an all-equal comparison of two no-ops cannot pass.
    assert_ne!(
        ported, source,
        "the filter must have changed the image at all"
    );
}
