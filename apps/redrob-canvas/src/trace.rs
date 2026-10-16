//! Timing trace for mode switches, on stderr when `REDROB_CANVAS_TRACE` is set: when a switch
//! was asked for, when it applied, how long each editor took to build, and each frame's cost
//! until the new editor has settled. Off (one relaxed load) otherwise.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

static ON: AtomicBool = AtomicBool::new(false);
static START: OnceLock<Instant> = OnceLock::new();

/// Read the environment once; before this nothing is traced.
pub fn init() {
    ON.store(
        std::env::var_os("REDROB_CANVAS_TRACE").is_some(),
        Ordering::Relaxed,
    );
    let _ = START.get_or_init(Instant::now);
}

pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Milliseconds since [`init`].
pub fn now_ms() -> f64 {
    START
        .get()
        .map(|s| s.elapsed().as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

/// One trace line, `[  1234.5 ms] text`, when tracing is on.
pub fn log(text: impl FnOnce() -> String) {
    if on() {
        eprintln!("[{:>9.1} ms] {}", now_ms(), text());
    }
}
