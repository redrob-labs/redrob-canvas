// SPDX-License-Identifier: GPL-3.0-or-later

//! BMP (M.1).
//!
//! Re-derived from `plug-ins/file-bmp/bmp-load.c` and `bmp-export.c` (GPL-3.0-or-later), pinned in
//! `docs/upstream-sources.toml`. The codec is `image`'s pure-Rust BMP support, which group M's own
//! policy prefers ("look for a pure-Rust codec first"); what is ported is the SNIFF, the wiring and
//! the policy, and what these tests do is check the codec against the facts the upstream loader
//! states — the three that are easiest to assume wrongly.

use redrob_core::{
    Command, Document, Editor, ExportOptions, FileFormat, FormatError, ImportOptions, Pixel,
    detect_format, export_document, import_document,
};

/// A `BITMAPINFOHEADER` BMP. `height` is SIGNED on purpose: that is the whole of test 3.
fn bmp(width: i32, height: i32, bits: u16, rows: &[u8]) -> Vec<u8> {
    let header = 14u32 + 40;
    let mut out = Vec::new();
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(header + rows.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&header.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // BI_RGB, so the DEFAULT masks apply
    out.extend_from_slice(&(rows.len() as u32).to_le_bytes());
    for _ in 0..4 {
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    out.extend_from_slice(rows);
    out
}

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

/// Only `BM` is sniffed, and a file too short to hold a header is not claimed.
///
/// Upstream accepts six signatures — `BM` plus the OS/2 `BA`, `IC`, `PT`, `CI`, `CP` — but only
/// `BM` is a standalone bitmap; `BA` in particular is a CONTAINER whose loader loops over an array
/// until it reaches a `BM`. Claiming those here would decode the wrong thing, so they are left to
/// report as unknown.
#[test]
fn only_a_long_enough_bm_signature_is_claimed() {
    let real = bmp(1, 1, 24, &[0, 0, 0, 0]);
    assert_eq!(detect_format(&real).unwrap(), FileFormat::Bmp);

    // Two bytes is a signature with no header behind it.
    assert!(matches!(
        detect_format(b"BM"),
        Err(FormatError::UnknownFormat)
    ));
    // The OS/2 array container is deliberately not claimed.
    let mut array = bmp(1, 1, 24, &[0, 0, 0, 0]);
    array[0] = b'B';
    array[1] = b'A';
    assert!(matches!(
        detect_format(&array),
        Err(FormatError::UnknownFormat)
    ));
}

/// **A NEGATIVE `biHeight` means the rows are stored top-down.**
///
/// `fi.height = ABS (bitmap_head.biHeight)`, and then
/// `if (bitmap_head.biHeight < 0) gimp_image_flip (image, GIMP_ORIENTATION_VERTICAL)` — so the
/// default, positive, is BOTTOM-UP.
///
/// One assertion about the pair rather than two about the files: the same payload with the sign
/// flipped must decode to vertically MIRRORED images. An implementation that took the absolute
/// value and forgot the flip would return the same image twice, and an implementation that always
/// flipped would also return the same image twice — so comparing the two is what pins it.
#[test]
fn a_negative_height_stores_the_rows_top_down() {
    // 2x2 at 24 bpp. Stride = ((2 * 24 + 31) / 32) * 4 = 8, so each row is 6 bytes plus 2 padding.
    // Stored row 0 is red then green; stored row 1 is blue then white. BMP is BGR.
    let rows = [
        0, 0, 255, 0, 255, 0, 0, 0, // red, green, pad
        255, 0, 0, 255, 255, 255, 0, 0, // blue, white, pad
    ];

    let (_, _, bottom_up) = decode(&bmp(2, 2, 24, &rows));
    let (_, _, top_down) = decode(&bmp(2, -2, 24, &rows));

    // Bottom-up: the LAST stored row is the top one, so the top-left pixel is blue.
    assert_eq!(&bottom_up[..4], &[0, 0, 255, 255]);
    // Top-down: the FIRST stored row is the top one, so the top-left pixel is red.
    assert_eq!(&top_down[..4], &[255, 0, 0, 255]);
    // And the two are exact mirrors: row 0 of one is row 1 of the other.
    assert_eq!(&bottom_up[..8], &top_down[8..16]);
    assert_eq!(&bottom_up[8..16], &top_down[..8]);
}

/// **The default 16-bit layout is 5-5-5, not 5-6-5.**
///
/// Under `BI_RGB` the loader installs `masks[] = 0x00007c00, 0x000003e0, 0x0000001f` for 16 bpp;
/// 5-6-5 appears only through `BI_BITFIELDS`. This is the single most commonly mis-assumed BMP
/// fact, so both wrong values are named: `0x03E0` is FULL green under 555 and about **124** under
/// 565, and `0x7C00` is full red under 555 and about **123** under 565.
#[test]
fn the_default_sixteen_bit_layout_is_five_five_five() {
    // 2x1 at 16 bpp. Stride = ((2 * 16 + 31) / 32) * 4 = 4, which is exactly the two pixels.
    let rows = [0xE0, 0x03, 0x00, 0x7C];
    let (width, height, pixels) = decode(&bmp(2, 1, 16, &rows));
    assert_eq!((width, height), (2, 1));

    assert_eq!(
        &pixels[..4],
        &[0, 255, 0, 255],
        "0x03E0 is full green under 555; 565 would give about 124"
    );
    assert_eq!(
        &pixels[4..8],
        &[255, 0, 0, 255],
        "0x7C00 is full red under 555; 565 would give about 123"
    );
}

/// **Rows are padded to a 4-byte stride**, `((width * bits + 31) / 32) * 4`.
///
/// A 3-wide 24-bit row is 9 bytes of colour padded to **12**. The three pixels are all different,
/// so an implementation that packed rows tightly would read the second row's bytes into the first
/// and every pixel after the first would be wrong.
#[test]
fn rows_are_padded_to_a_four_byte_stride() {
    // 3x2 at 24 bpp, stride 12: 9 colour bytes + 3 padding, twice.
    let rows = [
        // bottom row: red, green, blue
        0, 0, 255, 0, 255, 0, 255, 0, 0, 0, 0, 0, //
        // top row: white, black, white
        255, 255, 255, 0, 0, 0, 255, 255, 255, 0, 0, 0,
    ];
    let (width, height, pixels) = decode(&bmp(3, 2, 24, &rows));
    assert_eq!((width, height), (3, 2));

    // Top row first (bottom-up storage), then the colour row.
    assert_eq!(
        &pixels[..12],
        &[255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 255]
    );
    assert_eq!(
        &pixels[12..24],
        &[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255]
    );
}

/// Alpha survives a round trip, because upstream writes 32 bpp for an RGBA image.
///
/// `bmp-export.c`'s `GIMP_RGBA_IMAGE` case sets `BitsPerPixel = 32` and `RGBA_8888`, so BMP is not
/// an opaque-only format and must not be treated as one. Carrying alpha needs the channel masks,
/// which needs a header larger than `BITMAPINFOHEADER` — the export writes a **108-byte
/// `BITMAPV4HEADER`**, asserted from the header-size field rather than from the file length so the
/// claim is structural.
#[test]
fn alpha_survives_the_round_trip_in_a_thirty_two_bit_bmp() {
    let color = Pixel {
        r: 200,
        g: 100,
        b: 50,
        a: 128,
    };
    let editor = filled(2, 2, color);
    let exported = export_document(
        editor.document(),
        FileFormat::Bmp,
        &ExportOptions::default(),
    )
    .expect("export");

    let bytes = exported.bytes();
    assert_eq!(&bytes[..2], b"BM");
    let header_size = u32::from_le_bytes([bytes[14], bytes[15], bytes[16], bytes[17]]);
    assert_eq!(header_size, 108, "BITMAPV4HEADER, which the masks need");

    let (width, height, pixels) = decode(bytes);
    assert_eq!((width, height), (2, 2));
    assert_eq!(&pixels[..4], &[200, 100, 50, 128]);
}

/// An opaque export round-trips byte-exactly, and BMP is reported as LOSSLESS.
///
/// Not lossy like JPEG or DDS: a 24- or 32-bit BMP stores the samples it was given. Saying
/// otherwise would invite a caller to treat an archival copy as an approximation.
#[test]
fn an_opaque_bmp_round_trips_exactly_and_is_reported_lossless() {
    let color = Pixel {
        r: 10,
        g: 20,
        b: 30,
        a: 255,
    };
    let editor = filled(3, 1, color);
    let exported = export_document(
        editor.document(),
        FileFormat::Bmp,
        &ExportOptions::default(),
    )
    .expect("export");

    assert!(
        exported.metadata().lossless,
        "a BMP stores the samples it was given"
    );
    assert_eq!(detect_format(exported.bytes()).unwrap(), FileFormat::Bmp);

    let (width, height, pixels) = decode(exported.bytes());
    assert_eq!((width, height), (3, 1));
    for chunk in pixels.chunks_exact(4) {
        assert_eq!(chunk, &[10, 20, 30, 255]);
    }
}

/// A file whose signature says BMP but whose body is another format is refused by the format
/// check rather than decoded as whatever it really is.
#[test]
fn a_format_mismatch_is_refused() {
    let png = {
        let editor = filled(
            1,
            1,
            Pixel {
                r: 1,
                g: 2,
                b: 3,
                a: 255,
            },
        );
        export_document(
            editor.document(),
            FileFormat::Png,
            &ExportOptions::default(),
        )
        .expect("png")
        .bytes()
        .to_vec()
    };
    let options = ImportOptions::default().with_expected_format(FileFormat::Bmp);
    assert!(import_document(&png, &options).is_err());
}
