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

/// value-invert complements brightness and leaves hue alone.
///
/// Hue preservation is the claim that makes this a different filter from an RGB invert, so it is
/// measured through the shared HSV helper rather than inferred from the pixels changing.
#[test]
fn value_invert_complements_brightness_and_keeps_hue() {
    // A dark saturated red and a light desaturated blue, so both directions are exercised.
    let source = [
        Pixel::rgba(90, 20, 20, 255),
        Pixel::rgba(180, 190, 230, 255),
    ];
    let mut editor = row(&source);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ValueInvert,
        })
        .unwrap();
    let out = pixels(&editor);

    for index in 0..2 {
        let before = hsv_helper::hsv(source[index].r, source[index].g, source[index].b);
        let after = hsv_helper::hsv(out[index * 4], out[index * 4 + 1], out[index * 4 + 2]);
        let hue_before = before.chromatic_hue(&format!("source pixel {index}"));
        let hue_after = after.chromatic_hue(&format!("result pixel {index}"));
        assert!(
            (hue_before - hue_after).abs() < 2.0,
            "pixel {index}: hue must survive, {hue_before} -> {hue_after}"
        );
        // Value complemented.
        assert!(
            (after.value - (1.0 - before.value)).abs() < 0.01,
            "pixel {index}: value must be complemented, {} -> {} (expected {})",
            before.value,
            after.value,
            1.0 - before.value
        );
    }
    // The dark pixel became light and the light one dark.
    assert!(out[0] > source[0].r, "the dark pixel must brighten");
    assert!(out[4] < source[1].r, "and the light one must darken");
    assert_eq!(out[3], 255, "alpha untouched");
}

/// value-invert is NOT the same operation as the RGB invert.
///
/// If they agreed, one would be redundant. `Filter::Invert` complements every stored channel;
/// this complements only brightness, which is a different picture whenever the channels differ.
#[test]
fn value_invert_differs_from_the_rgb_invert() {
    let source = [Pixel::rgba(90, 20, 20, 255), Pixel::rgba(30, 140, 200, 255)];

    let mut value = row(&source);
    value
        .execute(Command::ApplyFilter {
            filter: Filter::ValueInvert,
        })
        .unwrap();

    let mut rgb = row(&source);
    rgb.execute(Command::ApplyFilter {
        filter: Filter::Invert,
    })
    .unwrap();

    assert_ne!(
        pixels(&value),
        pixels(&rgb),
        "the two inverts must differ, or one of them is redundant"
    );
}

/// value-invert is NOT an involution, and that is inherent to HSV rather than a defect.
///
/// # What measuring found
///
/// My first version asserted that applying it twice returns the original. It failed, and the
/// probe showed why: **pure red (255,0,0) comes back WHITE.**
///
/// Red has value 1 and saturation 1. Inverting the value gives value 0 — black. But at value 0
/// the colour is a single point in HSV: `saturation = delta / max` has max 0, so saturation and
/// hue are both undefined and our conversion reports saturation 0. Inverting the value again
/// gives value 1 with saturation 0, which is white.
///
/// So the operation destroys hue and saturation for any pixel it drives to value 0, and no
/// filter can recover them — the information is gone at the first application. Upstream's
/// operation has the same property for the same reason; it is a property of the colour space, not
/// of this implementation.
///
/// Asserted deliberately, because it is the kind of thing a user meets as "why did my red turn
/// white" and a test that merely tolerated it would hide it.
#[test]
fn value_invert_is_not_an_involution_at_the_extremes() {
    let mut editor = row(&[Pixel::rgba(255, 0, 0, 255)]);
    for _ in 0..2 {
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::ValueInvert,
            })
            .unwrap();
    }
    let out = pixels(&editor);
    assert_eq!(
        (out[0], out[1], out[2]),
        (255, 255, 255),
        "red -> black -> white: value 0 has no hue or saturation to invert back"
    );

    // And the intermediate really is black, so the loss happens on the FIRST application.
    let mut once = row(&[Pixel::rgba(255, 0, 0, 255)]);
    once.execute(Command::ApplyFilter {
        filter: Filter::ValueInvert,
    })
    .unwrap();
    let mid = pixels(&once);
    assert_eq!((mid[0], mid[1], mid[2]), (0, 0, 0));
}

/// Away from those extremes it DOES round-trip, within measured quantisation.
///
/// The tolerance is 4 because that is what the drift was measured to be on a double application
/// — two HSV conversions each way through 8-bit channels — not a number chosen until the test
/// passed. The probe that established it reported 4 and 3 on a light blue and 0 elsewhere.
#[test]
fn value_invert_round_trips_away_from_the_extremes() {
    let source = [
        Pixel::rgba(90, 20, 20, 255),
        Pixel::rgba(180, 190, 230, 255),
        Pixel::rgba(10, 200, 120, 255),
    ];
    let mut editor = row(&source);
    let before = pixels(&editor);
    for _ in 0..2 {
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::ValueInvert,
            })
            .unwrap();
    }
    let after = pixels(&editor);
    for (index, (a, b)) in before.iter().zip(after.iter()).enumerate() {
        assert!(
            a.abs_diff(*b) <= 4,
            "byte {index} drifted more than the measured quantisation: {a} -> {b}"
        );
    }
    // And it is genuinely close, not merely inside a loose bound: most bytes return exactly.
    let exact = before
        .iter()
        .zip(after.iter())
        .filter(|(a, b)| a == b)
        .count();
    assert!(
        exact * 2 >= before.len(),
        "at least half the bytes should return exactly, got {exact} of {}",
        before.len()
    );
}

/// It is allowed on a greyscale document, and inverts it.
///
/// No `!gray` guard upstream, and that is consistent: inverting VALUE is meaningful on a grey
/// image where inverting saturation would not be. Pinning the absence stops a future
/// "inverts need colour" guard being applied too widely.
#[test]
fn value_invert_is_allowed_on_a_greyscale_document() {
    use redrob_core::{ColorMode, DitherMode};

    let mut editor = row(&[
        Pixel::rgba(40, 40, 40, 255),
        Pixel::rgba(200, 200, 200, 255),
    ]);
    editor
        .execute(Command::ConvertColorMode {
            mode: ColorMode::Grayscale,
            palette: None,
            dither: DitherMode::None,
        })
        .unwrap();
    let before = pixels(&editor);

    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ValueInvert,
        })
        .expect("value-invert has no !gray guard upstream and must be allowed");
    let after = pixels(&editor);

    assert!(
        after[0] > before[0] + 100,
        "the dark grey must become light, {} -> {}",
        before[0],
        after[0]
    );
    assert!(
        after[4] + 100 < before[4],
        "and the light grey dark, {} -> {}",
        before[4],
        after[4]
    );
}

/// invert-linear and the gamma invert differ on a mid-tone, and agree at the endpoints.
///
/// Both halves are the point. If they agreed everywhere one would be redundant; if they disagreed
/// at 0 and 1 then one of them is not a complement at all.
///
/// Mid-grey is where the gap is widest, and the expected value is COMPUTED rather than estimated:
/// 128/255 = 0.5020 encoded, which decodes to 0.2158 linear; its complement 0.7842 re-encodes to
/// 0.8983, i.e. **229**. The gamma invert gives 127. My first version of this test wrote "about
/// 188" from mental arithmetic and asserted only `> 180` — loose enough to pass while the number
/// behind it was wrong, which the deep-document test then caught by asserting a tight range.
#[test]
fn invert_linear_differs_from_the_gamma_invert_on_midtones() {
    let source = [
        Pixel::rgba(0, 0, 0, 255),
        Pixel::rgba(128, 128, 128, 255),
        Pixel::rgba(255, 255, 255, 255),
    ];

    let mut linear = row(&source);
    linear
        .execute(Command::ApplyFilter {
            filter: Filter::InvertLinear,
        })
        .unwrap();
    let linear_out = pixels(&linear);

    let mut gamma = row(&source);
    gamma
        .execute(Command::ApplyFilter {
            filter: Filter::Invert,
        })
        .unwrap();
    let gamma_out = pixels(&gamma);

    // Endpoints agree: a complement must send 0 to 1 and 1 to 0 in either space.
    assert_eq!(linear_out[0], 255, "black must invert to white");
    assert_eq!(linear_out[8], 0, "and white to black");
    assert_eq!(linear_out[0], gamma_out[0]);
    assert_eq!(linear_out[8], gamma_out[8]);

    // Mid-tone diverges, and by a lot — this is the whole reason both filters exist.
    let mid_linear = linear_out[4];
    let mid_gamma = gamma_out[4];
    assert_eq!(
        mid_gamma, 127,
        "the gamma invert complements the stored value"
    );
    assert!(
        mid_linear.abs_diff(229) <= 1,
        "the linear invert of mid-grey is 229, got {mid_linear}"
    );
    assert!(
        mid_linear.abs_diff(mid_gamma) > 50,
        "the two inverts must diverge clearly on a mid-tone: {mid_linear} vs {mid_gamma}"
    );
}

/// Applied twice it returns the original, unlike value-invert.
///
/// This one IS an involution away from nothing at all: complementing in linear light loses no
/// information, because every channel keeps its own value rather than being folded through a
/// shared one. Contrast value-invert, where value 0 destroys hue and saturation — the comparison
/// is the point, and a tolerance of 2 covers the two encode/decode round trips.
#[test]
fn invert_linear_is_an_involution_including_at_the_extremes() {
    let source = [
        Pixel::rgba(255, 0, 0, 255),
        Pixel::rgba(0, 0, 0, 255),
        Pixel::rgba(128, 64, 200, 255),
        Pixel::rgba(255, 255, 255, 255),
    ];
    let mut editor = row(&source);
    let before = pixels(&editor);
    for _ in 0..2 {
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::InvertLinear,
            })
            .unwrap();
    }
    let after = pixels(&editor);
    for (index, (a, b)) in before.iter().zip(after.iter()).enumerate() {
        assert!(
            a.abs_diff(*b) <= 2,
            "byte {index} did not come back: {a} -> {b}"
        );
    }
    // Pure red survives, where value-invert turns it white.
    assert_eq!((after[0], after[1], after[2]), (255, 0, 0));
}

/// It is precision-native, so it runs on a 16-bit document instead of being refused.
///
/// Its sibling `Filter::Invert` has been native since J.1b. Had this one not been added to the
/// list it would be refused on exactly the deep documents where the difference between the two
/// inverts is most visible, which is the opposite of useful.
#[test]
fn invert_linear_runs_on_a_deep_document() {
    use redrob_core::precision::Precision;

    let mut editor = row(&[Pixel::rgba(128, 128, 128, 255)]);
    editor
        .execute(Command::SetDocumentPrecision {
            precision: Precision::U16,
        })
        .unwrap();
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::InvertLinear,
        })
        .expect("a precision-native filter must not be refused on a deep document");

    let stored = editor.document().layers()[0].pixels().to_vec();
    let sample = Precision::U16.read_sample(&stored, 0);
    assert!(
        (sample - 0.8983).abs() < 0.002,
        "mid-grey must invert to 0.8983 (229/255), got {sample}"
    );
    assert_eq!(
        editor.document().precision(),
        Precision::U16,
        "and the document must still be deep — a narrowed round trip would be the defect"
    );
}

/// A negative F32 sample survives without becoming NaN.
///
/// rgb-clip (cycle 26) established that out-of-range F32 samples genuinely exist, so this is
/// reachable rather than hypothetical. Both transfer functions take their LINEAR branch below the
/// breakpoint, so a negative input never reaches `powf` with a negative base. The complement of an
/// out-of-range value is another out-of-range value, which is correct — clipping it belongs to
/// rgb-clip, not here.
#[test]
fn invert_linear_does_not_produce_nan_from_an_out_of_range_sample() {
    use redrob_core::precision::Precision;
    use redrob_core::{ImportOptions, LossPolicy, import_document};

    let imported = import_document(
        &exr_with_out_of_range_samples(),
        &ImportOptions::default().with_loss_policy(LossPolicy::AllowLoss),
    )
    .unwrap();
    let mut editor = Editor::new(imported.document().clone()).unwrap();
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::InvertLinear,
        })
        .unwrap();

    let out = editor.document().layers()[0].pixels().to_vec();
    for index in 0..8 {
        let sample = Precision::F32.read_sample(&out, index);
        assert!(
            sample.is_finite(),
            "sample {index} came back as {sample}, so a transfer function hit an invalid branch"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// K.10 `gegl:stress` — Spatio Temporal Retinex-like Envelope with Stochastic Sampling.
//
// Ported from `gegl/operations/common/stress.c` and the `envelopes.h` it includes, the first item
// read from GEGL's tree rather than GIMP's. GIMP only names this operator.
// ---------------------------------------------------------------------------------------------

/// `Stress` at upstream's own defaults for everything except what a test varies.
fn stress(radius: u32, samples: u32, iterations: u32, enhance_shadows: bool) -> Filter {
    Filter::Stress {
        radius,
        samples,
        iterations,
        enhance_shadows,
    }
}

fn apply(editor: &mut Editor, filter: Filter) -> Vec<u8> {
    editor.execute(Command::ApplyFilter { filter }).unwrap();
    pixels(editor)
}

/// `enhance_shadows` picks the DIVISOR, and on a flat image the two choices are 128 and 255.
///
/// This is the test that separates the two modes, and it is a pair on purpose. On a flat image
/// every spray has a zero range, so both envelopes collapse onto the pixel:
///
/// * ON divides by `max - min`, which is **zero**, so it takes the zero-divisor branch -> 0.5 ->
///   **128**.
/// * OFF divides by `max` alone, which is the pixel itself, so the quotient is exactly **1.0** ->
///   **255**.
///
/// A "stronger/weaker" reading of the flag cannot produce that pair: it predicts two values on the
/// same side of the input, where the measurement puts one below it and one above.
#[test]
fn stress_enhance_shadows_chooses_the_divisor_not_a_strength() {
    let grey = Pixel::rgba(100, 100, 100, 255);
    let flat = [grey; 8];

    let mut editor = row(&flat);
    let on = apply(&mut editor, stress(4, 5, 5, true));
    let mut editor = row(&flat);
    let off = apply(&mut editor, stress(4, 5, 5, false));

    for x in 0..8 {
        assert_eq!(
            &on[x * 4..x * 4 + 3],
            [128, 128, 128],
            "x {x}: a zero-width envelope must give the 0.5 branch, not the input back"
        );
        assert_eq!(
            &off[x * 4..x * 4 + 3],
            [255, 255, 255],
            "x {x}: dividing by the upper envelope alone is exactly 1.0 on a flat image"
        );
    }
}

/// The zero-divisor branch exists in BOTH modes, which flat black is what proves.
///
/// With `enhance_shadows` off the divisor is the upper envelope, and on a black image that is 0 —
/// so the 0.5 branch is reached by a different route than the test above. Without it this would
/// divide by zero and the whole image would be `NaN`, which rounds to 0 and looks like a
/// plausible result rather than a fault.
///
/// It does NOT pin the mode choice, and the reverse-verification is what established that: forcing
/// both envelopes on leaves this test green, because on flat black `max - min` is zero too and
/// both routes reach 0.5. The test above is the one that separates the modes.
#[test]
fn stress_without_enhance_shadows_still_guards_a_zero_envelope() {
    let black = Pixel::rgba(0, 0, 0, 255);
    let mut editor = row(&[black; 4]);
    let out = apply(&mut editor, stress(4, 5, 5, false));

    for x in 0..4 {
        assert_eq!(
            &out[x * 4..x * 4 + 3],
            [128, 128, 128],
            "x {x}: black has a zero upper envelope, so this is the 0.5 branch"
        );
    }
}

/// A fully transparent neighbour is not in the envelope.
///
/// `sample_min_max` skips any sample whose alpha is 0 and draws another, so a grey pixel beside an
/// empty one must behave like the flat case. The two halves are measured together because the
/// number alone is not the claim — **128 against 51** is.
///
/// Note what the empty pixel actually holds: filling with an alpha of 0 composites nothing, so it
/// stays `(0, 0, 0, 0)`. If the alpha check were dropped it would enter the envelope as BLACK, and
/// the reverse-verification measured what that gives: **204**, not 128. I had reasoned 255 and was
/// wrong — not every spray reaches the neighbour, so the averaged range is smaller than the full
/// one. The number in this comment is the injected measurement, not the prediction.
#[test]
fn stress_ignores_fully_transparent_samples() {
    let grey = Pixel::rgba(100, 100, 100, 255);
    let empty = Pixel::rgba(255, 255, 255, 0);
    let white = Pixel::rgba(255, 255, 255, 255);

    let mut editor = row(&[grey, empty, grey, empty, grey, empty]);
    let ignored = apply(&mut editor, stress(4, 5, 5, true));
    let mut editor = row(&[grey, white, grey, white, grey, white]);
    let counted = apply(&mut editor, stress(4, 5, 5, true));

    assert_eq!(
        &ignored[0..3],
        [128, 128, 128],
        "an empty neighbour must leave the envelope flat"
    );
    assert_eq!(
        &counted[0..3],
        [51, 51, 51],
        "an OPAQUE neighbour of the same colour must change the result, or the test above proves \
         nothing"
    );
}

/// Alpha is carried through untouched.
///
/// Upstream copies `pixel[3]` into the destination unchanged. It is worth an assertion because the
/// operator's own sampling treats alpha as a mask, so "alpha is meaningful here" and "alpha is
/// modified here" are easy to conflate.
#[test]
fn stress_preserves_alpha() {
    let translucent = Pixel::rgba(100, 100, 100, 77);
    let mut editor = row(&[translucent; 4]);
    let out = apply(&mut editor, stress(4, 5, 5, true));

    for x in 0..4 {
        assert_eq!(out[x * 4 + 3], 77, "x {x}: alpha must not move");
    }
}

/// The same document twice gives byte-identical output.
///
/// Upstream cannot assert this: its spray comes from an unseeded PRNG, and `stress.c` carries its
/// real reference hash commented out above `"reference-hash", "unstable"` with the note that it is
/// *not consistent from run to run*. Our table is built from a fixed seed and the counters start at
/// zero per application, so this is the one place our behaviour is deliberately stronger than
/// upstream's rather than equal to it.
#[test]
fn stress_is_deterministic_where_upstream_is_not() {
    let colors = [
        Pixel::rgba(100, 100, 100, 255),
        Pixel::rgba(255, 255, 255, 255),
        Pixel::rgba(0, 0, 0, 255),
        Pixel::rgba(100, 100, 100, 255),
        Pixel::rgba(255, 255, 255, 255),
        Pixel::rgba(0, 0, 0, 255),
    ];

    let mut editor = row(&colors);
    let first = apply(&mut editor, stress(4, 5, 5, true));
    let mut editor = row(&colors);
    let second = apply(&mut editor, stress(4, 5, 5, true));

    assert_eq!(
        first, second,
        "two applications to the same document must agree byte for byte"
    );
}

/// The envelopes average RANGE and RELATIVE BRIGHTNESS, not the per-spray minima and maxima.
///
/// `iterations` is the knob that makes the difference observable: with one spray the two readings
/// of `envelopes.h` agree exactly, and they diverge only once there is something to average. So
/// this asserts a pair of exact outputs at 1 and 9 iterations on the same input.
///
/// The middle value is the sharp one. At one iteration the grey pixel comes back as **100** — its
/// own input, because that single spray put it at the envelope's edge. At nine it is **221**, a
/// value no single spray produces, which is only reachable if the averaged range is anchored back
/// onto the pixel.
#[test]
fn stress_averages_the_envelope_across_iterations() {
    let black = Pixel::rgba(0, 0, 0, 255);
    let grey = Pixel::rgba(100, 100, 100, 255);
    let white = Pixel::rgba(255, 255, 255, 255);
    let colors = [black, grey, white, grey, black, white, grey, black];

    let mut editor = row(&colors);
    let one = apply(&mut editor, stress(4, 5, 1, true));
    let mut editor = row(&colors);
    let nine = apply(&mut editor, stress(4, 5, 9, true));

    assert_eq!(&one[0..3], [128, 128, 128], "one spray, first pixel");
    assert_eq!(
        &one[4..7],
        [100, 100, 100],
        "one spray leaves the grey pixel at its own value"
    );
    assert_eq!(&nine[0..3], [71, 71, 71], "nine sprays, first pixel");
    assert_eq!(
        &nine[4..7],
        [221, 221, 221],
        "nine sprays give a value no single spray reaches"
    );
}

/// Upstream's declared ranges are enforced, and the radius is NOT capped at the convolution limit.
///
/// `value_range` in the property block gives radius `(2, 6000)`, samples `(2, 500)` and iterations
/// `(1, 1000)`. The radius case is the one with a decision in it: `MAX_FILTER_RADIUS` is 4096 and
/// every convolution-shaped filter is held to it, but here the work per pixel is
/// `samples * iterations` whatever the radius, so the cap would reject a value upstream accepts for
/// no cost we actually pay.
#[test]
fn stress_enforces_upstream_ranges_and_accepts_a_radius_past_the_convolution_cap() {
    let grey = Pixel::rgba(100, 100, 100, 255);
    let mut editor = row(&[grey, Pixel::rgba(255, 255, 255, 255), grey, grey]);

    for bad in [
        stress(1, 5, 5, true),
        stress(6_001, 5, 5, true),
        stress(4, 1, 5, true),
        stress(4, 501, 5, true),
        stress(4, 5, 0, true),
        stress(4, 5, 1_001, true),
    ] {
        assert!(
            editor
                .execute(Command::ApplyFilter {
                    filter: bad.clone()
                })
                .is_err(),
            "{bad:?} is outside upstream's declared range and must be refused"
        );
    }

    assert!(
        editor
            .execute(Command::ApplyFilter {
                filter: stress(6_000, 5, 5, true),
            })
            .is_ok(),
        "6000 is upstream's maximum radius and costs no extra work here"
    );
}

// ---------------------------------------------------------------------------------------------
// K.10 `gegl:reinhard05` — Reinhard 2005 tone mapping, a global HDR-to-LDR operator.
//
// Ported from `gegl/operations/common/reinhard05.c`. GIMP only names it. This is the first
// precision-native filter that is not a complement, which is the point: the operator exists to
// compress a range an 8-bit document no longer has.
// ---------------------------------------------------------------------------------------------

fn reinhard(brightness: f64, chromatic: f64, light: f64) -> Filter {
    Filter::Reinhard05 {
        brightness,
        chromatic,
        light,
    }
}

/// A fully black layer is REFUSED, because upstream's own assertion fails there.
///
/// `key` divides `ln(max) - mean ln` by `ln(max) - ln(eps + min)`; at max 0 both are infinite,
/// `contrast` arrives as `NaN`, and upstream's
/// `g_return_val_if_fail (contrast >= 0.3 && contrast <= 1.0)` fails the whole operation. So this
/// is equivalence, not caution — and the error names the IMAGE's problem rather than a parameter's,
/// because the parameters are fine.
#[test]
fn reinhard05_refuses_an_image_with_no_dynamic_range() {
    let black = Pixel::rgba(0, 0, 0, 255);
    let mut editor = row(&[black; 3]);

    let error = editor
        .execute(Command::ApplyFilter {
            filter: reinhard(0.0, 0.0, 1.0),
        })
        .expect_err("a black layer has no luminance range to map");
    assert!(
        matches!(
            error,
            redrob_core::CoreError::FilterNoDynamicRange("reinhard05")
        ),
        "the refusal must name this filter and this cause, got {error:?}"
    );
}

/// A flat image lands on exactly mid-grey, and every step of that is forced.
///
/// `key` is 1 exactly — its numerator and denominator are the same expression once min == max —
/// so `contrast` is `0.3 + 0.7 = 1.0`, which is also the upper bound upstream asserts. With
/// `chromatic` 0 the adaptation is the pixel's own luminance, so the mapping is `p / (p + p)` =
/// **0.5 in linear light**, and `linear_to_srgb(0.5)` encodes to **188**.
///
/// It also pins the one deliberate divergence. Every mapped sample is identical here, so the
/// rescaling range is zero and upstream computes `(p - min) / 0` = `NaN` for the whole image. We
/// leave the mapped values unrescaled. 188 is the value that proves we did not rescale; a `NaN`
/// would encode to 0 and look like a black layer.
#[test]
fn reinhard05_maps_a_flat_image_to_mid_grey_rather_than_nan() {
    let grey = Pixel::rgba(100, 100, 100, 255);
    let mut editor = row(&[grey; 4]);
    let out = apply(&mut editor, reinhard(0.0, 0.0, 1.0));

    for x in 0..4 {
        assert_eq!(
            &out[x * 4..x * 4 + 3],
            [188, 188, 188],
            "x {x}: p/(p+p) is 0.5 in linear light, which encodes to 188"
        );
    }
}

/// The rescale takes the mapped values to the full range, endpoints exact.
///
/// This is the half of the operator that is not the tone curve: after mapping, every channel is
/// shifted and scaled by the min and range of what was just written, so the darkest mapped sample
/// is 0 and the brightest is 255 by construction.
#[test]
fn reinhard05_rescales_the_mapped_values_to_the_full_range() {
    let ramp: Vec<Pixel> = (0..8)
        .map(|i| Pixel::rgba(i * 36, i * 36, i * 36, 255))
        .collect();
    let mut editor = row(&ramp);
    let out = apply(&mut editor, reinhard(0.0, 0.0, 1.0));

    assert_eq!(
        &out[0..3],
        [0, 0, 0],
        "the darkest sample anchors the rescale"
    );
    assert_eq!(
        &out[28..31],
        [255, 255, 255],
        "the brightest sample anchors the other end"
    );
    assert_eq!(
        &out[8..11],
        [122, 122, 122],
        "and the interior is the curve, not a straight line"
    );
}

/// ALPHA IS RESCALED TOO, and that is upstream's arithmetic rather than a defect here.
///
/// Upstream's final loop runs `for (c = 0; c < pix_stride; ++c)` where `pix_stride` is 4, while the
/// statistics it rescales by were gathered over RGB only. So a uniform alpha of 128 does not come
/// back as 128. Asserted rather than corrected, because a silent fix would be a divergence nobody
/// could see; the measured value is **139**.
#[test]
fn reinhard05_rescales_alpha_because_upstream_does() {
    let translucent: Vec<Pixel> = (0..4)
        .map(|i| Pixel::rgba(40 + i * 60, 40 + i * 60, 40 + i * 60, 128))
        .collect();
    let mut editor = row(&translucent);
    let out = apply(&mut editor, reinhard(0.0, 0.0, 1.0));

    for x in 0..4 {
        assert_eq!(
            out[x * 4 + 3],
            139,
            "x {x}: alpha goes through the same (p - min) / range as the colours"
        );
    }
}

/// `brightness` is INVERTED: a positive value lifts the image.
///
/// `intensity = exp(-brightness)`, so raising `brightness` lowers the adaptation the pixel is
/// divided by. The input needs **three or more distinct non-zero luminances** for this to be
/// visible at all: with two, the rescale pins them to 0 and 255 whatever the curve did, and the
/// first probe of this filter measured exactly that and proved nothing.
#[test]
fn reinhard05_brightness_lifts_the_midtones() {
    let ramp = [
        Pixel::rgba(30, 30, 30, 255),
        Pixel::rgba(80, 80, 80, 255),
        Pixel::rgba(140, 140, 140, 255),
        Pixel::rgba(200, 200, 200, 255),
        Pixel::rgba(255, 255, 255, 255),
    ];

    let mut editor = row(&ramp);
    let dark = apply(&mut editor, reinhard(-2.0, 0.0, 1.0));
    let mut editor = row(&ramp);
    let neutral = apply(&mut editor, reinhard(0.0, 0.0, 1.0));
    let mut editor = row(&ramp);
    let bright = apply(&mut editor, reinhard(2.0, 0.0, 1.0));

    assert_eq!(&dark[4..7], [133, 133, 133], "brightness -2");
    assert_eq!(&neutral[4..7], [148, 148, 148], "brightness 0");
    assert_eq!(&bright[4..7], [169, 169, 169], "brightness +2");
}

/// `chromatic` interpolates between the channel and the luminance, so it is inert on a grey image.
///
/// `local = chromatic * p + (1 - chromatic) * Y`. On a grey pixel the channel value and the
/// luminance are the same number, so the interpolation has nothing to interpolate. The pair is the
/// claim: a coloured image must move and a grey one must not.
#[test]
fn reinhard05_chromatic_adapts_colour_and_is_inert_on_grey() {
    let colored = [
        Pixel::rgba(200, 40, 40, 255),
        Pixel::rgba(40, 200, 40, 255),
        Pixel::rgba(40, 40, 200, 255),
    ];
    let mut editor = row(&colored);
    let none = apply(&mut editor, reinhard(0.0, 0.0, 1.0));
    let mut editor = row(&colored);
    let full = apply(&mut editor, reinhard(0.0, 1.0, 1.0));

    assert_eq!(
        &none[0..3],
        [238, 57, 57],
        "chromatic 0 keeps the luminance adaptation"
    );
    assert_eq!(
        &full[0..3],
        [255, 0, 0],
        "chromatic 1 adapts each channel to itself"
    );

    let greys = [
        Pixel::rgba(100, 100, 100, 255),
        Pixel::rgba(255, 255, 255, 255),
        Pixel::rgba(60, 60, 60, 255),
    ];
    let mut editor = row(&greys);
    let grey_none = apply(&mut editor, reinhard(0.0, 0.0, 1.0));
    let mut editor = row(&greys);
    let grey_full = apply(&mut editor, reinhard(0.0, 1.0, 1.0));

    assert_eq!(
        grey_none, grey_full,
        "on a grey image the channel IS the luminance, so chromatic cannot change anything"
    );
}

/// A pixel whose luminance is exactly zero is skipped, then rescaled below zero.
///
/// `if (lum[i] == 0.0) continue;` means the operator never touches it and it never enters the
/// rescaling statistics — but the rescale itself covers every pixel, so it becomes
/// `(0 - min) / range`, which is negative and clamps to 0 at an integer precision. Black stays
/// black, by a longer route than it looks.
///
/// **`light` is 0.5 here and that is the whole reason the test works.** At the default `light` of 1
/// the skip is UNOBSERVABLE: `adapt` collapses to `light_comp * global` = 0 for a black pixel, the
/// mapping computes `0 / 0`, and the resulting `NaN` is then ignored by `f64::min`/`max` and clamps
/// to 0 on the way to a byte — which is the same black the correct code produces. The injection
/// that removed the skip passed against a `light` of 1, and that is what sent me looking.
#[test]
fn reinhard05_skips_a_zero_luminance_pixel_and_then_rescales_it_negative() {
    let black = Pixel::rgba(0, 0, 0, 255);
    let grey = Pixel::rgba(100, 100, 100, 255);
    let white = Pixel::rgba(255, 255, 255, 255);
    let mut editor = row(&[black, grey, white, Pixel::rgba(160, 160, 160, 255)]);
    let out = apply(&mut editor, reinhard(0.0, 0.0, 0.5));

    assert_eq!(
        &out[0..3],
        [0, 0, 0],
        "the skipped pixel clamps back to black"
    );
    assert_eq!(
        &out[12..15],
        [182, 182, 182],
        "a skipped pixel must stay OUT of the rescaling statistics, which is what this value pins"
    );
    // This one is NOT a discriminating assertion and the reverse-verification is what showed it:
    // excluding alpha from the rescale leaves it at 255 too, so 255 proves nothing here. Kept as a
    // regression guard rather than as evidence — `reinhard05_rescales_alpha_because_upstream_does`
    // is where that claim is actually tested.
    assert_eq!(out[3], 255, "opaque either way; see the note above");
}

/// Upstream's declared ranges are enforced, and a non-finite parameter is refused.
#[test]
fn reinhard05_enforces_upstream_ranges() {
    let grey = Pixel::rgba(100, 100, 100, 255);
    let mut editor = row(&[grey, Pixel::rgba(255, 255, 255, 255)]);

    for bad in [
        reinhard(-100.1, 0.0, 1.0),
        reinhard(100.1, 0.0, 1.0),
        reinhard(0.0, -0.1, 1.0),
        reinhard(0.0, 1.1, 1.0),
        reinhard(0.0, 0.0, -0.1),
        reinhard(0.0, 0.0, 1.1),
        reinhard(f64::NAN, 0.0, 1.0),
    ] {
        assert!(
            editor
                .execute(Command::ApplyFilter {
                    filter: bad.clone()
                })
                .is_err(),
            "{bad:?} is outside upstream's declared range and must be refused"
        );
    }
}

/// It runs on a REAL HDR document, which is the whole reason it is precision-native.
///
/// Every other K.10 cycle could have shipped an 8-bit arm and looked identical in a test. This one
/// imports an EXR holding samples outside 0..1, keeps them at `Precision::F32`, and tone-maps them
/// — the input the operator exists for. The assertion is that no sample comes back non-finite:
/// a tone mapper that produces `NaN` on its own intended input is worse than one that refuses.
#[test]
fn reinhard05_tone_maps_a_real_hdr_document() {
    use redrob_core::precision::Precision;
    use redrob_core::{ImportOptions, LossPolicy, import_document};

    let imported = import_document(
        &exr_with_out_of_range_samples(),
        &ImportOptions::default().with_loss_policy(LossPolicy::AllowLoss),
    )
    .unwrap();
    assert_eq!(
        imported.document().precision(),
        Precision::F32,
        "the fixture must stay deep, or this test is an 8-bit test wearing a hat"
    );

    let mut editor = Editor::new(imported.document().clone()).unwrap();
    editor
        .execute(Command::ApplyFilter {
            filter: reinhard(0.0, 0.0, 1.0),
        })
        .expect("an HDR document is exactly this operator's input");

    let out = editor.document().layers()[0].pixels().to_vec();
    for index in 0..8 {
        let sample = Precision::F32.read_sample(&out, index);
        assert!(
            sample.is_finite(),
            "sample {index} came back as {sample}, so the operator produced a non-finite value"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// K.10 `gegl:fattal02` — Fattal/Lischinski/Werman 2002 gradient-domain tone mapping.
//
// Ported from `gegl/operations/common/fattal02.c`. This is the operator K.10's attribution blocker
// was written about: three of the group's four names are literal citations, and reconstructing this
// one from memory would have put my invention under three researchers' names.
//
// EVERY assertion here is STRUCTURAL. Upstream's Poisson solve is a truncated iteration with a
// coarsest level that is literally zeros, so two implementations cannot agree digit for digit and
// no number here is copied from GEGL. See `crate::fattal`'s module docs.
// ---------------------------------------------------------------------------------------------

fn fattal(alpha: f64, beta: f64, saturation: f64, noise: f64) -> Filter {
    Filter::Fattal02 {
        alpha,
        beta,
        saturation,
        noise,
    }
}

/// A `size` by `size` image painted by a closure.
///
/// Built through a PNG round trip rather than by poking the raster, so the test uses only the
/// public surface. The size matters: `fattal02`'s multigrid hierarchy only has a coarse level once
/// the smaller side reaches 16, so a 2x1 fixture would exercise the pipeline with the solver
/// returning zeros and prove almost nothing.
fn fattal_image(size: u32, paint: impl Fn(u32, u32) -> Pixel) -> Editor {
    use std::io::Cursor;

    let mut buffer: image::RgbaImage = image::ImageBuffer::new(size, size);
    for y in 0..size {
        for x in 0..size {
            let pixel = paint(x, y);
            buffer.put_pixel(x, y, image::Rgba([pixel.r, pixel.g, pixel.b, pixel.a]));
        }
    }
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgba8(buffer)
        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
        .expect("the PNG encoder must accept RGBA8");
    Editor::new(redrob_core::import_png(&bytes).expect("our own PNG must import")).unwrap()
}

/// A horizontal grey ramp, the fixture most of these tests use.
fn fattal_ramp(x: u32, _y: u32) -> Pixel {
    let value = (x * 8).min(255) as u8;
    Pixel::rgba(value, value, value, 255)
}

/// A black image is refused: the log-luminance normalisation divides by the maximum luminance.
///
/// Upstream divides without checking, so a black layer gives it a division by zero and then a
/// `NaN` image. Same judgement and the same error as `reinhard05`.
#[test]
fn fattal02_refuses_an_image_with_no_luminance() {
    let mut editor = fattal_image(32, |_, _| Pixel::rgba(0, 0, 0, 255));

    let error = editor
        .execute(Command::ApplyFilter {
            filter: fattal(1.0, 0.9, 0.8, 0.0),
        })
        .expect_err("a black layer has no maximum to normalise by");
    assert!(
        matches!(
            error,
            redrob_core::CoreError::FilterNoDynamicRange("fattal02")
        ),
        "the refusal must name this filter and this cause, got {error:?}"
    );
}

/// A flat image comes back white, and every step of that is forced rather than chosen.
///
/// Flat means zero gradients, so the attenuation field is 1 everywhere — upstream's `grad > 1e-4`
/// guard — the divergence is zero, and the recovered `U` is zero. `exp(0) - 1e-4` is then the same
/// value at every pixel, so the percentile range is zero: upstream divides by it and writes `NaN`
/// across the image, and we leave the values unrescaled instead. `L` is then ~1 and a grey pixel's
/// `C/Y` is exactly 1, so `1^saturation * 1` is white.
#[test]
fn fattal02_maps_a_flat_image_to_white_rather_than_nan() {
    let mut editor = fattal_image(32, |_, _| Pixel::rgba(100, 100, 100, 255));
    let out = apply(&mut editor, fattal(1.0, 0.9, 0.8, 0.0));

    assert_eq!(
        &out[0..3],
        [255, 255, 255],
        "a zero-range recovery must not be rescaled, and must not be NaN"
    );
}

/// A ramp survives the whole pipeline: monotone, finite, and spanning the full range.
///
/// This is the test that proves the solver ran. The claim is deliberately structural — order and
/// endpoints, not pixel values — because a truncated multigrid iteration has no reproducible
/// digits across implementations. What it would catch is the solve collapsing to a constant, or
/// coming back with the gradient reversed.
#[test]
fn fattal02_recovers_a_monotone_ramp_from_its_gradients() {
    let mut editor = fattal_image(32, fattal_ramp);
    let out = apply(&mut editor, fattal(1.0, 0.9, 0.8, 0.0));

    let row: Vec<u8> = (0..32).map(|x| out[(x * 4) as usize]).collect();
    assert_eq!(row[0], 0, "the dark end anchors at zero");
    assert_eq!(row[31], 255, "the bright end anchors at full");
    for x in 1..32 {
        assert!(
            row[x] >= row[x - 1],
            "the recovered image must stay monotone; x {x} fell from {} to {}",
            row[x - 1],
            row[x]
        );
    }
    assert!(
        row[16] > 100 && row[16] < 160,
        "the middle must be in the middle, not pinned to an end; got {}",
        row[16]
    );
}

/// Alpha is UNTOUCHED, which is the opposite of `reinhard05` and read from the same kind of
/// evidence.
///
/// `fattal02`'s `OUTPUT_FORMAT` is `"RGB float"` with `pix_stride` 3, so the operator never sees an
/// alpha channel at all. `reinhard05`'s is `RGBA float` with 4, and it rescales alpha as a result.
/// Two operators in the same group, two different answers, both read rather than assumed — which
/// is why this pair of tests exists instead of one shared convention.
#[test]
fn fattal02_leaves_alpha_alone_where_reinhard05_rescales_it() {
    let mut editor = fattal_image(32, |x, _| {
        let value = (x * 8).min(255) as u8;
        Pixel::rgba(value, value, value, 77)
    });
    let out = apply(&mut editor, fattal(1.0, 0.9, 0.8, 0.0));

    for x in 0..32usize {
        assert_eq!(
            out[x * 4 + 3],
            77,
            "x {x}: this operator has no alpha channel to modify"
        );
    }
}

/// `saturation` 0 discards the hue entirely; 1 keeps the channel ratios.
///
/// The colour step is `(C / Y)^saturation * L`, so at 0 every channel's ratio becomes 1 and the
/// result is grey whatever went in. The pair is the claim — a single value could not tell
/// "saturation works" from "the image was grey already".
#[test]
fn fattal02_saturation_zero_discards_hue_and_one_keeps_it() {
    let colourful = |x: u32, _y: u32| {
        let value = (x * 8).min(255) as u8;
        Pixel::rgba(value, value / 3, 200u8.saturating_sub(value), 255)
    };

    let mut editor = fattal_image(32, colourful);
    let none = apply(&mut editor, fattal(1.0, 0.9, 0.0, 0.0));
    let mut editor = fattal_image(32, colourful);
    let full = apply(&mut editor, fattal(1.0, 0.9, 1.0, 0.0));

    let grey = &none[20 * 4..20 * 4 + 3];
    assert_eq!(
        grey[0], grey[1],
        "saturation 0 must give a grey pixel, got {grey:?}"
    );
    assert_eq!(grey[1], grey[2], "saturation 0 must give a grey pixel");

    let kept = &full[20 * 4..20 * 4 + 3];
    assert!(
        kept[0] != kept[1] || kept[1] != kept[2],
        "saturation 1 must keep the hue, got {kept:?}"
    );
}

/// `noise` 0 is a SENTINEL for `alpha * 0.1`, not an absence of a noise floor.
///
/// The substitution lives in upstream's `process`, not in the property block, so a reader who
/// takes the declared default at face value ships an operator with no noise floor — which
/// amplifies shadow noise, the exact thing the parameter exists to prevent. With `alpha` at 1.0 the
/// sentinel resolves to 0.1, so these two must be byte-identical, and a different value must not
/// be.
#[test]
fn fattal02_noise_zero_means_alpha_over_ten() {
    let textured = |x: u32, y: u32| {
        let value = ((x * 5 + y * 3) % 256) as u8;
        Pixel::rgba(value, value, value, 255)
    };

    let mut editor = fattal_image(32, textured);
    let sentinel = apply(&mut editor, fattal(1.0, 0.9, 0.8, 0.0));
    let mut editor = fattal_image(32, textured);
    let explicit = apply(&mut editor, fattal(1.0, 0.9, 0.8, 0.1));
    let mut editor = fattal_image(32, textured);
    let different = apply(&mut editor, fattal(1.0, 0.9, 0.8, 0.5));

    assert_eq!(
        sentinel, explicit,
        "noise 0 with alpha 1.0 must resolve to exactly 0.1"
    );
    assert_ne!(
        sentinel, different,
        "a real noise value must change the result, or the test above proves nothing"
    );
}

/// `beta` changes the attenuation, and the same filter twice gives the same bytes.
///
/// Two claims in one test because the second is the control for the first: "these two differ"
/// means nothing from an operator that is not reproducible. The probe for this filter got that
/// wrong — it compared two runs with DIFFERENT betas and printed the result as a determinism
/// check, which it is not.
#[test]
fn fattal02_beta_changes_the_result_and_the_operator_is_deterministic() {
    let textured = |x: u32, y: u32| {
        let value = ((x * 5 + y * 3) % 256) as u8;
        Pixel::rgba(value, value, value, 255)
    };

    let mut editor = fattal_image(32, textured);
    let low = apply(&mut editor, fattal(1.0, 0.3, 0.8, 0.0));
    let mut editor = fattal_image(32, textured);
    let high = apply(&mut editor, fattal(1.0, 1.8, 0.8, 0.0));
    let mut editor = fattal_image(32, textured);
    let low_again = apply(&mut editor, fattal(1.0, 0.3, 0.8, 0.0));

    assert_eq!(
        low, low_again,
        "the same filter twice must agree byte for byte"
    );
    assert_ne!(low, high, "beta must change the local detail enhancement");
}

/// Upstream's declared ranges are enforced, and `beta`'s floor is 0.1 rather than 0.
#[test]
fn fattal02_enforces_upstream_ranges_including_betas_floor() {
    let mut editor = fattal_image(32, fattal_ramp);

    for bad in [
        fattal(-0.1, 0.9, 0.8, 0.0),
        fattal(2.1, 0.9, 0.8, 0.0),
        fattal(1.0, 0.0, 0.8, 0.0),
        fattal(1.0, 0.09, 0.8, 0.0),
        fattal(1.0, 2.1, 0.8, 0.0),
        fattal(1.0, 0.9, 1.1, 0.0),
        fattal(1.0, 0.9, 0.8, 1.1),
        fattal(f64::NAN, 0.9, 0.8, 0.0),
    ] {
        assert!(
            editor
                .execute(Command::ApplyFilter {
                    filter: bad.clone()
                })
                .is_err(),
            "{bad:?} is outside upstream's declared range and must be refused"
        );
    }
}

/// It runs at `F32` without producing a non-finite sample.
///
/// The fixture is the same out-of-range EXR the other precision tests use. It is only 2x1, so the
/// multigrid hierarchy has no coarse level and the solve returns zeros — this is a smoke test of
/// the deep path, not of the solver, and saying which is the point.
#[test]
fn fattal02_runs_on_a_deep_document_without_producing_nan() {
    use redrob_core::precision::Precision;
    use redrob_core::{ImportOptions, LossPolicy, import_document};

    let imported = import_document(
        &exr_with_out_of_range_samples(),
        &ImportOptions::default().with_loss_policy(LossPolicy::AllowLoss),
    )
    .unwrap();
    assert_eq!(imported.document().precision(), Precision::F32);

    let mut editor = Editor::new(imported.document().clone()).unwrap();
    editor
        .execute(Command::ApplyFilter {
            filter: fattal(1.0, 0.9, 0.8, 0.0),
        })
        .expect("a deep document must be accepted");

    let out = editor.document().layers()[0].pixels().to_vec();
    for index in 0..8 {
        let sample = Precision::F32.read_sample(&out, index);
        assert!(sample.is_finite(), "sample {index} came back as {sample}");
    }
}

// ---------------------------------------------------------------------------------------------
// K.10 `gegl:mantiuk06` — contrast-domain tone mapping, the last operator in the group and the
// largest filter in this backlog at 1654 upstream lines.
//
// Structural assertions again, for the same reason as `fattal02`: the solve is a truncated
// conjugate gradient with a restart guard, so no number here is copied from GEGL.
// ---------------------------------------------------------------------------------------------

fn mantiuk(contrast: f64, saturation: f64) -> Filter {
    Filter::Mantiuk06 {
        contrast,
        saturation,
    }
}

/// A black image is refused: there is no positive luminance to set the clip floor from.
#[test]
fn mantiuk06_refuses_an_image_with_no_luminance() {
    let mut editor = fattal_image(32, |_, _| Pixel::rgba(0, 0, 0, 255));
    let error = editor
        .execute(Command::ApplyFilter {
            filter: mantiuk(0.1, 0.8),
        })
        .expect_err("no luminance means no clip floor and no logarithm");
    assert!(
        matches!(
            error,
            redrob_core::CoreError::FilterNoDynamicRange("mantiuk06")
        ),
        "got {error:?}"
    );
}

/// An image too small for a single pyramid level is refused rather than quietly doing nothing.
///
/// Upstream's `pyramid_allocate` loops `while (rows >= 3 && cols >= 3)` and returns NULL below
/// that, which its own `contmap` then hands straight to `pyramid_calculate_gradient`. Refusing is
/// the honest reading of "this operator has nothing to work on", and it is a different refusal
/// from the black-image one even though both land on the same error.
#[test]
fn mantiuk06_refuses_an_image_too_small_for_a_pyramid() {
    let mut editor = fattal_image(2, fattal_ramp);
    assert!(
        editor
            .execute(Command::ApplyFilter {
                filter: mantiuk(0.1, 0.8),
            })
            .is_err(),
        "a 2x2 image builds no pyramid level at all"
    );
}

/// A ramp survives the contrast domain: monotone, endpoints at the extremes.
///
/// This is the test that proves the whole chain ran — transducer out, scale, transducer back,
/// divergence, solve, rescale. What it would catch is the transducer inverting, or the solve
/// collapsing.
#[test]
fn mantiuk06_recovers_a_monotone_ramp_through_the_contrast_domain() {
    let mut editor = fattal_image(32, fattal_ramp);
    let out = apply(&mut editor, mantiuk(0.1, 0.8));

    let row: Vec<u8> = (0..32).map(|x| out[(x * 4) as usize]).collect();
    assert_eq!(row[31], 255, "the bright end anchors at full");
    for x in 2..32 {
        assert!(
            row[x] >= row[x - 1],
            "must stay monotone; x {x} fell from {} to {}",
            row[x - 1],
            row[x]
        );
    }
}

/// `contrast` is a multiplier in RESPONSE space, so more of it keeps more contrast.
///
/// Three values on one fixture, asserted as an ordering rather than as pixels: at a quarter of the
/// way along the ramp, 0.05 < 0.1 < 0.9. A compression factor that worked the other way round — or
/// one that did nothing — fails this.
#[test]
fn mantiuk06_more_contrast_keeps_more_contrast() {
    let sample = |contrast: f64| -> u8 {
        let mut editor = fattal_image(32, fattal_ramp);
        let out = apply(&mut editor, mantiuk(contrast, 0.8));
        out[8 * 4]
    };

    let low = sample(0.05);
    let middle = sample(0.1);
    let high = sample(0.9);
    assert!(
        low < middle && middle < high,
        "expected 0.05 < 0.1 < 0.9, got {low}, {middle}, {high}"
    );
}

/// `contrast` exactly 0 zeroes every gradient, and upstream's behaviour there is degenerate.
///
/// Zero is the bottom of the declared range and it is NOT "no compression": it multiplies every
/// gradient in response space by 0, so the right-hand side of the solve is zero, the solve is
/// driven toward a constant image, and the final percentile rescale then amplifies whatever solver
/// residual is left.
///
/// # No byte is pinned here, and CI is what taught me that
///
/// The first version asserted the exact result, `[75, 75, 16, 255]`, measured on this machine. CI
/// produced **`[255, 16, 255, 255]`** from the same commit. Both are "amplified residual of a
/// truncated conjugate gradient", and that residual is not reproducible across machines — single
/// precision, a different CPU and a different optimisation level reorder the last bits, and this
/// path multiplies exactly those up into the visible range.
///
/// So the lesson is narrower than "the test was brittle": **a degenerate path's output is not a
/// fact about the product**, and pinning it asserts a property of the machine that measured it.
/// `mantiuk06_is_deterministic` still holds — the same binary on the same host agrees with itself —
/// which is a different claim from portability, and the two were conflated here.
///
/// What is asserted instead is the portable part: 0 does not behave like a small positive value,
/// because it takes a different arithmetic path. Upstream also branches to a histogram EQUALISATION
/// at this value; that branch is not implemented, and
/// `_CONTRAST_EQUALISATION_IS_UNREACHABLE` in `crate::mantiuk` carries the proof that it cannot
/// differ — both paths produce an all-zero gradient field at a factor of 0.
#[test]
fn mantiuk06_contrast_zero_is_degenerate_rather_than_neutral() {
    let mut editor = fattal_image(32, fattal_ramp);
    let zero = apply(&mut editor, mantiuk(0.0, 0.8));
    let mut editor = fattal_image(32, fattal_ramp);
    let lowest_nonzero = apply(&mut editor, mantiuk(0.05, 0.8));

    assert_ne!(
        zero, lowest_nonzero,
        "0 must not behave like a small positive value"
    );

    // The clean neighbour is the control: without it, "these two differ" would also pass for two
    // degenerate results. It is asserted as a PROPERTY and not as bytes, for exactly the reason
    // this test's own doc comment gives — anything downstream of the truncated solve is a fact
    // about the machine, so pinning 0.05's bytes here would repeat the mistake one line below
    // recording it.
    let clean: Vec<u8> = (0..32).map(|x| lowest_nonzero[(x * 4) as usize]).collect();
    for x in 1..32 {
        assert!(
            clean[x] >= clean[x - 1],
            "0.05 must stay the clean, monotone case; x {x} fell from {} to {}",
            clean[x - 1],
            clean[x]
        );
    }
    assert!(
        clean[31] > clean[0],
        "and it must still span a range, not collapse"
    );
}

/// `saturation` 0 discards hue; 2 pushes it past the input. The ceiling is 2, not 1.
///
/// `fattal02`'s saturation stops at 1 and this one's `value_range` is `(0.0, 2.0)`, which is why
/// the two filters validate it separately instead of sharing a bound. Measured at the same pixel:
/// grey at 0, `[136, 56, 45]` at the 0.8 default, `[255, 27, 13]` at 2 — the channels spread
/// further apart as it rises.
#[test]
fn mantiuk06_saturation_spans_zero_to_two() {
    let colourful = |x: u32, _y: u32| {
        let value = (x * 8).min(255) as u8;
        Pixel::rgba(value, value / 3, 200u8.saturating_sub(value), 255)
    };

    let mut editor = fattal_image(32, colourful);
    let none = apply(&mut editor, mantiuk(0.1, 0.0));
    let mut editor = fattal_image(32, colourful);
    let default = apply(&mut editor, mantiuk(0.1, 0.8));
    let mut editor = fattal_image(32, colourful);
    let doubled = apply(&mut editor, mantiuk(0.1, 2.0));

    let grey = &none[20 * 4..20 * 4 + 3];
    assert_eq!(grey[0], grey[1], "saturation 0 must be grey, got {grey:?}");
    assert_eq!(grey[1], grey[2], "saturation 0 must be grey");

    let spread = |pixel: &[u8]| i32::from(pixel[0]) - i32::from(pixel[2]);
    assert!(
        spread(&doubled[20 * 4..20 * 4 + 3]) > spread(&default[20 * 4..20 * 4 + 3]),
        "saturation 2 must spread the channels further than 0.8"
    );
}

/// Alpha comes through unchanged, which is a THIRD answer in this group.
///
/// `fattal02`'s buffer is `RGB float` so it has no alpha; `reinhard05` rescales alpha with the
/// colours; this one's buffer is `RGBA float` and its clip loop runs over all four components, so
/// alpha is raised to the `1e-7 * max(Y)` floor and otherwise left alone. At a byte that floor is
/// invisible, which is what this measures.
#[test]
fn mantiuk06_leaves_alpha_at_its_input_value() {
    let mut editor = fattal_image(32, |x, _| {
        let value = (x * 8).min(255) as u8;
        Pixel::rgba(value, value, value, 77)
    });
    let out = apply(&mut editor, mantiuk(0.1, 0.8));

    for x in 0..32usize {
        assert_eq!(out[x * 4 + 3], 77, "x {x}: alpha must survive the clip");
    }
}

/// The transducer SATURATES past the top of its table rather than extrapolating.
///
/// # The first version of this test proved nothing, and the injection is what showed it
///
/// It used a checkerboard of RGB 1 against white. That spans about 3.5 log10 units, so the
/// stimulus `10^|G| - 1` reaches roughly 3,200 — comfortably INSIDE the table, whose top entry is
/// 10,510. Replacing the saturating return with a linear extrapolation changed nothing, because
/// the extrapolated branch was never reached.
///
/// Pure BLACK against white is what reaches it: black clips to `1e-7 * max(Y)`, so the span is the
/// full 7 log10 units and the stimulus is about 10,000,000. Measured, saturating gives **253** in
/// the bright cell and extrapolating gives **255** — so `< 255` is the discriminating assertion,
/// and it is a property rather than one of our own arbitrary numbers.
///
/// The LOW end is not tested, because it cannot be reached: the stimulus is `10^|G| - 1` with
/// `|G| >= 0`, so it is never below the table's first entry of 0.
#[test]
fn mantiuk06_transducer_saturates_past_the_top_of_its_table() {
    let mut editor = fattal_image(32, |x, y| {
        if (x + y) % 2 == 0 {
            Pixel::rgba(255, 255, 255, 255)
        } else {
            Pixel::rgba(0, 0, 0, 255)
        }
    });
    let out = apply(&mut editor, mantiuk(0.1, 0.8));

    assert!(
        out[0] < 255,
        "the bright cell must come back below full: an extrapolating lookup returns exactly 255, \
         got {}",
        out[0]
    );
    assert_eq!(&out[4..7], [16, 16, 16], "the dark cell");
}

/// The same filter twice gives the same bytes.
///
/// This is a claim about ONE host, not about portability, and the two were conflated here until CI
/// separated them: `mantiuk06_contrast_zero_is_degenerate_rather_than_neutral` pinned a byte
/// sequence that this machine and the CI runner disagreed on. Same binary, same host, same input —
/// same output. That is what this asserts and all it asserts.
#[test]
fn mantiuk06_is_deterministic() {
    let mut editor = fattal_image(32, fattal_ramp);
    let first = apply(&mut editor, mantiuk(0.1, 0.8));
    let mut editor = fattal_image(32, fattal_ramp);
    let second = apply(&mut editor, mantiuk(0.1, 0.8));
    assert_eq!(first, second);
}

/// Upstream's declared ranges are enforced, saturation up to 2.
#[test]
fn mantiuk06_enforces_upstream_ranges() {
    let mut editor = fattal_image(32, fattal_ramp);
    for bad in [
        mantiuk(-0.1, 0.8),
        mantiuk(1.1, 0.8),
        mantiuk(0.1, -0.1),
        mantiuk(0.1, 2.1),
        mantiuk(f64::NAN, 0.8),
    ] {
        assert!(
            editor
                .execute(Command::ApplyFilter {
                    filter: bad.clone()
                })
                .is_err(),
            "{bad:?} is outside upstream's declared range"
        );
    }
}

/// `detail` is NOT a parameter here, because upstream never reads its own.
///
/// `property_double (detail, …)` is declared with a description and a `value_range (1.0, 99.0)`,
/// and `o->detail` appears nowhere else in `mantiuk06.c` — `process` does not pass it to `contmap`
/// and nothing else reads it. Offering it would be shipping a control that does nothing.
///
/// # What this asserts, after the first version asserted something false
///
/// The first version of this test expected a `detail` key to be REJECTED on the wire. It is not:
/// serde ignores unknown fields by default, and turning that off would change deserialisation for
/// every filter in the product to make one point about this one. So the claim is behavioural
/// instead, which is also the claim a user cares about: **supplying `detail` changes nothing**, so
/// there is no control to find. The variant simply has no such field.
#[test]
fn mantiuk06_does_not_offer_the_dead_detail_parameter() {
    let plain: Filter = serde_json::from_value(serde_json::json!({
        "kind": "mantiuk06",
        "contrast": 0.1,
        "saturation": 0.8,
    }))
    .expect("the two live parameters must deserialise");

    let with_detail: Filter = serde_json::from_value(serde_json::json!({
        "kind": "mantiuk06",
        "contrast": 0.1,
        "saturation": 0.8,
        "detail": 50.0,
    }))
    .expect("an unknown key is ignored, not an error");

    assert_eq!(
        plain, with_detail,
        "a `detail` key must make no difference: upstream declares it and never reads it"
    );

    let mut editor = fattal_image(32, fattal_ramp);
    let without = apply(&mut editor, plain);
    let mut editor = fattal_image(32, fattal_ramp);
    let with = apply(&mut editor, with_detail);
    assert_eq!(
        without, with,
        "and it must make no difference to the pixels either"
    );
}
