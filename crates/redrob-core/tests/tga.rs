// SPDX-License-Identifier: GPL-3.0-or-later

//! Truevision TGA (M.2).
//!
//! Re-derived from `plug-ins/common/file-tga.c` (GPL-3.0-or-later), pinned in
//! `docs/upstream-sources.toml`. The codec is `image`'s pure-Rust TGA support, which group M's own
//! policy prefers; what is ported is the footer sniff, the wiring and the policy, and what these
//! tests do is check the codec against the facts the upstream loader states.

use redrob_core::{
    Command, Document, Editor, ExportOptions, FileFormat, FormatError, ImportOptions, Pixel,
    TGA_FOOTER_SIGNATURE, detect_format, export_document, import_document,
};

/// A TGA 1.0 stream — header plus body, no footer. `descriptor` is header byte 17.
fn tga(image_type: u8, width: u16, height: u16, bpp: u8, descriptor: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(0); // idLength
    out.push(0); // colorMapType
    out.push(image_type);
    out.extend_from_slice(&[0, 0]); // colorMapIndex
    out.extend_from_slice(&[0, 0]); // colorMapLength
    out.push(0); // colorMapSize
    out.extend_from_slice(&[0, 0]); // xOrigin
    out.extend_from_slice(&[0, 0]); // yOrigin
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.push(bpp);
    out.push(descriptor);
    out.extend_from_slice(body);
    out
}

/// The 26-byte TGA 2.0 footer: two zero offsets then the signature.
fn with_footer(mut bytes: Vec<u8>) -> Vec<u8> {
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(TGA_FOOTER_SIGNATURE);
    bytes
}

fn decode(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let outcome = import_document(bytes, &ImportOptions::default()).expect("import");
    let document = outcome.document();
    let pixels = document.layers()[0].pixels().to_vec();
    (document.width(), document.height(), pixels)
}

/// 2x2 uncompressed 24-bit BGR. Stored row 0 is red then green; row 1 is blue then white.
const BODY_24: [u8; 12] = [
    0, 0, 255, 0, 255, 0, // red, green
    255, 0, 0, 255, 255, 255, // blue, white
];

/// **TGA's signature is at the END of the file, and a TGA 1.0 has none at all.**
///
/// Upstream registers the magic as `-18&,string,TRUEVISION-XFILE.,-1,byte,0` and its loader checks
/// `memcmp (footer + 8, magic, 18)` after reading 26 bytes from the end. So content detection can
/// only ever see the optional 2.0 footer; a footerless file reaches upstream's loader by file
/// EXTENSION, which is not a thing this product's strict-detection import has.
#[test]
fn the_signature_is_the_footer_and_a_footerless_tga_has_none() {
    assert_eq!(TGA_FOOTER_SIGNATURE, b"TRUEVISION-XFILE.\0");

    let bare = tga(2, 2, 2, 24, 0x00, &BODY_24);
    assert!(
        matches!(detect_format(&bare), Err(FormatError::UnknownFormat)),
        "a TGA 1.0 carries no signature anywhere"
    );
    assert_eq!(detect_format(&with_footer(bare)).unwrap(), FileFormat::Tga);

    // The signature has to be at the very end: the same bytes one position earlier is not a TGA.
    let mut shifted = with_footer(tga(2, 2, 2, 24, 0x00, &BODY_24));
    shifted.push(0);
    assert!(matches!(
        detect_format(&shifted),
        Err(FormatError::UnknownFormat)
    ));
}

/// **The vertical origin flag is INVERTED**: `flipVert = (header[17] & 0x20) ? 0 : 1`.
///
/// Bit 5 set means a TOP-left origin, so the DEFAULT — bit clear — is bottom-up, the same default
/// as BMP but reached by a different mechanism. Asserted as one claim about the pair: two files
/// differing only in that bit must decode to vertically mirrored images, because an implementation
/// that always flipped and one that never flipped both return the same image twice.
#[test]
fn the_vertical_origin_bit_is_inverted() {
    let (_, _, bottom_left) = decode(&with_footer(tga(2, 2, 2, 24, 0x00, &BODY_24)));
    let (_, _, top_left) = decode(&with_footer(tga(2, 2, 2, 24, 0x20, &BODY_24)));

    // Bit clear: bottom-up, so the LAST stored row is the top one and the top-left pixel is blue.
    assert_eq!(&bottom_left[..4], &[0, 0, 255, 255]);
    // Bit set: top-down, so the FIRST stored row is the top one and the top-left pixel is red.
    assert_eq!(&top_left[..4], &[255, 0, 0, 255]);
    // Exact mirrors.
    assert_eq!(&bottom_left[..8], &top_left[8..16]);
    assert_eq!(&bottom_left[8..16], &top_left[..8]);
}

/// **The horizontal flag is NOT inverted**: `flipHoriz = (header[17] & 0x10) ? 1 : 0`.
///
/// Pinned beside the vertical one because the two sit in the same byte and read in opposite
/// directions — an implementation that treated them uniformly would get exactly one of them wrong,
/// and only testing both catches that.
#[test]
fn the_horizontal_flag_is_not_inverted() {
    let (_, _, plain) = decode(&with_footer(tga(2, 2, 2, 24, 0x00, &BODY_24)));
    let (_, _, mirrored) = decode(&with_footer(tga(2, 2, 2, 24, 0x10, &BODY_24)));

    // The top row is blue then white; mirrored horizontally it is white then blue.
    assert_eq!(&plain[..8], &[0, 0, 255, 255, 255, 255, 255, 255]);
    assert_eq!(&mirrored[..8], &[255, 255, 255, 255, 0, 0, 255, 255]);
}

/// Image type 10 is RLE colour and 2 is uncompressed colour — `type - 8` is the same picture.
///
/// Upstream's switch maps 1/2/3 to mapped/colour/grey uncompressed and 9/10/11 to the same three
/// with `TGA_COMP_RLE`. Asserted as byte equality between the two encodings of one image rather
/// than as two separate pixel checks, because that is the claim: the compression is not supposed to
/// change anything.
#[test]
fn rle_and_uncompressed_decode_to_the_same_image() {
    // One RLE run packet per row: high bit set, low 7 bits are count-1, then one pixel.
    let rle_body = [
        0x81, 0, 0, 255, // run of 2 red
        0x81, 255, 0, 0, // run of 2 blue
    ];
    let flat_body = [0, 0, 255, 0, 0, 255, 255, 0, 0, 255, 0, 0];

    let (rw, rh, rle) = decode(&with_footer(tga(10, 2, 2, 24, 0x00, &rle_body)));
    let (fw, fh, flat) = decode(&with_footer(tga(2, 2, 2, 24, 0x00, &flat_body)));

    assert_eq!((rw, rh), (fw, fh));
    assert_eq!(rle, flat);
    assert_eq!(&rle[..8], &[0, 0, 255, 255, 0, 0, 255, 255]);
}

/// Our export round-trips with alpha, and it writes a file our own detector can read.
///
/// **Both halves were defects found by probing rather than by reading.** The encoder writes a TGA
/// 1.0 stream with no signature, so the exported file detected as nothing and this product refused
/// to import what it had just written; the 26-byte 2.0 footer is now appended, which is the
/// format's own mechanism for being identifiable. And `decode_dynamic` cross-checks our sniff
/// against the image crate's content GUESS, which for TGA can never succeed — so that one format
/// sets its already-detected value directly.
#[test]
fn an_exported_tga_carries_its_footer_and_round_trips_with_alpha() {
    let color = Pixel {
        r: 200,
        g: 100,
        b: 50,
        a: 128,
    };
    let mut editor = Editor::new(Document::new(2, 2).unwrap()).unwrap();
    editor.execute(Command::Fill { color }).unwrap();

    let exported = export_document(
        editor.document(),
        FileFormat::Tga,
        &ExportOptions::default(),
    )
    .expect("export");
    let bytes = exported.bytes();

    // The footer is there, so our own detector finds it.
    assert_eq!(&bytes[bytes.len() - 18..], TGA_FOOTER_SIGNATURE);
    assert_eq!(detect_format(bytes).unwrap(), FileFormat::Tga);

    // Type 10 is RLE colour, which matches upstream's `rle` argument defaulting to TRUE.
    assert_eq!(bytes[2], 10, "RLE colour");
    assert_eq!(bytes[16], 32, "32 bpp, which is how alpha is carried");
    // Descriptor: 8 alpha bits plus bit 5, a TOP-left origin. Upstream's own `origin` argument
    // defaults to `bottom-left` instead — a recorded divergence rather than a defect, since the
    // descriptor declares which convention the file uses and both are legal.
    assert_eq!(bytes[17], 0x28);

    let (width, height, pixels) = decode(bytes);
    assert_eq!((width, height), (2, 2));
    assert_eq!(&pixels[..4], &[200, 100, 50, 128]);
}

/// TGA is lossless: RLE is run-length, not approximation.
#[test]
fn tga_is_reported_lossless_and_opaque_pixels_survive_exactly() {
    let color = Pixel {
        r: 10,
        g: 20,
        b: 30,
        a: 255,
    };
    let mut editor = Editor::new(Document::new(3, 1).unwrap()).unwrap();
    editor.execute(Command::Fill { color }).unwrap();

    let exported = export_document(
        editor.document(),
        FileFormat::Tga,
        &ExportOptions::default(),
    )
    .expect("export");
    assert!(
        exported.metadata().lossless,
        "run-length coding is not approximation"
    );

    let (width, height, pixels) = decode(exported.bytes());
    assert_eq!((width, height), (3, 1));
    for chunk in pixels.chunks_exact(4) {
        assert_eq!(chunk, &[10, 20, 30, 255]);
    }
}

/// A PNG presented as a TGA is refused rather than decoded as whatever it really is.
///
/// Worth its own test here because TGA is the one format whose sniff had to drop the image crate's
/// guess cross-check — this is what shows the OUTER check still does its job.
#[test]
fn a_png_presented_as_a_tga_is_refused() {
    let mut editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel {
                r: 1,
                g: 2,
                b: 3,
                a: 255,
            },
        })
        .unwrap();
    let png = export_document(
        editor.document(),
        FileFormat::Png,
        &ExportOptions::default(),
    )
    .expect("png")
    .into_bytes();

    let options = ImportOptions::default().with_expected_format(FileFormat::Tga);
    assert!(import_document(&png, &options).is_err());
}
