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
