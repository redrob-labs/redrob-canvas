//! K.16, parameter gaps on filters we already ship.

use redrob_core::{Command, Document, Editor, Filter, HistogramChannel, Pixel};

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
