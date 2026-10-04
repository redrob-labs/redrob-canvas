// SPDX-License-Identifier: GPL-3.0-or-later

//! FITS (M.11b, second of M.11's two — this completes M.11).
//!
//! Re-derived from `plug-ins/file-fits/fits.c` (GPL-3.0-or-later), pinned in
//! `docs/upstream-sources.toml`. `crate::fits` marks which of its facts come from upstream and
//! which from the FITS specification — upstream delegates parsing to **CFITSIO**, so the card and
//! block layout is not visible in its source and is not presented as a re-derivation.

use redrob_core::{FileFormat, ImportOptions, detect_format, import_document};

/// Build a FITS: 80-column cards, `END`, padded to a 2880-byte block, then the data.
fn fits(cards: &[(&str, i64)], body: &[u8]) -> Vec<u8> {
    let mut header = format!("{:<8}= {:>20}{:<50}", "SIMPLE", "T", "");
    for (key, value) in cards {
        header.push_str(&format!("{key:<8}= {value:>20}{:<50}", ""));
    }
    header.push_str(&format!("{:<80}", "END"));

    let mut out = header.into_bytes();
    while out.len() % 2880 != 0 {
        out.push(b' ');
    }
    out.extend_from_slice(body);
    out
}

fn decode(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let outcome = import_document(bytes, &ImportOptions::default()).expect("import");
    let document = outcome.document();
    (
        document.width(),
        document.height(),
        document.layers()[0].pixels().to_vec(),
    )
}

fn refusal(bytes: &[u8]) -> String {
    import_document(bytes, &ImportOptions::default())
        .expect_err("refused")
        .to_string()
}

/// The magic is `SIMPLE` at offset 0, and the greys come back as stored.
#[test]
fn a_grey_fits_decodes() {
    let grey = fits(
        &[("BITPIX", 8), ("NAXIS", 2), ("NAXIS1", 2), ("NAXIS2", 2)],
        &[10, 20, 30, 40],
    );
    assert!(grey.starts_with(b"SIMPLE"));
    assert_eq!(detect_format(&grey).unwrap(), FileFormat::Fits);

    let (width, height, pixels) = decode(&grey);
    assert_eq!((width, height), (2, 2));
    assert_eq!(&pixels[..8], &[10, 10, 10, 255, 20, 20, 20, 255]);
}

/// **`BITPIX` IS SIGNED, and the refusal MESSAGE is what proves it was parsed.**
///
/// A negative value means floating point — `-32` is IEEE single, `-64` double. The naive mistake is
/// to read the field as unsigned, and the subtle part is that an unsigned read would ALSO refuse
/// `-32`, just for the wrong reason: the parse would fail and the error would say the file declares
/// no `BITPIX` at all. So this asserts WHICH refusal comes back. A test that only checked
/// "`-32` is refused" would pass either way.
#[test]
fn bitpix_is_parsed_as_signed_and_negative_means_float() {
    let one_pixel = |bitpix: i64| {
        fits(
            &[
                ("BITPIX", bitpix),
                ("NAXIS", 2),
                ("NAXIS1", 1),
                ("NAXIS2", 1),
            ],
            &[0u8; 16],
        )
    };

    // 8 is the one that decodes.
    assert!(import_document(&one_pixel(8), &ImportOptions::default()).is_ok());

    for bitpix in [-32i64, -64] {
        let message = refusal(&one_pixel(bitpix));
        assert!(
            message.contains("BZERO"),
            "BITPIX {bitpix} must be READ and then refused for scaling, not fail to parse: {message}"
        );
        assert!(
            !message.contains("declares no BITPIX"),
            "an unsigned parse would give this instead: {message}"
        );
    }

    // The positive non-8 values are refused for the same scaling reason.
    for bitpix in [16i64, 32] {
        assert!(refusal(&one_pixel(bitpix)).contains("BZERO"));
    }
}

/// **THE AXIS SHIFT: when the FIRST axis is 3, the dimensions are the SECOND and THIRD.**
///
/// Upstream: `if (hdu.naxisn[0] == 3) { width = hdu.naxisn[1]; height = hdu.naxisn[2]; }`. So RGB
/// can be stored `(w, h, 3)` or `(3, w, h)`, and the second form moves the dimensions along.
///
/// `(3, 2, 4)` is the discriminating case: read naively — first axis as width, second as height —
/// it would be **3 wide and 2 tall**. With the shift it is **2 wide and 4 tall**. Both forms are
/// asserted to give the same image, which is the actual claim: the storage order is not supposed to
/// change the picture.
#[test]
fn the_first_axis_being_three_shifts_the_dimensions() {
    // Three PLANES of 2x4 -- the third axis is the slowest, so R, then G, then B.
    let mut body = Vec::new();
    for channel in 0..3u8 {
        for index in 0..8u8 {
            body.push(channel * 100 + index);
        }
    }

    let shifted = fits(
        &[
            ("BITPIX", 8),
            ("NAXIS", 3),
            ("NAXIS1", 3),
            ("NAXIS2", 2),
            ("NAXIS3", 4),
        ],
        &body,
    );
    let (width, height, pixels) = decode(&shifted);
    assert_eq!(
        (width, height),
        (2, 4),
        "(3, 2, 4) is 2 wide and 4 tall; read naively it would be 3x2"
    );
    // Planar, so the first pixel takes one sample from each plane.
    assert_eq!(&pixels[..8], &[0, 100, 200, 255, 1, 101, 201, 255]);

    let plain = fits(
        &[
            ("BITPIX", 8),
            ("NAXIS", 3),
            ("NAXIS1", 2),
            ("NAXIS2", 4),
            ("NAXIS3", 3),
        ],
        &body,
    );
    let (plain_width, plain_height, plain_pixels) = decode(&plain);
    assert_eq!((plain_width, plain_height), (2, 4));
    assert_eq!(
        plain_pixels, pixels,
        "both storage orders are the same picture"
    );
}

/// The data begins at the next 2880-byte boundary past `END`, not immediately after it.
///
/// Six cards is 480 bytes, so the gap between `END` and the data is 2400 bytes of padding. Reading
/// the data straight after `END` would pick up the padding — spaces, `0x20` — so the greys would
/// come back as 32 rather than as the samples.
#[test]
fn the_data_starts_at_the_next_block_boundary() {
    let grey = fits(
        &[("BITPIX", 8), ("NAXIS", 2), ("NAXIS1", 2), ("NAXIS2", 1)],
        &[77, 88],
    );
    assert_eq!(grey.len(), 2880 + 2, "header padded to one whole block");

    let (_, _, pixels) = decode(&grey);
    assert_eq!(
        &pixels[..8],
        &[77, 77, 77, 255, 88, 88, 88, 255],
        "not 32, which is the space the padding is made of"
    );
}

/// Fewer than two axes is refused — upstream skips such an HDU rather than guessing.
#[test]
fn fewer_than_two_axes_is_refused() {
    let one_d = fits(&[("BITPIX", 8), ("NAXIS", 1), ("NAXIS1", 4)], &[1, 2, 3, 4]);
    assert_eq!(detect_format(&one_d).unwrap(), FileFormat::Fits);
    assert!(refusal(&one_d).contains("two axes"));
}

/// The keyword search is anchored to the first eight columns, so `NAXIS` does not match `NAXIS1`.
///
/// The cards are deliberately out of order here, with `NAXIS1` written BEFORE `NAXIS`: a search
/// that scanned anywhere in the card, or took the first card whose text began with the keyword,
/// would read 2 as the axis count and then look for a third axis that is not there.
#[test]
fn the_keyword_search_is_anchored_to_the_card_columns() {
    let reordered = fits(
        &[("NAXIS1", 2), ("NAXIS2", 2), ("BITPIX", 8), ("NAXIS", 2)],
        &[1, 2, 3, 4],
    );
    let (width, height, pixels) = decode(&reordered);
    assert_eq!((width, height), (2, 2));
    assert_eq!(&pixels[..4], &[1, 1, 1, 255]);
}

/// Export is refused by name.
#[test]
fn export_is_refused_by_name() {
    use redrob_core::{Document, Editor, ExportOptions, export_document};

    let editor = Editor::new(Document::new(2, 2).unwrap()).unwrap();
    assert!(
        export_document(
            editor.document(),
            FileFormat::Fits,
            &ExportOptions::default()
        )
        .is_err(),
        "what an exported sample would MEAN is a scientific-data decision, not a format one"
    );
}
