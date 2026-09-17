// SPDX-License-Identifier: GPL-3.0-or-later

use std::io::{Cursor, Write};

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
