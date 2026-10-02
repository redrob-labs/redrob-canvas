// SPDX-License-Identifier: GPL-3.0-or-later

use std::io::{Cursor, Read, Write};

use image::{ColorType, ImageEncoder};
use redrob_core::{
    AlphaPolicy, BlendMode, DocumentImportBuilder, DocumentMetadata, EMBEDDED_FONT_ID, Editor,
    ExportOptions, FileFormat, FormatError, FormatWarning, FrameId, ImportMask, ImportNode,
    ImportOptions, LossPolicy, PathCommand, Pixel, PlaybackMetadata, RasterCel, RenderSnapshot,
    TextContent, VectorContent, VectorPath, detect_format, export_document, export_png,
    import_document, import_png,
};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

fn raster_document(width: u32, height: u32, pixels: Vec<u8>) -> redrob_core::Document {
    let mut builder = DocumentImportBuilder::new(width, height).unwrap();
    builder
        .push_node(ImportNode::raster(
            "pixels",
            vec![RasterCel::new(FrameId::DEFAULT, pixels)],
        ))
        .unwrap();
    builder.build().unwrap()
}

fn png(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(pixels, width, height, ColorType::Rgba8.into())
        .unwrap();
    bytes
}

#[test]
fn detection_is_content_based_and_expected_format_is_strict() {
    let bytes = png(1, 1, &[1, 2, 3, 4]);
    assert_eq!(detect_format(&bytes).unwrap(), FileFormat::Png);
    let error = import_document(
        &bytes,
        &ImportOptions::default().with_expected_format(FileFormat::Jpeg),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        redrob_core::CoreError::Format(FormatError::FormatMismatch {
            expected: FileFormat::Jpeg,
            detected: FileFormat::Png
        })
    ));
    assert!(matches!(
        detect_format(b"not an image"),
        Err(FormatError::UnknownFormat)
    ));
}

#[test]
fn legacy_png_wrappers_remain_alpha_preserving_and_png_only() {
    let source = [200, 10, 30, 77];
    let bytes = png(1, 1, &source);
    let document = import_png(&bytes).unwrap();
    let encoded = export_png(&document).unwrap();
    let imported = import_png(&encoded).unwrap();
    assert_eq!(imported.layers()[0].pixels(), source);
    assert!(import_png(&[0xff, 0xd8, 0xff, 0xd9]).is_err());
}

#[test]
fn png_and_lossless_webp_preserve_rgba_pixels() {
    let pixels = vec![255, 9, 7, 0, 1, 2, 3, 127, 4, 5, 6, 255, 9, 8, 7, 33];
    let document = raster_document(2, 2, pixels.clone());
    for format in [FileFormat::Png, FileFormat::WebP] {
        let encoded = export_document(&document, format, &ExportOptions::default()).unwrap();
        assert_eq!(detect_format(encoded.bytes()).unwrap(), format);
        let decoded = import_document(encoded.bytes(), &ImportOptions::default()).unwrap();
        assert_eq!(decoded.document().layers()[0].pixels(), pixels);
        assert!(encoded.metadata().lossless);
    }
}

#[test]
fn jpeg_requires_explicit_alpha_handling_and_valid_quality() {
    let document = raster_document(1, 1, vec![200, 0, 0, 128]);
    let error =
        export_document(&document, FileFormat::Jpeg, &ExportOptions::default()).unwrap_err();
    assert!(matches!(
        error,
        redrob_core::CoreError::Format(FormatError::LossRequired(_))
    ));

    let error = export_document(
        &document,
        FileFormat::Jpeg,
        &ExportOptions::default()
            .with_alpha_policy(AlphaPolicy::Flatten {
                matte: Pixel::rgba(0, 0, 255, 255),
            })
            .with_jpeg_quality(0),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        redrob_core::CoreError::Format(FormatError::InvalidOption(_))
    ));

    let encoded = export_document(
        &document,
        FileFormat::Jpeg,
        &ExportOptions::default()
            .with_alpha_policy(AlphaPolicy::Flatten {
                matte: Pixel::rgba(0, 0, 255, 255),
            })
            .with_jpeg_quality(100),
    )
    .unwrap();
    let decoded = import_document(encoded.bytes(), &ImportOptions::default()).unwrap();
    let pixel = decoded.document().layers()[0].pixels();
    assert_eq!(pixel[3], 255);
    assert!((i16::from(pixel[0]) - 100).abs() < 15);
    assert!((i16::from(pixel[2]) - 127).abs() < 15);
    assert_eq!(encoded.metadata().jpeg_quality, Some(100));
    assert!(encoded.warnings().contains(&FormatWarning::FlattenedAlpha {
        matte: Pixel::rgba(0, 0, 255, 255),
    }));
}

#[test]
fn explicit_frame_render_and_export_do_not_mutate_editor_state() {
    let frame = FrameId::new(7);
    let mut builder = DocumentImportBuilder::new(1, 1).unwrap();
    builder
        .timeline(
            vec![FrameId::DEFAULT, frame],
            10.0,
            FrameId::DEFAULT,
            PlaybackMetadata {
                range_start: FrameId::DEFAULT,
                range_end: frame,
                looping: false,
                playing: false,
            },
        )
        .unwrap();
    builder
        .push_node(ImportNode::raster(
            "animated",
            vec![
                RasterCel::new(FrameId::DEFAULT, vec![255, 0, 0, 255]),
                RasterCel::new(frame, vec![0, 255, 0, 255]),
            ],
        ))
        .unwrap();
    let document = builder.build().unwrap();
    let editor = Editor::new(document.clone()).unwrap();
    let generation = editor.generation();
    let current = editor.document().current_frame_id();
    let snapshot = editor.render_frame_snapshot(frame).unwrap();
    assert_eq!(snapshot.pixels(), [0, 255, 0, 255]);
    assert_eq!(editor.generation(), generation);
    assert_eq!(editor.document().current_frame_id(), current);
    assert!(!editor.can_undo() && !editor.can_redo());

    let outcome = export_document(
        &document,
        FileFormat::Png,
        &ExportOptions::default()
            .with_frame(frame)
            .with_loss_policy(LossPolicy::AllowLoss),
    )
    .unwrap();
    assert!(
        outcome
            .warnings()
            .contains(&FormatWarning::OmittedFrames { exported: frame })
    );
    assert_eq!(document.current_frame_id(), current);
    assert!(matches!(
        RenderSnapshot::try_render_frame(&document, 0, FrameId::new(999)),
        Err(redrob_core::CoreError::FrameNotFound(_))
    ));
}

#[test]
fn import_builder_is_atomic_and_canonicalizes_bottom_to_top_postorder() {
    let mut builder = DocumentImportBuilder::new(1, 1).unwrap();
    let group = ImportNode::group("group");
    let group_id = group.id();
    builder.push_node(group).unwrap();
    builder
        .push_node(
            ImportNode::raster(
                "bottom",
                vec![RasterCel::new(FrameId::DEFAULT, vec![255, 0, 0, 255])],
            )
            .with_parent(Some(group_id)),
        )
        .unwrap();
    builder
        .push_node(
            ImportNode::raster(
                "top",
                vec![RasterCel::new(FrameId::DEFAULT, vec![0, 0, 255, 128])],
            )
            .with_parent(Some(group_id)),
        )
        .unwrap();
    let document = builder.build().unwrap();
    assert_eq!(
        document
            .nodes()
            .iter()
            .map(|node| node.name())
            .collect::<Vec<_>>(),
        ["bottom", "top", "group"]
    );
    assert_eq!(
        RenderSnapshot::try_render_frame(&document, 0, FrameId::DEFAULT)
            .unwrap()
            .pixels(),
        [127, 0, 128, 255]
    );

    let mut invalid = DocumentImportBuilder::new(1, 1).unwrap();
    invalid
        .push_node(ImportNode::raster(
            "bad",
            vec![RasterCel::new(FrameId::DEFAULT, vec![0; 3])],
        ))
        .unwrap();
    assert!(invalid.build().is_err());
}

fn ora_fixture(stack_xml: &str, entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file(
            "mimetype",
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
        )
        .unwrap();
    writer.write_all(b"image/openraster").unwrap();
    writer
        .start_file("stack.xml", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(stack_xml.as_bytes()).unwrap();
    for (name, bytes) in entries {
        writer
            .start_file(*name, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

#[test]
fn hand_built_ora_imports_top_first_hierarchy_blend_and_offsets() {
    let stack = r#"<?xml version="1.0"?><image version="0.0.1" w="2" h="1"><stack name="root"><stack name="group" opacity="0.5" composite-op="svg:multiply"><layer name="top" src="data/top.png" x="1" y="0" composite-op="svg:screen"/><layer name="bottom" src="data/bottom.png" x="0" y="0"/></stack></stack></image>"#;
    let bytes = ora_fixture(
        stack,
        &[
            ("data/top.png", png(1, 1, &[0, 0, 255, 255])),
            ("data/bottom.png", png(1, 1, &[255, 0, 0, 255])),
        ],
    );
    let imported = import_document(&bytes, &ImportOptions::default()).unwrap();
    let names = imported
        .document()
        .nodes()
        .iter()
        .map(|node| node.name())
        .collect::<Vec<_>>();
    assert_eq!(names, ["bottom", "top", "group"]);
    assert_eq!(
        imported.document().nodes()[1].blend_mode(),
        BlendMode::Screen
    );
    assert_eq!(
        imported.document().nodes()[2].blend_mode(),
        BlendMode::Multiply
    );
    assert_eq!(imported.document().nodes()[2].opacity(), 0.5);
    let rendered =
        RenderSnapshot::try_render_frame(imported.document(), 0, FrameId::DEFAULT).unwrap();
    assert_eq!(rendered.pixels()[3], 128);
    assert_eq!(rendered.pixels()[7], 128);
}

#[test]
fn ora_export_is_deterministic_and_roundtrips_render() {
    let document = raster_document(2, 1, vec![1, 2, 3, 4, 5, 6, 7, 255]);
    let first = export_document(&document, FileFormat::Ora, &ExportOptions::default()).unwrap();
    let second = export_document(&document, FileFormat::Ora, &ExportOptions::default()).unwrap();
    assert_eq!(first.bytes(), second.bytes());
    let mut archive = ZipArchive::new(Cursor::new(first.bytes())).unwrap();
    let first_entry = archive.by_index(0).unwrap();
    assert_eq!(first_entry.name(), "mimetype");
    assert_eq!(first_entry.compression(), CompressionMethod::Stored);
    drop(first_entry);
    assert!(archive.by_name("stack.xml").is_ok());
    assert!(archive.by_name("mergedimage.png").is_ok());

    let imported = import_document(first.bytes(), &ImportOptions::default()).unwrap();
    assert_eq!(
        RenderSnapshot::try_render_frame(imported.document(), 0, FrameId::DEFAULT)
            .unwrap()
            .pixels(),
        RenderSnapshot::try_render_frame(&document, 0, FrameId::DEFAULT)
            .unwrap()
            .pixels()
    );
}

#[test]
fn ora_rejects_malformed_root_containment_with_typed_errors() {
    let pixel = png(1, 1, &[0, 0, 0, 0]);
    let cases = [
        (
            r#"<image w="1" h="1"></image><stack><layer src="data/a.png"/></stack>"#,
            "unbalanced ORA image root",
        ),
        (
            r#"<stack><layer src="data/a.png"/></stack><image w="1" h="1"><stack><layer src="data/a.png"/></stack></image>"#,
            "ORA stack outside image",
        ),
        (
            r#"<image w="1" h="1"><stack><layer src="data/a.png"/></image></stack>"#,
            "invalid ORA XML",
        ),
        (
            r#"<image w="1" h="1"></image>"#,
            "unbalanced ORA image root",
        ),
        (
            r#"<image w="1" h="1"><stack><layer src="data/a.png"/></stack><stack><layer src="data/a.png"/></stack></image>"#,
            "multiple ORA root stacks",
        ),
        (
            r#"<image w="1" h="1"><stack><layer src="data/a.png"/></stack></stack></image>"#,
            "invalid ORA XML",
        ),
        (
            r#"<image w="1" h="1"><stack><layer src="data/a.png"/></stack></image></image>"#,
            "invalid ORA XML",
        ),
        (
            r#"<image w="1" h="1"><stack><layer src="data/a.png"/></stack>"#,
            "unclosed ORA image or stack",
        ),
        (
            r#"<image w="1" h="1"><stack><layer src="data/a.png"/></stack></image><extra/>"#,
            "content after ORA image root",
        ),
    ];
    for (xml, expected) in cases {
        let fixture = ora_fixture(xml, &[("data/a.png", pixel.clone())]);
        let error = import_document(&fixture, &ImportOptions::default()).unwrap_err();
        assert!(
            matches!(error, redrob_core::CoreError::Format(FormatError::Malformed(reason)) if reason == expected),
            "expected malformed {expected:?}, got {error:?} for {xml}"
        );
    }
}

#[test]
fn ora_rejects_traversal_duplicate_sources_and_dtd() {
    let traversal = ora_fixture(
        r#"<image w="1" h="1"><stack><layer src="../bad.png"/></stack></image>"#,
        &[],
    );
    assert!(matches!(
        import_document(&traversal, &ImportOptions::default()),
        Err(redrob_core::CoreError::Format(
            FormatError::UnsupportedFeature("unknown or unsafe ORA layer attribute")
        ))
    ));

    let duplicate = ora_fixture(
        r#"<image w="1" h="1"><stack><layer src="data/a.png"/><layer src="data/a.png"/></stack></image>"#,
        &[("data/a.png", png(1, 1, &[0, 0, 0, 0]))],
    );
    assert!(matches!(
        import_document(&duplicate, &ImportOptions::default()),
        Err(redrob_core::CoreError::Format(FormatError::Malformed(
            "duplicate ORA layer source"
        )))
    ));

    let duplicate_xml = br#"<image w="1" h="1"><stack><layer src="data/a.png"/></stack></image>"#;
    let duplicate_png = png(1, 1, &[0, 0, 0, 0]);
    let duplicate_entry = raw_stored_zip(&[
        ("mimetype", b"image/openraster"),
        ("stack.xml", duplicate_xml),
        ("data/a.png", &duplicate_png),
        ("data/a.png", &duplicate_png),
    ]);
    assert!(matches!(
        import_document(&duplicate_entry, &ImportOptions::default()),
        Err(redrob_core::CoreError::Format(FormatError::Malformed(
            "unsafe or duplicate ORA path"
        )))
    ));

    let dtd = ora_fixture(
        r#"<!DOCTYPE image [<!ENTITY x "x">]><image w="1" h="1"><stack><layer name="&x;" src="data/a.png"/></stack></image>"#,
        &[("data/a.png", png(1, 1, &[0, 0, 0, 0]))],
    );
    assert!(matches!(
        import_document(&dtd, &ImportOptions::default()),
        Err(redrob_core::CoreError::Format(
            FormatError::UnsupportedFeature("ORA DTD/entity content")
        ))
    ));
}

#[test]
fn svg_subset_accepts_shapes_and_normalizes_relative_smooth_paths() {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:redrob="https://redrob.io/ns/canvas/1" width="8px" height="8px" viewBox="0 0 8 8"><g redrob:name="g" opacity="0.5" redrob:blend="screen"><rect redrob:name="r" x="1" y="1" width="3" height="2" fill="#FF000080"/><path redrob:name="p" d="M0 0 c1 0 1 1 2 1 s1 1 2 1 z" fill="none" stroke="#00FF00" stroke-width="1"/></g></svg>"##;
    let imported = import_document(svg, &ImportOptions::default()).unwrap();
    assert_eq!(imported.document().width(), 8);
    assert_eq!(imported.document().nodes().len(), 3);
    assert_eq!(imported.document().nodes()[2].name(), "g");
    let redrob_core::NodeContent::Vector { vector } = imported.document().nodes()[1].content()
    else {
        panic!("expected vector")
    };
    assert!(matches!(
        vector.paths[0].commands[2],
        PathCommand::CubicTo { .. }
    ));
}

#[test]
fn svg_security_guards_report_the_intended_typed_error() {
    let dtd = br##"<?xml version="1.0"?><!DOCTYPE svg [<!ENTITY x "x">]><svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1" fill="#000"/></svg>"##;
    let script = br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1" fill="#000"/><script/></svg>"##;
    for svg in [dtd.as_slice(), script.as_slice()] {
        assert!(matches!(
            import_document(svg, &ImportOptions::default()),
            Err(redrob_core::CoreError::Format(
                FormatError::UnsupportedFeature("forbidden SVG construct")
            ))
        ));
    }

    let transform = br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1" fill="#000"/><path transform="scale(2)" d="M0 0L1 1" stroke="#000" fill="none"/></svg>"##;
    assert!(matches!(
        import_document(transform, &ImportOptions::default()),
        Err(redrob_core::CoreError::Format(
            FormatError::UnsupportedFeature("SVG CSS, transforms, and handlers")
        ))
    ));

    let generic_text = br##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:redrob="https://redrob.io/ns/canvas/1" width="8" height="8"><rect width="1" height="1" fill="#000"/><text font-size="8" fill="#000">A</text></svg>"##;
    assert!(matches!(
        import_document(generic_text, &ImportOptions::default()),
        Err(redrob_core::CoreError::Format(
            FormatError::UnsupportedFeature("generic SVG text")
        ))
    ));

    let undeclared = br##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="1" height="1" fill="#000"/><text redrob:kind="font8x8" font-size="8" fill="#000">A</text></svg>"##;
    assert!(matches!(
        import_document(undeclared, &ImportOptions::default()),
        Err(redrob_core::CoreError::Format(FormatError::Malformed(
            "undeclared Redrob SVG namespace"
        )))
    ));

    let wrong_namespace = br##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:redrob="https://wrong.invalid" width="8" height="8"><rect width="1" height="1" fill="#000"/><text redrob:kind="font8x8" font-size="8" fill="#000">A</text></svg>"##;
    assert!(matches!(
        import_document(wrong_namespace, &ImportOptions::default()),
        Err(redrob_core::CoreError::Format(FormatError::Malformed(
            "wrong Redrob SVG namespace"
        )))
    ));

    let external_image = br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1" fill="#000"/><image href="https://example.invalid/a.png" width="1" height="1"/></svg>"##;
    assert!(matches!(
        import_document(
            external_image,
            &ImportOptions::default().with_loss_policy(LossPolicy::AllowLoss),
        ),
        Err(redrob_core::CoreError::Format(
            FormatError::UnsupportedFeature("external or non-PNG SVG image")
        ))
    ));
}

#[test]
fn svg_xml_byte_limit_is_exact_and_reports_the_typed_error() {
    const MAX_SVG_XML_BYTES: usize = 16 * 1024 * 1024;
    let prefix = br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1" fill="#000"/>"##;
    let suffix = b"</svg>";
    let mut exact = Vec::with_capacity(MAX_SVG_XML_BYTES + 1);
    exact.extend_from_slice(prefix);
    exact.resize(MAX_SVG_XML_BYTES - suffix.len(), b' ');
    exact.extend_from_slice(suffix);
    assert_eq!(exact.len(), MAX_SVG_XML_BYTES);
    assert!(import_document(&exact, &ImportOptions::default()).is_ok());

    exact.push(b' ');
    assert!(matches!(
        import_document(&exact, &ImportOptions::default()),
        Err(redrob_core::CoreError::Format(FormatError::LimitExceeded(
            "SVG XML bytes"
        )))
    ));
}

#[test]
fn svg_vector_roundtrip_and_raster_loss_warnings_are_explicit() {
    let vector = VectorContent {
        paths: vec![VectorPath {
            commands: vec![
                PathCommand::MoveTo { x: 0.0, y: 0.0 },
                PathCommand::LineTo { x: 2.0, y: 0.0 },
                PathCommand::LineTo { x: 2.0, y: 2.0 },
                PathCommand::Close,
            ],
            fill: Some(Pixel::rgba(10, 20, 30, 200)),
            stroke: None,
            fill_rule: redrob_core::FillRule::NonZero,
        }],
    };
    let mut builder = DocumentImportBuilder::new(3, 3).unwrap();
    builder
        .push_node(ImportNode::vector("triangle", vector))
        .unwrap();
    let document = builder.build().unwrap();
    let exported = export_document(&document, FileFormat::Svg, &ExportOptions::default()).unwrap();
    assert!(exported.warnings().is_empty());
    let imported = import_document(exported.bytes(), &ImportOptions::default()).unwrap();
    assert_eq!(
        imported.document().nodes()[0].kind(),
        redrob_core::NodeKind::Vector
    );

    let raster = raster_document(1, 1, vec![1, 2, 3, 4]);
    assert!(export_document(&raster, FileFormat::Svg, &ExportOptions::default()).is_err());
    let exported = export_document(
        &raster,
        FileFormat::Svg,
        &ExportOptions::default().with_loss_policy(LossPolicy::AllowLoss),
    )
    .unwrap();
    assert!(matches!(
        exported.warnings(),
        [FormatWarning::EmbeddedRasterData { .. }]
    ));
    let imported = import_document(
        exported.bytes(),
        &ImportOptions::default().with_loss_policy(LossPolicy::AllowLoss),
    )
    .unwrap();
    assert!(matches!(
        imported.warnings(),
        [FormatWarning::EmbeddedRasterData { .. }]
    ));
    assert_eq!(imported.document().nodes()[0].pixels(), [1, 2, 3, 4]);
}

#[test]
fn option_work_bounds_reject_zero_or_excessive_limits_before_decode() {
    let bytes = png(1, 1, &[0, 0, 0, 0]);
    for limit in [0, redrob_core::MAX_FORMAT_INPUT_BYTES + 1] {
        let error = import_document(
            &bytes,
            &ImportOptions::default().with_max_input_bytes(limit),
        )
        .unwrap_err();
        assert!(matches!(
            error,
            redrob_core::CoreError::Format(FormatError::InvalidOption(_))
        ));
    }
    let error = import_document(
        &bytes,
        &ImportOptions::default().with_max_input_bytes(bytes.len() - 1),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        redrob_core::CoreError::Format(FormatError::InputTooLarge)
    ));
}

#[test]
fn ora_mask_baking_requires_allow_loss_and_warns_once() {
    let mut builder = DocumentImportBuilder::new(1, 1).unwrap();
    builder
        .push_node(
            ImportNode::raster(
                "masked",
                vec![RasterCel::new(FrameId::DEFAULT, vec![20, 30, 40, 255])],
            )
            .with_mask(Some(ImportMask::new(vec![128]))),
        )
        .unwrap();
    let document = builder.build().unwrap();
    assert!(export_document(&document, FileFormat::Ora, &ExportOptions::default()).is_err());
    let exported = export_document(
        &document,
        FileFormat::Ora,
        &ExportOptions::default().with_loss_policy(LossPolicy::AllowLoss),
    )
    .unwrap();
    assert!(matches!(
        exported.warnings(),
        [FormatWarning::BakedRasterMask { .. }]
    ));
    let imported = import_document(exported.bytes(), &ImportOptions::default()).unwrap();
    assert_eq!(imported.document().nodes()[0].pixels(), [20, 30, 40, 128]);
}

#[test]
fn redrob_namespaced_svg_text_roundtrips_escaped_content() {
    let mut builder = DocumentImportBuilder::new(32, 16).unwrap();
    builder
        .push_node(ImportNode::text(
            "label",
            TextContent {
                text: "A<&".into(),
                font_family: "embedded".into(),
                font_size: 8.0,
                color: Pixel::rgba(1, 2, 3, 200),
                origin_x: 2.0,
                origin_y: 3.0,
                font_id: EMBEDDED_FONT_ID.into(),
            },
        ))
        .unwrap();
    let document = builder.build().unwrap();
    let exported = export_document(&document, FileFormat::Svg, &ExportOptions::default()).unwrap();
    let imported = import_document(exported.bytes(), &ImportOptions::default()).unwrap();
    let redrob_core::NodeContent::Text { text } = imported.document().nodes()[0].content() else {
        panic!("expected text")
    };
    assert_eq!(text.text, "A<&");
    assert_eq!(text.font_id, EMBEDDED_FONT_ID);
    assert_eq!(text.origin_x, 2.0);
    assert_eq!(text.origin_y, 3.0);
}

#[test]
fn nonempty_metadata_is_rejected_or_warned_for_non_project_formats() {
    let mut builder = DocumentImportBuilder::new(1, 1).unwrap();
    builder.metadata(DocumentMetadata {
        title: "title".into(),
        author: Some("author".into()),
        properties: [("key".into(), "value".into())].into_iter().collect(),
    });
    builder
        .push_node(ImportNode::raster(
            "pixels",
            vec![RasterCel::new(FrameId::DEFAULT, vec![1, 2, 3, 255])],
        ))
        .unwrap();
    let document = builder.build().unwrap();
    for format in [FileFormat::Png, FileFormat::Ora, FileFormat::Svg] {
        assert!(export_document(&document, format, &ExportOptions::default()).is_err());
        let outcome = export_document(
            &document,
            format,
            &ExportOptions::default().with_loss_policy(LossPolicy::AllowLoss),
        )
        .unwrap();
        assert!(outcome.warnings().contains(&FormatWarning::OmittedMetadata));
    }
}

#[test]
fn ora_root_stack_semantics_are_materialized_instead_of_discarded() {
    let stack = r#"<image version="0.0.1" w="1" h="1"><stack name="semantic root" visibility="hidden" opacity="0.5" composite-op="svg:screen"><layer name="child" src="data/a.png"/></stack></image>"#;
    let bytes = ora_fixture(stack, &[("data/a.png", png(1, 1, &[255, 0, 0, 255]))]);
    let imported = import_document(&bytes, &ImportOptions::default()).unwrap();
    assert_eq!(
        imported
            .document()
            .nodes()
            .iter()
            .map(|node| node.name())
            .collect::<Vec<_>>(),
        ["child", "semantic root"]
    );
    let root = &imported.document().nodes()[1];
    assert!(!root.is_visible());
    assert_eq!(root.opacity(), 0.5);
    assert_eq!(root.blend_mode(), BlendMode::Screen);
    assert_eq!(
        RenderSnapshot::try_render_frame(imported.document(), 0, FrameId::DEFAULT)
            .unwrap()
            .pixels(),
        [0, 0, 0, 0]
    );
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0_u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

fn raw_stored_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut central = Vec::new();
    for (name, data) in entries {
        let offset = bytes.len() as u32;
        let checksum = crc32(data);
        bytes.extend_from_slice(&0x0403_4b50_u32.to_le_bytes());
        bytes.extend_from_slice(&20_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(&checksum.to_le_bytes());
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(name.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&0_u16.to_le_bytes());
        bytes.extend_from_slice(name.as_bytes());
        bytes.extend_from_slice(data);

        central.extend_from_slice(&0x0201_4b50_u32.to_le_bytes());
        central.extend_from_slice(&20_u16.to_le_bytes());
        central.extend_from_slice(&20_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&checksum.to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u16.to_le_bytes());
        central.extend_from_slice(&0_u32.to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let central_offset = bytes.len() as u32;
    bytes.extend_from_slice(&central);
    bytes.extend_from_slice(&0x0605_4b50_u32.to_le_bytes());
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    bytes.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    bytes.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    bytes.extend_from_slice(&(central.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&central_offset.to_le_bytes());
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    bytes
}

#[test]
fn psd_round_trips_a_raster_layer_and_detects() {
    // A 2x2 RGBA raster round-trips through PSD and is detected by its 8BPS signature.
    let pixels = vec![
        250, 10, 20, 255, 10, 250, 20, 255, 10, 20, 250, 128, 100, 100, 100, 255,
    ];
    let document = raster_document(2, 2, pixels.clone());
    let encoded = export_document(&document, FileFormat::Psd, &ExportOptions::default()).unwrap();
    assert_eq!(detect_format(encoded.bytes()).unwrap(), FileFormat::Psd);
    let decoded = import_document(encoded.bytes(), &ImportOptions::default()).unwrap();
    // One raster layer survives with its exact pixels.
    assert_eq!(decoded.document().layers()[0].pixels(), pixels);
}

#[test]
fn kra_round_trips_a_raster_layer_and_detects() {
    // A 2x2 RGBA raster round-trips through KRA and is detected by its mimetype.
    let pixels = vec![
        12, 240, 30, 255, 240, 12, 30, 255, 30, 12, 240, 200, 80, 80, 80, 255,
    ];
    let document = raster_document(2, 2, pixels.clone());
    let encoded = export_document(&document, FileFormat::Kra, &ExportOptions::default()).unwrap();
    assert_eq!(detect_format(encoded.bytes()).unwrap(), FileFormat::Kra);
    let decoded = import_document(encoded.bytes(), &ImportOptions::default()).unwrap();
    assert_eq!(decoded.document().layers()[0].pixels(), pixels);
}

#[test]
fn xcf_detection_and_round_trip() {
    // The 'gimp xcf' magic is detected as XCF.
    let mut header = b"gimp xcf v011\0".to_vec();
    header.extend_from_slice(&[0u8; 12]);
    assert_eq!(detect_format(&header).unwrap(), FileFormat::Xcf);

    // XCF is no longer read-only (H.10): a document round-trips through the writer and the reader.
    let pixels = vec![
        210, 10, 20, 255, 10, 210, 20, 255, 20, 10, 210, 120, 70, 70, 70, 255,
    ];
    let document = raster_document(2, 2, pixels.clone());
    let encoded = export_document(&document, FileFormat::Xcf, &ExportOptions::default()).unwrap();
    assert_eq!(detect_format(encoded.bytes()).unwrap(), FileFormat::Xcf);
    let decoded = import_document(encoded.bytes(), &ImportOptions::default()).unwrap();
    assert_eq!(decoded.document().layers()[0].pixels(), pixels);
}

#[test]
fn xcf_export_keeps_layer_order_names_and_flags() {
    // XCF stores layers TOP-first, the reverse of our sibling order, so a two-layer document is the
    // smallest case where getting that backwards is visible.
    let mut builder = DocumentImportBuilder::new(1, 1).unwrap();
    builder
        .push_node(ImportNode::raster(
            "bottom",
            vec![RasterCel::new(FrameId::DEFAULT, vec![255, 0, 0, 255])],
        ))
        .unwrap();
    builder
        .push_node(
            ImportNode::raster(
                "top",
                vec![RasterCel::new(FrameId::DEFAULT, vec![0, 0, 255, 255])],
            )
            .with_visibility(false)
            .with_opacity(0.5),
        )
        .unwrap();
    let document = builder.build().unwrap();

    let encoded = export_document(&document, FileFormat::Xcf, &ExportOptions::default()).unwrap();
    let decoded = import_document(encoded.bytes(), &ImportOptions::default()).unwrap();
    let layers = decoded.document().layers();
    assert_eq!(layers.len(), 2);
    assert_eq!(layers[0].name(), "bottom");
    assert_eq!(layers[1].name(), "top");
    assert!(!layers[1].is_visible());
    assert!((layers[1].opacity() - 0.5).abs() < 0.01);
    assert_eq!(layers[1].pixels(), vec![0, 0, 255, 255]);
}

#[test]
fn xcf_export_tiles_a_canvas_wider_than_one_tile() {
    // Edge tiles carry only their OWN rectangle in XCF, unlike Krita's always-64 tiles. A canvas that is
    // not a multiple of 64 is the case that catches a writer padding them: every row of the edge tiles
    // would shift.
    let wide = 70u32;
    let tall = 66u32;
    let mut pixels = Vec::with_capacity((wide * tall) as usize * 4);
    for i in 0..(wide * tall) {
        let value = (i % 251) as u8;
        pixels.extend_from_slice(&[value, 255 - value, 128, 255]);
    }
    let document = raster_document(wide, tall, pixels.clone());
    let encoded = export_document(&document, FileFormat::Xcf, &ExportOptions::default()).unwrap();
    let decoded = import_document(encoded.bytes(), &ImportOptions::default()).unwrap();
    assert_eq!(decoded.document().layers()[0].pixels(), pixels);
}

#[test]
fn tiff_round_trips_and_exr_encodes() {
    let pixels = vec![200, 10, 30, 255, 10, 200, 30, 255, 30, 10, 200, 255, 90, 90, 90, 255];
    let document = raster_document(2, 2, pixels.clone());
    // TIFF round-trips RGBA exactly (lossless).
    let tiff = export_document(&document, FileFormat::Tiff, &ExportOptions::default()).unwrap();
    assert_eq!(detect_format(tiff.bytes()).unwrap(), FileFormat::Tiff);
    let decoded = import_document(tiff.bytes(), &ImportOptions::default()).unwrap();
    assert_eq!(decoded.document().layers()[0].pixels(), pixels);
    // EXR encodes and is detected (float round-trip is not bit-exact, so only check it decodes).
    let exr = export_document(&document, FileFormat::Exr, &ExportOptions::default()).unwrap();
    assert_eq!(detect_format(exr.bytes()).unwrap(), FileFormat::Exr);
    assert!(import_document(exr.bytes(), &ImportOptions::default()).is_ok());
}

#[test]
fn heif_and_jxl_detect_but_are_unsupported() {
    // JPEG-XL codestream magic.
    let jxl = [0xff, 0x0a, 0, 0, 0, 0, 0, 0];
    assert_eq!(detect_format(&jxl).unwrap(), FileFormat::JpegXl);
    assert!(import_document(&jxl, &ImportOptions::default()).is_err());
    // HEIF ftyp box.
    let mut heif = vec![0, 0, 0, 0x18];
    heif.extend_from_slice(b"ftypheic");
    heif.extend_from_slice(&[0u8; 8]);
    assert_eq!(detect_format(&heif).unwrap(), FileFormat::Heif);
}

#[test]
fn pdf_and_camera_raw_detect_but_are_unsupported() {
    // PDF magic.
    let pdf = b"%PDF-1.7\n...".to_vec();
    assert_eq!(detect_format(&pdf).unwrap(), FileFormat::Pdf);
    assert!(import_document(&pdf, &ImportOptions::default()).is_err());

    // Canon CR2: a TIFF with "CR" at offset 8 — detected as Raw, not TIFF.
    let mut cr2 = b"II*\x00".to_vec();
    cr2.extend_from_slice(&[0, 0, 0, 0]); // ifd offset
    cr2.extend_from_slice(b"CR"); // CR2 marker at offset 8
    cr2.extend_from_slice(&[0u8; 16]);
    assert_eq!(detect_format(&cr2).unwrap(), FileFormat::Raw);
    assert!(import_document(&cr2, &ImportOptions::default()).is_err());

    // Fujifilm RAF.
    let mut raf = b"FUJIFILMCCD-RAW".to_vec();
    raf.extend_from_slice(&[0u8; 8]);
    assert_eq!(detect_format(&raf).unwrap(), FileFormat::Raw);
}

#[test]
fn animated_gif_and_apng_export() {
    let document = raster_document(2, 2, vec![200, 10, 30, 255, 10, 200, 30, 255, 30, 10, 200, 255, 90, 90, 90, 255]);
    // Animated GIF export (single frame here) is a valid GIF detected by its header.
    let gif = export_document(&document, FileFormat::Gif, &ExportOptions::default()).unwrap();
    assert_eq!(detect_format(gif.bytes()).unwrap(), FileFormat::Gif);
    // APNG export starts with the PNG signature and carries an acTL chunk.
    let apng = export_document(&document, FileFormat::Apng, &ExportOptions::default()).unwrap();
    assert!(apng.bytes().starts_with(&[0x89, b'P', b'N', b'G']), "APNG has the PNG signature");
    assert!(
        apng.bytes().windows(4).any(|w| w == b"acTL"),
        "APNG carries an animation control chunk"
    );
    // Animated WebP export is a real RIFF container now (H.12).
    let webp = export_document(&document, FileFormat::WebpAnim, &ExportOptions::default()).unwrap();
    let bytes = webp.bytes();
    assert!(bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP");
    // The RIFF size counts "WEBP" plus the chunks, and not its own eight-byte header.
    assert_eq!(
        u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize,
        bytes.len() - 8
    );
    // Without the extended header and its animation flag, a reader treats the file as one still image
    // and every frame after the first vanishes.
    assert_eq!(&bytes[12..16], b"VP8X");
    assert_eq!(bytes[20] & 0x02, 0x02, "animation flag must be set");
    // VP8X stores the canvas size MINUS ONE: a 2x2 canvas is written as 1, 1.
    assert_eq!(&bytes[24..27], &[1, 0, 0]);
    assert_eq!(&bytes[27..30], &[1, 0, 0]);
    assert!(bytes.windows(4).any(|w| w == b"ANIM"), "animation parameters");
    assert!(bytes.windows(4).any(|w| w == b"ANMF"), "at least one frame");
}

#[test]
fn animated_webp_writes_one_frame_chunk_per_timeline_frame() {
    // One ANMF per frame is the whole point of the container: the still encoder can only ever produce
    // the first one.
    let mut editor = Editor::new(raster_document(2, 2, vec![0; 16])).unwrap();
    editor
        .execute(redrob_core::Command::Fill {
            color: Pixel::rgba(10, 20, 30, 255),
        })
        .unwrap();
    editor
        .execute(redrob_core::Command::AddFrame {
            id: FrameId::new(2),
            index: 1,
        })
        .unwrap();

    let webp =
        export_document(editor.document(), FileFormat::WebpAnim, &ExportOptions::default()).unwrap();
    let frames = webp
        .bytes()
        .windows(4)
        .filter(|window| *window == b"ANMF")
        .count();
    assert_eq!(frames, 2, "one ANMF chunk per timeline frame");
}

#[test]
fn svg_imports_circle_and_ellipse_as_cubic_paths() {
    // General SVG <circle>/<ellipse> become vector nodes whose outline is four cubic Béziers.
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="20px" height="20px" viewBox="0 0 20 20"><circle cx="10" cy="10" r="6" fill="#FF0000"/><ellipse cx="10" cy="10" rx="8" ry="4" fill="none" stroke="#0000FF" stroke-width="1"/></svg>"##;
    let imported = import_document(svg, &ImportOptions::default()).unwrap();
    let nodes = imported.document().nodes();
    assert_eq!(nodes.len(), 2, "circle and ellipse become two vector nodes");
    for node in nodes {
        let redrob_core::NodeContent::Vector { vector } = node.content() else {
            panic!("expected vector");
        };
        // MoveTo + four CubicTo + Close.
        assert_eq!(vector.paths[0].commands.len(), 6);
        assert!(matches!(vector.paths[0].commands[1], PathCommand::CubicTo { .. }));
        assert!(matches!(
            vector.paths[0].commands[5],
            PathCommand::Close
        ));
    }
}

/// Builds a layerless PSD whose merged image carries `channels` planes at `depth` bits with the given
/// compression tag. Hand-built rather than fixtured: the point is to pin what a DEEP file's bytes mean,
/// and a fixture produced by our own writer could only ever prove the writer agrees with the reader.
fn deep_psd(width: u32, height: u32, depth: u16, compression: u16, planes: &[u8]) -> Vec<u8> {
    let mut bytes = b"8BPS".to_vec();
    bytes.extend_from_slice(&1_u16.to_be_bytes()); // version
    bytes.extend_from_slice(&[0u8; 6]); // reserved
    bytes.extend_from_slice(&3_u16.to_be_bytes()); // channels: R, G, B
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&depth.to_be_bytes());
    bytes.extend_from_slice(&3_u16.to_be_bytes()); // colour mode: RGB
    bytes.extend_from_slice(&0_u32.to_be_bytes()); // colour mode data length
    bytes.extend_from_slice(&0_u32.to_be_bytes()); // image resources length
    bytes.extend_from_slice(&0_u32.to_be_bytes()); // layer & mask length: no layer section
    bytes.extend_from_slice(&compression.to_be_bytes());
    bytes.extend_from_slice(planes);
    bytes
}

#[test]
fn psd_reads_sixteen_bit_raw_channels() {
    // 16-bit Photoshop samples run 0..=32768 for 0..=1, NOT the full u16 range, so 32768 is white and
    // 16384 is mid -- the thing a reader scaling against 65535 gets subtly wrong on every pixel.
    let mut planes = Vec::new();
    for value in [32768u16, 0] {
        planes.extend_from_slice(&value.to_be_bytes()); // red row
    }
    for value in [0u16, 32768] {
        planes.extend_from_slice(&value.to_be_bytes()); // green row
    }
    for value in [16384u16, 16384] {
        planes.extend_from_slice(&value.to_be_bytes()); // blue row
    }
    let bytes = deep_psd(2, 1, 16, 0, &planes);

    assert_eq!(detect_format(&bytes).unwrap(), FileFormat::Psd);
    let decoded = import_document(&bytes, &ImportOptions::default()).unwrap();
    assert_eq!(
        decoded.document().layers()[0].pixels(),
        vec![255, 0, 128, 255, 0, 255, 128, 255]
    );
    // The narrowing is reported, not silent.
    assert!(
        decoded
            .warnings()
            .contains(&FormatWarning::NarrowedDepth { source_bits: 16 })
    );
}

#[test]
fn psd_reads_sixteen_bit_zip_predicted_channels() {
    // Photoshop writes ZIP-with-prediction for deep documents, so a reader that knows only raw and RLE
    // opens almost no real 16-bit file. Prediction is a per-ROW delta on 16-bit words: the same pixels
    // as the raw case above, encoded as differences.
    let mut encoded_planes = Vec::new();
    for row in [[32768u16, 32768], [0, 32768], [16384, 0]] {
        for delta in row {
            encoded_planes.extend_from_slice(&delta.to_be_bytes());
        }
    }
    let mut zlib = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    zlib.write_all(&encoded_planes).unwrap();
    let stream = zlib.finish().unwrap();
    let bytes = deep_psd(2, 1, 16, 3, &stream);

    let decoded = import_document(&bytes, &ImportOptions::default()).unwrap();
    assert_eq!(
        decoded.document().layers()[0].pixels(),
        vec![255, 0, 128, 255, 0, 255, 128, 255]
    );
}

#[test]
fn psd_reads_thirty_two_bit_float_channels_through_the_srgb_transfer() {
    // A 32-bit document stores LINEAR floats. Encoding them with the sRGB transfer function is what
    // keeps it from opening darker than the same picture at 8-bit: linear 0.5 is ~188, not 128.
    let mut planes = Vec::new();
    for value in [1.0f32, 0.5] {
        planes.extend_from_slice(&value.to_be_bytes()); // red
    }
    for value in [0.0f32, 0.0] {
        planes.extend_from_slice(&value.to_be_bytes()); // green
    }
    for value in [0.0f32, 0.0] {
        planes.extend_from_slice(&value.to_be_bytes()); // blue
    }
    let bytes = deep_psd(2, 1, 32, 0, &planes);

    let decoded = import_document(&bytes, &ImportOptions::default()).unwrap();
    let pixels = decoded.document().layers()[0].pixels().to_vec();
    assert_eq!(pixels[0], 255);
    assert!(
        (186..=190).contains(&pixels[4]),
        "linear 0.5 should encode near 188, got {}",
        pixels[4]
    );
    assert!(
        decoded
            .warnings()
            .contains(&FormatWarning::NarrowedDepth { source_bits: 32 })
    );
}

#[test]
fn psd_rejects_an_unknown_bit_depth() {
    // 8, 16 and 32 are read; anything else is refused by name rather than decoded as bytes.
    let bytes = deep_psd(1, 1, 64, 0, &[0u8; 24]);
    let error = import_document(&bytes, &ImportOptions::default()).unwrap_err();
    assert!(matches!(
        error,
        redrob_core::CoreError::Format(FormatError::UnsupportedFeature(_))
    ));
}

/// Builds a layerless PSD in an arbitrary colour mode, with an optional colour-mode data block (the
/// palette, for indexed mode).
fn mode_psd(
    width: u32,
    height: u32,
    depth: u16,
    channels: u16,
    mode: u16,
    color_mode_data: &[u8],
    planes: &[u8],
) -> Vec<u8> {
    let mut bytes = b"8BPS".to_vec();
    bytes.extend_from_slice(&1_u16.to_be_bytes());
    bytes.extend_from_slice(&[0u8; 6]);
    bytes.extend_from_slice(&channels.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&depth.to_be_bytes());
    bytes.extend_from_slice(&mode.to_be_bytes());
    bytes.extend_from_slice(&(color_mode_data.len() as u32).to_be_bytes());
    bytes.extend_from_slice(color_mode_data);
    bytes.extend_from_slice(&0_u32.to_be_bytes()); // image resources
    bytes.extend_from_slice(&0_u32.to_be_bytes()); // layer & mask
    bytes.extend_from_slice(&0_u16.to_be_bytes()); // compression: raw
    bytes.extend_from_slice(planes);
    bytes
}

#[test]
fn psd_reads_greyscale_mode_as_grey_not_red() {
    // Channel ids are positional per colour mode: greyscale's plane 0 is GREY, not red. Reading it as
    // red is what made a greyscale file open as a red-ramp before this.
    let bytes = mode_psd(3, 1, 8, 1, 1, &[], &[0, 128, 255]);
    let decoded = import_document(&bytes, &ImportOptions::default()).unwrap();
    assert_eq!(
        decoded.document().layers()[0].pixels(),
        vec![0, 0, 0, 255, 128, 128, 128, 255, 255, 255, 255, 255]
    );
    assert!(
        decoded
            .warnings()
            .contains(&FormatWarning::ConvertedColorMode { source: "grayscale" })
    );
}

#[test]
fn psd_reads_cmyk_mode_with_inverted_ink_and_no_alpha_confusion() {
    // Two things this pins. PSD stores CMYK INVERTED (255 = no ink), so full cyan is a stored 0. And
    // the fourth plane is BLACK INK, not alpha -- treating it as alpha opened print documents as
    // nearly invisible.
    let planes = [
        0u8, 255, // cyan plane: full ink, then none
        255, 255, // magenta
        255, 255, // yellow
        255, 255, // black: no ink in either pixel
    ];
    let bytes = mode_psd(2, 1, 8, 4, 4, &[], &planes);
    let decoded = import_document(&bytes, &ImportOptions::default()).unwrap();
    let pixels = decoded.document().layers()[0].pixels().to_vec();
    // Full cyan ink with no other ink is cyan, fully opaque.
    assert_eq!(&pixels[0..4], &[0, 255, 255, 255]);
    // No ink at all is white, fully opaque -- not transparent.
    assert_eq!(&pixels[4..8], &[255, 255, 255, 255]);
}

#[test]
fn psd_reads_lab_mode_through_the_colour_module() {
    // Lab stores L as 0..=255 for 0..=100 and offsets a/b by 128, so a neutral white is (255, 128, 128).
    let bytes = mode_psd(1, 1, 8, 3, 9, &[], &[255, 128, 128]);
    let decoded = import_document(&bytes, &ImportOptions::default()).unwrap();
    let pixels = decoded.document().layers()[0].pixels().to_vec();
    assert!(pixels[0] >= 250 && pixels[1] >= 250 && pixels[2] >= 250, "{pixels:?}");
}

#[test]
fn psd_reads_indexed_mode_through_its_planar_palette() {
    // The palette in the colour-mode data block is PLANAR: 256 reds, then greens, then blues. Reading
    // it as interleaved triples gives every index the wrong colour.
    let mut palette = vec![0u8; 768];
    palette[1] = 200; // red of index 1
    palette[256 + 1] = 100; // green of index 1
    palette[512 + 1] = 50; // blue of index 1
    let bytes = mode_psd(1, 1, 8, 1, 2, &palette, &[1]);
    let decoded = import_document(&bytes, &ImportOptions::default()).unwrap();
    assert_eq!(
        decoded.document().layers()[0].pixels(),
        vec![200, 100, 50, 255]
    );
}

#[test]
fn psd_reads_one_bit_bitmap_mode_inverted() {
    // Bitmap mode is 1 bit per pixel AND inverted against every other mode: a SET bit is black. Rows
    // are padded to a whole byte, so a 2-pixel row occupies one byte.
    let bytes = mode_psd(2, 1, 1, 1, 0, &[], &[0b1000_0000]);
    let decoded = import_document(&bytes, &ImportOptions::default()).unwrap();
    assert_eq!(
        decoded.document().layers()[0].pixels(),
        vec![0, 0, 0, 255, 255, 255, 255, 255]
    );
}

#[test]
fn psd_rejects_one_bit_outside_bitmap_mode() {
    // A 1-bit RGB document does not exist; refusing the PAIR catches a malformed header instead of
    // shearing the image.
    let bytes = mode_psd(2, 1, 1, 3, 3, &[], &[0u8; 3]);
    let error = import_document(&bytes, &ImportOptions::default()).unwrap_err();
    assert!(matches!(
        error,
        redrob_core::CoreError::Format(FormatError::UnsupportedFeature(_))
    ));
}

/// Builds a 2x1 PSD with ONE layer that carries a user mask (its own 1x1 rectangle, default 255) and
/// an adjustment key in its additional information. Hand-built because the point is the byte layout of
/// the mask block, which our own writer does not produce.
fn psd_with_mask_and_adjustment() -> Vec<u8> {
    // --- one layer record ---
    let mut record = Vec::new();
    for value in [0i32, 0, 1, 2] {
        record.extend_from_slice(&value.to_be_bytes()); // top, left, bottom, right
    }
    record.extend_from_slice(&4_u16.to_be_bytes()); // channels: R, G, B, mask
    for (id, len) in [(0i16, 4u32), (1, 4), (2, 4), (-2, 3)] {
        record.extend_from_slice(&id.to_be_bytes());
        record.extend_from_slice(&len.to_be_bytes());
    }
    record.extend_from_slice(b"8BIM");
    record.extend_from_slice(b"norm");
    record.push(255); // opacity
    record.push(0); // clipping
    record.push(0); // flags: visible
    record.push(0); // filler

    let mut extra = Vec::new();
    // Layer mask data: 18 bytes. The mask's rectangle is NOT the layer's, and its default is 255.
    extra.extend_from_slice(&18_u32.to_be_bytes());
    for value in [0i32, 0, 1, 1] {
        extra.extend_from_slice(&value.to_be_bytes()); // mask top, left, bottom, right
    }
    extra.push(255); // default colour outside the mask rect
    extra.push(0); // flags: mask enabled
    extra.extend_from_slice(&0_u32.to_be_bytes()); // blending ranges: none
    extra.push(0); // Pascal name length 0
    extra.extend_from_slice(&[0u8; 3]); // padded to a multiple of 4
    // Additional layer information: an adjustment key with an empty payload.
    extra.extend_from_slice(b"8BIM");
    extra.extend_from_slice(b"levl");
    extra.extend_from_slice(&0_u32.to_be_bytes());

    record.extend_from_slice(&(extra.len() as u32).to_be_bytes());
    record.extend_from_slice(&extra);

    // --- channel image data, in record order ---
    let mut channel_data = Vec::new();
    for plane in [[200u8, 100], [50, 25], [10, 5]] {
        channel_data.extend_from_slice(&0_u16.to_be_bytes()); // raw
        channel_data.extend_from_slice(&plane);
    }
    channel_data.extend_from_slice(&0_u16.to_be_bytes()); // mask channel, raw
    channel_data.push(128); // the mask's single pixel

    let mut layer_info = Vec::new();
    layer_info.extend_from_slice(&1_i16.to_be_bytes()); // layer count
    layer_info.extend_from_slice(&record);
    layer_info.extend_from_slice(&channel_data);

    let mut layer_and_mask = Vec::new();
    layer_and_mask.extend_from_slice(&(layer_info.len() as u32).to_be_bytes());
    layer_and_mask.extend_from_slice(&layer_info);
    layer_and_mask.extend_from_slice(&0_u32.to_be_bytes()); // global layer mask info: none

    let mut bytes = b"8BPS".to_vec();
    bytes.extend_from_slice(&1_u16.to_be_bytes());
    bytes.extend_from_slice(&[0u8; 6]);
    bytes.extend_from_slice(&3_u16.to_be_bytes()); // channels
    bytes.extend_from_slice(&1_u32.to_be_bytes()); // height
    bytes.extend_from_slice(&2_u32.to_be_bytes()); // width
    bytes.extend_from_slice(&8_u16.to_be_bytes()); // depth
    bytes.extend_from_slice(&3_u16.to_be_bytes()); // RGB
    bytes.extend_from_slice(&0_u32.to_be_bytes()); // colour mode data
    bytes.extend_from_slice(&0_u32.to_be_bytes()); // image resources
    bytes.extend_from_slice(&(layer_and_mask.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&layer_and_mask);
    bytes
}

#[test]
fn psd_keeps_a_layer_mask_with_its_own_rect_and_default() {
    let decoded = import_document(&psd_with_mask_and_adjustment(), &ImportOptions::default()).unwrap();
    let mask = decoded.document().layers()[0]
        .mask()
        .expect("the layer's user mask should survive import");
    assert!(mask.is_enabled());
    // The mask's rect is 1x1 at the left; the rest of the canvas takes the mask's OWN default (255),
    // not zero -- zeroing it would reveal what the author masked out.
    assert_eq!(mask.pixels(), vec![128, 255]);
}

#[test]
fn psd_reports_an_adjustment_layer_it_cannot_apply() {
    let decoded = import_document(&psd_with_mask_and_adjustment(), &ImportOptions::default()).unwrap();
    // The layer is kept, and the unapplied adjustment is named rather than looking like a rendering bug.
    assert_eq!(decoded.document().layers().len(), 1);
    assert!(decoded.warnings().iter().any(|warning| matches!(
        warning,
        FormatWarning::UnappliedAdjustment { kind, .. } if kind == "levl"
    )));
}

/// Builds a KRA shaped the way KRITA writes one: the layer's pixels are the native tiled paint device
/// under a directory named after the IMAGE (not our own writer's name), with a `.defaultpixel` sidecar.
/// The single tile is stored uncompressed, which Krita also does whenever compression would not pay.
fn krita_tiled_kra() -> Vec<u8> {
    // One 64x64 tile at the origin, 4 bytes per pixel, BGRA.
    let tile_pixels = 64 * 64;
    let mut tile = vec![0u8; tile_pixels * 4];
    // Top-left pixel: opaque red. Written BGRA, which is the device's own channel order.
    tile[0] = 20; // blue
    tile[1] = 60; // green
    tile[2] = 200; // red
    tile[3] = 255; // alpha

    let mut device = Vec::new();
    device.extend_from_slice(b"VERSION 2\n");
    device.extend_from_slice(b"TILEWIDTH 64\n");
    device.extend_from_slice(b"TILEHEIGHT 64\n");
    device.extend_from_slice(b"PIXELSIZE 4\n");
    device.extend_from_slice(b"DATA 1\n");
    // Record header: pixel offsets, compression name, byte count (flag byte included).
    device.extend_from_slice(format!("0,0,LZF,{}\n", tile.len() + 1).as_bytes());
    device.push(0); // flag: raw, not compressed
    device.extend_from_slice(&tile);

    let maindoc = r#"<?xml version="1.0" encoding="UTF-8"?>
<DOC syntaxVersion="2">
 <IMAGE name="painting" width="2" height="2" colorspacename="RGBA">
  <layers>
   <layer name="Paint" filename="layer2" nodetype="paintlayer" opacity="255" visible="1"/>
  </layers>
 </IMAGE>
</DOC>"#;

    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file(
            "mimetype",
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
        )
        .unwrap();
    writer.write_all(b"application/x-krita").unwrap();
    writer
        .start_file("maindoc.xml", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(maindoc.as_bytes()).unwrap();
    // Named after the image, NOT after our own writer's document name.
    writer
        .start_file("painting/layers/layer2", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(&device).unwrap();
    writer
        .start_file(
            "painting/layers/layer2.defaultpixel",
            SimpleFileOptions::default(),
        )
        .unwrap();
    // Default pixel: opaque white, stored BGRA. Not transparent -- that is the point of reading it.
    writer.write_all(&[255u8, 255, 255, 255]).unwrap();
    writer.finish().unwrap().into_inner()
}

#[test]
fn kra_reads_kritas_native_tiled_layer() {
    let bytes = krita_tiled_kra();
    assert_eq!(detect_format(&bytes).unwrap(), FileFormat::Kra);
    let decoded = import_document(&bytes, &ImportOptions::default()).unwrap();
    // One real layer, not the flattened preview fallback.
    assert_eq!(decoded.document().layers().len(), 1);
    let pixels = decoded.document().layers()[0].pixels().to_vec();
    // BGRA in the file becomes RGBA here: reading it straight through would give (20, 60, 200).
    assert_eq!(&pixels[0..4], &[200, 60, 20, 255]);
    // Everything the tile covers but did not paint is transparent, because the tile's own bytes win
    // over the default pixel.
    assert_eq!(&pixels[4..8], &[0, 0, 0, 0]);
    // The merged-image fallback would have warned about flattening; reading the real layer does not.
    assert!(!decoded.warnings().contains(&FormatWarning::FlattenedHierarchy));
}

#[test]
fn kra_tiled_layer_uses_the_default_pixel_outside_every_tile() {
    // A 2x2 canvas whose single tile sits far to the right: nothing the tile covers is on canvas, so
    // every pixel takes the layer's default. A reader that ignores `.defaultpixel` opens this empty.
    let mut device = Vec::new();
    device.extend_from_slice(b"VERSION 2\nTILEWIDTH 64\nTILEHEIGHT 64\nPIXELSIZE 4\nDATA 1\n");
    let tile = vec![0u8; 64 * 64 * 4];
    device.extend_from_slice(format!("640,640,LZF,{}\n", tile.len() + 1).as_bytes());
    device.push(0);
    device.extend_from_slice(&tile);

    let maindoc = r#"<DOC><IMAGE name="painting" width="1" height="1"><layers>
   <layer name="Fill" filename="layer1" nodetype="paintlayer" opacity="255" visible="1"/>
  </layers></IMAGE></DOC>"#;
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file(
            "mimetype",
            SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
        )
        .unwrap();
    writer.write_all(b"application/x-krita").unwrap();
    writer
        .start_file("maindoc.xml", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(maindoc.as_bytes()).unwrap();
    writer
        .start_file("painting/layers/layer1", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(&device).unwrap();
    writer
        .start_file(
            "painting/layers/layer1.defaultpixel",
            SimpleFileOptions::default(),
        )
        .unwrap();
    writer.write_all(&[30u8, 90, 180, 255]).unwrap(); // BGRA
    let bytes = writer.finish().unwrap().into_inner();

    let decoded = import_document(&bytes, &ImportOptions::default()).unwrap();
    assert_eq!(decoded.document().layers()[0].pixels(), vec![180, 90, 30, 255]);
}

#[test]
fn kra_export_writes_the_native_tiled_device_not_a_png() {
    // What makes this item worth doing: a file only this product can open is not a KRA. The layer entry
    // must be the tiled paint device Krita reads, with its default-pixel sidecar beside it.
    let document = raster_document(2, 2, vec![10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255, 1, 2, 3, 4]);
    let encoded = export_document(&document, FileFormat::Kra, &ExportOptions::default()).unwrap();
    let mut archive = ZipArchive::new(Cursor::new(encoded.bytes())).unwrap();
    let names: Vec<String> = (0..archive.len())
        .map(|i| archive.by_index(i).unwrap().name().to_owned())
        .collect();
    assert!(
        names.iter().any(|n| n.ends_with("/layers/layer0")),
        "expected a native tiled device, got {names:?}"
    );
    assert!(names.iter().any(|n| n.ends_with("/layers/layer0.defaultpixel")));
    assert!(
        !names.iter().any(|n| n.ends_with("/layers/layer0.png")),
        "the PNG convention should be gone: {names:?}"
    );
    // The device's own header, so the entry is not merely named like one.
    let mut entry = archive.by_name(
        names
            .iter()
            .find(|n| n.ends_with("/layers/layer0"))
            .unwrap()
            .as_str(),
    )
    .unwrap();
    let mut device = Vec::new();
    entry.read_to_end(&mut device).unwrap();
    assert!(device.starts_with(b"VERSION 2\n"), "{:?}", &device[..16.min(device.len())]);
}

#[test]
fn kra_round_trips_through_the_tiled_device_exactly() {
    // The round-trip now goes through the tile writer AND the tile reader, so an error in either shows
    // up here rather than hiding behind a PNG that both sides agreed on.
    let pixels = vec![
        12, 240, 30, 255, 240, 12, 30, 255, 30, 12, 240, 200, 80, 80, 80, 255,
    ];
    let document = raster_document(2, 2, pixels.clone());
    let encoded = export_document(&document, FileFormat::Kra, &ExportOptions::default()).unwrap();
    let decoded = import_document(encoded.bytes(), &ImportOptions::default()).unwrap();
    assert_eq!(decoded.document().layers()[0].pixels(), pixels);
}

#[test]
fn kra_tiled_device_survives_the_compressed_branch() {
    // A flat canvas compresses, so this exercises LZF compression AND the byte-planarisation around it;
    // the earlier round-trip test's noisy 2x2 tile is small enough to stay raw. A compressed tile that
    // is not un-planarised on the way back comes out as one channel smeared across the image, which this
    // would catch as a colour mismatch rather than a crash.
    let wide = 200u32;
    let tall = 120u32;
    let mut pixels = Vec::with_capacity((wide * tall) as usize * 4);
    for _ in 0..(wide * tall) {
        pixels.extend_from_slice(&[18, 52, 86, 255]);
    }
    let document = raster_document(wide, tall, pixels.clone());
    let encoded = export_document(&document, FileFormat::Kra, &ExportOptions::default()).unwrap();
    // A flat 200x120 canvas must be far smaller than its raw tiles (6 tiles x 16 KiB).
    assert!(encoded.bytes().len() < 6 * 64 * 64 * 4, "{}", encoded.bytes().len());
    let decoded = import_document(encoded.bytes(), &ImportOptions::default()).unwrap();
    assert_eq!(decoded.document().layers()[0].pixels(), pixels);
}

/// Builds a minimal XCF v11: 8-byte file offsets and one zlib-compressed tile. Hand-built because the
/// two things under test are exactly the byte-level decisions — offset width and tile layout — and this
/// product has no XCF writer to produce a fixture from.
fn xcf_v11_zlib(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    fn be32(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_be_bytes());
    }
    fn be64(out: &mut Vec<u8>, value: u64) {
        out.extend_from_slice(&value.to_be_bytes());
    }

    // The tile: interleaved RGBA, zlib-compressed. Interleaved is the point — RLE would be planar.
    let mut zlib = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    zlib.write_all(rgba).unwrap();
    let tile = zlib.finish().unwrap();

    let mut out = Vec::new();
    out.extend_from_slice(b"gimp xcf "); // the magic carries its trailing space
    out.extend_from_slice(b"v011\0"); // version 11: offsets are 8 bytes
    be32(&mut out, width);
    be32(&mut out, height);
    be32(&mut out, 0); // base type: RGB
    be32(&mut out, 100); // precision: 8-bit
    // Image properties: COMPRESSION = 2 (zlib), then END.
    be32(&mut out, 17);
    be32(&mut out, 1);
    out.push(2);
    be32(&mut out, 0); // PROP_END
    be32(&mut out, 0); // its length

    // Layer pointer list: one layer, then the terminating zero. Both 8 bytes wide.
    let layer_pointer_at = out.len();
    be64(&mut out, 0); // placeholder, patched below
    be64(&mut out, 0); // terminator
    // The image's CHANNEL pointer list follows the layer list; this file has none.
    be64(&mut out, 0);

    let layer_offset = out.len();
    be32(&mut out, width);
    be32(&mut out, height);
    be32(&mut out, 1); // layer type: RGBA
    // Layer name as a length-prefixed string including its NUL.
    let name = b"Paint\0";
    be32(&mut out, name.len() as u32);
    out.extend_from_slice(name);
    be32(&mut out, 0); // PROP_END
    be32(&mut out, 0);
    let hierarchy_pointer_at = out.len();
    be64(&mut out, 0); // placeholder
    be64(&mut out, 0); // layer mask pointer: none

    let hierarchy_offset = out.len();
    be32(&mut out, width);
    be32(&mut out, height);
    be32(&mut out, 4); // bytes per pixel
    let level_pointer_at = out.len();
    be64(&mut out, 0); // placeholder
    be64(&mut out, 0); // no further levels

    let level_offset = out.len();
    be32(&mut out, width);
    be32(&mut out, height);
    let tile_pointer_at = out.len();
    be64(&mut out, 0); // placeholder for the single tile
    let terminator_at = out.len();
    be64(&mut out, 0); // terminator, patched so it bounds the tile's bytes

    let tile_offset = out.len();
    out.extend_from_slice(&tile);
    let after_tile = out.len();

    for (at, value) in [
        (layer_pointer_at, layer_offset),
        (hierarchy_pointer_at, hierarchy_offset),
        (level_pointer_at, level_offset),
        (tile_pointer_at, tile_offset),
        (terminator_at, after_tile),
    ] {
        out[at..at + 8].copy_from_slice(&(value as u64).to_be_bytes());
    }
    out
}

#[test]
fn xcf_reads_version_eleven_with_zlib_tiles() {
    // v11's real change is the OFFSET WIDTH (4 bytes to 8). Read with 4-byte offsets the file does not
    // fail, it lands in the middle of the data — so this test is about both the width and the zlib tile.
    let pixels = vec![
        200, 10, 20, 255, 10, 200, 20, 255, 20, 10, 200, 128, 90, 90, 90, 255,
    ];
    let bytes = xcf_v11_zlib(2, 2, &pixels);
    assert_eq!(detect_format(&bytes).unwrap(), FileFormat::Xcf);
    let decoded = import_document(&bytes, &ImportOptions::default()).unwrap();
    assert_eq!(decoded.document().layers().len(), 1);
    // Interleaved in, interleaved out: a reader that treated the zlib tile as planar would return the
    // first quarter of these bytes as the red channel.
    assert_eq!(decoded.document().layers()[0].pixels(), pixels);
}

#[test]
fn xcf_rejects_the_compression_gimp_never_implemented() {
    // Fractal compression (3) is declared by the format and was never implemented. Refused by name,
    // rather than decoded as one of the forms it is not.
    let mut bytes = xcf_v11_zlib(1, 1, &[1, 2, 3, 4]);
    // The COMPRESSION property's payload byte sits after the 9-byte magic, the 5-byte version tag, the
    // four u32 header fields, and the property's own id and length.
    let payload = 9 + 5 + 4 * 4 + 4 + 4;
    bytes[payload] = 3;
    let error = import_document(&bytes, &ImportOptions::default()).unwrap_err();
    assert!(matches!(
        error,
        redrob_core::CoreError::Format(FormatError::UnsupportedFeature(_))
    ));
}

/// Builds an INDEXED XCF v11 whose single layer also carries a layer mask. Uncompressed tiles, so the
/// bytes under test are the palette lookup and the mask's own channel structure.
fn xcf_indexed_with_mask() -> Vec<u8> {
    fn be32(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_be_bytes());
    }
    fn be64(out: &mut Vec<u8>, value: u64) {
        out.extend_from_slice(&value.to_be_bytes());
    }

    let mut out = Vec::new();
    out.extend_from_slice(b"gimp xcf ");
    out.extend_from_slice(b"v011\0");
    be32(&mut out, 2); // width
    be32(&mut out, 1); // height
    be32(&mut out, 2); // base type: INDEXED
    be32(&mut out, 100); // precision: 8-bit
    // COLORMAP: two entries, as interleaved RGB triples.
    be32(&mut out, 1);
    be32(&mut out, 4 + 6);
    be32(&mut out, 2);
    out.extend_from_slice(&[10, 20, 30, 200, 150, 100]);
    // COMPRESSION: none.
    be32(&mut out, 17);
    be32(&mut out, 1);
    out.push(0);
    be32(&mut out, 0); // PROP_END
    be32(&mut out, 0);

    let layer_pointer_at = out.len();
    be64(&mut out, 0);
    be64(&mut out, 0); // layer list terminator
    be64(&mut out, 0); // channel list: none

    let layer_offset = out.len();
    be32(&mut out, 2);
    be32(&mut out, 1);
    be32(&mut out, 4); // layer type: indexed
    let name = b"Indexed\0";
    be32(&mut out, name.len() as u32);
    out.extend_from_slice(name);
    be32(&mut out, 0); // PROP_END
    be32(&mut out, 0);
    let hierarchy_pointer_at = out.len();
    be64(&mut out, 0);
    let mask_pointer_at = out.len();
    be64(&mut out, 0);

    // The layer's hierarchy: one channel of palette indices.
    let hierarchy_offset = out.len();
    be32(&mut out, 2);
    be32(&mut out, 1);
    be32(&mut out, 1); // bytes per pixel: an index
    let level_pointer_at = out.len();
    be64(&mut out, 0);
    be64(&mut out, 0);

    let level_offset = out.len();
    be32(&mut out, 2);
    be32(&mut out, 1);
    let tile_pointer_at = out.len();
    be64(&mut out, 0);
    let tile_terminator_at = out.len();
    be64(&mut out, 0);

    let tile_offset = out.len();
    out.extend_from_slice(&[0, 1]); // index 0, then index 1
    let after_tile = out.len();

    // The mask is a CHANNEL structure: geometry, name, properties, hierarchy.
    let mask_offset = out.len();
    be32(&mut out, 2);
    be32(&mut out, 1);
    let mask_name = b"Mask\0";
    be32(&mut out, mask_name.len() as u32);
    out.extend_from_slice(mask_name);
    be32(&mut out, 0); // PROP_END
    be32(&mut out, 0);
    let mask_hierarchy_pointer_at = out.len();
    be64(&mut out, 0);

    let mask_hierarchy_offset = out.len();
    be32(&mut out, 2);
    be32(&mut out, 1);
    be32(&mut out, 1);
    let mask_level_pointer_at = out.len();
    be64(&mut out, 0);
    be64(&mut out, 0);

    let mask_level_offset = out.len();
    be32(&mut out, 2);
    be32(&mut out, 1);
    let mask_tile_pointer_at = out.len();
    be64(&mut out, 0);
    let mask_terminator_at = out.len();
    be64(&mut out, 0);

    let mask_tile_offset = out.len();
    out.extend_from_slice(&[255, 0]); // shows the first pixel, hides the second
    let after_mask_tile = out.len();

    for (at, value) in [
        (layer_pointer_at, layer_offset),
        (hierarchy_pointer_at, hierarchy_offset),
        (mask_pointer_at, mask_offset),
        (level_pointer_at, level_offset),
        (tile_pointer_at, tile_offset),
        (tile_terminator_at, after_tile),
        (mask_hierarchy_pointer_at, mask_hierarchy_offset),
        (mask_level_pointer_at, mask_level_offset),
        (mask_tile_pointer_at, mask_tile_offset),
        (mask_terminator_at, after_mask_tile),
    ] {
        out[at..at + 8].copy_from_slice(&(value as u64).to_be_bytes());
    }
    out
}

#[test]
fn xcf_reads_indexed_colour_through_its_colormap() {
    // A layer's hierarchy declares only that a pixel is ONE byte; the image's base type says whether
    // that byte is grey or a palette index. Read as greyscale this image would come out as a picture of
    // its indices -- near-black and banded, not obviously wrong.
    let decoded = import_document(&xcf_indexed_with_mask(), &ImportOptions::default()).unwrap();
    assert_eq!(
        decoded.document().layers()[0].pixels(),
        vec![10, 20, 30, 255, 200, 150, 100, 255]
    );
}

#[test]
fn xcf_reads_a_layer_mask_as_its_own_channel_structure() {
    let decoded = import_document(&xcf_indexed_with_mask(), &ImportOptions::default()).unwrap();
    let mask = decoded.document().layers()[0]
        .mask()
        .expect("the layer mask should survive import");
    assert!(mask.is_enabled());
    assert_eq!(mask.pixels(), vec![255, 0]);
}

#[test]
fn xcf_rejects_an_indexed_image_with_no_colormap() {
    // Without the palette an index is only a number, so inventing colours would be worse than refusing.
    let mut bytes = xcf_indexed_with_mask();
    // Turn the COLORMAP property id into an unknown one, which is then skipped by its length.
    let colormap_id_at = 9 + 5 + 4 * 4;
    bytes[colormap_id_at..colormap_id_at + 4].copy_from_slice(&999_u32.to_be_bytes());
    let error = import_document(&bytes, &ImportOptions::default()).unwrap_err();
    assert!(matches!(
        error,
        redrob_core::CoreError::Format(FormatError::Malformed(_))
    ));
}

#[test]
fn dds_writes_dxt1_for_an_opaque_image_and_reads_back_flat_colour() {
    // A flat colour is the case block compression reproduces EXACTLY: both endpoints quantise to the
    // same value, so every index is 0. It is therefore the only honest exact-equality assertion for a
    // lossy container, and it still proves the header, the FourCC and the block layout.
    let mut pixels = Vec::new();
    for _ in 0..(8 * 8) {
        pixels.extend_from_slice(&[64, 128, 192, 255]);
    }
    let document = raster_document(8, 8, pixels.clone());
    let encoded = export_document(&document, FileFormat::Dds, &ExportOptions::default()).unwrap();
    assert_eq!(detect_format(encoded.bytes()).unwrap(), FileFormat::Dds);
    // Opaque image: BC1, which is half the bytes and has no alpha block.
    assert_eq!(&encoded.bytes()[84..88], b"DXT1");
    assert_eq!(encoded.bytes().len(), 128 + 4 * 8);
    // A flat image is exact, so nothing is reported lost.
    assert!(!encoded.warnings().iter().any(|w| matches!(
        w,
        FormatWarning::BlockCompressed { .. }
    )));

    let decoded = import_document(encoded.bytes(), &ImportOptions::default()).unwrap();
    let out = decoded.document().layers()[0].pixels().to_vec();
    // 5:6:5 quantisation is the one loss a flat block still takes, so compare within a step.
    for (actual, expected) in out.chunks_exact(4).zip(pixels.chunks_exact(4)) {
        for channel in 0..3 {
            assert!(
                actual[channel].abs_diff(expected[channel]) <= 8,
                "{actual:?} vs {expected:?}"
            );
        }
        assert_eq!(actual[3], 255);
    }
}

#[test]
fn dds_writes_dxt5_when_the_image_has_alpha() {
    // The form is chosen by the IMAGE, not by an option: BC1 for an image with alpha would discard it
    // silently, and BC3 for an opaque one doubles the file for an alpha block that is all 255.
    let mut pixels = Vec::new();
    for i in 0..(8 * 8) {
        let alpha = if i % 2 == 0 { 255 } else { 0 };
        pixels.extend_from_slice(&[200, 100, 50, alpha]);
    }
    let document = raster_document(8, 8, pixels);
    let encoded = export_document(&document, FileFormat::Dds, &ExportOptions::default()).unwrap();
    assert_eq!(&encoded.bytes()[84..88], b"DXT5");
    // BC3 is 16 bytes per block: an alpha block plus a colour block.
    assert_eq!(encoded.bytes().len(), 128 + 4 * 16);
    // Hard 0 and 255 alpha survives exactly, because the alpha endpoints are those two values.
    let decoded = import_document(encoded.bytes(), &ImportOptions::default()).unwrap();
    let out = decoded.document().layers()[0].pixels().to_vec();
    for (i, pixel) in out.chunks_exact(4).enumerate() {
        assert_eq!(pixel[3], if i % 2 == 0 { 255 } else { 0 });
    }
    // The colour is approximated, and that is reported rather than implied.
    assert!(encoded.warnings().iter().any(|w| matches!(
        w,
        FormatWarning::BlockCompressed { fourcc } if *fourcc == "DXT5"
    )));
    // A lossy container must not claim to be lossless.
    assert!(!encoded.metadata().lossless);
}

#[test]
fn dds_pads_an_edge_block_by_repeating_the_edge() {
    // 5x5 is not a multiple of 4, so the right and bottom blocks extend past the image. Those pixels
    // REPEAT the edge: zero-padding would drag the endpoints of every edge block toward black and
    // darken the visible pixels inside it, which is why this asserts a bright flat image stays bright.
    let mut pixels = Vec::new();
    for _ in 0..(5 * 5) {
        pixels.extend_from_slice(&[240, 240, 240, 255]);
    }
    let document = raster_document(5, 5, pixels);
    let encoded = export_document(&document, FileFormat::Dds, &ExportOptions::default()).unwrap();
    // Two blocks across, two down.
    assert_eq!(encoded.bytes().len(), 128 + 4 * 8);
    let decoded = import_document(encoded.bytes(), &ImportOptions::default()).unwrap();
    for pixel in decoded.document().layers()[0].pixels().chunks_exact(4) {
        assert!(pixel[0] > 200, "edge padding darkened the image: {pixel:?}");
    }
}
