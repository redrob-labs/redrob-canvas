// SPDX-License-Identifier: GPL-3.0-or-later

//! Netpbm: PBM / PGM / PPM (M.3).
//!
//! Re-derived from `plug-ins/common/file-pnm.c` (GPL-3.0-or-later), pinned in
//! `docs/upstream-sources.toml`. Its `pnm_types[]` table IS the contract:
//!
//! | magic | planes | body  | maxval |
//! |-------|--------|-------|--------|
//! | `P1`  | 0      | ASCII | 1      |
//! | `P2`  | 1      | ASCII | 255    |
//! | `P3`  | 3      | ASCII | 255    |
//! | `P4`  | 0      | raw   | 1      |
//! | `P5`  | 1      | raw   | 255    |
//! | `P6`  | 3      | raw   | 255    |
//!
//! The plane count is **0 for a bitmap, not 1** — upstream branches on that rather than treating
//! PBM as one-plane grey. `P7` (PAM, 4 planes) and `PF`/`Pf` (PFM, float) come from the same
//! plug-in and are filed separately.

use redrob_core::{
    AlphaPolicy, Command, Document, Editor, ExportOptions, FileFormat, FormatError, ImportOptions,
    Pixel, detect_format, export_document, import_document,
};

fn decode(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let outcome = import_document(bytes, &ImportOptions::default()).expect("import");
    let document = outcome.document();
    let pixels = document.layers()[0].pixels().to_vec();
    (document.width(), document.height(), pixels)
}

fn filled(width: u32, height: u32, color: Pixel) -> Editor {
    let mut editor = Editor::new(Document::new(width, height).unwrap()).unwrap();
    editor.execute(Command::Fill { color }).unwrap();
    editor
}

/// `P1`..`P6` are claimed; `P7`, `PF` and `Pf` are not, and the magic is a TOKEN.
///
/// Upstream registers nine magics at offset 0 and its scanner reads the magic as a token before
/// eating whitespace — so `P6` followed immediately by a digit is not a Netpbm header. `P7` is PAM
/// and `PF`/`Pf` are PFM; claiming them here would decode something this item is not about.
#[test]
fn the_six_bitmap_magics_are_claimed_and_the_other_three_are_not() {
    for magic in ["P1", "P2", "P3", "P4", "P5", "P6"] {
        let mut bytes = magic.as_bytes().to_vec();
        bytes.extend_from_slice(b"\n1 1 255 0");
        assert_eq!(detect_format(&bytes).unwrap(), FileFormat::Pnm, "{magic}");
    }
    for magic in ["P7", "PF", "Pf"] {
        let mut bytes = magic.as_bytes().to_vec();
        bytes.extend_from_slice(b"\n1 1 255 0");
        assert!(
            matches!(detect_format(&bytes), Err(FormatError::UnknownFormat)),
            "{magic} is a different picture and is filed separately"
        );
    }
    // The magic is a token, so the third byte has to be whitespace.
    assert!(matches!(
        detect_format(b"P61 1 255 0"),
        Err(FormatError::UnknownFormat)
    ));
}

/// **In a PBM, 1 is BLACK and 0 is WHITE** — the inverse of every other Netpbm variant.
///
/// `pnm_types[]` gives `P1` and `P4` a maxval of **1** and a plane count of **0**, and the bitmap
/// convention is that a set bit is ink. Every other variant in the table is additive, where a
/// larger sample is brighter, so an implementation carrying that intuition into PBM inverts the
/// image. Both encodings of the same picture are asserted, so the claim is the convention rather
/// than one file.
#[test]
fn in_a_bitmap_one_is_black() {
    // ASCII: "1 0 / 0 1" is ink on the diagonal.
    let (width, height, ascii) = decode(b"P1\n2 2\n1 0\n0 1\n");
    assert_eq!((width, height), (2, 2));
    assert_eq!(&ascii[..4], &[0, 0, 0, 255], "a 1 bit is BLACK");
    assert_eq!(&ascii[4..8], &[255, 255, 255, 255], "a 0 bit is WHITE");

    // Raw, same picture: one bit per pixel, each row padded to a byte.
    let (_, _, raw) = decode(b"P4\n2 2\n\x80\x40");
    assert_eq!(ascii, raw, "P4 is P1's picture, bit-packed");
}

/// ASCII and raw are the same picture for grey and for colour: `P2`/`P5` and `P3`/`P6`.
///
/// Asserted as equality between the two encodings rather than as separate pixel checks, because
/// that is the claim the table makes — the body encoding is not supposed to change anything.
#[test]
fn ascii_and_raw_bodies_decode_to_the_same_image() {
    let (_, _, grey_ascii) = decode(b"P2\n2 1\n255\n0 128\n");
    let (_, _, grey_raw) = decode(b"P5\n2 1\n255\n\x00\x80");
    assert_eq!(grey_ascii, grey_raw);
    assert_eq!(&grey_ascii[..8], &[0, 0, 0, 255, 128, 128, 128, 255]);

    let (_, _, colour_ascii) = decode(b"P3\n2 1\n255\n255 0 0  0 0 255\n");
    let (_, _, colour_raw) = decode(b"P6\n2 1\n255\n\xff\x00\x00\x00\x00\xff");
    assert_eq!(colour_ascii, colour_raw);
    assert_eq!(&colour_ascii[..8], &[255, 0, 0, 255, 0, 0, 255, 255]);
}

/// The export writes `P6` — and **naming that subtype was a defect fix, not a preference.**
///
/// Left to itself the encoder picks `P7` (PAM) for four-plane input, and `P7` is deliberately not
/// claimed by `detect_format` — so the export produced a file this product could not read back.
/// Found by probing the round trip, exactly as M.2's missing TGA footer was. The header is asserted
/// byte for byte because that is the whole of the fix.
#[test]
fn the_export_writes_a_raw_ppm_our_own_sniff_can_read() {
    let color = Pixel {
        r: 200,
        g: 100,
        b: 50,
        a: 255,
    };
    let exported = export_document(
        filled(2, 1, color).document(),
        FileFormat::Pnm,
        &ExportOptions::default(),
    )
    .expect("export");
    let bytes = exported.bytes();

    // Measured, not assumed: the width, height and maxval share one line
    // (`P6\n2 1 255\n`), which upstream's scanner reads fine because it tokenises on whitespace
    // rather than on line breaks.
    assert!(
        bytes.starts_with(b"P6\n2 1 255\n"),
        "raw PPM, three planes, maxval 255 — not P7"
    );
    assert_eq!(detect_format(bytes).unwrap(), FileFormat::Pnm);
    assert!(
        exported.metadata().lossless,
        "a raw PPM stores the samples it was given"
    );

    let (width, height, pixels) = decode(bytes);
    assert_eq!((width, height), (2, 1));
    for chunk in pixels.chunks_exact(4) {
        assert_eq!(chunk, &[200, 100, 50, 255]);
    }
}

/// **No Netpbm variant below `P7` has an alpha plane**, so a non-opaque export goes through the
/// same alpha policy JPEG uses rather than silently flattening.
///
/// One assertion about the pair: the default `RejectNonOpaque` REFUSES, and an explicit matte
/// flattens with the arithmetic written out. A format that quietly dropped alpha would pass a test
/// that only checked the matte case.
#[test]
fn a_non_opaque_export_is_refused_unless_a_matte_is_given() {
    let color = Pixel {
        r: 200,
        g: 100,
        b: 50,
        a: 128,
    };
    let editor = filled(2, 1, color);

    let refused = export_document(
        editor.document(),
        FileFormat::Pnm,
        &ExportOptions::default(),
    );
    assert!(matches!(
        refused,
        Err(redrob_core::CoreError::Format(FormatError::LossRequired(_)))
    ));

    let white = Pixel {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    };
    let flattened = export_document(
        editor.document(),
        FileFormat::Pnm,
        &ExportOptions::default().with_alpha_policy(AlphaPolicy::Flatten { matte: white }),
    )
    .expect("flattened export");

    // (200 * 128 + 255 * 127 + 127) / 255 = 227, and the same arithmetic for the other two.
    let (_, _, pixels) = decode(flattened.bytes());
    assert_eq!(&pixels[..4], &[227, 177, 152, 255]);
}

/// A PNG presented as a PNM is refused rather than decoded as whatever it really is.
#[test]
fn a_png_presented_as_a_pnm_is_refused() {
    let png = export_document(
        filled(
            1,
            1,
            Pixel {
                r: 1,
                g: 2,
                b: 3,
                a: 255,
            },
        )
        .document(),
        FileFormat::Png,
        &ExportOptions::default(),
    )
    .expect("png")
    .into_bytes();

    let options = ImportOptions::default().with_expected_format(FileFormat::Pnm);
    assert!(import_document(&png, &options).is_err());
}
