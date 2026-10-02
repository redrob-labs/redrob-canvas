//! Where does a dab's latency actually go?
//!
//! The bench times `execute` + `render_snapshot` together. Before bounding the renderer, find out which half
//! is the cost: `execute_internal` clones the whole document three times for undo history, and at 4000x4000
//! one layer is 64 MB.

use redrob_core::{BrushPoint, BrushSettings, Command, Document, Editor, LayerId, Pixel};
use std::time::{Duration, Instant};

fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort_unstable();
    samples[samples.len() / 2]
}

fn main() {
    println!(
        "{:>12} {:>7} {:>12} {:>12} {:>12}",
        "size", "layers", "execute", "render", "total"
    );
    for (width, height, layers) in [
        (300u32, 300u32, 1usize),
        (800, 800, 1),
        (1920, 1080, 1),
        (4000, 4000, 1),
        (800, 800, 16),
        (800, 800, 40),
    ] {
        let mut editor = Editor::new(Document::new(width, height).unwrap()).unwrap();
        for index in 1..layers {
            editor
                .execute(Command::AddLayer {
                    id: LayerId::new(),
                    name: format!("layer {index}"),
                    index,
                })
                .unwrap();
        }
        let cx = width as f32 / 2.0;
        let cy = height as f32 / 2.0;

        let stroke = |offset: f32| Command::BrushStroke {
            points: vec![
                BrushPoint::new(cx + offset, cy + offset, 0.4),
                BrushPoint::new(cx + offset + 4.0, cy + offset, 0.9),
            ],
            color: Pixel::rgba(200, 40, 40, 255),
            size: 8.0,
            opacity: 1.0,
            settings: BrushSettings::default(),
            tip: None,
            pipe: Vec::new(),
        };

        // Warm up, exactly as the bench does.
        editor.execute(stroke(0.0)).unwrap();
        let _ = editor.render_snapshot().unwrap();

        let mut executes = Vec::new();
        let mut renders = Vec::new();
        for dab in 0..40 {
            let offset = (dab % 16) as f32;
            let start = Instant::now();
            editor.execute(stroke(offset)).unwrap();
            let executed = start.elapsed();
            let render_start = Instant::now();
            let snapshot = editor.render_snapshot().unwrap();
            std::hint::black_box(snapshot.pixels().len());
            renders.push(render_start.elapsed());
            executes.push(executed);
        }
        let execute_ms = median(executes).as_secs_f64() * 1000.0;
        let render_ms = median(renders).as_secs_f64() * 1000.0;
        println!(
            "{:>9}x{:<4} {:>5} {:>10.3}ms {:>10.3}ms {:>10.3}ms",
            width,
            height,
            layers,
            execute_ms,
            render_ms,
            execute_ms + render_ms
        );
    }
}
