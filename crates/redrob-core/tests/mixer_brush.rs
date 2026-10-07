//! Mixer brush (wet paint), batch 4 item L9.

use redrob_core::{BrushPoint, BrushSettings, Command, Document, Editor, MixerBrush, MixerWell, Pixel};

/// A 32x8 layer: left half red, right half empty.
fn canvas() -> Editor {
    let mut editor = Editor::new(Document::new(32, 8).unwrap()).unwrap();
    editor
        .execute(Command::SelectRectangle {
            rect: redrob_core::Rect {
                x: 0,
                y: 0,
                width: 16,
                height: 8,
            },
            mode: Default::default(),
        })
        .unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(255, 0, 0, 255),
        })
        .unwrap();
    editor.execute(Command::ClearSelection).unwrap();
    editor
}

fn stroke(editor: &mut Editor, mixer: Option<MixerBrush>) {
    let points = (0..=28)
        .map(|i| BrushPoint::new(2.0 + i as f32, 4.0, 1.0))
        .collect();
    editor
        .execute(Command::BrushStroke {
            points,
            color: Pixel::rgba(0, 0, 255, 255),
            size: 4.0,
            opacity: 1.0,
            tip: None,
            pipe: Vec::new(),
            settings: BrushSettings {
                mixer,
                ..BrushSettings::default()
            },
        })
        .unwrap();
}

fn px(editor: &Editor, x: usize, y: usize) -> [u8; 4] {
    let doc = editor.document();
    let p = doc.layer(doc.active_layer_id()).unwrap().pixels();
    let o = (y * 32 + x) * 4;
    [p[o], p[o + 1], p[o + 2], p[o + 3]]
}

#[test]
fn a_dry_fully_loaded_mixer_paints_the_brush_colour() {
    let mut editor = canvas();
    stroke(
        &mut editor,
        Some(MixerBrush {
            wet: 0.0,
            load: 1.0,
            mix: 0.0,
            sample_all_layers: false,
            well: None,
        }),
    );
    assert_eq!(px(&editor, 8, 4), [0, 0, 255, 255]);
}

#[test]
fn a_wet_mixer_carries_red_into_the_empty_half() {
    let mut editor = canvas();
    stroke(
        &mut editor,
        Some(MixerBrush {
            wet: 0.6,
            load: 0.9,
            mix: 0.6,
            sample_all_layers: false,
            well: None,
        }),
    );
    let over_red = px(&editor, 12, 4);
    assert!(
        over_red[0] > 60 && over_red[2] > 20,
        "blue mixed with red: {over_red:?}"
    );
    let past = px(&editor, 20, 4);
    assert!(past[0] > 0, "red dragged past the edge: {past:?}");
}

#[test]
fn out_of_range_mixer_settings_are_refused() {
    let mut editor = canvas();
    let result = editor.execute(Command::BrushStroke {
        points: vec![BrushPoint::new(4.0, 4.0, 1.0)],
        color: Pixel::rgba(0, 0, 255, 255),
        size: 4.0,
        opacity: 1.0,
        tip: None,
        pipe: Vec::new(),
        settings: BrushSettings {
            mixer: Some(MixerBrush {
                wet: 1.5,
                load: 1.0,
                mix: 0.0,
                sample_all_layers: false,
                well: None,
            }),
            ..BrushSettings::default()
        },
    });
    assert!(result.is_err());
}

// ---- U2: Photoshop parity ----

fn mixer(wet: f32, load: f32, mix: f32) -> MixerBrush {
    MixerBrush {
        wet,
        load,
        mix,
        sample_all_layers: false,
        well: None,
    }
}

#[test]
fn a_dry_brush_with_little_load_fades_along_the_stroke() {
    // Photoshop: with Wet 0 and a low Load the stroke runs out of paint and fades.
    let mut editor = canvas();
    stroke(&mut editor, Some(mixer(0.0, 0.05, 0.0)));
    let start = px(&editor, 3, 4);
    let end = px(&editor, 28, 4);
    assert_eq!(start[2], 255, "the stroke starts with full paint: {start:?}");
    assert!(end[3] < 128, "the dry end deposits little: {end:?}");
}

#[test]
fn a_full_load_never_runs_dry() {
    let mut editor = canvas();
    stroke(&mut editor, Some(mixer(0.0, 1.0, 0.0)));
    assert_eq!(px(&editor, 28, 4), [0, 0, 255, 255]);
}

#[test]
fn the_pickup_reads_the_whole_tip_not_one_point() {
    // A one-pixel red line under the dab centre's row is missed by a centre-only sample only when
    // the centre is off it; with the tip averaged, a clean brush (no paint, Mix 100%) over it
    // picks up some red even though the centre is two pixels away.
    let mut editor = Editor::new(Document::new(32, 8).unwrap()).unwrap();
    editor
        .execute(Command::SelectRectangle {
            rect: redrob_core::Rect {
                x: 0,
                y: 2,
                width: 32,
                height: 1,
            },
            mode: Default::default(),
        })
        .unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(255, 0, 0, 255),
        })
        .unwrap();
    editor.execute(Command::ClearSelection).unwrap();
    let points = (0..=20)
        .map(|i| BrushPoint::new(4.0 + i as f32, 4.5, 1.0))
        .collect();
    editor
        .execute(Command::BrushStroke {
            points,
            color: Pixel::rgba(0, 0, 255, 255),
            size: 8.0,
            opacity: 1.0,
            tip: None,
            pipe: Vec::new(),
            settings: BrushSettings {
                mixer: Some(MixerBrush {
                    well: Some(MixerWell {
                        color: [0.0; 4],
                        level: 0.0,
                    }),
                    ..mixer(1.0, 1.0, 1.0)
                }),
                ..BrushSettings::default()
            },
        })
        .unwrap();
    let mid = px(&editor, 14, 5);
    assert!(mid[0] > 0 && mid[3] > 0, "red picked up from the tip's edge: {mid:?}");
}

#[test]
fn sample_all_layers_picks_up_the_layer_below() {
    // Photoshop's "Sample All Layers": an empty layer over a red one picks up red.
    let run = |all: bool| {
        let mut editor = canvas();
        editor
            .execute(Command::AddLayer {
                id: redrob_core::LayerId::new(),
                name: "Wet paint".into(),
                index: 1,
            })
            .unwrap();
        let points = (0..=12)
            .map(|i| BrushPoint::new(2.0 + i as f32, 4.0, 1.0))
            .collect();
        editor
            .execute(Command::BrushStroke {
                points,
                color: Pixel::rgba(0, 0, 255, 255),
                size: 4.0,
                opacity: 1.0,
                tip: None,
                pipe: Vec::new(),
                settings: BrushSettings {
                    mixer: Some(MixerBrush {
                        sample_all_layers: all,
                        ..mixer(1.0, 1.0, 1.0)
                    }),
                    ..BrushSettings::default()
                },
            })
            .unwrap();
        px(&editor, 12, 4)
    };
    let own_layer = run(false);
    let all_layers = run(true);
    assert!(all_layers[0] > 100, "red from the layer below: {all_layers:?}");
    assert!(all_layers[0] > own_layer[0], "{own_layer:?} vs {all_layers:?}");
}

#[test]
fn a_stroke_reports_the_paint_left_and_starts_from_a_given_well() {
    let mut editor = canvas();
    stroke(&mut editor, Some(mixer(0.6, 0.5, 0.6)));
    let well = editor
        .document()
        .last_mixer_well()
        .expect("a mixer stroke reports its end state");
    assert!(well.level < 1.0, "paint was used: {well:?}");
    assert!(well.color[0] > 0.0, "the brush is dirty with red: {well:?}");

    // A green, full well paints green where a fresh brush would paint blue.
    let mut editor = canvas();
    stroke(
        &mut editor,
        Some(MixerBrush {
            well: Some(MixerWell {
                color: [0.0, 255.0, 0.0, 255.0],
                level: 1.0,
            }),
            ..mixer(0.0, 1.0, 0.0)
        }),
    );
    assert_eq!(px(&editor, 20, 4), [0, 255, 0, 255]);
}

#[test]
fn old_mixer_json_still_parses_and_new_fields_stay_out_when_default() {
    let old: MixerBrush = serde_json::from_str(r#"{"wet":0.5,"load":0.9,"mix":0.5}"#).unwrap();
    assert_eq!(old, mixer(0.5, 0.9, 0.5));
    assert_eq!(
        serde_json::to_string(&old).unwrap(),
        r#"{"wet":0.5,"load":0.9,"mix":0.5}"#
    );
    let bad = MixerBrush {
        well: Some(MixerWell {
            color: [0.0, 0.0, 0.0, 300.0],
            level: 1.0,
        }),
        ..old
    };
    assert!(!bad.is_valid());
}
