// SPDX-License-Identifier: GPL-3.0-or-later

//! GIMP patterns, the `.pat` format (M.10).
//!
//! Re-derived from `app/core/gimppattern-load.c` and `app/core/gimppattern-header.h`
//! (GPL-3.0-or-later), pinned in `docs/upstream-sources.toml`.
//!
//! A pattern is a **resource**, not a document, so this is not a `FileFormat` variant — it mirrors
//! `crate::brush_tip`, which does the same for `.gbr` brushes, and `audit6-format-gap.py` excludes
//! `pat` from the format gap by name for that reason.

use redrob_core::{MAX_PATTERN_EDGE, MAX_PATTERN_NAME, PatError, Pattern, looks_like_pat};

/// Build a `.pat` from its parts, so each test reads as the header layout it is exercising.
fn pat(
    header_size: u32,
    version: u32,
    width: u32,
    height: u32,
    depth: u32,
    name: &[u8],
    body: &[u8],
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&header_size.to_be_bytes()); // 0
    out.extend_from_slice(&version.to_be_bytes()); // 4
    out.extend_from_slice(&width.to_be_bytes()); // 8
    out.extend_from_slice(&height.to_be_bytes()); // 12
    out.extend_from_slice(&depth.to_be_bytes()); // 16
    out.extend_from_slice(b"GPAT"); // 20 -- note the offset
    out.extend_from_slice(name); // 24
    out.extend_from_slice(body);
    out
}

/// **The magic is at offset 20, not 0 — the fact a sniff gets wrong by assuming.**
///
/// `header_size` is the first field, so the signature sits after five `u32`s. This is the second
/// format in group M whose signature is not at the start: M.2's TGA keeps its at the very END.
///
/// Both directions are asserted. `GPAT` at offset 0 must NOT be claimed — otherwise the test would
/// pass under a detector that searched the whole file — and the same four bytes at 20 must be.
#[test]
fn the_magic_sits_at_offset_twenty_not_zero() {
    let mut at_zero = b"GPAT".to_vec();
    at_zero.extend_from_slice(&[0u8; 24]);
    assert!(
        !looks_like_pat(&at_zero),
        "GPAT at offset 0 is not a pattern header"
    );

    let real = pat(25, 1, 1, 1, 3, b"\0", &[9, 8, 7]);
    assert_eq!(&real[20..24], b"GPAT");
    assert!(looks_like_pat(&real));
    assert!(Pattern::from_pat(&real).is_ok());

    // And a file too short to contain the fixed header cannot be one.
    assert_eq!(
        Pattern::from_pat(&[0u8; 20]).unwrap_err(),
        PatError::TooShort
    );
}

/// **A NAME IS MANDATORY**, which is easy to miss and is a rule rather than a convention.
///
/// Upstream rejects `header.header_size <= sizeof (header)`, so a bare 24-byte header is refused
/// even with every other field valid. The smallest legal `header_size` is 25 — one NUL. Asserted as
/// a pair, because a check that refused everything would pass the first half alone.
#[test]
fn a_header_with_no_room_for_a_name_is_refused() {
    let bare = pat(24, 1, 1, 1, 3, b"", &[1, 2, 3]);
    assert_eq!(
        Pattern::from_pat(&bare).unwrap_err(),
        PatError::MissingName,
        "header_size 24 leaves no room for the name upstream requires"
    );

    let minimal = pat(25, 1, 1, 1, 3, b"\0", &[1, 2, 3]);
    assert!(
        Pattern::from_pat(&minimal).is_ok(),
        "25 is the smallest legal header_size"
    );
}

/// The name's terminating NUL is dropped rather than carried into the string.
///
/// Upstream converts `bn_size - 1` bytes for exactly this reason.
#[test]
fn the_name_loses_its_terminating_nul() {
    let named = pat(27, 1, 1, 1, 3, b"hi\0", &[1, 2, 3]);
    let pattern = Pattern::from_pat(&named).expect("decodes");
    assert_eq!(pattern.name(), "hi", "not \"hi\\0\"");

    let too_long = vec![b'x'; 300];
    assert_eq!(
        Pattern::from_pat(&pat(24 + 300, 1, 1, 1, 3, &too_long, &[1, 2, 3])).unwrap_err(),
        PatError::NameTooLong(300)
    );
    assert_eq!(MAX_PATTERN_NAME, 256, "GIMP_PATTERN_MAX_NAME");
}

/// **`bytes` counts BYTES, and each depth maps to a channel layout** — with DISTINCT samples, so
/// the channel order is pinned and not merely the count.
///
/// Upstream's own error message says what the range means: *"GIMP Patterns must be GRAY or RGB"*.
/// This is the fourth spelling of depth in group M — SGI's `bpp` is bytes, SUN raster's `depth` is
/// bits, XBM takes its word size from a C type, and this is bytes again.
#[test]
fn the_depth_is_in_bytes_and_each_value_is_a_channel_layout() {
    // Grey: one sample fills the three colour channels, alpha opaque.
    let grey = Pattern::from_pat(&pat(25, 1, 1, 1, 1, b"\0", &[77])).expect("grey");
    assert_eq!(grey.pixel(0, 0), [77, 77, 77, 255]);

    // Grey + alpha.
    let grey_alpha = Pattern::from_pat(&pat(25, 1, 1, 1, 2, b"\0", &[77, 128])).expect("grey+a");
    assert_eq!(grey_alpha.pixel(0, 0), [77, 77, 77, 128]);

    // RGB -- distinct values, so a channel swap fails here.
    let rgb = Pattern::from_pat(&pat(25, 1, 1, 1, 3, b"\0", &[10, 20, 30])).expect("rgb");
    assert_eq!(rgb.pixel(0, 0), [10, 20, 30, 255]);

    // RGBA.
    let rgba = Pattern::from_pat(&pat(25, 1, 1, 1, 4, b"\0", &[10, 20, 30, 40])).expect("rgba");
    assert_eq!(rgba.pixel(0, 0), [10, 20, 30, 40]);

    for depth in [0u32, 5, 8] {
        let body = vec![0u8; depth.max(1) as usize];
        assert_eq!(
            Pattern::from_pat(&pat(25, 1, 1, 1, depth, b"\0", &body)).unwrap_err(),
            PatError::UnsupportedDepth(depth)
        );
    }
}

/// Dimensions are 1..=10000, and all four boundary failures are checked.
#[test]
fn the_dimension_limits_upstream_enforces_are_enforced() {
    assert_eq!(MAX_PATTERN_EDGE, 10_000, "GIMP_PATTERN_MAX_SIZE");

    for (width, height) in [(0u32, 1u32), (1, 0), (10_001, 1), (1, 10_001)] {
        assert_eq!(
            Pattern::from_pat(&pat(25, 1, width, height, 3, b"\0", &[1, 2, 3])).unwrap_err(),
            PatError::InvalidDimensions { width, height },
            "{width}x{height}"
        );
    }

    // The edge itself is legal, so the check above is a window and not a blanket refusal. Only the
    // header is built here -- the body would be 300 MB, and the point is which error comes back.
    let at_edge = pat(25, 1, MAX_PATTERN_EDGE, MAX_PATTERN_EDGE, 3, b"\0", &[]);
    assert!(
        !matches!(
            Pattern::from_pat(&at_edge),
            Err(PatError::InvalidDimensions { .. })
        ),
        "10000x10000 is within upstream's limits; it fails for size or truncation, not dimensions"
    );
}

/// `version` must be exactly 1.
#[test]
fn only_version_one_is_read() {
    for version in [0u32, 2, 3] {
        assert_eq!(
            Pattern::from_pat(
                &pat(25, 1, 1, 1, 3, b"\0", &[1, 2, 3])
                    .iter()
                    .copied()
                    .enumerate()
                    .map(|(index, byte)| if (4..8).contains(&index) {
                        version.to_be_bytes()[index - 4]
                    } else {
                        byte
                    })
                    .collect::<Vec<_>>()
            )
            .unwrap_err(),
            PatError::UnsupportedVersion(version)
        );
    }
}

/// **Pattern lookup WRAPS**, because that is what a pattern is for.
///
/// A pattern fills an area larger than itself, so a coordinate outside it is the normal case rather
/// than an error. `(5, 7)` on a 2x2 tile must be `(1, 1)`.
#[test]
fn pixel_lookup_wraps_because_a_pattern_tiles() {
    let body = [10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120];
    let tile = Pattern::from_pat(&pat(25, 1, 2, 2, 3, b"\0", &body)).expect("2x2");

    assert_eq!(tile.pixel(0, 0), [10, 20, 30, 255]);
    assert_eq!(tile.pixel(1, 0), [40, 50, 60, 255]);
    assert_eq!(tile.pixel(0, 1), [70, 80, 90, 255]);
    assert_eq!(tile.pixel(1, 1), [100, 110, 120, 255]);

    assert_eq!(tile.pixel(5, 7), tile.pixel(1, 1), "5 % 2 == 1, 7 % 2 == 1");
    assert_eq!(tile.pixel(2, 2), tile.pixel(0, 0));
}

/// Declared pixel data that is not all there is refused rather than read short.
#[test]
fn a_truncated_body_is_refused() {
    let short = pat(25, 1, 4, 4, 3, b"\0", &[1, 2, 3]);
    assert_eq!(Pattern::from_pat(&short).unwrap_err(), PatError::Truncated);
}
