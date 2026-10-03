// SPDX-License-Identifier: GPL-3.0-or-later

//! K.0-b. Proof that moving every filter's edge policy onto the shared reader changed no pixel.
//!
//! These digests were captured from the code BEFORE the port and are not recomputed from the
//! current implementation. That is the whole point: a golden value taken after a refactor records
//! whatever the refactor did, including a mistake, and proves nothing. Each number below is what
//! the inline `x.clamp(0, w - 1)` version produced.
//!
//! A digest rather than full byte arrays because there are six images of 140 bytes each and the
//! comparison is all-or-nothing anyway — a mismatch is a mismatch, and the bytes would not say
//! more about why.

use redrob_core::{Command, Document, Editor, Filter, Pixel, Rect, SelectionMode};

/// FNV-1a. Hand-written because this needs no cryptographic property — only that two different
/// images are overwhelmingly unlikely to agree — and adding a hashing dependency for six test
/// values would be a poor trade.
fn fnv(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

/// A 7x5 image with structure in all three channels and a partly transparent first column.
///
/// The transparent column matters: several of these filters read alpha, and a fully opaque test
/// image would not notice a port that dropped it. The size is deliberately small and odd so that
/// every pixel is within a radius-2 window of a border — the region where an edge policy is the
/// only thing that decides the answer.
fn structured(width: u32, height: u32) -> Editor {
    let mut editor = Editor::new(Document::new(width, height).unwrap()).unwrap();
    for y in 0..height {
        for x in 0..width {
            editor
                .execute(Command::SelectRectangle {
                    rect: Rect::new(x as i32, y as i32, 1, 1),
                    mode: SelectionMode::Replace,
                })
                .unwrap();
            editor
                .execute(Command::Fill {
                    color: Pixel::rgba(
                        (x * 37 + 11) as u8,
                        (y * 53 + 7) as u8,
                        if (x + y) % 3 == 0 { 210 } else { 40 },
                        if x == 0 { 128 } else { 255 },
                    ),
                })
                .unwrap();
        }
    }
    editor.execute(Command::ClearSelection).unwrap();
    editor
}

/// Every filter whose edge policy moved onto the shared reader still produces the same pixels.
#[test]
fn porting_the_edge_policy_changed_no_pixel() {
    // Captured from the pre-port implementation. Do not regenerate these from current output.
    let cases: Vec<(&str, Filter, u64)> = vec![
        (
            "EdgeDetect",
            Filter::EdgeDetect { amount: 0.7 },
            0x5b85_1a3a_b492_9f9b,
        ),
        (
            "Emboss",
            Filter::Emboss {
                angle_degrees: 135.0,
            },
            0x6e8a_425d_b67c_4365,
        ),
        (
            "Pick",
            Filter::Pick {
                amount: 0.6,
                seed: 9,
            },
            0x9aea_2b06_bcff_846e,
        ),
        (
            "Spread",
            Filter::Spread { amount: 2, seed: 4 },
            0x28b9_cbd1_9b59_567e,
        ),
        (
            "Oilify",
            Filter::Oilify { radius: 2 },
            0x19c9_b2c6_8ade_579a,
        ),
        (
            "NormalMap",
            Filter::NormalMap { strength: 1.5 },
            0xb18f_0962_d733_7e04,
        ),
    ];

    for (name, filter, expected) in cases {
        let mut editor = structured(7, 5);
        let before = editor.document().layers()[0].pixels().to_vec();
        editor.execute(Command::ApplyFilter { filter }).unwrap();
        let after = editor.document().layers()[0].pixels().to_vec();
        assert_ne!(
            after, before,
            "{name} did not change the image, so its digest proves nothing"
        );
        assert_eq!(
            fnv(&after),
            expected,
            "{name} changed when its edge policy moved onto the shared reader"
        );
    }
}

/// No filter spells its own edge policy any more.
///
/// A guard rather than a behaviour test: the point of K.0 is that there is ONE place deciding what
/// lies outside the image, and that property decays the moment someone adds a filter with its own
/// inline clamp. Reading the source is the only way to assert it.
#[test]
fn no_filter_still_clamps_coordinates_inline() {
    let source = include_str!("../src/filters.rs");
    let offenders: Vec<usize> = source
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains("clamp(0, w - 1)") || line.contains("clamp(0, h - 1)"))
        .map(|(index, _)| index + 1)
        .collect();
    assert!(
        offenders.is_empty(),
        "filters.rs still decides its own edge policy at lines {offenders:?} — use \
         crate::neighbourhood::Neighbourhood instead"
    );
}
