//! Mixer brush (wet paint), batch 4 item L9.

use redrob_core::{BrushPoint, BrushSettings, Command, Document, Editor, MixerBrush, Pixel};

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
            }),
            ..BrushSettings::default()
        },
    });
    assert!(result.is_err());
}
