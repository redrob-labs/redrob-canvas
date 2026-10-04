// SPDX-License-Identifier: GPL-3.0-or-later

//! JPEG 2000, container and bare codestream (M.5).
//!
//! Re-derived from `plug-ins/common/file-jp2.c` (GPL-3.0-or-later), pinned in
//! `docs/upstream-sources.toml`. Upstream registers TWO procedures for one codec in two wrappers:
//!
//! | wrapper     | upstream magic                      | extensions      |
//! |-------------|-------------------------------------|-----------------|
//! | JP2         | `3,string,\x0CjP`                   | `jp2`           |
//! | codestream  | `0,string,\xff\x4f\xff\x51\x00`     | `j2k,j2c,jpc`   |
//!
//! **The JP2 magic is a documented workaround, and this product does not inherit it.** The comment
//! directly above it says so:
//!
//! > XXX: more complete magic number would be:
//! > `"0,string,\x00\x00\x00\x0C\x6A\x50\x20\x20\x0D\x0A\x87\x0A"`
//! > But the '\0' character makes problem in a 0-terminated string obviously […]
//!
//! That is a limit of GIMP's magic-string syntax, not of the format. Nothing here is
//! 0-terminated, so the full 12-byte signature box is checked and the difference is asserted.
//!
//! The fixtures were generated locally from a 2×2 PNG at reversible quality, so they are this
//! repository's own content rather than a vendored third-party sample. Red, green, blue and white
//! at four distinct positions — chosen so the test pins channel order and row order, which a flat
//! fill cannot do: a decoder that swapped R and B, or flipped the rows, would pass against a solid
//! colour.

use redrob_core::{
    Document, Editor, ExportOptions, FileFormat, FormatError, ImportOptions, detect_format,
    export_document, import_document,
};

const JP2: &[u8] = include_bytes!("fixtures/rgbw.jp2");
const J2K: &[u8] = include_bytes!("fixtures/rgbw.j2k");

/// The two wrappers are told apart, and each carries the signature upstream documents.
#[test]
fn both_wrappers_are_detected_from_their_own_signatures() {
    assert_eq!(
        &JP2[..12],
        b"\x00\x00\x00\x0CjP  \x0D\x0A\x87\x0A",
        "the JP2 signature box: length 12, type `jP  `, then 0D 0A 87 0A"
    );
    assert_eq!(detect_format(JP2).unwrap(), FileFormat::Jp2);

    assert_eq!(
        &J2K[..5],
        &[0xff, 0x4f, 0xff, 0x51, 0x00],
        "SOC then SIZ, and the zero upstream's magic also includes"
    );
    assert_eq!(detect_format(J2K).unwrap(), FileFormat::J2k);
}

/// **The divergence from upstream, asserted so it cannot quietly regress.**
///
/// Upstream matches three bytes at offset 3 because its magic strings cannot carry a `\0`. A file
/// carrying only those three bytes — with a wrong box length and wrong trailing content — satisfies
/// that rule and is not a JP2. Here it is refused, because the whole signature box is compared.
#[test]
fn the_three_bytes_upstream_can_match_are_not_enough_here() {
    let mut weak = vec![0xaa, 0xbb, 0xcc, 0x0c, b'j', b'P'];
    weak.extend_from_slice(&[0u8; 10]);
    assert_eq!(
        &weak[3..6],
        b"\x0CjP",
        "this is exactly what `3,string,\\x0CjP` matches"
    );
    assert!(matches!(
        detect_format(&weak),
        Err(FormatError::UnknownFormat)
    ));
}

/// Both wrappers decode, to the same pixels, in the right channel and row order.
///
/// Asserted as equality between the two decodes as well as against the known pattern: the wrapper
/// is not supposed to change the picture, and that is a stronger claim than two separate checks.
#[test]
fn both_wrappers_decode_to_the_same_image() {
    let mut decoded = Vec::new();
    for bytes in [JP2, J2K] {
        let outcome = import_document(bytes, &ImportOptions::default()).expect("import");
        let document = outcome.document();
        assert_eq!((document.width(), document.height()), (2, 2));
        decoded.push(document.layers()[0].pixels().to_vec());
    }
    assert_eq!(
        decoded[0], decoded[1],
        "the wrapper does not change the image"
    );

    // Red, green, blue, white -- in row order. A channel swap or a row flip fails here.
    assert_eq!(
        &decoded[0][..16],
        &[
            255, 0, 0, 255, // top-left red
            0, 255, 0, 255, // top-right green
            0, 0, 255, 255, // bottom-left blue
            255, 255, 255, 255, // bottom-right white
        ]
    );
}

/// **Export is refused, and that is a decision rather than an oversight.**
///
/// Upstream exports both wrappers, so this is a real difference from it. The codec here decodes
/// only. Pure-Rust encoders do exist by name — `oxideav-jpeg2000`, `justjp2`,
/// `openjpeg2-pure-rs` — and all were at 0.0.x or 0.1.x when this was written; a file a user keeps
/// is the wrong place to discover a pre-release encoder was wrong. The refusal is asserted so that
/// adopting one later is a deliberate change to a test, not a silent new behaviour.
#[test]
fn export_is_refused_for_both_wrappers() {
    let editor = Editor::new(Document::new(2, 2).unwrap()).unwrap();
    for format in [FileFormat::Jp2, FileFormat::J2k] {
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

/// A valid signature over a broken body ERRORS; it does not panic and does not return a picture.
///
/// Detection reads a signature, which is all it can do, so these files are correctly claimed and
/// then correctly refused by the decoder. Both wrappers are checked because they enter the decoder
/// by different paths.
#[test]
fn a_valid_signature_over_a_broken_body_is_refused() {
    let mut jp2 = b"\x00\x00\x00\x0CjP  \x0D\x0A\x87\x0A".to_vec();
    jp2.extend_from_slice(&[0xffu8; 40]);
    assert_eq!(detect_format(&jp2).unwrap(), FileFormat::Jp2);
    assert!(import_document(&jp2, &ImportOptions::default()).is_err());

    let mut j2k = vec![0xff, 0x4f, 0xff, 0x51, 0x00];
    j2k.extend_from_slice(&[0x00u8; 40]);
    assert_eq!(detect_format(&j2k).unwrap(), FileFormat::J2k);
    assert!(import_document(&j2k, &ImportOptions::default()).is_err());
}

/// A JP2 presented as a codestream is still refused, because detection runs first.
#[test]
fn the_expected_format_does_not_bypass_detection() {
    let options = ImportOptions::default().with_expected_format(FileFormat::J2k);
    assert!(
        import_document(JP2, &options).is_err(),
        "a container is not a bare codestream"
    );
}
