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
