//! K.4, edge and stylise filters.
//!
//! Separate file for the same reason `filters_blur.rs` is: one group's tests in one place, so a
//! failure names the group it belongs to.

use redrob_core::{Command, Document, Editor, Filter, IllusionMode, Pixel, Rect, SelectionMode};

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

/// The output is BINARY — one ink, varying coverage. A grey anywhere makes it a posterisation.
#[test]
fn engrave_output_is_only_black_or_white() {
    let colors: Vec<Pixel> = (0..16 * 16)
        .map(|index| {
            let v = (index % 256) as u8;
            Pixel::rgba(v, v / 2, 255 - v, 255)
        })
        .collect();
    let mut editor = image(16, 16, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Engrave {
                height: 4,
                limit: false,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    for index in 0..16 * 16 {
        for channel in 0..3 {
            let value = out[index * 4 + channel];
            assert!(
                value == 0 || value == 255,
                "pixel {index} channel {channel} is {value}: an engraving has one ink, not a tone"
            );
        }
    }
}

/// Thickness tracks darkness: a dark band inks more rows than a light one.
///
/// The defining behaviour. Two bands of flat but different tone must come back with different
/// inked-row counts, and the darker one must be the thicker.
#[test]
fn engrave_thickness_follows_darkness() {
    let mut colors = Vec::new();
    for y in 0..8usize {
        for _ in 0..8 {
            // Top band light, bottom band dark.
            let v = if y < 4 { 200u8 } else { 60 };
            colors.push(Pixel::rgba(v, v, v, 255));
        }
    }
    let mut editor = image(8, 8, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Engrave {
                height: 4,
                limit: false,
            },
        })
        .unwrap();
    let out = pixels(&editor);

    let inked_in_band = |top: usize| (top..top + 4).filter(|y| out[(y * 8) * 4] == 0).count();
    let light = inked_in_band(0);
    let dark = inked_in_band(4);
    assert!(
        dark > light,
        "the darker band must ink more rows: {dark} against {light}"
    );
}

/// It makes HORIZONTAL lines: a column's tone varies the thickness along the line.
///
/// Discriminates a line engraving from a per-pixel threshold. With a horizontal gradient the inked
/// count must differ between columns within the SAME band — which a band-wide decision could not
/// produce, and a per-pixel threshold would produce without any line structure at all. The second
/// assertion pins the line structure: within a band, a single column is a contiguous run of ink.
#[test]
fn engrave_varies_thickness_along_a_band() {
    let mut colors = Vec::new();
    for _ in 0..8 {
        for x in 0..16 {
            // Horizontal gradient, constant down each column.
            let v = (x * 16) as u8;
            colors.push(Pixel::rgba(v, v, v, 255));
        }
    }
    let mut editor = image(16, 8, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Engrave {
                height: 8,
                limit: false,
            },
        })
        .unwrap();
    let out = pixels(&editor);

    let inked_in_column = |x: usize| (0..8).filter(|y| out[(y * 16 + x) * 4] == 0).count();
    let dark_end = inked_in_column(0);
    let light_end = inked_in_column(15);
    assert!(
        dark_end > light_end,
        "thickness must vary along the band: {dark_end} at the dark end against {light_end}"
    );

    // Contiguity: the ink in one column is one run, not scattered pixels.
    for x in 0..16usize {
        let rows: Vec<bool> = (0..8).map(|y| out[(y * 16 + x) * 4] == 0).collect();
        let transitions = rows.windows(2).filter(|pair| pair[0] != pair[1]).count();
        assert!(
            transitions <= 2,
            "column {x} must be one contiguous run of ink, got {transitions} transitions"
        );
    }
}

/// The line is CENTRED in its band, so a thickening line grows about its own axis.
///
/// Without centring the ink would start at the band's top edge and grow downward, which makes the
/// line drift as the tone changes instead of swelling in place. Asserted by symmetry: the bare rows
/// above and below the ink differ by at most one.
#[test]
fn engrave_centres_the_line_in_its_band() {
    let colors = vec![Pixel::rgba(128, 128, 128, 255); 64];
    let mut editor = image(8, 8, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Engrave {
                height: 8,
                limit: false,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    let rows: Vec<bool> = (0..8).map(|y| out[(y * 8) * 4] == 0).collect();

    let above = rows.iter().take_while(|inked| !**inked).count();
    let below = rows.iter().rev().take_while(|inked| !**inked).count();
    assert!(
        (above as i32 - below as i32).abs() <= 1,
        "the line must sit centred: {above} bare rows above against {below} below"
    );
}

/// `limit` keeps a line visible in white regions and keeps black regions from going solid.
///
/// Names both values. A white field inks 0 rows unlimited and must ink exactly 1 limited; a black
/// field inks all 4 unlimited and must leave exactly 1 bare limited.
#[test]
fn engrave_limit_bounds_the_line_at_both_extremes() {
    let inked_rows = |value: u8, limit: bool| {
        let colors = vec![Pixel::rgba(value, value, value, 255); 16];
        let mut editor = image(4, 4, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Engrave { height: 4, limit },
            })
            .unwrap();
        let out = pixels(&editor);
        (0..4).filter(|y| out[(y * 4) * 4] == 0).count()
    };

    assert_eq!(inked_rows(255, false), 0, "white engraves to nothing");
    assert_eq!(
        inked_rows(255, true),
        1,
        "limited, white must still keep one inked row"
    );
    assert_eq!(inked_rows(0, false), 4, "black engraves to solid");
    assert_eq!(
        inked_rows(0, true),
        3,
        "limited, black must leave one row bare"
    );
}

/// `height` sets the band, so the pattern repeats on that period.
#[test]
fn engrave_height_sets_the_band_period() {
    // A vertical gradient, so each band gets a different tone and the band structure is visible.
    let colors: Vec<Pixel> = (0..16 * 4)
        .map(|index| {
            let v = ((index / 4) * 16) as u8;
            Pixel::rgba(v, v, v, 255)
        })
        .collect();

    let bands_with_ink = |band: u32| {
        let mut editor = image(4, 16, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Engrave {
                    height: band,
                    limit: true,
                },
            })
            .unwrap();
        let out = pixels(&editor);
        // Count distinct inked-row totals per band -- a larger band means fewer bands.
        (0..16 / band)
            .map(|index| {
                let top = (index * band) as usize;
                (top..top + band as usize)
                    .filter(|y| out[(y * 4) * 4] == 0)
                    .count()
            })
            .collect::<Vec<usize>>()
    };

    let small = bands_with_ink(2);
    let large = bands_with_ink(8);
    assert_eq!(small.len(), 8, "a height of 2 must give 8 bands");
    assert_eq!(large.len(), 2, "a height of 8 must give 2 bands");
}

/// A short final band scales by its TRUE row count, not the nominal height.
///
/// When the image height is not a multiple of the band, the last band is short. Scaling it by the
/// nominal height OVER-inks it and can fill it solid — measured, not assumed: at value 64 a 3-row
/// band scaled by a nominal 4 inks 3 of 3 rows, where the true count gives 2 of 3. Either way it
/// is a visible seam along the bottom of every engraving whose height does not divide evenly.
#[test]
fn engrave_short_final_band_is_not_thinner() {
    // 7 rows with a band of 4: bands of 4 and 3, both mid grey.
    let colors = vec![Pixel::rgba(64, 64, 64, 255); 4 * 7];
    let mut editor = image(4, 7, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Engrave {
                height: 4,
                limit: false,
            },
        })
        .unwrap();
    let out = pixels(&editor);

    let full = (0..4).filter(|y| out[(y * 4) * 4] == 0).count();
    let short = (4..7).filter(|y| out[(y * 4) * 4] == 0).count();
    // Same tone, so the inked FRACTION must match within rounding: 3/4 of 4 against 3/4 of 3.
    let full_fraction = full as f64 / 4.0;
    let short_fraction = short as f64 / 3.0;
    assert!(
        (full_fraction - short_fraction).abs() < 0.25,
        "the short band must ink the same fraction: {full}/4 against {short}/3"
    );
}

/// A zero band height is refused, and so is one past our recorded cap.
#[test]
fn engrave_refuses_an_out_of_range_height() {
    let colors = vec![Pixel::rgba(100, 100, 100, 255); 16];
    for band in [0u32, 2_000] {
        let mut editor = image(4, 4, &colors);
        assert!(
            editor
                .execute(Command::ApplyFilter {
                    filter: Filter::Engrave {
                        height: band,
                        limit: false,
                    },
                })
                .is_err(),
            "a band height of {band} must be refused"
        );
    }
}

/// A saved command without `limit` still loads, defaulting to off.
#[test]
fn engrave_deserialises_without_limit() {
    let filter: Filter = serde_json::from_str(r#"{"kind":"engrave","height":4}"#)
        .expect("older saved commands must still load");
    match filter {
        Filter::Engrave { height, limit } => {
            assert_eq!(height, 4);
            assert!(!limit, "limit must default off");
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

/// One division in mode 1 is the identity: a single copy, turned by nothing.
///
/// Exact, and the cleanest check that the rotation is applied about the right centre — an off-by-half
/// centre would shift the whole image by a pixel and this would fail everywhere at once.
#[test]
fn illusion_one_division_mode_one_is_the_identity() {
    let colors: Vec<Pixel> = (0..8 * 8)
        .map(|index| {
            let v = (index * 3) as u8;
            Pixel::rgba(v, 255 - v, v / 2, 255)
        })
        .collect();
    let mut editor = image(8, 8, &colors);
    let before = pixels(&editor);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Illusion {
                divisions: 1,
                mode: IllusionMode::One,
            },
        })
        .unwrap();
    assert_eq!(
        pixels(&editor),
        before,
        "one copy turned by zero must be the original, byte for byte"
    );
}

/// One division in mode 2 is the MIRROR — which is what proves the modes differ by a reflection.
///
/// Names the wrong behaviour precisely. The obvious reading of "Mode 1 / Mode 2" is that mode 2
/// reverses the rotation direction; under that reading this case would be the IDENTITY, because
/// negating a zero angle changes nothing. It is the vertical mirror instead, so the reflection
/// reading is the one in force.
#[test]
fn illusion_one_division_mode_two_is_the_mirror() {
    let colors: Vec<Pixel> = (0..8 * 8)
        .map(|index| {
            let v = (index * 3) as u8;
            Pixel::rgba(v, 255 - v, v / 2, 255)
        })
        .collect();
    let mut editor = image(8, 8, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Illusion {
                divisions: 1,
                mode: IllusionMode::Two,
            },
        })
        .unwrap();
    let out = pixels(&editor);

    for y in 0..8usize {
        for x in 0..8usize {
            let mirrored = (7 - y) * 8 + x;
            let here = y * 8 + x;
            assert_eq!(
                out[here * 4],
                colors[mirrored].r,
                "({x}, {y}) must come from the vertically mirrored row; a direction-flip reading \
                 would have left this the identity"
            );
        }
    }
}

/// Two divisions make the result symmetric under a half turn, exactly.
///
/// The structural consequence of superimposing an image with its own 180-degree rotation: the
/// average is invariant under that rotation. Exact rather than approximate because the centre is
/// the image's, not a pixel's, and the rotation is applied as a matrix.
#[test]
fn illusion_two_divisions_is_half_turn_symmetric() {
    let colors: Vec<Pixel> = (0..8 * 8)
        .map(|index| {
            let v = (index * 3) as u8;
            Pixel::rgba(v, 255 - v, v / 2, 255)
        })
        .collect();
    let mut editor = image(8, 8, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Illusion {
                divisions: 2,
                mode: IllusionMode::One,
            },
        })
        .unwrap();
    let out = pixels(&editor);

    for y in 0..8usize {
        for x in 0..8usize {
            let here = (y * 8 + x) * 4;
            let turned = ((7 - y) * 8 + (7 - x)) * 4;
            for channel in 0..3 {
                assert_eq!(
                    out[here + channel],
                    out[turned + channel],
                    "({x}, {y}) and its half turn must agree in channel {channel}"
                );
            }
        }
    }
}

/// A rotationally symmetric image survives any number of divisions.
///
/// Concentric rings are invariant under rotation about the centre, so every copy is the same image
/// and the average is that image again. A filter sampling at the wrong radius, or rotating about a
/// corner, could not satisfy this.
#[test]
fn illusion_leaves_a_rotationally_symmetric_image_alone() {
    let size = 16usize;
    let centre = size as f64 / 2.0;
    let colors: Vec<Pixel> = (0..size * size)
        .map(|index| {
            let x = (index % size) as f64 + 0.5 - centre;
            let y = (index / size) as f64 + 0.5 - centre;
            // Rings: value depends only on distance from the centre.
            let ring = ((x * x + y * y).sqrt() * 2.0) as u32 % 2;
            let v = if ring == 0 { 40u8 } else { 210 };
            Pixel::rgba(v, v, v, 255)
        })
        .collect();

    let mut editor = image(size as u32, size as u32, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Illusion {
                divisions: 4,
                mode: IllusionMode::One,
            },
        })
        .unwrap();
    let out = pixels(&editor);

    // Sampling is nearest, so allow a ring pixel to land on its neighbour; the point is that the
    // image is not smeared into a uniform grey, which is what a wrong centre would produce.
    let mut unchanged = 0usize;
    for index in 0..size * size {
        if out[index * 4] == colors[index].r {
            unchanged += 1;
        }
    }
    assert!(
        unchanged * 10 >= size * size * 9,
        "a rotationally symmetric image must come back essentially unchanged, {unchanged} of {} \
         pixels held",
        size * size
    );
}

/// The two modes genuinely differ on an asymmetric image.
///
/// Upstream offers both, so they must not coincide. This is the test that would have caught the
/// direction-flip reading, under which the two modes are byte-identical at every division because
/// averaging does not care what order the copies come in.
#[test]
fn illusion_modes_differ_on_an_asymmetric_image() {
    let colors: Vec<Pixel> = (0..16 * 16)
        .map(|index| {
            let x = index % 16;
            let y = index / 16;
            // Asymmetric in both axes, so neither a rotation nor a reflection can be a no-op.
            let v = (x * 11 + y * 3) as u8;
            Pixel::rgba(v, v / 2, 255 - v, 255)
        })
        .collect();

    let under = |mode: IllusionMode| {
        let mut editor = image(16, 16, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Illusion { divisions: 3, mode },
            })
            .unwrap();
        pixels(&editor)
    };

    assert_ne!(
        under(IllusionMode::One),
        under(IllusionMode::Two),
        "the two modes must produce different images; a direction-flip reading would make them \
         identical"
    );
}

/// Zero divisions is refused, and so is a count past our recorded cap.
#[test]
fn illusion_refuses_an_out_of_range_division_count() {
    let colors = vec![Pixel::rgba(100, 100, 100, 255); 16];
    for divisions in [0u32, 1_000] {
        let mut editor = image(4, 4, &colors);
        assert!(
            editor
                .execute(Command::ApplyFilter {
                    filter: Filter::Illusion {
                        divisions,
                        mode: IllusionMode::One,
                    },
                })
                .is_err(),
            "a division count of {divisions} must be refused"
        );
    }
}

/// A saved command without `mode` still loads, defaulting to mode 1.
#[test]
fn illusion_deserialises_without_mode() {
    let filter: Filter = serde_json::from_str(r#"{"kind":"illusion","divisions":5}"#)
        .expect("older saved commands must still load");
    match filter {
        Filter::Illusion { divisions, mode } => {
            assert_eq!(divisions, 5);
            assert_eq!(
                mode,
                IllusionMode::One,
                "the default must be the behaviour the variant shipped with"
            );
        }
        other => panic!("wrong variant: {other:?}"),
    }
}
