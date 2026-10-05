// SPDX-License-Identifier: GPL-3.0-or-later

//! Paint Shop Pro container tests (M.9a).
//!
//! The fixtures are built field by field rather than checked in as binary, because every refusal
//! this module makes is about a LENGTH or an OFFSET and a hand-built buffer is the only way to move
//! one of those without moving the others.

use redrob_core::psp::{PSP_SIGNATURE, PspColourModel, PspCompression, read_container};
use redrob_core::{FileFormat, ImportOptions, detect_format, import_document};

/// What goes into the General Image Attributes chunk. Defaults are a plausible 24-bit RGB image.
struct Attrs {
    width: u32,
    height: u32,
    resolution: f64,
    metric: u8,
    compression: u16,
    depth: u16,
    grayscale: u8,
    active_layer: u32,
    layer_count: u16,
}

impl Default for Attrs {
    fn default() -> Self {
        Self {
            width: 8,
            height: 4,
            resolution: 72.0,
            metric: 1,      // inches
            compression: 1, // RLE
            depth: 24,
            grayscale: 0,
            active_layer: 0,
            layer_count: 1,
        }
    }
}

impl Attrs {
    /// The 38 bytes upstream reads, in upstream's order. Their count is upstream's own minimum
    /// chunk size, which is the cross-check that the order is right.
    fn fields(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.width.to_le_bytes());
        out.extend_from_slice(&self.height.to_le_bytes());
        out.extend_from_slice(&self.resolution.to_le_bytes());
        out.push(self.metric);
        out.extend_from_slice(&self.compression.to_le_bytes());
        out.extend_from_slice(&self.depth.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes()); // plane count, skipped
        out.extend_from_slice(&0u32.to_le_bytes()); // colour count, skipped
        out.push(self.grayscale);
        out.extend_from_slice(&0u32.to_le_bytes()); // total image size, skipped
        out.extend_from_slice(&self.active_layer.to_le_bytes());
        out.extend_from_slice(&self.layer_count.to_le_bytes());
        assert_eq!(
            out.len(),
            38,
            "the field list must total upstream's minimum"
        );
        out
    }
}

/// A whole file: signature, version, one image block, and nothing else.
fn psp(major: u16, minor: u16, attrs: &Attrs) -> Vec<u8> {
    psp_blocks(major, minor, &[(0u16, chunk(major, attrs))])
}

/// The image block's data, which differs between version 3 and 4 by its own repeated length and a
/// trailing `graphics_content` field.
fn chunk(major: u16, attrs: &Attrs) -> Vec<u8> {
    let fields = attrs.fields();
    if major < 4 {
        fields
    } else {
        let mut out = Vec::new();
        out.extend_from_slice(&46u32.to_le_bytes()); // the inner length upstream re-reads
        out.extend_from_slice(&fields);
        out.extend_from_slice(&0u32.to_le_bytes()); // graphics_content
        assert_eq!(
            out.len(),
            46,
            "version 4 chunk must total upstream's minimum"
        );
        out
    }
}

/// Assemble arbitrary blocks, so a test can put the image block second or add a trailing block.
fn psp_blocks(major: u16, minor: u16, blocks: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(PSP_SIGNATURE.as_slice());
    out.extend_from_slice(&major.to_le_bytes());
    out.extend_from_slice(&minor.to_le_bytes());
    for (id, data) in blocks {
        out.extend_from_slice(b"~BK\0");
        out.extend_from_slice(&id.to_le_bytes());
        if major < 4 {
            out.extend_from_slice(&(data.len() as u32).to_le_bytes()); // initial length
            out.extend_from_slice(&(data.len() as u32).to_le_bytes()); // total length
        } else {
            out.extend_from_slice(&(data.len() as u32).to_le_bytes()); // total length only
        }
        out.extend_from_slice(data);
    }
    out
}

#[test]
fn the_signature_is_thirty_two_bytes_and_ends_in_five_nuls() {
    // Written out because miscounting the padding is the whole risk: 25 characters of text, a
    // newline, 0x1A, and five NULs.
    assert_eq!(PSP_SIGNATURE.len(), 32);
    assert_eq!(&PSP_SIGNATURE[..25], b"Paint Shop Pro Image File");
    assert_eq!(PSP_SIGNATURE[25], b'\n');
    assert_eq!(PSP_SIGNATURE[26], 0x1a);
    assert_eq!(&PSP_SIGNATURE[27..], &[0, 0, 0, 0, 0]);
}

#[test]
fn detection_keys_on_the_signature() {
    let file = psp(4, 0, &Attrs::default());
    assert_eq!(detect_format(&file).unwrap(), FileFormat::Psp);

    // One byte of the signature wrong is not a PSP, and must not be mistaken for one.
    let mut broken = file.clone();
    broken[24] = b'X';
    assert_ne!(detect_format(&broken).ok(), Some(FileFormat::Psp));

    // A prefix of the signature is not enough.
    assert_ne!(detect_format(&file[..31]).ok(), Some(FileFormat::Psp));
}

#[test]
fn version_three_and_version_four_carry_the_same_image_through_different_headers() {
    // THE headline fact of this item: the block header is 14 bytes at version 3 and 10 at version
    // 4, because the initial-chunk-length field was dropped. Same image, two layouts.
    let attrs = Attrs::default();
    let v3 = psp(3, 0, &attrs);
    let v4 = psp(4, 0, &attrs);

    let a = read_container(&v3).unwrap();
    let b = read_container(&v4).unwrap();

    assert_eq!((a.width, a.height), (8, 4));
    assert_eq!((b.width, b.height), (8, 4));
    assert_eq!(a.compression, PspCompression::Rle);
    assert_eq!(b.compression, PspCompression::Rle);
    assert_eq!(a.colour_model, PspColourModel::Rgb);
    assert_eq!(b.colour_model, PspColourModel::Rgb);
    assert_eq!(a.layer_count, 1);
    assert_eq!(b.layer_count, 1);

    // The headers really are different sizes, and the block data starts at different offsets as a
    // result. Asserted so a reader who "simplifies" the branch sees the consequence.
    assert_eq!(a.blocks[0].start, 36 + 14);
    assert_eq!(b.blocks[0].start, 36 + 10);
    // And the version-4 chunk is longer, because it repeats its length and gains a field.
    assert_eq!(a.blocks[0].len, 38);
    assert_eq!(b.blocks[0].len, 46);
}

#[test]
fn a_major_version_below_three_is_refused() {
    // Upstream's own reason is an admission -- it has no documentation before 3.0 -- so the floor
    // is reproduced rather than reasoned about.
    for major in [0u16, 1, 2] {
        let file = psp(major, 0, &Attrs::default());
        let error = read_container(&file).unwrap_err().to_string();
        assert!(error.contains("below 3"), "major {major}: {error}");
    }
    // And an unknown NEWER version is walked, not refused. The asymmetry is deliberate upstream.
    assert!(read_container(&psp(9, 5, &Attrs::default())).is_ok());
}

#[test]
fn the_compression_enum_has_a_value_that_is_illegal_here() {
    // 0, 1, 2 are the image's legal values.
    for (value, expected) in [
        (0u16, PspCompression::None),
        (1, PspCompression::Rle),
        (2, PspCompression::Lz77),
    ] {
        let attrs = Attrs {
            compression: value,
            ..Default::default()
        };
        assert_eq!(
            read_container(&psp(4, 0, &attrs)).unwrap().compression,
            expected
        );
    }

    // 3 is PSP_COMP_JPEG. It EXISTS in the format and upstream refuses it for the image, because
    // it is only legal in the thumbnail and composite blocks. The same number, two meanings.
    let attrs = Attrs {
        compression: 3,
        ..Default::default()
    };
    let error = read_container(&psp(4, 0, &attrs)).unwrap_err().to_string();
    assert!(error.contains("JPEG"), "{error}");

    let attrs = Attrs {
        compression: 4,
        ..Default::default()
    };
    assert!(read_container(&psp(4, 0, &attrs)).is_err());
}

#[test]
fn the_depth_table_is_reproduced_including_its_two_holes() {
    /// One row of upstream's depth table. A named row rather than a bare tuple because the row has
    /// three meanings and `(16, 0, None)` does not say which is which.
    struct Row {
        depth: u16,
        grayscale: u8,
        expected: Option<(PspColourModel, u8)>,
    }

    const fn row(depth: u16, grayscale: u8, expected: Option<(PspColourModel, u8)>) -> Row {
        Row {
            depth,
            grayscale,
            expected,
        }
    }

    let cases = [
        row(24, 0, Some((PspColourModel::Rgb, 1))),
        row(24, 1, Some((PspColourModel::Rgb, 1))),
        row(48, 0, Some((PspColourModel::Rgb, 2))),
        row(8, 1, Some((PspColourModel::Gray, 1))),
        row(16, 1, Some((PspColourModel::Gray, 2))),
        row(1, 0, Some((PspColourModel::Indexed, 1))),
        row(4, 0, Some((PspColourModel::Indexed, 1))),
        row(8, 0, Some((PspColourModel::Indexed, 1))),
        // THE TWO HOLES. Both fall between upstream's branches -- `grayscale && depth >= 8` and
        // `depth <= 8 && !grayscale` -- and upstream refuses both.
        row(16, 0, None), // 16-bit that is not flagged grey
        row(1, 1, None),  // 1-bit that IS flagged grey
        row(4, 1, None),  // 4-bit that IS flagged grey
        row(32, 0, None), // not in the table at all
        row(0, 0, None),
    ];

    for Row {
        depth,
        grayscale,
        expected,
    } in cases
    {
        let attrs = Attrs {
            depth,
            grayscale,
            ..Default::default()
        };
        let result = read_container(&psp(4, 0, &attrs));
        match expected {
            Some((model, bytes)) => {
                let container =
                    result.unwrap_or_else(|e| panic!("depth {depth} grey {grayscale}: {e}"));
                assert_eq!(
                    container.colour_model, model,
                    "depth {depth} grey {grayscale}"
                );
                assert_eq!(
                    container.bytes_per_sample, bytes,
                    "depth {depth} grey {grayscale}"
                );
            }
            None => assert!(
                result.is_err(),
                "depth {depth} grey {grayscale} must be refused, as upstream refuses it"
            ),
        }
    }
}

#[test]
fn a_one_bit_image_is_readable_or_not_depending_only_on_the_greyscale_flag() {
    // The sharpest statement of the second hole, as its own test: the SAME depth, one flag apart,
    // and the answers are "indexed" and "refused". A tidy-up of the table has to break this.
    let indexed = Attrs {
        depth: 1,
        grayscale: 0,
        ..Default::default()
    };
    let grey = Attrs {
        depth: 1,
        grayscale: 1,
        ..Default::default()
    };
    assert_eq!(
        read_container(&psp(4, 0, &indexed)).unwrap().colour_model,
        PspColourModel::Indexed
    );
    assert!(read_container(&psp(4, 0, &grey)).is_err());
}

#[test]
fn centimetres_become_inches_once() {
    let inches = Attrs {
        resolution: 254.0,
        metric: 1,
        ..Default::default()
    };
    let centimetres = Attrs {
        resolution: 254.0,
        metric: 2,
        ..Default::default()
    };
    let undefined = Attrs {
        resolution: 254.0,
        metric: 0,
        ..Default::default()
    };

    assert_eq!(
        read_container(&psp(4, 0, &inches))
            .unwrap()
            .resolution_per_inch,
        254.0
    );
    // 254 per centimetre is 100 per inch.
    assert_eq!(
        read_container(&psp(4, 0, &centimetres))
            .unwrap()
            .resolution_per_inch,
        100.0
    );
    // An UNDEFINED metric is left alone, not guessed at -- upstream only divides for centimetres.
    assert_eq!(
        read_container(&psp(4, 0, &undefined))
            .unwrap()
            .resolution_per_inch,
        254.0
    );
}

#[test]
fn a_block_length_past_the_end_of_the_file_is_refused() {
    let mut file = psp(4, 0, &Attrs::default());
    // The total-length field of the first block sits at 36 + 6 in a version-4 header.
    let at = 36 + 6;
    file[at..at + 4].copy_from_slice(&9999u32.to_le_bytes());
    let error = read_container(&file).unwrap_err().to_string();
    assert!(error.contains("past the end"), "{error}");
}

#[test]
fn the_image_block_must_come_first() {
    let attrs = Attrs::default();
    // A creator block (id 1) ahead of the image block. Upstream refuses this.
    let file = psp_blocks(4, 0, &[(1u16, vec![0u8; 8]), (0u16, chunk(4, &attrs))]);
    let error = read_container(&file).unwrap_err().to_string();
    assert!(error.contains("first block"), "{error}");
}

#[test]
fn a_file_with_no_image_block_is_refused() {
    let file = psp_blocks(4, 0, &[(1u16, vec![0u8; 8])]);
    let error = read_container(&file).unwrap_err().to_string();
    assert!(error.contains("no general image attributes"), "{error}");
}

#[test]
fn an_attribute_chunk_below_the_minimum_is_refused() {
    // 37 bytes: one short of upstream's 38, which is the exact field total.
    let file = psp_blocks(3, 0, &[(0u16, vec![0u8; 37])]);
    let error = read_container(&file).unwrap_err().to_string();
    assert!(error.contains("too short"), "{error}");

    // At version 4 the chunk repeats its own length and upstream wants 46 of it, not 38. A chunk
    // that is long enough by the OUTER rule and short by the inner one is the case that separates
    // the two guards.
    let mut data = Vec::new();
    data.extend_from_slice(&38u32.to_le_bytes()); // inner length: passes 38, fails 46
    data.extend_from_slice(&Attrs::default().fields());
    data.extend_from_slice(&0u32.to_le_bytes());
    let file = psp_blocks(4, 0, &[(0u16, data)]);
    let error = read_container(&file).unwrap_err().to_string();
    assert!(error.contains("too small a length"), "{error}");
}

#[test]
fn the_walk_collects_every_block_and_lands_exactly_on_the_end() {
    let attrs = Attrs::default();
    let file = psp_blocks(
        4,
        0,
        &[
            (0u16, chunk(4, &attrs)),
            (1u16, vec![0xaa; 16]),
            (9u16, vec![0xbb; 4]),
        ],
    );
    let container = read_container(&file).unwrap();
    assert_eq!(container.blocks.len(), 3);
    assert_eq!(container.blocks[1].id, 1);
    assert_eq!(container.blocks[1].len, 16);
    assert_eq!(container.blocks[2].id, 9);
    // The last block's end is the file's end: the only evidence that every length was read right.
    let last = container.blocks[2];
    assert_eq!(last.start + last.len, file.len());
}

#[test]
fn a_block_without_its_signature_is_refused() {
    let mut file = psp(4, 0, &Attrs::default());
    file[36] = b'!';
    let error = read_container(&file).unwrap_err().to_string();
    assert!(error.contains("block header signature"), "{error}");
}

#[test]
fn import_refuses_the_pixels_but_a_broken_container_gets_its_own_error() {
    // This pair is what makes M.9a more than a stub. A GOOD file gets the forward-looking refusal.
    let good = psp(4, 0, &Attrs::default());
    let error = import_document(&good, &ImportOptions::default())
        .unwrap_err()
        .to_string();
    assert!(error.contains("M.9b"), "{error}");

    // A file whose container is broken gets the CONTAINER's error instead, so the refusal above
    // can never hide a malformed file.
    let mut broken = good.clone();
    let at = 36 + 6;
    broken[at..at + 4].copy_from_slice(&9999u32.to_le_bytes());
    let error = import_document(&broken, &ImportOptions::default())
        .unwrap_err()
        .to_string();
    assert!(error.contains("past the end"), "{error}");
    assert!(!error.contains("M.9b"), "{error}");
}

#[test]
fn a_zero_dimension_is_refused() {
    for (width, height) in [(0u32, 4u32), (8, 0)] {
        let attrs = Attrs {
            width,
            height,
            ..Default::default()
        };
        assert!(read_container(&psp(4, 0, &attrs)).is_err());
    }
}
