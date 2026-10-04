// SPDX-License-Identifier: GPL-3.0-or-later

//! X BitMap (M.7d, LAST of M.7's four formats).
//!
//! Re-derived from `plug-ins/common/file-xbm.c` (GPL-3.0-or-later), pinned in
//! `docs/upstream-sources.toml`. `crate::xbm` carries the implementation and the reasoning;
//! `xbm 0.3.0` on the registry was deliberately not adopted, because the three facts below are
//! exactly what an independent implementation could differ on.
//!
//! **1. Upstream registers NO MAGIC and cannot** — the `#define` prefix is the image's own name —
//! so it reaches the loader by file extension. Third format in group M with no registered
//! signature, after TGA 1.0 and ICO.
//! **2. A SET BIT IS BLACK.** Upstream flips every word on read, `c ^= 0xffff`, under its own
//! comment: *"Flip all the bits so that 1's become black and 0's become white."*
//! **3. BITS RUN LEAST-SIGNIFICANT FIRST** — `data[...] = c & 1; c >>= 1;` — so bit 0 is the
//! LEFTMOST pixel. **SUN raster's 1-bit path (M.7b, this same item) and PBM (M.3) are both
//! most-significant-first.**

use redrob_core::{
    ExportOptions, FileFormat, FormatError, FormatWarning, ImportOptions, Pixel, detect_format,
    export_document, import_document,
};

#[path = "common/canvas.rs"]
mod canvas;

const BLACK: Pixel = Pixel {
    r: 0,
    g: 0,
    b: 0,
    a: 255,
};
const WHITE: Pixel = Pixel {
    r: 255,
    g: 255,
    b: 255,
    a: 255,
};

fn greys(text: &str) -> (u32, u32, Vec<u8>, Vec<FormatWarning>) {
    let outcome = import_document(text.as_bytes(), &ImportOptions::default()).expect("import");
    let document = outcome.document();
    let values = document.layers()[0]
        .pixels()
        .chunks_exact(4)
        .map(|pixel| pixel[0])
        .collect();
    (
        document.width(),
        document.height(),
        values,
        outcome.warnings().to_vec(),
    )
}

fn bitmap(width: u32, height: u32, ctype: &str, words: &str) -> String {
    format!(
        "#define t_width {width}\n#define t_height {height}\n\
         static {ctype} t_bits[] = {{ {words} }};\n"
    )
}

/// **The two facts that matter, in one test, because one byte cannot separate them.**
///
/// `0x01` and `0x80` are the discriminating pair. Four readings are possible and only one is right:
///
/// | reading                        | `0x01` gives     | `0x80` gives     |
/// |--------------------------------|------------------|------------------|
/// | set = black, bit 0 leftmost    | **black first**  | **black last**   |
/// | set = black, bit 7 leftmost    | black last       | black first      |
/// | set = white, bit 0 leftmost    | white first      | white last       |
/// | set = white, bit 7 leftmost    | white last       | white first      |
///
/// So asserting both bytes pins the polarity AND the bit order together; either byte alone leaves
/// two readings standing.
#[test]
fn a_set_bit_is_black_and_bit_zero_is_the_leftmost_pixel() {
    let (width, _, low, _) = greys(&bitmap(8, 1, "unsigned char", "0x01"));
    assert_eq!(width, 8);
    assert_eq!(
        low,
        vec![0, 255, 255, 255, 255, 255, 255, 255],
        "0x01: the set bit is BLACK and it is the LEFTMOST pixel"
    );

    let (_, _, high, _) = greys(&bitmap(8, 1, "unsigned char", "0x80"));
    assert_eq!(
        high,
        vec![255, 255, 255, 255, 255, 255, 255, 0],
        "0x80: bit 7 is the RIGHTMOST pixel -- most-significant-first would put it on the left"
    );
}

/// The export round-trips, and the word it writes is checkable by hand.
///
/// Black, white, black, white at 4 wide is bits 0 and 2 set, which is `0x05`. That single byte is
/// where facts 2 and 3 meet, so it is asserted literally rather than only round-tripped.
#[test]
fn the_exported_word_is_the_one_the_two_facts_predict() {
    let bytes = export_document(
        canvas::editor(4, 1, &[BLACK, WHITE, BLACK, WHITE]).document(),
        FileFormat::Xbm,
        &ExportOptions::default(),
    )
    .expect("export")
    .into_bytes();
    let text = String::from_utf8(bytes).expect("XBM is text");

    assert!(text.contains("#define image_width 4"), "got:\n{text}");
    assert!(text.contains("#define image_height 1"));
    assert!(
        text.contains("0x05"),
        "black at x=0 and x=2 sets bits 0 and 2; got:\n{text}"
    );

    assert_eq!(detect_format(text.as_bytes()).unwrap(), FileFormat::Xbm);
    let (width, height, values, _) = greys(&text);
    assert_eq!((width, height), (4, 1));
    assert_eq!(values, vec![0, 255, 0, 255]);
}

/// The C type is the word size: `char` is 8 bits, `short` is 16, and neither is refused.
///
/// Upstream sets `intbits` from whichever keyword it matched and errors when it saw no type at all,
/// so the declaration is load-bearing rather than decoration. A 16-wide `short` row is ONE word.
#[test]
fn the_c_type_declares_the_word_size() {
    let (width, _, values, _) = greys(&bitmap(16, 1, "short", "0x0003"));
    assert_eq!(width, 16);
    assert_eq!(
        &values[..4],
        &[0, 0, 255, 255],
        "one 16-bit word covers the whole row"
    );

    // Same picture through 8-bit words needs two of them.
    let (_, _, eight, _) = greys(&bitmap(16, 1, "unsigned char", "0x03, 0x00"));
    assert_eq!(eight, values, "8-bit and 16-bit words describe one image");

    let no_type = "#define t_width 8\n#define t_height 1\nstatic t_bits[] = { 0x01 };\n";
    assert_eq!(detect_format(no_type.as_bytes()).unwrap(), FileFormat::Xbm);
    assert!(
        import_document(no_type.as_bytes(), &ImportOptions::default()).is_err(),
        "upstream refuses a file whose type it never matched"
    );
}

/// **Each ROW starts on a word boundary**, so a short row's padding bits never bleed into the next.
///
/// Upstream's `k % intbits == 0` resets per row. A 4-wide image spends a whole byte per row, and
/// the fixture puts the set bit in a different column on each row so a reader that packed the rows
/// continuously would misplace the second one.
#[test]
fn every_row_starts_on_a_word_boundary() {
    let (width, height, values, _) = greys(&bitmap(4, 2, "unsigned char", "0x01, 0x08"));
    assert_eq!((width, height), (4, 2));
    assert_eq!(
        values,
        vec![0, 255, 255, 255, 255, 255, 255, 0],
        "row 0's bit 0 and row 1's bit 3, each in its own byte"
    );
}

/// **A declared hotspot is REPORTED, because this product cannot keep it.**
///
/// `_x_hot` / `_y_hot` carry a cursor hotspot that upstream stores as a parasite on the image.
/// There is nowhere to put it here, so the import says so rather than dropping it in silence —
/// the same call M.6 made for QOI's linear declaration. Asserted as a pair against a file with no
/// hotspot, since a warning that always fired would pass either half alone.
#[test]
fn a_declared_hotspot_is_reported_rather_than_dropped() {
    let plain = bitmap(8, 1, "unsigned char", "0x01");
    let (_, _, _, quiet) = greys(&plain);
    assert!(quiet.is_empty(), "no hotspot, nothing to report");

    let hot = "#define t_width 8\n#define t_height 1\n\
               #define t_x_hot 3\n#define t_y_hot 0\n\
               static unsigned char t_bits[] = { 0x01 };\n";
    let (_, _, _, warnings) = greys(hot);
    assert!(
        warnings
            .iter()
            .any(|warning| matches!(warning, FormatWarning::OmittedMetadata)),
        "a hotspot we cannot store must be reported, got {warnings:?}"
    );
}

/// **Detection needs all three parts**, because upstream registers no magic and this is a
/// divergence rather than an inherited rule.
///
/// `#define`, a name ending `_width`, and an integer. `#define` alone is any C file; `_width`
/// alone is any C file that happens to use the word; and a `_width` with no number is not a
/// declaration of anything.
#[test]
fn detection_requires_a_define_a_width_name_and_a_number() {
    assert_eq!(
        detect_format(bitmap(8, 1, "unsigned char", "0x01").as_bytes()).unwrap(),
        FileFormat::Xbm
    );

    for negative in [
        &b"#define FOO 3\nint main(){}\n"[..],
        &b"#define t_width\n"[..],
        &b"int t_width = 8;\n"[..],
    ] {
        assert!(
            matches!(detect_format(negative), Err(FormatError::UnknownFormat)),
            "{:?} is not an XBM",
            String::from_utf8_lossy(negative)
        );
    }
}

/// A PNG presented as an XBM is refused rather than decoded as what it really is.
#[test]
fn a_png_presented_as_an_xbm_is_refused() {
    let png = export_document(
        canvas::editor(1, 1, &[BLACK]).document(),
        FileFormat::Png,
        &ExportOptions::default(),
    )
    .expect("png")
    .into_bytes();

    let options = ImportOptions::default().with_expected_format(FileFormat::Xbm);
    assert!(import_document(&png, &options).is_err());
}
