//! K.4, edge and stylise filters.
//!
//! Separate file for the same reason `filters_blur.rs` is: one group's tests in one place, so a
//! failure names the group it belongs to.

use redrob_core::{
    Command, Document, Editor, Filter, IllusionMode, Pixel, Rect, SelectionMode, TilingPrimitive,
};

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

/// A mosaic with default settings flattens the image into tiles, each one flat.
///
/// The defining behaviour: inside a tile every pixel reads the same, which is what "tile" means and
/// what separates this from a blur. Checked on a gradient, where any per-pixel leakage would show.
#[test]
fn mosaic_tiles_are_flat() {
    let colors: Vec<Pixel> = (0..32 * 32)
        .map(|index| {
            let v = ((index % 32) * 8) as u8;
            Pixel::rgba(v, v, v, 255)
        })
        .collect();
    let mut editor = image(32, 32, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Mosaic {
                primitive: TilingPrimitive::Squares,
                tile_size: 8,
                tile_height: 0.0,
                tile_spacing: 0.0,
                tile_neatness: 1.0,
                light_direction: 0.0,
                color_variation: 0.0,
                antialiasing: false,
                color_averaging: true,
                allow_tile_splitting: false,
                pitted_surfaces: false,
                fg_bg_lighting: false,
                foreground: Pixel::rgba(255, 255, 255, 255),
                background: Pixel::rgba(0, 0, 0, 255),
            },
        })
        .unwrap();
    let out = pixels(&editor);

    // A 32x32 image with 8px tiles must hold far fewer distinct greys than the 32 it started with.
    let mut distinct: Vec<u8> = (0..32 * 32).map(|index| out[index * 4]).collect();
    distinct.sort_unstable();
    distinct.dedup();
    assert!(
        distinct.len() <= 8,
        "tiling must collapse the gradient into few flat values, got {}",
        distinct.len()
    );
    assert!(
        distinct.len() > 1,
        "but not into a single value -- that would be an average, not a mosaic"
    );
}

/// At neatness 1.0 with square tiles the cells are the exact lattice, so tile boundaries are
/// straight and axis-aligned.
///
/// This is the test that pins the lattice. Within one tile row, every pixel of a column band must
/// belong to the same flat value — a perturbed lattice could not produce that.
#[test]
fn mosaic_squares_at_full_neatness_are_axis_aligned() {
    let colors: Vec<Pixel> = (0..32 * 32)
        .map(|index| {
            let v = ((index % 32) * 8) as u8;
            Pixel::rgba(v, v, v, 255)
        })
        .collect();
    let mut editor = image(32, 32, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Mosaic {
                primitive: TilingPrimitive::Squares,
                tile_size: 8,
                tile_height: 0.0,
                tile_spacing: 0.0,
                tile_neatness: 1.0,
                light_direction: 0.0,
                color_variation: 0.0,
                antialiasing: false,
                color_averaging: true,
                allow_tile_splitting: false,
                pitted_surfaces: false,
                fg_bg_lighting: false,
                foreground: Pixel::rgba(255, 255, 255, 255),
                background: Pixel::rgba(0, 0, 0, 255),
            },
        })
        .unwrap();
    let out = pixels(&editor);

    // Two rows inside the same band of tiles must be identical, because a square lattice's cells
    // span full rows.
    let row = |y: usize| (0..32).map(|x| out[(y * 32 + x) * 4]).collect::<Vec<u8>>();
    assert_eq!(
        row(10),
        row(11),
        "rows inside one tile band must agree when the lattice is exact"
    );
}

/// `tile_neatness` below 1.0 breaks the alignment, which is what makes the tiles *irregular*.
///
/// Names the contrast: the test above asserts two rows agree at neatness 1.0, so this asserts they
/// stop agreeing once the seeds are perturbed. A neatness that did nothing would pass the first
/// test and fail this one.
#[test]
fn mosaic_low_neatness_makes_the_tiling_irregular() {
    let colors: Vec<Pixel> = (0..32 * 32)
        .map(|index| {
            let v = ((index % 32) * 8) as u8;
            Pixel::rgba(v, v, v, 255)
        })
        .collect();

    let under = |neatness: f64| {
        let mut editor = image(32, 32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Mosaic {
                    primitive: TilingPrimitive::Squares,
                    tile_size: 8,
                    tile_height: 0.0,
                    tile_spacing: 0.0,
                    tile_neatness: neatness,
                    light_direction: 0.0,
                    color_variation: 0.0,
                    antialiasing: false,
                    color_averaging: true,
                    allow_tile_splitting: false,
                    pitted_surfaces: false,
                    fg_bg_lighting: false,
                    foreground: Pixel::rgba(255, 255, 255, 255),
                    background: Pixel::rgba(0, 0, 0, 255),
                },
            })
            .unwrap();
        pixels(&editor)
    };

    assert_ne!(
        under(1.0),
        under(0.0),
        "perturbing the seeds must change the tiling"
    );
}

/// The four primitives produce four different tilings.
///
/// Upstream offers all four, so none may coincide. The pairwise comparison is the point: an
/// implementation that ignored the primitive, or that built two of them from the same lattice,
/// would collapse some pair.
#[test]
fn mosaic_four_primitives_are_four_tilings() {
    let colors: Vec<Pixel> = (0..32 * 32)
        .map(|index| {
            let v = ((index % 32) * 8) as u8;
            Pixel::rgba(v, v / 2, 255 - v, 255)
        })
        .collect();

    let under = |primitive: TilingPrimitive| {
        let mut editor = image(32, 32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Mosaic {
                    primitive,
                    tile_size: 8,
                    tile_height: 0.0,
                    tile_spacing: 0.0,
                    tile_neatness: 1.0,
                    light_direction: 0.0,
                    color_variation: 0.0,
                    antialiasing: false,
                    color_averaging: true,
                    allow_tile_splitting: false,
                    pitted_surfaces: false,
                    fg_bg_lighting: false,
                    foreground: Pixel::rgba(255, 255, 255, 255),
                    background: Pixel::rgba(0, 0, 0, 255),
                },
            })
            .unwrap();
        pixels(&editor)
    };

    let all = [
        (TilingPrimitive::Squares, under(TilingPrimitive::Squares)),
        (TilingPrimitive::Hexagons, under(TilingPrimitive::Hexagons)),
        (
            TilingPrimitive::OctagonsAndSquares,
            under(TilingPrimitive::OctagonsAndSquares),
        ),
        (
            TilingPrimitive::Triangles,
            under(TilingPrimitive::Triangles),
        ),
    ];
    for i in 0..all.len() {
        for j in (i + 1)..all.len() {
            assert_ne!(
                all[i].1, all[j].1,
                "{:?} and {:?} must tile differently",
                all[i].0, all[j].0
            );
        }
    }
}

/// `tile_spacing` puts grout between the tiles, and the grout is dark.
///
/// The spacing is a distance from the cell boundary in pixels, and there is a floor on what can
/// have any effect: seeds sit on an integer lattice while pixel centres sit on half-integers, so no
/// pixel is ever closer than **0.5** to a boundary. A spacing below that can never ink a single
/// pixel. Found by this test failing at exactly 0.5 — which is the boundary case, not grout — so
/// it asks for 1.0 and the floor is recorded here rather than left as a surprise.
#[test]
fn mosaic_spacing_adds_dark_grout() {
    let colors = vec![Pixel::rgba(200, 200, 200, 255); 32 * 32];

    let darkest = |spacing: f64| {
        let mut editor = image(32, 32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Mosaic {
                    primitive: TilingPrimitive::Squares,
                    tile_size: 8,
                    tile_height: 0.0,
                    tile_spacing: spacing,
                    tile_neatness: 1.0,
                    light_direction: 0.0,
                    color_variation: 0.0,
                    antialiasing: false,
                    color_averaging: true,
                    allow_tile_splitting: false,
                    pitted_surfaces: false,
                    fg_bg_lighting: false,
                    foreground: Pixel::rgba(255, 255, 255, 255),
                    background: Pixel::rgba(0, 0, 0, 255),
                },
            })
            .unwrap();
        let out = pixels(&editor);
        (0..32 * 32).map(|index| out[index * 4]).min().unwrap()
    };

    assert_eq!(
        darkest(0.0),
        200,
        "with no spacing the flat field stays flat"
    );
    assert_eq!(
        darkest(0.4),
        200,
        "and below the half-pixel floor there is still nothing to ink"
    );
    assert_eq!(
        darkest(1.0),
        0,
        "with real spacing there must be grout, and grout is dark"
    );
}

/// `tile_height` shades the tiles, and `light_direction` decides which side lifts.
///
/// Two assertions, because height alone could be satisfied by any shading at all. Reversing the
/// light by 180 degrees must mirror the relief — so the pixel that was brightest becomes dimmer
/// than it was.
#[test]
fn mosaic_height_and_light_direction_produce_relief() {
    let colors = vec![Pixel::rgba(128, 128, 128, 255); 32 * 32];

    let under = |height: f64, direction: f64| {
        let mut editor = image(32, 32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Mosaic {
                    primitive: TilingPrimitive::Squares,
                    tile_size: 8,
                    tile_height: height,
                    tile_spacing: 0.0,
                    tile_neatness: 1.0,
                    light_direction: direction,
                    color_variation: 0.0,
                    antialiasing: false,
                    color_averaging: true,
                    allow_tile_splitting: false,
                    pitted_surfaces: false,
                    fg_bg_lighting: false,
                    foreground: Pixel::rgba(255, 255, 255, 255),
                    background: Pixel::rgba(0, 0, 0, 255),
                },
            })
            .unwrap();
        pixels(&editor)
    };

    let flat = under(0.0, 0.0);
    let lit = under(0.3, 0.0);
    assert_ne!(flat, lit, "a non-zero height must shade the tiles");

    let reversed = under(0.3, 180.0);
    assert_ne!(
        lit, reversed,
        "reversing the light must mirror the relief, not leave it alone"
    );
}

/// `color_variation` shifts whole tiles, never single pixels.
///
/// The distinction matters: a per-pixel jitter would be noise and would break the flatness the
/// first test asserts. So variation must change the output AND keep each tile flat.
#[test]
fn mosaic_color_variation_shifts_whole_tiles() {
    let colors = vec![Pixel::rgba(128, 128, 128, 255); 32 * 32];

    let under = |variation: f64| {
        let mut editor = image(32, 32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Mosaic {
                    primitive: TilingPrimitive::Squares,
                    tile_size: 8,
                    tile_height: 0.0,
                    tile_spacing: 0.0,
                    tile_neatness: 1.0,
                    light_direction: 0.0,
                    color_variation: variation,
                    antialiasing: false,
                    color_averaging: true,
                    allow_tile_splitting: false,
                    pitted_surfaces: false,
                    fg_bg_lighting: false,
                    foreground: Pixel::rgba(255, 255, 255, 255),
                    background: Pixel::rgba(0, 0, 0, 255),
                },
            })
            .unwrap();
        pixels(&editor)
    };

    let plain = under(0.0);
    let varied = under(0.4);
    assert_ne!(plain, varied, "variation must change the tile colours");

    // Still flat within a tile: two adjacent rows well inside one band must agree.
    let row = |data: &[u8], y: usize| (0..32).map(|x| data[(y * 32 + x) * 4]).collect::<Vec<u8>>();
    assert_eq!(
        row(&varied, 10),
        row(&varied, 11),
        "variation must shift whole tiles, so a tile stays flat"
    );
}

/// `color_averaging` off takes the seed's own pixel, which keeps detail averaging washes out.
///
/// Names the difference concretely: a single bright pixel at a tile's seed survives without
/// averaging and is diluted with it.
#[test]
fn mosaic_color_averaging_changes_which_colour_a_tile_takes() {
    let mut colors = vec![Pixel::rgba(0, 0, 0, 255); 32 * 32];
    // A lone white pixel at the very centre of the image.
    colors[16 * 32 + 16] = Pixel::rgba(255, 255, 255, 255);

    let brightest = |averaging: bool| {
        let mut editor = image(32, 32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Mosaic {
                    primitive: TilingPrimitive::Squares,
                    tile_size: 8,
                    tile_height: 0.0,
                    tile_spacing: 0.0,
                    tile_neatness: 1.0,
                    light_direction: 0.0,
                    color_variation: 0.0,
                    antialiasing: false,
                    color_averaging: averaging,
                    allow_tile_splitting: false,
                    pitted_surfaces: false,
                    fg_bg_lighting: false,
                    foreground: Pixel::rgba(255, 255, 255, 255),
                    background: Pixel::rgba(0, 0, 0, 255),
                },
            })
            .unwrap();
        let out = pixels(&editor);
        (0..32 * 32).map(|index| out[index * 4]).max().unwrap()
    };

    let averaged = brightest(true);
    let sampled = brightest(false);
    assert!(
        averaged < 20,
        "averaging must dilute one white pixel across its tile, got {averaged}"
    );
    assert!(
        sampled >= averaged,
        "sampling the seed must not lose more than averaging does: {sampled} against {averaged}"
    );
}

/// `allow_tile_splitting` makes a strong contour act as a tile boundary.
///
/// This is the flag the "Finding edges" progress phase exists to serve, so the test puts a hard
/// contour where no lattice boundary falls and asserts the flag responds to it. Without the flag
/// the tiles ignore the picture entirely.
#[test]
fn mosaic_tile_splitting_follows_an_image_contour() {
    // A hard vertical edge at x = 13, deliberately NOT on an 8px tile boundary.
    let colors: Vec<Pixel> = (0..32 * 32)
        .map(|index| {
            let v = if index % 32 < 13 { 20u8 } else { 230 };
            Pixel::rgba(v, v, v, 255)
        })
        .collect();

    let under = |splitting: bool| {
        let mut editor = image(32, 32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Mosaic {
                    primitive: TilingPrimitive::Squares,
                    tile_size: 8,
                    tile_height: 0.0,
                    tile_spacing: 0.0,
                    tile_neatness: 1.0,
                    light_direction: 0.0,
                    color_variation: 0.0,
                    antialiasing: false,
                    color_averaging: true,
                    allow_tile_splitting: splitting,
                    pitted_surfaces: false,
                    fg_bg_lighting: false,
                    foreground: Pixel::rgba(255, 255, 255, 255),
                    background: Pixel::rgba(0, 0, 0, 255),
                },
            })
            .unwrap();
        pixels(&editor)
    };

    let plain = under(false);
    let split = under(true);
    assert_ne!(
        plain, split,
        "a contour crossing a tile must change the result when splitting is allowed"
    );

    // The split must appear AT the contour, around x = 13.
    let dark_at = |data: &[u8], x: usize| data[(16 * 32 + x) * 4] == 0;
    assert!(
        (11..=14).any(|x| dark_at(&split, x)),
        "the split must land on the contour, not on a lattice line"
    );
}

/// `pitted_surfaces` breaks the shading up WITHIN a tile, unlike colour variation.
#[test]
fn mosaic_pitted_surfaces_vary_inside_a_tile() {
    let colors = vec![Pixel::rgba(128, 128, 128, 255); 32 * 32];

    let under = |pitted: bool| {
        let mut editor = image(32, 32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Mosaic {
                    primitive: TilingPrimitive::Squares,
                    tile_size: 8,
                    tile_height: 0.4,
                    tile_spacing: 0.0,
                    tile_neatness: 1.0,
                    light_direction: 45.0,
                    color_variation: 0.0,
                    antialiasing: false,
                    color_averaging: true,
                    allow_tile_splitting: false,
                    pitted_surfaces: pitted,
                    fg_bg_lighting: false,
                    foreground: Pixel::rgba(255, 255, 255, 255),
                    background: Pixel::rgba(0, 0, 0, 255),
                },
            })
            .unwrap();
        pixels(&editor)
    };

    let smooth = under(false);
    let pitted = under(true);
    assert_ne!(smooth, pitted, "pitting must change the surface");

    // Adjacent pixels inside one tile must now differ, which is exactly what colour variation
    // must NOT do.
    let a = pitted[(10 * 32 + 10) * 4];
    let b = pitted[(10 * 32 + 11) * 4];
    let c = pitted[(11 * 32 + 10) * 4];
    assert!(
        a != b || a != c,
        "pitting must break up within a tile, got {a} {b} {c}"
    );
}

/// `fg_bg_lighting` lights the relief with the given colours instead of white and black.
#[test]
fn mosaic_fg_bg_lighting_uses_the_given_colours() {
    let colors = vec![Pixel::rgba(128, 128, 128, 255); 32 * 32];

    let under = |fg_bg: bool| {
        let mut editor = image(32, 32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Mosaic {
                    primitive: TilingPrimitive::Squares,
                    tile_size: 8,
                    tile_height: 0.5,
                    tile_spacing: 0.0,
                    tile_neatness: 1.0,
                    light_direction: 0.0,
                    color_variation: 0.0,
                    antialiasing: false,
                    color_averaging: true,
                    allow_tile_splitting: false,
                    pitted_surfaces: false,
                    fg_bg_lighting: fg_bg,
                    // A strongly coloured pair, so using them is unmistakable.
                    foreground: Pixel::rgba(255, 0, 0, 255),
                    background: Pixel::rgba(0, 0, 255, 255),
                },
            })
            .unwrap();
        pixels(&editor)
    };

    let neutral = under(false);
    let coloured = under(true);
    assert_ne!(neutral, coloured, "the flag must change the lighting");

    // The lit side must pick up red and the shadowed side blue, which neutral lighting on a grey
    // field cannot produce at all.
    let has_warm = (0..32 * 32).any(|index| coloured[index * 4] > coloured[index * 4 + 2] + 10);
    let has_cool = (0..32 * 32).any(|index| coloured[index * 4 + 2] > coloured[index * 4] + 10);
    assert!(
        has_warm && has_cool,
        "the given foreground and background must both appear in the relief"
    );
}

/// `antialiasing` softens the grout edge instead of stair-stepping it.
#[test]
fn mosaic_antialiasing_softens_the_grout_edge() {
    let colors = vec![Pixel::rgba(200, 200, 200, 255); 32 * 32];

    let distinct_values = |antialiasing: bool| {
        let mut editor = image(32, 32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Mosaic {
                    primitive: TilingPrimitive::Hexagons,
                    tile_size: 8,
                    tile_height: 0.0,
                    tile_spacing: 0.3,
                    tile_neatness: 1.0,
                    light_direction: 0.0,
                    color_variation: 0.0,
                    antialiasing,
                    color_averaging: true,
                    allow_tile_splitting: false,
                    pitted_surfaces: false,
                    fg_bg_lighting: false,
                    foreground: Pixel::rgba(255, 255, 255, 255),
                    background: Pixel::rgba(0, 0, 0, 255),
                },
            })
            .unwrap();
        let out = pixels(&editor);
        let mut values: Vec<u8> = (0..32 * 32).map(|index| out[index * 4]).collect();
        values.sort_unstable();
        values.dedup();
        values.len()
    };

    let hard = distinct_values(false);
    let soft = distinct_values(true);
    assert_eq!(
        hard, 2,
        "without antialiasing the grout decision is binary: tile or grout"
    );
    assert!(
        soft > hard,
        "antialiasing must introduce partial coverage: {soft} values against {hard}"
    );
}

/// "Octagons & squares" must produce TWO cell sizes, which is what the octagon weight is for.
///
/// Took three attempts, and the first two each passed under the defect. Setting the octagon weight
/// to zero still leaves this primitive different from the other three — two interleaved square
/// lattices with equal weights give a 45-degree-rotated square lattice, which is not squares,
/// hexagons or triangles — so the pairwise-difference test cannot see the weight. The second
/// attempt measured areas but counted border-clipped cells, whose sizes vary for reasons that have
/// nothing to do with the diagram.
///
/// Excluding cells that touch the border, the two cases are unmistakable and were measured rather
/// than predicted:
///
/// - weighted:   `[60, 60, 60, 60, 196]` — four squares and one octagon, ratio **3.3**
/// - unweighted: `[120, 136, 136, 136, 136]` — congruent cells, ratio **1.13**
///
/// And `196 + 60 = 256 = 16²`, so one octagon plus one square tiles the lattice cell exactly,
/// which is the arithmetic check that the diagram is a tiling and not merely a partition.
///
/// Cell areas are read by giving every seed a different colour — `color_averaging` off makes each
/// tile take its own seed's pixel — and counting pixels per colour.
#[test]
fn mosaic_octagons_and_squares_has_two_cell_sizes() {
    let colors: Vec<Pixel> = (0..64 * 64)
        .map(|index| {
            let x = (index % 64) as u8;
            let y = (index / 64) as u8;
            Pixel::rgba(x * 4, y * 4, 128, 255)
        })
        .collect();

    let mut editor = image(64, 64, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Mosaic {
                primitive: TilingPrimitive::OctagonsAndSquares,
                tile_size: 16,
                tile_height: 0.0,
                tile_spacing: 0.0,
                tile_neatness: 1.0,
                light_direction: 0.0,
                color_variation: 0.0,
                antialiasing: false,
                color_averaging: false,
                allow_tile_splitting: false,
                pitted_surfaces: false,
                fg_bg_lighting: false,
                foreground: Pixel::rgba(255, 255, 255, 255),
                background: Pixel::rgba(0, 0, 0, 255),
            },
        })
        .unwrap();
    let out = pixels(&editor);

    let mut areas: std::collections::HashMap<(u8, u8), usize> = std::collections::HashMap::new();
    let mut touches_border: std::collections::HashSet<(u8, u8)> = std::collections::HashSet::new();
    for index in 0..64 * 64usize {
        let key = (out[index * 4], out[index * 4 + 1]);
        *areas.entry(key).or_default() += 1;
        let (x, y) = (index % 64, index / 64);
        // A cell reaching into the margin is clipped, and a clipped cell's area says nothing about
        // the diagram.
        if x < 10 || y < 10 || x >= 54 || y >= 54 {
            touches_border.insert(key);
        }
    }

    let mut sizes: Vec<usize> = areas
        .iter()
        .filter(|(key, _)| !touches_border.contains(*key))
        .map(|(_, area)| *area)
        .collect();
    sizes.sort_unstable();
    assert!(
        sizes.len() >= 3,
        "there must be several whole interior cells to compare, got {sizes:?}"
    );

    let smallest = sizes[0];
    let largest = sizes[sizes.len() - 1];
    assert!(
        largest >= smallest * 2,
        "the octagons must be clearly larger than the squares: {largest} against {smallest} \
         (ratio {:.2}); an unweighted diagram gives congruent cells at a ratio near 1.13",
        largest as f64 / smallest as f64
    );
    assert_eq!(
        largest + smallest,
        256,
        "one octagon plus one square must tile the 16x16 lattice cell exactly"
    );
}

/// Out-of-range parameters are refused.
#[test]
fn mosaic_refuses_out_of_range_parameters() {
    let colors = vec![Pixel::rgba(100, 100, 100, 255); 64];
    let base = |tile_size: u32, spacing: f64, variation: f64, height: f64| Filter::Mosaic {
        primitive: TilingPrimitive::Squares,
        tile_size,
        tile_height: height,
        tile_spacing: spacing,
        tile_neatness: 1.0,
        light_direction: 0.0,
        color_variation: variation,
        antialiasing: false,
        color_averaging: true,
        allow_tile_splitting: false,
        pitted_surfaces: false,
        fg_bg_lighting: false,
        foreground: Pixel::rgba(255, 255, 255, 255),
        background: Pixel::rgba(0, 0, 0, 255),
    };

    for filter in [
        // A one-pixel tile is not a tiling.
        base(1, 0.0, 0.0, 0.0),
        base(0, 0.0, 0.0, 0.0),
        base(10_000, 0.0, 0.0, 0.0),
        base(8, -1.0, 0.0, 0.0),
        base(8, 0.0, -1.0, 0.0),
        base(8, 0.0, 0.0, f64::NAN),
        base(8, f64::INFINITY, 0.0, 0.0),
    ] {
        let mut editor = image(8, 8, &colors);
        assert!(
            editor.execute(Command::ApplyFilter { filter }).is_err(),
            "out-of-range mosaic parameters must be refused"
        );
    }
}

/// A saved command with only the two required fields still loads.
#[test]
fn mosaic_deserialises_with_defaults() {
    let filter: Filter = serde_json::from_str(r#"{"kind":"mosaic","tile_size":8}"#)
        .expect("older saved commands must still load");
    match filter {
        Filter::Mosaic {
            primitive,
            tile_size,
            tile_neatness,
            tile_height,
            color_averaging,
            ..
        } => {
            assert_eq!(primitive, TilingPrimitive::Squares);
            assert_eq!(tile_size, 8);
            assert_eq!(
                tile_neatness, 1.0,
                "neatness must default to the exact lattice"
            );
            assert_eq!(tile_height, 0.0, "and the surface must default flat");
            assert!(!color_averaging);
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

/// A pixel at a tile's centre samples itself, so tile centres come through untouched.
///
/// The sharpest check on the geometry, and exact. At the centre the within-tile offset is zero, so
/// any error in how that offset is computed — wrong half, wrong modulus, wrong sign — moves the
/// sample off the pixel and this fails.
#[test]
fn tile_glass_leaves_tile_centres_untouched() {
    let colors: Vec<Pixel> = (0..32 * 32)
        .map(|index| {
            let x = (index % 32) as u8;
            let y = (index / 32) as u8;
            Pixel::rgba(x * 8, y * 8, 128, 255)
        })
        .collect();
    let mut editor = image(32, 32, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::TileGlass {
                tile_width: 8,
                tile_height: 8,
            },
        })
        .unwrap();
    let out = pixels(&editor);

    // Tiles start at multiples of 8; with an integer half the centre is at +4.
    for ty in 0..4usize {
        for tx in 0..4usize {
            let x = tx * 8 + 4;
            let y = ty * 8 + 4;
            let index = y * 32 + x;
            assert_eq!(
                out[index * 4],
                colors[index].r,
                "the centre of tile ({tx}, {ty}) must sample itself"
            );
            assert_eq!(out[index * 4 + 1], colors[index].g, "and in green too");
        }
    }
}

/// A uniform field is unchanged: distortion moves samples, it does not invent values.
#[test]
fn tile_glass_on_a_flat_field_changes_nothing() {
    let colors = vec![Pixel::rgba(70, 130, 180, 255); 32 * 32];
    let mut editor = image(32, 32, &colors);
    let before = pixels(&editor);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::TileGlass {
                tile_width: 8,
                tile_height: 5,
            },
        })
        .unwrap();
    assert_eq!(
        pixels(&editor),
        before,
        "resampling a flat field must return it exactly"
    );
}

/// Each tile draws from TWICE its own span, so a gradient reads at double slope inside a tile.
///
/// This is the measurement that pins the refraction, and it is exact. The input rises 4 per pixel
/// across x; adding the within-tile offset doubles the sampled step, so inside a tile the output
/// must rise 8 per pixel. A filter that merely shifted each tile would keep the slope at 4, and one
/// that subtracted the offset would flatten it to 0.
#[test]
fn tile_glass_doubles_the_gradient_inside_a_tile() {
    let colors: Vec<Pixel> = (0..64 * 8)
        .map(|index| {
            let v = ((index % 64) * 4) as u8;
            Pixel::rgba(v, v, v, 255)
        })
        .collect();
    let mut editor = image(64, 8, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::TileGlass {
                tile_width: 16,
                tile_height: 8,
            },
        })
        .unwrap();
    let out = pixels(&editor);

    // Inside the second tile (x 16..32), away from its edges so no clamping interferes.
    let at = |x: usize| i32::from(out[(4 * 64 + x) * 4]);
    for x in 20..28usize {
        assert_eq!(
            at(x + 1) - at(x),
            8,
            "inside a tile the gradient must read at double slope at x={x}; a plain shift would \
             give 4 and subtracting the offset would give 0"
        );
    }
}

/// Width and height act INDEPENDENTLY, which is why upstream offers two parameters and not one.
///
/// Varying ONE axis at a time, which my first version of this test did not do. It compared
/// `(16, 4)` against `(4, 16)` — but an implementation using `tile_width` for both axes turns
/// those into 16×16 and 4×4, which still differ, so the injected defect passed. Swapping both
/// numbers cannot isolate either one.
///
/// Holding width fixed and changing only height is what names the mistake: under the shared-axis
/// reading both calls use width for everything, so the two results are byte-identical. The second
/// half checks the mirror case, because a filter could plausibly honour one axis and ignore the
/// other.
#[test]
fn tile_glass_width_and_height_are_independent() {
    let colors: Vec<Pixel> = (0..32 * 32)
        .map(|index| {
            let x = (index % 32) as u8;
            let y = (index / 32) as u8;
            // Different structure along each axis, so neither can be a no-op.
            Pixel::rgba(x * 8, y * 4, 60, 255)
        })
        .collect();

    let under = |w: u32, h: u32| {
        let mut editor = image(32, 32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::TileGlass {
                    tile_width: w,
                    tile_height: h,
                },
            })
            .unwrap();
        pixels(&editor)
    };

    assert_ne!(
        under(8, 4),
        under(8, 16),
        "height alone must change the result; a filter using width for both axes would make \
         these two byte-identical"
    );
    assert_ne!(
        under(4, 8),
        under(16, 8),
        "and width alone must change it too"
    );
}

/// Tile seams are real discontinuities — that is what makes it read as glass, not as a blur.
///
/// Across a tile boundary the sample jumps by a whole tile's worth, so a smooth input comes out
/// with a step at every seam. A blur or a shift would leave the gradient continuous.
#[test]
fn tile_glass_puts_a_discontinuity_at_every_seam() {
    let colors: Vec<Pixel> = (0..64 * 8)
        .map(|index| {
            let v = ((index % 64) * 4) as u8;
            Pixel::rgba(v, v, v, 255)
        })
        .collect();
    let mut editor = image(64, 8, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::TileGlass {
                tile_width: 16,
                tile_height: 8,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    let at = |x: usize| i32::from(out[(4 * 64 + x) * 4]);

    // At x = 32 a new tile begins: the step there must be larger in magnitude than the in-tile
    // step of 8, and opposite in sign, because the sample snaps back.
    let seam_step = at(32) - at(31);
    assert!(
        seam_step < 0,
        "the sample must snap back at a seam, got a step of {seam_step}"
    );
    assert!(
        seam_step.abs() > 8,
        "and the seam step must exceed the in-tile step of 8, got {}",
        seam_step.abs()
    );
}

/// A one-pixel tile is the identity: the offset is zero everywhere.
///
/// Accepted rather than refused, because it is a meaningful degenerate request and upstream's
/// parameter is a tile extent with no stated floor above 1.
#[test]
fn tile_glass_one_pixel_tiles_are_the_identity() {
    let colors: Vec<Pixel> = (0..16 * 16)
        .map(|index| Pixel::rgba((index * 7) as u8, (index * 3) as u8, 90, 255))
        .collect();
    let mut editor = image(16, 16, &colors);
    let before = pixels(&editor);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::TileGlass {
                tile_width: 1,
                tile_height: 1,
            },
        })
        .expect("a one-pixel tile is degenerate but meaningful");
    assert_eq!(
        pixels(&editor),
        before,
        "with no room to bend, the view is straight through"
    );
}

/// Zero and oversized extents are refused, on each axis separately.
#[test]
fn tile_glass_refuses_out_of_range_extents() {
    let colors = vec![Pixel::rgba(100, 100, 100, 255); 64];
    for (w, h) in [(0u32, 8u32), (8, 0), (0, 0), (5_000, 8), (8, 5_000)] {
        let mut editor = image(8, 8, &colors);
        assert!(
            editor
                .execute(Command::ApplyFilter {
                    filter: Filter::TileGlass {
                        tile_width: w,
                        tile_height: h,
                    },
                })
                .is_err(),
            "extents {w}x{h} must be refused"
        );
    }
}
