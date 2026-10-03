//! K.4, edge and stylise filters.
//!
//! Separate file for the same reason `filters_blur.rs` is: one group's tests in one place, so a
//! failure names the group it belongs to.

use redrob_core::{Command, Document, Editor, Filter, Pixel, Rect, SelectionMode};

/// Build an editor holding one layer painted from `colors`, row-major.
fn image(width: u32, height: u32, colors: &[Pixel]) -> Editor {
    let mut editor = Editor::new(Document::new(width, height).expect("document")).expect("editor");
    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let color = colors[(y as usize) * width as usize + x as usize];
            editor
                .execute(Command::SelectRectangle {
                    rect: Rect::new(x, y, 1, 1),
                    mode: SelectionMode::Replace,
                })
                .expect("select");
            editor.execute(Command::Fill { color }).expect("fill");
        }
    }
    editor.execute(Command::ClearSelection).expect("clear");
    editor
}

fn pixels(editor: &Editor) -> Vec<u8> {
    editor.document().layers()[0].pixels().to_vec()
}

/// A field with no edges detects nothing.
#[test]
fn edge_neon_on_a_flat_field_is_black() {
    let colors = vec![Pixel::rgba(120, 90, 200, 255); 81];
    let mut editor = image(9, 9, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::EdgeNeon {
                radius: 2.0,
                amount: 1.0,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    for index in 0..81 {
        for channel in 0..3 {
            assert_eq!(
                out[index * 4 + channel],
                0,
                "pixel {index} channel {channel}: a flat field has no gradient"
            );
        }
    }
}

/// It DOES respond to a straight edge — the contrast against `Antialias`, which ignores them.
///
/// The two filters are near neighbours in the menu and opposite in behaviour: antialias leaves a
/// straight boundary perfectly hard, while detecting one is this filter's entire purpose. The
/// response must also die away in the interior, so the output is an outline and not a wash.
#[test]
fn edge_neon_responds_to_a_straight_edge_and_not_the_interior() {
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
            filter: Filter::EdgeNeon {
                radius: 2.0,
                amount: 1.0,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    let at = |x: usize| i32::from(out[(10 * 21 + x) * 4]);

    assert!(
        at(10) > 20,
        "the boundary must glow, got {} -- antialias would leave this 0",
        at(10)
    );
    assert_eq!(at(0), 0, "and the far interior must stay dark");
    assert!(
        at(10) > at(5) && at(5) >= at(1),
        "the response must fall off away from the edge: {} {} {}",
        at(10),
        at(5),
        at(1)
    );
}

/// BOTH axes contribute: a horizontal edge reads the same as a vertical one.
///
/// Names the wrong behaviour exactly. A gx-only implementation — the easy mistake, and one that
/// looks completely correct on the test above — would read **0** along every horizontal edge. The
/// magnitude combines the two, so the same contrast must give the same peak whichever way it runs.
#[test]
fn edge_neon_is_symmetric_in_the_two_axes() {
    let vertical: Vec<Pixel> = (0..21 * 21)
        .map(|index| {
            let v = if index % 21 < 10 { 20u8 } else { 230 };
            Pixel::rgba(v, v, v, 255)
        })
        .collect();
    let horizontal: Vec<Pixel> = (0..21 * 21)
        .map(|index| {
            let v = if index / 21 < 10 { 20u8 } else { 230 };
            Pixel::rgba(v, v, v, 255)
        })
        .collect();

    let peak_of = |colors: &[Pixel]| {
        let mut editor = image(21, 21, colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::EdgeNeon {
                    radius: 2.0,
                    amount: 1.0,
                },
            })
            .unwrap();
        let out = pixels(&editor);
        (0..21 * 21)
            .map(|index| out[index * 4])
            .max()
            .expect("non-empty")
    };

    let v = peak_of(&vertical);
    let h = peak_of(&horizontal);
    assert!(v > 0, "the vertical edge must produce a response");
    assert_eq!(
        v, h,
        "the same contrast must read alike whichever way the edge runs; a gx-only version would \
         give 0 for the horizontal case"
    );
}

/// `amount` is a linear gain, which is the inferred part of the contract — so it is pinned.
///
/// The exact curve is NOT recoverable from source: the po file gives the parameter's name and its
/// place in the dialog and no more. Linear is the reading taken, and this test makes that visible
/// instead of leaving it buried in the implementation. Measured below the clamp so the doubling is
/// not hidden by saturation.
#[test]
fn edge_neon_amount_scales_the_response_linearly() {
    let mut colors = Vec::new();
    for _ in 0..21 {
        for x in 0..21 {
            // A gentle step, so the response stays well under 255 and can be doubled.
            let v = if x < 10 { 100u8 } else { 130 };
            colors.push(Pixel::rgba(v, v, v, 255));
        }
    }

    let peak_at = |amount: f64| {
        let mut editor = image(21, 21, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::EdgeNeon {
                    radius: 2.0,
                    amount,
                },
            })
            .unwrap();
        let out = pixels(&editor);
        (0..21 * 21)
            .map(|index| i32::from(out[index * 4]))
            .max()
            .expect("non-empty")
    };

    let single = peak_at(1.0);
    let double = peak_at(2.0);
    assert!(single > 0, "there must be a response to scale");
    assert!(
        double < 250,
        "the doubled response must stay under the clamp to be measurable, got {double}"
    );
    assert!(
        (double - single * 2).abs() <= 1,
        "doubling the amount must double the response: {single} then {double}"
    );
}

/// A zero amount detects nothing, and is accepted rather than refused.
#[test]
fn edge_neon_zero_amount_is_black_and_allowed() {
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
            filter: Filter::EdgeNeon {
                radius: 2.0,
                amount: 0.0,
            },
        })
        .expect("a zero gain is a meaningful request, not an error");
    let out = pixels(&editor);
    for index in 0..81 {
        assert_eq!(out[index * 4], 0, "a zero gain detects nothing");
    }
}

/// A larger radius detects the edge more broadly and less sharply.
///
/// The radius is the Gaussian's std-dev, so raising it spreads the derivative over more pixels: the
/// peak drops and the glow widens. Both halves asserted, since a radius that changed nothing would
/// satisfy neither.
#[test]
fn edge_neon_radius_widens_and_lowers_the_glow() {
    let mut colors = Vec::new();
    for _ in 0..31 {
        for x in 0..31 {
            let v = if x < 15 { 20u8 } else { 230 };
            colors.push(Pixel::rgba(v, v, v, 255));
        }
    }

    let profile = |radius: f64| {
        let mut editor = image(31, 31, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::EdgeNeon {
                    radius,
                    amount: 1.0,
                },
            })
            .unwrap();
        let out = pixels(&editor);
        let row: Vec<i32> = (0..31).map(|x| i32::from(out[(15 * 31 + x) * 4])).collect();
        let peak = *row.iter().max().expect("non-empty");
        let lit = row.iter().filter(|value| **value > 2).count();
        (peak, lit)
    };

    let (tight_peak, tight_width) = profile(1.0);
    let (wide_peak, wide_width) = profile(4.0);

    assert!(
        wide_peak < tight_peak,
        "a wider Gaussian must lower the peak: {wide_peak} against {tight_peak}"
    );
    assert!(
        wide_width > tight_width,
        "and spread the glow further: {wide_width} against {tight_width}"
    );
}

/// The radius is validated against the same bounds the blur filter uses, and the gain is capped.
#[test]
fn edge_neon_refuses_out_of_range_parameters() {
    let colors = vec![Pixel::rgba(100, 100, 100, 255); 25];
    for (radius, amount) in [
        (0.0, 1.0),
        (-1.0, 1.0),
        (2_000.0, 1.0),
        (2.0, -1.0),
        (2.0, 1_000.0),
        (f64::NAN, 1.0),
        (2.0, f64::INFINITY),
    ] {
        let mut editor = image(5, 5, &colors);
        assert!(
            editor
                .execute(Command::ApplyFilter {
                    filter: Filter::EdgeNeon { radius, amount },
                })
                .is_err(),
            "radius {radius} with amount {amount} must be refused"
        );
    }
}
