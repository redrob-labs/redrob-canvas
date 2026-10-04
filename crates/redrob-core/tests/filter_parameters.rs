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
                threshold: cut,
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
        Filter::Threshold { threshold, channel } => {
            assert_eq!(threshold, 100);
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

    let broken = LevelsSlot {
        input_black: 200,
        input_white: 100,
        gamma: 1.0,
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
