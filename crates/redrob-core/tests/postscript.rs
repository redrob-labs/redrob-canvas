// SPDX-License-Identifier: GPL-3.0-or-later

//! PostScript and Encapsulated PostScript (M.8) — **detected, rendering refused.**
//!
//! Re-derived from `plug-ins/common/file-ps.c` (GPL-3.0-or-later), pinned in
//! `docs/upstream-sources.toml`. `crate::postscript` carries the reasoning for the refusal; the
//! short form is that upstream renders by invoking **Ghostscript** and reading back PNM, this
//! product will not spawn an external binary, and the pure-Rust interpreter that does exist
//! (**`stet 0.8.3`**, named rather than wished away) is declined on the precedent `crate::pdf`
//! already set for PDF: *"a half-written interpreter renders something for every file."*
//! PostScript is Turing-complete, so it is the stronger case of that argument, not the weaker one.
//!
//! Not reading the embedded EPS preview is **parity**: upstream registers the DOS EPS magic and
//! never reads the preview behind it, admitting in its own thumbnail loader that *"We should look
//! for an embedded preview but for now we just load the document at a small resolution."*
//!
//! What IS re-derived here is detection, and the find of the item is that **telling EPS from PS is
//! a DISTANCE test**: `"EPSF-"` must begin 11 to 15 bytes after `"PS-Adobe-"`, inside the first
//! 512 bytes.

use redrob_core::{
    Document, Editor, ExportOptions, FileFormat, FormatError, ImportOptions, detect_format,
    export_document, import_document,
};

fn detect(bytes: &[u8]) -> FileFormat {
    detect_format(bytes).expect("detected")
}

/// Both magics upstream registers are claimed: `%!` at offset 0, and the DOS EPS binary header.
///
/// The two procedures register them IDENTICALLY — `0,string,%!,0,long,0xc5d0d3c6` for both PS and
/// EPS — so the magic alone cannot separate the flavours, which is why the distance test below
/// exists at all.
#[test]
fn both_registered_magics_are_claimed() {
    assert_eq!(
        detect(b"%!PS-Adobe-3.0\n%%BoundingBox: 0 0 10 10\n"),
        FileFormat::PostScript
    );

    let mut dos = vec![0xc5u8, 0xd0, 0xd3, 0xc6];
    dos.extend_from_slice(&[0u8; 26]);
    assert_eq!(
        detect(&dos),
        FileFormat::Eps,
        "the DOS EPS binary header is EPS unconditionally, with no text at all"
    );

    for negative in [&b"% not postscript\n"[..], &b"hello world\n"[..], &b"%"[..]] {
        assert!(
            matches!(detect_format(negative), Err(FormatError::UnknownFormat)),
            "{:?}",
            String::from_utf8_lossy(negative)
        );
    }
}

/// **The distance window, pinned from BOTH sides.**
///
/// Upstream computes `ds = epsf - adobe` and takes `ds >= 11 && ds <= 15`. Both substrings being
/// present is NOT the test — a document that merely mentions EPSF further along its header is a
/// plain PS. `"PS-Adobe-"` is 9 bytes, so N bytes of padding between them gives `ds = 9 + N`, and
/// four points settle it:
///
/// | `ds` | verdict        |
/// |------|----------------|
/// | 10   | **PostScript** |
/// | 11   | **Eps**        |
/// | 15   | **Eps**        |
/// | 16   | **PostScript** |
///
/// A single `%!PS-Adobe-3.0 EPSF-3.0` case would pass under a plain substring test too; only the
/// two OUTSIDE points prove there is a window at all.
#[test]
fn eps_is_decided_by_the_distance_between_the_two_markers() {
    let at = |distance: usize| -> FileFormat {
        // "PS-Adobe-" is 9 bytes; pad the remainder so `EPSF-` lands at exactly `distance`.
        let padding = "x".repeat(distance - 9);
        detect(format!("%!PS-Adobe-{padding}EPSF-3.0\n").as_bytes())
    };

    assert_eq!(at(10), FileFormat::PostScript, "10 is below the window");
    assert_eq!(at(11), FileFormat::Eps, "11 is the first inside");
    assert_eq!(at(15), FileFormat::Eps, "15 is the last inside");
    assert_eq!(at(16), FileFormat::PostScript, "16 is above the window");

    // And the real-world header, which is why the window is 11..=15: a version plus a space.
    assert_eq!(
        detect(b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 1 1\n"),
        FileFormat::Eps
    );
}

/// `EPSF-` appearing BEFORE `PS-Adobe-` is not EPS.
///
/// Upstream's `ds` is signed and a negative value cannot satisfy `>= 11`.
///
/// **The fixture here had to be chosen with care, and the first one did not work.** A naive
/// `%!EPSF-3.0 PS-Adobe-3.0` puts the markers 9 apart, which `11..=15` rejects whatever the sign —
/// so the test passed for the range's reason, not the ordering's, and an unsigned subtraction
/// slipped through it. Replacing `checked_sub` with `abs_diff` left all seven tests green.
///
/// Discriminating it needs the markers a VALID distance apart in the WRONG order: `EPSF-` at 2 and
/// `PS-Adobe-` at 15 is a gap of 13, inside the window. Signed arithmetic rejects it; unsigned
/// absolute difference accepts it. Now the mechanism is what is under test.
#[test]
fn a_marker_in_the_wrong_order_is_not_eps() {
    let mut reversed = b"%!EPSF-3.0".to_vec();
    // Pad so `PS-Adobe-` begins at offset 15, a gap of 13 from `EPSF-` at offset 2.
    reversed.extend_from_slice(&vec![b' '; 15 - reversed.len()]);
    reversed.extend_from_slice(b"PS-Adobe-3.0\n");

    assert_eq!(
        reversed.windows(5).position(|w| w == b"EPSF-"),
        Some(2),
        "fixture: EPSF- is at 2"
    );
    assert_eq!(
        reversed.windows(9).position(|w| w == b"PS-Adobe-"),
        Some(15),
        "fixture: PS-Adobe- is at 15, so the unsigned gap is 13 -- inside 11..=15"
    );

    assert_eq!(
        detect(&reversed),
        FileFormat::PostScript,
        "a valid-looking gap in the wrong order is still not EPS"
    );
}

/// Only the first 512 bytes are examined — upstream's own `hdr[512]`.
///
/// **This test was wrong on its first writing and the reverse-verification is what caught it.**
/// The first version put `EPSF-` some 600 bytes after `PS-Adobe-`, which the DISTANCE rule rejects
/// on its own — so widening the window to 64 KiB changed nothing and all seven tests still passed.
/// The window claim was untested while appearing to be tested.
///
/// Discriminating it needs the markers a VALID distance apart but straddling the boundary:
/// `PS-Adobe-` at offset 500 and `EPSF-` 13 bytes later at 513. Inside a 512-byte window the
/// second marker is cut off and this is a plain PS; widen the window and it becomes EPS. So this
/// now fails if the window grows, which is the whole point of asserting it.
#[test]
fn the_marker_is_invisible_past_the_header_window() {
    let mut straddling = b"%!".to_vec();
    // Pad so `PS-Adobe-` begins at exactly offset 500.
    straddling.extend_from_slice(&vec![b' '; 500 - straddling.len()]);
    straddling.extend_from_slice(b"PS-Adobe-");
    assert_eq!(straddling.len(), 509);
    // Four bytes of version puts `EPSF-` at 513 -- a distance of 13, inside 11..=15.
    straddling.extend_from_slice(b"3.0 ");
    assert_eq!(straddling.len(), 513, "EPSF- begins just past the window");
    straddling.extend_from_slice(b"EPSF-3.0\n");

    assert_eq!(
        detect(&straddling),
        FileFormat::PostScript,
        "the distance is valid but the second marker is past 512 bytes"
    );

    // The control: the same construction far enough inside the window IS found and IS EPS, so the
    // assertion above cannot pass merely because something else rejected the file.
    //
    // This control was ALSO wrong on its first writing — it placed `PS-Adobe-` at 498, which puts
    // `EPSF-` at 511 needing bytes 511..516, so it did not fit the 512-byte window either and the
    // control failed for the same reason as the case it was meant to contrast. 480 leaves room.
    let mut inside = b"%!".to_vec();
    inside.extend_from_slice(&vec![b' '; 480 - inside.len()]);
    inside.extend_from_slice(b"PS-Adobe-3.0 EPSF-3.0\n");
    assert_eq!(
        detect(&inside),
        FileFormat::Eps,
        "wholly inside the window, the verdict flips"
    );
}

/// **Rendering is refused for both flavours, and the message says which one it decided you had.**
///
/// Asserted as a pair so the refusal cannot collapse into one generic answer: the whole value of
/// detecting the flavour is lost if both report the same thing.
#[test]
fn rendering_is_refused_and_the_message_names_the_flavour() {
    let plain = import_document(
        b"%!PS-Adobe-3.0\n%%BoundingBox: 0 0 1 1\n",
        &ImportOptions::default(),
    )
    .expect_err("refused");
    let message = plain.to_string();
    assert!(message.contains("PostScript"), "{message}");
    assert!(
        !message.contains("Encapsulated"),
        "a plain PS must not be reported as EPS: {message}"
    );

    let encapsulated = import_document(
        b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 1 1\n",
        &ImportOptions::default(),
    )
    .expect_err("refused");
    assert!(
        encapsulated.to_string().contains("Encapsulated"),
        "{encapsulated}"
    );
}

/// Export is refused too, and for a different reason worth keeping separate.
///
/// Upstream CAN write PostScript, and writing needs no interpreter — so unlike the read side this
/// is a scope decision rather than a capability one. Asserted so that implementing it later is a
/// deliberate change to a test rather than a silent new behaviour.
#[test]
fn export_is_refused_as_a_scope_decision() {
    let editor = Editor::new(Document::new(2, 2).unwrap()).unwrap();
    for format in [FileFormat::PostScript, FileFormat::Eps] {
        let refused = export_document(editor.document(), format, &ExportOptions::default());
        assert!(
            matches!(
                refused,
                Err(redrob_core::CoreError::Format(
                    FormatError::UnsupportedFeature(_)
                ))
            ),
            "{format:?} must refuse rather than write something"
        );
    }
}

/// The expected format does not bypass detection: a PNG is not PostScript however it is labelled.
#[test]
fn the_expected_format_does_not_bypass_detection() {
    let png = export_document(
        Editor::new(Document::new(1, 1).unwrap())
            .unwrap()
            .document(),
        FileFormat::Png,
        &ExportOptions::default(),
    )
    .expect("png")
    .into_bytes();

    for format in [FileFormat::PostScript, FileFormat::Eps] {
        let options = ImportOptions::default().with_expected_format(format);
        assert!(import_document(&png, &options).is_err(), "{format:?}");
    }
}
