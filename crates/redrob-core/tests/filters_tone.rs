// SPDX-License-Identifier: GPL-3.0-or-later

//! K.1. Tone and contrast filters.

use redrob_core::{Command, Document, Editor, Filter, Pixel, Rect, SelectionMode};

/// Paints a one-row image from explicit colours, so every test value is visible in the test.
fn row(colors: &[Pixel]) -> Editor {
    let mut editor = Editor::new(Document::new(colors.len() as u32, 1).unwrap()).unwrap();
    for (x, color) in colors.iter().enumerate() {
        editor
            .execute(Command::SelectRectangle {
                rect: Rect::new(x as i32, 0, 1, 1),
                mode: SelectionMode::Replace,
            })
            .unwrap();
        editor.execute(Command::Fill { color: *color }).unwrap();
    }
    editor.execute(Command::ClearSelection).unwrap();
    editor
}

/// `ShadowsHighlights` with the four extra parameters at their NEUTRAL values.
///
/// The neutral set is stated once, here, because cycle 23's whole constraint was that adding four
/// parameters must not change what cycle 22's tests measure: `compress` 0 is no compression, and
/// `whitepoint` 0 no shift. The two `ccorrect` values are 100 — their own neutral is "restore the
/// saturation the tone change cost", not zero, which would wash the adjusted region out.
fn shadows_highlights(shadows: f32, highlights: f32, radius: f32) -> Filter {
    Filter::ShadowsHighlights {
        shadows,
        highlights,
        radius,
        whitepoint: 0.0,
        compress: 0.0,
        shadows_ccorrect: 100.0,
        highlights_ccorrect: 100.0,
    }
}

fn pixels(editor: &Editor) -> Vec<u8> {
    editor.document().layers()[0].pixels().to_vec()
}

/// Stretch-contrast rescales a narrow range to the full one, with exact endpoints.
///
/// The endpoints are what make this the operation rather than "something that brightens": the
/// darkest pixel must land on 0 and the brightest on 255, and a value in between must land
/// proportionally. A test that only asserted "contrast increased" would pass for any curve.
#[test]
fn stretch_contrast_maps_the_darkest_to_black_and_the_brightest_to_white() {
    // A grey ramp confined to 100..=180. Shared range, so all channels move together.
    let mut editor = row(&[
        Pixel::rgba(100, 100, 100, 255),
        Pixel::rgba(140, 140, 140, 255),
        Pixel::rgba(180, 180, 180, 255),
    ]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::StretchContrast { keep_colors: true },
        })
        .unwrap();
    let out = pixels(&editor);

    assert_eq!(out[0], 0, "the darkest pixel must become black");
    assert_eq!(out[8], 255, "the brightest must become white");
    // 140 is exactly half way between 100 and 180, so it lands on 128 (127.5 rounded).
    assert_eq!(out[4], 128, "the middle must land proportionally");
    // Alpha is untouched: it is coverage, not tone.
    assert_eq!((out[3], out[7], out[11]), (255, 255, 255));
}

/// `keep_colors` shares one range across the channels; without it each channel gets its own.
///
/// This is the only real decision in the operation and the two answers are different pictures.
/// Independent stretching moves the channels by different amounts, so it shifts hue — on an image
/// with a colour cast that is a white balance, which is a useful operation and NOT this one.
#[test]
fn keep_colors_decides_whether_the_stretch_can_shift_hue() {
    // A blue-cast image: red and green are confined low, blue is wide.
    let source = [Pixel::rgba(60, 70, 10, 255), Pixel::rgba(80, 90, 240, 255)];

    let mut shared = row(&source);
    shared
        .execute(Command::ApplyFilter {
            filter: Filter::StretchContrast { keep_colors: true },
        })
        .unwrap();
    let shared_out = pixels(&shared);

    let mut independent = row(&source);
    independent
        .execute(Command::ApplyFilter {
            filter: Filter::StretchContrast { keep_colors: false },
        })
        .unwrap();
    let independent_out = pixels(&independent);

    assert_ne!(
        shared_out, independent_out,
        "the two modes must produce different images, or the flag does nothing"
    );

    // Shared: one range of 10..=240 for every channel, so the ORDER of the channels within a pixel
    // is preserved — red still below green, which is what "no hue shift" means here.
    assert!(
        shared_out[0] < shared_out[1],
        "shared stretching must preserve the channel order: {:?}",
        &shared_out[..4]
    );

    // Independent: red's own range is 60..=80 and green's is 70..=90, so BOTH are stretched to the
    // full range and the first pixel's red and green both hit 0 — the cast is removed and the hue
    // with it. That is the behaviour, and it is why this is not the default.
    assert_eq!(
        (independent_out[0], independent_out[1]),
        (0, 0),
        "independent stretching pins each channel's own minimum to black: {:?}",
        &independent_out[..4]
    );
}

/// A flat channel is left alone rather than forced to an extreme.
///
/// Its range is zero, so there is nothing to stretch. Dividing by that span would be a division by
/// zero; forcing it to black or white instead would destroy a deliberately flat image, which is a
/// worse answer than doing nothing.
#[test]
fn a_flat_channel_is_left_alone() {
    let mut editor = row(&[
        Pixel::rgba(90, 40, 200, 255),
        Pixel::rgba(90, 160, 200, 255),
    ]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::StretchContrast { keep_colors: false },
        })
        .unwrap();
    let out = pixels(&editor);

    assert_eq!(out[0], 90, "a flat red channel must keep its value");
    assert_eq!(out[4], 90);
    assert_eq!(out[2], 200, "and so must a flat blue channel");
    // Green had a real range and was stretched.
    assert_eq!((out[1], out[5]), (0, 255));
}

/// Transparent pixels are excluded from the range.
///
/// A fully transparent pixel's stored colour is usually zero. Counting it would peg the minimum at
/// black, and the stretch would then do nothing at all on any image with a transparent border —
/// which is most images that have been cut out.
#[test]
fn transparent_pixels_do_not_set_the_range() {
    let mut editor = row(&[
        // Transparent, with a stored colour of pure black.
        Pixel::rgba(0, 0, 0, 0),
        Pixel::rgba(100, 100, 100, 255),
        Pixel::rgba(180, 180, 180, 255),
    ]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::StretchContrast { keep_colors: true },
        })
        .unwrap();
    let out = pixels(&editor);

    // The visible pixels still span the full range, exactly as without the transparent one.
    assert_eq!(out[4], 0, "the darkest VISIBLE pixel becomes black");
    assert_eq!(out[8], 255);
    // And the transparent pixel stayed transparent.
    assert_eq!(out[3], 0);
}

/// An image that is entirely transparent is left untouched rather than crashing.
///
/// There is no range to measure. Reached by any fully erased layer, so it is not a hypothetical.
#[test]
fn a_fully_transparent_image_is_left_untouched() {
    let mut editor = row(&[Pixel::rgba(0, 0, 0, 0), Pixel::rgba(70, 80, 90, 0)]);
    let before = pixels(&editor);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::StretchContrast { keep_colors: true },
        })
        .unwrap();
    assert_eq!(pixels(&editor), before);
}

/// The HSV stretch leaves HUE untouched while filling saturation and value.
///
/// This is the whole difference from the RGB version and the reason the filter exists. Hue is an
/// angle, and stretching a narrow band of hues across the colour wheel would turn a photograph of
/// autumn leaves into a rainbow — so the test asserts the hues come back unchanged, not merely
/// "close enough", while demanding the RGB values DID change.
///
/// # The consequence this test had to be rewritten to express
///
/// My first version used three colours of which one sat at the saturation MINIMUM, and it failed:
/// that pixel's hue "moved" from 120 to 0. The filter was right and the test was wrong. A stretch
/// maps the minimum to zero by definition, and saturation zero is grey — which has no hue at all.
///
/// So the least-saturated pixel in any image necessarily loses its hue to this filter. That is
/// inherent to the operation rather than a defect, it is not obvious from the name, and it is the
/// kind of thing that would otherwise be discovered by a user wondering why one patch of their
/// image went grey. Asserted here deliberately, alongside hue preservation for the pixels whose
/// saturation survives.
#[test]
fn the_hsv_stretch_fills_saturation_and_value_without_moving_hue() {
    // Saturations: 0.25, 0.33, 0.50. The first is the minimum and must go grey; the other two keep
    // their hues.
    let source = [
        Pixel::rgba(80, 60, 60, 255),
        Pixel::rgba(60, 90, 60, 255),
        Pixel::rgba(60, 60, 120, 255),
    ];
    let mut editor = row(&source);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::StretchContrastHsv,
        })
        .unwrap();
    let out = pixels(&editor);

    // Hue computed here rather than taken from the filter's own helper: a test that reused the
    // implementation's conversion would agree with it even if the conversion were wrong.
    let hue_of = |r: f32, g: f32, b: f32| -> Option<f32> {
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let delta = max - min;
        if delta < 1e-6 {
            // Achromatic: there is no hue to report, and pretending there is one is what made the
            // first version of this test fail against correct behaviour.
            return None;
        }
        let hue = if max == r {
            60.0 * ((g - b) / delta)
        } else if max == g {
            60.0 * ((b - r) / delta + 2.0)
        } else {
            60.0 * ((r - g) / delta + 4.0)
        };
        Some(hue.rem_euclid(360.0))
    };

    // The least-saturated pixel is mapped to saturation zero, so it is grey.
    let first = &out[0..3];
    assert_eq!(
        hue_of(first[0].into(), first[1].into(), first[2].into()),
        None,
        "the least-saturated pixel must become grey, got {first:?}"
    );
    assert_eq!(
        (first[0], first[1], first[2]),
        (first[0], first[0], first[0]),
        "and grey means three equal channels"
    );

    // The other two keep their hues exactly.
    for index in 1..3 {
        let before = source[index];
        let after = &out[index * 4..index * 4 + 3];
        let h0 = hue_of(before.r.into(), before.g.into(), before.b.into())
            .expect("the source pixel is chromatic");
        let h1 = hue_of(after[0].into(), after[1].into(), after[2].into())
            .expect("a pixel above the saturation minimum stays chromatic");
        assert!(
            (h0 - h1).abs() < 2.0,
            "pixel {index}: hue moved from {h0} to {h1}"
        );
        assert_ne!(
            [before.r, before.g, before.b],
            [after[0], after[1], after[2]],
            "pixel {index} did not change at all, so the stretch did nothing"
        );
    }

    // Value filled: the brightest pixel reaches full brightness.
    let brightest = out
        .chunks_exact(4)
        .map(|pixel| pixel[0].max(pixel[1]).max(pixel[2]))
        .max()
        .unwrap();
    assert_eq!(brightest, 255, "value must be stretched to full");
}

/// The HSV stretch and the RGB stretch are different operations.
///
/// If they agreed, one would be redundant. The RGB version moves channels with no regard for what
/// that does to hue; this one cannot touch hue at all.
#[test]
fn the_hsv_and_rgb_stretches_disagree() {
    let source = [Pixel::rgba(90, 60, 70, 255), Pixel::rgba(70, 100, 60, 255)];

    let mut hsv = row(&source);
    hsv.execute(Command::ApplyFilter {
        filter: Filter::StretchContrastHsv,
    })
    .unwrap();

    let mut rgb = row(&source);
    rgb.execute(Command::ApplyFilter {
        filter: Filter::StretchContrast { keep_colors: false },
    })
    .unwrap();

    assert_ne!(
        pixels(&hsv),
        pixels(&rgb),
        "the two stretches must differ, or one of them is redundant"
    );
}

/// A fully transparent image is left untouched by the HSV stretch too.
#[test]
fn the_hsv_stretch_leaves_a_fully_transparent_image_untouched() {
    let mut editor = row(&[Pixel::rgba(30, 40, 50, 0), Pixel::rgba(60, 70, 80, 0)]);
    let before = pixels(&editor);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::StretchContrastHsv,
        })
        .unwrap();
    assert_eq!(pixels(&editor), before);
}

/// Zero shadows and zero highlights is a no-op.
///
/// A filter's neutral setting must do nothing, and this one's upstream PDB registration lists its
/// "defaults" as each parameter's MINIMUM (shadows -100, radius 0.1) — a `g_param_spec` artefact
/// rather than a considered default. Pinning the neutral value here is what keeps that artefact
/// from being copied in as behaviour.
#[test]
fn shadows_highlights_at_zero_changes_nothing() {
    let mut editor = row(&[
        Pixel::rgba(20, 30, 40, 255),
        Pixel::rgba(200, 210, 220, 255),
    ]);
    let before = pixels(&editor);
    editor
        .execute(Command::ApplyFilter {
            filter: shadows_highlights(0.0, 0.0, 4.0),
        })
        .unwrap();
    assert_eq!(pixels(&editor), before);
}

/// Positive shadows lift the dark end and leave the bright end alone.
///
/// The one-sidedness is the point: a control called "shadows" that also moved the highlights would
/// be a brightness slider. The bright pixel is asserted to stay put, not merely to move less.
#[test]
fn positive_shadows_lift_the_dark_end_only() {
    // A wide-apart pair with a radius small enough that each pixel's neighbourhood is its own
    // brightness rather than the image average.
    let mut editor = Editor::new(Document::new(8, 1).unwrap()).unwrap();
    for x in 0..8 {
        let dark = x < 4;
        editor
            .execute(Command::SelectRectangle {
                rect: Rect::new(x, 0, 1, 1),
                mode: SelectionMode::Replace,
            })
            .unwrap();
        editor
            .execute(Command::Fill {
                color: if dark {
                    Pixel::rgba(20, 20, 20, 255)
                } else {
                    Pixel::rgba(240, 240, 240, 255)
                },
            })
            .unwrap();
    }
    editor.execute(Command::ClearSelection).unwrap();
    let before = pixels(&editor);

    editor
        .execute(Command::ApplyFilter {
            filter: shadows_highlights(60.0, 0.0, 1.0),
        })
        .unwrap();
    let after = pixels(&editor);

    // The darkest pixel (well inside the dark run) rose.
    assert!(
        after[0] > before[0] + 20,
        "shadows should lift the dark end: {} -> {}",
        before[0],
        after[0]
    );
    // The brightest pixel (well inside the bright run) is essentially untouched.
    let bright = 7 * 4;
    assert!(
        after[bright].abs_diff(before[bright]) <= 2,
        "shadows must not move the bright end: {} -> {}",
        before[bright],
        after[bright]
    );
}

/// Positive highlights pull the bright end DOWN, because the control recovers detail from white.
///
/// The direction is the part worth pinning. "Highlights" at a positive value brightening would be
/// the opposite of what the name promises, and it is an easy sign error to make — the two controls
/// are mirror images and the mirror is exactly where it goes wrong.
#[test]
fn positive_highlights_recover_the_bright_end_downward() {
    let mut editor = Editor::new(Document::new(8, 1).unwrap()).unwrap();
    for x in 0..8 {
        editor
            .execute(Command::SelectRectangle {
                rect: Rect::new(x, 0, 1, 1),
                mode: SelectionMode::Replace,
            })
            .unwrap();
        editor
            .execute(Command::Fill {
                color: if x < 4 {
                    Pixel::rgba(20, 20, 20, 255)
                } else {
                    Pixel::rgba(240, 240, 240, 255)
                },
            })
            .unwrap();
    }
    editor.execute(Command::ClearSelection).unwrap();
    let before = pixels(&editor);

    editor
        .execute(Command::ApplyFilter {
            filter: shadows_highlights(0.0, 60.0, 1.0),
        })
        .unwrap();
    let after = pixels(&editor);

    let bright = 7 * 4;
    assert!(
        after[bright] + 20 < before[bright],
        "positive highlights must pull the bright end DOWN: {} -> {}",
        before[bright],
        after[bright]
    );
    assert!(
        after[0].abs_diff(before[0]) <= 2,
        "highlights must not move the dark end: {} -> {}",
        before[0],
        after[0]
    );
}

/// The radius makes this a LOCAL operator: the same pixel is treated differently depending on its
/// surroundings.
///
/// This is the whole difference between this filter and a tone curve, and the only way to see it is
/// to put identical pixels in different neighbourhoods. A dark pixel surrounded by brightness is in
/// a bright REGION, so a shadows lift should barely touch it — where a curve would raise both
/// equally.
#[test]
fn the_radius_makes_the_operator_local_rather_than_a_curve() {
    // 16 wide: a dark run, then a single dark pixel marooned in a bright run.
    let mut editor = Editor::new(Document::new(16, 1).unwrap()).unwrap();
    for x in 0..16 {
        let dark = x < 6 || x == 11;
        editor
            .execute(Command::SelectRectangle {
                rect: Rect::new(x, 0, 1, 1),
                mode: SelectionMode::Replace,
            })
            .unwrap();
        editor
            .execute(Command::Fill {
                color: if dark {
                    Pixel::rgba(30, 30, 30, 255)
                } else {
                    Pixel::rgba(230, 230, 230, 255)
                },
            })
            .unwrap();
    }
    editor.execute(Command::ClearSelection).unwrap();

    editor
        .execute(Command::ApplyFilter {
            filter: shadows_highlights(80.0, 0.0, 3.0),
        })
        .unwrap();
    let after = pixels(&editor);

    let in_dark_region = after[2 * 4];
    let marooned = after[11 * 4];
    assert!(
        in_dark_region > marooned + 15,
        "the same dark value must be lifted MORE inside a dark region ({in_dark_region}) than \
         when surrounded by brightness ({marooned}) — otherwise the radius does nothing and this \
         is a tone curve"
    );
}

/// Parameters outside upstream's own ranges are refused by name.
///
/// The ranges are not invented: they are the `g_param_spec_double` bounds in
/// `app/pdb/drawable-color-cmds.c`. Clamping silently instead would let a caller believe a setting
/// of 500 was applied.
#[test]
fn out_of_range_shadows_highlights_parameters_are_refused() {
    use redrob_core::CoreError;

    for filter in [
        shadows_highlights(101.0, 0.0, 4.0),
        shadows_highlights(0.0, -101.0, 4.0),
        shadows_highlights(0.0, 0.0, 1501.0),
        shadows_highlights(0.0, 0.0, 0.0),
        shadows_highlights(f32::NAN, 0.0, 4.0),
    ] {
        let mut editor = row(&[Pixel::rgba(100, 100, 100, 255)]);
        let error = editor
            .execute(Command::ApplyFilter { filter })
            .expect_err("out-of-range parameters must be refused");
        assert!(
            matches!(error, CoreError::InvalidFilterParameter),
            "got {error:?}"
        );
    }
}

/// A split-tone strip, for the parameter tests: a dark run, a midtone run and a bright run, each
/// with real colour so saturation changes are observable.
fn split_strip() -> Editor {
    let mut editor = Editor::new(Document::new(18, 1).unwrap()).unwrap();
    for x in 0..18 {
        let color = match x / 6 {
            0 => Pixel::rgba(50, 30, 20, 255),
            1 => Pixel::rgba(140, 120, 100, 255),
            _ => Pixel::rgba(240, 225, 210, 255),
        };
        editor
            .execute(Command::SelectRectangle {
                rect: Rect::new(x, 0, 1, 1),
                mode: SelectionMode::Replace,
            })
            .unwrap();
        editor.execute(Command::Fill { color }).unwrap();
    }
    editor.execute(Command::ClearSelection).unwrap();
    editor
}

/// Each of the four added parameters is NEUTRAL at its documented resting value.
///
/// This is the constraint the whole cycle was built around: adding four parameters must not change
/// what the previous cycle's tests measure. Asserted directly against the three-parameter
/// behaviour rather than inferred from the older tests still passing — they pass because they were
/// routed through the neutral helper, which is the thing being checked here.
#[test]
fn the_four_added_parameters_are_neutral_at_their_resting_values() {
    let mut neutral = split_strip();
    neutral
        .execute(Command::ApplyFilter {
            filter: shadows_highlights(50.0, 40.0, 3.0),
        })
        .unwrap();

    let mut explicit = split_strip();
    explicit
        .execute(Command::ApplyFilter {
            filter: Filter::ShadowsHighlights {
                shadows: 50.0,
                highlights: 40.0,
                radius: 3.0,
                whitepoint: 0.0,
                compress: 0.0,
                shadows_ccorrect: 100.0,
                highlights_ccorrect: 100.0,
            },
        })
        .unwrap();
    assert_eq!(pixels(&neutral), pixels(&explicit));
}

/// `compress` preserves midtones: it pulls the effect toward the extremes.
///
/// Upstream's blurb is "Compress the effect on shadows/highlights and preserve midtones", so the
/// midtone is where the measurement belongs. A test that only checked the image changed would pass
/// for a parameter that did anything at all.
#[test]
fn compress_preserves_the_midtones() {
    let measure = |compress: f32| -> Vec<u8> {
        let mut editor = split_strip();
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::ShadowsHighlights {
                    shadows: 80.0,
                    highlights: 0.0,
                    radius: 2.0,
                    whitepoint: 0.0,
                    compress,
                    shadows_ccorrect: 100.0,
                    highlights_ccorrect: 100.0,
                },
            })
            .unwrap();
        pixels(&editor)
    };
    let source = pixels(&split_strip());
    let none = measure(0.0);
    let full = measure(100.0);

    // Pixel 8 is inside the midtone run.
    let midtone = 8 * 4;
    let moved_without = none[midtone].abs_diff(source[midtone]);
    let moved_with = full[midtone].abs_diff(source[midtone]);
    assert!(
        moved_with < moved_without,
        "compress must move the midtone LESS: {moved_with} vs {moved_without}"
    );

    // And the shadow end is still worked on, so compress is not simply turning the filter off.
    // Pixel 1, inside the shadow run.
    let shadow = 4;
    assert!(
        full[shadow] > source[shadow] + 10,
        "compress must keep the shadow effect: {} -> {}",
        source[shadow],
        full[shadow]
    );
}

/// `ccorrect` at zero leaves the lifted region desaturated; at 100 it restores the saturation.
///
/// The tone lift compresses the differences BETWEEN channels, so it desaturates as a side effect.
/// That is why this control exists, and measuring saturation rather than RGB is the only way to see
/// it — the RGB values differ under both settings.
#[test]
fn shadows_ccorrect_restores_the_saturation_the_lift_costs() {
    let saturation_at = |ccorrect: f32, index: usize| -> f64 {
        let mut editor = split_strip();
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::ShadowsHighlights {
                    shadows: 80.0,
                    highlights: 0.0,
                    radius: 2.0,
                    whitepoint: 0.0,
                    compress: 0.0,
                    shadows_ccorrect: ccorrect,
                    highlights_ccorrect: 100.0,
                },
            })
            .unwrap();
        let out = pixels(&editor);
        let pixel = &out[index * 4..index * 4 + 3];
        let max = pixel[0].max(pixel[1]).max(pixel[2]);
        let min = pixel[0].min(pixel[1]).min(pixel[2]);
        if max == 0 {
            0.0
        } else {
            f64::from(max - min) / f64::from(max)
        }
    };

    let source = pixels(&split_strip());
    let original = {
        let pixel = &source[4..7];
        let max = pixel[0].max(pixel[1]).max(pixel[2]);
        let min = pixel[0].min(pixel[1]).min(pixel[2]);
        f64::from(max - min) / f64::from(max)
    };

    let washed = saturation_at(0.0, 1);
    let restored = saturation_at(100.0, 1);

    assert!(
        washed < original,
        "the lift desaturates: {washed:.3} vs original {original:.3}"
    );
    assert!(
        restored > washed + 0.02,
        "ccorrect must put saturation back: {restored:.3} vs {washed:.3}"
    );
    assert!(
        restored <= original + 0.01,
        "and must not push PAST the original, which would make it a vibrance slider: \
         {restored:.3} vs {original:.3}"
    );
}

/// `whitepoint` shifts where white lands, in both directions.
#[test]
fn whitepoint_shifts_the_white_point() {
    let brightest = |whitepoint: f32| -> u8 {
        let mut editor = split_strip();
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::ShadowsHighlights {
                    shadows: 0.0,
                    highlights: 0.0,
                    radius: 2.0,
                    whitepoint,
                    compress: 0.0,
                    shadows_ccorrect: 100.0,
                    highlights_ccorrect: 100.0,
                },
            })
            .unwrap();
        pixels(&editor)
            .chunks_exact(4)
            .map(|pixel| pixel[0].max(pixel[1]).max(pixel[2]))
            .max()
            .unwrap()
    };
    let source_brightest = 240u8;
    assert!(
        brightest(-10.0) < source_brightest,
        "a negative whitepoint must pull white down, got {}",
        brightest(-10.0)
    );
    assert_eq!(brightest(0.0), source_brightest, "and zero must do nothing");
    assert!(
        brightest(10.0) > source_brightest,
        "a positive whitepoint must push white up, got {}",
        brightest(10.0)
    );
}

/// Each new parameter is refused outside upstream's own range.
#[test]
fn out_of_range_added_parameters_are_refused() {
    use redrob_core::CoreError;

    let base = |whitepoint: f32, compress: f32, sc: f32, hc: f32| Filter::ShadowsHighlights {
        shadows: 0.0,
        highlights: 0.0,
        radius: 4.0,
        whitepoint,
        compress,
        shadows_ccorrect: sc,
        highlights_ccorrect: hc,
    };
    for filter in [
        base(10.1, 0.0, 100.0, 100.0),
        base(-10.1, 0.0, 100.0, 100.0),
        base(0.0, 100.1, 100.0, 100.0),
        base(0.0, -0.1, 100.0, 100.0),
        base(0.0, 0.0, 100.1, 100.0),
        base(0.0, 0.0, 100.0, -0.1),
        base(f32::INFINITY, 0.0, 100.0, 100.0),
    ] {
        let mut editor = row(&[Pixel::rgba(100, 90, 80, 255)]);
        let error = editor
            .execute(Command::ApplyFilter { filter })
            .expect_err("out-of-range parameters must be refused");
        assert!(
            matches!(error, CoreError::InvalidFilterParameter),
            "got {error:?}"
        );
    }
}

/// Color-enhance stretches SATURATION to full range, leaving hue and value alone.
///
/// Leaving value alone is what separates this from the HSV stretch — a filter called "colour
/// enhance" that also changed brightness would be doing two things under one name. So the test
/// measures all three channels of HSV, not just that the image changed.
///
/// # I walked into cycle 21's trap again
///
/// Cycle 21 recorded that a saturation stretch maps the MINIMUM to zero, and saturation zero is
/// grey, which has no hue. I wrote this test fresh instead of reusing that pattern, asserted hue
/// preservation on all three pixels, and it failed on the least-saturated one exactly as before
/// (hue "moved" 12 → 0). Recording it because having the lesson written down one cycle earlier
/// did not stop me repeating it — a reusable helper would have, where a note did not.
///
/// Three pixels now, so the hue assertion has subjects whose saturation survives.
#[test]
fn color_enhance_stretches_saturation_only() {
    // Saturations roughly 0.25, 0.40 and 0.55; values deliberately different from each other so a
    // value change would be visible.
    let source = [
        Pixel::rgba(100, 80, 75, 255),
        Pixel::rgba(200, 140, 120, 255),
        Pixel::rgba(160, 90, 72, 255),
    ];
    let mut editor = row(&source);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ColorEnhance,
        })
        .unwrap();
    let out = pixels(&editor);

    // The SHARED helper (tests/common/hsv.rs), not a fourth inline copy. Its `Option<f64>` hue is
    // what stops this test repeating cycles 21 and 24's trap, and `chromatic_hue` names the pixel
    // in its panic so a future failure says which one went grey.
    let hsv = hsv_helper::hsv;

    // Value must not move on ANY pixel, including the one that goes grey.
    for index in 0..3 {
        let before = hsv(source[index].r, source[index].g, source[index].b);
        let after = hsv(out[index * 4], out[index * 4 + 1], out[index * 4 + 2]);
        assert!(
            (before.value - after.value).abs() < 0.01,
            "pixel {index}: VALUE must not move, {} -> {} — that is what separates this from the \
             HSV stretch",
            before.value,
            after.value
        );
    }

    // The least-saturated pixel is mapped to zero saturation, so it has no hue left. That is the
    // operation, not a defect.
    assert_eq!(
        hsv(out[0], out[1], out[2]).hue,
        None,
        "the least-saturated pixel becomes grey"
    );

    // Hue preserved on the two whose saturation survives.
    for index in 1..3 {
        let before = hsv(source[index].r, source[index].g, source[index].b)
            .chromatic_hue(&format!("source pixel {index}"));
        let after = hsv(out[index * 4], out[index * 4 + 1], out[index * 4 + 2])
            .chromatic_hue(&format!("result pixel {index}"));
        assert!(
            (before - after).abs() < 2.0,
            "pixel {index}: hue must not move, {before} -> {after}"
        );
    }

    // Saturation filled at the top end too.
    let most = hsv(out[8], out[9], out[10]).saturation;
    assert!(most > 0.99, "the most saturated must reach 1, got {most}");
}

/// On a GREYSCALE document the filter is refused by name, not silently inert.
///
/// This is upstream's own rule, read from vendored source: `filters-actions.c:1056` disables the
/// action with `writable && !force_nde && !gray`. Twelve chroma filters carry that `!gray` guard,
/// so it is a classification rather than an accident.
///
/// Refusing matters because a filter that runs and changes nothing is indistinguishable from one
/// that is broken — upstream greys the menu item out precisely so the user is told.
#[test]
fn color_enhance_is_refused_on_a_greyscale_document() {
    use redrob_core::{ColorMode, CoreError, DitherMode};

    let mut editor = row(&[
        Pixel::rgba(100, 80, 75, 255),
        Pixel::rgba(200, 140, 120, 255),
    ]);
    editor
        .execute(Command::ConvertColorMode {
            mode: ColorMode::Grayscale,
            palette: None,
            dither: DitherMode::None,
        })
        .unwrap();
    let before = pixels(&editor);

    let error = editor
        .execute(Command::ApplyFilter {
            filter: Filter::ColorEnhance,
        })
        .expect_err("a chroma filter on a greyscale document must be refused");
    assert!(
        matches!(error, CoreError::FilterRequiresColor("color_enhance")),
        "got {error:?}"
    );
    assert_eq!(
        pixels(&editor),
        before,
        "and the refusal must leave the image alone"
    );
}

/// An RGB document whose pixels happen to be grey is NOT refused.
///
/// The guard is on the document's declared MODE, which is what upstream's `!gray` tests, not on
/// whether the pixels currently have colour. A filter that inspected the pixels would refuse on an
/// RGB document the user is about to paint colour into, which is a different and wrong rule.
#[test]
fn color_enhance_is_allowed_on_an_rgb_document_that_looks_grey() {
    let mut editor = row(&[
        Pixel::rgba(90, 90, 90, 255),
        Pixel::rgba(180, 180, 180, 255),
    ]);
    let before = pixels(&editor);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ColorEnhance,
        })
        .expect("an RGB document must be accepted whatever its pixels look like");
    // Nothing to stretch — every saturation is zero — so the image is unchanged, which is correct
    // and is NOT the same as being refused.
    assert_eq!(pixels(&editor), before);
}

/// A fully transparent image is left untouched.
#[test]
fn color_enhance_leaves_a_fully_transparent_image_untouched() {
    let mut editor = row(&[Pixel::rgba(40, 20, 10, 0), Pixel::rgba(90, 70, 50, 0)]);
    let before = pixels(&editor);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ColorEnhance,
        })
        .unwrap();
    assert_eq!(pixels(&editor), before);
}

#[path = "common/hsv.rs"]
mod hsv_helper;

/// A high pass keeps edges and throws away the flat areas.
///
/// Both halves matter: a flat region must come back at mid-grey (no detail there to keep), and an
/// edge must come back away from mid-grey in both directions. A test that only checked "the image
/// changed" would pass for a filter that merely flattened everything to 128.
#[test]
fn high_pass_keeps_the_edge_and_discards_the_flat_area() {
    // Eight dark pixels then eight bright ones: one edge in the middle, flat on both sides.
    let mut editor = Editor::new(Document::new(16, 1).unwrap()).unwrap();
    for x in 0..16 {
        editor
            .execute(Command::SelectRectangle {
                rect: Rect::new(x, 0, 1, 1),
                mode: SelectionMode::Replace,
            })
            .unwrap();
        editor
            .execute(Command::Fill {
                color: if x < 8 {
                    Pixel::rgba(40, 40, 40, 255)
                } else {
                    Pixel::rgba(210, 210, 210, 255)
                },
            })
            .unwrap();
    }
    editor.execute(Command::ClearSelection).unwrap();

    editor
        .execute(Command::ApplyFilter {
            filter: Filter::HighPass {
                std_dev: 2.0,
                contrast: 1.0,
            },
        })
        .unwrap();
    let out = pixels(&editor);

    // Far from the edge there is no detail, so the result is mid-grey.
    assert!(
        out[0].abs_diff(128) <= 3,
        "a flat region must come back at mid-grey, got {}",
        out[0]
    );
    assert!(
        out[15 * 4].abs_diff(128) <= 3,
        "and so must the other flat side, got {}",
        out[15 * 4]
    );

    // At the edge the detail is strong, and SIGNED: the dark side goes below mid-grey and the
    // bright side above it. One-sided detail would mean the offset is wrong and half the signal is
    // being clamped away.
    let dark_side = out[7 * 4];
    let bright_side = out[8 * 4];
    assert!(
        dark_side < 110,
        "the dark side of the edge must fall below mid-grey, got {dark_side}"
    );
    assert!(
        bright_side > 146,
        "and the bright side must rise above it, got {bright_side}"
    );
    // Alpha untouched.
    assert_eq!(out[3], 255);
}

/// `contrast` scales the extracted detail, and zero flattens the result to mid-grey.
///
/// Zero is the interesting end: with no contrast there is no detail to show, so the whole image
/// must be uniform mid-grey. That is a stronger statement than "the image got duller".
#[test]
fn high_pass_contrast_scales_the_detail() {
    let strength_at = |contrast: f32| -> u8 {
        let mut editor = Editor::new(Document::new(16, 1).unwrap()).unwrap();
        for x in 0..16 {
            editor
                .execute(Command::SelectRectangle {
                    rect: Rect::new(x, 0, 1, 1),
                    mode: SelectionMode::Replace,
                })
                .unwrap();
            editor
                .execute(Command::Fill {
                    color: if x < 8 {
                        Pixel::rgba(40, 40, 40, 255)
                    } else {
                        Pixel::rgba(210, 210, 210, 255)
                    },
                })
                .unwrap();
        }
        editor.execute(Command::ClearSelection).unwrap();
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::HighPass {
                    std_dev: 2.0,
                    contrast,
                },
            })
            .unwrap();
        // How far the edge pixel departs from mid-grey.
        pixels(&editor)[8 * 4].abs_diff(128)
    };

    assert_eq!(
        strength_at(0.0),
        0,
        "zero contrast must leave a uniform mid-grey, with no detail at all"
    );
    let low = strength_at(1.0);
    let high = strength_at(3.0);
    assert!(low > 10, "unit contrast must show the edge, got {low}");
    assert!(
        high > low,
        "more contrast must show MORE detail: {high} vs {low}"
    );
}

/// High-pass is allowed on a greyscale document, unlike the chroma filters.
///
/// Upstream carries no `!gray` sensitivity guard for it, where twelve chroma filters do. Pinning
/// the absence is as much a behavioural claim as pinning the presence was for `color-enhance` —
/// and it stops a future "all filters need colour" guard being applied too widely.
#[test]
fn high_pass_is_allowed_on_a_greyscale_document() {
    use redrob_core::{ColorMode, DitherMode};

    let mut editor = Editor::new(Document::new(16, 1).unwrap()).unwrap();
    for x in 0..16 {
        editor
            .execute(Command::SelectRectangle {
                rect: Rect::new(x, 0, 1, 1),
                mode: SelectionMode::Replace,
            })
            .unwrap();
        editor
            .execute(Command::Fill {
                color: if x < 8 {
                    Pixel::rgba(40, 40, 40, 255)
                } else {
                    Pixel::rgba(210, 210, 210, 255)
                },
            })
            .unwrap();
    }
    editor.execute(Command::ClearSelection).unwrap();
    editor
        .execute(Command::ConvertColorMode {
            mode: ColorMode::Grayscale,
            palette: None,
            dither: DitherMode::None,
        })
        .unwrap();

    editor
        .execute(Command::ApplyFilter {
            filter: Filter::HighPass {
                std_dev: 2.0,
                contrast: 1.0,
            },
        })
        .expect("high-pass has no !gray guard upstream and must be allowed here");
    let out = pixels(&editor);
    assert!(
        out[8 * 4].abs_diff(128) > 10,
        "and it must actually extract the edge, got {}",
        out[8 * 4]
    );
}

/// Parameters outside their ranges are refused.
#[test]
fn out_of_range_high_pass_parameters_are_refused() {
    use redrob_core::CoreError;

    for filter in [
        Filter::HighPass {
            std_dev: 0.0,
            contrast: 1.0,
        },
        Filter::HighPass {
            std_dev: 1501.0,
            contrast: 1.0,
        },
        Filter::HighPass {
            std_dev: 2.0,
            contrast: -0.1,
        },
        Filter::HighPass {
            std_dev: 2.0,
            contrast: 10.1,
        },
        Filter::HighPass {
            std_dev: f32::NAN,
            contrast: 1.0,
        },
    ] {
        let mut editor = row(&[Pixel::rgba(100, 100, 100, 255)]);
        let error = editor
            .execute(Command::ApplyFilter { filter })
            .expect_err("out-of-range parameters must be refused");
        assert!(
            matches!(error, CoreError::InvalidFilterParameter),
            "got {error:?}"
        );
    }
}

/// The shared HSV helper reports grey as having NO hue.
///
/// Guards the helper that exists to stop cycles 21 and 24's repeated trap. If it ever returned a
/// number for grey, the saturation tests in K.2 would start passing for the wrong reason.
#[test]
fn the_shared_hsv_helper_refuses_to_invent_a_hue_for_grey() {
    assert_eq!(hsv_helper::hsv(128, 128, 128).hue, None);
    assert_eq!(hsv_helper::hsv(0, 0, 0).hue, None);
    assert!(hsv_helper::hsv(200, 40, 40).hue.is_some());
    // And it agrees with the obvious cases.
    let red = hsv_helper::hsv(255, 0, 0);
    assert_eq!(red.hue, Some(0.0));
    assert!((red.saturation - 1.0).abs() < 1e-9);
    assert!((red.value - 1.0).abs() < 1e-9);
}

/// An EXR carrying samples ABOVE 1.0, which is the only way out-of-range values reach a document.
///
/// Worth stating because finding this out changed the test: there is no command that writes raw
/// layer pixels, every filter that could overflow is refused at F32, and the compositor works in
/// clamped unit floats. So an imported float file is the sole producer — which is also exactly how
/// such values arrive in practice, from a renderer or an HDR capture.
fn exr_with_out_of_range_samples() -> Vec<u8> {
    use image::Rgba;
    use std::io::Cursor;

    let mut buffer: image::ImageBuffer<Rgba<f32>, Vec<f32>> = image::ImageBuffer::new(2, 1);
    // Pixel 0: red well above the range, green inside it. Pixel 1: red below zero.
    buffer.put_pixel(0, 0, Rgba([2.5, 0.5, 0.25, 1.0]));
    buffer.put_pixel(1, 0, Rgba([-0.75, 0.5, 0.25, 1.0]));
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgba32F(buffer)
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::OpenExr)
        .expect("the EXR encoder must accept float RGBA");
    bytes
}

/// rgb-clip actually clips at F32, where out-of-range samples can exist.
///
/// This is the only K.1 filter whose point IS the precision work. At 8- and 16-bit an encoding
/// cannot hold a value outside 0..1 at all, so the operation is inert there by construction; at
/// F32 `Precision::write_sample` stores a raw `f32` with no clamping, so the values genuinely
/// exist.
#[test]
fn rgb_clip_clips_out_of_range_samples_at_f32() {
    use redrob_core::precision::Precision;
    use redrob_core::{ImportOptions, LossPolicy, import_document};

    let imported = import_document(
        &exr_with_out_of_range_samples(),
        &ImportOptions::default().with_loss_policy(LossPolicy::AllowLoss),
    )
    .expect("a float EXR must import");
    let mut editor = Editor::new(imported.document().clone()).unwrap();
    assert_eq!(
        editor.document().precision(),
        Precision::F32,
        "a float EXR must import AS float, or there is nothing out of range to clip"
    );

    // The import must have preserved the out-of-range values; if it clamped them the filter would
    // have nothing to do and this test would pass for the wrong reason.
    let source = editor.document().layers()[0].pixels().to_vec();
    assert!(
        Precision::F32.read_sample(&source, 0) > 1.0,
        "the import clamped the high sample, so this test cannot measure clipping"
    );
    assert!(
        Precision::F32.read_sample(&source, 4) < 0.0,
        "the import clamped the low sample"
    );

    editor
        .execute(Command::ApplyFilter {
            filter: Filter::RgbClip {
                clip_low: true,
                clip_high: true,
                low_limit: 0.0,
                high_limit: 1.0,
            },
        })
        .unwrap();

    let out = editor.document().layers()[0].pixels().to_vec();
    assert_eq!(
        Precision::F32.read_sample(&out, 0),
        1.0,
        "2.5 must be clipped to the high limit"
    );
    assert_eq!(
        Precision::F32.read_sample(&out, 4),
        0.0,
        "-0.75 must be clipped to the low limit"
    );
    // A sample already in range is untouched.
    let untouched = Precision::F32.read_sample(&out, 1);
    assert!(
        (untouched - 0.5).abs() < 0.01,
        "an in-range sample must survive, got {untouched}"
    );
}

/// `clip_low` and `clip_high` are independent, and the limits are not assumed to be 0 and 1.
///
/// GEGL exposes both a flag and a limit per end. A filter that only ever clamped to 0..1 would
/// make all four parameters decoration.
#[test]
fn rgb_clip_honours_each_end_independently() {
    use redrob_core::precision::Precision;
    use redrob_core::{ImportOptions, LossPolicy, import_document};

    let prepared = || {
        let imported = import_document(
            &exr_with_out_of_range_samples(),
            &ImportOptions::default().with_loss_policy(LossPolicy::AllowLoss),
        )
        .unwrap();
        Editor::new(imported.document().clone()).unwrap()
    };

    // High only: the low sample is left out of range.
    let mut editor = prepared();
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::RgbClip {
                clip_low: false,
                clip_high: true,
                low_limit: 0.0,
                high_limit: 1.0,
            },
        })
        .unwrap();
    let out = editor.document().layers()[0].pixels().to_vec();
    assert_eq!(Precision::F32.read_sample(&out, 0), 1.0);
    assert!(
        Precision::F32.read_sample(&out, 4) < 0.0,
        "with clip_low off the low sample must stay out of range"
    );

    // Non-unit limits: the filter must use them rather than assuming 0..1.
    let mut editor = prepared();
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::RgbClip {
                clip_low: true,
                clip_high: true,
                low_limit: -0.5,
                high_limit: 2.0,
            },
        })
        .unwrap();
    let out = editor.document().layers()[0].pixels().to_vec();
    assert_eq!(
        Precision::F32.read_sample(&out, 0),
        2.0,
        "the high limit must be honoured, not hard-coded to 1"
    );
    assert_eq!(
        Precision::F32.read_sample(&out, 4),
        -0.5,
        "and so must the low limit"
    );
}

/// At 8-bit the filter is INERT, because the encoding cannot hold an out-of-range value.
///
/// Worth pinning rather than leaving implied: it explains why the filter exists at all, and it is
/// the observable consequence of being precision-native. A version routed through the byte path
/// would narrow the buffer first — and the narrowing itself clips, so the filter would look like
/// it worked while the conversion had already destroyed everything above the limit.
#[test]
fn rgb_clip_is_inert_at_eight_bit() {
    let mut editor = row(&[Pixel::rgba(0, 128, 255, 255), Pixel::rgba(40, 90, 200, 255)]);
    let before = pixels(&editor);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::RgbClip {
                clip_low: true,
                clip_high: true,
                low_limit: 0.0,
                high_limit: 1.0,
            },
        })
        .unwrap();
    assert_eq!(
        pixels(&editor),
        before,
        "every 8-bit sample is already in range, so there is nothing to clip"
    );
}

/// A low limit above the high limit is refused rather than silently swapped.
///
/// Swapping would apply a range the caller did not ask for, and a filter that quietly reinterprets
/// its parameters cannot be reasoned about from the call site.
#[test]
fn rgb_clip_refuses_an_inverted_range() {
    use redrob_core::CoreError;

    let mut editor = row(&[Pixel::rgba(100, 100, 100, 255)]);
    let error = editor
        .execute(Command::ApplyFilter {
            filter: Filter::RgbClip {
                clip_low: true,
                clip_high: true,
                low_limit: 0.8,
                high_limit: 0.2,
            },
        })
        .expect_err("an inverted range must be refused");
    assert!(
        matches!(error, CoreError::InvalidFilterParameter),
        "got {error:?}"
    );
}
