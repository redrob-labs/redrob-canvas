// SPDX-License-Identifier: GPL-3.0-or-later
//! Interactive latency: how long from a dab to a frame the user can see.
//!
//! WHY THIS EXISTS, and it is the whole point of the file. Stage 2 and 3 of the
//! porting plan build a golden-output harness that compares our pixels against
//! Krita's. Measuring `plugins/paintops` and `libs/image` found that the largest
//! genuinely missing subsystem -- Krita's stroke scheduler and dirty-region
//! walkers, 8,910 lines -- is INVISIBLE to such a harness. It changes no pixel. It
//! decides which pixels get recomputed.
//!
//! So a port validated purely by output parity would conclude the scheduler was
//! unnecessary, the harness would stay green, and the product would recompute the
//! whole canvas on every dab. This is the instrument that makes that failure
//! visible, and it is deliberately written BEFORE the work it measures.
//!
//! WHAT THE NUMBERS MEAN. The signal is not the absolute millisecond count, which
//! depends on the machine. It is the SHAPE:
//!
//!   * Latency that stays flat as the document grows means only the touched region
//!     is being recomputed. That is what a scheduler buys.
//!   * Latency that grows with document area means the whole canvas is recomputed
//!     per dab. A 4000x4000 document is 178 times the area of 300x300, so the
//!     difference is not subtle when it is there.
//!   * Latency that grows with LAYER COUNT at a fixed size means the composite is
//!     redone for every layer regardless of which one was touched.
//!
//! Run it with `cargo bench -p redrob-core` (or `cargo test --bench latency` to
//! check it still compiles). It prints a table and asserts nothing: a latency
//! budget that fails CI on a loaded build server is a flaky test, and the purpose
//! here is a recorded baseline to compare against, not a gate.

use std::time::{Duration, Instant};

use redrob_core::{BrushPoint, BrushSettings, Command, Document, Editor, LayerId, Pixel};

/// One measured configuration.
struct Case {
    width: u32,
    height: u32,
    layers: usize,
}

/// A constructor rather than struct literals in the table below, so the sweep reads
/// as the table it is. `cargo fmt` expands a braced literal to four lines each,
/// which turns seven readable rows into forty-five and hides the pattern the sweep
/// exists to show.
const fn case(width: u32, height: u32, layers: usize) -> Case {
    Case {
        width,
        height,
        layers,
    }
}

/// Median rather than mean. A single allocation or page fault in a hundred dabs
/// moves a mean and does not move a median, and the question here is what a user
/// feels per dab rather than what the run cost in total.
fn median(mut samples: Vec<Duration>) -> Duration {
    samples.sort_unstable();
    samples[samples.len() / 2]
}

fn worst(samples: &[Duration]) -> Duration {
    samples.iter().copied().max().unwrap_or_default()
}

/// Build a document of the given size with `layers` raster layers, then measure
/// the dab-to-visible-frame time for a short stroke near the centre.
///
/// The stroke is short and central on purpose: a long diagonal stroke across a
/// large canvas touches a large region legitimately, which would confound "the
/// region is large" with "the whole canvas is being recomputed".
fn measure(case: &Case, dabs: usize) -> (Duration, Duration) {
    let mut editor = Editor::new(Document::new(case.width, case.height).unwrap()).unwrap();

    for index in 1..case.layers {
        editor
            .execute(Command::AddLayer {
                id: LayerId::new(),
                name: format!("layer {index}"),
                index,
            })
            .unwrap();
    }

    let centre_x = case.width as f32 / 2.0;
    let centre_y = case.height as f32 / 2.0;

    // One untimed dab first. The first command through a fresh editor pays for
    // lazily allocated buffers, and charging that to the user's first brush stroke
    // would make every configuration look worse than it is at rest.
    editor
        .execute(Command::BrushStroke {
            points: vec![
                BrushPoint::new(centre_x, centre_y, 0.5),
                BrushPoint::new(centre_x + 4.0, centre_y, 0.5),
            ],
            color: Pixel::rgba(200, 40, 40, 255),
            size: 8.0,
            opacity: 1.0,
            settings: BrushSettings::default(),
            tip: None,
        })
        .unwrap();
    let _ = editor.render_snapshot().unwrap();

    let mut samples = Vec::with_capacity(dabs);
    for dab in 0..dabs {
        let offset = (dab % 16) as f32;
        let start = Instant::now();

        editor
            .execute(Command::BrushStroke {
                points: vec![
                    BrushPoint::new(centre_x + offset, centre_y + offset, 0.4),
                    BrushPoint::new(centre_x + offset + 4.0, centre_y + offset, 0.9),
                ],
                color: Pixel::rgba(200, 40, 40, 255),
                size: 8.0,
                opacity: 1.0,
                settings: BrushSettings::default(),
                tip: None,
            })
            .unwrap();

        // The render is INSIDE the timed section, and that is the measurement.
        // Executing a command and never presenting it is not latency the user
        // experiences; the frame has to become available.
        let snapshot = editor.render_snapshot().unwrap();
        std::hint::black_box(snapshot.pixels().len());

        samples.push(start.elapsed());
    }

    // Read the worst BEFORE median sorts and consumes the samples.
    let worst_seen = worst(&samples);
    (median(samples), worst_seen)
}

fn main() {
    // 60 fps is a 16.7 ms budget for everything, so a dab that costs more than
    // that cannot keep up with a fast stroke. 120 Hz styluses halve it again.
    const FRAME_60HZ: Duration = Duration::from_micros(16_667);

    let cases = [
        // Area sweep at one layer: does latency follow pixel count?
        case(300, 300, 1),
        case(800, 800, 1),
        case(1920, 1080, 1),
        case(4000, 4000, 1),
        // Layer sweep at one size: does latency follow layer count?
        case(800, 800, 4),
        case(800, 800, 16),
        case(800, 800, 40),
    ];

    println!();
    println!("Interactive latency: one dab to a presentable frame");
    println!("median of 40 dabs, 8px brush near the canvas centre");
    println!();
    println!(
        "{:>12}  {:>6}  {:>11}  {:>10}  {:>10}  {:>9}",
        "size", "layers", "megapixels", "median", "worst", "of 60Hz"
    );
    println!("{}", "-".repeat(70));

    let mut first_per_megapixel: Option<f64> = None;

    for case in &cases {
        let (median, worst) = measure(case, 40);
        let megapixels = (case.width as f64 * case.height as f64) / 1_000_000.0;
        let median_ms = median.as_secs_f64() * 1000.0;
        let budget = median.as_secs_f64() / FRAME_60HZ.as_secs_f64();

        if case.layers == 1 && first_per_megapixel.is_none() {
            first_per_megapixel = Some(median_ms / megapixels);
        }

        println!(
            "{:>12}  {:>6}  {:>11.2}  {:>8.3}ms  {:>8.3}ms  {:>8.1}x",
            format!("{}x{}", case.width, case.height),
            case.layers,
            megapixels,
            median_ms,
            worst.as_secs_f64() * 1000.0,
            budget
        );
    }

    println!();
    println!("Reading this table:");
    println!("  * A flat median down the area sweep means only the touched region is");
    println!("    recomputed. Growth proportional to megapixels means the whole canvas is.");
    println!("  * A flat median down the layer sweep means only the touched layer is");
    println!("    composited. Growth means every layer is, whichever one was painted.");
    println!("  * 'of 60Hz' above 1.0x means a dab cannot keep up with a 60Hz stylus.");
    println!();
    println!("This instrument exists because the golden-output harness cannot see any of");
    println!("it: Krita's stroke scheduler changes no pixel, so output parity stays green");
    println!("on a product that recomputes everything. See docs/latency-measurement.md.");
    println!();
}
