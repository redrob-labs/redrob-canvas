// SPDX-License-Identifier: GPL-3.0-or-later

//! DICOM (M.11a, first of M.11's two formats — FITS remains).
//!
//! Re-derived from `plug-ins/common/file-dicom.c` (GPL-3.0-or-later), pinned in
//! `docs/upstream-sources.toml`. `crate::dicom` carries the tag table and the reasoning.
//!
//! Per the loop rules a multi-part item is worked one part per cycle and ticked only when every
//! part has passed, exactly as M.7's four formats were — so M.11 stays open.

use redrob_core::{FileFormat, ImportOptions, detect_format, import_document};

/// Build a DICOM: a 128-byte preamble, `DICM`, then explicit-VR little-endian elements.
///
/// Written as a builder so each test reads as the tags it is exercising rather than as a blob.
struct Builder {
    out: Vec<u8>,
}

impl Builder {
    fn new() -> Self {
        let mut out = vec![0u8; 128];
        out.extend_from_slice(b"DICM");
        Self { out }
    }

    /// A `US` element: a 2-byte length, then one unsigned short.
    fn us(mut self, group: u16, element: u16, value: u16) -> Self {
        self.out.extend_from_slice(&group.to_le_bytes());
        self.out.extend_from_slice(&element.to_le_bytes());
        self.out.extend_from_slice(b"US");
        self.out.extend_from_slice(&2u16.to_le_bytes());
        self.out.extend_from_slice(&value.to_le_bytes());
        self
    }

    /// A `CS` string, space-padded to an even length the way a real file pads it.
    fn cs(mut self, group: u16, element: u16, text: &str) -> Self {
        let mut bytes = text.as_bytes().to_vec();
        if bytes.len() % 2 == 1 {
            bytes.push(b' ');
        }
        self.out.extend_from_slice(&group.to_le_bytes());
        self.out.extend_from_slice(&element.to_le_bytes());
        self.out.extend_from_slice(b"CS");
        self.out
            .extend_from_slice(&(bytes.len() as u16).to_le_bytes());
        self.out.extend_from_slice(&bytes);
        self
    }

    /// An `OW` element — a BINARY VR, so two RESERVED bytes precede a FOUR-byte length.
    fn ow(mut self, group: u16, element: u16, body: &[u8]) -> Self {
        self.out.extend_from_slice(&group.to_le_bytes());
        self.out.extend_from_slice(&element.to_le_bytes());
        self.out.extend_from_slice(b"OW");
        self.out.extend_from_slice(&[0, 0]);
        self.out
            .extend_from_slice(&(body.len() as u32).to_le_bytes());
        self.out.extend_from_slice(body);
        self
    }

    fn done(self) -> Vec<u8> {
        self.out
    }
}

fn grey8(photometric: &str, rows: u16, columns: u16, samples: &[u8]) -> Vec<u8> {
    Builder::new()
        .us(0x0028, 0x0002, 1)
        .cs(0x0028, 0x0004, photometric)
        .us(0x0028, 0x0010, rows)
        .us(0x0028, 0x0011, columns)
        .us(0x0028, 0x0100, 8)
        .us(0x0028, 0x0101, 8)
        .us(0x0028, 0x0102, 7)
        .us(0x0028, 0x0103, 0)
        .ow(0x7fe0, 0x0010, samples)
        .done()
}

fn decode(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let outcome = import_document(bytes, &ImportOptions::default()).expect("import");
    let document = outcome.document();
    let greys = document.layers()[0]
        .pixels()
        .chunks_exact(4)
        .map(|pixel| pixel[0])
        .collect();
    (document.width(), document.height(), greys)
}

/// **The signature is at offset 128, after a preamble** — and `DICM` at offset 0 is not a DICOM.
///
/// Third format in group M whose signature is not at the start: TGA's is at the very END, `.pat`'s
/// at 20, this at 128. Both directions asserted, so this cannot pass under a detector that searches
/// the whole file.
#[test]
fn the_signature_follows_a_128_byte_preamble() {
    let real = grey8("MONOCHROME2", 1, 1, &[42, 0]);
    assert_eq!(&real[128..132], b"DICM");
    assert_eq!(detect_format(&real).unwrap(), FileFormat::Dicom);

    let mut at_zero = b"DICM".to_vec();
    at_zero.extend_from_slice(&[0u8; 200]);
    assert!(
        detect_format(&at_zero).is_err(),
        "the magic belongs at 128, not 0"
    );
}

/// **`0028,0010` is ROWS — the HEIGHT — and `0028,0011` is COLUMNS.**
///
/// The lower-numbered element is the height, which is the opposite of the natural assumption, and
/// getting it backwards TRANSPOSES the image without failing. A non-square image is the only input
/// that can tell: the samples are laid out row-major, so 3 rows of 2 must come back 2 wide and 3
/// tall with the samples in order.
#[test]
fn rows_is_the_height_and_columns_is_the_width() {
    let (width, height, greys) = decode(&grey8("MONOCHROME2", 3, 2, &[1, 2, 3, 4, 5, 6]));

    assert_eq!(
        (width, height),
        (2, 3),
        "rows=3 columns=2 is 2 wide and 3 tall, not 3 wide and 2 tall"
    );
    assert_eq!(greys, vec![1, 2, 3, 4, 5, 6], "and in row-major order");
}

/// **`MONOCHROME1` means INVERTED** — its minimum sample is meant to display as white.
///
/// Asserted as a pair against `MONOCHROME2`, the ordinary direction, with the SAME samples: an
/// inversion that always happened, or never, would pass one half alone.
#[test]
fn monochrome1_inverts_and_monochrome2_does_not() {
    let (_, _, normal) = decode(&grey8("MONOCHROME2", 2, 2, &[10, 10, 10, 10]));
    assert_eq!(normal, vec![10, 10, 10, 10], "MONOCHROME2 is as stored");

    let (_, _, inverted) = decode(&grey8("MONOCHROME1", 2, 2, &[10, 10, 10, 10]));
    assert_eq!(
        inverted,
        vec![245, 245, 245, 245],
        "MONOCHROME1 is one's-complement: 255 - 10"
    );
}

/// **The 16-bit chain is TWO composed shifts, and the second is by `bits_stored`, not
/// `bits_allocated`.**
///
/// ```text
/// value = read_le(sample) >> (high_bit + 1 - bits_stored)
/// out   = value >> (bits_stored - 8)
/// ```
///
/// With `bits_allocated` 16, `bits_stored` 12 and `high_bit` 11: the alignment shift is
/// `11 + 1 - 12 = 0` and the narrowing shift is `12 - 8 = 4`. So `0x0FF0` gives **255**. Narrowing
/// by `bits_allocated` instead would shift 8 and give **15** — an image sixteen times too dark,
/// which is the whole reason this has its own test.
#[test]
fn the_sixteen_bit_narrowing_uses_bits_stored_not_bits_allocated() {
    let mut body = Vec::new();
    for _ in 0..4 {
        body.extend_from_slice(&0x0ff0u16.to_le_bytes());
    }
    let deep = Builder::new()
        .us(0x0028, 0x0002, 1)
        .cs(0x0028, 0x0004, "MONOCHROME2")
        .us(0x0028, 0x0010, 2)
        .us(0x0028, 0x0011, 2)
        .us(0x0028, 0x0100, 16)
        .us(0x0028, 0x0101, 12)
        .us(0x0028, 0x0102, 11)
        .us(0x0028, 0x0103, 0)
        .ow(0x7fe0, 0x0010, &body)
        .done();

    let (_, _, greys) = decode(&deep);
    assert_eq!(
        greys,
        vec![255, 255, 255, 255],
        "0x0FF0 >> 4 is 255; shifting by bits_allocated would give 15"
    );
}

/// Only 8 and 16 bits allocated are read, which is upstream's own ceiling.
///
/// Detection still claims the file — it IS a DICOM — and the decoder refuses it by name. That split
/// is deliberate and matches how a 16-bit SGI is handled in M.7a.
#[test]
fn an_unsupported_bit_depth_is_detected_then_refused() {
    let odd = Builder::new()
        .us(0x0028, 0x0002, 1)
        .us(0x0028, 0x0010, 2)
        .us(0x0028, 0x0011, 2)
        .us(0x0028, 0x0100, 12)
        .us(0x0028, 0x0101, 12)
        .us(0x0028, 0x0102, 11)
        .ow(0x7fe0, 0x0010, &[0u8; 8])
        .done();

    assert_eq!(detect_format(&odd).unwrap(), FileFormat::Dicom);
    assert!(
        import_document(&odd, &ImportOptions::default()).is_err(),
        "12 bits allocated is refused rather than guessed at"
    );
}

/// Three samples per pixel is RGB, in channel order.
///
/// Distinct values per channel, so a swap fails here rather than passing on a grey image.
#[test]
fn three_samples_per_pixel_is_rgb_in_order() {
    let rgb = Builder::new()
        .us(0x0028, 0x0002, 3)
        .cs(0x0028, 0x0004, "RGB")
        .us(0x0028, 0x0010, 1)
        .us(0x0028, 0x0011, 2)
        .us(0x0028, 0x0100, 8)
        .us(0x0028, 0x0101, 8)
        .us(0x0028, 0x0102, 7)
        .ow(0x7fe0, 0x0010, &[10, 20, 30, 40, 50, 60])
        .done();

    let outcome = import_document(&rgb, &ImportOptions::default()).expect("import");
    assert_eq!(
        &outcome.document().layers()[0].pixels()[..8],
        &[10, 20, 30, 255, 40, 50, 60, 255]
    );
}

/// A file with no pixel-data element is refused rather than producing an empty image.
#[test]
fn a_dicom_without_pixel_data_is_refused() {
    let headerless = Builder::new()
        .us(0x0028, 0x0010, 2)
        .us(0x0028, 0x0011, 2)
        .us(0x0028, 0x0100, 8)
        .done();

    assert_eq!(detect_format(&headerless).unwrap(), FileFormat::Dicom);
    assert!(import_document(&headerless, &ImportOptions::default()).is_err());
}

/// Export is refused by name rather than being silently absent.
#[test]
fn export_is_refused_by_name() {
    use redrob_core::{Document, Editor, ExportOptions, export_document};

    let editor = Editor::new(Document::new(2, 2).unwrap()).unwrap();
    let refused = export_document(
        editor.document(),
        FileFormat::Dicom,
        &ExportOptions::default(),
    );
    assert!(
        refused.is_err(),
        "a medical format written by an unchecked encoder is the worst place for a quiet mistake"
    );
}
