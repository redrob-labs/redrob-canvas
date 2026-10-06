//! P7. Pen tilt: carried on each brush point, readable as a dynamics sensor, invisible to every
//! stroke and file that has none.

use redrob_core::{
    BrushDynamic, BrushPoint, BrushSettings, Command, CoreError, Document, DynamicSensor, Editor,
    Pixel,
};

fn stroke(points: Vec<BrushPoint>, settings: BrushSettings) -> Command {
    Command::BrushStroke {
        points,
        color: Pixel::rgba(0, 0, 0, 255),
        size: 8.0,
        opacity: 1.0,
        settings,
        tip: None,
        pipe: Vec::new(),
    }
}

fn line(tilt_x: f32) -> Vec<BrushPoint> {
    (4..28)
        .map(|x| BrushPoint::new(x as f32, 16.0, 0.5).with_tilt(tilt_x, 0.0))
        .collect()
}

fn painted(points: Vec<BrushPoint>, settings: BrushSettings) -> usize {
    let mut editor = Editor::new(Document::new(32, 32).unwrap()).unwrap();
    editor.execute(stroke(points, settings)).unwrap();
    editor.document().layers()[0]
        .pixels()
        .chunks_exact(4)
        .filter(|pixel| pixel[3] > 0)
        .count()
}

fn size_from_tilt() -> BrushSettings {
    BrushSettings {
        dynamics: vec![BrushDynamic {
            sensor: DynamicSensor::Tilt,
            amount: 1.0,
        }],
        ..BrushSettings::default()
    }
}

#[test]
fn a_point_without_tilt_serialises_as_it_always_did() {
    let upright = serde_json::to_value(BrushPoint::new(1.0, 2.0, 0.5)).unwrap();
    assert_eq!(
        upright,
        serde_json::json!({ "x": 1.0, "y": 2.0, "pressure": 0.5 })
    );

    let leaning = BrushPoint::new(1.0, 2.0, 0.5).with_tilt(30.0, -45.0);
    let json = serde_json::to_value(leaning).unwrap();
    assert_eq!(json["tilt_x"], 30.0);
    assert_eq!(json["tilt_y"], -45.0);
    assert_eq!(serde_json::from_value::<BrushPoint>(json).unwrap(), leaning);

    // A stroke recorded before tilt existed still reads, as upright.
    let old: BrushPoint =
        serde_json::from_value(serde_json::json!({ "x": 1.0, "y": 2.0, "pressure": 0.5 })).unwrap();
    assert_eq!(old.tilt_amount(), 0.0);
}

#[test]
fn tilt_amount_is_zero_upright_and_one_flat() {
    assert_eq!(BrushPoint::new(0.0, 0.0, 1.0).tilt_amount(), 0.0);
    assert_eq!(
        BrushPoint::new(0.0, 0.0, 1.0)
            .with_tilt(90.0, 0.0)
            .tilt_amount(),
        1.0
    );
    let half = BrushPoint::new(0.0, 0.0, 1.0)
        .with_tilt(45.0, 0.0)
        .tilt_amount();
    assert!((half - 0.5).abs() < 1e-6, "{half}");
    // Both axes together lean further than either alone.
    assert!(
        BrushPoint::new(0.0, 0.0, 1.0)
            .with_tilt(30.0, 30.0)
            .tilt_amount()
            > BrushPoint::new(0.0, 0.0, 1.0)
                .with_tilt(30.0, 0.0)
                .tilt_amount()
    );
}

#[test]
fn a_tilt_binding_drives_the_brush_size() {
    let upright = painted(line(0.0), size_from_tilt());
    let flat = painted(line(90.0), size_from_tilt());
    assert!(
        flat > upright,
        "a leaning pen must paint wider under a tilt binding: upright {upright}, flat {flat}"
    );
    // Without the binding, tilt changes nothing.
    assert_eq!(
        painted(line(0.0), BrushSettings::default()),
        painted(line(90.0), BrushSettings::default()),
        "tilt must not reach the stroke unless a binding reads it"
    );
}

#[test]
fn smoothing_keeps_the_tilt() {
    // Smoothing rebuilds every point; it must carry the lean through, or a smoothed tablet
    // stroke would read as upright to the binding.
    let smoothed = BrushSettings {
        smoothing: redrob_core::BrushSmoothing::MovingAverage { window: 4 },
        ..size_from_tilt()
    };
    assert!(painted(line(90.0), smoothed.clone()) > painted(line(0.0), smoothed));
}

#[test]
fn a_tilt_beyond_a_right_angle_is_refused() {
    for (tx, ty) in [
        (91.0, 0.0),
        (0.0, -91.0),
        (f32::NAN, 0.0),
        (0.0, f32::INFINITY),
    ] {
        let mut editor = Editor::new(Document::new(8, 8).unwrap()).unwrap();
        let result = editor.execute(stroke(
            vec![BrushPoint::new(4.0, 4.0, 1.0).with_tilt(tx, ty)],
            BrushSettings::default(),
        ));
        assert!(
            matches!(result, Err(CoreError::InvalidPressure)),
            "tilt ({tx}, {ty}) must be refused, got {result:?}"
        );
    }
}
