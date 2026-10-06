//! Brush angle and pen-tilt direction, batch 4 item L3.

use redrob_core::{BrushPoint, BrushSettings, Command, DabMask, DabShape, Document, Editor, Pixel};

fn flat() -> DabShape {
    DabShape {
        ratio: 0.25,
        ..DabShape::round(1.0)
    }
}

#[test]
fn a_rotated_mask_is_the_unrotated_one_turned() {
    let plain = DabMask::new(flat(), 40.0);
    let turned = DabMask::new(flat(), 40.0).with_angle(std::f32::consts::FRAC_PI_2);
    // The flat dab is wide along x; turned a quarter it is wide along y.
    assert!(plain.coverage_at(15.0, 0.0) > 0.5 && plain.coverage_at(0.0, 15.0) == 0.0);
    assert!(turned.coverage_at(0.0, 15.0) > 0.5 && turned.coverage_at(15.0, 0.0) == 0.0);
}

#[test]
fn a_round_dab_does_not_change_with_angle() {
    let round = DabMask::new(DabShape::round(0.5), 30.0);
    let turned = DabMask::new(DabShape::round(0.5), 30.0).with_angle(1.0);
    for (x, y) in [(3.0, 4.0), (10.0, -2.0), (0.0, 12.0)] {
        assert!((round.coverage_at(x, y) - turned.coverage_at(x, y)).abs() < 1e-3);
    }
}

fn stroke(angle: f32, from_tilt: bool, tilt: (f32, f32)) -> Vec<u8> {
    let mut editor = Editor::new(Document::new(40, 40).unwrap()).unwrap();
    let point = BrushPoint {
        tilt_x: tilt.0,
        tilt_y: tilt.1,
        ..BrushPoint::new(20.0, 20.0, 1.0)
    };
    editor
        .execute(Command::BrushStroke {
            points: vec![point],
            color: Pixel::rgba(0, 0, 0, 255),
            size: 24.0,
            opacity: 1.0,
            settings: BrushSettings {
                shape: flat(),
                angle,
                angle_from_tilt: from_tilt,
                ..BrushSettings::default()
            },
            tip: None,
            pipe: Vec::new(),
        })
        .unwrap();
    let doc = editor.document();
    doc.layer(doc.active_layer_id()).unwrap().pixels().to_vec()
}

#[test]
fn the_setting_turns_painted_dabs_and_tilt_adds_to_it() {
    let alpha = |p: &[u8], x: usize, y: usize| p[(y * 40 + x) * 4 + 3];
    let level = stroke(0.0, false, (0.0, 0.0));
    let upright = stroke(90.0, false, (0.0, 0.0));
    assert!(alpha(&level, 30, 20) > 0 && alpha(&level, 20, 30) == 0);
    assert!(alpha(&upright, 20, 30) > 0 && alpha(&upright, 30, 20) == 0);
    // Pen leaning along +y adds 90 degrees to an angle of 0.
    let tilted = stroke(0.0, true, (0.0, 40.0));
    assert_eq!(tilted, upright);
}

#[test]
fn the_default_stroke_json_is_unchanged() {
    let json = serde_json::to_string(&BrushSettings::default()).unwrap();
    assert!(!json.contains("angle"), "{json}");
}
