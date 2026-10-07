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

/// An edge survives where interior noise is flattened.
///
/// This is the property that justifies the filter existing beside a box blur, and it falls out of
/// the equation: the denominator is the squared gradient magnitude, so a strong edge divides the
/// flow down to almost nothing while a flat region's noise is smoothed freely.
///
/// A box blur would do the opposite — soften the edge and spread the noise.
#[test]
fn mean_curvature_blur_preserves_an_edge_while_smoothing_beside_it() {
    // 7x7: left half dark, right half light, with one speck in the middle of each half.
    let mut colors = Vec::new();
    for y in 0..7 {
        for x in 0..7 {
            let base = if x < 3 { 40u8 } else { 210 };
            // TWO pixels wide, not one. A single-pixel extremum has exactly zero
            // central-difference gradient in both axes, so this scheme cannot see it at all --
            // pinned separately below. My first version used 1x1 specks and nothing moved.
            let value = if (x, y) == (1, 2) || (x, y) == (1, 3) {
                120
            } else if (x, y) == (5, 2) || (x, y) == (5, 3) {
                140
            } else {
                base
            };
            colors.push(Pixel::rgba(value, value, value, 255));
        }
    }

    let mut editor = image(7, 7, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::MeanCurvatureBlur {
                iterations: 8,
                edge_policy: EdgePolicy::Clamp,
            },
        })
        .unwrap();
    let out = pixels(&editor);

    let at = |x: usize, y: usize| i32::from(out[(y * 7 + x) * 4]);

    // The specks have moved toward their surroundings.
    assert!(
        at(1, 3) < 120,
        "the dark half's speck must be pulled down, got {}",
        at(1, 3)
    );
    assert!(
        at(5, 3) > 140,
        "the light half's speck must be pulled up, got {}",
        at(5, 3)
    );

    // And the edge between the halves is still an edge. Read on a row away from the specks.
    let left = at(2, 0);
    let right = at(3, 0);
    assert!(
        right - left > 120,
        "the edge must survive: {left} to {right} is only {} apart",
        right - left
    );
}

/// A box blur on the same image softens the edge, which this does not.
///
/// The contrast test. If the two agreed, one would be redundant — and the whole reason to reach
/// for curvature motion is that it is edge-preserving.
#[test]
fn mean_curvature_blur_keeps_an_edge_a_box_blur_loses() {
    let mut colors = Vec::new();
    for _ in 0..7 {
        for x in 0..7 {
            let v = if x < 3 { 40u8 } else { 210 };
            colors.push(Pixel::rgba(v, v, v, 255));
        }
    }

    let edge_of = |filter: Filter| {
        let mut editor = image(7, 7, &colors);
        editor.execute(Command::ApplyFilter { filter }).unwrap();
        let out = pixels(&editor);
        let at = |x: usize| i32::from(out[(3 * 7 + x) * 4]);
        at(3) - at(2)
    };

    let curvature = edge_of(Filter::MeanCurvatureBlur {
        iterations: 8,
        edge_policy: EdgePolicy::Clamp,
    });
    let boxed = edge_of(Filter::BoxBlur { radius: 2 });
    assert!(
        curvature > boxed,
        "curvature motion must keep more of the edge than a mean: {curvature} against {boxed}"
    );
}

/// A uniform field is untouched: there are no level sets to move.
///
/// The equation is 0/0 on a flat neighbourhood, and the limit is "do nothing" — with no gradient
/// there is no curve to shorten. Getting this wrong produces NaN, which would reach the buffer as
/// an arbitrary byte.
#[test]
fn mean_curvature_blur_leaves_a_uniform_field_alone() {
    let colors = vec![Pixel::rgba(90, 140, 200, 255); 25];
    let mut editor = image(5, 5, &colors);
    let before = pixels(&editor);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::MeanCurvatureBlur {
                iterations: 20,
                edge_policy: EdgePolicy::Clamp,
            },
        })
        .unwrap();
    assert_eq!(
        pixels(&editor),
        before,
        "a flat field has no curvature and must survive any number of passes"
    );
}

/// Each pass reads the PREVIOUS pass's output, so iterations are real iterations.
///
/// If a pass read its own partial results, the flow would race across the image within one pass
/// and the iteration count would stop meaning anything. Two passes must differ from one, and more
/// passes must keep moving in the same direction rather than converging immediately.
#[test]
fn mean_curvature_blur_iterations_accumulate() {
    // A 2x2 blob, for the same reason as above: a single pixel is invisible to this scheme.
    let mut colors = vec![Pixel::rgba(40, 40, 40, 255); 49];
    for index in [16usize, 17, 23, 24] {
        colors[index] = Pixel::rgba(200, 200, 200, 255);
    }

    let speck_after = |iterations: u32| {
        let mut editor = image(7, 7, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::MeanCurvatureBlur {
                    iterations,
                    edge_policy: EdgePolicy::Clamp,
                },
            })
            .unwrap();
        i32::from(pixels(&editor)[16 * 4])
    };

    let one = speck_after(1);
    let four = speck_after(4);
    assert!(one < 200, "one pass must already move the speck, got {one}");
    assert!(
        four < one,
        "four passes must move it further than one: {four} against {one}"
    );
}

/// Zero iterations is refused, and so is an absurd count.
///
/// Zero because a filter that does nothing is a mistake rather than a request, consistent with
/// `validate_radius` refusing radius 0. The upper cap because each pass is a full image sweep with
/// no window to amortise it, so a mistyped count becomes a hang that looks like a crash.
#[test]
fn mean_curvature_blur_refuses_zero_and_absurd_iteration_counts() {
    let colors = vec![Pixel::rgba(10, 20, 30, 255); 4];

    for iterations in [0u32, 100_000] {
        let mut editor = image(2, 2, &colors);
        let result = editor.execute(Command::ApplyFilter {
            filter: Filter::MeanCurvatureBlur {
                iterations,
                edge_policy: EdgePolicy::Clamp,
            },
        });
        assert!(result.is_err(), "{iterations} iterations must be refused");
    }
}

/// No pass produces a byte outside the range, and alpha is untouched.
///
/// The flow is signed and unbounded in principle, so the clamp is load-bearing. Alpha is left
/// alone because moving it would soften the edge of a mask the user drew deliberately.
#[test]
fn mean_curvature_blur_stays_in_range_and_keeps_alpha() {
    let colors = vec![
        Pixel::rgba(0, 0, 0, 10),
        Pixel::rgba(255, 255, 255, 200),
        Pixel::rgba(255, 0, 0, 255),
        Pixel::rgba(0, 255, 255, 0),
        Pixel::rgba(128, 128, 128, 128),
        Pixel::rgba(0, 0, 255, 77),
        Pixel::rgba(255, 255, 0, 255),
        Pixel::rgba(10, 240, 10, 1),
        Pixel::rgba(240, 10, 240, 254),
    ];
    let mut editor = image(3, 3, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::MeanCurvatureBlur {
                iterations: 12,
                edge_policy: EdgePolicy::Clamp,
            },
        })
        .unwrap();
    let out = pixels(&editor);

    for (index, expected) in colors.iter().enumerate() {
        assert_eq!(
            out[index * 4 + 3],
            expected.a,
            "pixel {index}'s alpha must be untouched"
        );
    }
}

/// A SINGLE-pixel speck is invisible to this filter — and median-blur is the one that removes it.
///
/// Not a defect, and worth pinning precisely because it looks like one. A one-pixel extremum has
/// **exactly zero** central-difference gradient in both axes: `(left - right) / 2` is zero when
/// both neighbours are the background. The flow's denominator is the squared gradient magnitude,
/// so there is nothing to divide by and the pixel is skipped. Continuous mean curvature motion
/// would erode such a speck; a 3x3 stencil cannot see it.
///
/// Measured: a 1x1 speck of 200 on a field of 40 comes back as 200 after six passes, while a 2x2
/// blob comes back as 57.
///
/// **This is why K.3 holds both filters.** They are complementary rather than redundant:
/// median-blur erases isolated specks and softens edges; mean-curvature-blur preserves edges and
/// cannot touch isolated specks. A user reaching for the wrong one gets nothing, so the asymmetry
/// is asserted here with both filters side by side.
#[test]
fn mean_curvature_blur_cannot_see_a_single_pixel_speck_but_median_blur_can() {
    let mut colors = vec![Pixel::rgba(40, 40, 40, 255); 49];
    colors[3 * 7 + 3] = Pixel::rgba(200, 200, 200, 255);

    let mut curvature = image(7, 7, &colors);
    curvature
        .execute(Command::ApplyFilter {
            filter: Filter::MeanCurvatureBlur {
                iterations: 6,
                edge_policy: EdgePolicy::Clamp,
            },
        })
        .unwrap();
    assert_eq!(
        pixels(&curvature)[(3 * 7 + 3) * 4],
        200,
        "a single-pixel speck has zero central-difference gradient, so the flow cannot move it"
    );

    let mut median = image(7, 7, &colors);
    median
        .execute(Command::ApplyFilter {
            filter: Filter::MedianBlur {
                radius: 1,
                edge_policy: EdgePolicy::Clamp,
            },
        })
        .unwrap();
    assert_eq!(
        pixels(&median)[(3 * 7 + 3) * 4],
        40,
        "median-blur erases it completely — which is why both filters are in this group"
    );
}

/// A focus-blur with the region's geometry spelled out once, so the tests vary one thing each.
fn focus(shape: redrob_core::FocusShape, focus_fraction: f32, radius: f32) -> Filter {
    Filter::FocusBlur {
        shape,
        x: 0.5,
        y: 0.5,
        radius,
        aspect_ratio: 1.0,
        rotation: 0.0,
        focus: focus_fraction,
        midpoint: 0.5,
        blur_radius: 3,
        edge_policy: EdgePolicy::Clamp,
    }
}

/// Builds a noisy field so blurring is detectable anywhere it happens.
fn speckled(width: u32, height: u32) -> Vec<Pixel> {
    (0..width * height)
        .map(|i| {
            // A deterministic checkerboard: any averaging at all moves these values.
            let v = if (i % width + i / width).is_multiple_of(2) {
                20u8
            } else {
                230
            };
            Pixel::rgba(v, v, v, 255)
        })
        .collect()
}

/// The centre stays sharp and the corners are blurred.
///
/// The defining behaviour. On a checkerboard, a sharp pixel keeps its extreme value while a
/// blurred one moves toward the local mean — so the test reads the distance from 20/230 rather
/// than guessing an exact output.
#[test]
fn focus_blur_keeps_the_centre_sharp_and_blurs_the_edges() {
    let colors = speckled(21, 21);
    let mut editor = image(21, 21, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: focus(redrob_core::FocusShape::Circle, 0.4, 0.6),
        })
        .unwrap();
    let out = pixels(&editor);

    let at = |x: usize, y: usize| i32::from(out[(y * 21 + x) * 4]);
    let original = |x: usize, y: usize| i32::from(colors[y * 21 + x].r);

    assert_eq!(
        at(10, 10),
        original(10, 10),
        "the centre is inside the focus region and must be untouched"
    );
    // A corner is well outside the region and must have moved toward the mean (~125).
    let corner_moved = (at(0, 0) - original(0, 0)).abs();
    assert!(
        corner_moved > 40,
        "the corner must be blurred, moved only {corner_moved}"
    );
}

/// `focus` moves the sharp/blurred boundary outward.
///
/// Pins that the parameter reaches the filter and does what its name says: a larger focus
/// fraction must leave MORE pixels sharp, so the count of untouched pixels must rise.
#[test]
fn focus_blur_focus_fraction_widens_the_sharp_region() {
    let colors = speckled(21, 21);

    let sharp_count = |focus_fraction: f32| {
        let mut editor = image(21, 21, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: focus(redrob_core::FocusShape::Circle, focus_fraction, 0.9),
            })
            .unwrap();
        let out = pixels(&editor);
        (0..21 * 21).filter(|i| out[i * 4] == colors[*i].r).count()
    };

    let narrow = sharp_count(0.2);
    let wide = sharp_count(0.8);
    assert!(
        wide > narrow,
        "a wider focus must leave more pixels sharp: {wide} against {narrow}"
    );
}

/// Each shape bounds a different region, and the five are genuinely different.
///
/// `GimpLimitType` names five shapes and the metric each implies follows from its name. A test
/// that only checked "something changed" would pass with all five wired to the same metric, so
/// this compares the SETS of sharp pixels and requires all five to differ.
#[test]
fn focus_blur_the_five_shapes_bound_different_regions() {
    let colors = speckled(21, 21);

    let sharp_mask = |shape| {
        let mut editor = image(21, 21, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: focus(shape, 0.5, 0.7),
            })
            .unwrap();
        let out = pixels(&editor);
        (0..21 * 21)
            .map(|i| out[i * 4] == colors[i].r)
            .collect::<Vec<bool>>()
    };

    let shapes = [
        redrob_core::FocusShape::Circle,
        redrob_core::FocusShape::Square,
        redrob_core::FocusShape::Diamond,
        redrob_core::FocusShape::Horizontal,
        redrob_core::FocusShape::Vertical,
    ];
    let masks: Vec<Vec<bool>> = shapes.iter().map(|s| sharp_mask(*s)).collect();

    for i in 0..masks.len() {
        for j in (i + 1)..masks.len() {
            assert_ne!(
                masks[i], masks[j],
                "{:?} and {:?} must bound different regions",
                shapes[i], shapes[j]
            );
        }
    }

    // And the two bands really are bands: a horizontal band must keep a pixel at the far LEFT
    // edge of the centre row sharp, which no bounded shape would.
    let horizontal = sharp_mask(redrob_core::FocusShape::Horizontal);
    assert!(
        horizontal[10 * 21],
        "a horizontal band spans the full width, so the left edge of the centre row is sharp"
    );
    let circle = sharp_mask(redrob_core::FocusShape::Circle);
    assert!(
        !circle[10 * 21],
        "a circle does not reach the left edge, which is what makes the band different"
    );
}

/// Rotation turns the region, and does so WITHOUT shearing it.
///
/// The aspect ratio is undone after un-rotating, which is what keeps a rotated ellipse an ellipse.
/// A 90-degree rotation of an elongated region must swap which axis is long — so a pixel sharp
/// before must be blurred after, and vice versa.
#[test]
fn focus_blur_rotation_turns_an_elongated_region() {
    let colors = speckled(21, 21);

    let sharp_at = |rotation: f32, x: usize, y: usize| {
        let mut editor = image(21, 21, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::FocusBlur {
                    shape: redrob_core::FocusShape::Circle,
                    x: 0.5,
                    y: 0.5,
                    radius: 0.9,
                    // Tall and narrow.
                    aspect_ratio: 4.0,
                    rotation,
                    focus: 0.3,
                    midpoint: 0.5,
                    blur_radius: 3,
                    edge_policy: EdgePolicy::Clamp,
                },
            })
            .unwrap();
        let out = pixels(&editor);
        out[(y * 21 + x) * 4] == colors[y * 21 + x].r
    };

    // Unrotated the region is tall, so a pixel above the centre is sharp and one beside it is not.
    assert!(sharp_at(0.0, 10, 4), "unrotated: the long axis is vertical");
    assert!(!sharp_at(0.0, 4, 10), "and the short axis is horizontal");
    // Rotated a quarter turn, the two swap.
    assert!(!sharp_at(90.0, 10, 4), "rotated: the vertical is now short");
    assert!(sharp_at(90.0, 4, 10), "and the horizontal is now long");
}

/// `midpoint` biases the falloff without moving either limit.
///
/// Both ends are pinned by construction — sharp at `focus`, fully blurred at the region edge — so
/// the only thing midpoint may change is the curve between. A pixel in the band must therefore be
/// blurred by a different amount while the centre and the corner are unchanged.
#[test]
fn focus_blur_midpoint_biases_the_falloff_without_moving_the_limits() {
    let colors = speckled(21, 21);

    let sample = |midpoint: f32| {
        let mut editor = image(21, 21, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::FocusBlur {
                    shape: redrob_core::FocusShape::Circle,
                    x: 0.5,
                    y: 0.5,
                    radius: 0.9,
                    aspect_ratio: 1.0,
                    rotation: 0.0,
                    focus: 0.2,
                    midpoint,
                    blur_radius: 4,
                    edge_policy: EdgePolicy::Clamp,
                },
            })
            .unwrap();
        pixels(&editor)
    };

    let early = sample(0.2);
    let late = sample(0.8);

    // The centre is inside `focus` under both, so it is pinned.
    assert_eq!(
        early[(10 * 21 + 10) * 4],
        late[(10 * 21 + 10) * 4],
        "the sharp limit must not move"
    );
    // A pixel mid-band must differ.
    assert_ne!(
        early[(10 * 21 + 15) * 4],
        late[(10 * 21 + 15) * 4],
        "midpoint must change the curve between the limits"
    );
}

/// Degenerate geometry is refused rather than producing a divide by zero.
#[test]
fn focus_blur_refuses_degenerate_geometry() {
    let colors = speckled(5, 5);

    for filter in [
        // A zero region has no extent to divide by.
        focus(redrob_core::FocusShape::Circle, 0.5, 0.0),
        // A zero blur radius means the filter cannot do anything.
        Filter::FocusBlur {
            shape: redrob_core::FocusShape::Circle,
            x: 0.5,
            y: 0.5,
            radius: 0.5,
            aspect_ratio: 1.0,
            rotation: 0.0,
            focus: 0.5,
            midpoint: 0.5,
            blur_radius: 0,
            edge_policy: EdgePolicy::Clamp,
        },
        // A zero aspect ratio would collapse one axis entirely.
        Filter::FocusBlur {
            shape: redrob_core::FocusShape::Circle,
            x: 0.5,
            y: 0.5,
            radius: 0.5,
            aspect_ratio: 0.0,
            rotation: 0.0,
            focus: 0.5,
            midpoint: 0.5,
            blur_radius: 3,
            edge_policy: EdgePolicy::Clamp,
        },
    ] {
        let mut editor = image(5, 5, &colors);
        assert!(
            editor.execute(Command::ApplyFilter { filter }).is_err(),
            "degenerate geometry must be refused"
        );
    }
}

/// Builds a two-layer document: a speckled target and a supplied map, the way the existing map
/// filter tests do it.
///
/// `DocumentImportBuilder` rather than layer commands, because that is the established path for
/// this crate's map filters — my first draft guessed at `AddRasterLayer` / `SelectLayer`, neither
/// of which exists.
fn target_and_map(
    width: u32,
    height: u32,
    target: &[Pixel],
    map: &[Pixel],
) -> (
    redrob_core::Document,
    redrob_core::NodeId,
    redrob_core::NodeId,
) {
    use redrob_core::{DocumentImportBuilder, FrameId, ImportNode, RasterCel};

    let flatten = |pixels: &[Pixel]| {
        let mut out = Vec::with_capacity(pixels.len() * 4);
        for p in pixels {
            out.extend_from_slice(&[p.r, p.g, p.b, p.a]);
        }
        out
    };

    let mut builder = DocumentImportBuilder::new(width, height).unwrap();
    builder
        .push_node(ImportNode::raster(
            "target",
            vec![RasterCel::new(FrameId::DEFAULT, flatten(target))],
        ))
        .unwrap();
    builder
        .push_node(ImportNode::raster(
            "map",
            vec![RasterCel::new(FrameId::DEFAULT, flatten(map))],
        ))
        .unwrap();
    let document = builder.build().unwrap();
    let target_id = document.nodes()[0].id();
    let map_id = document.nodes()[1].id();
    (document, target_id, map_id)
}

/// A map layer drives the blur: white blurs, black stays sharp.
///
/// The defining behaviour, and what separates this from focus-blur — the variation comes from an
/// IMAGE rather than from geometry.
#[test]
fn variable_blur_takes_its_amount_from_the_map_layer() {
    let target = speckled(12, 4);
    // Black left half, white right half.
    let map: Vec<Pixel> = (0..48)
        .map(|i| {
            let v = if i % 12 < 6 { 0u8 } else { 255 };
            Pixel::rgba(v, v, v, 255)
        })
        .collect();

    let (document, target_id, map_id) = target_and_map(12, 4, &target, &map);
    let mut editor = Editor::new(document).unwrap();
    editor
        .execute(Command::SetActiveLayer { id: target_id })
        .unwrap();
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::VariableBlur {
                radius: 3,
                map: Some(map_id),
                edge_policy: EdgePolicy::Clamp,
            },
        })
        .unwrap();

    let out = editor.document().layers()[0].pixels().to_vec();
    let at = |x: usize, y: usize| out[(y * 12 + x) * 4];
    let before = |x: usize, y: usize| target[y * 12 + x].r;

    for y in 0..4 {
        for x in 0..5 {
            assert_eq!(
                at(x, y),
                before(x, y),
                "({x}, {y}) is under a black map and must be byte-identical"
            );
        }
    }
    let moved = (i32::from(at(9, 2)) - i32::from(before(9, 2))).abs();
    assert!(
        moved > 40,
        "under a white map the pixel must be blurred, moved only {moved}"
    );
}

/// The map's LUMA drives it, not one channel.
///
/// A grey map is the normal case, so reading only red would work on every grey map and behave
/// surprisingly on a coloured one. Pure green's luma is 182 against pure blue's 18, so green must
/// blur much more — while a red-channel reading would blur NEITHER, which is the value the wrong
/// behaviour produces.
#[test]
fn variable_blur_reads_the_map_luma_not_one_channel() {
    let target = speckled(9, 3);

    let moved_under = |map_colour: Pixel| {
        let map = vec![map_colour; 27];
        let (document, target_id, map_id) = target_and_map(9, 3, &target, &map);
        let mut editor = Editor::new(document).unwrap();
        editor
            .execute(Command::SetActiveLayer { id: target_id })
            .unwrap();
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::VariableBlur {
                    radius: 4,
                    map: Some(map_id),
                    edge_policy: EdgePolicy::Clamp,
                },
            })
            .unwrap();
        let out = editor.document().layers()[0].pixels().to_vec();
        (i32::from(out[(9 + 4) * 4]) - i32::from(target[9 + 4].r)).abs()
    };

    let green = moved_under(Pixel::rgba(0, 255, 0, 255));
    let blue = moved_under(Pixel::rgba(0, 0, 255, 255));
    assert!(
        green > blue,
        "green's luma is 182 against blue's 18, so it must blur more: {green} against {blue}"
    );
    assert!(
        green > 30,
        "a green map must actually blur — a red-channel reading would blur neither, got {green}"
    );
}

/// Without a map, the layer's own luma drives it — matching `WarpMap`.
///
/// Not an arbitrary fallback: the map filters here already behave this way, so a user who has
/// learned one has learned this. A bright region blurring itself is rarely wanted, but it is the
/// honest reading of "no map supplied" and keeps the family consistent.
#[test]
fn variable_blur_without_a_map_uses_the_layer_luma() {
    let mut colors = Vec::new();
    for y in 0..4 {
        for x in 0..12 {
            let base = if x < 6 { 10u8 } else { 245 };
            // The speck goes in the BRIGHT half only. My first version put one at 60 in the dark
            // half and asserted it stayed sharp — but 60's own luma gives radius
            // round(60/255 * 3) = 1, so it blurs. The premise held for the 10-valued background
            // and not for the pixel I chose to read, which is the input's fault, not the filter's.
            let value = if (x, y) == (9, 2) { 195 } else { base };
            colors.push(Pixel::rgba(value, value, value, 255));
        }
    }

    let mut editor = image(12, 4, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::VariableBlur {
                radius: 3,
                map: None,
                edge_policy: EdgePolicy::Clamp,
            },
        })
        .unwrap();
    let out = pixels(&editor);

    // 10's luma gives round(10/255 * 3) = 0, so the whole dark half is skipped outright.
    for x in 0..6 {
        assert_eq!(
            out[(2 * 12 + x) * 4],
            10,
            "({x}, 2) has luma 0.04, which rounds to radius 0, so it must stay sharp"
        );
    }
    assert_ne!(
        out[(2 * 12 + 9) * 4],
        195,
        "the bright half's luma is high, so it must blur itself"
    );
}

/// A black map leaves the image BYTE-identical, not merely close.
///
/// A zero radius is skipped rather than averaged over a one-pixel window, so a map with hard edges
/// gives a hard edge in the result instead of a faint seam along it.
#[test]
fn variable_blur_under_a_black_map_is_byte_identical() {
    let target = speckled(8, 8);
    let map = vec![Pixel::rgba(0, 0, 0, 255); 64];
    let (document, target_id, map_id) = target_and_map(8, 8, &target, &map);
    let mut editor = Editor::new(document).unwrap();
    editor
        .execute(Command::SetActiveLayer { id: target_id })
        .unwrap();
    let before = editor.document().layers()[0].pixels().to_vec();
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::VariableBlur {
                radius: 6,
                map: Some(map_id),
                edge_policy: EdgePolicy::Clamp,
            },
        })
        .unwrap();
    assert_eq!(
        editor.document().layers()[0].pixels(),
        &before[..],
        "a fully black map must change nothing at all"
    );
}

/// A hard edge survives while low-contrast noise beside it is smoothed.
///
/// The defining behaviour, and it is achieved without the filter being told where the edge is —
/// which is what separates it from `VariableBlur`, where a map says so, and from `FocusBlur`,
/// where geometry does.
#[test]
fn selective_gaussian_blur_keeps_a_hard_edge_and_smooths_gentle_noise() {
    // Left half 40 with ±8 noise, right half 220. The noise is well inside a delta of 30; the
    // 180-step edge is far outside it.
    let mut colors = Vec::new();
    for y in 0..9u32 {
        for x in 0..9u32 {
            let value = if x < 4 {
                if (x + y).is_multiple_of(2) { 32u8 } else { 48 }
            } else {
                220
            };
            colors.push(Pixel::rgba(value, value, value, 255));
        }
    }

    let mut editor = image(9, 9, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::SelectiveGaussianBlur {
                radius: 2,
                max_delta: 30,
                edge_policy: EdgePolicy::Clamp,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    let at = |x: usize, y: usize| i32::from(out[(y * 9 + x) * 4]);

    // The noisy half has converged toward its own mean of 40.
    assert!(
        (at(1, 4) - 40).abs() <= 6,
        "the low-contrast noise must smooth toward 40, got {}",
        at(1, 4)
    );
    // The edge is intact: the pixel just left of it stays dark, the one right of it stays bright.
    assert!(
        at(3, 4) < 60,
        "the dark side of the edge must not pick up the bright half, got {}",
        at(3, 4)
    );
    assert!(
        at(4, 4) > 210,
        "and the bright side must not pick up the dark half, got {}",
        at(4, 4)
    );
}

/// A plain gaussian blur on the same image destroys the edge, which this does not.
///
/// The contrast test. A gaussian of comparable reach must bleed the two halves into each other.
#[test]
fn selective_gaussian_blur_keeps_an_edge_a_plain_gaussian_loses() {
    let mut colors = Vec::new();
    for _ in 0..9 {
        for x in 0..9 {
            let v = if x < 4 { 40u8 } else { 220 };
            colors.push(Pixel::rgba(v, v, v, 255));
        }
    }

    let step_of = |filter: Filter| {
        let mut editor = image(9, 9, &colors);
        editor.execute(Command::ApplyFilter { filter }).unwrap();
        let out = pixels(&editor);
        i32::from(out[(4 * 9 + 4) * 4]) - i32::from(out[(4 * 9 + 3) * 4])
    };

    let selective = step_of(Filter::SelectiveGaussianBlur {
        radius: 2,
        max_delta: 30,
        edge_policy: EdgePolicy::Clamp,
    });
    let plain = step_of(Filter::GaussianBlur { sigma: 0.667 });
    assert!(
        selective > plain,
        "the selective blur must keep more of the 180-step edge: {selective} against {plain}"
    );
    assert!(
        selective > 170,
        "and must keep nearly all of it, got {selective}"
    );
}

/// `max_delta` is the control: raising it past the edge height blurs through the edge.
///
/// Pins that the parameter reaches the filter and does what its name says. At a delta above the
/// step, every neighbour qualifies and the filter degenerates into a plain gaussian — which is the
/// correct behaviour, not a failure.
#[test]
fn selective_gaussian_blur_delta_above_the_edge_blurs_through_it() {
    let mut colors = Vec::new();
    for _ in 0..9 {
        for x in 0..9 {
            let v = if x < 4 { 40u8 } else { 220 };
            colors.push(Pixel::rgba(v, v, v, 255));
        }
    }

    let step_of = |max_delta: u8| {
        let mut editor = image(9, 9, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::SelectiveGaussianBlur {
                    radius: 2,
                    max_delta,
                    edge_policy: EdgePolicy::Clamp,
                },
            })
            .unwrap();
        let out = pixels(&editor);
        i32::from(out[(4 * 9 + 4) * 4]) - i32::from(out[(4 * 9 + 3) * 4])
    };

    let preserved = step_of(30);
    let blurred_through = step_of(255);
    assert!(
        blurred_through < preserved,
        "a delta above the 180 step must blur through it: {blurred_through} against {preserved}"
    );
}

/// A delta of zero is a no-op on a gradient.
///
/// Only exactly-equal neighbours qualify, so on an image where no two adjacent pixels match, every
/// window reduces to the centre alone. Byte-identical rather than approximately so, because the
/// centre's own weight divides out exactly.
#[test]
fn selective_gaussian_blur_zero_delta_is_a_no_op_on_a_gradient() {
    // Every pixel distinct.
    let colors: Vec<Pixel> = (0..49)
        .map(|i| {
            let v = (i * 5) as u8;
            Pixel::rgba(v, v, v, 255)
        })
        .collect();
    let mut editor = image(7, 7, &colors);
    let before = pixels(&editor);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::SelectiveGaussianBlur {
                radius: 3,
                max_delta: 0,
                edge_policy: EdgePolicy::Clamp,
            },
        })
        .unwrap();
    assert_eq!(
        pixels(&editor),
        before,
        "with delta 0 only the centre qualifies, so nothing may change"
    );
}

/// The delta is tested PER CHANNEL, so a red edge against green survives.
///
/// The strings do not settle per-channel against luma, and the choice is visible here: pure red
/// and pure green have very different luma (54 against 182) but a luma test on a carefully chosen
/// pair would blur through. Per-channel keeps any channel's edge.
///
/// Red (200, 40, 40) against green (40, 200, 40) differs by 160 on two channels, so with a delta
/// of 30 neither contributes to the other and both survive.
#[test]
fn selective_gaussian_blur_tests_the_delta_per_channel() {
    let mut colors = Vec::new();
    for _ in 0..9 {
        for x in 0..9 {
            colors.push(if x < 4 {
                Pixel::rgba(200, 40, 40, 255)
            } else {
                Pixel::rgba(40, 200, 40, 255)
            });
        }
    }

    let mut editor = image(9, 9, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::SelectiveGaussianBlur {
                radius: 2,
                max_delta: 30,
                edge_policy: EdgePolicy::Clamp,
            },
        })
        .unwrap();
    let out = pixels(&editor);

    let left = &out[(4 * 9 + 3) * 4..(4 * 9 + 3) * 4 + 3];
    let right = &out[(4 * 9 + 4) * 4..(4 * 9 + 4) * 4 + 3];
    assert_eq!(
        (left[0], left[1]),
        (200, 40),
        "the red side must stay red, got {left:?}"
    );
    assert_eq!(
        (right[0], right[1]),
        (40, 200),
        "and the green side green, got {right:?}"
    );
}

/// A lone speck is left alone — the opposite of median-blur, and for a readable reason.
///
/// Every neighbour of the speck fails its delta test, so the speck's window reduces to itself. And
/// the speck fails every neighbour's test too, so it does not contaminate them either. Pinned
/// beside median-blur, which erases it, because this is the second complementary pair in K.3 and
/// the group is easier to navigate if the pairs are written down.
#[test]
fn selective_gaussian_blur_leaves_a_lone_speck_where_median_blur_erases_it() {
    let mut colors = vec![Pixel::rgba(40, 40, 40, 255); 49];
    colors[3 * 7 + 3] = Pixel::rgba(220, 220, 220, 255);

    let mut selective = image(7, 7, &colors);
    selective
        .execute(Command::ApplyFilter {
            filter: Filter::SelectiveGaussianBlur {
                radius: 2,
                max_delta: 30,
                edge_policy: EdgePolicy::Clamp,
            },
        })
        .unwrap();
    assert_eq!(
        pixels(&selective)[(3 * 7 + 3) * 4],
        220,
        "every neighbour fails the speck's delta test, so the speck survives untouched"
    );
    assert_eq!(
        pixels(&selective)[(3 * 7 + 2) * 4],
        40,
        "and the speck fails its neighbours' tests, so it does not contaminate them"
    );

    let mut median = image(7, 7, &colors);
    median
        .execute(Command::ApplyFilter {
            filter: Filter::MedianBlur {
                radius: 1,
                edge_policy: EdgePolicy::Clamp,
            },
        })
        .unwrap();
    assert_eq!(
        pixels(&median)[(3 * 7 + 3) * 4],
        40,
        "median-blur erases it — the second complementary pair in this group"
    );
}

/// An edge survives with NO parameter to tune — the defining property.
///
/// Across an edge, the pair member on the centre's own side is always the nearer in value, so the
/// far side never contributes. There is no threshold to get wrong, which is exactly what separates
/// this from `SelectiveGaussianBlur`.
#[test]
fn snn_mean_keeps_an_edge_with_nothing_to_tune() {
    let mut colors = Vec::new();
    for y in 0..9u32 {
        for x in 0..9u32 {
            // Left half 40 with gentle noise, right half 220 flat.
            let value = if x < 4 {
                if (x + y).is_multiple_of(2) { 32u8 } else { 48 }
            } else {
                220
            };
            colors.push(Pixel::rgba(value, value, value, 255));
        }
    }

    let mut editor = image(9, 9, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::SnnMean {
                radius: 2,
                edge_policy: EdgePolicy::Clamp,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    let at = |x: usize, y: usize| i32::from(out[(y * 9 + x) * 4]);

    assert!(
        at(3, 4) < 60,
        "the dark side of the edge must not pick up the bright half, got {}",
        at(3, 4)
    );
    assert!(
        at(4, 4) > 210,
        "and the bright side must not pick up the dark half, got {}",
        at(4, 4)
    );
    // The noisy half has smoothed toward its own mean of 40.
    assert!(
        (at(1, 4) - 40).abs() <= 8,
        "the low-contrast noise must smooth toward 40, got {}",
        at(1, 4)
    );
}

/// Each symmetric pair is visited ONCE, not twice.
///
/// Visiting the whole window would pair every offset with itself from both ends and reduce the
/// filter to an ordinary mean, which is the one failure mode that still looks like a blur. The
/// value the wrong behaviour produces is named: on a hard step with radius 2, an ordinary mean
/// pulls the boundary pixels well off their original values, where SNN leaves them within a step
/// or two.
#[test]
fn snn_mean_visits_each_pair_once_and_is_not_an_ordinary_mean() {
    let mut colors = Vec::new();
    for _ in 0..9 {
        for x in 0..9 {
            let v = if x < 4 { 40u8 } else { 220 };
            colors.push(Pixel::rgba(v, v, v, 255));
        }
    }

    let step_of = |filter: Filter| {
        let mut editor = image(9, 9, &colors);
        editor.execute(Command::ApplyFilter { filter }).unwrap();
        let out = pixels(&editor);
        i32::from(out[(4 * 9 + 4) * 4]) - i32::from(out[(4 * 9 + 3) * 4])
    };

    let snn = step_of(Filter::SnnMean {
        radius: 2,
        edge_policy: EdgePolicy::Clamp,
    });
    let mean = step_of(Filter::BoxBlur { radius: 2 });
    assert!(
        snn > mean + 100,
        "SNN must keep far more of the 180-step edge than a mean: {snn} against {mean}"
    );
    assert!(snn > 170, "and nearly all of it, got {snn}");
}

/// A uniform field is untouched.
///
/// Every pair member equals the centre, so every pick equals the centre and the mean of a set of
/// identical values is that value. Byte-identical rather than approximately so.
#[test]
fn snn_mean_leaves_a_uniform_field_alone() {
    let colors = vec![Pixel::rgba(90, 140, 200, 255); 49];
    let mut editor = image(7, 7, &colors);
    let before = pixels(&editor);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::SnnMean {
                radius: 3,
                edge_policy: EdgePolicy::Clamp,
            },
        })
        .unwrap();
    assert_eq!(
        pixels(&editor),
        before,
        "every pick equals the centre, so a flat field must be byte-identical"
    );
}

/// A lone speck is REDUCED but not erased, which is its own behaviour again.
///
/// The speck's own window: each pair is two background pixels, so both members are equally far
/// from the speck and one is picked — background either way. The speck is therefore averaged
/// against a window full of background and pulled most of the way down, where
/// `SelectiveGaussianBlur` leaves it untouched and `median-blur` erases it outright.
///
/// Three filters, three different answers on the same single pixel. Asserted together because
/// that is the fact a user needs when choosing between them, and it is not obvious from any one.
#[test]
fn snn_mean_reduces_a_lone_speck_between_the_other_two_filters() {
    let mut colors = vec![Pixel::rgba(40, 40, 40, 255); 49];
    colors[3 * 7 + 3] = Pixel::rgba(220, 220, 220, 255);
    let centre = (3 * 7 + 3) * 4;

    let speck_under = |filter: Filter| {
        let mut editor = image(7, 7, &colors);
        editor.execute(Command::ApplyFilter { filter }).unwrap();
        i32::from(pixels(&editor)[centre])
    };

    let snn = speck_under(Filter::SnnMean {
        radius: 2,
        edge_policy: EdgePolicy::Clamp,
    });
    let selective = speck_under(Filter::SelectiveGaussianBlur {
        radius: 2,
        max_delta: 30,
        edge_policy: EdgePolicy::Clamp,
    });
    let median = speck_under(Filter::MedianBlur {
        radius: 1,
        edge_policy: EdgePolicy::Clamp,
    });

    assert_eq!(
        selective, 220,
        "selective gaussian leaves the speck untouched"
    );
    assert_eq!(median, 40, "median-blur erases it");
    assert!(
        snn > 40 && snn < 220,
        "SNN lands strictly between the two: {snn}"
    );
    // EXACT, because this is the one observable the half-window symmetry controls.
    //
    // Each pair is visited once, so the centre carries weight 1/(1+pairs) = 1/13 at radius 2:
    // (220 + 12*40) / 13 = 54. Visiting the whole window would pick the same nearer member twice
    // and give (220 + 24*40) / 25 = 47. Reverse-verification showed the loose `< 100` bound above
    // passed under BOTH, so it could not tell me the symmetry had been broken — naming the wrong
    // behaviour's value is what makes this assertion worth having.
    assert_eq!(
        snn, 54,
        "the centre's weight is 1/13 here; visiting the whole window would give 47"
    );
}

/// Under `Normalise`, a pair with only one resolvable side uses that side.
///
/// `Normalise` is the one policy whose `channel` returns `None` for an outside sample — the other
/// three answer with a real value, `TransparentBlack` included. So this is the policy that
/// exercises the pair logic's absent-sample branch, and my first version of this test used
/// `TransparentBlack` and failed at 138: under that policy a pair with BOTH sides outside
/// legitimately contributes black, which is the policy doing exactly what its name says rather
/// than a defect.
///
/// Dropping a half-resolvable pair would thin the sample set along every border and lighten the
/// result's edge; inventing the missing side would be worse.
#[test]
fn snn_mean_uses_the_resolvable_side_of_a_border_pair() {
    let colors = vec![Pixel::rgba(200, 200, 200, 255); 25];
    let mut editor = image(5, 5, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::SnnMean {
                radius: 2,
                edge_policy: EdgePolicy::Normalise,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    for index in 0..25 {
        assert_eq!(
            out[index * 4],
            200,
            "pixel {index}: only real samples count, so a uniform field is byte-identical"
        );
    }
}

/// Under `TransparentBlack` the border DOES darken, and that is the policy, not a defect.
///
/// Stated explicitly because it looks like one. A pair whose both sides fall outside resolves to
/// two transparent-black samples, which are equally far from the centre, so one is picked and it
/// is black. Pinning it stops a later reader "fixing" the policy into silence.
#[test]
fn snn_mean_under_transparent_black_darkens_the_border_by_design() {
    let colors = vec![Pixel::rgba(200, 200, 200, 255); 25];
    let mut editor = image(5, 5, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::SnnMean {
                radius: 2,
                edge_policy: EdgePolicy::TransparentBlack,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    assert!(
        out[0] < 200,
        "the corner must darken: outside is a real black sample under this policy, got {}",
        out[0]
    );
    // The middle is far enough in that every pair is interior, so it is untouched.
    assert_eq!(
        out[(2 * 5 + 2) * 4],
        200,
        "the centre's pairs are all interior and must be unaffected"
    );
}

/// An outlier is replaced and everything else is left BYTE-IDENTICAL.
///
/// The defining property, and the opposite polarity to a selective blur: this touches only pixels
/// that differ from their own surroundings by more than the threshold. A clean region must come
/// back untouched, not re-averaged and not nudged.
#[test]
fn noise_reduction_replaces_outliers_and_leaves_the_rest_byte_identical() {
    let mut colors = vec![Pixel::rgba(100, 100, 100, 255); 49];
    colors[3 * 7 + 3] = Pixel::rgba(240, 240, 240, 255);

    let mut editor = image(7, 7, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::NoiseReduction {
                threshold: 15,
                window_size: 1,
                edge_policy: EdgePolicy::Clamp,
            },
        })
        .unwrap();
    let out = pixels(&editor);

    // The speck differed from its blurred self by far more than 15, so it was replaced.
    assert!(
        out[(3 * 7 + 3) * 4] < 180,
        "the outlier must be pulled toward its surroundings, got {}",
        out[(3 * 7 + 3) * 4]
    );

    // Everything NOT adjacent to the speck must be exactly 100. The speck's own neighbours see it
    // in their window, so they are allowed to move; pixels two away are not.
    for y in 0..7usize {
        for x in 0..7usize {
            let far = (x as i32 - 3).abs() > 1 || (y as i32 - 3).abs() > 1;
            if far {
                assert_eq!(
                    out[(y * 7 + x) * 4],
                    100,
                    "({x}, {y}) is clean and must be byte-identical"
                );
            }
        }
    }
}

/// A clean image is untouched entirely, whatever the window.
///
/// No pixel differs from its own blurred value on a uniform field, so nothing crosses the
/// threshold. This is what makes the filter safe to apply to an image that does not need it —
/// unlike every other blur in the group, which always changes something.
#[test]
fn noise_reduction_on_a_clean_image_changes_nothing() {
    let colors = vec![Pixel::rgba(90, 140, 200, 255); 81];
    let mut editor = image(9, 9, &colors);
    let before = pixels(&editor);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::NoiseReduction {
                threshold: 15,
                window_size: 3,
                edge_policy: EdgePolicy::Clamp,
            },
        })
        .unwrap();
    assert_eq!(
        pixels(&editor),
        before,
        "a uniform field has no outliers, so nothing may change"
    );
}

/// The polarity is REPLACE-IF-DISSIMILAR, which is the opposite of the selective blur.
///
/// The two filters are easy to conflate — both compare a pixel against its neighbourhood with a
/// threshold — so this names what each does to the same input. On a hard edge, the selective blur
/// preserves it (the neighbours across are excluded from the average) while this one REPLACES the
/// boundary pixels, because they are exactly the pixels that differ most from their own blurred
/// surroundings.
#[test]
fn noise_reduction_polarity_is_opposite_to_the_selective_blur() {
    let mut colors = Vec::new();
    for _ in 0..9 {
        for x in 0..9 {
            let v = if x < 4 { 40u8 } else { 220 };
            colors.push(Pixel::rgba(v, v, v, 255));
        }
    }

    let step_of = |filter: Filter| {
        let mut editor = image(9, 9, &colors);
        editor.execute(Command::ApplyFilter { filter }).unwrap();
        let out = pixels(&editor);
        i32::from(out[(4 * 9 + 4) * 4]) - i32::from(out[(4 * 9 + 3) * 4])
    };

    let selective = step_of(Filter::SelectiveGaussianBlur {
        radius: 2,
        max_delta: 30,
        edge_policy: EdgePolicy::Clamp,
    });
    let reduction = step_of(Filter::NoiseReduction {
        threshold: 15,
        window_size: 2,
        edge_policy: EdgePolicy::Clamp,
    });

    assert!(selective > 170, "the selective blur preserves the edge");
    assert!(
        reduction < selective - 50,
        "noise reduction replaces the boundary pixels instead: {reduction} against {selective}"
    );
}

/// The threshold is the control: raising it past the outlier's deviation spares it.
///
/// Names the value the wrong behaviour produces. The speck here deviates from its blurred self by
/// about 120, so a threshold of 200 must leave it exactly 240 while a threshold of 15 must not.
#[test]
fn noise_reduction_threshold_above_the_deviation_spares_the_outlier() {
    let mut colors = vec![Pixel::rgba(100, 100, 100, 255); 49];
    colors[3 * 7 + 3] = Pixel::rgba(240, 240, 240, 255);

    let speck_under = |threshold: u8| {
        let mut editor = image(7, 7, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::NoiseReduction {
                    threshold,
                    window_size: 1,
                    edge_policy: EdgePolicy::Clamp,
                },
            })
            .unwrap();
        pixels(&editor)[(3 * 7 + 3) * 4]
    };

    assert_eq!(
        speck_under(200),
        240,
        "above the deviation the speck must be left exactly alone"
    );
    assert!(
        speck_under(15) < 180,
        "and below it the speck must be replaced, got {}",
        speck_under(15)
    );
}

/// The window is CIRCULAR, as Krita's mask generator makes it.
///
/// A square window would pull in the corners, which sit sqrt(2) further out and belong to a
/// different neighbourhood than the one the window size names. Discriminated by placing an outlier
/// only at the diagonal corners of a radius-1 window: under a circular mask those corners are
/// excluded, so the centre's blurred value is unaffected by them and it is not replaced.
#[test]
fn noise_reduction_window_is_circular_not_square() {
    let mut colors = vec![Pixel::rgba(100, 100, 100, 255); 49];
    // The four diagonal neighbours of the centre, which a radius-1 CIRCLE excludes (distance
    // sqrt(2) > 1) but a 3x3 square includes.
    for (x, y) in [(2usize, 2usize), (4, 2), (2, 4), (4, 4)] {
        colors[y * 7 + x] = Pixel::rgba(255, 255, 255, 255);
    }

    let mut editor = image(7, 7, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::NoiseReduction {
                threshold: 15,
                window_size: 1,
                edge_policy: EdgePolicy::Clamp,
            },
        })
        .unwrap();
    let out = pixels(&editor);

    assert_eq!(
        out[(3 * 7 + 3) * 4],
        100,
        "the diagonals are outside a radius-1 circle, so the centre must be untouched; a square \
         window would have averaged them in and replaced it"
    );
}

/// Window size 0 is a no-op, and it is ALLOWED — unlike every radius in this crate.
///
/// Krita's declared range is 0..10 with 0 permitted: a one-pixel window makes the blur the
/// identity, so no pixel can differ from it. That range is read from the source this filter is
/// derived from, where `validate_radius`'s refusal of zero is this crate's own convention, so the
/// source wins here. Above the declared maximum is refused.
#[test]
fn noise_reduction_window_zero_is_an_allowed_no_op() {
    let mut colors = vec![Pixel::rgba(100, 100, 100, 255); 49];
    colors[3 * 7 + 3] = Pixel::rgba(240, 240, 240, 255);

    let mut editor = image(7, 7, &colors);
    let before = pixels(&editor);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::NoiseReduction {
                threshold: 15,
                window_size: 0,
                edge_policy: EdgePolicy::Clamp,
            },
        })
        .expect("zero is inside upstream's own range and must be accepted");
    assert_eq!(
        pixels(&editor),
        before,
        "a one-pixel window is the identity blur, so nothing can exceed the threshold"
    );

    let mut too_big = image(7, 7, &colors);
    assert!(
        too_big
            .execute(Command::ApplyFilter {
                filter: Filter::NoiseReduction {
                    threshold: 15,
                    window_size: 11,
                    edge_policy: EdgePolicy::Clamp,
                },
            })
            .is_err(),
        "past upstream's declared maximum of 10 must be refused"
    );
}

/// A uniform field has no edges, so the difference is zero everywhere.
///
/// Both blurs of a flat field are the same flat field, so they cancel exactly. Names the value the
/// wrong behaviour gives: if the two radii were not actually applied separately — say both blurs
/// used `radius1` — this would still pass, so the test below discriminates that instead.
#[test]
fn difference_of_gaussians_on_a_flat_field_is_zero() {
    let colors = vec![Pixel::rgba(120, 90, 200, 255); 81];
    let mut editor = image(9, 9, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::DifferenceOfGaussians {
                radius1: 1.0,
                radius2: 3.0,
                normalize: false,
                invert: false,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    for index in 0..81 {
        for channel in 0..3 {
            assert_eq!(
                out[index * 4 + channel],
                0,
                "pixel {index} channel {channel}: two blurs of a flat field cancel exactly"
            );
        }
    }
}

/// The two radii must be applied SEPARATELY, so equal radii give exactly zero.
///
/// This is the test that catches a single-blur implementation. With `radius1 == radius2` the two
/// blurs are identical and the difference is zero on ANY image, edges included — so a non-zero
/// result anywhere proves the two sides were not the same blur.
#[test]
fn difference_of_gaussians_with_equal_radii_is_zero_even_on_an_edge() {
    let mut colors = Vec::new();
    for _ in 0..9 {
        for x in 0..9 {
            let v = if x < 4 { 20u8 } else { 230 };
            colors.push(Pixel::rgba(v, v, v, 255));
        }
    }
    let mut editor = image(9, 9, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::DifferenceOfGaussians {
                radius1: 2.0,
                radius2: 2.0,
                normalize: false,
                invert: false,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    for index in 0..81 {
        assert_eq!(
            out[index * 4],
            0,
            "pixel {index}: identical radii must cancel, edge or not"
        );
    }
}

/// It finds the edge, and the response is BIPOLAR — a dark band and a bright band straddling it.
///
/// The defining behaviour, and the property that separates this from a gradient-magnitude edge
/// detector: a gradient gives one bright ridge at the boundary, while subtracting two blurs gives a
/// trough on one side and a peak on the other. The measured row across a hard 20/230 edge is
/// `[123, 121, 115, 105, 93, 73, 47, 14, 0, 63, 192, 255, 241, ...]` — minimum hard against the
/// boundary on the dark side, maximum hard against it on the light side.
///
/// My first version of this test asserted that two columns in the flat interior read ALIKE, and it
/// failed at 121 against 105. That premise was wrong: the wide blur still reaches the image border
/// out there, and the two blurs feel it by different amounts, so the far field decays toward the
/// neutral value without ever becoming flat. The response IS monotone away from the edge, which is
/// what this now asserts instead.
#[test]
fn difference_of_gaussians_response_is_bipolar_across_the_edge() {
    let mut colors = Vec::new();
    for _ in 0..21 {
        for x in 0..21 {
            let v = if x < 10 { 20u8 } else { 230 };
            colors.push(Pixel::rgba(v, v, v, 255));
        }
    }
    let mut editor = image(21, 21, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::DifferenceOfGaussians {
                radius1: 1.0,
                radius2: 4.0,
                normalize: true,
                invert: false,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    let at = |x: usize| i32::from(out[(10 * 21 + x) * 4]);

    // The extremes sit immediately either side of the boundary, not out in the field.
    assert_eq!(
        at(8),
        0,
        "the trough is hard against the edge on the dark side"
    );
    assert_eq!(
        at(11),
        255,
        "and the peak hard against it on the light side"
    );

    // Both lobes exist and straddle the far-field value. A single-signed response — what a
    // gradient magnitude would give — could not satisfy both of these at once.
    let far = at(0);
    assert!(
        at(8) < far && at(11) > far,
        "the two lobes must straddle the far field: {} and {} against {far}",
        at(8),
        at(11)
    );

    // And the response decays with distance on BOTH sides, rather than wandering.
    assert!(
        at(1) > at(4) && at(4) > at(7),
        "the dark lobe must deepen toward the edge: {} {} {}",
        at(1),
        at(4),
        at(7)
    );
    assert!(
        at(20) < at(16) && at(16) < at(13),
        "and the bright lobe must decay away from it: {} {} {}",
        at(20),
        at(16),
        at(13)
    );
}

/// `normalize` is what makes the output visible, and without it the result is nearly black.
///
/// Names both values. The raw signed difference on a photograph-like gradient is small and centred
/// on zero, so clamping leaves almost nothing; the stretch maps the real extremes onto 0 and 255.
#[test]
fn difference_of_gaussians_normalize_recovers_the_clamped_range() {
    let mut colors = Vec::new();
    for y in 0..17i32 {
        for x in 0..17i32 {
            // A gentle gradient, the case where the raw difference is small.
            let v = (120 + (x - 8) * 3 + (y - 8)).clamp(0, 255) as u8;
            colors.push(Pixel::rgba(v, v, v, 255));
        }
    }

    let brightest = |normalize: bool| {
        let mut editor = image(17, 17, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::DifferenceOfGaussians {
                    radius1: 1.0,
                    radius2: 5.0,
                    normalize,
                    invert: false,
                },
            })
            .unwrap();
        let out = pixels(&editor);
        (0..17 * 17)
            .map(|index| out[index * 4])
            .max()
            .expect("non-empty")
    };

    let raw = brightest(false);
    let stretched = brightest(true);
    assert!(
        stretched > raw,
        "the stretch must recover range the clamp discarded: {stretched} against {raw}"
    );
    assert_eq!(
        stretched, 255,
        "and it must reach the top of the range by construction"
    );
}

/// `invert` is applied AFTER the stretch, which is the only order in which it does anything.
///
/// The ordering is load-bearing and easy to get wrong. Inverting before normalising would hand the
/// stretch the same pair of extremes, map them to the same pair of ends, and the flag would be a
/// no-op whenever `normalize` was on. So with both flags set the result must be the complement of
/// the un-inverted one, not identical to it.
#[test]
fn difference_of_gaussians_invert_applies_after_the_stretch() {
    let mut colors = Vec::new();
    for _ in 0..15 {
        for x in 0..15 {
            let v = if x < 7 { 30u8 } else { 210 };
            colors.push(Pixel::rgba(v, v, v, 255));
        }
    }

    let sample = |invert: bool| {
        let mut editor = image(15, 15, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::DifferenceOfGaussians {
                    radius1: 1.0,
                    radius2: 4.0,
                    normalize: true,
                    invert,
                },
            })
            .unwrap();
        let out = pixels(&editor);
        (0..15usize)
            .map(|x| i32::from(out[(7 * 15 + x) * 4]))
            .collect::<Vec<i32>>()
    };

    let plain = sample(false);
    let inverted = sample(true);
    for (x, (a, b)) in plain.iter().zip(inverted.iter()).enumerate() {
        assert_eq!(
            a + b,
            255,
            "column {x}: inverted must be the complement, so the pair sums to 255"
        );
    }
}

/// The invert ORDER is observable only with `normalize` off, and this is that case.
///
/// Added because reverse-verification passed without it. Negating the raw difference before the
/// stretch gives `(high - raw)/(high - low)`, the exact complement of `(raw - low)/(high - low)` —
/// so with `normalize` on the two orders are mathematically identical and no input can tell them
/// apart. My first test used `normalize: true` and therefore could not fail.
///
/// With `normalize` off the two diverge, and a flat field names both values: the raw difference is
/// 0, so complementing after the clamp gives **255** while negating before it gives **0**.
#[test]
fn difference_of_gaussians_invert_without_normalize_whitens_a_flat_field() {
    let colors = vec![Pixel::rgba(140, 140, 140, 255); 49];
    let mut editor = image(7, 7, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::DifferenceOfGaussians {
                radius1: 1.0,
                radius2: 3.0,
                normalize: false,
                invert: true,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    for index in 0..49 {
        assert_eq!(
            out[index * 4],
            255,
            "pixel {index}: a flat field differences to 0, and inverting AFTER the clamp makes \
             that white; negating BEFORE it would leave this 0"
        );
    }
}

/// `radius2 > radius1` is accepted, because the dialog accepts it.
///
/// The two are presented symmetrically upstream and swapping them only flips the sign, which
/// `invert` exists to flip back. Refusing the order would reject a state upstream allows.
#[test]
fn difference_of_gaussians_accepts_either_radius_order() {
    let colors = vec![Pixel::rgba(100, 100, 100, 255); 25];
    for (r1, r2) in [(1.0, 4.0), (4.0, 1.0)] {
        let mut editor = image(5, 5, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::DifferenceOfGaussians {
                    radius1: r1,
                    radius2: r2,
                    normalize: false,
                    invert: false,
                },
            })
            .unwrap_or_else(|_| panic!("radii {r1}/{r2} must be accepted"));
    }
}

/// Both radii are validated against the same bounds the blur filter uses.
#[test]
fn difference_of_gaussians_refuses_a_radius_the_blur_filter_would_refuse() {
    let colors = vec![Pixel::rgba(100, 100, 100, 255); 25];
    for (r1, r2) in [(0.0, 2.0), (2.0, 0.0), (-1.0, 2.0), (2.0, 2_000.0)] {
        let mut editor = image(5, 5, &colors);
        assert!(
            editor
                .execute(Command::ApplyFilter {
                    filter: Filter::DifferenceOfGaussians {
                        radius1: r1,
                        radius2: r2,
                        normalize: false,
                        invert: false,
                    },
                })
                .is_err(),
            "radii {r1}/{r2} must be refused, as GaussianBlur refuses them"
        );
    }
}

/// A saved command without the two flags still deserialises, defaulting to off.
#[test]
fn difference_of_gaussians_deserialises_without_the_flags() {
    let json = r#"{"kind":"difference_of_gaussians","radius1":1.0,"radius2":3.0}"#;
    let filter: Filter = serde_json::from_str(json).expect("older saved commands must still load");
    match filter {
        Filter::DifferenceOfGaussians {
            normalize, invert, ..
        } => {
            assert!(!normalize, "normalize must default off");
            assert!(!invert, "invert must default off");
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

/// A flat field is byte-identical: no rule fires, so all nine subpixels are the centre.
#[test]
fn antialias_leaves_a_flat_field_byte_identical() {
    let colors = vec![Pixel::rgba(70, 130, 180, 255); 49];
    let mut editor = image(7, 7, &colors);
    let before = pixels(&editor);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Antialias,
        })
        .unwrap();
    assert_eq!(
        pixels(&editor),
        before,
        "with no diagonal structure every subpixel is the centre"
    );
}

/// A STRAIGHT edge is left completely alone, and this is the algorithm's defining restraint.
///
/// Scale3X's corner rules each need two equal neighbours whose opposite pair is unequal. A vertical
/// boundary satisfies the first clause and fails the last, so no rule fires anywhere along it. The
/// edge stays perfectly hard.
///
/// Names the wrong behaviour's value: anything that blurred instead of extrapolating would put
/// intermediate values in the boundary columns, so every pixel here would stop being exactly 20 or
/// exactly 230.
#[test]
fn antialias_leaves_a_straight_vertical_edge_perfectly_hard() {
    let mut colors = Vec::new();
    for _ in 0..9 {
        for x in 0..9 {
            let v = if x < 4 { 20u8 } else { 230 };
            colors.push(Pixel::rgba(v, v, v, 255));
        }
    }
    let mut editor = image(9, 9, &colors);
    let before = pixels(&editor);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Antialias,
        })
        .unwrap();
    assert_eq!(
        pixels(&editor),
        before,
        "a straight edge fails every Scale3X rule and must come back untouched"
    );
}

/// A straight HORIZONTAL edge is likewise untouched.
#[test]
fn antialias_leaves_a_straight_horizontal_edge_perfectly_hard() {
    let mut colors = Vec::new();
    for y in 0..9 {
        for _ in 0..9 {
            let v = if y < 4 { 20u8 } else { 230 };
            colors.push(Pixel::rgba(v, v, v, 255));
        }
    }
    let mut editor = image(9, 9, &colors);
    let before = pixels(&editor);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Antialias,
        })
        .unwrap();
    assert_eq!(
        pixels(&editor),
        before,
        "the rules are orientation-symmetric: a horizontal edge is untouched too"
    );
}

/// A DIAGONAL staircase does get softened — the contrast that makes the filter worth having.
///
/// This is the pair to the straight-edge tests. Same two colours, same hard boundary, but arranged
/// as a 45-degree step: now the corner rules fire and the averaged subpixels put intermediate
/// values along the diagonal. A filter that left this alone too would be doing nothing at all.
#[test]
fn antialias_softens_a_diagonal_staircase() {
    let mut colors = Vec::new();
    for y in 0..9usize {
        for x in 0..9usize {
            let v = if x < y { 20u8 } else { 230 };
            colors.push(Pixel::rgba(v, v, v, 255));
        }
    }
    let mut editor = image(9, 9, &colors);
    let before = pixels(&editor);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Antialias,
        })
        .unwrap();
    let out = pixels(&editor);

    assert_ne!(out, before, "a diagonal must actually be softened");

    // At least one pixel must now hold a value that is neither of the two originals -- that is
    // what "antialiased" means here, and a filter that only ever swapped whole pixels around
    // could not produce one.
    let intermediate = (0..81)
        .map(|index| out[index * 4])
        .filter(|value| *value != 20 && *value != 230)
        .count();
    assert!(
        intermediate > 0,
        "the averaged subpixels must introduce intermediate coverage"
    );
}

/// The nine subpixels are AVERAGED, not sampled — so a softened pixel lies between the two colours.
///
/// Discriminates averaging from picking: if the implementation returned a single subpixel instead
/// of the mean, every output pixel would still be exactly 20 or 230 and the test above would be
/// the only thing that failed. This pins the value into the open interval.
#[test]
fn antialias_averages_the_subpixels_rather_than_picking_one() {
    let mut colors = Vec::new();
    for y in 0..9usize {
        for x in 0..9usize {
            let v = if x < y { 20u8 } else { 230 };
            colors.push(Pixel::rgba(v, v, v, 255));
        }
    }
    let mut editor = image(9, 9, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Antialias,
        })
        .unwrap();
    let out = pixels(&editor);

    let softened: Vec<u8> = (0..81)
        .map(|index| out[index * 4])
        .filter(|value| *value != 20 && *value != 230)
        .collect();
    assert!(
        !softened.is_empty(),
        "there must be softened pixels to check"
    );
    for value in softened {
        assert!(
            value > 20 && value < 230,
            "a softened pixel must lie strictly between the two colours, got {value}"
        );
    }
}

/// Exact equality is the comparison, so near-but-unequal colours are left alone.
///
/// Scale3X is defined on exact matches, which is why this filter is for hard-edged art and nearly
/// inert on a photograph. Stated as a test so the property is pinned rather than merely described:
/// a diagonal whose two sides differ by one count in one channel triggers no rule at all.
#[test]
fn antialias_needs_exact_equality_so_near_colours_do_not_match() {
    let mut colors = Vec::new();
    for y in 0..9usize {
        for x in 0..9usize {
            // Every pixel slightly different from every other, so no two are ever bit-equal.
            let v = (100 + y * 9 + x) as u8;
            colors.push(Pixel::rgba(v, v, v, 255));
        }
    }
    let mut editor = image(9, 9, &colors);
    let before = pixels(&editor);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Antialias,
        })
        .unwrap();
    assert_eq!(
        pixels(&editor),
        before,
        "no two neighbours are bit-equal, so no Scale3X rule can fire"
    );
}

/// The third clause of each corner rule is load-bearing, and only a NOTCH can prove it.
///
/// Added because reverse-verification passed without it. Dropping `!same(d, h)` from the `db` rule
/// does not change a straight vertical edge at all: there `d` and `e` are both on the dark side and
/// bit-equal, so the rule substitutes a pixel of the IDENTICAL colour and nothing is observable.
/// The straight-edge tests above therefore cannot see the clause.
///
/// This input can. The centre is a lone pixel differing from its up/left/down neighbours, which are
/// all equal, while its right neighbour differs from them — so `d == b`, `b != f` and `d == h` all
/// hold at once, which is exactly the combination the third clause exists to reject.
///
/// Both values named: the centre must stay **40**, and dropping the clause makes exactly three of
/// the nine subpixels 200 — E0 takes `d`, E1 takes `b`, E3 takes `d`, and the other six stay at the
/// centre — giving `(3·200 + 6·40)/9` = **93**.
///
/// I first predicted 111 here, from four substituted subpixels rather than three, and the injected
/// defect returned 93. The assertion was unaffected, but the prediction being visible is the only
/// reason the miscount surfaced at all.
#[test]
fn antialias_third_clause_rejects_a_notch_that_is_not_a_diagonal() {
    let mut colors = vec![Pixel::rgba(200, 200, 200, 255); 25];
    colors[2 * 5 + 2] = Pixel::rgba(40, 40, 40, 255);
    colors[2 * 5 + 3] = Pixel::rgba(100, 100, 100, 255);

    let mut editor = image(5, 5, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Antialias,
        })
        .unwrap();
    let out = pixels(&editor);

    assert_eq!(
        out[(2 * 5 + 2) * 4],
        40,
        "d == h here, so no corner rule may fire; without the third clause this reads 93"
    );
}

/// It is parameterless, and deserialises from a bare tag.
#[test]
fn antialias_deserialises_from_a_bare_tag() {
    let filter: Filter =
        serde_json::from_str(r#"{"kind":"antialias"}"#).expect("a parameterless filter tag");
    assert!(matches!(filter, Filter::Antialias));
}

/// `focus_blur` with nothing but the kind loads upstream's defaults — and one that is NOT upstream's
/// number on purpose.
///
/// # Nothing pinned these before, which is why correcting them broke no test
///
/// Every existing focus-blur test builds the filter through the `focus` helper and names each
/// parameter, so the serde defaults were never exercised. That is the third filter in a row where
/// K.17f found the same shape: the defaults were wrong AND unasserted, and the Qt bridge sends
/// exactly this minimal JSON for any field its dialog does not expose.
///
/// `radius` and `focus` are plain corrections. **`aspect_ratio` is deliberately NOT upstream's
/// 0.0**, and that is the point worth keeping: upstream's is a signed bias on
/// `value_range (-1.0, +1.0)` which its own code turns into a scale —
/// `scale = 1.0 - ratio` when non-negative, `1.0 / (1.0 + ratio)` below zero — so its `0.0` is the
/// neutral circle. Ours IS that scale, applied as a divisor, so our neutral is `1.0`. They are the
/// same state, and setting ours to 0.0 would be refused by our own validation because zero would
/// divide by zero.
#[test]
fn focus_blur_deserialises_with_upstreams_defaults() {
    let filter: Filter =
        serde_json::from_str(r#"{"kind":"focus_blur","blur_radius":4}"#).expect("deserialise");

    let Filter::FocusBlur {
        x,
        y,
        radius,
        aspect_ratio,
        rotation,
        focus,
        midpoint,
        ..
    } = filter
    else {
        panic!("expected a focus blur");
    };

    assert_eq!((x, y), (0.5, 0.5), "upstream's Center X/Y are both 0.5");
    assert_eq!(radius, 0.75, "was 0.5");
    assert_eq!(focus, 0.25, "was 0.0 -- no sharp core at all");
    assert_eq!(midpoint, 0.5, "upstream's Midpoint, unchanged");
    assert_eq!(rotation, 0.0, "upstream's Rotation, unchanged");
    assert_eq!(
        aspect_ratio, 1.0,
        "ours stays 1.0: it is upstream's `scale`, not its signed bias"
    );

    // And the correction is visible. `focus` is the inner limit of the sharp core, so at 0.25 a
    // ring of pixels around the centre must come through untouched; at the old 0.0 the ramp began
    // at the very centre and only the single centre pixel was exactly sharp.
    let colors = speckled(21, 21);
    let mut editor = image(21, 21, &colors);
    editor.execute(Command::ApplyFilter { filter }).unwrap();
    let out = pixels(&editor);

    let untouched = (0..21 * 21)
        .filter(|index| out[index * 4] == colors[*index].r)
        .count();
    assert!(
        untouched > 1,
        "a 0.25 sharp core must leave more than the centre pixel alone, got {untouched}"
    );
}
