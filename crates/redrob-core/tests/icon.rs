// SPDX-License-Identifier: GPL-3.0-or-later

//! Windows icon and Apple icon image (M.4).
//!
//! Re-derived from `plug-ins/file-ico/` and `plug-ins/file-icns/` (GPL-3.0-or-later), pinned in
//! `docs/upstream-sources.toml`.
//!
//! The two are opposites on the one question that matters here. **ICNS registers a real magic**
//! (`0,string,icns`). **ICO registers none at all**, and upstream says why in a comment placed
//! exactly where the magic would have gone:
//!
//! > We do not set magics here, since that interferes with certain types of TGA images.
//!
//! An ICO header is `reserved: u16 = 0`, `resource_type: u16 ∈ {1, 2}`, `count: u16`, so it begins
//! `00 00 01 00`. An uncompressed colour-mapped TGA begins `id_length: u8 = 0`,
//! `colour_map_type: u8 = 0`, `image_type: u8 = 1`. **The first three bytes are the same bytes.**
//! Upstream's answer is to not detect ICO by content. This product's answer is to detect it, but
//! strictly after TGA's footer, and to assert which way every case of the collision falls.

use redrob_core::{
    Command, Document, Editor, ExportOptions, FileFormat, FormatError, ImportOptions, Pixel,
    TGA_FOOTER_SIGNATURE, detect_format, export_document, import_document,
};

fn icon(width: u32, height: u32) -> Editor {
    let mut editor = Editor::new(Document::new(width, height).unwrap()).unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel {
                r: 200,
                g: 100,
                b: 50,
                a: 255,
            },
        })
        .unwrap();
    editor
}

/// **The TGA collision, as the triple that pins it.**
///
/// One head — `00 00 01 00` with a count of 1 — and three different verdicts depending on evidence
/// that is nowhere near those four bytes:
///
/// * with a TGA 2.0 footer at the end of the file, it is a **TGA**;
/// * without one, it is an **ICO**;
/// * with a count of zero it is **neither**, which is upstream's own rule — `ico-load.c` sets
///   `icon_count = 0` when the header fails and an icon file with no entries has nothing in it.
///
/// Asserted as a triple rather than three separate tests because the claim is the ORDER, and any
/// one of these passing alone would also pass under a detector that simply always answered the same
/// way. 18 bytes of signature beats four mostly-zero bytes of header; that is the whole argument.
#[test]
fn the_ico_header_collides_with_tga_and_the_footer_wins() {
    let ico_head = [0u8, 0, 1, 0, 1, 0];

    let mut with_footer = ico_head.to_vec();
    with_footer.extend_from_slice(&[0u8; 12]);
    with_footer.extend_from_slice(&[0u8; 8]);
    with_footer.extend_from_slice(TGA_FOOTER_SIGNATURE);
    assert_eq!(
        detect_format(&with_footer).unwrap(),
        FileFormat::Tga,
        "a real TGA signature outweighs an ICO-shaped head"
    );

    let mut without_footer = ico_head.to_vec();
    without_footer.extend_from_slice(&[0u8; 12]);
    assert_eq!(
        detect_format(&without_footer).unwrap(),
        FileFormat::Ico,
        "with no TGA evidence the same head is an icon"
    );

    // Upstream rejects a zero count, so neither format claims this.
    assert!(matches!(
        detect_format(&[0u8, 0, 1, 0, 0, 0]),
        Err(FormatError::UnknownFormat)
    ));
}

/// A CUR is `resource_type == 2` and is not claimed as an icon.
///
/// It shares the container but carries a hotspot where an ICO carries colour planes and bit depth,
/// so decoding one as an icon would read two fields as something they are not. Filed instead.
#[test]
fn a_cursor_is_not_claimed_as_an_icon() {
    assert!(matches!(
        detect_format(&[0u8, 0, 2, 0, 1, 0]),
        Err(FormatError::UnknownFormat)
    ));
}

/// ICO round trip, with the header asserted field by field.
///
/// The round trip is checked FIRST on every format item now, because the last three each shipped a
/// defect where our own export produced a file our own sniff refused — M.1's `unreachable!()`
/// panic, M.2's missing TGA footer, M.3's `P7` — and none of them were visible from reading.
#[test]
fn an_exported_ico_carries_the_header_upstream_validates() {
    let exported = export_document(
        icon(32, 32).document(),
        FileFormat::Ico,
        &ExportOptions::default(),
    )
    .expect("export");
    let bytes = exported.bytes();

    assert_eq!(&bytes[0..2], &[0, 0], "reserved is zero");
    assert_eq!(
        &bytes[2..4],
        &[1, 0],
        "resource_type 1 is an icon, not a cursor"
    );
    assert_eq!(&bytes[4..6], &[1, 0], "one entry");
    assert_eq!(&bytes[6..8], &[32, 32], "and the entry declares its size");

    assert_eq!(detect_format(bytes).unwrap(), FileFormat::Ico);
    let back = import_document(bytes, &ImportOptions::default()).expect("re-import");
    let document = back.document();
    assert_eq!((document.width(), document.height()), (32, 32));
    assert_eq!(&document.layers()[0].pixels()[..4], &[200, 100, 50, 255]);
}

/// ICNS round trip. Unlike ICO this one has a real magic, and it declares its own total length.
#[test]
fn an_exported_icns_declares_its_own_length() {
    let exported = export_document(
        icon(32, 32).document(),
        FileFormat::Icns,
        &ExportOptions::default(),
    )
    .expect("export");
    let bytes = exported.bytes();

    assert_eq!(&bytes[0..4], b"icns");
    let declared = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    assert_eq!(
        declared as usize,
        bytes.len(),
        "the length field counts the whole file, header included"
    );

    assert_eq!(detect_format(bytes).unwrap(), FileFormat::Icns);
    let back = import_document(bytes, &ImportOptions::default()).expect("re-import");
    let document = back.document();
    assert_eq!((document.width(), document.height()), (32, 32));
    assert_eq!(&document.layers()[0].pixels()[..4], &[200, 100, 50, 255]);
}

/// The ICNS length field is CHECKED, so the word alone does not win.
///
/// Four bytes of text is weak evidence on its own; a declared length shorter than the header it
/// sits in is proof the file is not one.
#[test]
fn the_icns_magic_alone_is_not_enough() {
    assert!(matches!(
        detect_format(b"icns\x00\x00\x00\x04"),
        Err(FormatError::UnknownFormat)
    ));
    assert_eq!(
        detect_format(b"icns\x00\x00\x00\x08").unwrap(),
        FileFormat::Icns
    );
}

/// **An icon family cannot hold an arbitrary size**, and the refusal is passed through.
///
/// Every ICNS element type declares fixed dimensions, so a 2x1 canvas has nowhere to go. Rescaling
/// the user's document to make the export succeed would be a worse answer than refusing, so the
/// refusal is the asserted behaviour — and it is contrasted against a 32x32 that works, because a
/// test that only checked the failure would also pass if ICNS export were broken outright.
#[test]
fn icns_export_refuses_a_size_the_format_has_no_slot_for() {
    let refused = export_document(
        icon(2, 1).document(),
        FileFormat::Icns,
        &ExportOptions::default(),
    );
    assert!(refused.is_err(), "2x1 is not an icon size");

    assert!(
        export_document(
            icon(32, 32).document(),
            FileFormat::Icns,
            &ExportOptions::default()
        )
        .is_ok(),
        "but 32x32 is"
    );
}

/// A PNG presented as either icon format is refused rather than decoded as what it really is.
#[test]
fn a_png_presented_as_an_icon_is_refused() {
    let png = export_document(
        icon(32, 32).document(),
        FileFormat::Png,
        &ExportOptions::default(),
    )
    .expect("png")
    .into_bytes();

    for format in [FileFormat::Ico, FileFormat::Icns] {
        let options = ImportOptions::default().with_expected_format(format);
        assert!(import_document(&png, &options).is_err(), "{format:?}");
    }
}
