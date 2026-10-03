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
