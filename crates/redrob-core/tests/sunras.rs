// SPDX-License-Identifier: GPL-3.0-or-later

//! SUN raster (M.7b, second of M.7's four formats).
//!
//! Re-derived from `plug-ins/common/file-sunras.c` (GPL-3.0-or-later), pinned in
//! `docs/upstream-sources.toml`. No pure-Rust codec exists, so `crate::sunras` carries the
//! implementation and the full eight-field header table.
//!
//! Three facts here are worth holding next to the formats this group already landed, because each
//! one is the SAME idea spelled differently and getting it wrong still produces a picture:
//!
//! * **`depth` counts BITS** (1, 8, 24, 32). SGI — M.7's previous part — spells the same concept
//!   `bpp` and counts **BYTES** (1 or 2).
//! * **The RLE is an escape scheme on the single byte `0x80`, and its count is off by one**:
//!   `0x80 n v` yields **n + 1** copies. TGA keys on a packet header's high bit meaning repeat;
//!   SGI keys on a control byte's high bit meaning literal. **Three formats, three schemes.**
//! * **`type` declares the channel ORDER as well as the compression.** Every type except 3 is BGR.

use redrob_core::{
    ExportOptions, FileFormat, FormatError, ImportOptions, Pixel, detect_format, export_document,
    import_document,
};

#[path = "common/canvas.rs"]
mod canvas;

const RED: Pixel = Pixel {
    r: 250,
    g: 10,
    b: 20,
    a: 255,
};
const BLUE: Pixel = Pixel {
    r: 5,
    g: 15,
    b: 240,
    a: 255,
};

fn export(width: u32, height: u32, pixels: &[Pixel]) -> Vec<u8> {
    export_document(
        canvas::editor(width, height, pixels).document(),
        FileFormat::SunRaster,
        &ExportOptions::default(),
    )
    .expect("export")
    .into_bytes()
}

/// A 32-byte header with the fields a hand-built fixture needs.
fn header(width: u32, height: u32, depth: u32, kind: u32, maplength: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&0x59a6_6a95u32.to_be_bytes());
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&depth.to_be_bytes());
    bytes.extend_from_slice(&0u32.to_be_bytes()); // length -- upstream notes it may be 0
    bytes.extend_from_slice(&kind.to_be_bytes());
    bytes.extend_from_slice(&u32::from(maplength > 0).to_be_bytes());
    bytes.extend_from_slice(&maplength.to_be_bytes());
    bytes
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

/// The export round-trips, and **`depth` is asserted in BITS**.
///
/// 24 for a three-byte pixel. SGI, the part of M.7 done last cycle, writes **1** in its own
/// equivalent field for a one-byte channel, because that one counts bytes. The two conventions sit
/// one backlog line apart, so this assertion exists to name the difference rather than to check
/// arithmetic.
#[test]
fn the_header_counts_depth_in_bits_not_bytes() {
    let bytes = export(2, 1, &[RED, BLUE]);

    assert_eq!(&bytes[0..4], &0x59a6_6a95u32.to_be_bytes());
    assert_eq!(
        u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        2
    );
    assert_eq!(
        u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
        1
    );
    assert_eq!(
        u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]),
        24,
        "BITS per pixel -- SGI's bpp would say 1 here, counting bytes"
    );
    assert_eq!(
        u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]),
        1,
        "standard, uncompressed -- and therefore BGR, since it is not type 3"
    );

    assert_eq!(detect_format(&bytes).unwrap(), FileFormat::SunRaster);
    let (width, height, pixels) = decode(&bytes);
    assert_eq!((width, height), (2, 1));
    assert_eq!(&pixels[..8], &[250, 10, 20, 255, 5, 15, 240, 255]);
}

/// **An ODD row is padded to an even byte count, and that is the case that shears if missed.**
///
/// Upstream computes the pad three ways — `(((width+7)/8) % 2)` at 1 bit, `(width % 2)` at 8,
/// `((width*3) % 2)` at 24 — which is one rule: pad when the row's byte count is odd. A 3-wide
/// 24-bit row is 9 bytes, so each row carries one pad byte, and the byte count is asserted as well
/// as the pixels: dropping the pad would shift every row after the first by one byte, rotating the
/// colours rather than failing outright.
#[test]
fn an_odd_width_row_is_padded_to_an_even_byte_count() {
    let bytes = export(3, 2, &[RED, BLUE, RED, BLUE, RED, BLUE]);

    // 32-byte header, then two rows of 9 bytes each padded to 10.
    assert_eq!(bytes.len(), 32 + 10 * 2, "one pad byte per odd row");

    let (width, height, pixels) = decode(&bytes);
    assert_eq!((width, height), (3, 2));
    // The second row must be intact, which is what the pad protects.
    assert_eq!(&pixels[12..16], &[5, 15, 240, 255], "row 2 did not shear");
    assert_eq!(&pixels[16..20], &[250, 10, 20, 255]);
}

/// **`type` declares the channel ORDER too: every type except 3 is BGR.**
///
/// Upstream's 24-bit path reads
/// `if (l_ras_type == 3) /* RGB-format ? That is what GIMP wants */ ... else /* We have BGR
/// format. Correct it */`, which is also why it accepts `type <= 5` rather than only its two named
/// compression modes.
///
/// The SAME three body bytes are decoded under both types, so the pixels cannot be the cause of
/// the difference — only the type field can.
#[test]
fn the_type_field_also_declares_the_channel_order() {
    let body = [11u8, 22, 33, 0]; // one 24-bit pixel plus the row pad

    let mut bgr = header(1, 1, 24, 1, 0);
    bgr.extend_from_slice(&body);
    let (_, _, bgr_pixels) = decode(&bgr);
    assert_eq!(
        &bgr_pixels[..4],
        &[33, 22, 11, 255],
        "type 1 is BGR, so the first byte is BLUE"
    );

    let mut rgb = header(1, 1, 24, 3, 0);
    rgb.extend_from_slice(&body);
    let (_, _, rgb_pixels) = decode(&rgb);
    assert_eq!(
        &rgb_pixels[..4],
        &[11, 22, 33, 255],
        "type 3 is RGB, so the first byte is RED"
    );
}

/// **The RLE count is `n + 1`, and `0x80 0x00` is a literal `0x80`.**
///
/// `rle_fgetc` sets `rlebuf.n = runcnt` *and* returns `runval` immediately, so one copy is emitted
/// now and `runcnt` more come from the buffer. The off-by-one is deliberate. One row carries both
/// cases: a run of three and an escaped `0x80`, so a reader that emitted `n` copies would land the
/// escape in the wrong column and fail on both counts at once.
#[test]
fn the_rle_run_length_is_one_more_than_the_count_byte() {
    let mut bytes = header(4, 1, 8, 2, 0);
    //  0x80 0x02 0x41  -> three copies of 0x41 (65), NOT two
    //  0x80 0x00       -> one literal 0x80 (128)
    bytes.extend_from_slice(&[0x80, 0x02, 0x41, 0x80, 0x00]);

    let (width, height, pixels) = decode(&bytes);
    assert_eq!((width, height), (4, 1));

    let greys: Vec<u8> = pixels.chunks_exact(4).map(|pixel| pixel[0]).collect();
    assert_eq!(
        greys,
        vec![65, 65, 65, 128],
        "n + 1 copies, then the escaped 0x80"
    );
}

/// **At 32 bits the unused byte comes FIRST**, not last.
///
/// `getc (ifp); /* Skip unused byte */` precedes the three samples, so the layout is pad-then-BGR.
/// Reading it as BGR-then-pad would take the pad as blue and shift the rest.
#[test]
fn the_unused_byte_at_thirty_two_bits_comes_first() {
    let mut bytes = header(1, 1, 32, 1, 0);
    bytes.extend_from_slice(&[0x00, 0x11, 0x22, 0x33]);

    let (_, _, pixels) = decode(&bytes);
    assert_eq!(
        &pixels[..4],
        &[0x33, 0x22, 0x11, 255],
        "the leading 0x00 is the pad; the rest is BGR"
    );
}

/// The two header validations upstream performs are performed here.
///
/// `type > 5` and `maplength > 256 * 3` are both refused outright by `load_image`. A legal type 5
/// is asserted to still be claimed, so this cannot pass with the whole field rejected.
#[test]
fn the_header_validations_upstream_performs_are_performed() {
    let mut bad_type = header(1, 1, 24, 6, 0);
    bad_type.extend_from_slice(&[0, 0, 0, 0]);
    assert!(matches!(
        detect_format(&bad_type),
        Err(FormatError::UnknownFormat)
    ));

    let mut legal_type = header(1, 1, 24, 5, 0);
    legal_type.extend_from_slice(&[0, 0, 0, 0]);
    assert_eq!(
        detect_format(&legal_type).unwrap(),
        FileFormat::SunRaster,
        "upstream rejects only type > 5"
    );

    let mut bad_map = header(1, 1, 8, 1, 769);
    bad_map.extend_from_slice(&[0u8; 800]);
    assert!(matches!(
        detect_format(&bad_map),
        Err(FormatError::UnknownFormat)
    ));
}

/// The colormap is PLANAR — every red, then every green, then every blue.
///
/// Upstream indexes it `suncolmap[j]`, `suncolmap[j + ncols]`, `suncolmap[j + 2 * ncols]`. Read as
/// RGB triples instead, a two-entry map would give the first pixel `(red0, red1, green0)`.
#[test]
fn the_colormap_is_planar_not_interleaved() {
    // Two entries: reds 10, 20 | greens 30, 40 | blues 50, 60
    let mut bytes = header(2, 1, 8, 1, 6);
    bytes.extend_from_slice(&[10, 20, 30, 40, 50, 60]);
    bytes.extend_from_slice(&[0, 1]); // index 0 then index 1

    let (_, _, pixels) = decode(&bytes);
    assert_eq!(&pixels[..4], &[10, 30, 50, 255], "entry 0");
    assert_eq!(&pixels[4..8], &[20, 40, 60, 255], "entry 1");
}

/// A PNG presented as a SUN raster is refused rather than decoded as what it really is.
#[test]
fn a_png_presented_as_a_sun_raster_is_refused() {
    let png = export_document(
        canvas::editor(1, 1, &[RED]).document(),
        FileFormat::Png,
        &ExportOptions::default(),
    )
    .expect("png")
    .into_bytes();

    let options = ImportOptions::default().with_expected_format(FileFormat::SunRaster);
    assert!(import_document(&png, &options).is_err());
}
