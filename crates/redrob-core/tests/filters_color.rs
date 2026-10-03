// SPDX-License-Identifier: GPL-3.0-or-later

//! K.2. Colour operations.

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

/// Counts how many times the output's direction of travel reverses.
///
/// Deliberately NOT a strict `w[1] > w[0] && w[2] < w[1]` turning-point test. My first version was,
/// and it reported ZERO turns on a correct filter: a sine is flat near its extremes, so over a
/// dense 256-sample ramp several consecutive inputs round to the same byte and the strict
/// inequalities never fire. A five-sample probe had no plateaus and passed, which is why the
/// failure looked like a broken filter rather than a broken test.
///
/// Skipping the zero differences and counting sign changes among the rest is immune to a plateau,
/// and still counts exactly two reversals per cycle.
fn direction_reversals(values: &[i32]) -> usize {
    let deltas: Vec<i32> = values
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|d| *d != 0)
        .collect();
    deltas
        .windows(2)
        .filter(|w| (w[0] > 0) != (w[1] > 0))
        .count()
}

/// Frequency means what upstream's blurb says: cycles covering the full value range.
///
/// This is the load-bearing test of the whole filter, because GEGL is not vendored and the blurb
/// IS the specification. "Number of cycles covering full value range" means that at frequency 1
/// the output completes exactly one sine cycle while the input sweeps 0..255 — so it must rise to
/// a single maximum near a quarter of the way, fall to a single minimum near three quarters, and
/// return. A wrong constant in the argument (pi instead of tau, say) changes that count, which is
/// exactly what this counts.
#[test]
fn alien_map_frequency_counts_cycles_over_the_full_range() {
    let ramp: Vec<Pixel> = (0..=255u16)
        .map(|v| Pixel::rgba(v as u8, v as u8, v as u8, 255))
        .collect();
    let mut editor = row(&ramp);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::AlienMap {
                model: redrob_core::AlienMapModel::Rgb,
                cpn1_frequency: 1.0,
                cpn1_phase: 0.0,
                cpn1_enabled: true,
                cpn2_frequency: 1.0,
                cpn2_phase: 0.0,
                cpn2_enabled: false,
                cpn3_frequency: 1.0,
                cpn3_phase: 0.0,
                cpn3_enabled: false,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    let reds: Vec<i32> = (0..256).map(|i| i32::from(out[i * 4])).collect();

    // One full cycle reverses direction exactly twice: up-to-down, then down-to-up.
    let turns = direction_reversals(&reds);
    assert_eq!(
        turns, 2,
        "frequency 1 must be exactly one cycle over the range, got {turns} reversals"
    );

    // And the peak and trough sit where one cycle puts them.
    let peak = reds.iter().enumerate().max_by_key(|(_, v)| **v).unwrap().0;
    let trough = reds.iter().enumerate().min_by_key(|(_, v)| **v).unwrap().0;
    assert!(
        (peak as i32 - 64).abs() <= 4,
        "the maximum belongs near a quarter of the range, got {peak}"
    );
    assert!(
        (trough as i32 - 191).abs() <= 4,
        "the minimum belongs near three quarters, got {trough}"
    );

    // Channels left disabled must be untouched, which is what makes the toggle meaningful.
    for i in 0..256 {
        assert_eq!(
            out[i * 4 + 1],
            i as u8,
            "green was disabled and must pass through"
        );
    }
}

/// Doubling the frequency doubles the cycle count.
///
/// Guards the unit rather than one magic output: a formula that merely scaled the input would pass
/// the single-frequency test and fail this one.
#[test]
fn alien_map_frequency_two_gives_two_cycles() {
    let ramp: Vec<Pixel> = (0..=255u16)
        .map(|v| Pixel::rgba(v as u8, v as u8, v as u8, 255))
        .collect();
    let mut editor = row(&ramp);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::AlienMap {
                model: redrob_core::AlienMapModel::Rgb,
                cpn1_frequency: 2.0,
                cpn1_phase: 0.0,
                cpn1_enabled: true,
                cpn2_frequency: 0.0,
                cpn2_phase: 0.0,
                cpn2_enabled: false,
                cpn3_frequency: 0.0,
                cpn3_phase: 0.0,
                cpn3_enabled: false,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    let reds: Vec<i32> = (0..256).map(|i| i32::from(out[i * 4])).collect();
    let turns = direction_reversals(&reds);
    assert_eq!(
        turns, 4,
        "two cycles reverse direction four times, got {turns}"
    );
}

/// Phase is in DEGREES, as "Phase angle, range 0-360" states.
///
/// A phase of 90 degrees must advance the wave by a quarter cycle. If the field were taken as
/// radians, 90 would be more than fourteen full turns and the output would be unrelated.
#[test]
fn alien_map_phase_is_degrees_not_radians() {
    let black = [Pixel::rgba(0, 0, 0, 255)];

    let sample = |phase: f32| -> u8 {
        let mut editor = row(&black);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::AlienMap {
                    model: redrob_core::AlienMapModel::Rgb,
                    cpn1_frequency: 1.0,
                    cpn1_phase: phase,
                    cpn1_enabled: true,
                    cpn2_frequency: 1.0,
                    cpn2_phase: 0.0,
                    cpn2_enabled: false,
                    cpn3_frequency: 1.0,
                    cpn3_phase: 0.0,
                    cpn3_enabled: false,
                },
            })
            .unwrap();
        pixels(&editor)[0]
    };

    // At input 0 the argument is the phase alone, so the output reads sin(phase) directly.
    assert_eq!(sample(0.0), 128, "sin(0) = 0 maps to the midpoint");
    assert_eq!(sample(90.0), 255, "sin(90 degrees) = 1 maps to the top");
    assert_eq!(sample(270.0), 0, "sin(270 degrees) = -1 maps to the bottom");
    // 360 degrees is a full turn and must agree with 0 -- within one step, because the value
    // there is exactly 127.5 and the sine of a full turn is a hair below zero rather than zero, so
    // the two land on opposite sides of the rounding boundary. Demanding exact equality was asking
    // f64 for a precision it does not have at the wrap point.
    assert!(
        sample(360.0).abs_diff(sample(0.0)) <= 1,
        "a full turn must wrap: {} vs {}",
        sample(360.0),
        sample(0.0)
    );
}

/// The HSL model remaps lightness, not HSV's value — they are different numbers.
///
/// Upstream labels the third slider "Luminosity" and offers "HSL color model". HSL lightness is
/// (max+min)/2 where HSV value is max, so for any colour that is not a pure tint the two disagree.
/// Picking the wrong one would remap a different channel than the one upstream names, and nothing
/// about the output would look obviously wrong — which is why this is pinned.
#[test]
fn alien_map_hsl_model_uses_lightness_not_value() {
    // max 200, min 40: HSV value is 0.784 while HSL lightness is 0.471. Far enough apart that the
    // result says which one was fed to the remap.
    let source = [Pixel::rgba(200, 40, 40, 255)];
    let mut editor = row(&source);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::AlienMap {
                model: redrob_core::AlienMapModel::Hsl,
                cpn1_frequency: 1.0,
                cpn1_phase: 0.0,
                cpn1_enabled: false,
                cpn2_frequency: 1.0,
                cpn2_phase: 0.0,
                cpn2_enabled: false,
                // Only the third channel is remapped, isolating which channel it is.
                cpn3_frequency: 1.0,
                cpn3_phase: 90.0,
                cpn3_enabled: true,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    let (r, g, b) = (
        f32::from(out[0]) / 255.0,
        f32::from(out[1]) / 255.0,
        f32::from(out[2]) / 255.0,
    );
    let lightness = (r.max(g).max(b) + r.min(g).min(b)) / 2.0;
    // Both expectations are COMPUTED, not estimated by hand. Lightness 120/255 = 0.4706 through
    // 0.5*(1+sin(tau*L + pi/2)) gives 0.0085; HSV value 200/255 = 0.7843 would give 0.607. My
    // first draft wrote 0.403 and 0.615 from mental arithmetic and both were wrong -- the same
    // mistake as cycle 28, so the numbers here come from evaluating the expression.
    assert!(
        (lightness - 0.0085).abs() < 0.02,
        "the HSL model must remap lightness to about 0.0085, got {lightness}"
    );
    assert!(
        (lightness - 0.607).abs() > 0.1,
        "and must NOT have remapped HSV value, which would give about 0.607"
    );
}

/// All three channels disabled leaves the image untouched.
///
/// The toggles are a real part of the contract — upstream ships six "Modify … channel" strings —
/// so the fully-off case must be a genuine no-op rather than a flat grey.
#[test]
fn alien_map_with_every_channel_disabled_changes_nothing() {
    let source = [
        Pixel::rgba(200, 40, 90, 255),
        Pixel::rgba(10, 230, 120, 128),
    ];
    let mut editor = row(&source);
    let before = pixels(&editor);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::AlienMap {
                model: redrob_core::AlienMapModel::Rgb,
                cpn1_frequency: 3.0,
                cpn1_phase: 45.0,
                cpn1_enabled: false,
                cpn2_frequency: 3.0,
                cpn2_phase: 45.0,
                cpn2_enabled: false,
                cpn3_frequency: 3.0,
                cpn3_phase: 45.0,
                cpn3_enabled: false,
            },
        })
        .unwrap();
    assert_eq!(pixels(&editor), before, "every channel off must be a no-op");
}

/// The tolerance region is an axis-aligned BOX, not a sphere.
///
/// This is the load-bearing property, because it is what three independent per-channel thresholds
/// mean, and it is where a plausible-looking Euclidean implementation diverges. With every
/// threshold at 10, a colour offset by (10, 10, 10) is INSIDE the box but 17.3 away by distance —
/// so the two readings disagree on exactly this pixel, and nothing about the output would look
/// wrong if the distance reading had been used.
#[test]
fn color_exchange_tolerance_is_a_box_not_a_sphere() {
    let target = Pixel::rgba(100, 100, 100, 255);
    let corner = Pixel::rgba(110, 110, 110, 255); // inside the box, outside a radius-10 sphere
    let outside = Pixel::rgba(111, 100, 100, 255); // one step past the box on red alone
    let mut editor = row(&[target, corner, outside]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ColorExchange {
                from: target,
                to: Pixel::rgba(0, 0, 255, 255),
                red_threshold: 10,
                green_threshold: 10,
                blue_threshold: 10,
            },
        })
        .unwrap();
    let out = pixels(&editor);

    assert_eq!(
        (out[0], out[1], out[2]),
        (0, 0, 255),
        "the exact colour must be exchanged"
    );
    assert_eq!(
        (out[4], out[5], out[6]),
        (0, 0, 255),
        "a corner of the box is inside the tolerance, even though it is 17.3 away by distance"
    );
    assert_eq!(
        (out[8], out[9], out[10]),
        (111, 100, 100),
        "one step past the box on a single channel must NOT be exchanged"
    );
}

/// Each threshold applies to its own channel only.
///
/// Three separate sliders are pointless if they act together, and the dialog's "Lock thresholds"
/// checkbox only makes sense because they are separate. A pixel inside tolerance on red and green
/// but outside on blue must survive.
#[test]
fn color_exchange_thresholds_are_per_channel() {
    let target = Pixel::rgba(100, 100, 100, 255);
    let loose_red = Pixel::rgba(140, 100, 100, 255);
    let tight_blue = Pixel::rgba(100, 100, 140, 255);
    let mut editor = row(&[loose_red, tight_blue]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ColorExchange {
                from: target,
                to: Pixel::rgba(255, 0, 0, 255),
                red_threshold: 50,
                green_threshold: 0,
                blue_threshold: 0,
            },
        })
        .unwrap();
    let out = pixels(&editor);

    assert_eq!(
        (out[0], out[1], out[2]),
        (255, 0, 0),
        "red is within its own generous threshold, so this matches"
    );
    assert_eq!(
        (out[4], out[5], out[6]),
        (100, 100, 140),
        "blue is outside its zero threshold, so the generous red one must not rescue it"
    );
}

/// A zero threshold matches the exact colour and nothing else.
#[test]
fn color_exchange_zero_threshold_is_exact() {
    let target = Pixel::rgba(60, 120, 180, 255);
    let one_off = Pixel::rgba(61, 120, 180, 255);
    let mut editor = row(&[target, one_off]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ColorExchange {
                from: target,
                to: Pixel::rgba(0, 0, 0, 255),
                red_threshold: 0,
                green_threshold: 0,
                blue_threshold: 0,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    assert_eq!((out[0], out[1], out[2]), (0, 0, 0));
    assert_eq!(
        (out[4], out[5], out[6]),
        (61, 120, 180),
        "one step away must survive a zero threshold"
    );
}

/// A target at the ends of the range does not wrap.
///
/// The comparison is `abs_diff` on `u8` rather than a signed subtraction, because near 0 or 255 a
/// subtraction wraps and the match region silently jumps to the other end of the scale. This is
/// the class of defect that works perfectly on mid-tones and only fails at the extremes.
#[test]
fn color_exchange_does_not_wrap_at_the_ends_of_the_range() {
    // Target black with a tolerance of 5: the bright pixel is as far away as possible and must not
    // be caught by a wrapped comparison.
    let mut editor = row(&[
        Pixel::rgba(0, 0, 0, 255),
        Pixel::rgba(3, 3, 3, 255),
        Pixel::rgba(252, 252, 252, 255),
        Pixel::rgba(255, 255, 255, 255),
    ]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ColorExchange {
                from: Pixel::rgba(0, 0, 0, 255),
                to: Pixel::rgba(0, 255, 0, 255),
                red_threshold: 5,
                green_threshold: 5,
                blue_threshold: 5,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    assert_eq!((out[0], out[1], out[2]), (0, 255, 0), "black matches");
    assert_eq!((out[4], out[5], out[6]), (0, 255, 0), "and so does 3");
    assert_eq!(
        (out[8], out[9], out[10]),
        (252, 252, 252),
        "252 is 252 away from black, not 3 — a wrapped comparison would have matched it"
    );
    assert_eq!(
        (out[12], out[13], out[14]),
        (255, 255, 255),
        "and white is 255 away, not 0"
    );

    // THE OTHER END, and it is the one that matters. Reverse-verification exposed that the
    // black-target case above cannot detect a wrapping comparison: with `from` at 0,
    // `wrapping_sub(0)` and `abs_diff(0)` are the same function, so the injected defect passed.
    // A target near 255 is where they diverge -- 3.wrapping_sub(255) is 4, so a dark pixel would
    // be caught by a tolerance of 5 around WHITE.
    let mut editor = row(&[
        Pixel::rgba(255, 255, 255, 255),
        Pixel::rgba(252, 252, 252, 255),
        Pixel::rgba(3, 3, 3, 255),
        Pixel::rgba(0, 0, 0, 255),
    ]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ColorExchange {
                from: Pixel::rgba(255, 255, 255, 255),
                to: Pixel::rgba(255, 0, 0, 255),
                red_threshold: 5,
                green_threshold: 5,
                blue_threshold: 5,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    assert_eq!((out[0], out[1], out[2]), (255, 0, 0), "white matches");
    assert_eq!((out[4], out[5], out[6]), (255, 0, 0), "and so does 252");
    assert_eq!(
        (out[8], out[9], out[10]),
        (3, 3, 3),
        "3 is 252 away from white — a wrapped comparison would read it as 4 and match it"
    );
    assert_eq!(
        (out[12], out[13], out[14]),
        (0, 0, 0),
        "and black is 255 away, which a wrapped comparison would read as 1"
    );
}

/// Alpha is untouched, and neither colour's own alpha participates.
///
/// Exchanging coverage would let the operation erase or reveal pixels, which "swap one color with
/// another" does not mean. A transparent pixel whose colour matches is still exchanged — its
/// colour is what matched — but it stays transparent.
#[test]
fn color_exchange_leaves_alpha_alone() {
    let mut editor = row(&[Pixel::rgba(70, 70, 70, 40), Pixel::rgba(70, 70, 70, 255)]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ColorExchange {
                // Both colours carry a deliberately odd alpha that must be ignored.
                from: Pixel::rgba(70, 70, 70, 7),
                to: Pixel::rgba(10, 20, 30, 200),
                red_threshold: 0,
                green_threshold: 0,
                blue_threshold: 0,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    assert_eq!(
        (out[0], out[1], out[2]),
        (10, 20, 30),
        "matching ignores the from-colour's alpha"
    );
    assert_eq!(
        out[3], 40,
        "the pixel keeps its own alpha, not the to-colour's"
    );
    assert_eq!((out[4], out[5], out[6]), (10, 20, 30));
    assert_eq!(out[7], 255, "and so does the opaque one");
}

/// Helper: color-rotate with the grey block switched off.
///
/// A zero gray threshold means no pixel counts as grey, so these tests measure the arc mapping on
/// its own. Stated once here because every arc test needs it and repeating it would invite one of
/// them to drift.
fn rotate(source_from: f32, source_to: f32, dest_from: f32, dest_to: f32) -> Filter {
    Filter::ColorRotate {
        source_from,
        source_to,
        dest_from,
        dest_to,
        gray_mode: redrob_core::GrayMode::TreatAsThis,
        gray_threshold: 0.0,
        gray_hue: 0.0,
        gray_saturation: 0.0,
    }
}

/// Reads a pixel's hue in degrees.
fn hue_of(rgba: &[u8]) -> f32 {
    let r = f32::from(rgba[0]) / 255.0;
    let g = f32::from(rgba[1]) / 255.0;
    let b = f32::from(rgba[2]) / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    assert!(delta > 1e-6, "this pixel is grey and has no hue to read");
    let hue = if (max - r).abs() < 1e-6 {
        60.0 * ((g - b) / delta % 6.0)
    } else if (max - g).abs() < 1e-6 {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    hue.rem_euclid(360.0)
}

/// A source arc maps proportionally onto the destination arc.
///
/// The midpoint of the source must land on the midpoint of the destination; that is what
/// "replace a range of colors with another" means, as opposed to a flat hue offset which would
/// move every hue by the same amount regardless of the arcs' lengths.
#[test]
fn color_rotate_maps_the_source_arc_proportionally() {
    // Source 0..120 (red to green), destination 180..300 (cyan to magenta).
    // Red 0 -> 180, yellow 60 -> 240, green 120 -> 300.
    let red = Pixel::rgba(255, 0, 0, 255);
    let yellow = Pixel::rgba(255, 255, 0, 255);
    let green = Pixel::rgba(0, 255, 0, 255);
    let mut editor = row(&[red, yellow, green]);
    editor
        .execute(Command::ApplyFilter {
            filter: rotate(0.0, 120.0, 180.0, 300.0),
        })
        .unwrap();
    let out = pixels(&editor);

    assert!(
        (hue_of(&out[0..4]) - 180.0).abs() < 2.0,
        "the arc start maps to the destination start, got {}",
        hue_of(&out[0..4])
    );
    assert!(
        (hue_of(&out[4..8]) - 240.0).abs() < 2.0,
        "the MIDPOINT maps to the destination midpoint, got {}",
        hue_of(&out[4..8])
    );
    assert!(
        (hue_of(&out[8..12]) - 300.0).abs() < 2.0,
        "the arc end maps to the destination end, got {}",
        hue_of(&out[8..12])
    );
}

/// A shorter destination arc COMPRESSES the hues into it.
///
/// Distinguishes a proportional mapping from a flat offset: with a 120-degree source and a
/// 30-degree destination, the source midpoint must land at the destination midpoint, which is a
/// quarter of the way round compared to where an offset would put it.
#[test]
fn color_rotate_compresses_into_a_shorter_destination() {
    let yellow = Pixel::rgba(255, 255, 0, 255); // hue 60, the middle of 0..120
    let mut editor = row(&[yellow]);
    editor
        .execute(Command::ApplyFilter {
            filter: rotate(0.0, 120.0, 200.0, 230.0),
        })
        .unwrap();
    let out = pixels(&editor);
    assert!(
        (hue_of(&out[0..4]) - 215.0).abs() < 2.0,
        "half way along a 30-degree destination is 215, got {}",
        hue_of(&out[0..4])
    );
}

/// An arc that WRAPS past 360 is the arc the user asked for.
///
/// This is the property a plain subtraction gets wrong, and it gets it wrong silently: 300 to 60
/// is a 120-degree arc through red, but `to - from` is -240, which would both invert the direction
/// and change the length. Magenta at 300 is the arc's start and must land on the destination's
/// start.
#[test]
fn color_rotate_handles_an_arc_that_wraps_past_zero() {
    let magenta = Pixel::rgba(255, 0, 255, 255); // hue 300, the arc start
    let red = Pixel::rgba(255, 0, 0, 255); // hue 0, half way along 300..60
    let mut editor = row(&[magenta, red]);
    editor
        .execute(Command::ApplyFilter {
            filter: rotate(300.0, 60.0, 90.0, 150.0),
        })
        .unwrap();
    let out = pixels(&editor);
    assert!(
        (hue_of(&out[0..4]) - 90.0).abs() < 2.0,
        "the wrapped arc's start maps to the destination start, got {}",
        hue_of(&out[0..4])
    );
    assert!(
        (hue_of(&out[4..8]) - 120.0).abs() < 2.0,
        "and its midpoint to the destination midpoint, got {}",
        hue_of(&out[4..8])
    );
}

/// Hues outside the source arc are left alone.
///
/// "Replace a range of colors" means the rest of the wheel is not a range. A filter that rotated
/// everything would be a hue shift, which is a different operation we already have.
#[test]
fn color_rotate_leaves_hues_outside_the_source_arc_untouched() {
    let blue = Pixel::rgba(0, 0, 255, 255); // hue 240, outside 0..120
    let mut editor = row(&[blue]);
    let before = pixels(&editor);
    editor
        .execute(Command::ApplyFilter {
            filter: rotate(0.0, 120.0, 180.0, 300.0),
        })
        .unwrap();
    assert_eq!(
        pixels(&editor),
        before,
        "a hue outside the source arc must survive untouched"
    );
}

/// "Change to this" replaces a grey outright, with no rotation.
#[test]
fn color_rotate_gray_change_to_this_replaces_without_rotating() {
    let grey = Pixel::rgba(128, 128, 128, 255);
    let mut editor = row(&[grey]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ColorRotate {
                // A source arc that CONTAINS the grey hue, so a rotation would be visible if one
                // were wrongly applied.
                source_from: 0.0,
                source_to: 120.0,
                dest_from: 200.0,
                dest_to: 260.0,
                gray_mode: redrob_core::GrayMode::ChangeToThis,
                gray_threshold: 0.5,
                gray_hue: 60.0,
                gray_saturation: 1.0,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    assert!(
        (hue_of(&out[0..4]) - 60.0).abs() < 2.0,
        "the grey takes the configured hue and is NOT rotated into 200..260, got {}",
        hue_of(&out[0..4])
    );
}

/// "Treat as this" lends the grey a colour and THEN rotates it.
///
/// The two modes differ only in whether the rotation applies, so this is the other half of the
/// pair: the same grey, the same configured hue, the same arcs — and a different answer.
#[test]
fn color_rotate_gray_treat_as_this_rotates_the_lent_colour() {
    let grey = Pixel::rgba(128, 128, 128, 255);
    let mut editor = row(&[grey]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ColorRotate {
                source_from: 0.0,
                source_to: 120.0,
                dest_from: 200.0,
                dest_to: 260.0,
                gray_mode: redrob_core::GrayMode::TreatAsThis,
                gray_threshold: 0.5,
                // Hue 60 is the middle of the source arc, so it must land in the middle of the
                // destination: 230.
                gray_hue: 60.0,
                gray_saturation: 1.0,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    assert!(
        (hue_of(&out[0..4]) - 230.0).abs() < 2.0,
        "the lent hue must be rotated to the destination midpoint, got {}",
        hue_of(&out[0..4])
    );
}

/// A pixel above the grey threshold is not treated as grey.
///
/// The threshold is the only thing separating the two populations, so a saturated pixel must take
/// the ordinary path even when the grey block is configured to something conspicuous.
#[test]
fn color_rotate_respects_the_gray_threshold() {
    let saturated = Pixel::rgba(255, 255, 0, 255); // saturation 1.0, hue 60
    let mut editor = row(&[saturated]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ColorRotate {
                source_from: 0.0,
                source_to: 120.0,
                dest_from: 200.0,
                dest_to: 260.0,
                gray_mode: redrob_core::GrayMode::ChangeToThis,
                // Well below this pixel's saturation, so the grey branch must not fire.
                gray_threshold: 0.2,
                gray_hue: 300.0,
                gray_saturation: 1.0,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    assert!(
        (hue_of(&out[0..4]) - 230.0).abs() < 2.0,
        "a saturated pixel takes the arc path, not the grey one, got {}",
        hue_of(&out[0..4])
    );
}

/// The target colour becomes fully transparent.
#[test]
fn color_to_alpha_removes_the_target_colour() {
    let target = Pixel::rgba(255, 255, 255, 255);
    let mut editor = row(&[target, Pixel::rgba(0, 0, 0, 255)]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ColorToAlpha {
                color: target,
                transparency_threshold: 0.0,
                opacity_threshold: 1.0,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    assert_eq!(out[3], 0, "the target colour must become transparent");
    assert_eq!(
        out[7], 255,
        "a colour at the far end of the range stays opaque"
    );
}

/// The distance is CHEBYSHEV — the largest per-channel difference — not Euclidean.
///
/// Taken from the vendored prop GUI's pick callback, which computes a threshold as the MAX over
/// the three channel differences. The two metrics disagree measurably: a colour differing by 0.2
/// on all three channels is 0.2 away by Chebyshev and 0.346 away by Euclidean, so with an opacity
/// threshold of 0.3 it is inside the ramp under one reading and fully opaque under the other.
#[test]
fn color_to_alpha_distance_is_chebyshev_not_euclidean() {
    // 0.2 of 255 is 51. Target black, so this pixel is (51, 51, 51).
    let mut editor = row(&[Pixel::rgba(51, 51, 51, 255)]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ColorToAlpha {
                color: Pixel::rgba(0, 0, 0, 255),
                transparency_threshold: 0.0,
                opacity_threshold: 0.3,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    // Chebyshev 0.2 / 0.3 = 0.667 coverage -> alpha about 170.
    // Euclidean would be 0.346, past the 0.3 threshold, leaving alpha at 255.
    assert!(
        out[3] > 160 && out[3] < 180,
        "Chebyshev distance gives alpha near 170, got {} (255 would mean Euclidean)",
        out[3]
    );
}

/// One channel far from the target is enough, because the metric takes the maximum.
///
/// The other half of the Chebyshev property: a pixel matching the target exactly on two channels
/// is still distant if the third differs. A metric that averaged, or that required all three to
/// differ, would keep this pixel.
#[test]
fn color_to_alpha_a_single_distant_channel_decides() {
    let mut editor = row(&[Pixel::rgba(0, 0, 255, 255)]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ColorToAlpha {
                color: Pixel::rgba(0, 0, 0, 255),
                transparency_threshold: 0.0,
                opacity_threshold: 0.5,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    assert_eq!(
        out[3], 255,
        "blue is 1.0 away from black on one channel, which is past the 0.5 threshold"
    );
}

/// Between the two thresholds the alpha ramps, giving a soft edge.
///
/// The band is the reason there are two properties rather than one. With a single cutoff the
/// result is a hard cut-out, which is not what "farthest full-transparency" and "nearest
/// full-opacity" describe.
#[test]
fn color_to_alpha_ramps_between_the_two_thresholds() {
    // Target black; thresholds 0.2 and 0.6. A pixel at 0.4 is half way along the band.
    let half = (0.4 * 255.0) as u8;
    let mut editor = row(&[
        Pixel::rgba(25, 25, 25, 255), // 0.098, below the transparency threshold
        Pixel::rgba(half, half, half, 255), // mid band
        Pixel::rgba(200, 200, 200, 255), // 0.784, above the opacity threshold
    ]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ColorToAlpha {
                color: Pixel::rgba(0, 0, 0, 255),
                transparency_threshold: 0.2,
                opacity_threshold: 0.6,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    assert_eq!(out[3], 0, "below the transparency threshold is fully clear");
    assert!(
        out[7] > 115 && out[7] < 140,
        "half way along the band is about half alpha, got {}",
        out[7]
    );
    assert_eq!(out[11], 255, "above the opacity threshold is fully opaque");
}

/// The kept colour is UNMIXED, so it carries no tint of the removed colour.
///
/// This is what makes the operation "color to alpha" rather than "color to mask", and it is the
/// whole reason it is useful for knocking out a background. A mid-grey over white at half coverage
/// must come back as BLACK: it is being read as black showing through white at 50%, and inverting
/// source-over recovers the black. A plain alpha mask would leave it grey, and the grey would
/// reappear as a halo the moment the layer was composited over anything dark.
#[test]
fn color_to_alpha_unmixes_the_remaining_colour() {
    // White target. A pixel at 50% between black and white, with a band that puts it at coverage
    // 0.5: distance from white is 0.5, thresholds 0.0 and 1.0.
    let mut editor = row(&[Pixel::rgba(128, 128, 128, 255)]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ColorToAlpha {
                color: Pixel::rgba(255, 255, 255, 255),
                transparency_threshold: 0.0,
                opacity_threshold: 1.0,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    assert!(
        out[3] > 120 && out[3] < 136,
        "coverage is about half, got alpha {}",
        out[3]
    );
    assert!(
        out[0] < 12,
        "the colour must be unmixed back to black, got {} — a mask would have left it at 128",
        out[0]
    );
}

/// Existing transparency is composed with, never increased.
///
/// Running the filter on an already part-transparent area must not make it more opaque: alpha
/// multiplies rather than replaces. Otherwise applying the filter twice, or applying it inside a
/// feathered selection, would resurrect coverage the user had already removed.
#[test]
fn color_to_alpha_composes_with_existing_alpha() {
    let mut editor = row(&[Pixel::rgba(128, 128, 128, 100)]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ColorToAlpha {
                color: Pixel::rgba(255, 255, 255, 255),
                transparency_threshold: 0.0,
                opacity_threshold: 1.0,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    assert!(
        out[3] < 100,
        "alpha must compose with the existing 100, got {}",
        out[3]
    );
    assert!(out[3] > 40, "and not collapse to nothing, got {}", out[3]);
}

/// A degenerate band is a hard cutoff rather than a division by zero.
#[test]
fn color_to_alpha_handles_a_degenerate_band() {
    let mut editor = row(&[
        Pixel::rgba(10, 10, 10, 255),
        Pixel::rgba(200, 200, 200, 255),
    ]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ColorToAlpha {
                color: Pixel::rgba(0, 0, 0, 255),
                // Both thresholds equal: no ramp at all.
                transparency_threshold: 0.5,
                opacity_threshold: 0.5,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    assert_eq!(out[3], 0, "inside the cutoff is clear");
    assert_eq!(out[7], 255, "outside it is opaque");
}

/// Extracting a component produces a MONO image: all three channels carry the same value.
///
/// That is what "extract component" means. A version that only replaced one channel would leave a
/// coloured image, which is not a component view.
#[test]
fn component_extract_produces_a_mono_image() {
    let mut editor = row(&[Pixel::rgba(200, 100, 50, 255)]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ComponentExtract {
                component: redrob_core::ColorComponent::Red,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    assert_eq!(
        (out[0], out[1], out[2]),
        (200, 200, 200),
        "the red component must appear in all three channels"
    );
    assert_eq!(out[3], 255, "alpha preserved");
}

/// Each of the three stored channels is read from its own slot.
///
/// Guards against the off-by-one that is invisible on a grey test pixel: a filter reading
/// `pixel[0]` for all three would pass a single-channel test and fail this one.
#[test]
fn component_extract_reads_the_right_channel() {
    let source = Pixel::rgba(10, 120, 240, 255);
    for (component, expected) in [
        (redrob_core::ColorComponent::Red, 10u8),
        (redrob_core::ColorComponent::Green, 120),
        (redrob_core::ColorComponent::Blue, 240),
    ] {
        let mut editor = row(&[source]);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::ComponentExtract { component },
            })
            .unwrap();
        assert_eq!(
            pixels(&editor)[0],
            expected,
            "{component:?} must read its own channel"
        );
    }
}

/// Alpha can be extracted, and doing so does not consume the alpha itself.
///
/// The result is an IMAGE of the mask. Making it transparent where the mask is dark would hide the
/// very thing being inspected, so alpha is preserved even when alpha is the component.
#[test]
fn component_extract_renders_alpha_without_consuming_it() {
    let mut editor = row(&[Pixel::rgba(255, 0, 0, 64)]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ComponentExtract {
                component: redrob_core::ColorComponent::Alpha,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    assert_eq!(
        (out[0], out[1], out[2]),
        (64, 64, 64),
        "the alpha value must be rendered as the colour"
    );
    assert_eq!(
        out[3], 64,
        "and the pixel must keep its alpha, or the view hides itself"
    );
}

/// Value, Lightness and Luminance are three DIFFERENT components.
///
/// They are easy to conflate and each is a distinct definition: HSV value is `max`, HSL lightness
/// is `(max + min) / 2`, and Rec. 709 luminance is a weighted sum. Offering one under another's
/// name would quietly deny the user the one they asked for, and nothing about a single output
/// would reveal it.
///
/// Pure green makes the point: value 255, lightness 128, luminance 182.
#[test]
fn component_extract_value_lightness_and_luminance_differ() {
    let green = Pixel::rgba(0, 255, 0, 255);

    let sample = |component| {
        let mut editor = row(&[green]);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::ComponentExtract { component },
            })
            .unwrap();
        pixels(&editor)[0]
    };

    assert_eq!(
        sample(redrob_core::ColorComponent::Value),
        255,
        "HSV value is the largest channel"
    );
    let lightness = sample(redrob_core::ColorComponent::Lightness);
    assert!(
        lightness.abs_diff(128) <= 1,
        "HSL lightness is (max+min)/2 = 128, got {lightness}"
    );
    let luminance = sample(redrob_core::ColorComponent::Luminance);
    assert!(
        luminance.abs_diff(182) <= 1,
        "Rec. 709 luminance of pure green is 182, not 85 — got {luminance}"
    );
}

/// Lab lightness is PERCEPTUAL and therefore not relative luminance.
///
/// Mid-grey is the clearest case: L* puts it near 50 of 100 (so 128 of 255 here) where relative
/// luminance puts it near 22 of 100 (so about 55). Picking one for the other is a plausible
/// mistake with a very visible result, once you know to look.
#[test]
fn component_extract_lab_lightness_is_not_luminance() {
    let mid_grey = Pixel::rgba(128, 128, 128, 255);

    let sample = |component| {
        let mut editor = row(&[mid_grey]);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::ComponentExtract { component },
            })
            .unwrap();
        pixels(&editor)[0]
    };

    let lab_l = sample(redrob_core::ColorComponent::LabLightness);
    let luminance = sample(redrob_core::ColorComponent::Luminance);
    assert!(
        lab_l > 125 && lab_l < 145,
        "L* of mid-grey is about 53.6 of 100, i.e. near 137 of 255, got {lab_l}"
    );
    assert_eq!(
        luminance, 128,
        "relative luminance of a neutral equals the channel value itself"
    );
    assert!(
        lab_l.abs_diff(luminance) > 5,
        "the two must differ: perceptual lightness is not relative luminance"
    );
}

/// The signed Lab axes are offset so negative values survive.
///
/// a* and b* run roughly -128..127. Without the offset every negative value clamps to 0 and half
/// of each axis renders as flat black — which looks like a working filter on a warm image and
/// loses everything on a cool one.
#[test]
fn component_extract_offsets_the_signed_lab_axes() {
    // Blue has a strongly NEGATIVE b*, red a positive a*.
    let blue = Pixel::rgba(0, 0, 255, 255);
    let red = Pixel::rgba(255, 0, 0, 255);
    let grey = Pixel::rgba(128, 128, 128, 255);

    let sample = |color, component| {
        let mut editor = row(&[color]);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::ComponentExtract { component },
            })
            .unwrap();
        pixels(&editor)[0]
    };

    // A neutral has a* = b* = 0, which must land on the offset itself rather than at an end.
    let grey_a = sample(grey, redrob_core::ColorComponent::LabA);
    let grey_b = sample(grey, redrob_core::ColorComponent::LabB);
    assert!(
        grey_a.abs_diff(128) <= 2 && grey_b.abs_diff(128) <= 2,
        "a neutral must sit at the offset, got a*={grey_a} b*={grey_b}"
    );

    // Blue's b* is around -108, so it must land well BELOW the offset and not clamp to zero.
    let blue_b = sample(blue, redrob_core::ColorComponent::LabB);
    assert!(
        blue_b > 5 && blue_b < 60,
        "blue's negative b* must survive the offset, got {blue_b}"
    );

    // Red's a* is around +80, above the offset.
    let red_a = sample(red, redrob_core::ColorComponent::LabA);
    assert!(
        red_a > 180,
        "red's positive a* must land above the offset, got {red_a}"
    );
}

/// Hue is reported as a fraction of a turn, and a grey has no hue to report.
#[test]
fn component_extract_hue_scales_a_turn_into_a_byte() {
    let sample = |color| {
        let mut editor = row(&[color]);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::ComponentExtract {
                    component: redrob_core::ColorComponent::Hue,
                },
            })
            .unwrap();
        pixels(&editor)[0]
    };

    assert_eq!(sample(Pixel::rgba(255, 0, 0, 255)), 0, "red is hue 0");
    let green = sample(Pixel::rgba(0, 255, 0, 255));
    assert!(
        green.abs_diff(85) <= 1,
        "green is 120 degrees, a third of a turn, so 85 — got {green}"
    );
    let blue = sample(Pixel::rgba(0, 0, 255, 255));
    assert!(
        blue.abs_diff(170) <= 1,
        "blue is 240 degrees, two thirds, so 170 — got {blue}"
    );
    assert_eq!(
        sample(Pixel::rgba(128, 128, 128, 255)),
        0,
        "a grey has no hue and reports 0, which is red's position rather than a real value"
    );
}

/// LCh chroma measures colourfulness, independent of lightness.
///
/// This is what separates it from every other component already here: a neutral has zero chroma
/// whatever its lightness, and a vivid colour has high chroma whatever its lightness. A component
/// that tracked lightness instead would pass no part of this.
#[test]
fn component_extract_lch_chroma_is_colourfulness_not_lightness() {
    let sample = |color| {
        let mut editor = row(&[color]);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::ComponentExtract {
                    component: redrob_core::ColorComponent::LchChroma,
                },
            })
            .unwrap();
        pixels(&editor)[0]
    };

    // Three neutrals of very different lightness must all read ~0.
    for grey in [30u8, 128, 220] {
        let chroma = sample(Pixel::rgba(grey, grey, grey, 255));
        assert!(
            chroma <= 2,
            "a neutral at {grey} must have no chroma, got {chroma}"
        );
    }
    // A vivid colour must read high.
    let red = sample(Pixel::rgba(255, 0, 0, 255));
    assert!(red > 100, "pure red is strongly chromatic, got {red}");
}

/// LCh hue is PERCEPTUALLY spaced, so it disagrees with the HSV wheel.
///
/// Both are "hue", and offering only one would be defensible — but they are measurably different,
/// and that difference is the reason Lab exists. Pure blue is the clearest case: HSV puts it at
/// exactly two thirds of the wheel, Lab's hue angle does not.
#[test]
fn component_extract_lch_hue_differs_from_the_hsv_wheel() {
    let blue = Pixel::rgba(0, 0, 255, 255);

    let sample = |component| {
        let mut editor = row(&[blue]);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::ComponentExtract { component },
            })
            .unwrap();
        pixels(&editor)[0]
    };

    let hsv = sample(redrob_core::ColorComponent::Hue);
    let lch = sample(redrob_core::ColorComponent::LchHue);
    assert!(
        hsv.abs_diff(170) <= 1,
        "the HSV wheel puts blue at two thirds, 170, got {hsv}"
    );
    assert!(
        lch.abs_diff(hsv) > 5,
        "perceptual hue must differ from the HSV wheel: {lch} vs {hsv}"
    );
}

/// u'v' is the CIE 1976 form, not the 1960 one.
///
/// The two differ in a single coefficient — v = 6Y/d in 1960 against v' = 9Y/d in 1976 — so the
/// wrong choice gives plausible numbers that are off by exactly 1.5 on one axis and correct on the
/// other. That is why this is asserted against a computed value rather than merely checked for
/// being non-zero.
///
/// The D65 white point sits at u' = 0.1978, v' = 0.4683. Scaled by 1/0.7 into a byte that is 72
/// and 171.
#[test]
fn component_extract_yuv_is_the_1976_form() {
    let white = Pixel::rgba(255, 255, 255, 255);

    let sample = |component| {
        let mut editor = row(&[white]);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::ComponentExtract { component },
            })
            .unwrap();
        pixels(&editor)[0]
    };

    let u = sample(redrob_core::ColorComponent::YuvU);
    let v = sample(redrob_core::ColorComponent::YuvV);
    assert!(
        u.abs_diff(72) <= 2,
        "u' of the white point is 0.1978, i.e. 72 of 255, got {u}"
    );
    assert!(
        v.abs_diff(171) <= 2,
        "v' of the white point is 0.4683, i.e. 171 — the 1960 form would give 114, got {v}"
    );
}

/// xyY chromaticity sums with z to one, which is what makes it a chromaticity.
///
/// x + y + z = 1 by construction, so x + y must never exceed 1. Checking it on several colours
/// catches a normalisation that divided by the wrong sum.
#[test]
fn component_extract_xyy_chromaticities_are_normalised() {
    for color in [
        Pixel::rgba(255, 0, 0, 255),
        Pixel::rgba(0, 255, 0, 255),
        Pixel::rgba(0, 0, 255, 255),
        Pixel::rgba(255, 255, 255, 255),
        Pixel::rgba(90, 140, 40, 255),
    ] {
        let sample = |component| {
            let mut editor = row(&[color]);
            editor
                .execute(Command::ApplyFilter {
                    filter: Filter::ComponentExtract { component },
                })
                .unwrap();
            f32::from(pixels(&editor)[0]) / 255.0
        };
        let x = sample(redrob_core::ColorComponent::XyyX);
        let y = sample(redrob_core::ColorComponent::XyyY);
        assert!(
            x + y <= 1.01,
            "x + y must not exceed 1 for {color:?}, got {x} + {y}"
        );
        assert!(x > 0.0 && y > 0.0, "a visible colour has both positive");
    }
}

/// The unprofiled CMYK separation puts black entirely in the key channel.
///
/// The defining property of the default separation: K takes everything it can, so a pure black has
/// no chromatic ink at all and a pure colour has no key. A separation that spread black across
/// CMY — which is what a press profile might legitimately do — is exactly what we are NOT claiming
/// to implement.
#[test]
fn component_extract_device_cmyk_puts_black_in_the_key() {
    let sample = |color, component| {
        let mut editor = row(&[color]);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::ComponentExtract { component },
            })
            .unwrap();
        pixels(&editor)[0]
    };

    let black = Pixel::rgba(0, 0, 0, 255);
    assert_eq!(
        sample(black, redrob_core::ColorComponent::CmykKey),
        255,
        "black is all key"
    );
    for chromatic in [
        redrob_core::ColorComponent::CmykCyan,
        redrob_core::ColorComponent::CmykMagenta,
        redrob_core::ColorComponent::CmykYellow,
    ] {
        assert_eq!(
            sample(black, chromatic),
            0,
            "{chromatic:?} must be empty for black"
        );
    }

    // Pure red is magenta plus yellow, no cyan, no key.
    let red = Pixel::rgba(255, 0, 0, 255);
    assert_eq!(sample(red, redrob_core::ColorComponent::CmykCyan), 0);
    assert_eq!(sample(red, redrob_core::ColorComponent::CmykMagenta), 255);
    assert_eq!(sample(red, redrob_core::ColorComponent::CmykYellow), 255);
    assert_eq!(
        sample(red, redrob_core::ColorComponent::CmykKey),
        0,
        "a full-brightness colour needs no key"
    );

    // White is nothing at all.
    let white = Pixel::rgba(255, 255, 255, 255);
    for component in [
        redrob_core::ColorComponent::CmykCyan,
        redrob_core::ColorComponent::CmykMagenta,
        redrob_core::ColorComponent::CmykYellow,
        redrob_core::ColorComponent::CmykKey,
    ] {
        assert_eq!(
            sample(white, component),
            0,
            "{component:?} must be empty for white — paper is the white"
        );
    }
}

/// The gains weight each channel, producing one grey.
///
/// A gain of 1 on red alone must copy the red channel into all three, which is the simplest
/// statement that the weights address the channels they claim to.
#[test]
fn mono_mixer_weights_each_channel() {
    let source = Pixel::rgba(200, 100, 50, 255);
    for (gains, expected) in [
        ((1.0, 0.0, 0.0), 200u8),
        ((0.0, 1.0, 0.0), 100),
        ((0.0, 0.0, 1.0), 50),
    ] {
        let mut editor = row(&[source]);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::MonoMixer {
                    red_gain: gains.0,
                    green_gain: gains.1,
                    blue_gain: gains.2,
                    preserve_luminosity: false,
                },
            })
            .unwrap();
        let out = pixels(&editor);
        assert_eq!(
            (out[0], out[1], out[2]),
            (expected, expected, expected),
            "gains {gains:?} must select that channel into all three"
        );
    }
}

/// preserve_luminosity normalises the gains, so balance and brightness are independent.
///
/// This is the whole point of the flag. Gains of (2, 2, 2) are a triple-brightness setting with it
/// off and an equal-weight average with it on — the ratio between them is unchanged, which is what
/// "preserve luminosity" claims.
#[test]
fn mono_mixer_preserve_luminosity_normalises_the_gains() {
    let source = Pixel::rgba(90, 90, 90, 255);

    let mut unnormalised = row(&[source]);
    unnormalised
        .execute(Command::ApplyFilter {
            filter: Filter::MonoMixer {
                red_gain: 2.0,
                green_gain: 2.0,
                blue_gain: 2.0,
                preserve_luminosity: false,
            },
        })
        .unwrap();
    assert_eq!(
        pixels(&unnormalised)[0],
        255,
        "without the flag, gains summing to 6 blow past the range"
    );

    let mut normalised = row(&[source]);
    normalised
        .execute(Command::ApplyFilter {
            filter: Filter::MonoMixer {
                red_gain: 2.0,
                green_gain: 2.0,
                blue_gain: 2.0,
                preserve_luminosity: true,
            },
        })
        .unwrap();
    assert_eq!(
        pixels(&normalised)[0],
        90,
        "with the flag, equal gains are an average whatever their magnitude"
    );
}

/// Normalising preserves the RATIO between channels, not just the total.
///
/// A weaker test could pass by simply scaling the output. This one checks that gains of (6, 3, 3)
/// and (2, 1, 1) give the same answer under the flag, which only holds if the gains themselves
/// were normalised.
#[test]
fn mono_mixer_preserve_luminosity_keeps_the_balance() {
    let source = Pixel::rgba(240, 60, 30, 255);

    let sample = |gains: (f32, f32, f32)| {
        let mut editor = row(&[source]);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::MonoMixer {
                    red_gain: gains.0,
                    green_gain: gains.1,
                    blue_gain: gains.2,
                    preserve_luminosity: true,
                },
            })
            .unwrap();
        pixels(&editor)[0]
    };

    let scaled = sample((6.0, 3.0, 3.0));
    let unit = sample((2.0, 1.0, 1.0));
    assert_eq!(
        scaled, unit,
        "proportional gains must give the same result once normalised"
    );
    // And it really is a weighted mix, not an average: red is weighted double here.
    // (2*240 + 1*60 + 1*30) / 4 = 142.5
    assert!(
        unit.abs_diff(143) <= 1,
        "the weighted mix of (240, 60, 30) at 2:1:1 is 142.5, got {unit}"
    );
}

/// Gains summing to zero are passed through rather than divided by.
///
/// (1, 0, -1) is a legitimate difference-of-channels setting whose sum is zero. Normalising it is
/// impossible, so the flag leaves it alone — which is better than a division producing infinities,
/// and better than refusing a setting the user is entitled to.
#[test]
fn mono_mixer_handles_gains_that_sum_to_zero() {
    let mut editor = row(&[Pixel::rgba(200, 100, 50, 255)]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::MonoMixer {
                red_gain: 1.0,
                green_gain: 0.0,
                blue_gain: -1.0,
                preserve_luminosity: true,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    assert_eq!(
        out[0], 150,
        "red minus blue is 150, computed with the gains left unnormalised"
    );
}

/// A negative gain subtracts, and the result clamps rather than wrapping.
///
/// The gains are unbounded on purpose — that is how a channel is emphasised or subtracted — so
/// going out of range is the normal case. It must clamp: a wrap would turn a dark result bright.
#[test]
fn mono_mixer_clamps_rather_than_wrapping() {
    let mut editor = row(&[Pixel::rgba(10, 200, 200, 255)]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::MonoMixer {
                red_gain: 1.0,
                green_gain: -1.0,
                blue_gain: -1.0,
                preserve_luminosity: false,
            },
        })
        .unwrap();
    assert_eq!(
        pixels(&editor)[0],
        0,
        "10 - 200 - 200 is strongly negative and must clamp to 0, not wrap bright"
    );
}

/// channel-mixer's preserve_luminosity normalises each OUTPUT ROW, not all nine gains.
///
/// Per-row is what upstream's own layout says: `gimppropgui-channel-mixer.c` groups the nine gains
/// into three frames labelled "Red Channel", "Green Channel" and "Blue Channel" — so a frame is a
/// row — with the single checkbox outside all three.
///
/// The distinction is measurable. Here the red row is (2, 2, 2) and the other two rows are
/// identity. Normalised per row, red becomes a plain average of the inputs and green and blue are
/// untouched. Normalised over all nine instead, every weight would be divided by 8 and the
/// identity rows would collapse to near-black — so the two readings cannot be confused.
#[test]
fn channel_mixer_preserve_luminosity_normalises_each_row() {
    let source = Pixel::rgba(90, 120, 150, 255);
    let mut editor = row(&[source]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ChannelMixer {
                matrix: [
                    2.0, 2.0, 2.0, // red out: equal weights, magnitude 6
                    0.0, 1.0, 0.0, // green out: identity
                    0.0, 0.0, 1.0, // blue out: identity
                ],
                offset: [0.0, 0.0, 0.0],
                preserve_luminosity: true,
            },
        })
        .unwrap();
    let out = pixels(&editor);

    // (90 + 120 + 150) / 3 = 120
    assert!(
        out[0].abs_diff(120) <= 1,
        "the red row must become a plain average, got {}",
        out[0]
    );
    assert_eq!(
        out[1], 120,
        "an identity row already sums to 1 and must be untouched"
    );
    assert_eq!(out[2], 150, "likewise blue");
}

/// With the flag off, the matrix is used exactly as given.
///
/// This is the pre-existing behaviour and the default, so an older saved command must keep meaning
/// what it meant. The same row of (2, 2, 2) that averages above must overflow here.
#[test]
fn channel_mixer_without_the_flag_uses_the_matrix_as_given() {
    let source = Pixel::rgba(90, 120, 150, 255);
    let mut editor = row(&[source]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ChannelMixer {
                matrix: [2.0, 2.0, 2.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
                offset: [0.0, 0.0, 0.0],
                preserve_luminosity: false,
            },
        })
        .unwrap();
    assert_eq!(
        pixels(&editor)[0],
        255,
        "unnormalised, 2*(90+120+150) overflows and must clamp"
    );
}

/// Proportional rows agree under the flag, which is what proves the GAINS were normalised.
///
/// A version that scaled the output instead would fail this: (6, 3, 3) and (2, 1, 1) are the same
/// balance at different magnitudes, so they must give the same answer once the row is normalised.
#[test]
fn channel_mixer_preserve_luminosity_keeps_the_balance() {
    let source = Pixel::rgba(240, 60, 30, 255);

    let sample = |row_weights: [f32; 3]| {
        let mut editor = row(&[source]);
        let mut matrix = [0.0f32; 9];
        matrix[0..3].copy_from_slice(&row_weights);
        matrix[4] = 1.0;
        matrix[8] = 1.0;
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::ChannelMixer {
                    matrix,
                    offset: [0.0, 0.0, 0.0],
                    preserve_luminosity: true,
                },
            })
            .unwrap();
        pixels(&editor)[0]
    };

    assert_eq!(
        sample([6.0, 3.0, 3.0]),
        sample([2.0, 1.0, 1.0]),
        "proportional rows must agree once normalised"
    );
    // And it is a weighted mix, not an average: (2*240 + 60 + 30) / 4 = 142.5
    let weighted = sample([2.0, 1.0, 1.0]);
    assert!(
        weighted.abs_diff(143) <= 1,
        "the 2:1:1 mix of (240, 60, 30) is 142.5, got {weighted}"
    );
}

/// A row summing to zero is passed through rather than divided by.
///
/// (1, 0, -1) is a legitimate difference-of-channels row. Same rule as mono-mixer, and stated in
/// both places because the two filters share the flag and must not disagree about its edge case.
#[test]
fn channel_mixer_handles_a_row_summing_to_zero() {
    let mut editor = row(&[Pixel::rgba(200, 100, 50, 255)]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ChannelMixer {
                matrix: [1.0, 0.0, -1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
                offset: [0.0, 0.0, 0.0],
                preserve_luminosity: true,
            },
        })
        .unwrap();
    assert_eq!(
        pixels(&editor)[0],
        150,
        "red minus blue is 150, with the zero-sum row left unnormalised"
    );
}

/// A ChannelMixer command saved BEFORE this field existed still deserialises.
///
/// This is the first time this parity work has added a field to a command variant that already
/// shipped, which is a different risk from adding a new variant: saved history and saved documents
/// contain `ChannelMixer` objects with no `preserve_luminosity` key at all. Without
/// `#[serde(default)]` they would fail to load with a missing-field error — the same defect that
/// hit `ChangeSet::palette_snapped` in J.3-b, caught then by a round-trip test.
///
/// The default must also be `false`, not merely present: `true` would silently change what every
/// previously saved command means.
#[test]
fn channel_mixer_deserialises_without_the_new_field() {
    let json = r#"{
        "kind": "channel_mixer",
        "matrix": [2.0, 2.0, 2.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        "offset": [0.0, 0.0, 0.0]
    }"#;
    let filter: Filter =
        serde_json::from_str(json).expect("an older saved command must still load");

    // And it must mean what it meant before the field existed, which is the unnormalised matrix.
    let mut editor = row(&[Pixel::rgba(90, 120, 150, 255)]);
    editor.execute(Command::ApplyFilter { filter }).unwrap();
    assert_eq!(
        pixels(&editor)[0],
        255,
        "the default must be false, or every saved command quietly changes meaning"
    );
}

/// Strength zero is EXACTLY the input image, not approximately.
///
/// The blend is against the original channel rather than a reconstructed one, so a neutral setting
/// must be byte-identical. A filter whose "off" position shifts the image by a step is one a user
/// cannot trust to preview.
#[test]
fn sepia_at_zero_strength_is_byte_identical() {
    let source = [
        Pixel::rgba(200, 100, 50, 255),
        Pixel::rgba(0, 0, 0, 255),
        Pixel::rgba(17, 200, 243, 128),
    ];
    let mut editor = row(&source);
    let before = pixels(&editor);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Sepia { strength: 0.0 },
        })
        .unwrap();
    assert_eq!(
        pixels(&editor),
        before,
        "strength 0 must leave every byte untouched"
    );
}

/// Full strength produces a warm monochrome: red >= green >= blue on every pixel.
///
/// That ordering IS the sepia tone, and it must hold regardless of the input's own hue — a blue
/// sky and a red brick must both come out warm, or the filter is tinting rather than toning.
#[test]
fn sepia_at_full_strength_is_a_warm_monochrome() {
    let source = [
        Pixel::rgba(0, 0, 255, 255), // strongly blue input
        Pixel::rgba(255, 0, 0, 255), // strongly red input
        Pixel::rgba(120, 120, 120, 255),
    ];
    let mut editor = row(&source);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Sepia { strength: 1.0 },
        })
        .unwrap();
    let out = pixels(&editor);

    for index in 0..3 {
        let (r, g, b) = (out[index * 4], out[index * 4 + 1], out[index * 4 + 2]);
        assert!(
            r >= g && g >= b,
            "pixel {index} must be warm-ordered, got ({r}, {g}, {b})"
        );
        assert!(
            r > b,
            "pixel {index} must actually be warm, not neutral: ({r}, {g}, {b})"
        );
    }
}

/// The monochrome underneath is the SAME luminance the rest of the codebase produces.
///
/// The tone is our choice, but the greyscale it is built on should not be a second definition of
/// luminance. Pure green is the discriminating case: Rec. 709 gives 182 where a channel mean gives
/// 85, so the red channel at full strength (tint 1.0 on red) must land on 182.
#[test]
fn sepia_uses_the_same_luminance_as_the_rest_of_the_crate() {
    let mut editor = row(&[Pixel::rgba(0, 255, 0, 255)]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Sepia { strength: 1.0 },
        })
        .unwrap();
    let out = pixels(&editor);
    assert!(
        out[0].abs_diff(182) <= 1,
        "the red channel carries unscaled luminance, which is 182 for pure green — got {}",
        out[0]
    );

    // And it agrees with the dedicated luminance component, which reads the same weights.
    let mut reference = row(&[Pixel::rgba(0, 255, 0, 255)]);
    reference
        .execute(Command::ApplyFilter {
            filter: Filter::ComponentExtract {
                component: redrob_core::ColorComponent::Luminance,
            },
        })
        .unwrap();
    assert_eq!(
        out[0],
        pixels(&reference)[0],
        "sepia and the luminance component must not be two definitions of luminance"
    );
}

/// The tint MULTIPLIES, so black stays black.
///
/// An additive tint would lift the blacks into a grey-brown haze. That is the difference between
/// toned silver and a cheap colour overlay, and it is visible on exactly one pixel.
#[test]
fn sepia_keeps_black_black() {
    let mut editor = row(&[Pixel::rgba(0, 0, 0, 255)]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Sepia { strength: 1.0 },
        })
        .unwrap();
    let out = pixels(&editor);
    assert_eq!(
        (out[0], out[1], out[2]),
        (0, 0, 0),
        "a multiplicative tint leaves black alone; an additive one would not"
    );
}

/// Partial strength lies between the original and the full tone.
///
/// Checks the blend is a blend rather than a threshold: half strength must sit strictly between
/// the two endpoints on a channel where they differ.
#[test]
fn sepia_partial_strength_interpolates() {
    let source = Pixel::rgba(0, 0, 255, 255);

    let sample = |strength| {
        let mut editor = row(&[source]);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Sepia { strength },
            })
            .unwrap();
        pixels(&editor)[2]
    };

    let original = sample(0.0);
    let half = sample(0.5);
    let full = sample(1.0);
    assert_eq!(original, 255, "blue starts at full");
    assert!(
        full < 50,
        "and ends low, since blue's luminance is small and the blue tint is lowest — got {full}"
    );
    assert!(
        half < original && half > full,
        "half strength must lie strictly between {full} and {original}, got {half}"
    );
}

/// Strength outside 0..1 is clamped rather than extrapolating.
///
/// Unlike mono-mixer's gains, which are unbounded on purpose, a blend above one has no meaning:
/// it would overshoot past the fully-toned image into a caricature of it. Clamping is the honest
/// reading of a 0..1 parameter.
#[test]
fn sepia_clamps_strength_to_the_unit_range() {
    let source = Pixel::rgba(0, 0, 255, 255);

    let sample = |strength| {
        let mut editor = row(&[source]);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Sepia { strength },
            })
            .unwrap();
        pixels(&editor)
    };

    assert_eq!(sample(5.0), sample(1.0), "above one clamps to full");
    assert_eq!(
        sample(-3.0),
        sample(0.0),
        "below zero clamps to the original"
    );
}

/// Every pixel takes the configured hue, whatever its own was.
///
/// That is what "colorize" means: the hue is replaced rather than shifted. Three inputs of
/// completely different hues must all come out on the same one.
#[test]
fn colorize_replaces_every_hue_with_one() {
    let source = [
        Pixel::rgba(255, 0, 0, 255),
        Pixel::rgba(0, 255, 0, 255),
        Pixel::rgba(0, 0, 255, 255),
    ];
    let mut editor = row(&source);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Colorize {
                // A third of a turn: green.
                hue: 1.0 / 3.0,
                saturation: 1.0,
                lightness: 0.0,
            },
        })
        .unwrap();
    let out = pixels(&editor);

    for index in 0..3 {
        let hue = hue_of(&out[index * 4..index * 4 + 4]);
        assert!(
            (hue - 120.0).abs() < 2.0,
            "pixel {index} must take the configured hue of 120 degrees, got {hue}"
        );
    }
}

/// The tonal structure survives: a lighter input stays lighter.
///
/// This is the other half of colorize. Replacing the hue while flattening the tones would be a
/// fill, not a colorize, so the ordering of luminance across pixels must be preserved.
#[test]
fn colorize_preserves_the_tonal_order() {
    let source = [
        Pixel::rgba(30, 30, 30, 255),
        Pixel::rgba(128, 128, 128, 255),
        Pixel::rgba(220, 220, 220, 255),
    ];
    let mut editor = row(&source);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Colorize {
                hue: 0.5,
                saturation: 0.5,
                lightness: 0.0,
            },
        })
        .unwrap();
    let out = pixels(&editor);

    let brightness =
        |i: usize| i32::from(out[i * 4]) + i32::from(out[i * 4 + 1]) + i32::from(out[i * 4 + 2]);
    assert!(
        brightness(0) < brightness(1) && brightness(1) < brightness(2),
        "the dark/mid/light order must survive: {} {} {}",
        brightness(0),
        brightness(1),
        brightness(2)
    );
}

/// Luminance uses GIMP's weights, NOT Rec. 709.
///
/// The two differ, and this filter is a faithful port, so it must use upstream's. GIMP's red
/// weight is 0.22248840 against Rec. 709's 0.2126 — about 4.7% higher — so a pure red input lands
/// on a measurably different luminance, and therefore a different output lightness, under each.
///
/// Asserted against the GIMP figure computed on LINEAR input, which is where upstream computes it:
/// linear red is 1.0, so lum = 0.22248840, and with saturation 0 the result is that luminance
/// written straight out as a non-linear grey — 0.2225 * 255 = 57.
#[test]
fn colorize_uses_gimps_luminance_weights_not_rec_709() {
    let mut editor = row(&[Pixel::rgba(255, 0, 0, 255)]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Colorize {
                hue: 0.0,
                // Zero saturation takes upstream's achromatic path, so the output IS the
                // luminance and the test reads it directly.
                saturation: 0.0,
                lightness: 0.0,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    assert!(
        out[0].abs_diff(57) <= 1,
        "GIMP's red weight 0.22248840 gives 57; Rec. 709's 0.2126 would give 54 — got {}",
        out[0]
    );
    assert_eq!(
        (out[0], out[1], out[2]),
        (out[0], out[0], out[0]),
        "zero saturation is achromatic"
    );
}

/// Luminance is computed on LINEAR values, not on the stored gamma-encoded ones.
///
/// Upstream's `prepare()` states the reason outright: "GIMP_RGB_LUMINANCE() requires the input to
/// be linear RGB for correctness." Mid-grey is the discriminating case: 128/255 is 0.502 encoded
/// but only 0.216 linear, so the two readings differ by more than a factor of two and nothing
/// subtle is needed to tell them apart.
#[test]
fn colorize_computes_luminance_in_linear_light() {
    let mut editor = row(&[Pixel::rgba(128, 128, 128, 255)]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Colorize {
                hue: 0.0,
                saturation: 0.0,
                lightness: 0.0,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    // Linear 0.2159 written straight out as a non-linear grey: 0.2159 * 255 = 55.
    assert!(
        out[0].abs_diff(55) <= 2,
        "linear luminance of mid-grey is 0.216, i.e. 55; using the encoded value would give 128 — got {}",
        out[0]
    );
}

/// Positive lightness lerps toward white; negative scales toward black.
///
/// Two different operations, which is how upstream writes them, and the asymmetry is testable:
/// negative lightness can reach pure black (scaling by zero) while positive lightness reaches
/// pure white (lerping all the way to one). A single signed offset would not behave like this at
/// the extremes.
#[test]
fn colorize_lightness_is_two_operations_not_one() {
    let sample = |lightness| {
        let mut editor = row(&[Pixel::rgba(128, 128, 128, 255)]);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Colorize {
                    hue: 0.0,
                    saturation: 0.0,
                    lightness,
                },
            })
            .unwrap();
        pixels(&editor)[0]
    };

    let neutral = sample(0.0);
    assert!(
        sample(0.5) > neutral,
        "positive lightness must brighten, {} vs {neutral}",
        sample(0.5)
    );
    assert!(
        sample(-0.5) < neutral,
        "negative lightness must darken, {} vs {neutral}",
        sample(-0.5)
    );
    // The extremes are exact, and that is the asymmetry: -1 scales by zero, +1 lerps to one.
    assert_eq!(sample(-1.0), 0, "lightness -1 scales the luminance to zero");
    assert_eq!(sample(1.0), 255, "lightness +1 lerps the luminance to one");
}

/// Alpha is copied through, as upstream's `dest[ALPHA] = src[ALPHA]` does.
#[test]
fn colorize_copies_alpha_through() {
    let mut editor = row(&[Pixel::rgba(200, 50, 50, 77)]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Colorize {
                hue: 0.5,
                saturation: 0.5,
                lightness: 0.0,
            },
        })
        .unwrap();
    assert_eq!(pixels(&editor)[3], 77, "alpha must be untouched");
}

/// Builds a ColorBalance with only the midtones axes set, which is the shape of a command saved
/// before the shadows and highlights ranges existed.
fn balance_midtones(red: f32, green: f32, blue: f32) -> Filter {
    Filter::ColorBalance {
        red,
        green,
        blue,
        red_shadows: 0.0,
        green_shadows: 0.0,
        blue_shadows: 0.0,
        red_highlights: 0.0,
        green_highlights: 0.0,
        blue_highlights: 0.0,
        preserve_luminosity: false,
    }
}

/// Each of the three ranges reaches only its own tones.
///
/// This is the whole point of the three-range structure, and it is what our previous single-shift
/// approximation could not express. A shadows-only correction must move a dark pixel and leave a
/// light one essentially alone, and a highlights-only correction must do the reverse.
#[test]
fn color_balance_each_range_affects_only_its_own_tones() {
    let dark = Pixel::rgba(20, 20, 20, 255);
    let light = Pixel::rgba(235, 235, 235, 255);

    let shift = |filter: Filter| {
        let mut editor = row(&[dark, light]);
        let before = pixels(&editor);
        editor.execute(Command::ApplyFilter { filter }).unwrap();
        let after = pixels(&editor);
        (
            i32::from(after[0]) - i32::from(before[0]),
            i32::from(after[4]) - i32::from(before[4]),
        )
    };

    let (dark_moved, light_moved) = shift(Filter::ColorBalance {
        red: 0.0,
        green: 0.0,
        blue: 0.0,
        red_shadows: 100.0,
        green_shadows: 0.0,
        blue_shadows: 0.0,
        red_highlights: 0.0,
        green_highlights: 0.0,
        blue_highlights: 0.0,
        preserve_luminosity: false,
    });
    assert!(
        dark_moved > 30,
        "a shadows correction must move the dark pixel, moved {dark_moved}"
    );
    assert_eq!(
        light_moved, 0,
        "and must not reach the light one, moved {light_moved}"
    );

    let (dark_moved, light_moved) = shift(Filter::ColorBalance {
        red: 0.0,
        green: 0.0,
        blue: 0.0,
        red_shadows: 0.0,
        green_shadows: 0.0,
        blue_shadows: 0.0,
        red_highlights: 100.0,
        green_highlights: 0.0,
        blue_highlights: 0.0,
        preserve_luminosity: false,
    });
    assert_eq!(
        dark_moved, 0,
        "a highlights correction must not reach the dark pixel, moved {dark_moved}"
    );
    assert!(
        light_moved > 10,
        "and must move the light one, moved {light_moved}"
    );
}

/// The masks are driven by the pixel's LIGHTNESS, not by each channel's own value.
///
/// The three ranges are ranges of the pixel's tone, so all three channels must be weighted by the
/// same number. On a saturated colour the channels differ wildly — here red is 250 and blue is 10 —
/// so a per-channel reading would put them in different ranges and shift them by different
/// amounts. Equal shifts on all three is what proves one shared weight.
#[test]
fn color_balance_masks_are_driven_by_lightness_not_channel_value() {
    let saturated = Pixel::rgba(250, 130, 10, 255);
    let mut editor = row(&[saturated]);
    let before = pixels(&editor);
    editor
        .execute(Command::ApplyFilter {
            // The same correction on all three axes, so any difference in the result comes from
            // the WEIGHT rather than from the corrections.
            filter: balance_midtones(50.0, 50.0, 50.0),
        })
        .unwrap();
    let after = pixels(&editor);

    // Blue is far from clipping, so its shift is the honest one to read; red is near 255 and will
    // clamp, which is upstream's behaviour too.
    let blue_shift = i32::from(after[2]) - i32::from(before[2]);
    let green_shift = i32::from(after[1]) - i32::from(before[1]);
    assert!(
        (blue_shift - green_shift).abs() <= 1,
        "one shared lightness weight must shift green and blue alike: {green_shift} vs {blue_shift}"
    );
    assert!(blue_shift > 0, "and must actually shift, got {blue_shift}");
}

/// Equal corrections in two adjacent ranges behave like one correction over both.
///
/// Upstream states this property outright: "The sum of these masks equals 1 for x in 0..1, so
/// applying the same correction in the shadows and in the midtones is equivalent to applying this
/// correction on a virtual shadows_and_midtones range." It is a strong check on the mask
/// constants, because it only holds if the ramps line up exactly.
#[test]
fn color_balance_adjacent_ranges_sum_as_upstream_states() {
    // A pixel in the ramp between shadows and midtones, where both masks are partly on.
    let mid_dark = Pixel::rgba(70, 70, 70, 255);

    let apply = |shadows: f32, midtones: f32| {
        let mut editor = row(&[mid_dark]);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::ColorBalance {
                    red: midtones,
                    green: 0.0,
                    blue: 0.0,
                    red_shadows: shadows,
                    green_shadows: 0.0,
                    blue_shadows: 0.0,
                    red_highlights: 0.0,
                    green_highlights: 0.0,
                    blue_highlights: 0.0,
                    preserve_luminosity: false,
                },
            })
            .unwrap();
        i32::from(pixels(&editor)[0])
    };

    // Shadows 40 + midtones 40 must equal what a single 40 over both ranges would give, which is
    // the sum of the two masks times 40. Since the masks sum to `scale` where both are on, the
    // combined result is predictable from either single application.
    let both = apply(40.0, 40.0);
    let shadows_only = apply(40.0, 0.0);
    let midtones_only = apply(0.0, 40.0);
    let base = 70;

    let combined_shift = both - base;
    let separate_sum = (shadows_only - base) + (midtones_only - base);
    assert!(
        (combined_shift - separate_sum).abs() <= 1,
        "the masks must sum: {combined_shift} against {separate_sum}"
    );
}

/// preserve_luminosity restores the ORIGINAL lightness, keeping the hue the shift produced.
///
/// A different mechanism from the flag of the same name on channel-mixer and mono-mixer, which
/// normalises weights. AUDIT-4's note said to reuse that rule; reading the operation showed it
/// converts the result to HSL, copies the original lightness back, and converts back. So the
/// colour must change while the lightness does not.
#[test]
fn color_balance_preserve_luminosity_restores_lightness_only() {
    let source = Pixel::rgba(120, 120, 120, 255);

    let mut without = row(&[source]);
    without
        .execute(Command::ApplyFilter {
            // ASYMMETRIC on purpose. My first version used red +60 with blue -60, and the test
            // could not discriminate: a symmetric shift leaves (max + min) / 2 invariant by
            // construction, so lightness did not move even without the flag. Red alone raises the
            // maximum and leaves the minimum, so lightness genuinely changes and the flag has
            // something to restore.
            filter: balance_midtones(60.0, 0.0, 0.0),
        })
        .unwrap();
    let plain = pixels(&without);

    let mut with = row(&[source]);
    with.execute(Command::ApplyFilter {
        filter: Filter::ColorBalance {
            red: 60.0,
            green: 0.0,
            blue: 0.0,
            red_shadows: 0.0,
            green_shadows: 0.0,
            blue_shadows: 0.0,
            red_highlights: 0.0,
            green_highlights: 0.0,
            blue_highlights: 0.0,
            preserve_luminosity: true,
        },
    })
    .unwrap();
    let preserved = pixels(&with);

    let lightness = |p: &[u8]| {
        let (r, g, b) = (
            f32::from(p[0]) / 255.0,
            f32::from(p[1]) / 255.0,
            f32::from(p[2]) / 255.0,
        );
        (r.max(g).max(b) + r.min(g).min(b)) / 2.0
    };

    let original = lightness(&[120, 120, 120, 255]);
    assert!(
        (lightness(&preserved) - original).abs() < 0.02,
        "lightness must come back to {original}, got {}",
        lightness(&preserved)
    );
    assert!(
        (lightness(&plain) - original).abs() > 0.01,
        "and without the flag it must have moved, or the test proves nothing"
    );
    // The colour still shifted: red up, blue down.
    assert!(
        preserved[0] > preserved[2],
        "the hue the shift produced must survive: {} vs {}",
        preserved[0],
        preserved[2]
    );
}

/// A command saved before the two extra ranges existed still deserialises, and still means what
/// it meant.
///
/// Cycle 38 established this as a risk class in its own right: a field added to a shipped command
/// variant must default to the behaviour the variant already had. Upstream's own default for
/// preserve-luminosity is TRUE, and ours is deliberately false for exactly this reason — the
/// dialog may offer true as its initial value, the command cannot.
#[test]
fn color_balance_deserialises_without_the_new_fields() {
    let json = r#"{
        "kind": "color_balance",
        "red": 50.0,
        "green": 0.0,
        "blue": 0.0
    }"#;
    let filter: Filter =
        serde_json::from_str(json).expect("an older saved command must still load");

    let Filter::ColorBalance {
        red_shadows,
        red_highlights,
        preserve_luminosity,
        ..
    } = filter
    else {
        panic!("deserialised to the wrong variant");
    };
    assert_eq!(red_shadows, 0.0, "absent ranges must be neutral");
    assert_eq!(red_highlights, 0.0);
    assert!(
        !preserve_luminosity,
        "the default must be false even though upstream's dialog default is true"
    );
}

/// Builds a HueSaturation with only the ALL range set — the shape of a command saved before the
/// six sectors existed.
fn hue_sat_all(hue_degrees: f32, saturation: f32, lightness: f32) -> Filter {
    Filter::HueSaturation {
        hue_degrees,
        saturation,
        lightness,
        hue_sectors: [0.0; 6],
        saturation_sectors: [0.0; 6],
        lightness_sectors: [0.0; 6],
        overlap: 0.0,
    }
}

/// A sector adjustment reaches only pixels whose hue falls in that sector.
///
/// This is what the seven-range structure buys and what an ALL-only filter cannot express: a
/// saturation boost on the red sector must leave a green pixel alone.
#[test]
fn hue_saturation_sector_reaches_only_its_own_hues() {
    // Red is sector 0, green is sector 2.
    let red = Pixel::rgba(200, 60, 60, 255);
    let green = Pixel::rgba(60, 200, 60, 255);

    let mut editor = row(&[red, green]);
    let before = pixels(&editor);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::HueSaturation {
                hue_degrees: 0.0,
                saturation: 0.0,
                lightness: 0.0,
                hue_sectors: [0.0; 6],
                // Red sector only.
                saturation_sectors: [-100.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                lightness_sectors: [0.0; 6],
                overlap: 0.0,
            },
        })
        .unwrap();
    let after = pixels(&editor);

    // The red pixel's saturation is scaled to zero, so it becomes grey.
    assert_eq!(
        (after[0], after[1], after[2]),
        (after[0], after[0], after[0]),
        "the red pixel must be fully desaturated"
    );
    // The green pixel is untouched.
    assert_eq!(
        &after[4..8],
        &before[4..8],
        "a red-sector adjustment must not reach a green pixel"
    );
}

/// The master hue shift is AVERAGED with the sector's, not added to it.
///
/// Upstream's `map_hue` is `value += (hue[ALL] + hue[range]) / 2`, so a 120-degree master shift
/// rotates 60 with the sector at zero. The divisor exists so setting both does not double-count.
///
/// The value the WRONG behaviour would give is named here on purpose: a plain addition would send
/// pure red to pure green (hue 120), where the average sends it to yellow (hue 60).
#[test]
fn hue_saturation_master_hue_is_averaged_with_the_sector() {
    let mut editor = row(&[Pixel::rgba(255, 0, 0, 255)]);
    editor
        .execute(Command::ApplyFilter {
            filter: hue_sat_all(120.0, 0.0, 0.0),
        })
        .unwrap();
    let out = pixels(&editor);
    assert_eq!(
        (out[0], out[1], out[2]),
        (255, 255, 0),
        "the average gives yellow; a plain addition would give green (0, 255, 0)"
    );
}

/// Setting the master and the sector to the same value gives the FULL shift.
///
/// The other half of the averaging rule, and the reason it exists: `(x + x) / 2 = x`. This is what
/// makes a sector adjustment stack with the master rather than fight it.
#[test]
fn hue_saturation_master_plus_matching_sector_is_the_full_shift() {
    let mut editor = row(&[Pixel::rgba(255, 0, 0, 255)]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::HueSaturation {
                hue_degrees: 120.0,
                saturation: 0.0,
                lightness: 0.0,
                // Red is sector 0, so give that sector the same 120.
                hue_sectors: [120.0, 0.0, 0.0, 0.0, 0.0, 0.0],
                saturation_sectors: [0.0; 6],
                lightness_sectors: [0.0; 6],
                overlap: 0.0,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    assert_eq!(
        (out[0], out[1], out[2]),
        (0, 255, 0),
        "master 120 plus sector 120 averages to 120, giving green"
    );
}

/// A GREY takes only the ALL range's lightness, and keeps its neutrality.
///
/// Upstream's `map_lightness_achromatic`: a pixel with no saturation has no hue to shift and no
/// saturation to scale, so running the sector maps would read a sector chosen from an undefined
/// hue. The grey must stay grey and only its lightness may move.
#[test]
fn hue_saturation_grey_takes_only_the_all_lightness() {
    let mut editor = row(&[Pixel::rgba(120, 120, 120, 255)]);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::HueSaturation {
                // A conspicuous hue shift and sector saturation that must NOT apply.
                hue_degrees: 120.0,
                saturation: 100.0,
                lightness: 50.0,
                hue_sectors: [180.0; 6],
                saturation_sectors: [100.0; 6],
                // NON-ZERO on purpose. My first version zeroed this, and the test then could not
                // detect the achromatic guard at all: a grey has saturation 0, so
                // `map_saturation` returns 0 either way and the pixel stays neutral regardless,
                // leaving LIGHTNESS as the only observable difference. With the sector at zero,
                // guard and no-guard agree exactly. Reverse-verification is what exposed it --
                // the injected defect passed.
                lightness_sectors: [-90.0; 6],
                overlap: 0.0,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    assert_eq!(
        (out[0], out[1], out[2]),
        (out[0], out[0], out[0]),
        "a grey must stay neutral: no hue to shift, no saturation to scale"
    );
    // ALL lightness is +50, the sectors are -90. With the guard only the +50 applies and the grey
    // LIFTS; without it the sum is -40 and the grey would darken instead. Naming the wrong
    // answer is what makes this assertion worth having.
    assert!(
        out[0] > 120,
        "only the ALL range's +50 applies, lifting from 120; summing the -90 sector would darken it — got {}",
        out[0]
    );
}

/// Overlap blends a boundary pixel between two sectors.
///
/// With overlap at zero the sectors have hard edges and a boundary pixel takes one sector's
/// adjustment outright. With overlap on, it takes a mix — so two conflicting sector settings must
/// give a different answer under each.
#[test]
fn hue_saturation_overlap_blends_neighbouring_sectors() {
    // A yellow-green at hue 71 degrees, i.e. h = 1.19 in sector units.
    //
    // My first attempt used a pure yellow at hue 60, where h is exactly 1.0 -- precisely ON the
    // sector threshold, and upstream's comparisons there are STRICT (`h > threshold - overlap`),
    // so no blending happens and both overlap settings gave the same answer. The boundary itself
    // is the one hue an overlap test must not use. 1.19 sits strictly inside the band.
    let yellow = Pixel::rgba(170, 200, 40, 255);

    let sample = |overlap: f32| {
        let mut editor = row(&[yellow]);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::HueSaturation {
                    hue_degrees: 0.0,
                    saturation: 0.0,
                    lightness: 0.0,
                    hue_sectors: [0.0; 6],
                    // Opposite lightness pushes in the two adjacent sectors, so a blend lands
                    // between them while a hard edge lands on one.
                    lightness_sectors: [80.0, -80.0, 0.0, 0.0, 0.0, 0.0],
                    saturation_sectors: [0.0; 6],
                    overlap,
                },
            })
            .unwrap();
        pixels(&editor)[0]
    };

    assert_ne!(
        sample(0.0),
        sample(1.0),
        "overlap must reach the filter; identical results would mean it is ignored"
    );
}

/// A command saved before the sectors existed still deserialises and still means what it meant.
#[test]
fn hue_saturation_deserialises_without_the_new_fields() {
    let json = r#"{
        "kind": "hue_saturation",
        "hue_degrees": 30.0,
        "saturation": 10.0,
        "lightness": 0.0
    }"#;
    let filter: Filter =
        serde_json::from_str(json).expect("an older saved command must still load");
    let Filter::HueSaturation {
        hue_sectors,
        saturation_sectors,
        lightness_sectors,
        overlap,
        ..
    } = filter
    else {
        panic!("deserialised to the wrong variant");
    };
    assert_eq!(hue_sectors, [0.0; 6], "absent sectors must be neutral");
    assert_eq!(saturation_sectors, [0.0; 6]);
    assert_eq!(lightness_sectors, [0.0; 6]);
    assert_eq!(overlap, 0.0, "and overlap must default to hard edges");
}
