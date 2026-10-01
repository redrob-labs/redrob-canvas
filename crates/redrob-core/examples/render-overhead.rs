//! Is the remaining per-layer render cost inside the composite or around it?
//!
//! An idle render (no damage outstanding) hands back the cached projection without compositing anything. If an
//! idle render on a 40-layer document is also slow, the cost is NOT the compositing.

use redrob_core::{BrushPoint, BrushSettings, Command, Document, Editor, LayerId, Pixel};
use std::time::Instant;

fn main() {
    for layers in [1usize, 16, 40] {
        let mut editor = Editor::new(Document::new(800, 800).unwrap()).unwrap();
        for index in 1..layers {
            editor
                .execute(Command::AddLayer {
                    id: LayerId::new(),
                    name: format!("layer {index}"),
                    index,
                })
                .unwrap();
        }
        let stroke = Command::BrushStroke {
            points: vec![BrushPoint::new(400.0, 400.0, 0.9)],
            color: Pixel::rgba(200, 40, 40, 255),
            size: 8.0,
            opacity: 1.0,
            settings: BrushSettings::default(),
            tip: None,
        };
        editor.execute(stroke.clone()).unwrap();
        let _ = editor.render_snapshot().unwrap();

        // Damaged render: one dab, then present.
        let mut damaged = Vec::new();
        let mut idle = Vec::new();
        for _ in 0..30 {
            editor.execute(stroke.clone()).unwrap();
            let start = Instant::now();
            let held = editor.render_snapshot().unwrap();
            damaged.push(start.elapsed());
            drop(held);
            // Now nothing is damaged.
            let start = Instant::now();
            let held = editor.render_snapshot().unwrap();
            idle.push(start.elapsed());
            drop(held);
        }
        damaged.sort_unstable();
        idle.sort_unstable();
        println!(
            "{layers:>3} layers   damaged {:>8.3}ms   idle {:>8.3}ms",
            damaged[damaged.len() / 2].as_secs_f64() * 1000.0,
            idle[idle.len() / 2].as_secs_f64() * 1000.0
        );
    }
}
