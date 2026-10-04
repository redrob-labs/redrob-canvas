// SPDX-License-Identifier: GPL-3.0-or-later

//! Quite OK Image (M.6).
//!
//! Re-derived from `plug-ins/common/file-qoi.c` (GPL-3.0-or-later), pinned in
//! `docs/upstream-sources.toml`. Magic `0,string,qoif`, and a fixed 14-byte header: the magic, a
//! big-endian width and height, then `channels` and `colorspace` as one byte each.
//!
//! The codec is trivial by design, so the whole substance of this item is **one byte**. Upstream
//! maps `colorspace` straight onto precision, and does not touch the samples either way:
//!
//! ```text
//! desc.colorspace ? GIMP_PRECISION_U8_LINEAR : GIMP_PRECISION_U8_NON_LINEAR
//! ```
//!
//! Export runs it backwards: any `*_LINEAR` image precision writes `QOI_LINEAR`, everything else
//! writes `QOI_SRGB`. This product's `Precision` carries only a DEPTH — U8, U16, F32 — with no
//! transfer-curve axis, so a linear QOI cannot be represented and must at least be reported.

use redrob_core::{
    Command, Document, Editor, ExportOptions, FileFormat, FormatError, FormatWarning,
    ImportOptions, Pixel, detect_format, export_document, import_document,
};

fn filled(color: Pixel) -> Editor {
    let mut editor = Editor::new(Document::new(2, 2).unwrap()).unwrap();
    editor.execute(Command::Fill { color }).unwrap();
    editor
}

fn exported(color: Pixel) -> Vec<u8> {
    export_document(
        filled(color).document(),
        FileFormat::Qoi,
        &ExportOptions::default(),
    )
    .expect("export")
    .into_bytes()
}

const OPAQUE: Pixel = Pixel {
    r: 200,
    g: 100,
    b: 50,
    a: 255,
};

/// The exported header is checked field by field, and so is the end marker.
///
/// The round trip is checked FIRST on every format item now, because M.1 through M.3 each shipped
/// a defect where our own export produced a file our own sniff refused, and none were visible from
/// reading. Width and height are big-endian here — the opposite of ICO's little-endian count two
/// items ago, which is the kind of detail that is cheap to assert and expensive to get wrong.
#[test]
fn the_exported_header_and_end_marker_match_the_specification() {
    let bytes = exported(OPAQUE);

    assert_eq!(&bytes[..4], b"qoif");
    assert_eq!(
        u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        2
    );
    assert_eq!(
        u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
        2
    );
    assert_eq!(bytes[12], 4, "RGBA");
    assert_eq!(bytes[13], 0, "sRGB — nothing here can ask for linear");
    assert_eq!(
        &bytes[bytes.len() - 8..],
        &[0, 0, 0, 0, 0, 0, 0, 1],
        "the stream ends with seven zero bytes and a one"
    );

    assert_eq!(detect_format(&bytes).unwrap(), FileFormat::Qoi);
    let back = import_document(&bytes, &ImportOptions::default()).expect("re-import");
    let document = back.document();
    assert_eq!((document.width(), document.height()), (2, 2));
    for chunk in document.layers()[0].pixels().chunks_exact(4) {
        assert_eq!(chunk, &[200, 100, 50, 255]);
    }
}

/// **The one byte this item is really about, asserted as a PAIR.**
///
/// A `colorspace` of 1 means the samples are linear. `Precision` here has no transfer-curve axis,
/// so they are necessarily read as non-linear — reading linear samples as sRGB is a wrong picture,
/// not a rounding difference, so the import says so instead of staying quiet. Dropping the
/// declaration silently is what the probe found, and this is the test over that fix.
///
/// Both sides are asserted in one test on purpose: an import that always warned, or never warned,
/// would pass either half alone.
#[test]
fn a_linear_qoi_is_reported_and_an_srgb_one_is_not() {
    let srgb = exported(OPAQUE);
    assert_eq!(srgb[13], 0);
    let quiet = import_document(&srgb, &ImportOptions::default()).expect("import");
    assert!(
        quiet.warnings().is_empty(),
        "an sRGB QOI needs no explanation"
    );

    // The same file, re-declared linear. Only that byte differs, so the pixels cannot be the cause.
    let mut linear = srgb.clone();
    linear[13] = 1;
    let reported = import_document(&linear, &ImportOptions::default()).expect("import");
    assert!(
        reported.warnings().iter().any(|warning| matches!(
            warning,
            FormatWarning::ConvertedColorMode {
                source: "qoi-linear"
            }
        )),
        "a linear declaration this product cannot hold must be reported, got {:?}",
        reported.warnings()
    );
}

/// `channels` and `colorspace` are enumerated, so they are evidence and are checked.
///
/// Four bytes of lowercase text is weak on its own. The specification allows only 3 or 4 channels
/// and only 0 or 1 for the colour space, so anything else means the file is not one whatever it
/// begins with — and a header shorter than 14 bytes cannot be one either.
#[test]
fn the_enumerated_header_fields_are_evidence_not_decoration() {
    let valid = exported(OPAQUE);

    let mut bad_channels = valid.clone();
    bad_channels[12] = 2;
    assert!(matches!(
        detect_format(&bad_channels),
        Err(FormatError::UnknownFormat)
    ));

    let mut bad_colorspace = valid.clone();
    bad_colorspace[13] = 7;
    assert!(matches!(
        detect_format(&bad_colorspace),
        Err(FormatError::UnknownFormat)
    ));

    // Three channels is legal, so this one must still be claimed — otherwise the check above would
    // also pass with the whole field rejected.
    let mut three = valid.clone();
    three[12] = 3;
    assert_eq!(detect_format(&three).unwrap(), FileFormat::Qoi);

    assert!(matches!(
        detect_format(b"qoif"),
        Err(FormatError::UnknownFormat)
    ));
}

/// Alpha survives the round trip, and the header says so.
#[test]
fn alpha_survives_and_the_channel_count_declares_it() {
    let bytes = exported(Pixel {
        r: 200,
        g: 100,
        b: 50,
        a: 128,
    });
    assert_eq!(bytes[12], 4);

    let back = import_document(&bytes, &ImportOptions::default()).expect("re-import");
    assert_eq!(
        &back.document().layers()[0].pixels()[..4],
        &[200, 100, 50, 128]
    );
}

/// A PNG presented as a QOI is refused rather than decoded as what it really is.
#[test]
fn a_png_presented_as_a_qoi_is_refused() {
    let png = export_document(
        filled(OPAQUE).document(),
        FileFormat::Png,
        &ExportOptions::default(),
    )
    .expect("png")
    .into_bytes();

    let options = ImportOptions::default().with_expected_format(FileFormat::Qoi);
    assert!(import_document(&png, &options).is_err());
}
