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
fn import_reads_a_whole_file_and_a_broken_container_still_gets_its_own_error() {
    // **M.9a to M.9g built seven parsers and `import_document` refused the pixels the whole time.**
    // M.9h is the part that makes the format actually open, so this test replaces the refusal
    // assertion that stood for seven cycles.

    // A file with no layer bank: upstream's only pixel path IS the layer bank (its composite
    // reader is "Not yet implemented"), so there is nothing to draw and that is reported as an
    // unsupported feature rather than as a malformed file -- the file is fine.
    let bare = psp(4, 0, &Attrs::default());
    let error = import_document(&bare, &ImportOptions::default())
        .unwrap_err()
        .to_string();
    assert!(error.contains("no layer bank"), "{error}");

    // A file whose container is broken still gets the CONTAINER's error, so the message above can
    // never hide a malformed file.
    let mut broken = bare.clone();
    let at = 36 + 6;
    broken[at..at + 4].copy_from_slice(&9999u32.to_le_bytes());
    let error = import_document(&broken, &ImportOptions::default())
        .unwrap_err()
        .to_string();
    assert!(error.contains("past the end"), "{error}");
    assert!(!error.contains("no layer bank"), "{error}");
}

/// A whole PSP carrying one RGB layer, assembled from the pieces the other fixtures build.
fn psp_with_layer(major: u16, attrs: &Attrs, spec: &Layer, channels: &[u8]) -> Vec<u8> {
    let mut layer_sub = spec.chunk(major);
    layer_sub.extend_from_slice(channels);

    let mut bank = Vec::new();
    bank.extend_from_slice(b"~BK\0");
    bank.extend_from_slice(&4u16.to_le_bytes()); // PSP_LAYER_BLOCK
    if major < 4 {
        // Version 3's initial length is what positions past the information chunk, so it must be
        // the chunk's real length -- the name, the fields, the reserved span and the counts.
        bank.extend_from_slice(&(256u32 + 72 + 43 + 4).to_le_bytes());
    }
    bank.extend_from_slice(&(layer_sub.len() as u32).to_le_bytes());
    bank.extend_from_slice(&layer_sub);

    psp_blocks(
        major,
        0,
        &[(0u16, chunk(major, attrs)), (3u16, bank)], // image attributes, then the layer bank
    )
}

#[test]
fn the_icc_block_is_thirty_two_and_a_composite_bank_at_eighteen_is_not_mistaken_for_it() {
    // **THIRD TEST WRITTEN BECAUSE A REVERSE-VERIFICATION PASSED.** Setting the ICC block id to 18
    // broke nothing: no fixture carried an ICC profile. 18 is the COMPOSITE IMAGE BANK, and a first
    // draft of this importer had exactly that number -- which would have fed a composite bank to
    // the ICC reader and reported a corrupt profile on a perfectly good file.
    let attrs = Attrs {
        width: 2,
        height: 1,
        depth: 24,
        compression: 0,
        ..Default::default()
    };
    let spec = Layer {
        name: b"L".to_vec(),
        saved: (0, 0, 2, 1),
        bitmaps: 1,
        ..Default::default()
    };
    let mut channels = Vec::new();
    for channel in [1u16, 2, 3] {
        channels.extend_from_slice(&channel_block(4, 0, channel, &[0x40, 0x80]));
    }
    let mut layer_sub = spec.chunk(4);
    layer_sub.extend_from_slice(&channels);
    let mut bank = Vec::new();
    bank.extend_from_slice(b"~BK\0");
    bank.extend_from_slice(&4u16.to_le_bytes());
    bank.extend_from_slice(&(layer_sub.len() as u32).to_le_bytes());
    bank.extend_from_slice(&layer_sub);

    // An ICC profile block: the variable header, then the bytes.
    let profile = b"icc-profile-payload";
    let mut icc = Vec::new();
    icc.extend_from_slice(&8u32.to_le_bytes()); // header size: itself plus the profile size
    icc.extend_from_slice(&(profile.len() as u32).to_le_bytes());
    icc.extend_from_slice(profile);

    // A composite image bank at id 18, holding bytes that are NOT a valid ICC header. If 18 were
    // read as the profile block, this would fail the import.
    let composite = vec![0xffu8; 24];

    let file = psp_blocks(
        4,
        0,
        &[
            (0u16, chunk(4, &attrs)),
            (3u16, bank),
            (18u16, composite),
            (32u16, icc),
        ],
    );

    // The import succeeds, which it would not if 18 were taken for the profile block.
    let outcome = import_document(&file, &ImportOptions::default()).unwrap();
    assert_eq!(outcome.document().layers().len(), 1);

    // And the profile really was read, from id 32.
    let image = redrob_core::psp::read_image(&file).unwrap();
    assert_eq!(
        image.sidecar.icc_profile.as_deref(),
        Some(profile.as_slice())
    );
}

#[test]
fn an_indexed_palette_stored_after_the_layers_is_still_applied() {
    // **The block order is not assumed.** Upstream dispatches on each block's id as it walks, so
    // nothing stops a file from putting the colour palette AFTER the layer bank -- and an indexed
    // layer cannot be converted before its palette is known. A reader that converted layers during
    // the walk would work on most files and produce a black image on these.
    let attrs = Attrs {
        width: 2,
        height: 1,
        depth: 8,
        grayscale: 0, // 8-bit, not grey -> indexed
        compression: 0,
        ..Default::default()
    };
    let spec = Layer {
        name: b"Indexed".to_vec(),
        saved: (0, 0, 2, 1),
        bitmaps: 1,
        ..Default::default()
    };

    // Index 0 then index 2.
    let channels = channel_block(4, 0, 0, &[0, 2]);
    let mut layer_sub = spec.chunk(4);
    layer_sub.extend_from_slice(&channels);

    let mut bank = Vec::new();
    bank.extend_from_slice(b"~BK\0");
    bank.extend_from_slice(&4u16.to_le_bytes());
    bank.extend_from_slice(&(layer_sub.len() as u32).to_le_bytes());
    bank.extend_from_slice(&layer_sub);

    let palette = colour_block(4, &[[10, 20, 30], [0, 0, 0], [200, 100, 50]]);

    // THE PALETTE COMES LAST, after the layer bank.
    let file = psp_blocks(
        4,
        0,
        &[(0u16, chunk(4, &attrs)), (3u16, bank), (2u16, palette)],
    );

    let outcome = import_document(&file, &ImportOptions::default()).unwrap();
    let layers = outcome.document().layers();
    assert_eq!(
        layers[0].pixels(),
        &[10, 20, 30, 0xff, 200, 100, 50, 0xff],
        "indices must resolve through a palette that appears later in the file"
    );
    // No "indexed without a palette" warning, because the palette was found.
    assert!(
        !outcome
            .warnings()
            .iter()
            .any(|w| format!("{w:?}").contains("indexed")),
        "{:?}",
        outcome.warnings()
    );
}

#[test]
fn a_forty_eight_bit_file_narrows_by_the_high_byte_and_warns() {
    // **SECOND TEST WRITTEN BECAUSE A REVERSE-VERIFICATION PASSED.** Swapping the high byte for the
    // low one broke nothing: every fixture used depth 24, so the narrowing path never ran.
    //
    // Upstream's precision for a 48-bit image is U16_NON_LINEAR and this product's raster is 8-bit,
    // so the bits have to go somewhere. Taking the HIGH byte is what `>> 8` does; rounding would be
    // a different answer and neither is more faithful, so the choice is pinned by a test rather
    // than left to whichever byte an index happens to reach.
    let attrs = Attrs {
        width: 2,
        height: 1,
        depth: 48,
        compression: 0,
        ..Default::default()
    };
    let spec = Layer {
        name: b"Deep".to_vec(),
        saved: (0, 0, 2, 1),
        bitmaps: 1,
        ..Default::default()
    };

    // Two 16-bit samples per channel, little-endian. The HIGH bytes are what must survive.
    let mut channels = Vec::new();
    channels.extend_from_slice(&channel_block(4, 0, 1, &[0x34, 0x12, 0x78, 0x56])); // red
    channels.extend_from_slice(&channel_block(4, 0, 2, &[0xbc, 0x9a, 0xf0, 0xde])); // green
    channels.extend_from_slice(&channel_block(4, 0, 3, &[0x11, 0x22, 0x33, 0x44])); // blue

    let file = psp_with_layer(4, &attrs, &spec, &channels);
    let outcome = import_document(&file, &ImportOptions::default()).unwrap();

    let layers = outcome.document().layers();
    assert_eq!(
        layers[0].pixels(),
        &[0x12, 0x9a, 0x22, 0xff, 0x56, 0xde, 0x44, 0xff],
        "the high byte of each 16-bit sample must survive, not the low one"
    );

    // And the loss is REPORTED rather than silent -- a narrowed import that says nothing trains a
    // reader to assume nothing was lost.
    assert!(
        outcome
            .warnings()
            .iter()
            .any(|w| format!("{w:?}").contains("Narrowed")),
        "a 48-bit import must warn: {:?}",
        outcome.warnings()
    );
}

#[test]
fn a_version_nine_layer_extension_may_be_preceded_by_a_block_and_is_told_apart_by_value() {
    // **THIS TEST EXISTS BECAUSE A REVERSE-VERIFICATION PASSED.** Disabling the `~BK\0` check broke
    // nothing: every fixture used version 4, where the check cannot fire. The most interesting
    // thing in M.9h was untested.
    //
    // From PSP 9 the layer extension may be preceded by an extra block of id 0x21, added -- in
    // upstream's words -- "to fix an oversight in the specification". Upstream cannot tell by
    // version number: "we can't test based on version number only". So it compares the four bytes
    // it just read against `~BK\0`, justified by "the layer_extension_len here is always a small
    // number" -- a genuine length can never spell the signature.
    let attrs = Attrs {
        width: 2,
        height: 1,
        depth: 24,
        compression: 0,
        ..Default::default()
    };
    let spec = Layer {
        name: b"Nine".to_vec(),
        saved: (0, 0, 2, 1),
        bitmaps: 1,
        ..Default::default()
    };

    let mut channels = Vec::new();
    for (channel, payload) in [(1u16, [0x11, 0x22]), (2, [0x33, 0x44]), (3, [0x55, 0x66])] {
        channels.extend_from_slice(&channel_block(9, 0, channel, &payload));
    }

    // Build the layer sub-block by hand: the information chunk, then the 0x21 prefix block, then
    // the real extension, then the channels.
    let name = &spec.name;
    let body_len = 4 + 2 + name.len() + 72;
    let mut sub = Vec::new();
    sub.extend_from_slice(&(body_len as u32).to_le_bytes());
    sub.extend_from_slice(&(name.len() as u16).to_le_bytes());
    sub.extend_from_slice(name);
    sub.extend_from_slice(&spec.fixed());
    assert_eq!(sub.len(), body_len);

    // The prefix block upstream has to skip: `~BK\0`, id 0x21, and a length covering its own data.
    let prefix_payload = [0xeeu8; 12];
    sub.extend_from_slice(b"~BK\0");
    sub.extend_from_slice(&0x21u16.to_le_bytes());
    sub.extend_from_slice(&(prefix_payload.len() as u32).to_le_bytes());
    sub.extend_from_slice(&prefix_payload);

    // Then the extension proper.
    sub.extend_from_slice(&8u32.to_le_bytes());
    sub.extend_from_slice(&1u16.to_le_bytes()); // bitmap count
    sub.extend_from_slice(&1u16.to_le_bytes()); // channel count
    sub.extend_from_slice(&channels);

    let mut bank = Vec::new();
    bank.extend_from_slice(b"~BK\0");
    bank.extend_from_slice(&4u16.to_le_bytes());
    bank.extend_from_slice(&(sub.len() as u32).to_le_bytes());
    bank.extend_from_slice(&sub);

    let file = psp_blocks(9, 0, &[(0u16, chunk(9, &attrs)), (3u16, bank.clone())]);

    let outcome = import_document(&file, &ImportOptions::default()).unwrap();
    let layers = outcome.document().layers();
    assert_eq!(layers.len(), 1);
    assert_eq!(layers[0].name(), "Nine");
    // The prefix block's 0xee bytes must appear NOWHERE: if the skip were missed, they would be
    // read as the extension's length and counts and the channels would be found at the wrong place.
    assert_eq!(
        layers[0].pixels(),
        &[0x11, 0x33, 0x55, 0xff, 0x22, 0x44, 0x66, 0xff]
    );

    // And the same bytes at version 8 must NOT be treated as a prefix block, because upstream
    // gates the check on the version as well as the signature.
    let file_v8 = psp_blocks(8, 0, &[(0u16, chunk(8, &attrs)), (3u16, bank.clone())]);
    assert!(import_document(&file_v8, &ImportOptions::default()).is_err());
}

#[test]
fn a_file_with_one_rgb_layer_imports_to_a_document() {
    let attrs = Attrs {
        width: 2,
        height: 1,
        depth: 24,
        compression: 0, // none
        ..Default::default()
    };
    let spec = Layer {
        name: b"Only".to_vec(),
        saved: (0, 0, 2, 1),
        bitmaps: 1, // no alpha
        ..Default::default()
    };

    let mut channels = Vec::new();
    channels.extend_from_slice(&channel_block(4, 0, 1, &[0xff, 0x00])); // red
    channels.extend_from_slice(&channel_block(4, 0, 2, &[0x00, 0xff])); // green
    channels.extend_from_slice(&channel_block(4, 0, 3, &[0x00, 0x00])); // blue

    let file = psp_with_layer(4, &attrs, &spec, &channels);
    assert_eq!(detect_format(&file).unwrap(), FileFormat::Psp);

    let outcome = import_document(&file, &ImportOptions::default()).unwrap();
    let document = outcome.document();
    assert_eq!(document.width(), 2);
    assert_eq!(document.height(), 1);

    // **Assert the PIXELS, not just that a document came back.** Cycle 12's RGB-offset bug slipped
    // through a test that only checked `is_ok()`, so the end-to-end case has to say what landed
    // where: pixel 0 pure red, pixel 1 pure green, both opaque -- which only holds if the channel
    // offsets, the RGBA conversion and the placement all agree.
    let layers = document.layers();
    assert_eq!(layers.len(), 1);
    assert_eq!(layers[0].name(), "Only");
    assert_eq!(
        layers[0].pixels(),
        &[0xff, 0x00, 0x00, 0xff, 0x00, 0xff, 0x00, 0xff]
    );
}

#[test]
fn version_three_and_version_four_files_import_to_the_same_document() {
    // The two versions differ in the block header, the layer name field, the channel chunk AND
    // where the counts live. A file built in either shape must arrive at the same document, which
    // is the one assertion that exercises all four splits at once.
    let attrs = Attrs {
        width: 2,
        height: 1,
        depth: 24,
        compression: 0,
        ..Default::default()
    };
    let spec = Layer {
        name: b"Same".to_vec(),
        saved: (0, 0, 2, 1),
        bitmaps: 1,
        ..Default::default()
    };

    let mut v3_channels = Vec::new();
    let mut v4_channels = Vec::new();
    for (channel, payload) in [(1u16, [0x10, 0x20]), (2, [0x30, 0x40]), (3, [0x50, 0x60])] {
        v3_channels.extend_from_slice(&channel_block(3, 0, channel, &payload));
        v4_channels.extend_from_slice(&channel_block(4, 0, channel, &payload));
    }

    let v3 = psp_with_layer(3, &attrs, &spec, &v3_channels);
    let v4 = psp_with_layer(4, &attrs, &spec, &v4_channels);

    let a = import_document(&v3, &ImportOptions::default()).unwrap();
    let b = import_document(&v4, &ImportOptions::default()).unwrap();
    assert_eq!(a.document().width(), b.document().width());
    assert_eq!(a.document().height(), b.document().height());

    // The PIXELS must match, not just the shape -- that is what exercises all four version splits
    // through to the output.
    assert_eq!(
        a.document().layers()[0].pixels(),
        b.document().layers()[0].pixels()
    );
    assert_eq!(
        a.document().layers()[0].pixels(),
        &[0x10, 0x30, 0x50, 0xff, 0x20, 0x40, 0x60, 0xff]
    );
    assert_eq!(a.document().layers()[0].name(), "Same");
    assert_eq!(b.document().layers()[0].name(), "Same");
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
    /// 1 means no alpha; anything else means alpha.
    bitmaps: u16,
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
            bitmaps: 1,
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
    ///
    /// **The counts live in different places in the two versions**, so the fixture has to build
    /// both tails: version 3 keeps `bitmap_count` and `channel_count` inside this chunk, 43 bytes
    /// past the fields, and version 4 puts them in a separate extension chunk that follows.
    fn chunk(&self, major: u16) -> Vec<u8> {
        let mut out = Vec::new();
        if major >= 4 {
            // The chunk length points at the extension that follows it.
            let body_len = 4 + 2 + self.name.len() + 72;
            out.extend_from_slice(&(body_len as u32).to_le_bytes());
            out.extend_from_slice(&(self.name.len() as u16).to_le_bytes());
            out.extend_from_slice(&self.name);
            out.extend_from_slice(&self.fixed());
            assert_eq!(out.len(), body_len);
            // The layer extension: its own length, then the two counts.
            out.extend_from_slice(&8u32.to_le_bytes());
            out.extend_from_slice(&self.bitmaps.to_le_bytes());
            out.extend_from_slice(&1u16.to_le_bytes()); // channel count
        } else {
            // Version 3: a fixed 256-byte name field, the fields, a 43-byte reserved span, then
            // the counts.
            let mut field = vec![0u8; 256];
            field[..self.name.len()].copy_from_slice(&self.name);
            out.extend_from_slice(&field);
            out.extend_from_slice(&self.fixed());
            out.extend(std::iter::repeat_n(0u8, 43));
            out.extend_from_slice(&self.bitmaps.to_le_bytes());
            out.extend_from_slice(&1u16.to_le_bytes());
        }
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

// ---------------------------------------------------------------------------------------------
// M.9d: palette, LZ77 and the sub-8-bit index upscale.
// ---------------------------------------------------------------------------------------------

use redrob_core::psp::{
    PSP_MAX_PALETTE_ENTRIES, decompress_lz77, read_palette, upscale_indexed_sub_8,
};

/// A colour block's data for the given version, from BGR triples.
fn colour_block(major: u16, entries: &[[u8; 3]]) -> Vec<u8> {
    let mut out = Vec::new();
    if major >= 4 {
        // chunk_len first, then the count; the entries begin chunk_len bytes into the block.
        let chunk_len = 8u32;
        out.extend_from_slice(&chunk_len.to_le_bytes());
        out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    } else {
        out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    }
    for entry in entries {
        // Stored blue, green, red, then the byte upstream says is always zero.
        out.push(entry[2]);
        out.push(entry[1]);
        out.push(entry[0]);
        out.push(0);
    }
    out
}

fn zlib(plain: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(plain).unwrap();
    encoder.finish().unwrap()
}

#[test]
fn the_palette_is_stored_blue_first_whatever_upstreams_comment_says() {
    // Upstream's comment reads "Convert to BGR palette" and the code does the opposite: source
    // byte 2 becomes destination byte 0, and the result is handed to babl as "R'G'B' u8". That tag
    // settles it -- the stored order is B, G, R. Following the comment swaps red and blue in every
    // indexed image.
    //
    // The fixture writes a pure RED entry in the file's own order, so a reader that keeps the byte
    // order would report blue.
    let red = [0xff, 0x00, 0x00];
    let data = colour_block(4, &[red]);
    // Byte 0 of the stored entry is blue, which for pure red is zero.
    assert_eq!(data[8], 0x00);
    assert_eq!(data[10], 0xff);

    let palette = read_palette(&data, 4, PspColourModel::Indexed)
        .unwrap()
        .unwrap();
    assert_eq!(palette, vec![[0xff, 0x00, 0x00]], "red must come back red");
}

#[test]
fn a_palette_on_a_non_indexed_image_is_skipped_and_is_not_an_error() {
    // Upstream's own words: "Skipping, but not an error, can happen for grayscale PSP images".
    let data = colour_block(4, &[[1, 2, 3]]);
    for model in [PspColourModel::Gray, PspColourModel::Rgb] {
        let result = read_palette(&data, 4, model).unwrap();
        assert!(
            result.is_none(),
            "{model:?} must skip the palette, not fail"
        );
    }
    // Indexed reads it.
    assert!(
        read_palette(&data, 4, PspColourModel::Indexed)
            .unwrap()
            .is_some()
    );
}

#[test]
fn the_palette_entry_count_and_entries_are_not_adjacent_at_version_four() {
    // At version 4 the block opens with its chunk length, then the count, and the ENTRIES begin
    // chunk_len bytes into the block -- not just after the count. At version 3 there is no chunk
    // length and the entries follow the count directly. Same palette, two layouts.
    let entries = [[10, 20, 30], [40, 50, 60]];
    let v3 = read_palette(&colour_block(3, &entries), 3, PspColourModel::Indexed)
        .unwrap()
        .unwrap();
    let v4 = read_palette(&colour_block(4, &entries), 4, PspColourModel::Indexed)
        .unwrap()
        .unwrap();
    assert_eq!(v3, vec![[10, 20, 30], [40, 50, 60]]);
    assert_eq!(v4, v3);
}

#[test]
fn a_palette_above_two_hundred_and_fifty_six_entries_is_refused() {
    // Upstream's limit, and its own comment says the limit is GIMP's rather than the format's.
    assert_eq!(PSP_MAX_PALETTE_ENTRIES, 256);

    let full: Vec<[u8; 3]> = (0..256).map(|i| [i as u8, 0, 0]).collect();
    assert!(read_palette(&colour_block(4, &full), 4, PspColourModel::Indexed).is_ok());

    // 257 is refused by the count alone, before the entries are read.
    let mut data = colour_block(4, &full);
    data[4..8].copy_from_slice(&257u32.to_le_bytes());
    let error = read_palette(&data, 4, PspColourModel::Indexed)
        .unwrap_err()
        .to_string();
    assert!(error.contains("256"), "{error}");
}

#[test]
fn a_truncated_palette_is_refused() {
    let mut data = colour_block(4, &[[1, 2, 3], [4, 5, 6]]);
    data.truncate(12); // claims two entries, holds one
    assert!(read_palette(&data, 4, PspColourModel::Indexed).is_err());
}

#[test]
fn lz77_is_a_plain_zlib_stream() {
    // The single most useful fact about this path: upstream calls inflateInit/inflate/inflateEnd
    // with no custom window or dictionary, so there is no bespoke scheme to re-derive.
    let plain: Vec<u8> = (0..16u8).collect();
    let mut dest = vec![0u8; 16];
    decompress_lz77(&zlib(&plain), &mut dest, layout(1, 0, 1), 16).unwrap();
    assert_eq!(dest, plain);
}

#[test]
fn lz77_is_strict_where_rle_is_lenient() {
    // Upstream requires Z_STREAM_END here and fails the load otherwise, while the RLE path breaks
    // out of its loop and hands back a partly-filled channel. The same image's two compression
    // schemes have opposite failure policies, so this asserts both halves together.
    let plain: Vec<u8> = (0..16u8).collect();
    let stream = zlib(&plain);

    // A truncated zlib stream is an ERROR, not a partial channel.
    let mut dest = vec![0u8; 16];
    assert!(decompress_lz77(&stream[..stream.len() / 2], &mut dest, layout(1, 0, 1), 16).is_err());
    assert!(
        dest.iter().all(|b| *b == 0),
        "nothing may be written on failure"
    );

    // A stream that inflates to the wrong size is also an error.
    let mut dest = vec![0u8; 16];
    assert!(decompress_lz77(&zlib(&plain[..8]), &mut dest, layout(1, 0, 1), 16).is_err());

    // Whereas RLE keeps what it had: two good bytes then an oversized run.
    let mut dest = vec![0u8; 4];
    decompress_rle(&[2u8, 0xaa, 0xbb, 200, 0xcc], &mut dest, layout(1, 0, 1), 4).unwrap();
    assert_eq!(dest[0], 0xaa);
}

#[test]
fn an_lz77_stream_carrying_more_than_the_channel_is_refused() {
    // **This test exists because a reverse-verification PASSED.** Disabling the Z_STREAM_END check
    // broke nothing: every case the tests had was a SHORT stream, which the separate
    // inflated-length check already catches. The case only Z_STREAM_END catches is the opposite
    // one -- a stream carrying MORE than the channel holds.
    //
    // Upstream's `avail_out` is exactly the channel size, so inflate runs out of room and never
    // reaches Z_STREAM_END. The inflated length then equals the expected length, so the length
    // check is satisfied and cannot see the problem. Without this test the strictness claim was
    // asserted only where it was redundant.
    let plain: Vec<u8> = (0..32u8).collect();
    let mut dest = vec![0u8; 16];
    let error = decompress_lz77(&zlib(&plain), &mut dest, layout(1, 0, 1), 16)
        .unwrap_err()
        .to_string();
    assert!(error.contains("end of its zlib stream"), "{error}");
    assert!(
        dest.iter().all(|b| *b == 0),
        "nothing may be written when the stream overruns the channel"
    );
}

#[test]
fn lz77_scatters_into_an_interleaved_destination() {
    let plain = [1u8, 2, 3, 4];
    let mut dest = vec![0u8; 16];
    decompress_lz77(&zlib(&plain), &mut dest, layout(4, 3, 1), 4).unwrap();
    assert_eq!(dest[3], 1);
    assert_eq!(dest[7], 2);
    assert_eq!(dest[11], 3);
    assert_eq!(dest[15], 4);
    assert_eq!(dest[0], 0);
}

#[test]
fn lz77_two_byte_samples_are_little_endian() {
    let plain = [0x34u8, 0x12, 0x78, 0x56];
    let mut dest = vec![0u8; 8];
    decompress_lz77(&zlib(&plain), &mut dest, layout(4, 0, 2), 2).unwrap();
    assert_eq!(u16::from_le_bytes([dest[0], dest[1]]), 0x1234);
    assert_eq!(u16::from_le_bytes([dest[4], dest[5]]), 0x5678);
}

#[test]
fn sub_eight_bit_indices_unpack_most_significant_bit_first() {
    // Upstream's mask is `128 >> (bit % 8)` and it accumulates `1 << (bpp - 1 - b)`, so a pixel's
    // first bit is the HIGH bit of its index. Reading the other way round gives a plausible image
    // with every index bit-reversed, which no visual check catches.
    //
    // One bit, 8 pixels: 0b1000_0001 is pixel 0 set and pixel 7 set.
    let packed = {
        let mut row = vec![0u8; 4]; // a 1-bit 8-pixel line pads to 4 bytes
        row[0] = 0b1000_0001;
        row
    };
    let out = upscale_indexed_sub_8(&packed, 8, 1, 1).unwrap();
    assert_eq!(out, vec![1, 0, 0, 0, 0, 0, 0, 1]);

    // Four bits, 2 pixels: 0x3A is index 3 then index 10, high nibble first.
    let packed = {
        let mut row = vec![0u8; 4];
        row[0] = 0x3a;
        row
    };
    let out = upscale_indexed_sub_8(&packed, 2, 1, 4).unwrap();
    assert_eq!(out, vec![3, 10]);
}

#[test]
fn the_index_upscale_uses_the_padded_line_width() {
    // A 1-bit, 8-pixel, 2-row image: each row occupies FOUR bytes, not one. A reader using one
    // byte per row would read row 1's data from row 0's padding.
    let mut packed = vec![0u8; 8];
    packed[0] = 0b1111_0000; // row 0
    packed[4] = 0b0000_1111; // row 1, after three bytes of padding
    let out = upscale_indexed_sub_8(&packed, 8, 2, 1).unwrap();
    assert_eq!(&out[..8], &[1, 1, 1, 1, 0, 0, 0, 0]);
    assert_eq!(&out[8..], &[0, 0, 0, 0, 1, 1, 1, 1]);
}

#[test]
fn the_index_upscale_refuses_depths_it_does_not_apply_to() {
    let packed = vec![0u8; 64];
    for depth in [8u16, 16, 24, 2, 0] {
        assert!(
            upscale_indexed_sub_8(&packed, 8, 1, depth).is_err(),
            "depth {depth} is not a sub-8-bit index"
        );
    }
    // And a truncated buffer is refused rather than part-read.
    assert!(upscale_indexed_sub_8(&[0u8; 2], 8, 2, 1).is_err());
}

// ---------------------------------------------------------------------------------------------
// M.9e: channel assembly.
// ---------------------------------------------------------------------------------------------

use redrob_core::psp::{
    PspImageContext, PspSkippedChannel, assemble_layer, layer_line_width, layer_origin,
};

/// Build one channel sub-block. `chunk_len` is where the payload starts, as upstream seeks.
fn channel_block(major: u16, bitmap_type: u16, channel_type: u16, payload: &[u8]) -> Vec<u8> {
    let mut chunk = Vec::new();
    if major >= 4 {
        chunk.extend_from_slice(&16u32.to_le_bytes()); // chunk_len: 4 + 12
    }
    chunk.extend_from_slice(&(payload.len() as u32).to_le_bytes()); // compressed_len
    chunk.extend_from_slice(&(payload.len() as u32).to_le_bytes()); // uncompressed_len
    chunk.extend_from_slice(&bitmap_type.to_le_bytes());
    chunk.extend_from_slice(&channel_type.to_le_bytes());
    if major < 4 {
        // Version 3 has no inner length, so the header's initial length must point at the payload:
        // that is the 12 bytes of fields above.
        assert_eq!(chunk.len(), 12);
    } else {
        assert_eq!(chunk.len(), 16, "the version-4 minimum is the field total");
    }
    let initial_len = chunk.len() as u32;
    chunk.extend_from_slice(payload);

    let mut out = Vec::new();
    out.extend_from_slice(b"~BK\0");
    out.extend_from_slice(&5u16.to_le_bytes()); // PSP_CHANNEL_BLOCK
    if major < 4 {
        out.extend_from_slice(&initial_len.to_le_bytes());
    }
    out.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
    out.extend_from_slice(&chunk);
    out
}

fn context(
    major: u16,
    model: PspColourModel,
    depth: u16,
    bps: u8,
    compression: PspCompression,
) -> PspImageContext {
    PspImageContext {
        version_major: major,
        colour_model: model,
        depth,
        bytes_per_sample: bps,
        compression,
    }
}

/// A layer read back through the real reader, so the fixtures cannot drift from M.9c's parsing.
fn one_layer(major: u16, spec: &Layer) -> redrob_core::psp::PspLayer {
    read_layer_bank(&bank(major, std::slice::from_ref(spec)), major)
        .unwrap()
        .remove(0)
}

#[test]
fn rgb_channels_land_in_red_green_blue_order_with_the_mask_last() {
    // Upstream's offset rule, on the case it was built for: red, green and blue go to 0, 1, 2 by
    // `(channel_type - PSP_CHANNEL_RED) * bytes_per_sample`, and a transparency bitmap goes to
    // `bytespp - bytes_per_sample`, which for RGBA is 3.
    let spec = Layer {
        saved: (0, 0, 2, 1),
        ..Default::default()
    };
    let layer = one_layer(4, &spec);

    let mut area = Vec::new();
    area.extend_from_slice(&channel_block(4, 0, 1, &[0x11, 0x22])); // red
    area.extend_from_slice(&channel_block(4, 0, 2, &[0x33, 0x44])); // green
    area.extend_from_slice(&channel_block(4, 0, 3, &[0x55, 0x66])); // blue
    area.extend_from_slice(&channel_block(4, 1, 0, &[0x77, 0x88])); // transparency mask

    let pixels = assemble_layer(
        &area,
        &layer,
        context(4, PspColourModel::Rgb, 24, 1, PspCompression::None),
        2, // bitmap_count > 1 -> alpha
    )
    .unwrap();

    assert_eq!(pixels.components, 4);
    assert_eq!(
        pixels.data,
        vec![0x11, 0x33, 0x55, 0x77, 0x22, 0x44, 0x66, 0x88]
    );
}

#[test]
fn a_grey_layer_with_alpha_is_the_one_place_this_diverges_from_upstream() {
    // Upstream's rule sends a composite channel to `bytespp - bytes_per_sample`. For a grey image
    // the only legal channel type IS composite, so with alpha that is the ALPHA slot -- upstream
    // writes the grey there and the transparency mask then overwrites it, losing the colour data.
    // The formula is built for RGB, where composite never arrives as colour data.
    //
    // This reader sends an IMAGE-bitmap composite channel to component 0 when the layer has alpha.
    // Everything else is upstream's.
    let spec = Layer {
        saved: (0, 0, 2, 1),
        ..Default::default()
    };
    let layer = one_layer(4, &spec);

    let mut area = Vec::new();
    area.extend_from_slice(&channel_block(4, 0, 0, &[0x11, 0x22])); // grey, IMAGE bitmap
    area.extend_from_slice(&channel_block(4, 1, 0, &[0xaa, 0xbb])); // transparency mask

    let pixels = assemble_layer(
        &area,
        &layer,
        context(4, PspColourModel::Gray, 8, 1, PspCompression::None),
        2,
    )
    .unwrap();

    assert_eq!(pixels.components, 2);
    // Grey in component 0, alpha in component 1 -- both survive.
    assert_eq!(pixels.data, vec![0x11, 0xaa, 0x22, 0xbb]);

    // WITHOUT alpha the divergence does not apply and upstream's rule is already right: one
    // component, so the last component IS component 0.
    let area = channel_block(4, 0, 0, &[0x11, 0x22]);
    let pixels = assemble_layer(
        &area,
        &layer,
        context(4, PspColourModel::Gray, 8, 1, PspCompression::None),
        1,
    )
    .unwrap();
    assert_eq!(pixels.components, 1);
    assert_eq!(pixels.data, vec![0x11, 0x22]);
}

#[test]
fn a_grey_image_may_carry_only_a_composite_channel() {
    // Upstream refuses `channel_type >= PSP_CHANNEL_RED` for anything that is not RGB, and
    // `channel_type > PSP_CHANNEL_BLUE` for RGB. Asymmetric, so both halves are asserted.
    let spec = Layer {
        saved: (0, 0, 2, 1),
        ..Default::default()
    };
    let layer = one_layer(4, &spec);

    for channel_type in [1u16, 2, 3] {
        let area = channel_block(4, 0, channel_type, &[0x11, 0x22]);
        assert!(
            assemble_layer(
                &area,
                &layer,
                context(4, PspColourModel::Gray, 8, 1, PspCompression::None),
                1
            )
            .is_err(),
            "channel type {channel_type} must be refused on a grey image"
        );
    }

    // RGB accepts 0 through 3 and refuses 4. Asserting the OFFSET and not merely success, because
    // the first version of this test only checked `is_ok()` and so did not notice that the grey
    // divergence above was also firing on RGB. An RGB composite channel keeps upstream's rule:
    // `bytespp - bytes_per_sample`, which for three components is 2.
    let rgb = context(4, PspColourModel::Rgb, 24, 1, PspCompression::None);
    let area = channel_block(4, 0, 0, &[0x11, 0x22]);
    let pixels = assemble_layer(&area, &layer, rgb, 1).unwrap();
    assert_eq!(
        pixels.data,
        vec![0x00, 0x00, 0x11, 0x00, 0x00, 0x22],
        "an RGB composite channel must still go to the last component"
    );

    for channel_type in [1u16, 2, 3] {
        let area = channel_block(4, 0, channel_type, &[0x11, 0x22]);
        let pixels = assemble_layer(&area, &layer, rgb, 1).unwrap();
        let at = (channel_type - 1) as usize;
        assert_eq!(pixels.data[at], 0x11, "channel type {channel_type}");
        assert_eq!(pixels.data[3 + at], 0x22, "channel type {channel_type}");
    }
    let area = channel_block(4, 0, 4, &[0x11, 0x22]);
    assert!(assemble_layer(&area, &layer, rgb, 1).is_err());
}

#[test]
fn the_two_unsupported_bitmap_kinds_are_reported_and_skipped_not_failed() {
    // Upstream prints a message for both and carries on: `bitmap_type > PSP_DIB_USER_MASK` is
    // "Conversion of bitmap type %d is not supported", and `== PSP_DIB_USER_MASK` is "Conversion
    // of layer mask is not supported" beside its own FIXME. Neither fails the load.
    let spec = Layer {
        saved: (0, 0, 2, 1),
        ..Default::default()
    };
    let layer = one_layer(4, &spec);

    let mut area = Vec::new();
    area.extend_from_slice(&channel_block(4, 0, 0, &[0x11, 0x22])); // the real data
    area.extend_from_slice(&channel_block(4, 2, 0, &[0xff, 0xff])); // user mask
    area.extend_from_slice(&channel_block(4, 7, 0, &[0xff, 0xff])); // an unsupported kind

    let pixels = assemble_layer(
        &area,
        &layer,
        context(4, PspColourModel::Gray, 8, 1, PspCompression::None),
        1,
    )
    .unwrap();

    // The data survived and the skips are reported rather than swallowed.
    assert_eq!(pixels.data, vec![0x11, 0x22]);
    assert_eq!(
        pixels.skipped,
        vec![
            PspSkippedChannel::LayerMask,
            PspSkippedChannel::UnsupportedBitmapType(7)
        ]
    );
}

#[test]
fn a_zero_height_layer_becomes_a_one_row_null_layer() {
    // Upstream: `if (height == 0) { height++; null_layer = TRUE; }`. A null layer allocates its
    // buffer and reads no channels at all, so it arrives as a one-row transparent strip rather
    // than failing the load.
    let spec = Layer {
        saved: (0, 0, 4, 0),
        ..Default::default()
    };
    let layer = one_layer(4, &spec);
    assert_eq!(layer.height, 0);

    // Channel data is present and must be IGNORED, which is what proves the null path ran.
    let area = channel_block(4, 0, 0, &[0xff, 0xff, 0xff, 0xff]);
    let pixels = assemble_layer(
        &area,
        &layer,
        context(4, PspColourModel::Gray, 8, 1, PspCompression::None),
        1,
    )
    .unwrap();
    assert_eq!(pixels.height, 1);
    assert_eq!(pixels.data, vec![0, 0, 0, 0]);
}

#[test]
fn the_layer_origin_is_the_sum_of_both_rectangles() {
    // Upstream: `image_rect[0] + saved_image_rect[0]`. So the saved rectangle's origin is RELATIVE
    // to the image rectangle's, which neither name suggests and which puts every layer in the
    // wrong place by a constant if read as absolute.
    let spec = Layer {
        saved: (3, 2, 7, 4),
        ..Default::default()
    };
    let layer = one_layer(4, &spec);
    // The fixture's image_rect is 0,0,8,4, so the sum is the saved origin here...
    assert_eq!(layer_origin(&layer), (3, 2));
    // ...and the test that matters is that it is a SUM, which the rect values make visible.
    assert_eq!(layer.image_rect.left, 0);
    assert_eq!(layer.saved_image_rect.left, 3);
}

#[test]
fn the_line_width_is_raised_to_the_padded_scanline_only_when_that_is_larger() {
    // Upstream's own words: "For small widths, when depth is 1, or 4, the number of bytes used can
    // be larger than the width * bytespp. Adjust for that." It is a MAXIMUM, not a branch.
    //
    // A 1-bit, 2-pixel indexed layer: width * bytespp is 2, the padded scanline is 4, so 4 wins.
    assert_eq!(layer_line_width(2, 1, 1, 1), 4);
    // A 1-bit, 64-pixel layer: width * bytespp is 64, the padded scanline is 8, so 64 wins.
    assert_eq!(layer_line_width(64, 1, 1, 1), 64);
    // At 8 bits and above the padding rule does not apply at all.
    assert_eq!(layer_line_width(2, 4, 1, 24), 8);
    assert_eq!(layer_line_width(2, 1, 2, 16), 4);
}

#[test]
fn version_three_and_version_four_channel_chunks_find_their_payload_differently() {
    // Third place in this format where the two versions answer the same question with different
    // fields: version 3 uses the block header's INITIAL length, version 4 an inner chunk length
    // that must be at least 16 -- which is exactly the field total.
    let spec = Layer {
        saved: (0, 0, 2, 1),
        ..Default::default()
    };

    for major in [3u16, 4] {
        let layer = one_layer(major, &spec);
        let area = channel_block(major, 0, 0, &[0x11, 0x22]);
        let pixels = assemble_layer(
            &area,
            &layer,
            context(major, PspColourModel::Gray, 8, 1, PspCompression::None),
            1,
        )
        .unwrap();
        assert_eq!(pixels.data, vec![0x11, 0x22], "version {major}");
    }

    // A version-4 chunk length below 16 is refused.
    let layer = one_layer(4, &spec);
    let mut area = channel_block(4, 0, 0, &[0x11, 0x22]);
    area[10..14].copy_from_slice(&15u32.to_le_bytes());
    assert!(
        assemble_layer(
            &area,
            &layer,
            context(4, PspColourModel::Gray, 8, 1, PspCompression::None),
            1
        )
        .is_err()
    );
}

#[test]
fn assembly_runs_rle_and_lz77_channels_through_the_same_offsets() {
    let spec = Layer {
        saved: (0, 0, 4, 1),
        ..Default::default()
    };
    let layer = one_layer(4, &spec);

    // RLE: a run of 4 of 0x5a.
    let area = channel_block(4, 0, 0, &[132u8, 0x5a]);
    let pixels = assemble_layer(
        &area,
        &layer,
        context(4, PspColourModel::Gray, 8, 1, PspCompression::Rle),
        1,
    )
    .unwrap();
    assert_eq!(pixels.data, vec![0x5a; 4]);

    // LZ77: the same four bytes as a zlib stream.
    let stream = zlib(&[0x5a, 0x5a, 0x5a, 0x5a]);
    let area = channel_block(4, 0, 0, &stream);
    let pixels = assemble_layer(
        &area,
        &layer,
        context(4, PspColourModel::Gray, 8, 1, PspCompression::Lz77),
        1,
    )
    .unwrap();
    assert_eq!(pixels.data, vec![0x5a; 4]);
}

#[test]
fn a_layer_sub_block_that_is_not_a_channel_is_refused() {
    let spec = Layer {
        saved: (0, 0, 2, 1),
        ..Default::default()
    };
    let layer = one_layer(4, &spec);
    let mut area = channel_block(4, 0, 0, &[0x11, 0x22]);
    area[4..6].copy_from_slice(&9u16.to_le_bytes()); // not PSP_CHANNEL_BLOCK
    let error = assemble_layer(
        &area,
        &layer,
        context(4, PspColourModel::Gray, 8, 1, PspCompression::None),
        1,
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("not a channel"), "{error}");
}

// ---------------------------------------------------------------------------------------------
// M.9f: the creator block and the colour profile.
// ---------------------------------------------------------------------------------------------

use redrob_core::psp::{PspCreator, read_colour_profile, read_creator};

/// One creator field chunk. Note the signature: `~FL\0`, not `~BK\0`.
fn field(keyword: u16, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"~FL\0");
    out.extend_from_slice(&keyword.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    out
}

#[test]
fn creator_fields_use_their_own_signature() {
    // A reader that reuses the block-header walk rejects every creator block, because the field
    // chunks carry `~FL\0` rather than `~BK\0`. Asserted in both directions.
    let mut data = field(0, b"Title");
    assert_eq!(&data[..4], b"~FL\0");
    let creator = read_creator(&data).unwrap();
    assert_eq!(creator.title.as_deref(), Some("Title"));

    data[..4].copy_from_slice(b"~BK\0");
    let error = read_creator(&data).unwrap_err().to_string();
    assert!(error.contains("signature missing"), "{error}");
}

#[test]
fn the_comment_is_assembled_in_a_fixed_order_with_a_literal_copyright_prefix() {
    // Upstream's order is title, artist, "Copyright " then the holder, description -- each followed
    // by a newline. The prefix is a literal in upstream's source, not part of the stored string.
    let mut data = Vec::new();
    data.extend_from_slice(&field(5, b"A description")); // written FIRST in the file
    data.extend_from_slice(&field(4, b"Someone"));
    data.extend_from_slice(&field(3, b"An artist"));
    data.extend_from_slice(&field(0, b"A title"));

    let creator = read_creator(&data).unwrap();
    assert_eq!(
        creator.comment().as_deref(),
        Some("A title\nAn artist\nCopyright Someone\nA description\n"),
        "the file's order must not change the comment's order"
    );
}

#[test]
fn an_empty_creator_block_leaves_no_comment_at_all() {
    // Upstream attaches the parasite only when the assembled comment is non-empty, so an absent
    // creator block and an all-empty one behave the same.
    assert_eq!(read_creator(&[]).unwrap(), PspCreator::default());
    assert_eq!(read_creator(&[]).unwrap().comment(), None);

    // A block holding only fields upstream ignores is also empty.
    let mut data = Vec::new();
    data.extend_from_slice(&field(1, &[0, 0, 0, 0])); // creation date
    data.extend_from_slice(&field(6, &[0, 0, 0, 0])); // application id
    assert_eq!(read_creator(&data).unwrap().comment(), None);
}

#[test]
fn the_four_numeric_creator_fields_are_skipped_because_upstream_never_uses_them() {
    // Creation date, modification date, application id and application version are all declared
    // `guint32 __attribute__((unused))` upstream: read, assigned, and then nothing. Dead reads,
    // the same shape as mantiuk06's unread `detail` property.
    //
    // What matters here is that skipping them does not disturb the walk: a string field AFTER them
    // must still be found.
    let mut data = Vec::new();
    data.extend_from_slice(&field(1, &[1, 2, 3, 4])); // creation date
    data.extend_from_slice(&field(2, &[5, 6, 7, 8])); // modification date
    data.extend_from_slice(&field(0, b"Survived"));
    data.extend_from_slice(&field(7, &[9, 10, 11, 12])); // application version
    data.extend_from_slice(&field(3, b"Also survived"));

    let creator = read_creator(&data).unwrap();
    assert_eq!(creator.title.as_deref(), Some("Survived"));
    assert_eq!(creator.artist.as_deref(), Some("Also survived"));
}

#[test]
fn a_numeric_field_of_an_unexpected_length_does_not_desynchronise_the_walk() {
    // **This is the one place M.9f does not follow upstream's code exactly.** Upstream's
    // known-numeric branch reads exactly FOUR bytes whatever the declared length says, while its
    // unknown-keyword branch advances by the declared length. So a creation-date field declaring
    // six bytes leaves upstream two bytes out of step and every later field misparsed.
    //
    // Advancing by the declared length is the only reading that cannot desynchronise, and it agrees
    // with upstream wherever upstream agrees with itself.
    let mut data = Vec::new();
    data.extend_from_slice(&field(1, &[1, 2, 3, 4, 5, 6])); // a six-byte creation date
    data.extend_from_slice(&field(0, b"Still found"));

    let creator = read_creator(&data).unwrap();
    assert_eq!(creator.title.as_deref(), Some("Still found"));
}

#[test]
fn a_repeated_keyword_keeps_the_last_one() {
    // Upstream frees the previous value before assigning, so the last field through the loop wins.
    let mut data = Vec::new();
    data.extend_from_slice(&field(0, b"First"));
    data.extend_from_slice(&field(0, b"Second"));
    assert_eq!(
        read_creator(&data).unwrap().title.as_deref(),
        Some("Second")
    );
}

#[test]
fn creator_strings_are_latin_one_and_are_not_nul_terminated() {
    // Upstream's own note: "PSP does not zero terminate strings" -- the length is the only
    // terminator, so a reader hunting for a NUL runs into the next field. And the conversion is
    // from ISO-8859-1 despite the PSP8 specification calling the strings ASCII.
    let data = field(0, &[0x41, 0xe9, 0x42]); // A, e-acute in latin-1, B
    let creator = read_creator(&data).unwrap();
    assert_eq!(creator.title.as_deref(), Some("AéB"));
    // Those bytes are not valid UTF-8, which is what makes the encoding claim testable.
    assert!(String::from_utf8(vec![0x41, 0xe9, 0x42]).is_err());

    // A string containing a NUL keeps it rather than stopping there.
    let data = field(0, b"a\0b");
    assert_eq!(read_creator(&data).unwrap().title.as_deref(), Some("a\0b"));
}

#[test]
fn a_creator_field_running_past_its_block_is_refused() {
    let mut data = field(0, b"Title");
    data[6..10].copy_from_slice(&999u32.to_le_bytes());
    let error = read_creator(&data).unwrap_err().to_string();
    assert!(error.contains("past its block"), "{error}");
}

#[test]
fn the_icc_profile_size_lives_in_the_last_four_bytes_of_a_variable_header() {
    // Upstream's `psp_header_size - 8` says it: four bytes are already consumed by reading the
    // header size, four more are about to be read at the end, and everything between is PSP's own
    // name for the profile, which upstream does not want.
    let name = b"PSP internal profile name";
    let profile = b"fake-icc-bytes";

    let mut data = Vec::new();
    // header_size counts itself (4), the name, and the profile size (4).
    let header_size = 4 + name.len() + 4;
    data.extend_from_slice(&(header_size as u32).to_le_bytes());
    data.extend_from_slice(name);
    data.extend_from_slice(&(profile.len() as u32).to_le_bytes());
    data.extend_from_slice(profile);

    assert_eq!(read_colour_profile(&data).unwrap(), profile.to_vec());
}

#[test]
fn a_header_size_below_eight_is_refused_rather_than_rewinding() {
    // Upstream's seek is `SEEK_CUR` with `psp_header_size - 8`, so a header size under 8 moves
    // BACKWARDS over bytes already read. A rewind is never what the format meant.
    for header_size in [0u32, 1, 7] {
        let mut data = Vec::new();
        data.extend_from_slice(&header_size.to_le_bytes());
        data.extend_from_slice(&[0u8; 32]);
        let error = read_colour_profile(&data).unwrap_err().to_string();
        assert!(error.contains("too small"), "header {header_size}: {error}");
    }

    // Exactly 8 is legal and means an empty name.
    let mut data = Vec::new();
    data.extend_from_slice(&8u32.to_le_bytes());
    data.extend_from_slice(&4u32.to_le_bytes()); // profile size
    data.extend_from_slice(b"abcd");
    assert_eq!(read_colour_profile(&data).unwrap(), b"abcd".to_vec());
}

#[test]
fn a_truncated_icc_profile_is_refused() {
    let mut data = Vec::new();
    data.extend_from_slice(&8u32.to_le_bytes());
    data.extend_from_slice(&100u32.to_le_bytes()); // claims 100 bytes
    data.extend_from_slice(b"short");
    assert!(read_colour_profile(&data).is_err());

    // And a header that runs past the block.
    let mut data = Vec::new();
    data.extend_from_slice(&500u32.to_le_bytes());
    data.extend_from_slice(b"short");
    assert!(read_colour_profile(&data).is_err());
}

// ---------------------------------------------------------------------------------------------
// M.9g: the selection block and the picture tube.
// ---------------------------------------------------------------------------------------------

use redrob_core::psp::{read_selection, read_tube, tube_cell_size};

/// A forward-compatible chunk: its declared size, the fields, then `surplus` bytes of padding the
/// reader must skip.
fn forward_chunk(fields: &[u8], surplus: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let declared = 4 + fields.len() + surplus;
    out.extend_from_slice(&(declared as u32).to_le_bytes());
    out.extend_from_slice(fields);
    out.extend(std::iter::repeat_n(0xcdu8, surplus));
    out
}

/// A whole selection block: the rect chunk, the channel-count chunk, then one channel block.
fn selection_block(major: u16, rect: (u32, u32, u32, u32), mask: &[u8], surplus: usize) -> Vec<u8> {
    let mut fields = Vec::new();
    fields.extend_from_slice(&rect.0.to_le_bytes());
    fields.extend_from_slice(&rect.1.to_le_bytes());
    fields.extend_from_slice(&rect.2.to_le_bytes());
    fields.extend_from_slice(&rect.3.to_le_bytes());
    // saved_rect: deliberately given DIFFERENT values, so a reader that takes the size from it
    // produces the wrong answer rather than the right one by coincidence.
    for value in [99u32, 99, 199, 199] {
        fields.extend_from_slice(&value.to_le_bytes());
    }

    let mut out = forward_chunk(&fields, surplus);
    out.extend_from_slice(&forward_chunk(&1u32.to_le_bytes(), surplus)); // one channel
    out.extend_from_slice(&channel_block(major, 0, 0, mask));
    out
}

#[test]
fn the_selection_takes_its_size_from_rect_and_a_layer_takes_it_from_saved_rect() {
    // THE asymmetry of this block. Upstream computes the selection's size from `rect` and a
    // layer's from `saved_image_rect` -- the same pair of rectangles, read the other way round. A
    // reader that factors "read a rect pair, take the size" into one helper gets one of them wrong.
    //
    // The fixture's saved_rect is 99,99,199,199, so taking the size from it would give 100 x 100.
    let data = selection_block(4, (2, 1, 10, 5), &[0u8; 32], 0);
    let selection = read_selection(
        &data,
        context(4, PspColourModel::Gray, 8, 1, PspCompression::None),
    )
    .unwrap();

    assert_eq!((selection.width, selection.height), (8, 4));
    assert_eq!(selection.origin, (2, 1));
    assert_eq!(selection.mask.len(), 32);
}

#[test]
fn a_chunk_longer_than_its_known_fields_has_its_surplus_skipped() {
    // The selection block's whole structure, three times over: read a declared size, read the
    // fields, SKIP the remainder. A chunk longer than the fields a reader knows is normal -- it is
    // how the format adds fields without breaking old readers -- and treating the surplus as the
    // next field misparses everything after it.
    for surplus in [0usize, 1, 7, 64] {
        let data = selection_block(4, (0, 0, 4, 2), &[0xab; 8], surplus);
        let selection = read_selection(
            &data,
            context(4, PspColourModel::Gray, 8, 1, PspCompression::None),
        )
        .unwrap_or_else(|e| panic!("surplus {surplus}: {e}"));
        assert_eq!((selection.width, selection.height), (4, 2));
        assert_eq!(selection.mask, vec![0xab; 8], "surplus {surplus}");
    }
}

#[test]
fn a_chunk_declaring_less_than_its_fields_is_refused() {
    let mut data = selection_block(4, (0, 0, 4, 2), &[0u8; 8], 0);
    // The first chunk really carries 4 + 32 bytes; claim 20.
    data[..4].copy_from_slice(&20u32.to_le_bytes());
    let error = read_selection(
        &data,
        context(4, PspColourModel::Gray, 8, 1, PspCompression::None),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("less than the fields"), "{error}");
}

#[test]
fn an_empty_selection_rectangle_is_refused() {
    for rect in [(4u32, 0u32, 4u32, 2u32), (0, 2, 4, 2), (6, 0, 4, 2)] {
        let data = selection_block(4, rect, &[0u8; 8], 0);
        assert!(
            read_selection(
                &data,
                context(4, PspColourModel::Gray, 8, 1, PspCompression::None)
            )
            .is_err(),
            "rect {rect:?} must be refused"
        );
    }
}

#[test]
fn an_uncompressed_selection_mask_does_not_use_the_padded_scanline() {
    // Upstream reads `width * height` bytes directly here rather than going through its channel
    // reader, so the 4-byte scanline padding that applies to a layer's sub-8-bit channels does not
    // apply to a selection mask. A 1-pixel-wide mask is one byte per row, not four.
    let data = selection_block(4, (0, 0, 1, 3), &[0x11, 0x22, 0x33], 0);
    let selection = read_selection(
        &data,
        context(4, PspColourModel::Indexed, 1, 1, PspCompression::None),
    )
    .unwrap();
    assert_eq!(selection.mask, vec![0x11, 0x22, 0x33]);
}

#[test]
fn a_selection_sub_block_that_is_not_a_channel_is_refused() {
    let mut data = selection_block(4, (0, 0, 4, 2), &[0u8; 8], 0);
    // Find the channel block's id field: after the two chunks.
    let two_chunks = (4 + 32) + (4 + 4);
    data[two_chunks + 4..two_chunks + 6].copy_from_slice(&9u16.to_le_bytes());
    let error = read_selection(
        &data,
        context(4, PspColourModel::Gray, 8, 1, PspCompression::None),
    )
    .unwrap_err()
    .to_string();
    assert!(error.contains("not a channel"), "{error}");
}

#[test]
fn a_compressed_selection_mask_goes_through_the_same_decompressors() {
    // RLE: a run of 8 of 0x7f.
    let data = selection_block(4, (0, 0, 4, 2), &[136u8, 0x7f], 0);
    let selection = read_selection(
        &data,
        context(4, PspColourModel::Gray, 8, 1, PspCompression::Rle),
    )
    .unwrap();
    assert_eq!(selection.mask, vec![0x7f; 8]);

    // LZ77.
    let stream = zlib(&[0x5a; 8]);
    let data = selection_block(4, (0, 0, 4, 2), &stream, 0);
    let selection = read_selection(
        &data,
        context(4, PspColourModel::Gray, 8, 1, PspCompression::Lz77),
    )
    .unwrap();
    assert_eq!(selection.mask, vec![0x5a; 8]);
}

/// A tube block in the given version's shape.
fn tube_block(major: u16, cols: u32, rows: u32, placement: u32, selection: u32) -> Vec<u8> {
    let mut out = Vec::new();
    if major >= 4 {
        out.extend_from_slice(&30u32.to_le_bytes()); // chunk length
        out.extend_from_slice(&1u16.to_le_bytes()); // version, which upstream never uses
    } else {
        out.extend_from_slice(&1u16.to_le_bytes()); // version
        out.extend(std::iter::repeat_n(0x41u8, 513)); // the 513-byte name, also never used
    }
    out.extend_from_slice(&7u32.to_le_bytes()); // step size
    out.extend_from_slice(&cols.to_le_bytes());
    out.extend_from_slice(&rows.to_le_bytes());
    out.extend_from_slice(&12u32.to_le_bytes()); // cell count
    out.extend_from_slice(&placement.to_le_bytes());
    out.extend_from_slice(&selection.to_le_bytes());
    out
}

#[test]
fn version_four_drops_the_tube_name_entirely() {
    // A DIFFERENT KIND of version difference from the three already in this format. The block
    // header, the layer name and the channel chunk all RESHAPED -- same information, different
    // form. Here version 3 carries a 513-byte name and version 4 carries no name at all, replacing
    // it with a chunk length. The two blocks are different LENGTHS for the same tube.
    let v3 = tube_block(3, 4, 3, 0, 1);
    let v4 = tube_block(4, 4, 3, 0, 1);
    assert_eq!(v3.len(), 2 + 513 + 24);
    assert_eq!(v4.len(), 4 + 2 + 24);

    let a = read_tube(
        &v3,
        context(3, PspColourModel::Rgb, 24, 1, PspCompression::None),
    )
    .unwrap();
    let b = read_tube(
        &v4,
        context(4, PspColourModel::Rgb, 24, 1, PspCompression::None),
    )
    .unwrap();
    assert_eq!(a, b, "the same tube must read the same from either shape");
    assert_eq!(a.columns, 4);
    assert_eq!(a.rows, 3);
    assert_eq!(a.step_size, 7);
    assert_eq!(a.cell_count, 12);
}

#[test]
fn the_tube_guides_sit_between_the_cells_and_not_at_the_edges() {
    // Upstream's loops run `i` from 1 to cols-1, so there are cols-1 guides and none at 0 or at
    // the far edge. That is what makes this a grid of cells rather than a set of borders.
    let tube = read_tube(
        &tube_block(4, 4, 2, 0, 0),
        context(4, PspColourModel::Rgb, 24, 1, PspCompression::None),
    )
    .unwrap();

    assert_eq!(tube.vertical_guides(100), vec![25, 50, 75]);
    assert_eq!(tube.horizontal_guides(100), vec![50]);

    // One column means no guides at all, not one at zero.
    let tube = read_tube(
        &tube_block(4, 1, 1, 0, 0),
        context(4, PspColourModel::Rgb, 24, 1, PspCompression::None),
    )
    .unwrap();
    assert!(tube.vertical_guides(100).is_empty());
    assert!(tube.horizontal_guides(100).is_empty());
}

#[test]
fn the_guides_and_cells_both_use_integer_division() {
    // 100 across 3 columns: the guides land at 33 and 66, not 33.3, and the cells are 33 wide --
    // so three cells cover 99 of the 100 pixels and upstream does not correct the remainder.
    let tube = read_tube(
        &tube_block(4, 3, 3, 0, 0),
        context(4, PspColourModel::Rgb, 24, 1, PspCompression::None),
    )
    .unwrap();
    assert_eq!(tube.vertical_guides(100), vec![33, 66]);
    assert_eq!(tube_cell_size(&tube, 100, 100), (33, 33));
}

#[test]
fn the_mode_values_map_to_upstreams_strings_with_its_own_default_fallback() {
    let read = |placement, selection| {
        read_tube(
            &tube_block(4, 2, 2, placement, selection),
            context(4, PspColourModel::Rgb, 24, 1, PspCompression::None),
        )
        .unwrap()
    };

    assert_eq!(read(0, 0).placement, "random");
    assert_eq!(read(1, 0).placement, "constant");
    // Upstream's ternary chain ends in "default" for anything it does not recognise.
    assert_eq!(read(2, 0).placement, "default");
    assert_eq!(read(999, 0).placement, "default");

    assert_eq!(read(0, 0).selection, "random");
    assert_eq!(read(0, 1).selection, "incremental");
    assert_eq!(read(0, 2).selection, "angular");
    assert_eq!(read(0, 3).selection, "pressure");
    assert_eq!(read(0, 4).selection, "velocity");
    assert_eq!(read(0, 5).selection, "default");
}

#[test]
fn a_tube_with_no_columns_is_refused_rather_than_dividing_by_zero() {
    // Upstream divides the image width by params.cols without checking it. A tube with no columns
    // is not a tube, so this refuses instead.
    for (cols, rows) in [(0u32, 2u32), (2, 0), (0, 0)] {
        let data = tube_block(4, cols, rows, 0, 0);
        assert!(
            read_tube(
                &data,
                context(4, PspColourModel::Rgb, 24, 1, PspCompression::None)
            )
            .is_err(),
            "{cols}x{rows} must be refused"
        );
    }
}

#[test]
fn a_truncated_tube_block_is_refused_in_either_shape() {
    for major in [3u16, 4] {
        let mut data = tube_block(major, 2, 2, 0, 0);
        data.truncate(data.len() - 1);
        assert!(
            read_tube(
                &data,
                context(major, PspColourModel::Rgb, 24, 1, PspCompression::None)
            )
            .is_err(),
            "version {major}"
        );
    }
}
