// SPDX-License-Identifier: GPL-3.0-or-later

use std::io::{Cursor, Read, Write};

use image::{ColorType, ImageEncoder};
use redrob_core::{
    AlphaPolicy, BlendMode, BrushPoint, BrushSettings, Command, CoreError, Document,
    DocumentImportBuilder, DocumentMetadata, EMBEDDED_FONT_ID, Editor, ExportOptions, FileFormat,
    FormatError, FormatWarning, FrameId, ImportMask, ImportNode, ImportOptions, LayerId,
    LossPolicy, PathCommand, Pixel, PlaybackMetadata, RasterCel, Rect, RenderSnapshot,
    SelectionMode, TextContent, VectorContent, VectorPath, detect_format, export_document,
    export_png, import_document, import_png,
};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

/// One pixel of a layer, for the colour-mode tests.
fn pixel(editor: &Editor, layer: LayerId, x: u32, y: u32) -> Pixel {
    editor
        .document()
        .layer(layer)
        .unwrap()
        .pixel(editor.document().width(), x, y)
        .unwrap()
}

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

    // A transform is now baked into geometry (H.20), so the refusal that remains is CSS: a `style`
    // attribute can restate any presentation property, and honouring one of those while ignoring the
    // rest would render a file nobody authored.
    let styled = br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1" fill="#000"/><path style="fill:red" d="M0 0L1 1" stroke="#000" fill="none"/></svg>"##;
    assert!(matches!(
        import_document(styled, &ImportOptions::default()),
        Err(redrob_core::CoreError::Format(
            FormatError::UnsupportedFeature("SVG CSS and event handlers")
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
    let pixels = vec![
        200, 10, 30, 255, 10, 200, 30, 255, 30, 10, 200, 255, 90, 90, 90, 255,
    ];
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
fn jxl_is_decoded_and_malformed_input_is_rejected_as_malformed() {
    // JPEG-XL codestream magic.
    let jxl = [0xff, 0x0a, 0, 0, 0, 0, 0, 0];
    assert_eq!(detect_format(&jxl).unwrap(), FileFormat::JpegXl);
    // Detected but truncated: now that a decoder is wired, the failure must be MALFORMED rather than
    // "unsupported feature" — the distinction is what tells a caller whether the file or the product is
    // the problem.
    let error = import_document(&jxl, &ImportOptions::default()).unwrap_err();
    assert!(
        matches!(
            error,
            redrob_core::CoreError::Format(FormatError::Malformed(_))
        ),
        "{error:?}"
    );
}

/// Builds a minimal ISO base media container: an `ftyp` with the given brands, then
/// meta > iprp > ipco > ispe carrying the primary image's size. No codec payload — the point is the
/// container, which is the half this product reads.
fn isobmff(major: &[u8; 4], compatible: &[&[u8; 4]], width: u32, height: u32) -> Vec<u8> {
    fn boxed(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut out = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(payload);
        out
    }

    let mut ispe = vec![0u8; 4]; // version and flags
    ispe.extend_from_slice(&width.to_be_bytes());
    ispe.extend_from_slice(&height.to_be_bytes());
    let ipco = boxed(b"ipco", &boxed(b"ispe", &ispe));
    let iprp = boxed(b"iprp", &ipco);
    let mut meta_payload = vec![0u8; 4]; // meta is a full box: version and flags first
    meta_payload.extend_from_slice(&iprp);
    let meta = boxed(b"meta", &meta_payload);

    let mut ftyp_payload = major.to_vec();
    ftyp_payload.extend_from_slice(b"\0\0\0\0"); // minor version
    for brand in compatible {
        ftyp_payload.extend_from_slice(*brand);
    }
    let mut out = boxed(b"ftyp", &ftyp_payload);
    out.extend_from_slice(&meta);
    out
}

#[test]
fn avif_is_told_apart_from_heif_by_its_brands() {
    // Both formats are the SAME container with different codecs inside, so a file whose major brand is
    // the generic `mif1` is AVIF when `avif` appears among its compatible brands. Folding the two into
    // one name refuses an AVIF with a message about HEVC, which sends the user after the wrong thing.
    let avif = isobmff(b"mif1", &[b"mif1", b"avif"], 32, 16);
    assert_eq!(detect_format(&avif).unwrap(), FileFormat::Avif);
    let heif = isobmff(b"heic", &[b"mif1"], 32, 16);
    assert_eq!(detect_format(&heif).unwrap(), FileFormat::Heif);
}

#[test]
fn heif_and_avif_refuse_by_codec_after_reading_the_container() {
    // A well-formed container refuses because of the CODEC, naming which one.
    for (bytes, needle) in [
        (isobmff(b"avif", &[b"avif"], 8, 8), "AV1"),
        (isobmff(b"heic", &[b"mif1"], 8, 8), "HEVC"),
    ] {
        let error = import_document(&bytes, &ImportOptions::default()).unwrap_err();
        match error {
            redrob_core::CoreError::Format(FormatError::UnsupportedFeature(message)) => {
                assert!(message.contains(needle), "{message}");
            }
            other => panic!("expected an unsupported-codec error, got {other:?}"),
        }
    }
}

#[test]
fn a_corrupt_heif_container_fails_as_malformed_not_unsupported() {
    // The container is READ before the codec is refused, so a truncated file says the FILE is the
    // problem. Without that, every broken AVIF looks like a missing feature.
    let mut truncated = isobmff(b"avif", &[b"avif"], 8, 8);
    assert_eq!(detect_format(&truncated).unwrap(), FileFormat::Avif);
    // Drop the meta box, leaving only the brands.
    truncated.truncate(24);
    let error = import_document(&truncated, &ImportOptions::default()).unwrap_err();
    assert!(
        matches!(
            error,
            redrob_core::CoreError::Format(FormatError::Malformed(_))
        ),
        "{error:?}"
    );
}

#[test]
fn heif_detects_but_is_unsupported() {
    // HEIF ftyp box. Still unsupported: its codec is HEVC, which has no pure-Rust decoder to wire.
    let mut heif = vec![0, 0, 0, 0x18];
    heif.extend_from_slice(b"ftypheic");
    heif.extend_from_slice(&[0u8; 8]);
    assert_eq!(detect_format(&heif).unwrap(), FileFormat::Heif);
    let error = import_document(&heif, &ImportOptions::default()).unwrap_err();
    assert!(matches!(
        error,
        redrob_core::CoreError::Format(FormatError::Malformed(_))
            | redrob_core::CoreError::Format(FormatError::UnsupportedFeature(_))
    ));
}

#[test]
fn pdf_and_camera_raw_detection() {
    // PDF magic. Reading one is covered by the PDF tests below; this only pins detection.
    let pdf = b"%PDF-1.7\n...".to_vec();
    assert_eq!(detect_format(&pdf).unwrap(), FileFormat::Pdf);

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
    let document = raster_document(
        2,
        2,
        vec![
            200, 10, 30, 255, 10, 200, 30, 255, 30, 10, 200, 255, 90, 90, 90, 255,
        ],
    );
    // Animated GIF export (single frame here) is a valid GIF detected by its header.
    let gif = export_document(&document, FileFormat::Gif, &ExportOptions::default()).unwrap();
    assert_eq!(detect_format(gif.bytes()).unwrap(), FileFormat::Gif);
    // APNG export starts with the PNG signature and carries an acTL chunk.
    let apng = export_document(&document, FileFormat::Apng, &ExportOptions::default()).unwrap();
    assert!(
        apng.bytes().starts_with(&[0x89, b'P', b'N', b'G']),
        "APNG has the PNG signature"
    );
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
    assert!(
        bytes.windows(4).any(|w| w == b"ANIM"),
        "animation parameters"
    );
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

    let webp = export_document(
        editor.document(),
        FileFormat::WebpAnim,
        &ExportOptions::default(),
    )
    .unwrap();
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
        assert!(matches!(
            vector.paths[0].commands[1],
            PathCommand::CubicTo { .. }
        ));
        assert!(matches!(vector.paths[0].commands[5], PathCommand::Close));
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
    // J.1c: the depth is KEPT. The document is 16-bit and the stored samples are our own
    // little-endian encoding of the same unit values, full scale being 65535 rather than 32768.
    assert_eq!(
        decoded.document().precision(),
        redrob_core::precision::Precision::U16
    );
    let pixels = decoded.document().layers()[0].pixels().to_vec();
    assert_eq!(
        pixels.len(),
        2 * 4 * 2,
        "two pixels, four samples, two bytes each"
    );
    let sample = |index: usize| u16::from_le_bytes([pixels[index * 2], pixels[index * 2 + 1]]);
    assert_eq!(sample(0), 65535, "red full scale");
    assert_eq!(sample(1), 0);
    assert_eq!(sample(2), 32768, "blue mid: 16384/32768 of full scale");
    assert_eq!(sample(3), 65535, "no alpha plane means opaque");

    // Nothing was narrowed, so nothing is reported. A warning left in place here would be worse
    // than silence: it trains a reader to ignore the one case that still matters.
    assert!(
        !decoded
            .warnings()
            .iter()
            .any(|warning| matches!(warning, FormatWarning::NarrowedDepth { .. })),
        "a 16-bit RGB file keeps its depth, so it must not report narrowing"
    );
}

#[test]
fn psd_sixteen_bit_import_keeps_detail_eight_bit_would_merge() {
    // The acceptance that matters for keeping depth: two samples ONE 16-bit step apart must stay
    // distinct. Both land on byte 128 at 8-bit, so a path that narrows anywhere -- on decode, in
    // composition, or on the way into storage -- merges them and this fails. Asserting only that
    // the precision FIELD says 16-bit would pass while the pixels were flattened.
    let mut planes = Vec::new();
    for value in [16384u16, 16385] {
        planes.extend_from_slice(&value.to_be_bytes()); // red row
    }
    planes.extend_from_slice(&[0u8; 4]); // green
    planes.extend_from_slice(&[0u8; 4]); // blue
    let bytes = deep_psd(2, 1, 16, 0, &planes);

    let decoded = import_document(&bytes, &ImportOptions::default()).unwrap();
    let pixels = decoded.document().layers()[0].pixels().to_vec();
    let red_of = |pixel: usize| {
        let index = pixel * 4;
        u16::from_le_bytes([pixels[index * 2], pixels[index * 2 + 1]])
    };
    // EXACT values, not merely "different". An assert_ne here passes even when the decode narrows
    // and the 16-bit reader is then reading pairs of unrelated bytes -- measured: it did. Two
    // samples that both become byte 128 at 8-bit must land on their own full-scale values.
    assert_eq!(red_of(0), 32768, "16384/32768 of full scale");
    assert_eq!(
        red_of(1),
        32770,
        "one 16-bit step above it, which 8-bit cannot hold"
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
        decoded.document().precision(),
        redrob_core::precision::Precision::U16
    );
    let pixels = decoded.document().layers()[0].pixels().to_vec();
    let sample = |index: usize| u16::from_le_bytes([pixels[index * 2], pixels[index * 2 + 1]]);
    assert_eq!(sample(0), 65535);
    assert_eq!(sample(1), 0);
    assert_eq!(sample(2), 32768);
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
    // J.1c: a 32-bit RGB file is imported AS float, so the transfer function is still applied but
    // the result is no longer squeezed into a byte.
    assert_eq!(
        decoded.document().precision(),
        redrob_core::precision::Precision::F32
    );
    let pixels = decoded.document().layers()[0].pixels().to_vec();
    let sample = |index: usize| {
        f32::from_le_bytes([
            pixels[index * 4],
            pixels[index * 4 + 1],
            pixels[index * 4 + 2],
            pixels[index * 4 + 3],
        ])
    };
    assert_eq!(sample(0), 1.0, "linear 1.0 encodes to full scale");
    let encoded_half = sample(4);
    assert!(
        (0.72..=0.75).contains(&encoded_half),
        "linear 0.5 should encode near 0.735 (byte 188), got {encoded_half}"
    );
    assert!(
        !decoded
            .warnings()
            .iter()
            .any(|warning| matches!(warning, FormatWarning::NarrowedDepth { .. })),
        "a 32-bit RGB file keeps its depth as float"
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
fn psd_sixteen_bit_cmyk_narrows_rather_than_taking_the_rgb_shaped_deep_path() {
    // Depth alone does not decide whether depth can be KEPT (J.1c). A 16-bit CMYK file has depth
    // worth keeping, but the conversion out of CMYK -- inverted ink, four planes, the fourth being
    // black and not alpha -- is written against bytes. Keeping the depth here would send it down a
    // path that reads planes positionally as red/green/blue and drops the black plane entirely, so
    // a print document would open with wrong colours and no complaint.
    //
    // Found by reverse-verification: widening `keep_depth_precision` to every colour mode broke no
    // test, because nothing covered a deep non-RGB file.
    let mut planes = Vec::new();
    for value in [0u16, 32768] {
        planes.extend_from_slice(&value.to_be_bytes()); // cyan: full ink, then none
    }
    for _ in 0..3 {
        for _ in 0..2 {
            planes.extend_from_slice(&32768u16.to_be_bytes()); // magenta, yellow, black: no ink
        }
    }
    let bytes = mode_psd(2, 1, 16, 4, 4, &[], &planes);

    let decoded = import_document(&bytes, &ImportOptions::default()).unwrap();
    assert_eq!(
        decoded.document().precision(),
        redrob_core::precision::Precision::U8,
        "a deep CMYK file narrows, because its conversion is 8-bit"
    );
    // And the colours are the ones the 8-bit CMYK path produces: full cyan ink, fully opaque.
    assert_eq!(
        &decoded.document().layers()[0].pixels()[0..4],
        &[0, 255, 255, 255]
    );
    // The narrowing is real here, so it IS reported.
    assert!(
        decoded
            .warnings()
            .contains(&FormatWarning::NarrowedDepth { source_bits: 16 }),
        "a depth that really is dropped must still be reported"
    );
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
            .contains(&FormatWarning::ConvertedColorMode {
                source: "grayscale"
            })
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
    assert!(
        pixels[0] >= 250 && pixels[1] >= 250 && pixels[2] >= 250,
        "{pixels:?}"
    );
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
    let decoded =
        import_document(&psd_with_mask_and_adjustment(), &ImportOptions::default()).unwrap();
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
    let decoded =
        import_document(&psd_with_mask_and_adjustment(), &ImportOptions::default()).unwrap();
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
    assert!(
        !decoded
            .warnings()
            .contains(&FormatWarning::FlattenedHierarchy)
    );
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
    assert_eq!(
        decoded.document().layers()[0].pixels(),
        vec![180, 90, 30, 255]
    );
}

#[test]
fn kra_export_writes_the_native_tiled_device_not_a_png() {
    // What makes this item worth doing: a file only this product can open is not a KRA. The layer entry
    // must be the tiled paint device Krita reads, with its default-pixel sidecar beside it.
    let document = raster_document(
        2,
        2,
        vec![
            10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255, 1, 2, 3, 4,
        ],
    );
    let encoded = export_document(&document, FileFormat::Kra, &ExportOptions::default()).unwrap();
    let mut archive = ZipArchive::new(Cursor::new(encoded.bytes())).unwrap();
    let names: Vec<String> = (0..archive.len())
        .map(|i| archive.by_index(i).unwrap().name().to_owned())
        .collect();
    assert!(
        names.iter().any(|n| n.ends_with("/layers/layer0")),
        "expected a native tiled device, got {names:?}"
    );
    assert!(
        names
            .iter()
            .any(|n| n.ends_with("/layers/layer0.defaultpixel"))
    );
    assert!(
        !names.iter().any(|n| n.ends_with("/layers/layer0.png")),
        "the PNG convention should be gone: {names:?}"
    );
    // The device's own header, so the entry is not merely named like one.
    let mut entry = archive
        .by_name(
            names
                .iter()
                .find(|n| n.ends_with("/layers/layer0"))
                .unwrap()
                .as_str(),
        )
        .unwrap();
    let mut device = Vec::new();
    entry.read_to_end(&mut device).unwrap();
    assert!(
        device.starts_with(b"VERSION 2\n"),
        "{:?}",
        &device[..16.min(device.len())]
    );
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
    assert!(
        encoded.bytes().len() < 6 * 64 * 64 * 4,
        "{}",
        encoded.bytes().len()
    );
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
fn xcf_reads_gimps_own_2_6_test_file_with_empty_levels() {
    // GIMP's own fixture (app/tests/files/gimp-2-6-file.xcf, GPL-3.0-or-later). Its layers were never
    // painted, so each level's tile list is a lone 0, which GIMP's loader reads as "empty level".
    // Reading a full tile count regardless walked past the terminator: "XCF truncated".
    let bytes = include_bytes!("fixtures/gimp-2-6-file.xcf");
    assert_eq!(detect_format(bytes).unwrap(), FileFormat::Xcf);
    let decoded = import_document(bytes, &ImportOptions::default()).unwrap();
    let document = decoded.document();
    assert_eq!((document.width(), document.height()), (100, 90));
    let names: Vec<_> = document
        .layers()
        .iter()
        .map(|l| l.name().to_string())
        .collect();
    // XCF stores top-first; the stack here is bottom-first.
    assert_eq!(names, ["layer1", "layer2"]);
    // Never painted: every level is empty, so every colour sample stays 0 (layer2 has no alpha
    // channel, so it is opaque black; layer1's alpha is 0). A reader that walks past the empty list
    // reads the NEXT structure's numbers as tile offsets and decodes garbage here.
    for layer in document.layers() {
        assert!(
            layer.pixels().chunks(4).all(|p| p[..3] == [0, 0, 0]),
            "{} decoded pixels from an empty level",
            layer.name()
        );
    }
    assert!(
        document.layers()[1].mask().is_some(),
        "layer2's mask was dropped"
    );
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
    assert!(
        !encoded
            .warnings()
            .iter()
            .any(|w| matches!(w, FormatWarning::BlockCompressed { .. }))
    );

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
    // darken the visible pixels inside it.
    //
    // Asserted on the ENCODED BLOCKS rather than by round-tripping, and the reason is a real
    // limitation worth recording: `image`'s DXT decoder refuses any width or height that is not a
    // multiple of 4, so a file like this one — which is valid DDS, and which real tools produce —
    // cannot be read back by this product's own importer. Round-tripping at a 4-multiple size would
    // pass without ever exercising the padding, which is the only thing this test is about.
    let mut pixels = Vec::new();
    for _ in 0..(5 * 5) {
        pixels.extend_from_slice(&[240, 240, 240, 255]);
    }
    let document = raster_document(5, 5, pixels);
    let encoded = export_document(&document, FileFormat::Dds, &ExportOptions::default()).unwrap();
    // Two blocks across, two down, BC1 at 8 bytes each after the 128-byte header.
    assert_eq!(encoded.bytes().len(), 128 + 4 * 8);
    // A BC1 block is two RGB565 endpoints then four bytes of 2-bit indices. Every block here covers
    // flat bright pixels, so BOTH endpoints must be bright — a zero-padded edge block would put one
    // endpoint at black and the indices would then interpolate the visible pixels toward it.
    for block in encoded.bytes()[128..].chunks_exact(8) {
        for endpoint in [
            u16::from_le_bytes([block[0], block[1]]),
            u16::from_le_bytes([block[2], block[3]]),
        ] {
            let red5 = endpoint >> 11;
            let green6 = (endpoint >> 5) & 0x3F;
            let blue5 = endpoint & 0x1F;
            assert!(
                red5 >= 24 && green6 >= 48 && blue5 >= 24,
                "edge padding darkened a block endpoint: r{red5} g{green6} b{blue5}"
            );
        }
    }
}

/// Builds a one-page PDF whose only content is a raw 8-bit DeviceRGB image XObject, which is the shape
/// a scan or a flattened export takes. Hand-built with a real cross-reference table, because the thing
/// under test is reaching a page's resources — a fake that skips the xref would not exercise that.
fn pdf_with_rgb_image(width: u32, height: u32, rgb: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut offsets = Vec::new();
    out.extend_from_slice(b"%PDF-1.7\n");

    let object = |out: &mut Vec<u8>, offsets: &mut Vec<usize>, body: &[u8]| {
        offsets.push(out.len());
        let number = offsets.len();
        out.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };

    object(&mut out, &mut offsets, b"<< /Type /Catalog /Pages 2 0 R >>");
    object(
        &mut out,
        &mut offsets,
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    );
    object(
        &mut out,
        &mut offsets,
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] \
             /Resources << /XObject << /Im0 4 0 R >> >> /Contents 5 0 R >>"
        )
        .as_bytes(),
    );
    let mut image_object = format!(
        "<< /Type /XObject /Subtype /Image /Width {width} /Height {height} \
         /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length {} >>\nstream\n",
        rgb.len()
    )
    .into_bytes();
    image_object.extend_from_slice(rgb);
    image_object.extend_from_slice(b"\nendstream");
    object(&mut out, &mut offsets, &image_object);
    let content = b"q 1 0 0 1 0 0 cm /Im0 Do Q";
    let mut content_object = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
    content_object.extend_from_slice(content);
    content_object.extend_from_slice(b"\nendstream");
    object(&mut out, &mut offsets, &content_object);

    let xref_at = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", offsets.len() + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for offset in &offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n",
            offsets.len() + 1
        )
        .as_bytes(),
    );
    out
}

#[test]
fn pdf_reads_the_first_pages_embedded_image() {
    // The case this product is actually asked for: a page that IS an image. Reaching it is object
    // parsing, not rasterisation, which is why it does not need a graphics engine.
    let rgb = vec![
        10, 20, 30, 40, 50, 60, //
        70, 80, 90, 100, 110, 120,
    ];
    let bytes = pdf_with_rgb_image(2, 2, &rgb);
    assert_eq!(detect_format(&bytes).unwrap(), FileFormat::Pdf);
    let decoded = import_document(&bytes, &ImportOptions::default()).unwrap();
    assert_eq!(
        decoded.document().layers()[0].pixels(),
        vec![
            10, 20, 30, 255, 40, 50, 60, 255, //
            70, 80, 90, 255, 100, 110, 120, 255
        ]
    );
}

#[test]
fn pdf_without_an_image_says_it_is_drawn_rather_than_scanned() {
    // A drawn page is refused by NAME. A half-written content-stream interpreter would render
    // something for this file, and a page that silently lost its text would look like our bug.
    let mut bytes = pdf_with_rgb_image(2, 2, &[0u8; 12]);
    // Remove the XObject resource so the page has no image, leaving the rest of the file valid.
    let patched = String::from_utf8_lossy(&bytes)
        .replace("/XObject << /Im0 4 0 R >>", "/XObject <<            >>");
    bytes = patched.into_bytes();
    let error = import_document(&bytes, &ImportOptions::default()).unwrap_err();
    match error {
        redrob_core::CoreError::Format(FormatError::UnsupportedFeature(message)) => {
            assert!(message.contains("graphics engine"), "{message}");
        }
        other => panic!("expected a named refusal, got {other:?}"),
    }
}

#[test]
fn a_corrupt_pdf_fails_as_malformed() {
    // Detected by magic, unreadable as structure: the FILE is the problem, and the error says so.
    let bytes = b"%PDF-1.7\nnot actually a pdf".to_vec();
    assert_eq!(detect_format(&bytes).unwrap(), FileFormat::Pdf);
    let error = import_document(&bytes, &ImportOptions::default()).unwrap_err();
    assert!(
        matches!(
            error,
            redrob_core::CoreError::Format(FormatError::Malformed(_))
        ),
        "{error:?}"
    );
}

#[test]
fn camera_raw_is_developed_and_a_corrupt_one_fails_as_malformed() {
    // A Canon CR2 header is detected as Raw, not TIFF. With no sensor data behind it the UNPACKER
    // fails, and the error must say the FILE is the problem rather than claim camera raw is unsupported
    // — the pipeline is wired now, so "unsupported" would be a lie.
    let mut cr2 = b"II*\x00".to_vec();
    cr2.extend_from_slice(&[0, 0, 0, 0]);
    cr2.extend_from_slice(b"CR");
    cr2.extend_from_slice(&[0u8; 16]);
    assert_eq!(detect_format(&cr2).unwrap(), FileFormat::Raw);
    let error = import_document(&cr2, &ImportOptions::default()).unwrap_err();
    assert!(
        matches!(
            error,
            redrob_core::CoreError::Format(FormatError::Malformed(_))
        ),
        "{error:?}"
    );
}

/// Builds a minimal matrix-shaper RGB ICC profile whose primaries are WIDER than sRGB's, with a plain
/// gamma curve. Hand-built because the point is what the bytes mean: a fixture from some other tool
/// would prove only that we agree with it.
fn wide_gamut_icc() -> Vec<u8> {
    // Adobe RGB's colorants, adapted to D50 as the specification requires.
    matrix_icc(
        (0.609_74, 0.311_11, 0.019_47),
        (0.205_28, 0.625_91, 0.060_87),
        (0.149_19, 0.063_0, 0.744_57),
    )
}

/// A profile NARROWER than sRGB, for the soft-proof and gamut-check tests (J.5).
///
/// Its primaries are pulled well in toward the white point, so a saturated sRGB colour genuinely
/// falls outside it. Built by desaturating Adobe RGB's colorants toward equal-energy white rather
/// than by inventing numbers, so the result is still a valid, self-consistent matrix profile — an
/// arbitrary matrix can be singular or non-invertible and would test the error path instead.
fn narrow_gamut_icc() -> Vec<u8> {
    let pull = |colorant: (f64, f64, f64)| {
        let white = (colorant.0 + colorant.1 + colorant.2) / 3.0;
        (
            colorant.0 * 0.45 + white * 0.55,
            colorant.1 * 0.45 + white * 0.55,
            colorant.2 * 0.45 + white * 0.55,
        )
    };
    matrix_icc(
        pull((0.609_74, 0.311_11, 0.019_47)),
        pull((0.205_28, 0.625_91, 0.060_87)),
        pull((0.149_19, 0.063_0, 0.744_57)),
    )
}

/// A minimal matrix-and-curve ICC profile with the given D50-adapted colorants and gamma 2.2.
fn matrix_icc(red: (f64, f64, f64), green: (f64, f64, f64), blue: (f64, f64, f64)) -> Vec<u8> {
    fn s15(out: &mut Vec<u8>, value: f64) {
        out.extend_from_slice(&((value * 65536.0).round() as i32).to_be_bytes());
    }
    fn xyz_tag(x: f64, y: f64, z: f64) -> Vec<u8> {
        let mut tag = b"XYZ ".to_vec();
        tag.extend_from_slice(&[0, 0, 0, 0]); // reserved
        s15(&mut tag, x);
        s15(&mut tag, y);
        s15(&mut tag, z);
        tag
    }
    // Gamma 2.2 as a single-entry curve, which is u8Fixed8 and not s15Fixed16.
    let mut curve = b"curv".to_vec();
    curve.extend_from_slice(&[0, 0, 0, 0]);
    curve.extend_from_slice(&1_u32.to_be_bytes());
    curve.extend_from_slice(&((2.2 * 256.0) as u16).to_be_bytes());

    let tags: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"rXYZ", xyz_tag(red.0, red.1, red.2)),
        (b"gXYZ", xyz_tag(green.0, green.1, green.2)),
        (b"bXYZ", xyz_tag(blue.0, blue.1, blue.2)),
        (b"rTRC", curve.clone()),
        (b"gTRC", curve.clone()),
        (b"bTRC", curve),
    ];

    let header_and_table = 132 + tags.len() * 12;
    let mut body = Vec::new();
    let mut table = Vec::new();
    for (signature, data) in &tags {
        let offset = header_and_table + body.len();
        table.extend_from_slice(*signature);
        table.extend_from_slice(&(offset as u32).to_be_bytes());
        table.extend_from_slice(&(data.len() as u32).to_be_bytes());
        body.extend_from_slice(data);
        // Tag data is padded to a four-byte boundary.
        while body.len() % 4 != 0 {
            body.push(0);
        }
    }

    let total = header_and_table + body.len();
    let mut out = vec![0u8; 132];
    out[0..4].copy_from_slice(&(total as u32).to_be_bytes());
    out[12..16].copy_from_slice(b"mntr"); // device class
    out[16..20].copy_from_slice(b"RGB "); // data colour space
    out[20..24].copy_from_slice(b"XYZ "); // PCS
    out[36..40].copy_from_slice(b"acsp"); // signature
    out[128..132].copy_from_slice(&(tags.len() as u32).to_be_bytes());
    out.extend_from_slice(&table);
    out.extend_from_slice(&body);
    out
}

/// Wraps an existing PNG's bytes with an `iCCP` chunk inserted before its first `IDAT`.
fn png_with_icc(png: &[u8], profile: &[u8]) -> Vec<u8> {
    let mut compressed =
        flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    compressed.write_all(profile).unwrap();
    let compressed = compressed.finish().unwrap();

    let mut payload = b"probe\0".to_vec(); // profile name, NUL-terminated
    payload.push(0); // compression method: deflate
    payload.extend_from_slice(&compressed);

    let mut chunk = (payload.len() as u32).to_be_bytes().to_vec();
    chunk.extend_from_slice(b"iCCP");
    chunk.extend_from_slice(&payload);
    // The CRC covers the type and the data. Computed here because a reader that validates it would
    // otherwise skip the chunk and the test would pass for the wrong reason.
    let mut crc_input = b"iCCP".to_vec();
    crc_input.extend_from_slice(&payload);
    let mut crc = 0xffff_ffffu32;
    for byte in &crc_input {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    chunk.extend_from_slice(&(crc ^ 0xffff_ffff).to_be_bytes());

    // Insert before the first IDAT.
    let idat = png
        .windows(4)
        .position(|window| window == b"IDAT")
        .expect("a PNG has an IDAT chunk");
    let mut out = png[..idat - 4].to_vec();
    out.extend_from_slice(&chunk);
    out.extend_from_slice(&png[idat - 4..]);
    out
}

#[test]
fn a_png_tagged_with_a_wide_gamut_profile_is_converted_to_srgb() {
    // The whole point of reading a profile: a saturated red in a WIDER space is not the same colour as
    // the same numbers in sRGB. Converting it must pull the value toward sRGB's own red, and leaving
    // the tag unread is what made wide-gamut photos open visibly dull.
    let pixels = vec![230, 30, 40, 255];
    let document = raster_document(1, 1, pixels.clone());
    let png = export_document(&document, FileFormat::Png, &ExportOptions::default()).unwrap();
    let tagged = png_with_icc(png.bytes(), &wide_gamut_icc());

    assert_eq!(detect_format(&tagged).unwrap(), FileFormat::Png);
    let decoded = import_document(&tagged, &ImportOptions::default()).unwrap();
    let out = decoded.document().layers()[0].pixels().to_vec();
    // The conversion happened: the pixel is not the untouched source.
    assert_ne!(out[..3], pixels[..3], "the profile was not applied");
    // Alpha is coverage, not colour, and must pass through untouched.
    assert_eq!(out[3], 255);
    // A wide-gamut red read into sRGB clips at the red primary and loses the other channels: the
    // direction is what matters, not the exact value.
    assert!(out[0] >= 240, "red should saturate, got {out:?}");
    assert!(out[2] <= 60, "blue should not grow, got {out:?}");
    // The conversion is reported rather than silent.
    assert!(
        decoded
            .warnings()
            .contains(&FormatWarning::ConvertedColorMode { source: "icc" })
    );
}

#[test]
fn an_untagged_png_is_left_exactly_alone() {
    // No profile means no transform: a file that says nothing about its colour must not be "corrected".
    let pixels = vec![230, 30, 40, 255, 10, 200, 90, 128];
    let document = raster_document(2, 1, pixels.clone());
    let png = export_document(&document, FileFormat::Png, &ExportOptions::default()).unwrap();
    let decoded = import_document(png.bytes(), &ImportOptions::default()).unwrap();
    assert_eq!(decoded.document().layers()[0].pixels(), pixels);
    assert!(decoded.warnings().is_empty());
}

#[test]
fn an_unreadable_icc_profile_is_dropped_and_the_image_still_opens() {
    // Renamed in J.5-b: table-based profiles are now READ (see
    // `a_profile_with_a_b2a0_table_honours_the_perceptual_intent`), so the old name described
    // behaviour that no longer exists. What this still pins is the surviving, more general rule: a
    // profile this product cannot interpret — here one whose red colorant tag was renamed to a
    // signature carrying no parseable `mft2` table — is DROPPED rather than made fatal. A file we
    // cannot colour-manage is not a file we should refuse to open.
    let mut profile = wide_gamut_icc();
    // Rewrite the red colorant's signature to A2B0, leaving a profile with a table and no matrix.
    let position = profile
        .windows(4)
        .position(|window| window == b"rXYZ")
        .unwrap();
    profile[position..position + 4].copy_from_slice(b"A2B0");

    let pixels = vec![200, 100, 50, 255];
    let document = raster_document(1, 1, pixels.clone());
    let png = export_document(&document, FileFormat::Png, &ExportOptions::default()).unwrap();
    let tagged = png_with_icc(png.bytes(), &profile);
    let decoded = import_document(&tagged, &ImportOptions::default()).unwrap();
    assert_eq!(decoded.document().layers()[0].pixels(), pixels);
}

/// Collects a vector node's path points, so a geometry assertion does not depend on command shape.
fn vector_points(document: &redrob_core::Document, index: usize) -> Vec<(f32, f32)> {
    let mut points = Vec::new();
    if let redrob_core::NodeContent::Vector { vector } = document.nodes()[index].content() {
        for path in &vector.paths {
            for command in &path.commands {
                match *command {
                    PathCommand::MoveTo { x, y } | PathCommand::LineTo { x, y } => {
                        points.push((x, y));
                    }
                    PathCommand::CubicTo { x, y, .. } => points.push((x, y)),
                    PathCommand::Close => {}
                }
            }
        }
    }
    points
}

#[test]
fn svg_imports_an_elliptical_arc_as_cubics() {
    // The arc command was refused outright. A half-circle arc from (10,20) to (30,20) with radius 10
    // must land ON its endpoint and bulge to y = 30 — the sweep flag picks which of four arcs this is,
    // and getting the centre's sign wrong draws the complementary one: a smooth curve the wrong way.
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="40px" height="40px" viewBox="0 0 40 40"><path d="M 10 20 A 10 10 0 0 1 30 20" fill="none" stroke="#000000" stroke-width="1"/></svg>"##;
    let imported = import_document(svg, &ImportOptions::default()).unwrap();
    let points = vector_points(imported.document(), 0);
    let last = *points.last().expect("the arc produced commands");
    assert!(
        (last.0 - 30.0).abs() < 0.1 && (last.1 - 20.0).abs() < 0.1,
        "an arc must end exactly on its endpoint, got {last:?}"
    );
    // Which way sweep=1 bulges is the thing that is easy to get backwards, so it is worth stating.
    // A point on the arc is (cx + r·cos θ, cy + r·sin θ); here the centre is (20,20), the start is
    // θ=180° and the end θ=360°. Sweep 1 means θ INCREASES, so the arc passes through θ=270°, which
    // is (20, 20 − 10) = (20,10) — upward on screen, because SVG's y axis points down. A positive-angle
    // sweep therefore looks clockwise and bulges toward DECREASING y.
    assert!(
        points.iter().any(|(_, y)| *y < 15.0),
        "the sweep flag chose the wrong arc: {points:?}"
    );
    // At most 90 degrees per cubic, so a half circle is at least two of them.
    assert!(points.len() >= 2, "{points:?}");
}

#[test]
fn svg_arc_with_radii_too_small_grows_them_instead_of_failing() {
    // The spec says radii too small to span the endpoints are SCALED UP until they fit. Refusing
    // instead would lose the segment, and a file can legitimately contain this.
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="40px" height="40px" viewBox="0 0 40 40"><path d="M 0 10 A 1 1 0 0 1 30 10" fill="#FF0000"/></svg>"##;
    let imported = import_document(svg, &ImportOptions::default()).unwrap();
    let points = vector_points(imported.document(), 0);
    let last = *points.last().unwrap();
    assert!((last.0 - 30.0).abs() < 0.1, "{last:?}");
}

#[test]
fn svg_bakes_a_transform_into_the_geometry() {
    // `transform` used to be refused for the whole file. It is now baked, because our vector nodes have
    // no transform of their own — the alternative is dropping it, and a dropped transform is a shape in
    // the wrong place with nothing reporting it.
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="40px" height="40px" viewBox="0 0 40 40"><rect x="0" y="0" width="10" height="10" transform="translate(5 7)" fill="#00FF00"/></svg>"##;
    let imported = import_document(svg, &ImportOptions::default()).unwrap();
    let points = vector_points(imported.document(), 0);
    assert!(
        points
            .iter()
            .any(|(x, y)| (*x - 5.0).abs() < 0.01 && (*y - 7.0).abs() < 0.01),
        "the rect's origin should have moved to (5, 7): {points:?}"
    );
}

#[test]
fn svg_composes_a_groups_transform_outside_its_childs() {
    // Order is the whole risk here. The child scales by 2 and the group translates by 10, so the
    // child's own transform applies FIRST: a point at 3 becomes 6, then 16. Composing the other way
    // would give (3 + 10) * 2 = 26 — a plausible number from the wrong matrix.
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="64px" height="64px" viewBox="0 0 64 64"><g transform="translate(10 0)"><rect x="3" y="0" width="4" height="4" transform="scale(2)" fill="#0000FF"/></g></svg>"##;
    let imported = import_document(svg, &ImportOptions::default()).unwrap();
    // Nodes are stored bottom-first and a group is emitted when it CLOSES, so the group node comes
    // after the child it contains: the rect is node 0 and the Group is last.
    let points = vector_points(imported.document(), 0);
    assert!(
        points.iter().any(|(x, _)| (*x - 16.0).abs() < 0.01),
        "expected 3 * 2 + 10 = 16, got {points:?}"
    );
    assert!(
        !points.iter().any(|(x, _)| (*x - 26.0).abs() < 0.01),
        "26 means the transforms composed in the wrong order: {points:?}"
    );
}

#[test]
fn svg_pops_a_groups_transform_so_siblings_are_unaffected() {
    // A transform left on the stack would silently apply to everything after the group closes.
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="64px" height="64px" viewBox="0 0 64 64"><g transform="translate(20 0)"><rect x="0" y="0" width="4" height="4" fill="#0000FF"/></g><rect x="1" y="1" width="4" height="4" fill="#FF0000"/></svg>"##;
    let imported = import_document(svg, &ImportOptions::default()).unwrap();
    let nodes = imported.document().nodes();
    let outside = vector_points(imported.document(), nodes.len() - 1);
    assert!(
        outside
            .iter()
            .any(|(x, y)| (*x - 1.0).abs() < 0.01 && (*y - 1.0).abs() < 0.01),
        "the sibling after the group must keep its own coordinates: {outside:?}"
    );
}

/// J.1c-b. A 16-bit TIFF is imported AT 16 bits, and the detail that 8 bits cannot hold survives.
///
/// Before this these formats went through the shared byte path, which narrowed them and -- unlike
/// the layered reader -- reported nothing at all, so the loss was invisible from both ends.
#[test]
fn sixteen_bit_tiff_imports_at_sixteen_bits_and_keeps_detail_eight_bits_cannot_hold() {
    use image::{ImageEncoder, Rgba};

    // Two reds one 16-bit step apart around mid grey. Both are byte 128 at 8-bit, so any narrowing
    // anywhere in the path merges them.
    let mut source = image::ImageBuffer::<Rgba<u16>, Vec<u16>>::new(2, 1);
    source.put_pixel(0, 0, Rgba([32896, 0, 0, 65535]));
    source.put_pixel(1, 0, Rgba([32897, 0, 0, 65535]));

    let mut bytes = Vec::new();
    image::codecs::tiff::TiffEncoder::new(std::io::Cursor::new(&mut bytes))
        .write_image(
            bytemuck_cast_u16_to_u8(source.as_raw()),
            2,
            1,
            image::ExtendedColorType::Rgba16,
        )
        .unwrap();

    assert_eq!(detect_format(&bytes).unwrap(), FileFormat::Tiff);
    let decoded = import_document(&bytes, &ImportOptions::default()).unwrap();
    assert_eq!(
        decoded.document().precision(),
        redrob_core::precision::Precision::U16,
        "a 16-bit TIFF must be imported at 16 bits, not narrowed"
    );
    let pixels = decoded.document().layers()[0].pixels().to_vec();
    let red_of = |pixel: usize| {
        let index = pixel * 4;
        u16::from_le_bytes([pixels[index * 2], pixels[index * 2 + 1]])
    };
    assert_eq!(red_of(0), 32896);
    assert_eq!(
        red_of(1),
        32897,
        "one 16-bit step apart; both are byte 128 at 8-bit, so narrowing anywhere merges them"
    );
}

/// An EXR is imported as float, and values ABOVE 1.0 survive.
///
/// An EXR is never 8-bit, and highlight headroom above full scale is the main reason to read one at
/// all. Clamping it into an integer on the way in is the loss that makes the format pointless, and
/// it is the loss the byte path performed silently.
#[test]
fn exr_imports_as_float_and_keeps_values_above_full_scale() {
    use image::{ImageEncoder, Rgba};

    let mut source = image::ImageBuffer::<Rgba<f32>, Vec<f32>>::new(2, 1);
    source.put_pixel(0, 0, Rgba([4.0, 0.25, 0.0, 1.0]));
    source.put_pixel(1, 0, Rgba([0.5, 0.5, 0.5, 1.0]));

    let mut bytes = Vec::new();
    image::codecs::openexr::OpenExrEncoder::new(std::io::Cursor::new(&mut bytes))
        .write_image(
            bytemuck_cast_f32_to_u8(source.as_raw()),
            2,
            1,
            image::ExtendedColorType::Rgba32F,
        )
        .unwrap();

    assert_eq!(detect_format(&bytes).unwrap(), FileFormat::Exr);
    let decoded = import_document(&bytes, &ImportOptions::default()).unwrap();
    assert_eq!(
        decoded.document().precision(),
        redrob_core::precision::Precision::F32,
        "an EXR is never 8-bit; importing one at 8 bits discards the format's whole point"
    );
    let pixels = decoded.document().layers()[0].pixels().to_vec();
    let sample = |index: usize| {
        f32::from_le_bytes([
            pixels[index * 4],
            pixels[index * 4 + 1],
            pixels[index * 4 + 2],
            pixels[index * 4 + 3],
        ])
    };
    assert!(
        sample(0) > 3.9,
        "a highlight above full scale must survive the import, got {}",
        sample(0)
    );
    assert!((sample(1) - 0.25).abs() < 0.001);
}

/// `&[u16]` as little-endian bytes, for building a deep TIFF fixture.
fn bytemuck_cast_u16_to_u8(samples: &[u16]) -> &[u8] {
    // Written by hand rather than pulling in a casting crate for two test fixtures. The encoder
    // wants native-endian bytes, which on every target this builds for is little-endian.
    unsafe { std::slice::from_raw_parts(samples.as_ptr() as *const u8, samples.len() * 2) }
}

/// `&[f32]` as native bytes, for building an EXR fixture.
fn bytemuck_cast_f32_to_u8(samples: &[f32]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(samples.as_ptr() as *const u8, samples.len() * 4) }
}

/// J.1c-c. A 16-bit PNG is imported AT 16 bits, and a profile on it is applied at 16 bits too.
///
/// This was the last of the image-backed formats still narrowing, and the reason it waited is the
/// interesting half: PNG import also applies an embedded profile, and that transform used to take
/// and return bytes. Preserving the depth and then colour-managing through a byte round trip would
/// have given back exactly what the narrowing gave — so both halves had to move together.
#[test]
fn sixteen_bit_png_imports_at_sixteen_bits_and_is_colour_managed_at_that_depth() {
    use image::{ImageEncoder, Rgba};

    // Two reds one 16-bit step apart. They are the same byte at 8-bit.
    let mut source = image::ImageBuffer::<Rgba<u16>, Vec<u16>>::new(2, 1);
    source.put_pixel(0, 0, Rgba([40000, 8000, 9000, 65535]));
    source.put_pixel(1, 0, Rgba([40001, 8000, 9000, 65535]));

    let mut plain = Vec::new();
    image::codecs::png::PngEncoder::new(std::io::Cursor::new(&mut plain))
        .write_image(
            u16_samples_as_bytes(source.as_raw()),
            2,
            1,
            image::ExtendedColorType::Rgba16,
        )
        .unwrap();

    // Untagged first: the depth survives on its own.
    let untagged = import_document(&plain, &ImportOptions::default()).unwrap();
    assert_eq!(
        untagged.document().precision(),
        redrob_core::precision::Precision::U16,
        "a 16-bit PNG must import at 16 bits"
    );
    let pixels = untagged.document().layers()[0].pixels().to_vec();
    let red_of = |pixels: &[u8], pixel: usize| {
        let index = pixel * 4;
        u16::from_le_bytes([pixels[index * 2], pixels[index * 2 + 1]])
    };
    assert_eq!(red_of(&pixels, 0), 40000);
    assert_eq!(
        red_of(&pixels, 1),
        40001,
        "one 16-bit step apart; the same byte at 8-bit, so any narrowing merges them"
    );

    // Tagged: the profile is applied, and applying it does NOT cost the depth. The two neighbouring
    // samples must still differ afterwards — a byte round trip inside the colour transform would
    // collapse them while leaving the precision field saying 16-bit.
    let tagged = png_with_icc(&plain, &wide_gamut_icc());
    let managed = import_document(&tagged, &ImportOptions::default()).unwrap();
    assert_eq!(
        managed.document().precision(),
        redrob_core::precision::Precision::U16
    );
    let converted = managed.document().layers()[0].pixels().to_vec();
    assert_ne!(
        red_of(&converted, 0),
        red_of(&pixels, 0),
        "the profile was not applied"
    );
    assert_ne!(
        red_of(&converted, 0),
        red_of(&converted, 1),
        "colour management must not quantise a 16-bit image to bytes on the way through"
    );
    assert!(
        managed
            .warnings()
            .contains(&FormatWarning::ConvertedColorMode { source: "icc" })
    );
}

/// `&[u16]` as native-endian bytes, for building a deep PNG or TIFF fixture.
fn u16_samples_as_bytes(samples: &[u16]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(samples.as_ptr() as *const u8, samples.len() * 2) }
}

/// J.3. An indexed document can be AUTHORED — palette built from the image, pixels snapped to it —
/// and exported as a palette PNG that carries that palette.
///
/// The product could already read an indexed PSD or XCF by converting it on import. Being able to
/// make one is a different capability, and the export is what proves the palette is the document's
/// property rather than a transient of the conversion.
#[test]
fn an_indexed_document_is_authored_and_exported_as_a_palette_png() {
    use redrob_core::{ColorMode, DitherMode, PaletteChoice};

    // Four distinct colours, two pixels each.
    let mut editor = Editor::new(Document::new(4, 2).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    for (index, color) in [
        Pixel::rgba(200, 10, 10, 255),
        Pixel::rgba(10, 200, 10, 255),
        Pixel::rgba(10, 10, 200, 255),
        Pixel::rgba(200, 200, 10, 255),
    ]
    .into_iter()
    .enumerate()
    {
        editor
            .execute(Command::SelectRectangle {
                rect: Rect::new(index as i32, 0, 1, 2),
                mode: SelectionMode::Replace,
            })
            .unwrap();
        editor.execute(Command::Fill { color }).unwrap();
    }
    editor.execute(Command::ClearSelection).unwrap();

    editor
        .execute(Command::ConvertColorMode {
            mode: ColorMode::Indexed,
            palette: Some(PaletteChoice::Generate { max_colors: 4 }),
            dither: DitherMode::None,
        })
        .unwrap();

    assert_eq!(editor.document().color_mode(), ColorMode::Indexed);
    assert_eq!(
        editor.document().palette().len(),
        4,
        "four distinct colours, four palette entries"
    );
    // Every pixel is now one of the palette's colours. This is the constraint the mode asserts.
    let palette: Vec<Pixel> = editor.document().palette().to_vec();
    for x in 0..4 {
        let got = pixel(&editor, layer, x, 0);
        assert!(
            palette
                .iter()
                .any(|entry| entry.r == got.r && entry.g == got.g && entry.b == got.b),
            "pixel {x} is {got:?}, which is not in the palette {palette:?}"
        );
    }

    // Export: a colour-type-3 PNG with a PLTE chunk, and the palette written out.
    let png = export_document(
        editor.document(),
        FileFormat::Png,
        &ExportOptions::default(),
    )
    .unwrap();
    let bytes = png.bytes();
    assert_eq!(detect_format(bytes).unwrap(), FileFormat::Png);
    // IHDR colour type is the 10th byte of the chunk data: width(4) height(4) depth(1) type(1).
    assert_eq!(bytes[25], 3, "colour type 3 is indexed");
    assert!(
        bytes.windows(4).any(|window| window == b"PLTE"),
        "an indexed PNG must carry its palette"
    );
    // And it reads back as the same picture through the ordinary decoder.
    let reread = import_document(bytes, &ImportOptions::default()).unwrap();
    assert_eq!(
        reread.document().layers()[0].pixels(),
        editor.document().layers()[0].pixels(),
        "the exported palette PNG decodes to the pixels it was made from"
    );
}

/// Greyscale conversion uses perceptual luma, not the channel mean.
///
/// The mean makes a saturated blue as bright as a mid grey, which is visibly wrong on any image
/// with strong colour — and it is the conversion someone writes when they are not thinking about it.
#[test]
fn greyscale_conversion_uses_perceptual_luma() {
    use redrob_core::{ColorMode, DitherMode};

    let mut editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    // Pure green: luma 0.7152 -> 182. The channel mean would be 85.
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(0, 255, 0, 255),
        })
        .unwrap();
    editor
        .execute(Command::ConvertColorMode {
            mode: ColorMode::Grayscale,
            palette: None,
            dither: DitherMode::None,
        })
        .unwrap();
    let got = pixel(&editor, layer, 0, 0);
    assert_eq!(got.r, got.g, "a grey pixel has equal channels");
    assert_eq!(got.g, got.b);
    assert!(
        (180..=184).contains(&got.r),
        "pure green should be near 182 by luma, not 85 by mean; got {}",
        got.r
    );
}

/// Floyd–Steinberg dithering spreads the snapping error, so a gradient keeps its shape.
///
/// With a two-colour palette and no dithering, a left-to-right ramp becomes one hard edge: every
/// pixel below the midpoint is black and every pixel above it is white. With error diffusion the
/// black and white pixels interleave, so the count of switches between them is much higher. That
/// count is the measurement — comparing individual pixels would be testing the matrix rather than
/// the behaviour.
#[test]
fn error_diffusion_turns_a_ramp_into_texture_rather_than_one_hard_edge() {
    use redrob_core::{ColorMode, DitherMode, PaletteChoice};

    let switches = |dither: DitherMode| {
        let mut editor = Editor::new(Document::new(32, 4).unwrap()).unwrap();
        let layer = editor.document().active_layer_id();
        // A horizontal ramp, painted a column at a time.
        for x in 0..32u32 {
            let value = (x * 255 / 31) as u8;
            editor
                .execute(Command::SelectRectangle {
                    rect: Rect::new(x as i32, 0, 1, 4),
                    mode: SelectionMode::Replace,
                })
                .unwrap();
            editor
                .execute(Command::Fill {
                    color: Pixel::rgba(value, value, value, 255),
                })
                .unwrap();
        }
        editor.execute(Command::ClearSelection).unwrap();
        editor
            .execute(Command::ConvertColorMode {
                mode: ColorMode::Indexed,
                palette: Some(PaletteChoice::Mono),
                dither,
            })
            .unwrap();
        // Count left-to-right changes across every row.
        let mut count = 0;
        for y in 0..4 {
            for x in 1..32 {
                if pixel(&editor, layer, x, y).r != pixel(&editor, layer, x - 1, y).r {
                    count += 1;
                }
            }
        }
        count
    };

    let plain = switches(DitherMode::None);
    let diffused = switches(DitherMode::FloydSteinberg);
    assert_eq!(
        plain, 4,
        "with no dithering a ramp is one hard edge per row, got {plain}"
    );
    assert!(
        diffused > plain * 3,
        "error diffusion must break the edge into texture: {diffused} switches vs {plain}"
    );
}

/// A colour-mode conversion is refused on a deep document rather than silently narrowing it.
///
/// Indexed and 16-bit is not a combination that means anything — a palette is at most 256 colours,
/// so the extra width can only describe entries not in it. Converting the precision as a side
/// effect of a colour-mode change nobody asked about is the alternative, and it is worse.
#[test]
fn converting_colour_mode_on_a_deep_document_is_refused_by_name() {
    use redrob_core::{ColorMode, DitherMode, PaletteChoice, precision::Precision};

    let mut editor = Editor::new(Document::new(2, 2).unwrap()).unwrap();
    editor
        .execute(Command::SetDocumentPrecision {
            precision: Precision::U16,
        })
        .unwrap();
    let error = editor
        .execute(Command::ConvertColorMode {
            mode: ColorMode::Indexed,
            palette: Some(PaletteChoice::Mono),
            dither: DitherMode::None,
        })
        .expect_err("indexed at 16-bit must be refused");
    assert!(
        matches!(error, CoreError::UnsupportedColorModeConversion),
        "got {error:?}"
    );
    assert_eq!(
        editor.document().precision(),
        Precision::U16,
        "and the refusal did not change the precision"
    );
}

/// J.3-b. A stroke in indexed mode cannot leave a colour that is not in the palette.
///
/// This is the gap J.3 opened and recorded rather than hid: the mode is a declared constraint and
/// the pixels are RGBA, so there is no storage format doing the snap the way upstream's indexed
/// buffer does. Both of the editor's write paths are exercised, because the brush fast path bypasses
/// the generic command path entirely and a snap wired into only one of them looks correct until
/// someone paints.
#[test]
fn an_edit_in_indexed_mode_cannot_leave_an_off_palette_colour() {
    use redrob_core::{ColorMode, DitherMode, PaletteChoice};

    let mut editor = Editor::new(Document::new(8, 8).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(0, 0, 0, 255),
        })
        .unwrap();
    // A two-colour palette: black and white, nothing else is legal.
    editor
        .execute(Command::ConvertColorMode {
            mode: ColorMode::Indexed,
            palette: Some(PaletteChoice::Mono),
            dither: DitherMode::None,
        })
        .unwrap();

    // Path 1: a generic command. Mid-grey is in neither entry.
    let changes = editor
        .execute(Command::Fill {
            color: Pixel::rgba(130, 130, 130, 255),
        })
        .unwrap();
    assert!(
        changes.palette_snapped,
        "the fill wrote a colour the palette does not have, and must say so"
    );
    let got = pixel(&editor, layer, 0, 0);
    assert_eq!(
        (got.r, got.g, got.b),
        (255, 255, 255),
        "130 is nearer white than black"
    );

    // Path 2: the brush fast path, which does not go through the command bus.
    let changes = editor
        .execute(Command::BrushStroke {
            points: vec![
                BrushPoint::new(2.0, 2.0, 1.0),
                BrushPoint::new(5.0, 5.0, 1.0),
            ],
            color: Pixel::rgba(200, 30, 30, 255),
            size: 4.0,
            opacity: 1.0,
            settings: BrushSettings::default(),
            tip: None,
            pipe: Vec::new(),
        })
        .unwrap();
    assert!(
        changes.palette_snapped,
        "the brush fast path must snap too -- it bypasses the command bus"
    );
    // Every pixel in the document is black or white. Nothing else exists in this mode.
    for y in 0..8 {
        for x in 0..8 {
            let got = pixel(&editor, layer, x, y);
            assert!(
                (got.r, got.g, got.b) == (0, 0, 0) || (got.r, got.g, got.b) == (255, 255, 255),
                "pixel ({x},{y}) is {got:?}, which is not in a black-and-white palette"
            );
        }
    }
}

/// Redo after an indexed edit replays the SNAPPED pixels, not the ones the command asked for.
///
/// The snap happens before history records the change for exactly this reason. Recording first and
/// snapping after would store the off-palette pixels as the redo side, so undo-then-redo would put
/// a colour back that the mode forbids — and it would only ever be noticed by someone pressing redo.
#[test]
fn redo_of_an_indexed_edit_replays_the_snapped_colour() {
    use redrob_core::{ColorMode, DitherMode, PaletteChoice};

    let mut editor = Editor::new(Document::new(4, 4).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(0, 0, 0, 255),
        })
        .unwrap();
    editor
        .execute(Command::ConvertColorMode {
            mode: ColorMode::Indexed,
            palette: Some(PaletteChoice::Mono),
            dither: DitherMode::None,
        })
        .unwrap();
    editor
        .execute(Command::BrushStroke {
            points: vec![
                BrushPoint::new(1.0, 1.0, 1.0),
                BrushPoint::new(2.0, 2.0, 1.0),
            ],
            color: Pixel::rgba(200, 200, 200, 255),
            size: 3.0,
            opacity: 1.0,
            settings: BrushSettings::default(),
            tip: None,
            pipe: Vec::new(),
        })
        .unwrap();
    let after_stroke: Vec<Pixel> = (0..4)
        .flat_map(|y| (0..4).map(move |x| (x, y)))
        .map(|(x, y)| pixel(&editor, layer, x, y))
        .collect();

    editor.undo().unwrap();
    editor.redo().unwrap();

    let after_redo: Vec<Pixel> = (0..4)
        .flat_map(|y| (0..4).map(move |x| (x, y)))
        .map(|(x, y)| pixel(&editor, layer, x, y))
        .collect();
    assert_eq!(
        after_redo, after_stroke,
        "redo must replay the snapped pixels, not the colour the command asked for"
    );
    for got in after_redo {
        assert!(
            (got.r, got.g, got.b) == (0, 0, 0) || (got.r, got.g, got.b) == (255, 255, 255),
            "redo reintroduced {got:?}, which is not in the palette"
        );
    }
}

/// A fully transparent pixel is left alone by the snap.
///
/// It has no colour to constrain. Writing a palette colour under zero alpha is invisible now and
/// wrong the moment anything raises that alpha — and it would make the snap report a change on an
/// edit that altered nothing anyone can see.
///
/// The palette here deliberately contains no black: a transparent pixel's stored RGB is 0,0,0, so a
/// palette with black in it would snap it to the colour it already has and the test could not fail.
#[test]
fn the_palette_snap_leaves_transparent_pixels_alone() {
    use redrob_core::{ColorMode, DitherMode, PaletteChoice};

    let mut editor = Editor::new(Document::new(2, 2).unwrap()).unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(200, 30, 30, 255),
        })
        .unwrap();
    editor
        .execute(Command::ConvertColorMode {
            mode: ColorMode::Indexed,
            palette: Some(PaletteChoice::Custom {
                colors: vec![Pixel::rgba(200, 30, 30, 255)],
            }),
            dither: DitherMode::None,
        })
        .unwrap();
    // A new layer is transparent everywhere.
    editor.execute(Command::add_layer("empty", 1)).unwrap();
    let empty = editor.document().active_layer_id();

    // A whole-canvas command on the transparent layer, so the snap visits every pixel of it.
    let changes = editor
        .execute(Command::Fill {
            color: Pixel::rgba(90, 90, 90, 0),
        })
        .unwrap();
    assert!(
        !changes.palette_snapped,
        "a transparent pixel has no colour to snap, so nothing should be reported"
    );
    for y in 0..2 {
        for x in 0..2 {
            let got = pixel(&editor, empty, x, y);
            assert_eq!(
                (got.r, got.g, got.b, got.a),
                (0, 0, 0, 0),
                "pixel ({x},{y}) was snapped to a palette colour under zero alpha"
            );
        }
    }
}

/// J.3-c. An indexed document with a transparent region round-trips through PNG with that region
/// still transparent.
///
/// This is the defect J.3-b's testing turned up and filed rather than papered over: PNG colour type
/// 3 has no alpha channel, only a per-entry `tRNS`, and every palette this product generates is
/// opaque — so an indexed export turned every transparent pixel into a solid colour.
///
/// Re-derived from upstream's PNG export (`plug-ins/common/file-png.c`): find an index no OPAQUE
/// pixel uses, or append one, then swap it to index 0 so `tRNS` is a single byte.
#[test]
fn an_indexed_export_keeps_a_transparent_region_transparent() {
    use redrob_core::{ColorMode, DitherMode, PaletteChoice};

    // Left half red, right half left transparent.
    let mut editor = Editor::new(Document::new(4, 2).unwrap()).unwrap();
    editor
        .execute(Command::SelectRectangle {
            rect: Rect::new(0, 0, 2, 2),
            mode: SelectionMode::Replace,
        })
        .unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(200, 30, 30, 255),
        })
        .unwrap();
    editor.execute(Command::ClearSelection).unwrap();
    editor
        .execute(Command::ConvertColorMode {
            mode: ColorMode::Indexed,
            palette: Some(PaletteChoice::Custom {
                colors: vec![Pixel::rgba(200, 30, 30, 255)],
            }),
            dither: DitherMode::None,
        })
        .unwrap();

    let png = export_document(
        editor.document(),
        FileFormat::Png,
        &ExportOptions::default(),
    )
    .unwrap();
    let bytes = png.bytes();
    assert_eq!(bytes[25], 3, "still an indexed PNG");
    assert!(
        bytes.windows(4).any(|window| window == b"tRNS"),
        "an indexed PNG carrying transparency must write tRNS"
    );

    let reread = import_document(bytes, &ImportOptions::default()).unwrap();
    let decoded = reread.document().layers()[0].pixels();
    // The right half must still be transparent, and the left half still red.
    for y in 0..2u32 {
        for x in 0..4u32 {
            let base = (y as usize * 4 + x as usize) * 4;
            let alpha = decoded[base + 3];
            if x < 2 {
                assert_eq!(alpha, 255, "pixel ({x},{y}) should still be opaque");
                assert_eq!(
                    (decoded[base], decoded[base + 1], decoded[base + 2]),
                    (200, 30, 30),
                    "pixel ({x},{y}) lost its colour"
                );
            } else {
                assert_eq!(
                    alpha, 0,
                    "pixel ({x},{y}) came back opaque -- the transparency was lost"
                );
            }
        }
    }
}

/// The transparent index REUSES an entry no opaque pixel points at, rather than growing the palette.
///
/// This is the case that matters in practice, because quantizing has already assigned the
/// transparent pixels somewhere. Growing the palette when a free entry exists would waste a slot of
/// the 256 — and in a full palette it is the difference between keeping transparency and not.
#[test]
fn the_transparent_index_reuses_an_entry_no_opaque_pixel_uses() {
    use redrob_core::reserve_transparent_index;

    let palette = vec![
        Pixel::rgba(10, 10, 10, 255),
        Pixel::rgba(20, 20, 20, 255),
        Pixel::rgba(30, 30, 30, 255),
    ];
    // Entry 1 is pointed at only by a transparent pixel, so it is free.
    let indices = [0u8, 1, 2, 1];
    let alphas = [255u8, 0, 255, 0];
    let (reserved, transparent) =
        reserve_transparent_index(&palette, &indices, &alphas).expect("an entry is free");
    assert_eq!(transparent, 1, "entry 1 is the one no opaque pixel uses");
    assert_eq!(
        reserved.len(),
        3,
        "the palette must not grow when an entry is already free"
    );
    assert_eq!(reserved[0].a, 0, "entry 0 is now the transparent one");
    assert_eq!(
        (reserved[1].r, reserved[1].g, reserved[1].b),
        (10, 10, 10),
        "the old entry 0 moved to where the transparent one was"
    );
}

/// A full palette with every entry visible cannot express transparency, and says so.
///
/// Dropping one of the 256 colours to make room would be worse than dropping the alpha: the colour
/// loss is visible everywhere that colour appears, where the alpha loss is confined to the pixels
/// that were transparent. Reporting it is the part that must not be skipped.
#[test]
fn a_full_palette_reports_that_transparency_could_not_be_kept() {
    use redrob_core::reserve_transparent_index;

    let palette: Vec<Pixel> = (0..256u32)
        .map(|index| Pixel::rgba(index as u8, 0, 0, 255))
        .collect();
    // Every entry is used by an opaque pixel, and one pixel is transparent.
    let mut indices: Vec<u8> = (0..256u32).map(|index| index as u8).collect();
    let mut alphas = vec![255u8; 256];
    indices.push(7);
    alphas.push(0);

    assert!(
        reserve_transparent_index(&palette, &indices, &alphas).is_none(),
        "a full palette with every entry visible has nowhere to put transparency"
    );
}

/// J.4. A path is stored geometry that draws NOTHING by itself, which is what makes it different
/// from the vector layer this product already had.
///
/// The test asserts the distinction directly: adding a path leaves the rendered canvas
/// byte-identical and adds no layer. Had paths been modelled as a vector layer with no fill — the
/// obvious shortcut — this would fail on the layer count, and would later fail on the pixels the
/// moment anyone gave the path a stroke to see what they were editing.
#[test]
fn a_stored_path_adds_no_layer_and_changes_no_pixel() {
    let mut editor = Editor::new(Document::new(8, 8).unwrap()).unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(40, 60, 80, 255),
        })
        .unwrap();
    let before_layers = editor.document().layers().len();
    let before_pixels = editor.document().layers()[0].pixels().to_vec();

    editor
        .execute(Command::AddPath {
            id: redrob_core::PathId::new_v4(),
            name: "Outline".into(),
            commands: vec![
                PathCommand::MoveTo { x: 1.0, y: 1.0 },
                PathCommand::LineTo { x: 6.0, y: 1.0 },
                PathCommand::LineTo { x: 6.0, y: 6.0 },
                PathCommand::Close,
            ],
        })
        .unwrap();

    assert_eq!(editor.document().paths().len(), 1, "the path is stored");
    assert_eq!(
        editor.document().layers().len(),
        before_layers,
        "a path must not occupy the layer stack"
    );
    assert_eq!(
        editor.document().layers()[0].pixels(),
        &before_pixels[..],
        "a path draws nothing by itself"
    );
}

/// Selection → path → selection returns the region it started from.
///
/// The round trip is the only honest test of the trace: a path that looks right but selects a
/// different region than it came from is worse than no conversion, because the error is invisible
/// until someone acts on the selection.
///
/// A rectangular selection is used deliberately. Upstream fits Bézier curves; this traces straight
/// segments, so a circle would come back as a polygon and the two would differ by design. For a
/// rectangle the results are identical, which is why this is the shape the acceptance uses — and
/// the curve-fitting difference is recorded in the backlog rather than hidden behind a loose
/// tolerance here.
#[test]
fn selection_to_path_and_back_returns_the_same_region() {
    let mut editor = Editor::new(Document::new(12, 10).unwrap()).unwrap();
    editor
        .execute(Command::SelectRectangle {
            rect: Rect::new(2, 3, 5, 4),
            mode: SelectionMode::Replace,
        })
        .unwrap();
    let before: Vec<u8> = (0..10)
        .flat_map(|y| (0..12).map(move |x| (x, y)))
        .map(|(x, y)| editor.document().selection().coverage(x, y))
        .collect();

    editor
        .execute(Command::PathFromSelection {
            name: "From selection".into(),
            fit: false,
        })
        .unwrap();
    let id = editor.document().paths()[0].id;

    // A rectangle is four sides: five anchors with the close, not one per boundary pixel.
    let anchors = editor.document().paths()[0]
        .commands
        .iter()
        .filter(|command| !matches!(command, PathCommand::Close))
        .count();
    assert_eq!(
        anchors, 4,
        "a traced rectangle must collapse its straight runs, got {anchors} anchors"
    );

    editor.execute(Command::ClearSelection).unwrap();
    editor
        .execute(Command::SelectionFromPath {
            id,
            mode: SelectionMode::Replace,
        })
        .unwrap();

    let after: Vec<u8> = (0..10)
        .flat_map(|y| (0..12).map(move |x| (x, y)))
        .map(|(x, y)| editor.document().selection().coverage(x, y))
        .collect();
    assert_eq!(
        after, before,
        "the path must select exactly the region it was traced from"
    );
}

/// The same selection traces to the SAME path, every time.
///
/// Not a theoretical worry: the first version collected boundary edges in a `HashMap`, so the walk
/// started wherever the first key landed and the whole command list changed between runs of the
/// same binary. The anchor-count assertion in the test above passed once and failed on the next
/// run with identical input, which is how it was found. A path that is not byte-stable cannot be
/// compared, cannot be tested, and makes a saved document differ from itself.
#[test]
fn tracing_the_same_selection_twice_gives_byte_identical_paths() {
    let trace = || {
        let mut editor = Editor::new(Document::new(16, 12).unwrap()).unwrap();
        editor
            .execute(Command::SelectEllipse {
                rect: Rect::new(2, 2, 11, 8),
                mode: SelectionMode::Replace,
            })
            .unwrap();
        editor
            .execute(Command::PathFromSelection {
                name: "T".into(),
                fit: true,
            })
            .unwrap();
        editor.document().paths()[0].commands.clone()
    };
    let first = trace();
    for attempt in 0..8 {
        assert_eq!(
            trace(),
            first,
            "attempt {attempt} traced a different path from the same selection"
        );
    }
}

/// Stroking a path paints along it with the brush, not with a hairline of its own.
///
/// Reusing the brush is the point: "stroke this path" means the path drawn with the tool the user
/// set up, dynamics and all. A separate line renderer would ignore every brush setting and produce
/// something nobody asked for.
#[test]
fn stroking_a_path_paints_along_it_with_the_brush() {
    let mut editor = Editor::new(Document::new(16, 8).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    let id = redrob_core::PathId::new_v4();
    editor
        .execute(Command::AddPath {
            id,
            name: "Line".into(),
            commands: vec![
                PathCommand::MoveTo { x: 2.0, y: 4.0 },
                PathCommand::LineTo { x: 13.0, y: 4.0 },
            ],
        })
        .unwrap();
    // Nothing is painted until the stroke is asked for.
    assert_eq!(
        editor.document().layers()[0].pixels().iter().copied().max(),
        Some(0),
        "the path alone paints nothing"
    );

    editor
        .execute(Command::StrokePath {
            id,
            color: Pixel::rgba(255, 0, 0, 255),
            size: 3.0,
            opacity: 1.0,
            settings: BrushSettings::default(),
        })
        .unwrap();

    // Paint lands along the path's own line and not off it.
    let on_line = pixel(&editor, layer, 7, 4);
    assert!(
        on_line.a > 128 && on_line.r > 128,
        "the middle of the stroked line should be painted, got {on_line:?}"
    );
    assert_eq!(
        pixel(&editor, layer, 7, 0).a,
        0,
        "and nothing should land four rows away from the path"
    );
}

/// Stored paths survive an SVG round trip, and do NOT come back as vector layers.
///
/// They are written into `<defs>` with our own marker. Writing them as ordinary `<path>` elements
/// was the alternative, and it is wrong twice over: every other SVG reader would DRAW them — with a
/// default black fill, the opposite of geometry that draws nothing — and importing our own file
/// back would turn each one into a layer.
#[test]
fn stored_paths_survive_an_svg_round_trip_without_becoming_layers() {
    let mut editor = Editor::new(Document::new(10, 10).unwrap()).unwrap();
    editor
        .execute(Command::AddPath {
            id: redrob_core::PathId::new_v4(),
            name: "Kept".into(),
            commands: vec![
                PathCommand::MoveTo { x: 1.0, y: 1.0 },
                PathCommand::LineTo { x: 8.0, y: 1.0 },
                PathCommand::LineTo { x: 8.0, y: 8.0 },
                PathCommand::Close,
            ],
        })
        .unwrap();
    let layers_before = editor.document().layers().len();

    // AllowLoss because the document carries a raster layer the SVG cannot hold; the paths are
    // what this test is about.
    let svg = export_document(
        editor.document(),
        FileFormat::Svg,
        &ExportOptions::default().with_loss_policy(LossPolicy::AllowLoss),
    )
    .unwrap();
    let bytes = svg.bytes();
    let text = std::str::from_utf8(bytes).unwrap();
    assert!(
        text.contains("stored-path"),
        "the path must be marked so it is not re-imported as a shape"
    );
    assert!(
        text.contains("<defs>"),
        "a non-rendering path belongs in defs"
    );

    let reread = import_document(
        bytes,
        &ImportOptions::default().with_loss_policy(LossPolicy::AllowLoss),
    )
    .unwrap();
    assert_eq!(
        reread.document().paths().len(),
        1,
        "the stored path must come back as a path"
    );
    assert_eq!(
        reread.document().paths()[0].name,
        "Kept",
        "and keep its name"
    );
    assert_eq!(
        reread.document().layers().len(),
        layers_before,
        "it must NOT come back as a vector layer"
    );
}

/// Tracing an inactive selection is refused rather than returning the whole canvas.
///
/// An inactive selection reports full coverage by design — no selection means every pixel is
/// available — so a trace of it is a path around the entire canvas. That is never what someone
/// pressing "selection to path" means, and it is the kind of result that looks like it worked.
#[test]
fn path_from_an_inactive_selection_is_refused() {
    let mut editor = Editor::new(Document::new(6, 6).unwrap()).unwrap();
    let error = editor
        .execute(Command::PathFromSelection {
            name: "Nothing".into(),
            fit: true,
        })
        .expect_err("there is no selection to trace");
    assert!(matches!(error, CoreError::NoSelection), "got {error:?}");
    assert!(editor.document().paths().is_empty());
}

/// J.4-b. An elliptical selection fits to a path with few anchors that still selects the same
/// region.
///
/// Both halves matter and the test would be dishonest with either one alone. Few anchors with a
/// drifted boundary is a path that looks editable and selects the wrong pixels; an exact boundary
/// with 200 anchors is J.4's straight trace, which is what this item exists to improve on.
///
/// The coverage tolerance is one percent of the canvas rather than exact: a fitted curve is allowed
/// to disagree with a pixel staircase, which is the entire point of fitting it. The error threshold
/// inside the fitter is 0.4 pixels — under half a pixel, so the fit cannot move the boundary into a
/// neighbouring pixel — and this assertion is what checks that claim end to end.
#[test]
fn an_elliptical_selection_fits_to_few_anchors_and_still_selects_itself() {
    let mut editor = Editor::new(Document::new(64, 64).unwrap()).unwrap();
    editor
        .execute(Command::SelectEllipse {
            rect: Rect::new(6, 6, 50, 50),
            mode: SelectionMode::Replace,
        })
        .unwrap();
    let before: Vec<u8> = (0..64)
        .flat_map(|y| (0..64).map(move |x| (x, y)))
        .map(|(x, y)| editor.document().selection().coverage(x, y))
        .collect();
    let selected_before = before.iter().filter(|value| **value >= 128).count();

    // The straight trace, for the comparison this item is measured against.
    editor
        .execute(Command::PathFromSelection {
            name: "Straight".into(),
            fit: false,
        })
        .unwrap();
    let straight_anchors = editor.document().paths()[0]
        .commands
        .iter()
        .filter(|command| !matches!(command, PathCommand::Close))
        .count();

    editor
        .execute(Command::PathFromSelection {
            name: "Fitted".into(),
            fit: true,
        })
        .unwrap();
    let fitted = &editor.document().paths()[1];
    let fitted_anchors = fitted
        .commands
        .iter()
        .filter(|command| !matches!(command, PathCommand::Close))
        .count();
    let id = fitted.id;

    assert!(
        straight_anchors > 100,
        "the straight trace of a 50px circle should be heavy; got {straight_anchors}"
    );
    assert!(
        fitted_anchors < 20,
        "the fitted path must be editable: got {fitted_anchors} anchors, straight was {straight_anchors}"
    );
    assert!(
        fitted
            .commands
            .iter()
            .any(|command| matches!(command, PathCommand::CubicTo { .. })),
        "a circle must fit with curves, not straight segments"
    );

    // And it still selects the region it came from.
    editor.execute(Command::ClearSelection).unwrap();
    editor
        .execute(Command::SelectionFromPath {
            id,
            mode: SelectionMode::Replace,
        })
        .unwrap();
    let after: Vec<u8> = (0..64)
        .flat_map(|y| (0..64).map(move |x| (x, y)))
        .map(|(x, y)| editor.document().selection().coverage(x, y))
        .collect();

    // The criterion is CONFINEMENT, not an area percentage. Smoothing deliberately moves the
    // boundary off the pixel staircase -- by up to half a pixel, which is enough to flip a boundary
    // pixel either way -- so demanding a percentage just encodes a guess about how many flipped.
    // What has to be true is that every disagreement sits ON the original boundary: the fitted path
    // bounds the same region with an edge that may shift by a pixel, and nothing in the interior or
    // out in the background has changed.
    //
    // My first version asserted "under one percent of selected pixels differ" and it failed at
    // 1.01% -- a tolerance tuned to nothing, which would have been loosened to 2% and tested less
    // each time.
    let selected = |values: &[u8], x: i32, y: i32| -> bool {
        if x < 0 || y < 0 || x >= 64 || y >= 64 {
            return false;
        }
        values[y as usize * 64 + x as usize] >= 128
    };
    let on_original_boundary = |x: i32, y: i32| -> bool {
        let here = selected(&before, x, y);
        (-1..=1).any(|dy| {
            (-1..=1).any(|dx| (dx != 0 || dy != 0) && selected(&before, x + dx, y + dy) != here)
        })
    };
    let mut strays = Vec::new();
    for y in 0..64i32 {
        for x in 0..64i32 {
            if selected(&before, x, y) != selected(&after, x, y) && !on_original_boundary(x, y) {
                strays.push((x, y));
            }
        }
    }
    assert!(
        strays.is_empty(),
        "the fitted path changed {} pixels away from the original boundary: {:?}",
        strays.len(),
        &strays[..strays.len().min(8)]
    );
    // And it is still substantially the same selection -- a guard against a path that bounds
    // nothing, which would satisfy the confinement check trivially.
    let still_selected = after.iter().filter(|value| **value >= 128).count();
    assert!(
        still_selected * 10 >= selected_before * 9,
        "the fitted path selects {still_selected} where the original selected {selected_before}"
    );
}

/// A rectangle still fits as four straight sides.
///
/// This is the test of whether the fitter behaves rather than a special case inside it: a straight
/// run fits a line with no measurable error, and the corner detector must see the four right angles
/// as corners instead of smoothing through them. A fitter that rounds a rectangle's corners is the
/// classic failure of this algorithm, and it is why corners are found before anything is fitted.
#[test]
fn fitting_a_rectangle_keeps_its_corners_square() {
    let mut editor = Editor::new(Document::new(40, 30).unwrap()).unwrap();
    editor
        .execute(Command::SelectRectangle {
            rect: Rect::new(5, 5, 28, 18),
            mode: SelectionMode::Replace,
        })
        .unwrap();
    editor
        .execute(Command::PathFromSelection {
            name: "Square".into(),
            fit: true,
        })
        .unwrap();
    let commands = &editor.document().paths()[0].commands;
    // Distinct anchor POINTS, not command count: the fitted form repeats the first corner in its
    // MoveTo because its final segment is explicit, where the straight form lets `Close` draw that
    // side. `Close` draws a LINE, so a curved final side has to be emitted.
    let mut points: Vec<(u32, u32)> = commands
        .iter()
        .filter_map(|command| match command {
            PathCommand::MoveTo { x, y } | PathCommand::LineTo { x, y } => {
                Some((*x as u32, *y as u32))
            }
            PathCommand::CubicTo { x, y, .. } => Some((*x as u32, *y as u32)),
            PathCommand::Close => None,
        })
        .collect();
    points.sort_unstable();
    points.dedup();
    assert_eq!(
        points.len(),
        4,
        "a fitted rectangle is still four anchors, got {points:?}: {commands:?}"
    );

    // The corners must land exactly on the selection's own bounds. A rounded corner would pull them
    // inwards, and the anchor count alone would not notice.
    let corners: Vec<(f32, f32)> = commands
        .iter()
        .filter_map(|command| match command {
            PathCommand::MoveTo { x, y } | PathCommand::LineTo { x, y } => Some((*x, *y)),
            PathCommand::CubicTo { x, y, .. } => Some((*x, *y)),
            PathCommand::Close => None,
        })
        .collect();
    for expected in [(5.0, 5.0), (33.0, 5.0), (33.0, 23.0), (5.0, 23.0)] {
        assert!(
            corners.contains(&expected),
            "corner {expected:?} is missing from {corners:?}"
        );
    }
}

/// J.5. A colour-managed display changes what is SHOWN and never what is stored.
///
/// This is the invariant the whole feature rests on. Soft-proofing exists to show what an image
/// would look like somewhere else without changing it, and a display transform that reached the
/// document would destroy the thing it was meant to describe. Asserted on the document's own bytes
/// before and after, not inferred from the design.
#[test]
fn a_colour_managed_display_changes_the_view_and_not_the_document() {
    use redrob_core::{ColorManagementMode, DisplaySettings, RenderingIntent};

    let document = raster_document(2, 1, vec![230, 30, 40, 255, 40, 60, 220, 255]);
    let before = document.layers()[0].pixels().to_vec();
    let snapshot = RenderSnapshot::try_render_frame(&document, 0, FrameId::DEFAULT).unwrap();
    let plain = snapshot.rgba8().to_vec();

    let settings = DisplaySettings {
        mode: ColorManagementMode::Display,
        display_profile: Some(redrob_core::icc::IccProfile::parse(&wide_gamut_icc()).unwrap()),
        display_intent: RenderingIntent::RelativeColorimetric,
        ..DisplaySettings::default()
    };
    let shown = snapshot.display_rgba8(&settings).to_vec();

    assert_ne!(
        shown, plain,
        "a monitor profile must change what is displayed"
    );
    assert_eq!(
        document.layers()[0].pixels(),
        &before[..],
        "and must never touch the document"
    );
    assert_eq!(
        snapshot.rgba8().to_vec(),
        plain,
        "nor the document-space projection an export reads"
    );
    // Alpha is coverage, not colour: it has no profile and must pass through.
    assert_eq!(shown[3], 255);
    assert_eq!(shown[7], 255);
}

/// With management off, or a mode whose profile is missing, the buffer is returned untouched.
///
/// The missing-profile case is the one worth pinning: a UI is easily half-way through being set up,
/// and converting against a profile that is not there is worse than not converting — it shifts
/// every colour on a correctly calibrated screen and looks like a broken monitor.
#[test]
fn display_management_without_a_profile_is_a_no_op() {
    use redrob_core::{ColorManagementMode, DisplaySettings};

    let document = raster_document(1, 1, vec![200, 100, 50, 255]);
    let snapshot = RenderSnapshot::try_render_frame(&document, 0, FrameId::DEFAULT).unwrap();
    let plain = snapshot.rgba8().to_vec();

    for mode in [
        ColorManagementMode::Off,
        ColorManagementMode::Display,
        ColorManagementMode::SoftProof,
    ] {
        let settings = DisplaySettings {
            mode,
            ..DisplaySettings::default()
        };
        assert!(!settings.is_active(), "{mode:?} with no profile is inert");
        assert_eq!(
            snapshot.display_rgba8(&settings).to_vec(),
            plain,
            "{mode:?} with no profile must change nothing"
        );
    }
}

/// Soft-proofing round-trips through the simulated device, so a colour it cannot hold comes back
/// changed.
///
/// The round trip IS the preview: what returns is what the device could actually reproduce, and the
/// difference from what went in is the loss being shown. A colour well inside the device's gamut
/// must survive it — otherwise the proof would report loss everywhere and mean nothing.
#[test]
fn soft_proofing_shows_the_loss_a_narrow_device_would_cause() {
    use redrob_core::{ColorManagementMode, DisplaySettings, RenderingIntent};

    // A saturated red, and a neutral grey. The profile here is WIDER than sRGB, so proofing sRGB
    // content through it loses nothing — the direction is what the test checks, and the grey is the
    // control that proves the transform is not simply mangling everything.
    let document = raster_document(2, 1, vec![255, 0, 0, 255, 128, 128, 128, 255]);
    let snapshot = RenderSnapshot::try_render_frame(&document, 0, FrameId::DEFAULT).unwrap();
    let profile = redrob_core::icc::IccProfile::parse(&wide_gamut_icc()).unwrap();

    let settings = DisplaySettings {
        mode: ColorManagementMode::SoftProof,
        simulation_profile: Some(profile),
        simulation_intent: RenderingIntent::RelativeColorimetric,
        ..DisplaySettings::default()
    };
    let proofed = snapshot.display_rgba8(&settings).to_vec();

    // A neutral grey is inside any sane RGB device's gamut and must survive the round trip within
    // rounding. A tolerance of 2 is one more than the 8-bit step the round trip can cost.
    for channel in 0..3 {
        let difference = i32::from(proofed[4 + channel]) - 128;
        assert!(
            difference.abs() <= 2,
            "grey must survive the proof round trip, channel {channel} moved by {difference}"
        );
    }
    assert_eq!(proofed[3], 255, "alpha is untouched");
}

/// The gamut check paints colours the simulated device cannot reproduce in a flat warning colour.
///
/// A proof that silently clips tells the user nothing: the clipped colour just looks like a slightly
/// different colour. The point of the check is that it is impossible to mistake for the image.
#[test]
fn the_gamut_check_marks_what_the_device_cannot_reproduce() {
    use redrob_core::{ColorManagementMode, DisplaySettings, Pixel};

    // A narrow device cannot hold a saturated sRGB primary.
    let document = raster_document(1, 1, vec![255, 0, 255, 255]);
    let snapshot = RenderSnapshot::try_render_frame(&document, 0, FrameId::DEFAULT).unwrap();

    let marker = Pixel::rgba(0, 255, 0, 255);
    let settings = DisplaySettings {
        mode: ColorManagementMode::SoftProof,
        simulation_profile: Some(redrob_core::icc::IccProfile::parse(&narrow_gamut_icc()).unwrap()),
        simulation_gamut_check: true,
        out_of_gamut_color: marker,
        ..DisplaySettings::default()
    };
    let checked = snapshot.display_rgba8(&settings).to_vec();
    assert_eq!(
        (checked[0], checked[1], checked[2]),
        (marker.r, marker.g, marker.b),
        "a colour the device cannot hold must be marked, got {checked:?}"
    );

    // And with the check off, the same pixel is proofed rather than marked.
    let settings = DisplaySettings {
        simulation_gamut_check: false,
        ..settings
    };
    let proofed = snapshot.display_rgba8(&settings).to_vec();
    assert_ne!(
        (proofed[0], proofed[1], proofed[2]),
        (marker.r, marker.g, marker.b),
        "the marker colour must not appear when the check is off"
    );
}

/// A fully transparent pixel is left alone by the display transform.
///
/// It has no visible colour to convert, and its stored RGB is usually zero — which would come back
/// as the destination's black and then appear the moment anything raised that alpha.
#[test]
fn the_display_transform_leaves_transparent_pixels_alone() {
    use redrob_core::{ColorManagementMode, DisplaySettings};

    let document = raster_document(1, 1, vec![0, 0, 0, 0]);
    let snapshot = RenderSnapshot::try_render_frame(&document, 0, FrameId::DEFAULT).unwrap();
    let settings = DisplaySettings {
        mode: ColorManagementMode::Display,
        display_profile: Some(redrob_core::icc::IccProfile::parse(&wide_gamut_icc()).unwrap()),
        display_bpc: true,
        ..DisplaySettings::default()
    };
    assert_eq!(
        snapshot.display_rgba8(&settings).to_vec(),
        vec![0, 0, 0, 0],
        "a transparent pixel must stay exactly as it was"
    );
}

/// The absolute-colorimetric intent differs from relative by keeping the source white point.
///
/// That is the whole observable difference between the two for a matrix profile: relative maps the
/// source white onto the destination's white, absolute preserves it, so paper white shows as the
/// paper's own tint instead of as screen white. If white came out identical under both, the intent
/// would be a setting that does nothing.
#[test]
fn absolute_colorimetric_keeps_the_source_white_where_relative_maps_it() {
    use redrob_core::{ColorManagementMode, DisplaySettings, RenderingIntent};

    let document = raster_document(1, 1, vec![255, 255, 255, 255]);
    let snapshot = RenderSnapshot::try_render_frame(&document, 0, FrameId::DEFAULT).unwrap();
    let profile = redrob_core::icc::IccProfile::parse(&wide_gamut_icc()).unwrap();

    let white_under = |intent: RenderingIntent| {
        let settings = DisplaySettings {
            mode: ColorManagementMode::Display,
            display_profile: Some(profile.clone()),
            display_intent: intent,
            ..DisplaySettings::default()
        };
        snapshot.display_rgba8(&settings).to_vec()
    };

    let relative = white_under(RenderingIntent::RelativeColorimetric);
    let absolute = white_under(RenderingIntent::AbsoluteColorimetric);
    assert_ne!(
        relative[..3],
        absolute[..3],
        "the two intents must treat white differently: relative {relative:?} absolute {absolute:?}"
    );
}

/// Builds an ICC profile carrying a real `mft2` `B2A0` lookup table (J.5-b).
///
/// The table's CLUT scales each PCS channel by `scale`, which makes it unmistakably distinguishable
/// from the colorimetric matrix path — a table that merely approximated the matrix would leave the
/// test unable to tell whether it was read at all.
///
/// A 2-point grid is used deliberately: it is the smallest grid that still exercises trilinear
/// interpolation across the whole cube, so a broken interpolator cannot pass by rounding.
fn icc_with_b2a0_table(scale: f64) -> Vec<u8> {
    let mut lut = b"mft2".to_vec();
    lut.extend_from_slice(&[0, 0, 0, 0]); // reserved
    lut.push(3); // input channels
    lut.push(3); // output channels
    lut.push(2); // CLUT grid points
    lut.push(0); // pad
    // The pipeline matrix, identity. Only legal for an XYZ PCS, which this profile declares.
    for row in 0..3 {
        for column in 0..3 {
            let value: f64 = if row == column { 1.0 } else { 0.0 };
            lut.extend_from_slice(&((value * 65536.0) as i32).to_be_bytes());
        }
    }
    lut.extend_from_slice(&2u16.to_be_bytes()); // input table entries
    lut.extend_from_slice(&2u16.to_be_bytes()); // output table entries
    // Input tables: identity, two entries per channel.
    for _ in 0..3 {
        lut.extend_from_slice(&0u16.to_be_bytes());
        lut.extend_from_slice(&u16::MAX.to_be_bytes());
    }
    // CLUT: 2x2x2 cells, three outputs each. Each corner's output is its own coordinate scaled.
    for x in 0..2u32 {
        for y in 0..2u32 {
            for z in 0..2u32 {
                for coordinate in [x, y, z] {
                    let value = (coordinate as f64) * scale * 65535.0;
                    lut.extend_from_slice(&(value.round() as u16).to_be_bytes());
                }
            }
        }
    }
    // Output tables: identity.
    for _ in 0..3 {
        lut.extend_from_slice(&0u16.to_be_bytes());
        lut.extend_from_slice(&u16::MAX.to_be_bytes());
    }

    // Wrap it in a profile that ALSO carries colorants, so the colorimetric path stays available
    // and the test can compare the two rather than one against a failure.
    let mut base = wide_gamut_icc();
    let tag_count = u32::from_be_bytes([base[128], base[129], base[130], base[131]]) as usize;
    // Rebuild the tag table with one more entry; every existing offset shifts by 12 bytes.
    let old_table_start = 132;
    let old_body_start = old_table_start + tag_count * 12;
    let mut entries: Vec<([u8; 4], usize, usize)> = Vec::new();
    for index in 0..tag_count {
        let at = old_table_start + index * 12;
        let mut signature = [0u8; 4];
        signature.copy_from_slice(&base[at..at + 4]);
        let offset =
            u32::from_be_bytes([base[at + 4], base[at + 5], base[at + 6], base[at + 7]]) as usize;
        let size =
            u32::from_be_bytes([base[at + 8], base[at + 9], base[at + 10], base[at + 11]]) as usize;
        entries.push((signature, offset, size));
    }
    let old_body = base[old_body_start..].to_vec();

    let new_table_start = 132;
    let new_body_start = new_table_start + (tag_count + 1) * 12;
    let shift = new_body_start - old_body_start;
    let mut table = Vec::new();
    for (signature, offset, size) in &entries {
        table.extend_from_slice(signature);
        table.extend_from_slice(&((offset + shift) as u32).to_be_bytes());
        table.extend_from_slice(&(*size as u32).to_be_bytes());
    }
    let mut body = old_body;
    while !body.len().is_multiple_of(4) {
        body.push(0);
    }
    let lut_offset = new_body_start + body.len();
    table.extend_from_slice(b"B2A0");
    table.extend_from_slice(&(lut_offset as u32).to_be_bytes());
    table.extend_from_slice(&(lut.len() as u32).to_be_bytes());
    body.extend_from_slice(&lut);

    base.truncate(132);
    base[128..132].copy_from_slice(&((tag_count + 1) as u32).to_be_bytes());
    base.extend_from_slice(&table);
    base.extend_from_slice(&body);
    let total = base.len() as u32;
    base[0..4].copy_from_slice(&total.to_be_bytes());
    base
}

/// J.5-b. A profile carrying a `B2A0` table is USED for the perceptual intent, and gives a
/// measurably different result from relative colorimetric.
///
/// This is what J.5 could not do: with no table to read, perceptual and saturation were necessarily
/// the colorimetric transform in disguise, and the setting did nothing. The table IS the intent — it
/// is where the profile's author recorded what perceptual should mean — so the test asserts both
/// that it is read and that the profile reports honestly which intents it can honour.
#[test]
fn a_profile_with_a_b2a0_table_honours_the_perceptual_intent() {
    use redrob_core::{ColorManagementMode, DisplaySettings, RenderingIntent};

    let profile =
        redrob_core::icc::IccProfile::parse(&icc_with_b2a0_table(0.5)).expect("profile parses");
    assert!(
        profile.has_intent_table(0),
        "the perceptual table must be found"
    );
    assert!(
        !profile.has_intent_table(2),
        "and a saturation table that is not there must not be claimed"
    );

    let document = raster_document(1, 1, vec![255, 255, 255, 255]);
    let snapshot = RenderSnapshot::try_render_frame(&document, 0, FrameId::DEFAULT).unwrap();
    let shown = |intent: RenderingIntent| {
        let settings = DisplaySettings {
            mode: ColorManagementMode::Display,
            display_profile: Some(profile.clone()),
            display_intent: intent,
            ..DisplaySettings::default()
        };
        snapshot.display_rgba8(&settings).to_vec()
    };

    let perceptual = shown(RenderingIntent::Perceptual);
    let colorimetric = shown(RenderingIntent::RelativeColorimetric);
    assert_ne!(
        perceptual[..3],
        colorimetric[..3],
        "the perceptual table must change the result: perceptual {perceptual:?} colorimetric {colorimetric:?}"
    );
    // The table halves each PCS channel, so white comes out far darker than the colorimetric white.
    assert!(
        perceptual[1] < colorimetric[1] / 2,
        "the table's halving must show: perceptual green {} vs colorimetric {}",
        perceptual[1],
        colorimetric[1]
    );
    // Saturation has no table, so it falls back and matches the colorimetric path's shape rather
    // than silently reading the perceptual one.
    let saturation = shown(RenderingIntent::Saturation);
    assert_ne!(
        saturation[..3],
        perceptual[..3],
        "saturation must not borrow the perceptual table"
    );
}

/// A table-only profile — no RGB colorants at all — now parses instead of being refused.
///
/// This is the case J.5 had to turn away: without table support a profile with no colorants had no
/// transform at all. The identity matrix kept for it is never consulted, because every intent on
/// such a profile resolves to a table.
#[test]
fn a_profile_with_only_a_table_parses_and_transforms() {
    let mut bytes = icc_with_b2a0_table(0.5);
    // Rename the red colorant so the profile has a table and no matrix.
    let position = bytes
        .windows(4)
        .position(|window| window == b"rXYZ")
        .unwrap();
    bytes[position..position + 4].copy_from_slice(b"rXYz");

    let profile = redrob_core::icc::IccProfile::parse(&bytes)
        .expect("a table-only profile must parse, not be refused");
    assert!(profile.has_intent_table(0));
    let device = profile.from_srgb_unit_with_intent([1.0, 1.0, 1.0], 0, true);
    assert!(
        device.iter().all(|value| *value > 0.0 && *value < 0.5),
        "the table must drive the transform, got {device:?}"
    );
}

/// A lookup table with more than three channels is refused by name rather than read as three.
///
/// A CMYK pipeline is a different colour model with its own black generation; reading its four
/// input channels as three would produce a plausible-looking colour that is wrong everywhere, which
/// is worse than declining.
#[test]
fn a_four_channel_lookup_table_is_refused_rather_than_misread() {
    let mut bytes = icc_with_b2a0_table(0.5);
    let position = bytes
        .windows(4)
        .position(|window| window == b"mft2")
        .unwrap();
    // Byte 8 of the tag body is the input channel count.
    bytes[position + 8] = 4;
    let profile = redrob_core::icc::IccProfile::parse(&bytes)
        .expect("the profile still parses on its matrix");
    assert!(
        !profile.has_intent_table(0),
        "a four-channel table must not be used as a three-channel one"
    );
}

/// Both modes survive an SVG round trip.
///
/// The export writes our own names because CSS `mix-blend-mode` has no equivalent for alpha
/// arithmetic. Without the matching reader arms a document using either mode could be SAVED and not
/// reopened, which is worse than not supporting it at all.
#[test]
fn merge_and_split_round_trip_through_svg() {
    use redrob_core::BlendMode;

    for mode in [BlendMode::Merge, BlendMode::Split] {
        let mut editor = Editor::new(Document::new(4, 4).unwrap()).unwrap();
        editor.execute(Command::add_layer("Upper", 1)).unwrap();
        let upper = editor.document().layers()[1].id();
        editor
            .execute(Command::SetLayerBlendMode { id: upper, mode })
            .unwrap();

        let svg = export_document(
            editor.document(),
            FileFormat::Svg,
            &ExportOptions::default().with_loss_policy(LossPolicy::AllowLoss),
        )
        .unwrap();
        let reread = import_document(
            svg.bytes(),
            &ImportOptions::default().with_loss_policy(LossPolicy::AllowLoss),
        )
        .unwrap();
        let modes: Vec<BlendMode> = reread
            .document()
            .layers()
            .iter()
            .map(|layer| layer.blend_mode())
            .collect();
        assert!(
            modes.contains(&mode),
            "{mode:?} was lost in the round trip: got {modes:?}"
        );
    }
}
