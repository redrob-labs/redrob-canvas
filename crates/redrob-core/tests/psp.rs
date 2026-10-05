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

// ---------------------------------------------------------------------------------------------
// M.9b: channel decompression.
// ---------------------------------------------------------------------------------------------

use redrob_core::psp::{ChannelLayout, channel_line_width, decompress_rle, read_uncompressed};

fn layout(stride: usize, offset: usize, bytes_per_sample: u8) -> ChannelLayout {
    ChannelLayout {
        stride,
        offset,
        bytes_per_sample,
    }
}

#[test]
fn the_run_flag_is_strictly_above_128_and_the_boundary_is_the_encoding() {
    // 128 is a LITERAL of 128 bytes. 129 is a RUN of one. Getting this comparison wrong shifts
    // every later byte of the channel rather than breaking one pixel, which is why the two sit in
    // one test.
    let mut literal_src = vec![128u8];
    literal_src.extend(0..128u8);
    let mut dest = vec![0u8; 128];
    decompress_rle(&literal_src, &mut dest, layout(1, 0, 1), 128).unwrap();
    assert_eq!(dest[0], 0);
    assert_eq!(dest[127], 127);

    // 129 -> one copy of the value byte, not 129 of them and not a literal.
    let run_src = [129u8, 0xab];
    let mut dest = vec![0u8; 4];
    decompress_rle(&run_src, &mut dest, layout(1, 0, 1), 1).unwrap();
    assert_eq!(dest, vec![0xab, 0, 0, 0]);

    // 255 is the longest run: 127 copies.
    let run_src = [255u8, 0x7f];
    let mut dest = vec![0u8; 127];
    decompress_rle(&run_src, &mut dest, layout(1, 0, 1), 127).unwrap();
    assert!(dest.iter().all(|b| *b == 0x7f));
}

#[test]
fn a_zero_count_is_refused_rather_than_looped() {
    // THE ONE DELIBERATE DIVERGENCE, and the reverse-verification sharpened it. Upstream's
    // `while (q < endq)` neither advances nor fails on a zero count, and its own overflow guard
    // cannot catch it because zero is never greater than the space left.
    //
    // Removing the check here does NOT hang this port -- it reads a slice, so the next count byte
    // runs off the end and the error becomes "ends mid-channel". What the check buys is the TRUE
    // cause rather than a misleading one: a zero count is a malformed stream, not a truncated
    // one, and the distinction is what a reader debugging a real file needs.
    let source = [0u8, 0, 0, 0];
    let mut dest = vec![0u8; 8];
    let error = decompress_rle(&source, &mut dest, layout(1, 0, 1), 8)
        .unwrap_err()
        .to_string();
    assert!(error.contains("would not advance"), "{error}");
}

#[test]
fn a_truncated_literal_or_run_is_refused() {
    // A literal that claims more than the file holds.
    let mut dest = vec![0u8; 8];
    assert!(decompress_rle(&[4u8, 1, 2], &mut dest, layout(1, 0, 1), 8).is_err());
    // A run whose value byte is missing.
    let mut dest = vec![0u8; 8];
    assert!(decompress_rle(&[200u8], &mut dest, layout(1, 0, 1), 8).is_err());
    // Data that stops before the channel is full.
    let mut dest = vec![0u8; 8];
    assert!(decompress_rle(&[2u8, 1, 2], &mut dest, layout(1, 0, 1), 8).is_err());
}

#[test]
fn one_byte_samples_scatter_by_the_stride() {
    // A channel is ONE component; the destination interleaves four. So a run of four writes four
    // bytes four apart, not a contiguous block -- the property that makes this not a memcpy.
    let source = [132u8, 0x11]; // run of 4
    let mut dest = vec![0u8; 16];
    decompress_rle(&source, &mut dest, layout(4, 2, 1), 16).unwrap();
    assert_eq!(dest[2], 0x11);
    assert_eq!(dest[6], 0x11);
    assert_eq!(dest[10], 0x11);
    assert_eq!(dest[14], 0x11);
    // Every other byte untouched, including the ones before the offset.
    assert_eq!(dest[0], 0);
    assert_eq!(dest[1], 0);
    assert_eq!(dest[3], 0);
}

#[test]
fn two_byte_samples_are_little_endian_and_an_odd_run_loses_its_last_byte() {
    // Upstream's loop count is `runcount / 2`, so an odd literal's trailing byte starts a sample
    // whose other half does not exist and upstream does not invent one. Faithful, and asserted so
    // a future "round up" looks like a change in behaviour rather than a tidy-up.
    // The fixture has to clear upstream's overflow guard first, or the guard -- not the odd-run
    // rule -- is what the test measures. With four samples of room the guard's threshold is
    // `4 / 1 + 1 = 5`, so a literal of five passes it.
    let source = [
        5u8, 0x34, 0x12, 0x78, 0x56, 0x99, // two whole samples, then an orphan byte
        2, 0xaa, 0xbb, // one more sample
        2, 0xcc, 0xdd, // and one more, filling the channel
    ];
    let mut dest = vec![0u8; 8];
    decompress_rle(&source, &mut dest, layout(2, 0, 2), 8).unwrap();

    assert_eq!(u16::from_le_bytes([dest[0], dest[1]]), 0x1234);
    assert_eq!(u16::from_le_bytes([dest[2], dest[3]]), 0x5678);
    // The orphan 0x99 was skipped, and the NEXT literal starts a fresh sample rather than
    // completing it -- so 0xaa pairs with 0xbb, not with 0x99.
    assert_eq!(u16::from_le_bytes([dest[4], dest[5]]), 0xbbaa);
    assert_eq!(u16::from_le_bytes([dest[6], dest[7]]), 0xddcc);
    assert!(!dest.contains(&0x99));
}

#[test]
fn the_overflow_guard_keeps_what_it_has_instead_of_failing_the_load() {
    // Upstream prints a warning and BREAKS, handing back a partly-decoded channel. Failing instead
    // would reject images that currently open, so the partial result is the faithful answer.
    let source = [2u8, 0xaa, 0xbb, 200u8, 0xcc]; // 2 good bytes, then a run of 72 into 2 bytes
    let mut dest = vec![0u8; 4];
    decompress_rle(&source, &mut dest, layout(1, 0, 1), 4).unwrap();
    assert_eq!(dest[0], 0xaa);
    assert_eq!(dest[1], 0xbb);
    // The oversized run was dropped whole rather than partly applied.
    assert_eq!(dest[2], 0);
    assert_eq!(dest[3], 0);
}

#[test]
fn a_channel_that_does_not_fit_its_destination_is_refused() {
    // An offset genuinely past the buffer is refused. An offset that merely makes the CURSOR limit
    // exceed the buffer is NOT -- that is the ordinary interleaved case, asserted above.
    let mut dest = vec![0u8; 4];
    assert!(decompress_rle(&[129u8, 1], &mut dest, layout(1, 5, 1), 4).is_err());

    // A layout that cannot work at all.
    let mut dest = vec![0u8; 8];
    assert!(decompress_rle(&[129u8, 1], &mut dest, layout(0, 0, 1), 8).is_err());
    assert!(decompress_rle(&[129u8, 1], &mut dest, layout(1, 0, 3), 8).is_err());
}

#[test]
fn scanlines_pad_to_four_bytes_below_eight_bits_and_not_at_or_above() {
    // Upstream's split, in its own words: "Scanlines for 1 and 4 bit only end on a 4-byte
    // boundary", and in the other branch it contradicts the specification -- "Contrary to what the
    // PSP specification seems to suggest scanlines are not stored on a 4-byte boundary."
    //
    // One bit, 1 pixel: one byte of data, padded to 4.
    assert_eq!(channel_line_width(1, 1, 1), 4);
    // One bit, 33 pixels: 5 bytes of data, padded to 8.
    assert_eq!(channel_line_width(33, 1, 1), 8);
    // Four bits, 9 pixels: 5 bytes, padded to 8.
    assert_eq!(channel_line_width(9, 4, 1), 8);

    // Eight bits and above: NOT padded. A 5-pixel 8-bit line is 5 bytes, not 8.
    assert_eq!(channel_line_width(5, 8, 1), 5);
    assert_eq!(channel_line_width(5, 16, 2), 10);
    assert_eq!(channel_line_width(3, 24, 1), 3);
}

#[test]
fn an_uncompressed_contiguous_channel_is_one_bulk_read() {
    let source: Vec<u8> = (0..12u8).collect();
    let mut dest = vec![0u8; 12];
    read_uncompressed(&source, &mut dest, layout(1, 0, 1), 4, 3, 8).unwrap();
    assert_eq!(dest, source);

    // Truncated input is refused rather than part-filled.
    assert!(read_uncompressed(&source[..5], &mut [0u8; 12], layout(1, 0, 1), 4, 3, 8).is_err());
}

#[test]
fn an_uncompressed_interleaved_channel_scatters_row_by_row() {
    // 2x2, one byte per sample, into a 4-component destination at offset 1.
    let source = [1u8, 2, 3, 4];
    let mut dest = vec![0u8; 16];
    read_uncompressed(&source, &mut dest, layout(4, 1, 1), 2, 2, 8).unwrap();
    assert_eq!(dest[1], 1);
    assert_eq!(dest[5], 2);
    assert_eq!(dest[9], 3);
    assert_eq!(dest[13], 4);
    assert_eq!(dest[0], 0);
}

#[test]
fn an_uncompressed_two_byte_channel_reads_width_samples_not_line_width_bytes() {
    // The two uncompressed paths do not count the same thing: one byte per sample loops over
    // `line_width`, two bytes per sample reads `width` samples. At 16 bits those agree in bytes
    // but the loop bound differs, and this asserts the sample count is what drives it.
    let source = [0x34u8, 0x12, 0x78, 0x56]; // two 16-bit samples
    let mut dest = vec![0u8; 8];
    read_uncompressed(&source, &mut dest, layout(4, 0, 2), 2, 1, 16).unwrap();
    assert_eq!(u16::from_le_bytes([dest[0], dest[1]]), 0x1234);
    assert_eq!(u16::from_le_bytes([dest[4], dest[5]]), 0x5678);
}

// ---------------------------------------------------------------------------------------------
// M.9c: the layer bank and layer attributes.
// ---------------------------------------------------------------------------------------------

use redrob_core::BlendMode;
use redrob_core::psp::{PspLayerKind, PspRect, blend_mode_for, read_layer_bank};

/// What goes into one layer's information chunk.
struct Layer {
    name: Vec<u8>,
    kind: u8,
    saved: (u32, u32, u32, u32),
    opacity: u8,
    blend: u8,
    visible: u8,
}

impl Default for Layer {
    fn default() -> Self {
        Self {
            name: b"Background".to_vec(),
            kind: 0, // raster
            saved: (0, 0, 8, 4),
            opacity: 255,
            blend: 0, // normal
            visible: 1,
        }
    }
}

fn rect(l: u32, t: u32, r: u32, b: u32) -> Vec<u8> {
    let mut out = Vec::new();
    for value in [l, t, r, b] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

impl Layer {
    /// The run of fields after the name, identical in both versions.
    fn fixed(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(self.kind);
        out.extend_from_slice(&rect(0, 0, 8, 4)); // image_rect
        out.extend_from_slice(&rect(
            self.saved.0,
            self.saved.1,
            self.saved.2,
            self.saved.3,
        ));
        out.push(self.opacity);
        out.push(self.blend);
        out.push(self.visible);
        out.push(0); // transparency_protected
        out.push(7); // link_group_id
        out.extend_from_slice(&rect(0, 0, 0, 0)); // mask_rect
        out.extend_from_slice(&rect(0, 0, 0, 0)); // saved_mask_rect
        out.push(1); // mask_linked
        out.push(0); // mask_disabled
        assert_eq!(
            out.len(),
            72,
            "the fixed field run is the same in both versions"
        );
        out
    }

    /// One layer sub-block's data, in the version's own shape.
    fn chunk(&self, major: u16) -> Vec<u8> {
        let mut out = Vec::new();
        if major >= 4 {
            let body_len = 4 + 2 + self.name.len() + 72;
            out.extend_from_slice(&(body_len as u32).to_le_bytes());
            out.extend_from_slice(&(self.name.len() as u16).to_le_bytes());
            out.extend_from_slice(&self.name);
        } else {
            // Version 3: a fixed 256-byte name field.
            let mut field = vec![0u8; 256];
            field[..self.name.len()].copy_from_slice(&self.name);
            out.extend_from_slice(&field);
        }
        out.extend_from_slice(&self.fixed());
        out
    }
}

/// A layer bank block's data: one layer sub-block per layer.
fn bank(major: u16, layers: &[Layer]) -> Vec<u8> {
    bank_with_ids(
        major,
        &layers
            .iter()
            .map(|l| (4u16, l.chunk(major)))
            .collect::<Vec<_>>(),
    )
}

fn bank_with_ids(major: u16, blocks: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (id, data) in blocks {
        out.extend_from_slice(b"~BK\0");
        out.extend_from_slice(&id.to_le_bytes());
        if major < 4 {
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        }
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
    }
    out
}

#[test]
fn the_name_field_has_a_different_shape_in_each_version() {
    // Version 3 stores 256 fixed bytes; version 4 stores a u16 length then the bytes. Using one
    // shape on the other version either eats 254 bytes of the following fields or reads a length
    // out of the middle of a name -- so both are asserted to produce the SAME layer.
    let layer = Layer::default();

    let v3 = read_layer_bank(&bank(3, std::slice::from_ref(&layer)), 3).unwrap();
    let v4 = read_layer_bank(&bank(4, std::slice::from_ref(&layer)), 4).unwrap();

    assert_eq!(v3.len(), 1);
    assert_eq!(v4.len(), 1);
    assert_eq!(v3[0].name, "Background");
    assert_eq!(v4[0].name, "Background");
    assert_eq!(v3[0].opacity, 255);
    assert_eq!(v4[0].opacity, 255);
    assert_eq!(v3[0].link_group_id, 7);
    assert_eq!(v4[0].link_group_id, 7);
    assert_eq!((v3[0].width, v3[0].height), (8, 4));
    assert_eq!((v4[0].width, v4[0].height), (8, 4));
}

#[test]
fn a_zero_length_name_is_valid() {
    // Upstream says so in a comment, and the guard it wrote beside that comment cannot fire:
    // `namelen && namelen == 0` is always false. Porting the guard instead of the comment yields a
    // condition that does nothing, so the accepting behaviour is pinned here.
    let layer = Layer {
        name: Vec::new(),
        ..Default::default()
    };
    let v4 = read_layer_bank(&bank(4, std::slice::from_ref(&layer)), 4).unwrap();
    assert_eq!(v4[0].name, "");

    // At version 3 an all-NUL field is the same thing.
    let v3 = read_layer_bank(&bank(3, std::slice::from_ref(&layer)), 3).unwrap();
    assert_eq!(v3[0].name, "");
}

#[test]
fn the_name_is_latin_one_not_utf_eight() {
    // Upstream converts from "iso8859-1", so a byte above 0x7F is ONE character. Read as UTF-8
    // these bytes are not valid at all, which is how the distinction shows.
    let layer = Layer {
        name: vec![0xc9, 0xe9, 0xfc], // E-acute, e-acute, u-diaeresis in latin-1
        ..Default::default()
    };
    let layers = read_layer_bank(&bank(4, std::slice::from_ref(&layer)), 4).unwrap();
    assert_eq!(layers[0].name, "Ééü");
    assert!(String::from_utf8(vec![0xc9, 0xe9, 0xfc]).is_err());
}

#[test]
fn version_three_forces_every_layer_to_raster() {
    // Upstream reads the type byte and then assigns `type = keGLTRaster` unconditionally. So a
    // version-3 file has no vector or group layers whatever its bytes say.
    for kind in [0u8, 2, 3, 4, 5, 6] {
        let layer = Layer {
            kind,
            ..Default::default()
        };
        let layers = read_layer_bank(&bank(3, std::slice::from_ref(&layer)), 3).unwrap();
        assert_eq!(
            layers[0].kind,
            PspLayerKind::Raster,
            "version 3 type byte {kind} must still be raster"
        );
    }

    // At version 4 the byte IS the type.
    let layer = Layer {
        kind: 2,
        ..Default::default()
    };
    let layers = read_layer_bank(&bank(4, std::slice::from_ref(&layer)), 4).unwrap();
    assert_eq!(layers[0].kind, PspLayerKind::Vector);

    // And an unknown type at version 4 is refused rather than guessed.
    let layer = Layer {
        kind: 99,
        ..Default::default()
    };
    assert!(read_layer_bank(&bank(4, std::slice::from_ref(&layer)), 4).is_err());
}

#[test]
fn an_unmappable_blend_mode_hides_the_layer_instead_of_failing_the_image() {
    // Upstream keeps the layer, sets normal, and turns visibility OFF with a message. Failing
    // would reject a whole image for one unmappable layer.
    let layer = Layer {
        blend: 255, // PSP_BLEND_ADJUST, which upstream explicitly declines to map
        visible: 1,
        ..Default::default()
    };
    let layers = read_layer_bank(&bank(4, std::slice::from_ref(&layer)), 4).unwrap();
    assert_eq!(layers[0].blend_mode, BlendMode::Normal);
    assert!(
        !layers[0].visible,
        "an unmappable blend mode must hide the layer"
    );
    // The raw byte survives, so the information is not lost even though the mode is.
    assert_eq!(layers[0].blend_mode_raw, 255);

    // A mappable mode leaves visibility alone.
    let layer = Layer {
        blend: 7, // multiply
        visible: 1,
        ..Default::default()
    };
    let layers = read_layer_bank(&bank(4, std::slice::from_ref(&layer)), 4).unwrap();
    assert_eq!(layers[0].blend_mode, BlendMode::Multiply);
    assert!(layers[0].visible);
}

#[test]
fn the_blend_table_collapses_psp_eight_true_modes_onto_ours() {
    // Upstream keeps these apart by legacy-ness -- PSP_BLEND_HUE to HSV_HUE_LEGACY and
    // PSP_BLEND_TRUE_HUE to HSV_HUE. This product has one of each, so the pairs collapse. That is
    // a real loss of fidelity against upstream and is asserted so it reads as measured rather
    // than overlooked.
    assert_eq!(blend_mode_for(3), blend_mode_for(17)); // hue / true hue
    assert_eq!(blend_mode_for(4), blend_mode_for(18)); // saturation / true saturation
    assert_eq!(blend_mode_for(5), blend_mode_for(19)); // colour / true colour
    assert_eq!(blend_mode_for(6), blend_mode_for(20)); // luminosity / true lightness

    // 255 has no mapping, and that is upstream's own decision, marked in its source with "???".
    assert_eq!(blend_mode_for(255), None);
    // So does anything outside the table.
    assert_eq!(blend_mode_for(21), None);
    assert_eq!(blend_mode_for(100), None);

    // Spot checks that the table is not accidentally the identity.
    assert_eq!(blend_mode_for(0), Some(BlendMode::Normal));
    assert_eq!(blend_mode_for(9), Some(BlendMode::Dissolve));
    assert_eq!(blend_mode_for(16), Some(BlendMode::Exclusion));
}

#[test]
fn the_dimensions_come_from_the_saved_rectangle_not_the_image_rectangle() {
    // Upstream computes width from saved_image_rect. The fixture's image_rect is always 0,0,8,4,
    // so a different saved rectangle proves which one is used.
    let layer = Layer {
        saved: (2, 1, 7, 3),
        ..Default::default()
    };
    let layers = read_layer_bank(&bank(4, std::slice::from_ref(&layer)), 4).unwrap();
    assert_eq!((layers[0].width, layers[0].height), (5, 2));
    assert_eq!(
        layers[0].image_rect,
        PspRect {
            left: 0,
            top: 0,
            right: 8,
            bottom: 4
        }
    );
}

#[test]
fn an_inverted_rectangle_is_refused() {
    let layer = Layer {
        saved: (7, 0, 2, 4), // right left of left
        ..Default::default()
    };
    let error = read_layer_bank(&bank(4, std::slice::from_ref(&layer)), 4)
        .unwrap_err()
        .to_string();
    assert!(error.contains("inverted"), "{error}");
}

#[test]
fn the_area_limit_is_looser_than_a_true_area_check() {
    // Upstream's third test is `(width / 256) * (height / 256) >= 8192` with INTEGER division,
    // which discards up to 255 from each edge and so UNDER-estimates the area. The limit is
    // therefore LOOSER than the true area check it resembles, and reproducing it faithfully means
    // accepting layers a true check would refuse.
    //
    // **The first version of this test claimed the quirk showed on a tall, narrow layer, and that
    // was wrong** -- a reverse-verification replacing the formula with a true area check PASSED it.
    // With the edge capped at 2^18, a layer narrower than 256 reaches at most 255 * 262144 =
    // 66,846,720 pixels, far below the 536,870,912 a true check would refuse, so a narrow layer
    // cannot distinguish the two formulas at all. These fixtures can.

    // 23200 x 23200: integer division gives 90 * 90 = 8100, under the limit, so upstream ACCEPTS.
    // The true area is 538,240,000, which is over 8192 * 65536 and would be refused.
    let accepted = Layer {
        saved: (0, 0, 23_200, 23_200),
        ..Default::default()
    };
    assert!(
        read_layer_bank(&bank(4, std::slice::from_ref(&accepted)), 4).is_ok(),
        "upstream's integer division accepts this; a true area check would not"
    );

    // 65536 x 8192: integer division gives 256 * 32 = 8192, meeting the limit exactly, so this is
    // refused. Both edges are inside 2^18, so it is the AREA test rejecting it and not the edge
    // test -- which the previous fixture got wrong by using a height of 2^20.
    let refused = Layer {
        saved: (0, 0, 65_536, 8_192),
        ..Default::default()
    };
    assert!(read_layer_bank(&bank(4, std::slice::from_ref(&refused)), 4).is_err());

    // And an edge past 2^18 is refused on its own, by the other test.
    let too_wide = Layer {
        saved: (0, 0, (1 << 18) + 1, 1),
        ..Default::default()
    };
    assert!(read_layer_bank(&bank(4, std::slice::from_ref(&too_wide)), 4).is_err());
}

#[test]
fn a_bank_sub_block_that_is_not_a_layer_is_refused() {
    // Upstream names the offending block rather than skipping it, so this is a refusal.
    let layer = Layer::default();
    let data = bank_with_ids(4, &[(5u16, layer.chunk(4))]); // 5 is CHANNEL, not LAYER
    let error = read_layer_bank(&data, 4).unwrap_err().to_string();
    assert!(error.contains("not a layer"), "{error}");
}

#[test]
fn the_bank_walk_reads_every_layer_in_order() {
    let layers = [
        Layer {
            name: b"one".to_vec(),
            opacity: 10,
            ..Default::default()
        },
        Layer {
            name: b"two".to_vec(),
            opacity: 20,
            ..Default::default()
        },
        Layer {
            name: b"three".to_vec(),
            opacity: 30,
            ..Default::default()
        },
    ];
    let read = read_layer_bank(&bank(4, &layers), 4).unwrap();
    assert_eq!(read.len(), 3);
    assert_eq!(read[0].name, "one");
    assert_eq!(read[1].name, "two");
    assert_eq!(read[2].name, "three");
    assert_eq!(read[2].opacity, 30);
}

#[test]
fn a_sub_block_running_past_the_bank_is_refused() {
    let layer = Layer::default();
    let mut data = bank(4, std::slice::from_ref(&layer));
    // The total-length field of a version-4 sub-block header sits at offset 6.
    data[6..10].copy_from_slice(&99_999u32.to_le_bytes());
    let error = read_layer_bank(&data, 4).unwrap_err().to_string();
    assert!(error.contains("past its bank"), "{error}");
}

#[test]
fn a_truncated_layer_chunk_is_refused_rather_than_part_read() {
    let layer = Layer::default();
    let mut chunk = layer.chunk(4);
    chunk.truncate(20);
    let data = bank_with_ids(4, &[(4u16, chunk)]);
    assert!(read_layer_bank(&data, 4).is_err());

    // And at version 3, a name field shorter than 256 bytes.
    let data = bank_with_ids(3, &[(4u16, vec![0u8; 100])]);
    assert!(read_layer_bank(&data, 3).is_err());
}
