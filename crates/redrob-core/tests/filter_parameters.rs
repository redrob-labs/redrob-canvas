//! K.16, parameter gaps on filters we already ship.

use redrob_core::{
    Command, CurvePoint, Document, Editor, Filter, HistogramChannel, LevelsSlot, Pixel,
};

/// A saturated warm colour, chosen so that every channel reading is a different number:
/// max 200, min 30, red 200, green 60, blue 30, alpha 255, GIMP luminance 89.
const WARM: Pixel = Pixel {
    r: 200,
    g: 60,
    b: 30,
    a: 255,
};

fn threshold(colour: Pixel, cut: u8, channel: HistogramChannel) -> u8 {
    let mut editor = Editor::new(Document::new(4, 4).expect("document")).expect("editor");
    editor
        .execute(Command::Fill { color: colour })
        .expect("fill");
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Threshold {
                low: cut,
                high: 255,
                channel,
            },
        })
        .expect("filter");
    editor.document().layers()[0].pixels()[0]
}

/// `Value` is the MAXIMUM of red, green and blue and `Rgb` is the MINIMUM — so on one saturated
/// colour they give opposite verdicts at the same cut point.
///
/// Asserted as one claim about the difference, because the pair is the whole point: nobody reading
/// the property names would guess that `Rgb` means the minimum channel. This is the trap K.16's own
/// preamble warns about, where AUDIT-4 twice named a gap correctly and described it wrongly from the
/// name alone.
#[test]
fn threshold_value_and_rgb_are_the_max_and_min_so_they_disagree() {
    assert_eq!(
        threshold(WARM, 100, HistogramChannel::Value),
        255,
        "max is 200, which clears 100"
    );
    assert_eq!(
        threshold(WARM, 100, HistogramChannel::Rgb),
        0,
        "min is 30, which does not"
    );
}

/// The three colour channels and alpha read their own component, nothing more.
#[test]
fn threshold_reads_the_named_component_for_the_single_channels() {
    for (channel, component) in [
        (HistogramChannel::Red, 200u8),
        (HistogramChannel::Green, 60),
        (HistogramChannel::Blue, 30),
        (HistogramChannel::Alpha, 255),
    ] {
        // One below the component's own value clears it; one above does not.
        assert_eq!(
            threshold(WARM, component.saturating_sub(1), channel),
            255,
            "{channel:?} should clear a cut just under {component}"
        );
        if component < 255 {
            assert_eq!(
                threshold(WARM, component + 1, channel),
                0,
                "{channel:?} should fail a cut just over {component}"
            );
        }
    }
}

/// `Luminance` uses GIMP's own weights, not Rec. 709 — and the two differ enough to be separated by
/// a single cut point.
///
/// For (200, 60, 30):
/// - GIMP: `200*0.22248840 + 60*0.71690369 + 30*0.06060791` = 89.33, so 89
/// - Rec. 709: `200*0.2126 + 60*0.7152 + 30*0.0722` = 87.65, so 88
///
/// A cut at 89 therefore clears under GIMP's weights and fails under Rec. 709. Our `Threshold` used
/// Rec. 709 before this item, which was upstream's behaviour for neither the default channel nor
/// this one.
#[test]
fn threshold_luminance_uses_gimps_weights_not_rec_709() {
    assert_eq!(
        threshold(WARM, 89, HistogramChannel::Luminance),
        255,
        "GIMP's weights give 89, which clears a cut of 89"
    );
    assert_eq!(
        threshold(WARM, 90, HistogramChannel::Luminance),
        0,
        "and fail a cut of 90, so the reading is 89 exactly"
    );
}

/// The default is upstream's `GIMP_HISTOGRAM_VALUE`, declared
/// `g_param_spec_enum ("channel", ..., GIMP_HISTOGRAM_VALUE)`.
///
/// This DOES change what an existing saved `Threshold` does, and that is deliberate — see the field's
/// own documentation. The project rule that a new field defaults to the variant's previous behaviour
/// guards against accidental change; here the change is the correction.
#[test]
fn threshold_channel_defaults_to_value() {
    let filter: Filter =
        serde_json::from_str(r#"{"kind":"threshold","threshold":100}"#).expect("deserialise");
    match filter {
        Filter::Threshold {
            low,
            high: _,
            channel,
        } => {
            assert_eq!(low, 100);
            assert_eq!(channel, HistogramChannel::Value, "the enum's first member");
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

// ---------------------------------------------------------------------------------------------
// K.16, per-channel curves. Five slots applied in one pass, not a channel selector.
// ---------------------------------------------------------------------------------------------

/// A straight two-point curve from `from` to `to`.
fn ramp(from: f32, to: f32) -> Vec<CurvePoint> {
    vec![CurvePoint::smooth(0.0, from), CurvePoint::smooth(1.0, to)]
}

fn identity_curve() -> Vec<CurvePoint> {
    ramp(0.0, 1.0)
}

/// R 100, G 150, B 200, alpha 128 — every channel a different number, and a partial alpha so the
/// alpha claims below are observable.
fn curved(
    points: Vec<CurvePoint>,
    red: Option<Vec<CurvePoint>>,
    green: Option<Vec<CurvePoint>>,
    blue: Option<Vec<CurvePoint>>,
    alpha: Option<Vec<CurvePoint>>,
) -> (u8, u8, u8, u8) {
    let mut editor = Editor::new(Document::new(4, 4).expect("document")).expect("editor");
    editor
        .execute(Command::Fill {
            color: Pixel {
                r: 100,
                g: 150,
                b: 200,
                a: 128,
            },
        })
        .expect("fill");
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Curves {
                points,
                red,
                green,
                blue,
                alpha,
            },
        })
        .expect("filter");
    let pixels = editor.document().layers()[0].pixels();
    (pixels[0], pixels[1], pixels[2], pixels[3])
}

/// The composition order, read from `gimpcurve-map.c`'s default case: the per-channel curve is
/// applied FIRST and the colours curve on top of its result.
///
/// A constant red curve at 0.5 gives 128, which the halving colours curve then takes to 64. Were the
/// order reversed, 100 would be halved to 50 and then replaced by the constant 128. **64 against 128**
/// — so one pixel separates the two orders, and nothing else in this test file can.
#[test]
fn curves_apply_the_per_channel_curve_before_the_colours_curve() {
    let (red, green, blue, alpha) = curved(ramp(0.0, 0.5), Some(ramp(0.5, 0.5)), None, None, None);

    assert_eq!(
        red, 64,
        "per-channel inner, colours outer; reversed gives 128"
    );
    assert_eq!(
        (green, blue),
        (75, 100),
        "the untouched channels are only halved"
    );
    assert_eq!(alpha, 128, "and alpha is not in this path at all");
}

/// `/* don't apply the colors curve to the alpha channel */` — upstream states it twice, in the
/// `CURVE_COLORS` fast path and again in the general case.
///
/// So a colours curve that maps everything to zero still leaves alpha alone, and only alpha's own
/// curve can change it. The zeroing curve is the strongest form of the claim: if the colours curve
/// reached alpha at all, alpha would be 0.
#[test]
fn curves_colours_curve_never_touches_alpha_but_its_own_curve_does() {
    assert_eq!(
        curved(ramp(0.0, 0.0), None, None, None, None),
        (0, 0, 0, 128),
        "a colours curve that zeroes everything leaves alpha at its input"
    );

    assert_eq!(
        curved(identity_curve(), None, None, None, Some(ramp(0.0, 0.5))),
        (100, 150, 200, 64),
        "and alpha's own curve applies, to alpha only"
    );
}

/// Each per-channel slot reaches only its own channel.
#[test]
fn curves_per_channel_slots_are_independent() {
    assert_eq!(
        curved(identity_curve(), None, Some(ramp(0.5, 0.5)), None, None),
        (100, 128, 200, 128),
        "a green curve moves green and nothing else"
    );
}

/// Rule 9, verified by measurement rather than assumed: a `Curves` saved before this item had only
/// `points`, and it must still mean exactly what it meant.
///
/// `points` keeps its role as the colours curve — which is what this variant always did, applying one
/// table to R, G and B and leaving alpha — so the four new slots deserialise to `None`, the identity.
#[test]
fn curves_legacy_json_is_unchanged() {
    let filter: Filter =
        serde_json::from_str(r#"{"kind":"curves","points":[{"x":0.0,"y":0.0},{"x":1.0,"y":0.5}]}"#)
            .expect("deserialise");

    match &filter {
        Filter::Curves {
            red,
            green,
            blue,
            alpha,
            ..
        } => assert!(
            red.is_none() && green.is_none() && blue.is_none() && alpha.is_none(),
            "every new slot defaults to the identity"
        ),
        other => panic!("wrong variant: {other:?}"),
    }

    assert_eq!(
        curved(ramp(0.0, 0.5), None, None, None, None),
        (50, 75, 100, 128),
        "the halving colours curve behaves exactly as it did before the widening"
    );
}

// ---------------------------------------------------------------------------------------------
// K.16, per-channel levels. The same five-slot shape as curves, from the same kind of loop.
// ---------------------------------------------------------------------------------------------

/// A slot mapping the full input range onto `output_black..output_white` with gamma 1.
fn slot(output_black: u8, output_white: u8) -> LevelsSlot {
    LevelsSlot {
        input_black: 0,
        input_white: 255,
        gamma: 1.0,
        output_black,
        output_white,
    }
}

/// A slot that maps every input to one value.
fn constant(value: u8) -> LevelsSlot {
    slot(value, value)
}

fn levelled(
    overall: LevelsSlot,
    red: Option<LevelsSlot>,
    green: Option<LevelsSlot>,
    blue: Option<LevelsSlot>,
    alpha: Option<LevelsSlot>,
) -> (u8, u8, u8, u8) {
    let mut editor = Editor::new(Document::new(4, 4).expect("document")).expect("editor");
    editor
        .execute(Command::Fill {
            color: Pixel {
                r: 100,
                g: 150,
                b: 200,
                a: 128,
            },
        })
        .expect("fill");
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Levels {
                input_black: overall.input_black,
                input_white: overall.input_white,
                gamma: overall.gamma,
                output_black: overall.output_black,
                output_white: overall.output_white,
                red,
                green,
                blue,
                alpha,
                clamp_input: true,
                clamp_output: true,
            },
        })
        .expect("filter");
    let pixels = editor.document().layers()[0].pixels();
    (pixels[0], pixels[1], pixels[2], pixels[3])
}

/// The composition order, read from `gimpoperationlevels.c`: the per-channel slot is applied to
/// `src[channel]` first, then the overall slot (index 0) on top of its result.
///
/// A constant red slot at 128 under a halving overall slot gives **64**. Reversed, 100 would halve to
/// 50 and then be replaced by the constant **128**. The same discriminator as curves, which is the
/// point — both filters share one composition rule, so both are pinned by the same shape.
#[test]
fn levels_apply_the_per_channel_slot_before_the_overall_one() {
    let (red, green, blue, alpha) = levelled(slot(0, 128), Some(constant(128)), None, None, None);

    assert_eq!(
        red, 64,
        "per-channel inner, overall outer; reversed gives 128"
    );
    assert_eq!(
        (green, blue),
        (75, 100),
        "the untouched channels are only halved"
    );
    assert_eq!(alpha, 128, "and alpha is not in this path");
}

/// `/* don't apply the overall curve to the alpha channel */`, guarded upstream by
/// `if (channel != ALPHA)`.
///
/// An overall slot that maps everything to zero still leaves alpha alone — the strongest form of the
/// claim, since if the overall slot reached alpha, alpha would be 0.
#[test]
fn levels_overall_slot_never_touches_alpha_but_its_own_slot_does() {
    assert_eq!(
        levelled(constant(0), None, None, None, None),
        (0, 0, 0, 128),
        "an overall slot that zeroes everything leaves alpha at its input"
    );

    assert_eq!(
        levelled(slot(0, 255), None, None, None, Some(slot(0, 128))),
        (100, 150, 200, 64),
        "alpha's own slot applies, to alpha only"
    );
}

/// A per-channel slot is validated by the same rule as the overall one, so it cannot express
/// something the overall slot would be refused for.
#[test]
fn levels_rejects_an_invalid_per_channel_slot() {
    let mut editor = Editor::new(Document::new(4, 4).expect("document")).expect("editor");
    editor
        .execute(Command::Fill {
            color: Pixel {
                r: 100,
                g: 100,
                b: 100,
                a: 255,
            },
        })
        .expect("fill");

    // Gamma is the only thing still refused: an empty input range shifts (cycle 112) and an
    // inverted one inverts (cycle 113), both because upstream does. Upstream guards gamma with
    // `g_return_val_if_fail (config->gamma[channel] != 0.0)`.
    let broken = LevelsSlot {
        input_black: 0,
        input_white: 255,
        gamma: 0.0,
        output_black: 0,
        output_white: 255,
    };

    assert!(
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Levels {
                    input_black: 0,
                    input_white: 255,
                    gamma: 1.0,
                    output_black: 0,
                    output_white: 255,
                    red: Some(broken),
                    green: None,
                    blue: None,
                    alpha: None,
                    clamp_input: true,
                    clamp_output: true,
                },
            })
            .is_err(),
        "an inverted input range must be refused in a per-channel slot too"
    );
}

/// Rule 9 by measurement: a `Levels` saved before this item had only the five scalars, and they keep
/// their meaning as the overall slot — which is exactly what the variant already did, mapping R, G
/// and B and leaving alpha.
#[test]
fn levels_legacy_json_is_unchanged() {
    let filter: Filter = serde_json::from_str(
        r#"{"kind":"levels","input_black":0,"input_white":255,"gamma":1.0,"output_black":0,"output_white":128}"#,
    )
    .expect("deserialise");

    match &filter {
        Filter::Levels {
            red,
            green,
            blue,
            alpha,
            ..
        } => assert!(
            red.is_none() && green.is_none() && blue.is_none() && alpha.is_none(),
            "every new slot defaults to the identity"
        ),
        other => panic!("wrong variant: {other:?}"),
    }

    assert_eq!(
        levelled(slot(0, 128), None, None, None, None),
        (50, 75, 100, 128),
        "the halving overall slot behaves exactly as it did before the widening"
    );
}

/// `if (high_input != low_input) value = (value - low_input) / (high_input - low_input);
/// else value = (value - low_input);`
///
/// So an empty input range is neither a division by zero nor an error — it degenerates to a plain
/// **shift**, un-normalised. Upstream's values are already in 0..1, so the faithful translation of
/// that difference divides by 255 rather than by the zero range.
///
/// With black 100 and a full output range that makes the result `value - 100`, clamped below at 0.
/// Predicted before running: 150 gives 50, 255 gives 155, 50 gives 0.
///
/// Our validation refused this outright before K.16. A strictly inverted range is still refused.
#[test]
fn levels_an_empty_input_range_is_a_shift_not_an_error() {
    let shift = LevelsSlot {
        input_black: 100,
        input_white: 100,
        gamma: 1.0,
        output_black: 0,
        output_white: 255,
    };

    for (input, expected) in [(100u8, 0u8), (150, 50), (255, 155), (50, 0)] {
        let mut editor = Editor::new(Document::new(4, 4).expect("document")).expect("editor");
        editor
            .execute(Command::Fill {
                color: Pixel {
                    r: input,
                    g: input,
                    b: input,
                    a: 255,
                },
            })
            .expect("fill");
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Levels {
                    input_black: shift.input_black,
                    input_white: shift.input_white,
                    gamma: shift.gamma,
                    output_black: shift.output_black,
                    output_white: shift.output_white,
                    red: None,
                    green: None,
                    blue: None,
                    alpha: None,
                    clamp_input: true,
                    clamp_output: true,
                },
            })
            .expect("an empty input range is legal");
        assert_eq!(
            editor.document().layers()[0].pixels()[0],
            expected,
            "input {input} shifted by 100"
        );
    }
}

/// `clamp_input` is observable only through a NARROWED OUTPUT RANGE, because the byte write clamps
/// to 0..255 regardless. With the full output range both settings agree, which is why this test
/// narrows it.
///
/// Input window 100..200 with gamma 2 and output 0..128, on a pixel of 250:
/// - normalised is `(250 - 100) / 100` = 1.5
/// - clamped: `1.0 ^ 0.5` = 1.0, so `0 + 1.0 * 128` = **128**
/// - unclamped: `1.5 ^ 0.5` = 1.2247, so `0 + 1.2247 * 128` = 156.8 → **157**
///
/// Both numbers were written down before running.
#[test]
fn levels_clamp_input_is_observable_through_a_narrowed_output_range() {
    let measure = |clamp_input: bool| {
        let mut editor = Editor::new(Document::new(4, 4).expect("document")).expect("editor");
        editor
            .execute(Command::Fill {
                color: Pixel {
                    r: 250,
                    g: 250,
                    b: 250,
                    a: 255,
                },
            })
            .expect("fill");
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Levels {
                    input_black: 100,
                    input_white: 200,
                    gamma: 2.0,
                    output_black: 0,
                    output_white: 128,
                    red: None,
                    green: None,
                    blue: None,
                    alpha: None,
                    clamp_input,
                    clamp_output: true,
                },
            })
            .expect("filter");
        editor.document().layers()[0].pixels()[0]
    };

    assert_eq!(measure(true), 128, "clamped at 1.0 before the output stage");
    assert_eq!(
        measure(false),
        157,
        "1.5 ^ 0.5 carried into the output stage"
    );
}

/// Below the input window the unclamped path goes NEGATIVE, and gamma is skipped for a non-positive
/// value (`if (inv_gamma != 1.0 && value > 0)`), so the negative reaches the output stage intact.
///
/// Input window 100..200, output 50..200, gamma 1, on a pixel of 50:
/// - normalised is `(50 - 100) / 100` = −0.5
/// - clamped: 0.0, so `50 + 0` = **50**
/// - unclamped: `50 + (−0.5 × 150)` = −25, which the byte write floors at **0**
#[test]
fn levels_unclamped_input_can_go_below_the_output_floor() {
    let measure = |clamp_input: bool| {
        let mut editor = Editor::new(Document::new(4, 4).expect("document")).expect("editor");
        editor
            .execute(Command::Fill {
                color: Pixel {
                    r: 50,
                    g: 50,
                    b: 50,
                    a: 255,
                },
            })
            .expect("fill");
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Levels {
                    input_black: 100,
                    input_white: 200,
                    gamma: 1.0,
                    output_black: 50,
                    output_white: 200,
                    red: None,
                    green: None,
                    blue: None,
                    alpha: None,
                    clamp_input,
                    clamp_output: true,
                },
            })
            .expect("filter");
        editor.document().layers()[0].pixels()[0]
    };

    assert_eq!(measure(true), 50, "clamped to the output floor");
    assert_eq!(
        measure(false),
        0,
        "the negative survives and the byte write floors it"
    );
}

/// Both flags default to `true`, preserving what this variant always did — while upstream declares
/// both as `FALSE`.
///
/// The divergence is in the DEFAULT only, and it is deliberate: clamping IS upstream's behaviour
/// with these flags set, so nothing here is wrong, and the parity requirement is that both
/// behaviours be expressible. That differs from `Threshold`'s `channel`, whose old behaviour matched
/// no upstream configuration at all and so could not be preserved.
#[test]
fn levels_clamp_flags_default_to_the_existing_behaviour() {
    let filter: Filter = serde_json::from_str(
        r#"{"kind":"levels","input_black":0,"input_white":255,"gamma":1.0,"output_black":0,"output_white":255}"#,
    )
    .expect("deserialise");

    match filter {
        Filter::Levels {
            clamp_input,
            clamp_output,
            ..
        } => assert!(
            clamp_input && clamp_output,
            "both default to true so a saved Levels keeps its meaning"
        ),
        other => panic!("wrong variant: {other:?}"),
    }
}

/// Upstream declares `low-input`, `high-input`, `low-output` and `high-output` each as an
/// independent `0.0, 1.0` with **no ordering guard** — the only comparison anywhere in the operation
/// is `!=`, never `<` or `>`. So an inverted range is expressible and inverts the mapping.
///
/// With black 200 and white 100 the normaliser becomes `(value - 200) / (100 - 200)`, i.e.
/// `(200 - value) / 100`. Predicted before running: 200 gives 0, 150 gives 128 (0.5 × 255 = 127.5,
/// rounded up), 100 gives 255. Values outside the window go out of 0..1 and the input clamp catches
/// them.
///
/// Our validation refused this outright until cycle 113.
#[test]
fn levels_an_inverted_input_range_inverts_the_mapping() {
    let inverted = LevelsSlot {
        input_black: 200,
        input_white: 100,
        gamma: 1.0,
        output_black: 0,
        output_white: 255,
    };

    for (input, expected) in [(200u8, 0u8), (150, 128), (100, 255), (250, 0), (50, 255)] {
        let mut editor = Editor::new(Document::new(4, 4).expect("document")).expect("editor");
        editor
            .execute(Command::Fill {
                color: Pixel {
                    r: input,
                    g: input,
                    b: input,
                    a: 255,
                },
            })
            .expect("fill");
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Levels {
                    input_black: inverted.input_black,
                    input_white: inverted.input_white,
                    gamma: inverted.gamma,
                    output_black: inverted.output_black,
                    output_white: inverted.output_white,
                    red: None,
                    green: None,
                    blue: None,
                    alpha: None,
                    clamp_input: true,
                    clamp_output: true,
                },
            })
            .expect("an inverted input range is legal");
        assert_eq!(
            editor.document().layers()[0].pixels()[0],
            expected,
            "input {input} through an inverted window"
        );
    }
}

/// An inverted OUTPUT range inverts too, and here upstream's intent is explicit rather than merely
/// unguarded: it wrote a dedicated `else` branch for `high_output < low_output`.
///
/// Cycle 111 established that branch is algebraically identical to the main one
/// (`v·(high−low)+low` equals `low − v·(low−high)`), so one expression serves both — which is why
/// allowing this needed no new arithmetic, only the validation relaxed and the range computed in
/// f32 instead of as a u8 subtraction that would underflow.
///
/// Predicted: output 255..0 is a straight inversion — 0 gives 255, 255 gives 0, 128 gives 127.
#[test]
fn levels_an_inverted_output_range_inverts_too() {
    for (input, expected) in [(0u8, 255u8), (255, 0), (128, 127)] {
        let mut editor = Editor::new(Document::new(4, 4).expect("document")).expect("editor");
        editor
            .execute(Command::Fill {
                color: Pixel {
                    r: input,
                    g: input,
                    b: input,
                    a: 255,
                },
            })
            .expect("fill");
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Levels {
                    input_black: 0,
                    input_white: 255,
                    gamma: 1.0,
                    output_black: 255,
                    output_white: 0,
                    red: None,
                    green: None,
                    blue: None,
                    alpha: None,
                    clamp_input: true,
                    clamp_output: true,
                },
            })
            .expect("an inverted output range is legal");
        assert_eq!(
            editor.document().layers()[0].pixels()[0],
            expected,
            "input {input} through an inverted output range"
        );
    }
}

/// `value = (value >= threshold->low && value <= threshold->high) ? 1.0 : 0.0;`
///
/// A band, not a cut — and the capability a single cut point cannot express at all: keeping the
/// midtones while blacking out shadows AND highlights together.
///
/// `low` 0.3 and `high` 0.6 of upstream's 0..1 range are 77 and 153 in bytes. Predicted before
/// running: 50 black, 100 white, 200 black. A single cut can produce at most one of those two black
/// regions.
#[test]
fn threshold_band_blacks_both_shadows_and_highlights() {
    let band = |value: u8| {
        let mut editor = Editor::new(Document::new(4, 4).expect("document")).expect("editor");
        editor
            .execute(Command::Fill {
                color: Pixel {
                    r: value,
                    g: value,
                    b: value,
                    a: 255,
                },
            })
            .expect("fill");
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Threshold {
                    low: 77,
                    high: 153,
                    channel: HistogramChannel::Value,
                },
            })
            .expect("filter");
        editor.document().layers()[0].pixels()[0]
    };

    assert_eq!(band(50), 0, "shadows are blacked");
    assert_eq!(band(100), 255, "midtones are kept");
    assert_eq!(band(200), 0, "and highlights are blacked too");
}

/// Both bounds are inclusive — `>=` and `<=`, not a half-open interval. The two boundary bytes and
/// their immediate neighbours pin it in one place.
#[test]
fn threshold_band_bounds_are_both_inclusive() {
    let band = |value: u8| {
        let mut editor = Editor::new(Document::new(4, 4).expect("document")).expect("editor");
        editor
            .execute(Command::Fill {
                color: Pixel {
                    r: value,
                    g: value,
                    b: value,
                    a: 255,
                },
            })
            .expect("fill");
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Threshold {
                    low: 77,
                    high: 153,
                    channel: HistogramChannel::Value,
                },
            })
            .expect("filter");
        editor.document().layers()[0].pixels()[0]
    };

    assert_eq!(band(76), 0, "one below the low bound is out");
    assert_eq!(band(77), 255, "the low bound itself is in");
    assert_eq!(band(153), 255, "the high bound itself is in");
    assert_eq!(band(154), 0, "one above it is out");
}

/// The rename is backward compatible, which is the whole reason for the crate's first
/// `serde(alias)`: a document saved with `threshold` still loads, as `low`, and `high` defaults to
/// 255 — making the band exactly the single cut it used to be.
///
/// No behavioural change at all, so unlike `channel` two cycles ago there is no departure from the
/// serde-default rule here.
#[test]
fn threshold_legacy_field_name_still_loads_as_the_low_bound() {
    let filter: Filter =
        serde_json::from_str(r#"{"kind":"threshold","threshold":100}"#).expect("deserialise");

    match filter {
        Filter::Threshold { low, high, .. } => {
            assert_eq!(low, 100, "the old `threshold` is the band's low bound");
            assert_eq!(high, 255, "and the band is open at the top");
        }
        other => panic!("wrong variant: {other:?}"),
    }

    // And upstream's own name works too.
    let modern: Filter =
        serde_json::from_str(r#"{"kind":"threshold","low":100}"#).expect("deserialise");
    assert_eq!(
        modern,
        Filter::Threshold {
            low: 100,
            high: 255,
            channel: HistogramChannel::Value,
        }
    );
}
