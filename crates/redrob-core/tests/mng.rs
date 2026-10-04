// SPDX-License-Identifier: GPL-3.0-or-later

//! MNG (M.12) — **detected, not decoded.** The last item in group M.
//!
//! Re-derived from `plug-ins/common/file-mng.c` (GPL-3.0-or-later), pinned in
//! `docs/upstream-sources.toml`. Upstream registers `0,string,\212MNG\r\n\032\n`.
//!
//! **Decoding is refused and the search was done**: there is no pure-Rust MNG codec on the
//! registry — `mng 0.0.0` is a mail-server API — and no libmng bindings. Upstream delegates to
//! **libmng** and admits its limits in its own header comment: *"Since libmng cannot write PNG,
//! JNG and delta PNG chunks at this time"*. MNG is a full animation and compositing container —
//! delta-PNG frames, embedded JNG, chunk-level object and loop models — for a format **superseded
//! by two this product already supports**, APNG and animated WebP.

use redrob_core::{
    Document, Editor, ExportOptions, FileFormat, FormatError, ImportOptions, detect_format,
    export_document, import_document,
};

/// The three signatures of the family, each with its real first chunk following.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\x0dIHDR";
const MNG: &[u8] = b"\x8aMNG\r\n\x1a\n\x00\x00\x00\x1cMHDR";
const JNG: &[u8] = b"\x8bJNG\r\n\x1a\n\x00\x00\x00\x10JHDR";

/// **THE SIGNATURE FAMILY, as a matrix — because PNG and MNG are one bit and three letters apart.**
///
/// `89 50 4E 47 0D 0A 1A 0A` against `8A 4D 4E 47 0D 0A 1A 0A`. The structure is deliberate: MNG,
/// PNG and JNG were designed as a family, so their signatures differ only in the leading byte and
/// the three letters, and everything after is identical.
///
/// Four cases, and the last two are the ones that matter:
///
/// | bytes          | verdict        |
/// |----------------|----------------|
/// | `89` + `PNG`   | **Png**        |
/// | `8A` + `MNG`   | **Mng**        |
/// | `89` + `MNG`   | **neither**    |
/// | `8A` + `PNG`   | **neither**    |
///
/// The crosses prove the leading byte AND the letters are both required. A sniff that checked only
/// the letters, or only the byte, would pass the first two and fail here — which is exactly the
/// mistake the two formats' similarity invites.
#[test]
fn the_png_family_signatures_do_not_claim_each_other() {
    assert_eq!(detect_format(PNG).unwrap(), FileFormat::Png);
    assert_eq!(detect_format(MNG).unwrap(), FileFormat::Mng);

    let mut png_bytes_mng_letters = MNG.to_vec();
    png_bytes_mng_letters[0] = 0x89;
    assert!(
        matches!(
            detect_format(&png_bytes_mng_letters),
            Err(FormatError::UnknownFormat)
        ),
        "PNG's leading byte with MNG's letters is neither"
    );

    let mut mng_byte_png_letters = PNG.to_vec();
    mng_byte_png_letters[0] = 0x8a;
    assert!(
        matches!(
            detect_format(&mng_byte_png_letters),
            Err(FormatError::UnknownFormat)
        ),
        "MNG's leading byte with PNG's letters is neither"
    );
}

/// JNG is the third of the family and is claimed by NEITHER.
///
/// MNG may embed JNG streams, but upstream ships no standalone JNG procedure — its MNG plug-in
/// handles JNG as a chunk type inside a container. So a bare JNG is correctly unrecognised rather
/// than being quietly folded into MNG.
#[test]
fn a_bare_jng_is_not_claimed() {
    assert!(matches!(
        detect_format(JNG),
        Err(FormatError::UnknownFormat)
    ));
}

/// The whole eight bytes are required: a near-miss in the tail is not an MNG.
#[test]
fn the_tail_of_the_signature_is_checked_too() {
    for position in [4usize, 5, 6, 7] {
        let mut broken = MNG.to_vec();
        broken[position] = b'X';
        assert!(
            matches!(detect_format(&broken), Err(FormatError::UnknownFormat)),
            "byte {position} is part of the signature"
        );
    }
}

/// **Detected, then refused BY NAME** — which is strictly better than being unrecognised.
///
/// The message says what is missing rather than that the file is broken, because the file is fine
/// and this product is the part that is incomplete.
#[test]
fn decoding_is_refused_by_name() {
    assert_eq!(detect_format(MNG).unwrap(), FileFormat::Mng);

    let refused = import_document(MNG, &ImportOptions::default()).expect_err("refused");
    let message = refused.to_string();
    assert!(message.contains("MNG"), "{message}");
    assert!(
        message.contains("does not decode"),
        "the refusal must say what is missing, not imply the file is bad: {message}"
    );
}

/// Export is refused by name too.
#[test]
fn export_is_refused_by_name() {
    let editor = Editor::new(Document::new(2, 2).unwrap()).unwrap();
    let refused = export_document(
        editor.document(),
        FileFormat::Mng,
        &ExportOptions::default(),
    );
    assert!(matches!(
        refused,
        Err(redrob_core::CoreError::Format(
            FormatError::UnsupportedFeature(_)
        ))
    ));
}

/// The expected format does not bypass detection: a PNG is not an MNG however it is labelled.
///
/// This one matters more here than for most formats, because the two signatures are so close that a
/// caller mislabelling one as the other is a realistic mistake rather than a contrived one.
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

    let options = ImportOptions::default().with_expected_format(FileFormat::Mng);
    assert!(import_document(&png, &options).is_err());
}
