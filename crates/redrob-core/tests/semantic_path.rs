// SPDX-License-Identifier: GPL-3.0-or-later

use redrob_core::{
    BlendMode, Command, CoreError, Document, EMBEDDED_FONT_ID, Editor, FillRule, FrameId, LayerId,
    MAX_FONT_FAMILY_BYTES, MAX_FONT_ID_BYTES, MAX_PATH_COMMANDS_PER_PATH,
    MAX_SEMANTIC_SAMPLE_EDGE_VISITS, MAX_STORED_RASTER_BYTES, MAX_TEXT_CONTENT_BYTES,
    MAX_VECTOR_PATHS, Navigation, NodeContent, NodeKind, PathCommand, Pixel, StrokeStyle,
    TextContent, VectorContent, VectorPath, load_project, save_project,
};

fn text(content: &str, x: f32, y: f32, size: f32, color: Pixel) -> TextContent {
    TextContent {
        text: content.into(),
        font_family: "metadata-only family".into(),
        font_size: size,
        color,
        origin_x: x,
        origin_y: y,
        font_id: EMBEDDED_FONT_ID.into(),
    }
}

fn rectangle(x: f32, y: f32, width: f32, height: f32, fill: Pixel) -> VectorContent {
    VectorContent {
        paths: vec![VectorPath {
            commands: vec![
                PathCommand::MoveTo { x, y },
                PathCommand::LineTo { x: x + width, y },
                PathCommand::LineTo {
                    x: x + width,
                    y: y + height,
                },
                PathCommand::LineTo { x, y: y + height },
                PathCommand::Close,
            ],
            fill: Some(fill),
            stroke: None,
            fill_rule: FillRule::NonZero,
        }],
    }
}

fn fnv64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3)
    })
}

#[test]
fn exact_text_and_vector_goldens_are_stable() {
    let mut text_editor = Editor::new(Document::new(8, 8).unwrap()).unwrap();
    text_editor
        .execute(Command::AddTextNode {
            id: LayerId::new(),
            name: "A".into(),
            parent: None,
            sibling_index: 1,
            text: text("A", 0.0, 0.0, 8.0, Pixel::rgba(255, 0, 0, 255)),
        })
        .unwrap();
    let text_pixels = text_editor.render_snapshot().unwrap();
    assert_eq!(text_pixels.pixels().len(), 8 * 8 * 4);
    assert_eq!(fnv64(text_pixels.pixels()), 13_321_014_043_445_772_205);

    let mut vector_editor = Editor::new(Document::new(4, 4).unwrap()).unwrap();
    vector_editor
        .execute(Command::AddVectorNode {
            id: LayerId::new(),
            name: "Rectangle".into(),
            parent: None,
            sibling_index: 1,
            vector: rectangle(0.0, 0.0, 2.0, 2.0, Pixel::rgba(0, 255, 0, 255)),
        })
        .unwrap();
    assert_eq!(
        vector_editor.render_snapshot().unwrap().pixels(),
        &[
            0, 255, 0, 255, 0, 255, 0, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255, 0, 255, 0, 255, 0, 255,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        ]
    );
}

#[test]
fn rasterize_preserves_identity_properties_and_exact_visual_then_undo_redo() {
    let mut editor = Editor::new(Document::new(8, 8).unwrap()).unwrap();
    let frame = FrameId::new(7);
    editor
        .execute(Command::AddFrame {
            id: frame,
            index: 1,
        })
        .unwrap();
    editor
        .navigate(Navigation::SetCurrentFrame { id: frame })
        .unwrap();
    let id = LayerId::new();
    editor
        .execute(Command::AddVectorNode {
            id,
            name: "Semantic".into(),
            parent: None,
            sibling_index: 1,
            vector: rectangle(1.0, 1.0, 4.0, 3.0, Pixel::rgba(30, 80, 200, 220)),
        })
        .unwrap();
    editor
        .execute(Command::SetLayerOpacity { id, opacity: 0.5 })
        .unwrap();
    editor
        .execute(Command::SetLayerBlendMode {
            id,
            mode: BlendMode::Multiply,
        })
        .unwrap();
    editor.execute(Command::SelectAll).unwrap();
    let selection_before = editor.selection_mask_snapshot();
    let visual_before = editor.render_snapshot().unwrap().pixels().to_vec();
    let before = editor.document().layer(id).unwrap().clone();

    editor
        .execute(Command::RasterizeSemanticNode { id })
        .unwrap();
    let after = editor.document().layer(id).unwrap();
    assert_eq!(after.id(), before.id());
    assert_eq!(after.name(), before.name());
    assert_eq!(after.parent_id(), before.parent_id());
    assert_eq!(after.is_visible(), before.is_visible());
    assert_eq!(after.opacity(), before.opacity());
    assert_eq!(after.blend_mode(), before.blend_mode());
    assert_eq!(after.kind(), NodeKind::Raster);
    assert_eq!(after.raster_cels().unwrap().len(), 1);
    assert_eq!(after.raster_cels().unwrap()[0].frame(), frame);
    assert_eq!(editor.selection_mask_snapshot(), selection_before);
    assert_eq!(editor.render_snapshot().unwrap().pixels(), visual_before);

    editor
        .navigate(Navigation::SetCurrentFrame {
            id: FrameId::DEFAULT,
        })
        .unwrap();
    assert!(
        editor
            .render_snapshot()
            .unwrap()
            .pixels()
            .iter()
            .all(|byte| *byte == 0)
    );
    editor.undo().unwrap();
    assert_eq!(
        editor.document().layer(id).unwrap().kind(),
        NodeKind::Vector
    );
    assert!(
        editor
            .render_snapshot()
            .unwrap()
            .pixels()
            .iter()
            .any(|byte| *byte != 0)
    );
    editor.redo().unwrap();
    assert_eq!(
        editor.document().layer(id).unwrap().kind(),
        NodeKind::Raster
    );
    assert!(
        editor
            .render_snapshot()
            .unwrap()
            .pixels()
            .iter()
            .all(|byte| *byte == 0)
    );
}

#[test]
fn invalid_semantic_inputs_are_typed_and_transactional() {
    let mut editor = Editor::new(Document::new(8, 8).unwrap()).unwrap();
    let before = editor.document().clone();
    let generation = editor.generation();
    let id = LayerId::new();
    let error = editor
        .execute(Command::AddTextNode {
            id,
            name: "Bad".into(),
            parent: None,
            sibling_index: 1,
            text: text("not supported: é", 0.0, 0.0, 8.0, Pixel::rgba(0, 0, 0, 255)),
        })
        .unwrap_err();
    assert!(matches!(error, CoreError::UnsupportedTextGlyph('é')));
    assert_eq!(editor.document(), &before);
    assert_eq!(editor.generation(), generation);
    assert!(!editor.can_undo());

    let vector_id = LayerId::new();
    editor
        .execute(Command::AddVectorNode {
            id: vector_id,
            name: "Vector".into(),
            parent: None,
            sibling_index: 1,
            vector: rectangle(0.0, 0.0, 1.0, 1.0, Pixel::rgba(1, 2, 3, 255)),
        })
        .unwrap();
    let before = editor.document().clone();
    let generation = editor.generation();
    let error = editor
        .execute(Command::SetTextContent {
            id: vector_id,
            text: text("A", 0.0, 0.0, 8.0, Pixel::rgba(0, 0, 0, 255)),
        })
        .unwrap_err();
    assert!(matches!(
        error,
        CoreError::UnsupportedNodeContent(NodeKind::Vector)
    ));
    assert_eq!(editor.document(), &before);
    assert_eq!(editor.generation(), generation);
}

#[test]
fn malformed_paths_and_out_of_bounds_styles_reject_before_commit() {
    let mut editor = Editor::new(Document::new(8, 8).unwrap()).unwrap();
    let invalid = VectorContent {
        paths: vec![VectorPath {
            commands: vec![PathCommand::LineTo { x: 1.0, y: 1.0 }],
            fill: Some(Pixel::rgba(1, 2, 3, 255)),
            stroke: Some(StrokeStyle {
                color: Pixel::rgba(0, 0, 0, 255),
                width: 5000.0,
            }),
            fill_rule: FillRule::EvenOdd,
        }],
    };
    let error = editor
        .execute(Command::AddVectorNode {
            id: LayerId::new(),
            name: "Bad path".into(),
            parent: None,
            sibling_index: 1,
            vector: invalid,
        })
        .unwrap_err();
    assert!(matches!(error, CoreError::InvalidSemanticStyle));
    assert_eq!(editor.generation(), 0);
    assert!(!editor.can_undo());
}

#[test]
fn semantic_work_limits_and_off_canvas_clipping_are_checked() {
    let mut off_canvas = Editor::new(Document::new(8, 8).unwrap()).unwrap();
    off_canvas
        .execute(Command::AddVectorNode {
            id: LayerId::new(),
            name: "Off canvas".into(),
            parent: None,
            sibling_index: 1,
            vector: rectangle(
                1_000_000.0,
                1_000_000.0,
                10.0,
                10.0,
                Pixel::rgba(255, 0, 0, 255),
            ),
        })
        .unwrap();
    assert!(
        off_canvas
            .render_snapshot()
            .unwrap()
            .pixels()
            .iter()
            .all(|byte| *byte == 0)
    );

    let mut expensive_commands = vec![PathCommand::MoveTo { x: 0.0, y: 0.0 }];
    for index in 0..20 {
        expensive_commands.push(PathCommand::LineTo {
            x: if index % 2 == 0 { 1024.0 } else { 0.0 },
            y: (index as f32 * 53.0).min(1024.0),
        });
    }
    expensive_commands.push(PathCommand::Close);
    let mut expensive = Editor::new(Document::new(1024, 1024).unwrap()).unwrap();
    let before = expensive.document().clone();
    let generation = expensive.generation();
    let error = expensive
        .execute(Command::AddVectorNode {
            id: LayerId::new(),
            name: "Bounded".into(),
            parent: None,
            sibling_index: 1,
            vector: VectorContent {
                paths: vec![VectorPath {
                    commands: expensive_commands,
                    fill: Some(Pixel::rgba(1, 2, 3, 255)),
                    stroke: None,
                    fill_rule: FillRule::EvenOdd,
                }],
            },
        })
        .unwrap_err();
    assert!(matches!(error, CoreError::SemanticWorkLimitExceeded));
    assert_eq!(expensive.generation(), generation);
    assert_eq!(expensive.document(), &before);
    assert!(!expensive.can_undo());
}

#[test]
fn fixed_cubic_flattening_has_an_exact_golden() {
    let mut editor = Editor::new(Document::new(16, 16).unwrap()).unwrap();
    editor
        .execute(Command::AddVectorNode {
            id: LayerId::new(),
            name: "Curve".into(),
            parent: None,
            sibling_index: 1,
            vector: VectorContent {
                paths: vec![VectorPath {
                    commands: vec![
                        PathCommand::MoveTo { x: 1.0, y: 14.0 },
                        PathCommand::CubicTo {
                            control1_x: 1.0,
                            control1_y: 1.0,
                            control2_x: 14.0,
                            control2_y: 1.0,
                            x: 14.0,
                            y: 14.0,
                        },
                        PathCommand::Close,
                    ],
                    fill: Some(Pixel::rgba(12, 34, 56, 200)),
                    stroke: Some(StrokeStyle {
                        color: Pixel::rgba(220, 210, 200, 255),
                        width: 1.5,
                    }),
                    fill_rule: FillRule::NonZero,
                }],
            },
        })
        .unwrap();
    assert_eq!(
        fnv64(editor.render_snapshot().unwrap().pixels()),
        4_278_607_839_294_759_260
    );
}

#[test]
fn v2_semantic_wire_defaults_load_without_os_font_state() {
    let base = redrob_core::save_project(&Document::new(8, 8).unwrap()).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&base).unwrap();
    let node = &mut value["document"]["nodes"][0];
    node["content"] = serde_json::json!({
        "kind": "text",
        "text": {
            "text": "A",
            "font_family": "Whatever Was Installed",
            "font_size": 8.0,
            "color": { "r": 255, "g": 255, "b": 255, "a": 255 }
        }
    });
    let loaded = redrob_core::load_project(&serde_json::to_vec(&value).unwrap()).unwrap();
    let NodeContent::Text { text } = loaded.nodes()[0].content() else {
        panic!("text")
    };
    assert_eq!(text.origin_x, 0.0);
    assert_eq!(text.origin_y, 0.0);
    assert_eq!(text.font_id, EMBEDDED_FONT_ID);
    assert_eq!(
        Editor::new(loaded)
            .unwrap()
            .render_snapshot()
            .unwrap()
            .pixels()
            .len(),
        8 * 8 * 4
    );
}

fn rendered_pixel(editor: &Editor, x: usize, y: usize) -> Pixel {
    let snapshot = editor.render_snapshot().unwrap();
    let offset = (y * snapshot.width() as usize + x) * 4;
    Pixel::rgba(
        snapshot.pixels()[offset],
        snapshot.pixels()[offset + 1],
        snapshot.pixels()[offset + 2],
        snapshot.pixels()[offset + 3],
    )
}

fn vector_with_path(
    commands: Vec<PathCommand>,
    fill: Option<Pixel>,
    stroke: Option<StrokeStyle>,
    fill_rule: FillRule,
) -> VectorContent {
    VectorContent {
        paths: vec![VectorPath {
            commands,
            fill,
            stroke,
            fill_rule,
        }],
    }
}

#[test]
fn open_fills_implicitly_close_each_subpath_and_preserve_even_odd_holes() {
    let mut triangle = Editor::new(Document::new(4, 4).unwrap()).unwrap();
    triangle
        .execute(Command::AddVectorNode {
            id: LayerId::new(),
            name: "Open triangle".into(),
            parent: None,
            sibling_index: 1,
            vector: vector_with_path(
                vec![
                    PathCommand::MoveTo { x: 0.0, y: 0.0 },
                    PathCommand::LineTo { x: 4.0, y: 0.0 },
                    PathCommand::LineTo { x: 4.0, y: 4.0 },
                ],
                Some(Pixel::rgba(20, 40, 60, 255)),
                None,
                FillRule::NonZero,
            ),
        })
        .unwrap();
    assert_eq!(
        rendered_pixel(&triangle, 3, 0),
        Pixel::rgba(20, 40, 60, 255)
    );
    assert_eq!(rendered_pixel(&triangle, 0, 3), Pixel::TRANSPARENT);

    let mut hole = Editor::new(Document::new(6, 6).unwrap()).unwrap();
    hole.execute(Command::AddVectorNode {
        id: LayerId::new(),
        name: "Open contours".into(),
        parent: None,
        sibling_index: 1,
        vector: vector_with_path(
            vec![
                PathCommand::MoveTo { x: 0.0, y: 0.0 },
                PathCommand::LineTo { x: 6.0, y: 0.0 },
                PathCommand::LineTo { x: 6.0, y: 6.0 },
                PathCommand::LineTo { x: 0.0, y: 6.0 },
                PathCommand::MoveTo { x: 2.0, y: 2.0 },
                PathCommand::LineTo { x: 4.0, y: 2.0 },
                PathCommand::LineTo { x: 4.0, y: 4.0 },
                PathCommand::LineTo { x: 2.0, y: 4.0 },
            ],
            Some(Pixel::rgba(200, 100, 50, 255)),
            None,
            FillRule::EvenOdd,
        ),
    })
    .unwrap();
    assert_eq!(rendered_pixel(&hole, 0, 0), Pixel::rgba(200, 100, 50, 255));
    assert_eq!(rendered_pixel(&hole, 2, 2), Pixel::TRANSPARENT);
}

#[test]
fn close_grammar_return_to_start_and_zero_length_policy_are_transactional() {
    let mut editor = Editor::new(Document::new(8, 8).unwrap()).unwrap();
    let invalid = vector_with_path(
        vec![
            PathCommand::MoveTo { x: 1.0, y: 1.0 },
            PathCommand::LineTo { x: 5.0, y: 1.0 },
            PathCommand::Close,
            PathCommand::LineTo { x: 5.0, y: 5.0 },
        ],
        Some(Pixel::rgba(1, 2, 3, 255)),
        None,
        FillRule::NonZero,
    );
    assert!(matches!(
        editor.execute(Command::AddVectorNode {
            id: LayerId::new(),
            name: "Post close".into(),
            parent: None,
            sibling_index: 1,
            vector: invalid,
        }),
        Err(CoreError::InvalidSemanticPath)
    ));
    assert_eq!(editor.generation(), 0);

    editor
        .execute(Command::AddVectorNode {
            id: LayerId::new(),
            name: "Return then close".into(),
            parent: None,
            sibling_index: 1,
            vector: vector_with_path(
                vec![
                    PathCommand::MoveTo { x: 1.0, y: 1.0 },
                    PathCommand::LineTo { x: 5.0, y: 1.0 },
                    PathCommand::LineTo { x: 1.0, y: 1.0 },
                    PathCommand::Close,
                ],
                None,
                Some(StrokeStyle {
                    color: Pixel::rgba(255, 0, 0, 255),
                    width: 1.0,
                }),
                FillRule::NonZero,
            ),
        })
        .unwrap();

    let no_op_id = LayerId::new();
    editor
        .execute(Command::AddVectorNode {
            id: no_op_id,
            name: "Zero no-op".into(),
            parent: None,
            sibling_index: 2,
            vector: vector_with_path(
                vec![
                    PathCommand::MoveTo { x: 1.0, y: 3.0 },
                    PathCommand::LineTo { x: 1.0, y: 3.0 },
                    PathCommand::LineTo { x: 6.0, y: 3.0 },
                    PathCommand::CubicTo {
                        control1_x: 6.0,
                        control1_y: 3.0,
                        control2_x: 6.0,
                        control2_y: 3.0,
                        x: 6.0,
                        y: 3.0,
                    },
                ],
                None,
                Some(StrokeStyle {
                    color: Pixel::rgba(0, 255, 0, 255),
                    width: 1.0,
                }),
                FillRule::NonZero,
            ),
        })
        .unwrap();
    assert_eq!(
        editor.document().layer(no_op_id).unwrap().kind(),
        NodeKind::Vector
    );

    let all_zero = vector_with_path(
        vec![
            PathCommand::MoveTo { x: 2.0, y: 2.0 },
            PathCommand::LineTo { x: 2.0, y: 2.0 },
        ],
        None,
        Some(StrokeStyle {
            color: Pixel::rgba(0, 0, 0, 255),
            width: 2.0,
        }),
        FillRule::NonZero,
    );
    assert!(matches!(
        editor.execute(Command::AddVectorNode {
            id: LayerId::new(),
            name: "All zero".into(),
            parent: None,
            sibling_index: 3,
            vector: all_zero,
        }),
        Err(CoreError::InvalidSemanticPath)
    ));
}

#[test]
fn paintless_paths_are_typed_rejections_and_never_enter_render_work() {
    let mut editor = Editor::new(Document::new(1024, 1024).unwrap()).unwrap();
    let before = editor.document().clone();
    let generation = editor.generation();
    let error = editor
        .execute(Command::AddVectorNode {
            id: LayerId::new(),
            name: "Paintless".into(),
            parent: None,
            sibling_index: 1,
            vector: vector_with_path(
                vec![
                    PathCommand::MoveTo { x: 0.0, y: 0.0 },
                    PathCommand::LineTo {
                        x: 1024.0,
                        y: 1024.0,
                    },
                ],
                None,
                None,
                FillRule::NonZero,
            ),
        })
        .unwrap_err();
    assert!(matches!(error, CoreError::InvalidSemanticStyle));
    assert_eq!(editor.document(), &before);
    assert_eq!(editor.generation(), generation);
    assert!(!editor.can_undo());

    editor
        .execute(Command::AddVectorNode {
            id: LayerId::new(),
            name: "Empty transparent vector".into(),
            parent: None,
            sibling_index: 1,
            vector: VectorContent { paths: vec![] },
        })
        .unwrap();
    assert!(editor.render_snapshot().is_ok());
}

#[test]
fn rasterize_matches_semantic_over_opaque_nested_backdrop_for_every_blend_mode() {
    for mode in [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::Add,
    ] {
        let mut editor = Editor::new(Document::new(8, 8).unwrap()).unwrap();
        editor
            .execute(Command::Fill {
                color: Pixel::rgba(40, 80, 120, 255),
            })
            .unwrap();
        let group = LayerId::new();
        editor
            .execute(Command::AddGroup {
                id: group,
                name: "Nested".into(),
                parent: None,
                sibling_index: 1,
            })
            .unwrap();
        let semantic = LayerId::new();
        editor
            .execute(Command::AddVectorNode {
                id: semantic,
                name: "Fill and stroke".into(),
                parent: Some(group),
                sibling_index: 0,
                vector: vector_with_path(
                    vec![
                        PathCommand::MoveTo { x: 1.0, y: 1.0 },
                        PathCommand::LineTo { x: 7.0, y: 1.0 },
                        PathCommand::LineTo { x: 7.0, y: 7.0 },
                        PathCommand::LineTo { x: 1.0, y: 7.0 },
                        PathCommand::Close,
                    ],
                    Some(Pixel::rgba(200, 60, 20, 220)),
                    Some(StrokeStyle {
                        color: Pixel::rgba(10, 220, 90, 255),
                        width: 1.5,
                    }),
                    FillRule::NonZero,
                ),
            })
            .unwrap();
        editor
            .execute(Command::SetLayerOpacity {
                id: semantic,
                opacity: 0.5,
            })
            .unwrap();
        editor
            .execute(Command::SetLayerBlendMode { id: semantic, mode })
            .unwrap();
        let before = editor.render_snapshot().unwrap().pixels().to_vec();
        editor
            .execute(Command::RasterizeSemanticNode { id: semantic })
            .unwrap();
        assert_eq!(
            editor.render_snapshot().unwrap().pixels(),
            before,
            "{mode:?}"
        );
    }
}

#[test]
fn vector_fill_is_composited_before_stroke() {
    let mut editor = Editor::new(Document::new(2, 2).unwrap()).unwrap();
    editor
        .execute(Command::AddVectorNode {
            id: LayerId::new(),
            name: "Ordered paint".into(),
            parent: None,
            sibling_index: 1,
            vector: vector_with_path(
                vec![
                    PathCommand::MoveTo { x: 0.0, y: 0.0 },
                    PathCommand::LineTo { x: 2.0, y: 0.0 },
                    PathCommand::LineTo { x: 2.0, y: 2.0 },
                    PathCommand::LineTo { x: 0.0, y: 2.0 },
                    PathCommand::Close,
                ],
                Some(Pixel::rgba(255, 0, 0, 255)),
                Some(StrokeStyle {
                    color: Pixel::rgba(0, 0, 255, 255),
                    width: 2.0,
                }),
                FillRule::NonZero,
            ),
        })
        .unwrap();
    assert_eq!(rendered_pixel(&editor, 0, 0), Pixel::rgba(0, 0, 255, 255));
}

#[test]
fn stroke_only_open_subpath_does_not_gain_a_closing_edge() {
    let mut editor = Editor::new(Document::new(6, 6).unwrap()).unwrap();
    editor
        .execute(Command::AddVectorNode {
            id: LayerId::new(),
            name: "Open stroke".into(),
            parent: None,
            sibling_index: 1,
            vector: vector_with_path(
                vec![
                    PathCommand::MoveTo { x: 0.0, y: 0.0 },
                    PathCommand::LineTo { x: 6.0, y: 0.0 },
                    PathCommand::LineTo { x: 6.0, y: 6.0 },
                ],
                None,
                Some(StrokeStyle {
                    color: Pixel::rgba(255, 0, 0, 255),
                    width: 0.5,
                }),
                FillRule::NonZero,
            ),
        })
        .unwrap();
    assert_eq!(rendered_pixel(&editor, 2, 2), Pixel::TRANSPARENT);
    assert_ne!(rendered_pixel(&editor, 5, 0), Pixel::TRANSPARENT);
}

#[test]
fn every_non_move_command_is_rejected_after_close() {
    let trailing = [
        PathCommand::LineTo { x: 3.0, y: 3.0 },
        PathCommand::CubicTo {
            control1_x: 1.0,
            control1_y: 1.0,
            control2_x: 2.0,
            control2_y: 2.0,
            x: 3.0,
            y: 3.0,
        },
        PathCommand::Close,
    ];
    for command in trailing {
        let mut editor = Editor::new(Document::new(4, 4).unwrap()).unwrap();
        let error = editor
            .execute(Command::AddVectorNode {
                id: LayerId::new(),
                name: "Post close".into(),
                parent: None,
                sibling_index: 1,
                vector: vector_with_path(
                    vec![
                        PathCommand::MoveTo { x: 0.0, y: 0.0 },
                        PathCommand::LineTo { x: 4.0, y: 0.0 },
                        PathCommand::LineTo { x: 4.0, y: 4.0 },
                        PathCommand::Close,
                        command,
                    ],
                    Some(Pixel::rgba(1, 2, 3, 255)),
                    None,
                    FillRule::NonZero,
                ),
            })
            .unwrap_err();
        assert!(matches!(error, CoreError::InvalidSemanticPath));
        assert_eq!(editor.generation(), 0);
    }
}

#[test]
fn typed_semantic_payload_limits_match_decoder_and_round_trip_at_exact_boundaries() {
    let exact_text_id = LayerId::new();
    let mut exact_text_editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
    exact_text_editor
        .execute(Command::AddTextNode {
            id: exact_text_id,
            name: "Exact text".into(),
            parent: None,
            sibling_index: 1,
            text: text(
                &"x".repeat(MAX_TEXT_CONTENT_BYTES),
                2.0,
                0.0,
                1.0,
                Pixel::rgba(1, 2, 3, 255),
            ),
        })
        .unwrap();
    let encoded = save_project(exact_text_editor.document()).unwrap();
    let decoded = load_project(&encoded).unwrap();
    let NodeContent::Text { text: decoded_text } = decoded.layer(exact_text_id).unwrap().content()
    else {
        panic!("exact text node changed kind after round trip");
    };
    assert_eq!(decoded_text.text.len(), MAX_TEXT_CONTENT_BYTES);

    let mut over_text_editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
    let before = over_text_editor.document().clone();
    let error = over_text_editor
        .execute(Command::AddTextNode {
            id: LayerId::new(),
            name: "Over text".into(),
            parent: None,
            sibling_index: 1,
            text: text(
                &"x".repeat(MAX_TEXT_CONTENT_BYTES + 1),
                2.0,
                0.0,
                1.0,
                Pixel::rgba(1, 2, 3, 255),
            ),
        })
        .unwrap_err();
    assert!(matches!(
        error,
        CoreError::DocumentLimitExceeded("text content bytes")
    ));
    assert_eq!(over_text_editor.document(), &before);
    assert_eq!(over_text_editor.generation(), 0);

    let family_id = LayerId::new();
    let mut family_editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
    let mut exact_family = text("A", 2.0, 0.0, 1.0, Pixel::rgba(1, 2, 3, 255));
    exact_family.font_family = "f".repeat(MAX_FONT_FAMILY_BYTES);
    family_editor
        .execute(Command::AddTextNode {
            id: family_id,
            name: "Exact family".into(),
            parent: None,
            sibling_index: 1,
            text: exact_family,
        })
        .unwrap();
    load_project(&save_project(family_editor.document()).unwrap()).unwrap();
    let before = family_editor.document().clone();
    let generation = family_editor.generation();
    let mut over_family = text("A", 2.0, 0.0, 1.0, Pixel::rgba(1, 2, 3, 255));
    over_family.font_family = "f".repeat(MAX_FONT_FAMILY_BYTES + 1);
    assert!(matches!(
        family_editor.execute(Command::SetTextContent {
            id: family_id,
            text: over_family,
        }),
        Err(CoreError::DocumentLimitExceeded("font family bytes"))
    ));
    assert_eq!(family_editor.document(), &before);
    assert_eq!(family_editor.generation(), generation);

    let mut exact_invalid_font_id = text("A", 2.0, 0.0, 1.0, Pixel::rgba(1, 2, 3, 255));
    exact_invalid_font_id.font_id = "f".repeat(MAX_FONT_ID_BYTES);
    assert!(matches!(
        family_editor.execute(Command::SetTextContent {
            id: family_id,
            text: exact_invalid_font_id,
        }),
        Err(CoreError::InvalidSemanticStyle)
    ));
    let mut over_font_id = text("A", 2.0, 0.0, 1.0, Pixel::rgba(1, 2, 3, 255));
    over_font_id.font_id = "f".repeat(MAX_FONT_ID_BYTES + 1);
    assert!(matches!(
        family_editor.execute(Command::SetTextContent {
            id: family_id,
            text: over_font_id,
        }),
        Err(CoreError::DocumentLimitExceeded("font id bytes"))
    ));

    let off_canvas_path = VectorPath {
        commands: vec![
            PathCommand::MoveTo {
                x: 1_000_000.0,
                y: 1_000_000.0,
            },
            PathCommand::LineTo {
                x: 1_000_001.0,
                y: 1_000_000.0,
            },
        ],
        fill: None,
        stroke: Some(StrokeStyle {
            color: Pixel::rgba(9, 8, 7, 255),
            width: 1.0,
        }),
        fill_rule: FillRule::NonZero,
    };
    let exact_paths_id = LayerId::new();
    let mut exact_paths_editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
    exact_paths_editor
        .execute(Command::AddVectorNode {
            id: exact_paths_id,
            name: "Exact paths".into(),
            parent: None,
            sibling_index: 1,
            vector: VectorContent {
                paths: vec![off_canvas_path.clone(); MAX_VECTOR_PATHS],
            },
        })
        .unwrap();
    let decoded = load_project(&save_project(exact_paths_editor.document()).unwrap()).unwrap();
    let NodeContent::Vector { vector } = decoded.layer(exact_paths_id).unwrap().content() else {
        panic!("exact vector node changed kind after round trip");
    };
    assert_eq!(vector.paths.len(), MAX_VECTOR_PATHS);
    let before = exact_paths_editor.document().clone();
    let generation = exact_paths_editor.generation();
    assert!(matches!(
        exact_paths_editor.execute(Command::SetVectorContent {
            id: exact_paths_id,
            vector: VectorContent {
                paths: vec![off_canvas_path.clone(); MAX_VECTOR_PATHS + 1],
            },
        }),
        Err(CoreError::DocumentLimitExceeded("vector path count"))
    ));
    assert_eq!(exact_paths_editor.document(), &before);
    assert_eq!(exact_paths_editor.generation(), generation);

    let commands = |count: usize| {
        let mut commands = Vec::with_capacity(count);
        commands.push(PathCommand::MoveTo {
            x: 1_000_000.0,
            y: 1_000_000.0,
        });
        for index in 1..count {
            commands.push(PathCommand::LineTo {
                x: if index % 2 == 0 {
                    1_000_000.0
                } else {
                    1_000_001.0
                },
                y: 1_000_000.0,
            });
        }
        commands
    };
    let exact_commands_id = LayerId::new();
    let mut exact_commands_editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
    exact_commands_editor
        .execute(Command::AddVectorNode {
            id: exact_commands_id,
            name: "Exact commands".into(),
            parent: None,
            sibling_index: 1,
            vector: VectorContent {
                paths: vec![VectorPath {
                    commands: commands(MAX_PATH_COMMANDS_PER_PATH),
                    ..off_canvas_path.clone()
                }],
            },
        })
        .unwrap();
    let decoded = load_project(&save_project(exact_commands_editor.document()).unwrap()).unwrap();
    let NodeContent::Vector { vector } = decoded.layer(exact_commands_id).unwrap().content() else {
        panic!("exact command vector changed kind after round trip");
    };
    assert_eq!(vector.paths[0].commands.len(), MAX_PATH_COMMANDS_PER_PATH);
    let before = exact_commands_editor.document().clone();
    let generation = exact_commands_editor.generation();
    assert!(matches!(
        exact_commands_editor.execute(Command::SetVectorContent {
            id: exact_commands_id,
            vector: VectorContent {
                paths: vec![VectorPath {
                    commands: commands(MAX_PATH_COMMANDS_PER_PATH + 1),
                    ..off_canvas_path
                }],
            },
        }),
        Err(CoreError::DocumentLimitExceeded("path commands per path"))
    ));
    assert_eq!(exact_commands_editor.document(), &before);
    assert_eq!(exact_commands_editor.generation(), generation);
}

#[test]
fn add_and_set_semantic_commands_preflight_before_commit() {
    let width = 2_049;
    let height = 2_048;
    let expensive_text = text("A", 0.0, 0.0, width as f32, Pixel::rgba(10, 20, 30, 255));
    let mut editor = Editor::new(Document::new(width, height).unwrap()).unwrap();
    let before = editor.document().clone();
    assert!(matches!(
        editor.execute(Command::AddTextNode {
            id: LayerId::new(),
            name: "Expensive text".into(),
            parent: None,
            sibling_index: 1,
            text: expensive_text.clone(),
        }),
        Err(CoreError::SemanticWorkLimitExceeded)
    ));
    assert_eq!(editor.document(), &before);
    assert_eq!(editor.generation(), 0);
    assert!(!editor.can_undo());

    let text_id = LayerId::new();
    editor
        .execute(Command::AddTextNode {
            id: text_id,
            name: "Editable text".into(),
            parent: None,
            sibling_index: 1,
            text: text(
                "A",
                1_000_000.0,
                1_000_000.0,
                8.0,
                Pixel::rgba(1, 2, 3, 255),
            ),
        })
        .unwrap();
    let before = editor.document().clone();
    let generation = editor.generation();
    assert!(matches!(
        editor.execute(Command::SetTextContent {
            id: text_id,
            text: expensive_text,
        }),
        Err(CoreError::SemanticWorkLimitExceeded)
    ));
    assert_eq!(editor.document(), &before);
    assert_eq!(editor.generation(), generation);

    let vector_id = LayerId::new();
    editor
        .execute(Command::AddVectorNode {
            id: vector_id,
            name: "Editable vector".into(),
            parent: None,
            sibling_index: 2,
            vector: rectangle(
                1_000_000.0,
                1_000_000.0,
                10.0,
                10.0,
                Pixel::rgba(1, 2, 3, 255),
            ),
        })
        .unwrap();
    let mut commands = vec![PathCommand::MoveTo { x: 0.0, y: 0.0 }];
    for index in 0..20 {
        commands.push(PathCommand::LineTo {
            x: if index % 2 == 0 { width as f32 } else { 0.0 },
            y: (index as f32 * 109.0).min(height as f32),
        });
    }
    commands.push(PathCommand::Close);
    let expensive_vector = VectorContent {
        paths: vec![VectorPath {
            commands,
            fill: Some(Pixel::rgba(1, 2, 3, 255)),
            stroke: None,
            fill_rule: FillRule::EvenOdd,
        }],
    };
    let before = editor.document().clone();
    let generation = editor.generation();
    assert!(matches!(
        editor.execute(Command::SetVectorContent {
            id: vector_id,
            vector: expensive_vector,
        }),
        Err(CoreError::SemanticWorkLimitExceeded)
    ));
    assert_eq!(editor.document(), &before);
    assert_eq!(editor.generation(), generation);
}

#[test]
fn hidden_semantic_nodes_are_preflighted_during_project_load() {
    let base = save_project(&Document::new(1_024, 1_024).unwrap()).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&base).unwrap();
    let node = &mut value["document"]["nodes"][0];
    node["visible"] = serde_json::json!(false);
    let mut commands = vec![serde_json::json!({"type":"move_to", "x":0.0, "y":0.0})];
    for index in 0..20 {
        commands.push(serde_json::json!({
            "type":"line_to",
            "x": if index % 2 == 0 { 1_024.0 } else { 0.0 },
            "y": (index as f32 * 53.0).min(1_024.0)
        }));
    }
    commands.push(serde_json::json!({"type":"close"}));
    node["content"] = serde_json::json!({
        "kind":"vector",
        "vector":{"paths":[{
            "commands":commands,
            "fill":{"r":1,"g":2,"b":3,"a":255},
            "fill_rule":"even_odd"
        }]}
    });
    assert!(matches!(
        load_project(&serde_json::to_vec(&value).unwrap()),
        Err(CoreError::SemanticWorkLimitExceeded)
    ));
}

#[test]
fn rasterize_storage_quota_accepts_exact_limit_and_rejects_next_allocation() {
    let width = 4_096;
    let height = 2_048;
    let raster_bytes = u64::from(width) * u64::from(height) * 4;
    let mut editor = Editor::new(Document::new(width, height).unwrap()).unwrap();
    for raw_id in 1..30 {
        editor
            .execute(Command::DuplicateFrame {
                source: FrameId::DEFAULT,
                id: FrameId::new(raw_id),
                index: raw_id as usize,
            })
            .unwrap();
    }

    let raster_id = editor.document().active_layer_id();
    editor
        .execute(Command::AddRasterMask { id: raster_id })
        .unwrap();
    for index in 0..2 {
        let group = LayerId::new();
        editor
            .execute(Command::AddGroup {
                id: group,
                name: format!("Quota group {index}"),
                parent: None,
                sibling_index: index + 1,
            })
            .unwrap();
        editor
            .execute(Command::AddRasterMask { id: group })
            .unwrap();
    }

    let semantic_id = LayerId::new();
    editor
        .execute(Command::AddVectorNode {
            id: semantic_id,
            name: "Quota semantic".into(),
            parent: None,
            sibling_index: 3,
            vector: rectangle(
                1_000_000.0,
                1_000_000.0,
                10.0,
                10.0,
                Pixel::rgba(1, 2, 3, 255),
            ),
        })
        .unwrap();
    assert_eq!(
        editor.document().stored_raster_bytes(),
        MAX_STORED_RASTER_BYTES - raster_bytes
    );
    editor
        .execute(Command::RasterizeSemanticNode { id: semantic_id })
        .unwrap();
    assert_eq!(
        editor.document().stored_raster_bytes(),
        MAX_STORED_RASTER_BYTES
    );

    editor.undo().unwrap();
    let over_group = LayerId::new();
    editor
        .execute(Command::AddGroup {
            id: over_group,
            name: "One allocation over".into(),
            parent: None,
            sibling_index: 3,
        })
        .unwrap();
    editor
        .execute(Command::AddRasterMask { id: over_group })
        .unwrap();
    let before = editor.document().clone();
    let generation = editor.generation();
    assert!(matches!(
        editor.execute(Command::RasterizeSemanticNode { id: semantic_id }),
        Err(CoreError::DocumentLimitExceeded("stored raster bytes"))
    ));
    assert_eq!(editor.document(), &before);
    assert_eq!(editor.generation(), generation);
}

fn full_canvas_edge_vector(
    edge_count: usize,
    fill: Option<Pixel>,
    stroke: Option<StrokeStyle>,
) -> VectorContent {
    let explicit_edges = edge_count - usize::from(fill.is_some());
    let mut commands = Vec::with_capacity(explicit_edges + 1);
    commands.push(PathCommand::MoveTo { x: 0.0, y: 0.0 });
    for index in 1..=explicit_edges {
        commands.push(PathCommand::LineTo {
            x: if index % 2 == 0 { 0.0 } else { 1_024.0 },
            y: 1_024.0 * index as f32 / explicit_edges as f32,
        });
    }
    vector_with_path(commands, fill, stroke, FillRule::EvenOdd)
}

#[test]
fn semantic_sample_edge_budget_counts_each_enabled_paint_branch() {
    assert_eq!(MAX_SEMANTIC_SAMPLE_EDGE_VISITS, 1_024_u64 * 1_024 * 16 * 16);
    let fill = Some(Pixel::rgba(1, 2, 3, 255));
    let stroke = Some(StrokeStyle {
        color: Pixel::rgba(4, 5, 6, 255),
        width: 1.0,
    });
    let cases = [
        ("fill", 16, 17, fill, None),
        ("stroke", 16, 17, None, stroke),
        ("fill and stroke", 8, 9, fill, stroke),
    ];

    for (name, exact_edges, over_edges, fill, stroke) in cases {
        let mut exact = Editor::new(Document::new(1_024, 1_024).unwrap()).unwrap();
        exact
            .execute(Command::AddVectorNode {
                id: LayerId::new(),
                name: format!("Exact {name}"),
                parent: None,
                sibling_index: 1,
                vector: full_canvas_edge_vector(exact_edges, fill, stroke),
            })
            .unwrap();
        assert_eq!(exact.generation(), 1, "{name} exact limit");

        let mut over = Editor::new(Document::new(1_024, 1_024).unwrap()).unwrap();
        let before = over.document().clone();
        assert!(matches!(
            over.execute(Command::AddVectorNode {
                id: LayerId::new(),
                name: format!("Over {name}"),
                parent: None,
                sibling_index: 1,
                vector: full_canvas_edge_vector(over_edges, fill, stroke),
            }),
            Err(CoreError::SemanticWorkLimitExceeded)
        ));
        assert_eq!(over.generation(), 0, "{name} one over");
        assert_eq!(over.document(), &before, "{name} transaction");
        assert!(!over.can_undo(), "{name} history");
    }
}
