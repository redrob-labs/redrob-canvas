//! S1. Live strokes: the layer shows the stroke while it is drawn, and what it shows is exactly
//! what the committed stroke paints.

use redrob_core::{
    BrushPoint, BrushSettings, Command, CoreError, Document, Editor, LayerId, Pixel,
};

fn blank() -> Editor {
    Editor::new(Document::new(64, 64).unwrap()).unwrap()
}

fn points() -> Vec<BrushPoint> {
    (0..40)
        .map(|i| {
            let t = i as f32 / 39.0;
            BrushPoint::new(
                8.0 + 48.0 * t,
                20.0 + 24.0 * (t * 3.0).sin().abs(),
                0.4 + 0.6 * t,
            )
        })
        .collect()
}

fn settings() -> BrushSettings {
    BrushSettings {
        smoothing: redrob_core::BrushSmoothing::MovingAverage { window: 3 },
        ..BrushSettings::default()
    }
}

fn begin(editor: &mut Editor) {
    editor
        .begin_live_stroke(
            Pixel::rgba(200, 40, 40, 255),
            9.0,
            0.6,
            settings(),
            None,
            Vec::new(),
        )
        .unwrap();
}

fn committed() -> Vec<u8> {
    let mut editor = blank();
    editor
        .execute(Command::BrushStroke {
            points: points(),
            color: Pixel::rgba(200, 40, 40, 255),
            size: 9.0,
            opacity: 0.6,
            settings: settings(),
            tip: None,
            pipe: Vec::new(),
        })
        .unwrap();
    editor.document().layers()[0].pixels().to_vec()
}

fn layer(editor: &Editor) -> Vec<u8> {
    editor.document().layers()[0].pixels().to_vec()
}

#[test]
fn the_live_pixels_are_the_committed_pixels() {
    let mut editor = blank();
    begin(&mut editor);
    let all = points();
    // In uneven chunks, the way pointer moves arrive.
    for chunk in [&all[..1], &all[1..7], &all[7..22], &all[22..]] {
        editor.extend_live_stroke(chunk).unwrap();
    }
    assert_eq!(
        layer(&editor),
        committed(),
        "drawing live must look exactly like the stroke that will be committed"
    );
    // And the screen shows it before release.
    let shown = editor.render_snapshot().unwrap();
    assert!(
        shown.pixels().chunks_exact(4).any(|p| p[3] > 0),
        "the live stroke is not rendered"
    );
}

#[test]
fn live_painting_leaves_history_alone_and_release_is_one_undo_step() {
    let mut editor = blank();
    let original = layer(&editor);
    let (depth, generation) = (editor.undo_depth(), editor.generation());
    begin(&mut editor);
    editor.extend_live_stroke(&points()[..20]).unwrap();
    assert_eq!(
        editor.undo_depth(),
        depth,
        "a stroke still being drawn is not history"
    );
    assert_eq!(editor.generation(), generation);

    editor.extend_live_stroke(&points()[20..]).unwrap();
    editor.end_live_stroke().unwrap();
    assert!(!editor.has_live_stroke());
    assert_eq!(layer(&editor), committed());
    assert_eq!(editor.undo_depth(), depth + 1);
    editor.undo().unwrap();
    assert_eq!(
        layer(&editor),
        original,
        "one undo removes the whole stroke"
    );
}

#[test]
fn cancel_restores_the_layer_exactly() {
    let mut editor = blank();
    let original = layer(&editor);
    begin(&mut editor);
    editor.extend_live_stroke(&points()).unwrap();
    assert_ne!(layer(&editor), original);
    let changes = editor.cancel_live_stroke().unwrap();
    assert!(
        changes.canvas_changed,
        "the painted region must be refreshed"
    );
    assert_eq!(layer(&editor), original);
    assert_eq!(editor.undo_depth(), 0);
}

#[test]
fn another_edit_mid_stroke_drops_the_unfinished_paint_instead_of_recording_it() {
    let mut editor = blank();
    let original = layer(&editor);
    begin(&mut editor);
    editor.extend_live_stroke(&points()).unwrap();
    editor
        .execute(Command::AddLayer {
            id: LayerId::new(),
            name: "Mid-stroke".into(),
            index: 1,
        })
        .unwrap();
    assert!(!editor.has_live_stroke());
    assert_eq!(
        layer(&editor),
        original,
        "the unfinished stroke leaked into the document"
    );
    editor.undo().unwrap();
    assert_eq!(
        layer(&editor),
        original,
        "the unfinished stroke leaked into history"
    );
}

#[test]
fn a_layer_without_a_cel_falls_back_to_commit_on_release() {
    let mut editor = blank();
    editor
        .execute(Command::AddGroup {
            id: LayerId::new(),
            name: "Group".into(),
            parent: None,
            sibling_index: 1,
        })
        .unwrap();
    // A group is active and has no cel.
    let result = editor.begin_live_stroke(
        Pixel::rgba(0, 0, 0, 255),
        4.0,
        1.0,
        settings(),
        None,
        Vec::new(),
    );
    assert!(
        matches!(result, Err(CoreError::LiveStrokeUnavailable)),
        "{result:?}"
    );
}

/// Measures the cost of one repaint, which is what a pointer move pays. Run with
/// `cargo test --release -p redrob-core --test live_stroke -- --ignored --nocapture`.
#[test]
#[ignore = "timing, not correctness"]
fn repaint_cost() {
    for (canvas, size, count) in [
        (1280_u32, 18.0_f32, 400_usize),
        (1280, 120.0, 400),
        (4000, 60.0, 400),
    ] {
        let mut editor = Editor::new(Document::new(canvas, canvas * 5 / 8).unwrap()).unwrap();
        editor
            .begin_live_stroke(
                Pixel::rgba(0, 0, 0, 255),
                size,
                1.0,
                BrushSettings::default(),
                None,
                Vec::new(),
            )
            .unwrap();
        let line: Vec<BrushPoint> = (0..count)
            .map(|i| BrushPoint::new(20.0 + i as f32 * 2.0, 300.0, 1.0))
            .collect();
        let started = std::time::Instant::now();
        let mut worst = std::time::Duration::ZERO;
        for point in &line {
            let one = std::time::Instant::now();
            editor
                .extend_live_stroke(std::slice::from_ref(point))
                .unwrap();
            editor.render_snapshot().unwrap();
            worst = worst.max(one.elapsed());
        }
        println!(
            "canvas {canvas} brush {size}: {count} moves in {:?}, worst move {:?}",
            started.elapsed(),
            worst
        );
    }
}
