// SPDX-License-Identifier: GPL-3.0-or-later

//! K.3. Blurs and denoise.

use redrob_core::neighbourhood::EdgePolicy;
use redrob_core::{Command, Document, Editor, Filter, Pixel, Rect, SelectionMode};

/// Paints an image row by row from explicit colours.
fn image(width: u32, height: u32, colors: &[Pixel]) -> Editor {
    assert_eq!(colors.len(), (width * height) as usize);
    let mut editor = Editor::new(Document::new(width, height).unwrap()).unwrap();
    for (index, color) in colors.iter().enumerate() {
        let x = (index as u32 % width) as i32;
        let y = (index as u32 / width) as i32;
        editor
            .execute(Command::SelectRectangle {
                rect: Rect::new(x, y, 1, 1),
                mode: SelectionMode::Replace,
            })
            .unwrap();
        editor.execute(Command::Fill { color: *color }).unwrap();
    }
    editor.execute(Command::ClearSelection).unwrap();
    editor
}

fn pixels(editor: &Editor) -> Vec<u8> {
    editor.document().layers()[0].pixels().to_vec()
}

/// A median removes a single outlier completely; a mean only dilutes it.
///
/// This is the property the filter exists for, and it is the sharpest way to tell a median from a
/// box blur: one bright pixel in an otherwise uniform field must vanish entirely, leaving the
/// field untouched. A mean would spread it across the whole neighbourhood instead.
#[test]
fn median_blur_erases_an_isolated_outlier() {
    // 5x5 of value 40 with a single 255 in the middle.
    let mut colors = vec![Pixel::rgba(40, 40, 40, 255); 25];
    colors[12] = Pixel::rgba(255, 255, 255, 255);
    let mut editor = image(5, 5, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::MedianBlur {
                radius: 1,
                edge_policy: EdgePolicy::Clamp,
            },
        })
        .unwrap();
    let out = pixels(&editor);

    // The outlier's own pixel is now the median of its 3x3, which is 40.
    assert_eq!(out[12 * 4], 40, "the outlier itself must be replaced by 40");
    // And every pixel in the image is 40 — the outlier left no trace at all.
    for index in 0..25 {
        assert_eq!(
            out[index * 4],
            40,
            "pixel {index} must be untouched; a mean would have spread the outlier here"
        );
    }
}

/// A box blur on the same image does NOT erase the outlier, which is why both filters exist.
///
/// Stated as a contrast test rather than left implicit: if the two agreed, one would be redundant.
#[test]
fn median_blur_differs_from_a_box_blur_on_an_outlier() {
    let mut colors = vec![Pixel::rgba(40, 40, 40, 255); 25];
    colors[12] = Pixel::rgba(255, 255, 255, 255);

    let mut median = image(5, 5, &colors);
    median
        .execute(Command::ApplyFilter {
            filter: Filter::MedianBlur {
                radius: 1,
                edge_policy: EdgePolicy::Clamp,
            },
        })
        .unwrap();

    let mut box_blurred = image(5, 5, &colors);
    box_blurred
        .execute(Command::ApplyFilter {
            filter: Filter::BoxBlur { radius: 1 },
        })
        .unwrap();

    assert_ne!(
        pixels(&median),
        pixels(&box_blurred),
        "a median and a mean must differ on an outlier, or one of them is redundant"
    );
    assert!(
        pixels(&box_blurred)[12 * 4] > 40,
        "the box blur must still carry some of the outlier at its centre"
    );
}

/// The median picks an EXISTING sample, never a new value.
///
/// A mean invents values that appear nowhere in the input. A median cannot — every output sample
/// must be one of the inputs it looked at. Checked on a neighbourhood with only two distinct
/// values, where any averaging would produce a third.
#[test]
fn median_blur_never_invents_a_value() {
    // Alternating 0 and 200 in a checkerboard, so no neighbourhood is uniform.
    let colors: Vec<Pixel> = (0..25)
        .map(|i| {
            let v = if (i / 5 + i % 5) % 2 == 0 { 0 } else { 200 };
            Pixel::rgba(v, v, v, 255)
        })
        .collect();
    let mut editor = image(5, 5, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::MedianBlur {
                radius: 1,
                edge_policy: EdgePolicy::Clamp,
            },
        })
        .unwrap();
    let out = pixels(&editor);

    for index in 0..25 {
        let value = out[index * 4];
        assert!(
            value == 0 || value == 200,
            "pixel {index} is {value}, which appears nowhere in the input — a mean would do this"
        );
    }
}

/// Radius 0 is REFUSED, consistently with every other radius filter.
///
/// `validate_radius` is shared across the crate and rejects zero, so `BoxBlur` and the rest
/// already behave this way. I first documented median-blur's zero as a no-op without checking that
/// shared validator; this test is what caught the contradiction. Pinning the refusal keeps the
/// family consistent — one filter accepting what its siblings reject is a surprise, not a kindness.
#[test]
fn median_blur_refuses_radius_zero_like_its_siblings() {
    let colors = vec![Pixel::rgba(10, 20, 30, 255); 4];

    let mut editor = image(2, 2, &colors);
    let median = editor.execute(Command::ApplyFilter {
        filter: Filter::MedianBlur {
            radius: 0,
            edge_policy: EdgePolicy::Clamp,
        },
    });
    assert!(median.is_err(), "radius 0 must be refused");

    // And the sibling it must agree with.
    let mut box_editor = image(2, 2, &colors);
    let boxed = box_editor.execute(Command::ApplyFilter {
        filter: Filter::BoxBlur { radius: 0 },
    });
    assert!(
        boxed.is_err(),
        "BoxBlur refuses it too, which is why median-blur must"
    );
}

/// A transparent-black edge does not drag an opaque region's median toward black.
///
/// Samples the policy cannot resolve contribute NO sample rather than a zero. Contributing zeros
/// would mean a corner pixel's median is computed over a neighbourhood that is mostly fabricated
/// black, which silently darkens every edge of the image.
#[test]
fn median_blur_edge_samples_are_omitted_not_zeroed() {
    // Uniform bright field. Under TransparentBlack, a corner's 3x3 has 4 real samples and 5
    // unresolvable ones; omitting them leaves the median at 200, while zeroing them would make
    // the median 0.
    let colors = vec![Pixel::rgba(200, 200, 200, 255); 9];
    let mut editor = image(3, 3, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::MedianBlur {
                radius: 1,
                edge_policy: EdgePolicy::TransparentBlack,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    assert_eq!(
        out[0], 200,
        "the corner must stay bright; counting unresolvable samples as zero would give 0"
    );
    for index in 0..9 {
        assert_eq!(
            out[index * 4],
            200,
            "pixel {index}: a uniform field must survive a median unchanged"
        );
    }
}

/// The edge policy is honoured, and different policies give different edges.
///
/// Pins that the parameter is wired through rather than ignored: with a bright field beside a dark
/// one, Clamp and Wrap see different neighbours at the boundary.
#[test]
fn median_blur_honours_the_edge_policy() {
    // A 3x1 row: dark, bright, bright.
    //
    // My first attempt used four pixels split evenly and the two policies AGREED, which was a
    // flaw in the test rather than in the filter: the image is one pixel tall, so every column is
    // replicated three times vertically under either policy, and replication cannot change the
    // ratio of dark to bright samples. Only the horizontal neighbour differs, so the case has to
    // be one where that single column tips the count.
    //
    // At pixel 0 with radius 1: clamping sees columns (0, 0, 1) = dark, dark, bright, so six dark
    // against three bright and the median is dark. Wrapping sees (2, 0, 1) = bright, dark, bright,
    // so three dark against six bright and the median is bright.
    let colors = vec![
        Pixel::rgba(0, 0, 0, 255),
        Pixel::rgba(255, 255, 255, 255),
        Pixel::rgba(255, 255, 255, 255),
    ];

    let sample = |policy| {
        let mut editor = image(3, 1, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::MedianBlur {
                    radius: 1,
                    edge_policy: policy,
                },
            })
            .unwrap();
        pixels(&editor)
    };

    let clamped = sample(EdgePolicy::Clamp);
    let wrapped = sample(EdgePolicy::Wrap);
    assert_ne!(
        clamped, wrapped,
        "the policy must reach the filter; identical results would mean it is ignored"
    );
    assert_eq!(clamped[0], 0, "clamping keeps pixel 0 dark");
    assert_eq!(wrapped[0], 255, "wrapping tips pixel 0 bright");
}

/// Alpha is medianed alongside the colour channels.
///
/// A median of a mask is a meaningful denoise of that mask, and treating alpha as a fourth channel
/// is what makes an isolated transparent speck removable.
#[test]
fn median_blur_includes_alpha() {
    let mut colors = vec![Pixel::rgba(100, 100, 100, 255); 9];
    colors[4] = Pixel::rgba(100, 100, 100, 0);
    let mut editor = image(3, 3, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::MedianBlur {
                radius: 1,
                edge_policy: EdgePolicy::Clamp,
            },
        })
        .unwrap();
    assert_eq!(
        pixels(&editor)[4 * 4 + 3],
        255,
        "an isolated transparent speck must be filled in by the median"
    );
}
