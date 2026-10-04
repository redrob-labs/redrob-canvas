// SPDX-License-Identifier: GPL-3.0-or-later

//! Silicon Graphics image (M.7, first of four).
//!
//! Re-derived from `plug-ins/file-sgi/sgi-lib.c` and `plug-ins/file-sgi/sgi.c`
//! (GPL-3.0-or-later), pinned in `docs/upstream-sources.toml`. No pure-Rust SGI codec exists on
//! the registry, so the codec is written in `crate::sgi`, which carries the full header table.
//!
//! M.7 lists four formats — SGI, SUN raster, XPM, XBM — and the loop rules say a multi-part item
//! is done one part per cycle, with the item line ticked only when every part has passed. This is
//! the first part; the other three stay open.

use redrob_core::{
    Document, Editor, ExportOptions, FileFormat, FormatError, ImportOptions, Pixel, detect_format,
    export_document, import_document,
};

#[path = "common/canvas.rs"]
mod canvas;

const SGI_MAGIC: u16 = 474;

fn export(editor: &Editor) -> Vec<u8> {
    export_document(
        editor.document(),
        FileFormat::Sgi,
        &ExportOptions::default(),
    )
    .expect("export")
    .into_bytes()
}

/// A 512-byte SGI header with the fields this test file needs, so the RLE cases below read as the
/// byte layout they are rather than as an opaque fixture.
fn header(comp: u8, bpp: u8, width: u16, height: u16, channels: u16) -> Vec<u8> {
    let mut bytes = vec![0u8; 512];
    bytes[0..2].copy_from_slice(&SGI_MAGIC.to_be_bytes());
    bytes[2] = comp;
    bytes[3] = bpp;
    bytes[4..6].copy_from_slice(&3u16.to_be_bytes());
    bytes[6..8].copy_from_slice(&width.to_be_bytes());
    bytes[8..10].copy_from_slice(&height.to_be_bytes());
    bytes[10..12].copy_from_slice(&channels.to_be_bytes());
    bytes
}

/// The export round-trips, and the header is asserted field by field against upstream's table.
///
/// Two rows with different colours, so a vertical flip cannot pass: SGI stores rows BOTTOM-UP —
/// upstream fills output row `y` from stored row `ysize - 1 - y` — and channels as separate
/// PLANES, so both conventions are live in this one assertion.
#[test]
fn the_export_round_trips_through_the_bottom_up_planar_layout() {
    let top = Pixel {
        r: 10,
        g: 20,
        b: 30,
        a: 255,
    };
    let bottom = Pixel {
        r: 200,
        g: 100,
        b: 50,
        a: 128,
    };
    let editor = canvas::editor(1, 2, &[top, bottom]);
    let bytes = export(&editor);

    assert_eq!(u16::from_be_bytes([bytes[0], bytes[1]]), SGI_MAGIC);
    assert_eq!(bytes[2], 0, "uncompressed");
    assert_eq!(bytes[3], 1, "bpp counts BYTES per channel, not bits");
    assert_eq!(u16::from_be_bytes([bytes[6], bytes[7]]), 1, "xsize");
    assert_eq!(u16::from_be_bytes([bytes[8], bytes[9]]), 2, "ysize");
    assert_eq!(u16::from_be_bytes([bytes[10], bytes[11]]), 4, "zsize, RGBA");
    assert_eq!(
        bytes.len(),
        // 512-byte header, then width x height x channels bytes: 1 x 2 x 4 = 8.
        512 + 8,
        "512-byte header then the planes"
    );

    assert_eq!(detect_format(&bytes).unwrap(), FileFormat::Sgi);
    let back = import_document(&bytes, &ImportOptions::default()).expect("re-import");
    let document = back.document();
    assert_eq!((document.width(), document.height()), (1, 2));
    assert_eq!(
        &document.layers()[0].pixels()[..8],
        &[10, 20, 30, 255, 200, 100, 50, 128],
        "the top row must come back on top"
    );
}

/// **Either byte order of the magic is accepted, and that is a deliberate divergence.**
///
/// `sgiOpen` reads the magic big-endian and, on a mismatch, swaps the two bytes and retries,
/// setting `swapBytes` for the rest of the file. But upstream *registers* only `0,short,474`, so
/// **its own content detection cannot find a little-endian SGI that its loader reads perfectly
/// well.** Both orders are claimed here.
///
/// The third case is the control: two bytes that are neither order of 474 must not be claimed, so
/// this cannot pass under a detector that simply says yes.
#[test]
fn the_magic_is_accepted_in_either_byte_order() {
    let bytes = export(&canvas::editor(
        1,
        1,
        &[Pixel {
            r: 1,
            g: 2,
            b: 3,
            a: 255,
        }],
    ));
    assert_eq!(&bytes[..2], &[0x01, 0xda], "474 is 0x01DA");
    assert_eq!(detect_format(&bytes).unwrap(), FileFormat::Sgi);

    let mut swapped = bytes.clone();
    swapped.swap(0, 1);
    assert_eq!(&swapped[..2], &[0xda, 0x01]);
    assert_eq!(
        detect_format(&swapped).unwrap(),
        FileFormat::Sgi,
        "upstream's loader reads this and its own magic cannot find it"
    );

    let mut neither = bytes.clone();
    neither[0] = 0x02;
    neither[1] = 0x02;
    assert!(matches!(
        detect_format(&neither),
        Err(FormatError::UnknownFormat)
    ));
}

/// **The RLE control bit is INVERTED relative to TGA's**, and this is the test that pins it.
///
/// Here `count = control & 127`; bit 7 SET means `count` literal bytes follow, CLEAR means ONE
/// byte follows and repeats `count` times. TGA — implemented in M.2, four items earlier in this
/// same group — reads the high bit the other way round. Getting it backwards still produces a
/// picture, just the wrong one, so the row below carries BOTH kinds of run and the expected output
/// differs under either polarity:
///
/// * `0x82` then `11, 22` — literal, so two distinct samples;
/// * `0x02` then `33` — repeat, so the same sample twice.
///
/// Under TGA's polarity the first would repeat `11` and the second would read `33` as a literal,
/// giving `[11, 11, 33, ...]` instead of `[11, 22, 33, 33]`.
#[test]
fn the_rle_control_bit_is_inverted_relative_to_tga() {
    let mut bytes = header(1, 1, 4, 1, 1);
    // One row, one channel: a 4-byte offset table then a 4-byte length table.
    bytes.extend_from_slice(&520u32.to_be_bytes());
    bytes.extend_from_slice(&5u32.to_be_bytes());
    assert_eq!(
        bytes.len(),
        520,
        "the row begins where the table says it does"
    );
    bytes.extend_from_slice(&[0x82, 11, 22, 0x02, 33]);

    assert_eq!(detect_format(&bytes).unwrap(), FileFormat::Sgi);
    let back = import_document(&bytes, &ImportOptions::default()).expect("import");
    let document = back.document();
    assert_eq!((document.width(), document.height()), (4, 1));

    let greys: Vec<u8> = document.layers()[0]
        .pixels()
        .chunks_exact(4)
        .map(|pixel| pixel[0])
        .collect();
    assert_eq!(
        greys,
        vec![11, 22, 33, 33],
        "bit 7 set is a LITERAL run here; TGA would give [11, 11, 33, ...]"
    );

    // A single plane is GREY, so it feeds all three colour channels and alpha stays opaque.
    for pixel in document.layers()[0].pixels().chunks_exact(4) {
        assert_eq!(pixel[0], pixel[1], "grey: r == g");
        assert_eq!(pixel[1], pixel[2], "grey: g == b");
        assert_eq!(pixel[3], 255, "no alpha plane means opaque");
    }
}

/// `comp` and `bpp` are checked as upstream's loader checks them.
///
/// Upstream refuses `bpp` outside 1..=2 outright, and knows only three compression modes. A
/// two-byte magic is thin evidence, so these fields carry part of the weight — and `bpp == 2`
/// is asserted to still DETECT while the decoder refuses it, which is the honest split: the file
/// is an SGI, this product just cannot read 16-bit samples yet.
#[test]
fn the_header_fields_upstream_validates_are_validated() {
    let valid = export(&canvas::editor(
        1,
        1,
        &[Pixel {
            r: 1,
            g: 2,
            b: 3,
            a: 255,
        }],
    ));

    let mut bad_bpp = valid.clone();
    bad_bpp[3] = 3;
    assert!(matches!(
        detect_format(&bad_bpp),
        Err(FormatError::UnknownFormat)
    ));

    let mut bad_comp = valid.clone();
    bad_comp[2] = 9;
    assert!(matches!(
        detect_format(&bad_comp),
        Err(FormatError::UnknownFormat)
    ));

    // 16-bit is a legal SGI and is detected; only the decode refuses.
    let mut deep = valid.clone();
    deep[3] = 2;
    assert_eq!(detect_format(&deep).unwrap(), FileFormat::Sgi);
    assert!(
        import_document(&deep, &ImportOptions::default()).is_err(),
        "refused rather than silently narrowed to 8 bits"
    );
}

/// **A channel count above four is CLAMPED, not rejected** — upstream's own leniency, kept.
///
/// `sgiOpen` warns and sets `zsize = 4` rather than failing, on the reasoning that a file
/// overstating its channels still has four readable ones.
#[test]
fn an_overstated_channel_count_is_clamped_like_upstream() {
    let mut bytes = header(0, 1, 1, 1, 7);
    bytes.extend_from_slice(&[9u8; 7]);

    assert_eq!(detect_format(&bytes).unwrap(), FileFormat::Sgi);
    let back = import_document(&bytes, &ImportOptions::default()).expect("clamped, not refused");
    assert_eq!((back.document().width(), back.document().height()), (1, 1));
}

/// A PNG presented as an SGI is refused rather than decoded as what it really is.
#[test]
fn a_png_presented_as_an_sgi_is_refused() {
    let png = export_document(
        canvas::editor(
            1,
            1,
            &[Pixel {
                r: 1,
                g: 2,
                b: 3,
                a: 255,
            }],
        )
        .document(),
        FileFormat::Png,
        &ExportOptions::default(),
    )
    .expect("png")
    .into_bytes();

    let options = ImportOptions::default().with_expected_format(FileFormat::Sgi);
    assert!(import_document(&png, &options).is_err());
}

/// A document too large for the 16-bit dimension fields is not silently truncated.
#[test]
fn dimensions_beyond_the_header_fields_are_not_wrapped() {
    // 65536 would wrap to 0 in a u16. Only run this if such a document is constructible.
    if let Ok(document) = Document::new(65_536, 1) {
        let result = export_document(&document, FileFormat::Sgi, &ExportOptions::default());
        if let Ok(outcome) = result {
            let bytes = outcome.into_bytes();
            assert_ne!(
                u16::from_be_bytes([bytes[6], bytes[7]]),
                0,
                "a wrapped xsize would claim an empty image"
            );
        }
    }
}
