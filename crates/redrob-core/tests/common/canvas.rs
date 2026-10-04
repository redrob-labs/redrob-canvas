// SPDX-License-Identifier: GPL-3.0-or-later

//! A memoised canvas builder for the filter tests.
//!
//! # Why this exists
//!
//! `Editor` has no public direct-pixel path -- a layer's `pixels()` is read-only -- so a test builds
//! its input by issuing a `SelectRectangle` and a `Fill` **per pixel**. A 64-pixel square canvas is
//! therefore 4096 command pairs, each recorded into history. That is the single largest cost in the
//! test suite.
//!
//! AUDIT-8 (cycle 85) measured it for the first time: 974 tests, 290s wall clock, with
//! `filters_distort.rs` and `filters_render.rs` alone accounting for 228s of it -- 79% of the suite
//! while holding 17% of the tests, ten times slower per test than `editor_core`. A mechanical count
//! then found **164 redundant canvas builds across 90 tests** in three files: the pattern is a
//! closure or helper that builds a canvas and is invoked several times with the same input.
//!
//! # Why memoisation rather than editing 90 tests
//!
//! The waste is always the *same* canvas built again, so caching on its content removes all of it
//! without touching a single assertion. `Document` is `Clone`, and every caller immediately hands a
//! clone to `Editor::new`, so the cached original is never mutated and cannot leak state between
//! tests.
//!
//! # Why the suite total is the thing to watch, not the slowest test
//!
//! AUDIT-8 also found that fixing the slowest single test -- 47.56s to 8.17s, a 5.8x gain -- moved
//! the suite by **nothing**. Cargo runs test BINARIES serially while threading tests within one, so
//! a file with 91 tests on 16 threads is throughput-bound: removing one test's work just leaves the
//! others to fill the threads. The lever is total CPU work per file, which is why this is applied
//! everywhere rather than to the worst offenders.

use redrob_core::{Command, Document, Editor, Pixel, Rect, SelectionMode};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Build a canvas the slow way: one select-and-fill command pair per pixel.
fn build(width: u32, height: u32, colors: &[Pixel]) -> Document {
    let mut editor = Editor::new(Document::new(width, height).expect("document")).expect("editor");
    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let color = colors[(y as usize) * width as usize + x as usize];
            editor
                .execute(Command::SelectRectangle {
                    rect: Rect::new(x, y, 1, 1),
                    mode: SelectionMode::Replace,
                })
                .expect("select");
            editor.execute(Command::Fill { color }).expect("fill");
        }
    }
    editor.execute(Command::ClearSelection).expect("clear");
    editor.document().clone()
}

type Cache = Mutex<HashMap<(u32, u32, Vec<u8>), Document>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// An `Editor` over the given canvas, building it at most once per distinct content.
///
/// The key is the pixel bytes themselves, so two tests asking for the same canvas share one build
/// and a test asking twice pays once. Returns a fresh `Editor` over a CLONE every time, so nothing
/// is shared beyond the cached original.
///
/// # Why the lock is never held across a build
///
/// The first version of this did hold it, and **made two of the three files substantially worse** --
/// `filters_distort` went 116s to 189s and `filters_edge` 22s to 41s, while only `filters_render`
/// improved. Tests run on 16 threads, and a build takes seconds; holding one global mutex across it
/// serialised every canvas construction in the file, which is strictly worse than the concurrent
/// rebuilding it replaced.
///
/// So the lookup and the insert each take the lock briefly and the build happens outside it. Two
/// threads racing on the same missing key both build, which wastes one build once rather than
/// serialising all of them -- and that trade is why the hit rate decides the gain: `filters_render`
/// reuses a handful of flat canvases and benefits most, `filters_distort` asks for many distinct
/// ones and benefits least.
pub fn editor(width: u32, height: u32, colors: &[Pixel]) -> Editor {
    let key: Vec<u8> = colors.iter().flat_map(|p| [p.r, p.g, p.b, p.a]).collect();

    // Look, then release.
    if let Some(found) = cache()
        .lock()
        .expect("canvas cache")
        .get(&(width, height, key.clone()))
    {
        return Editor::new(found.clone()).expect("editor");
    }

    // Build with no lock held.
    let document = build(width, height, colors);

    cache()
        .lock()
        .expect("canvas cache")
        .insert((width, height, key), document.clone());
    Editor::new(document).expect("editor")
}
