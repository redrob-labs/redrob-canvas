// SPDX-License-Identifier: GPL-3.0-or-later

use redrob_core::{
    Affine2D, BlendMode, BrushPoint, BrushSettings, BrushSmoothing, BrushTip, Command, CoreError,
    CurvePoint, DabShape, Document, DocumentMetadata, EMBEDDED_FONT_ID, Editor, Filter,
    FloodFillOptions, FrameId, GradientKind, GradientStop, HistoryConfig, LayerId,
    MAX_BRUSH_PIXEL_VISITS, MAX_BRUSH_POINTS, MAX_BRUSH_SIZE, MAX_FRAMES, MAX_HIERARCHY_DEPTH,
    MAX_NODES, MAX_PATH_COMMANDS, MAX_PATH_COMMANDS_PER_PATH, MAX_RENDER_PIXEL_VISITS,
    MAX_SEMANTIC_MEMORY_BYTES, MAX_TEXT_BYTES, MAX_TEXT_CONTENT_BYTES, MAX_VECTOR_PATHS, NodeKind,
    PathCommand, Pixel, Rect, SamplingMode, SelectionMode, SemanticUsage, SpacingOptions,
    TextContent, ToneCurve, VectorContent, VectorPath, admit_semantic_replacement, export_png,
    import_png, load_project, save_project,
};

fn pixel(editor: &Editor, layer: LayerId, x: u32, y: u32) -> Pixel {
    editor
        .document()
        .layer(layer)
        .unwrap()
        .pixel(editor.document().width(), x, y)
        .unwrap()
}

#[test]
fn typed_commands_serialize_and_layer_commands_preserve_identity() {
    let document = Document::new(2, 2).unwrap();
    let mut editor = Editor::new(document).unwrap();
    let id = LayerId::new();
    let command = Command::AddLayer {
        id,
        name: "Ink".into(),
        index: 1,
    };
    let json = serde_json::to_string(&command).unwrap();
    let decoded: Command = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded, command);

    editor.execute(decoded).unwrap();
    editor
        .execute(Command::RenameLayer {
            id,
            name: "Line art".into(),
        })
        .unwrap();
    editor
        .execute(Command::SetLayerOpacity { id, opacity: 0.5 })
        .unwrap();
    editor
        .execute(Command::ReorderLayer { id, new_index: 0 })
        .unwrap();
    assert_eq!(editor.document().layers()[0].id(), id);
    assert_eq!(editor.document().layer(id).unwrap().name(), "Line art");
    assert_eq!(editor.document().active_layer_id(), id);

    editor.execute(Command::RemoveLayer { id }).unwrap();
    assert!(editor.document().layer(id).is_none());
    assert!(matches!(
        editor.execute(Command::RemoveLayer {
            id: editor.document().active_layer_id()
        }),
        Err(CoreError::LastLayer)
    ));
}

#[test]
fn rendering_composites_order_opacity_visibility_and_blend_modes() {
    let mut editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
    let bottom = editor.document().active_layer_id();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(100, 120, 200, 255),
        })
        .unwrap();
    let top = LayerId::new();
    editor
        .execute(Command::AddLayer {
            id: top,
            name: "Top".into(),
            index: 1,
        })
        .unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(200, 100, 50, 255),
        })
        .unwrap();

    let cases = [
        (BlendMode::Normal, [200, 100, 50, 255]),
        // Multiply and Add blend in linear light (A.7), so these differ from the sRGB product/sum.
        (BlendMode::Multiply, [77, 43, 37, 255]),
        (BlendMode::Screen, [222, 173, 211, 255]),
        (BlendMode::Overlay, [157, 94, 167, 255]),
        (BlendMode::Add, [219, 152, 205, 255]),
    ];
    for (mode, expected) in cases {
        editor
            .execute(Command::SetLayerBlendMode { id: top, mode })
            .unwrap();
        assert_eq!(editor.render_snapshot().unwrap().pixels(), expected);
    }
    editor
        .execute(Command::SetLayerVisibility {
            id: top,
            visible: false,
        })
        .unwrap();
    assert_eq!(
        editor.render_snapshot().unwrap().pixels(),
        [100, 120, 200, 255]
    );
    editor
        .execute(Command::SetLayerVisibility {
            id: top,
            visible: true,
        })
        .unwrap();
    editor
        .execute(Command::SetLayerBlendMode {
            id: top,
            mode: BlendMode::Normal,
        })
        .unwrap();
    editor
        .execute(Command::SetLayerOpacity {
            id: top,
            opacity: 0.5,
        })
        .unwrap();
    assert_eq!(
        editor.render_snapshot().unwrap().pixels(),
        [150, 110, 125, 255]
    );
    assert_eq!(editor.document().layer(bottom).unwrap().opacity(), 1.0);
}

#[test]
fn selection_boolean_modes_limit_fill_clear_and_filters() {
    let mut editor = Editor::new(Document::new(4, 2).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(10, 20, 30, 255),
        })
        .unwrap();
    editor
        .execute(Command::SelectRectangle {
            rect: Rect::new(0, 0, 2, 2),
            mode: SelectionMode::Replace,
        })
        .unwrap();
    editor
        .execute(Command::SelectRectangle {
            rect: Rect::new(1, 0, 2, 2),
            mode: SelectionMode::Intersect,
        })
        .unwrap();
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Invert,
        })
        .unwrap();
    assert_eq!(pixel(&editor, layer, 0, 0), Pixel::rgba(10, 20, 30, 255));
    assert_eq!(pixel(&editor, layer, 1, 0), Pixel::rgba(245, 235, 225, 255));
    assert_eq!(pixel(&editor, layer, 2, 0), Pixel::rgba(10, 20, 30, 255));

    editor
        .execute(Command::SelectRectangle {
            rect: Rect::new(2, 0, 1, 2),
            mode: SelectionMode::Add,
        })
        .unwrap();
    editor
        .execute(Command::SelectRectangle {
            rect: Rect::new(1, 0, 1, 1),
            mode: SelectionMode::Subtract,
        })
        .unwrap();
    editor.execute(Command::Clear).unwrap();
    assert_eq!(pixel(&editor, layer, 1, 0), Pixel::rgba(245, 235, 225, 255));
    assert_eq!(pixel(&editor, layer, 1, 1), Pixel::TRANSPARENT);
    assert_eq!(pixel(&editor, layer, 2, 0), Pixel::TRANSPARENT);
}

#[test]
fn brush_interpolates_and_uses_pressure_and_selection() {
    let mut editor = Editor::new(Document::new(12, 5).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    editor
        .execute(Command::SelectRectangle {
            rect: Rect::new(2, 0, 8, 5),
            mode: SelectionMode::Replace,
        })
        .unwrap();
    editor
        .execute(Command::BrushStroke {
            points: vec![
                BrushPoint::new(1.0, 2.5, 0.0),
                BrushPoint::new(11.0, 2.5, 1.0),
            ],
            color: Pixel::rgba(255, 0, 0, 255),
            size: 3.0,
            opacity: 1.0,
            settings: BrushSettings::default(),
            tip: None,
            pipe: Vec::new(),
        })
        .unwrap();
    assert_eq!(pixel(&editor, layer, 0, 2), Pixel::TRANSPARENT);
    assert!(pixel(&editor, layer, 5, 2).a > 0);
    assert_eq!(pixel(&editor, layer, 11, 2), Pixel::TRANSPARENT);
    assert!(pixel(&editor, layer, 9, 2).a > pixel(&editor, layer, 3, 2).a);
}

#[test]
fn brush_point_limit_accepts_exact_boundary_and_rejects_one_over_transactionally() {
    let point = BrushPoint::new(0.5, 0.5, 1.0);
    let command = |count| Command::BrushStroke {
        points: vec![point; count],
        color: Pixel::rgba(255, 0, 0, 255),
        size: 1.0,
        opacity: 1.0,
        settings: BrushSettings::default(),
        tip: None,
        pipe: Vec::new(),
    };
    let mut editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
    editor.execute(command(MAX_BRUSH_POINTS)).unwrap();
    assert_eq!(editor.generation(), 1);

    let error = editor.execute(command(MAX_BRUSH_POINTS + 1)).unwrap_err();
    assert!(matches!(
        error,
        CoreError::InvalidBrushPointCount {
            actual,
            max: MAX_BRUSH_POINTS
        } if actual == MAX_BRUSH_POINTS + 1
    ));
    assert_eq!(editor.generation(), 1);

    assert!(matches!(
        editor.execute(command(0)),
        Err(CoreError::InvalidBrushPointCount { actual: 0, .. })
    ));
    assert_eq!(editor.generation(), 1);
}

#[test]
fn brush_size_limit_accepts_boundary_and_rejects_one_over_transactionally() {
    let command = |size| Command::BrushStroke {
        points: vec![BrushPoint::new(0.5, 0.5, 1.0)],
        color: Pixel::rgba(255, 0, 0, 255),
        size,
        opacity: 1.0,
        settings: BrushSettings::default(),
        tip: None,
        pipe: Vec::new(),
    };
    let mut editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
    editor.execute(command(MAX_BRUSH_SIZE)).unwrap();
    let generation = editor.generation();
    let pixels = editor.render_snapshot().unwrap().pixels().to_vec();

    assert!(matches!(
        editor.execute(command(MAX_BRUSH_SIZE + 1.0)),
        Err(CoreError::InvalidBrushSize)
    ));
    assert_eq!(editor.generation(), generation);
    assert_eq!(editor.render_snapshot().unwrap().pixels(), pixels);
}

#[test]
fn brush_work_amplification_is_rejected_before_pixels_change() {
    const CANVAS_SIZE: u32 = 129;

    let mut editor = Editor::new(Document::new(CANVAS_SIZE, CANVAS_SIZE).unwrap()).unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(7, 11, 13, 255),
        })
        .unwrap();
    let generation = editor.generation();
    let pixels = editor.render_snapshot().unwrap().pixels().to_vec();
    let center = CANVAS_SIZE as f32 / 2.0;

    // The points MOVE, and the spacing is set to its minimum. They used to be `MAX_BRUSH_POINTS` copies
    // of one position, which the old dab placer turned into one dab each -- it emitted `steps.max(1.0)`
    // per segment, so a zero-length segment still painted. The translated spacing places no dab for a
    // zero-length move, which is Krita's own `if (start == end) return -1`.
    //
    // Both parts are needed now. A moving stroke alone is not enough: at a 1,000-pixel brush the default
    // quarter-size spacing is a 250-pixel ellipse, larger than this canvas, so a stroke across it places
    // one dab. The minimum spacing of 0.02 gives a 20-pixel ellipse, and 60-pixel moves then cross it
    // repeatedly -- which is what it takes to exceed a 64Mi visit budget at ~16.6K visits per
    // canvas-clipped dab.
    let points: Vec<BrushPoint> = (0..MAX_BRUSH_POINTS)
        .map(|index| {
            let offset = if index % 2 == 0 { -30.0 } else { 30.0 };
            BrushPoint::new(center + offset, center + offset * 0.5, 1.0)
        })
        .collect();

    let error = editor
        .execute(Command::BrushStroke {
            points,
            color: Pixel::rgba(255, 0, 0, 255),
            size: MAX_BRUSH_SIZE,
            opacity: 1.0,
            settings: BrushSettings {
                spacing: redrob_core::SpacingOptions {
                    spacing: redrob_core::MIN_SPACING,
                    isotropic: false,
                },
                ..Default::default()
            },
            tip: None,
            pipe: Vec::new(),
        })
        .unwrap_err();

    assert!(matches!(
        error,
        CoreError::BrushWorkLimitExceeded {
            max_pixel_visits: MAX_BRUSH_PIXEL_VISITS
        }
    ));
    assert_eq!(editor.generation(), generation);
    assert_eq!(editor.render_snapshot().unwrap().pixels(), pixels);
}

#[test]
fn grouped_undo_redo_is_atomic_and_history_is_bounded() {
    let config = HistoryConfig {
        max_entries: 1,
        memory_budget_bytes: usize::MAX,
    };
    let mut editor = Editor::with_history_config(Document::new(2, 2).unwrap(), config).unwrap();
    let layer = editor.document().active_layer_id();
    assert!(!editor.is_group_active());
    editor.begin_group("layer properties").unwrap();
    assert!(editor.is_group_active());
    editor
        .execute(Command::RenameLayer {
            id: layer,
            name: "Renamed".into(),
        })
        .unwrap();
    editor
        .execute(Command::SetLayerVisibility {
            id: layer,
            visible: false,
        })
        .unwrap();
    editor.end_group().unwrap();
    assert!(!editor.is_group_active());
    editor.undo().unwrap();
    assert_eq!(editor.document().layer(layer).unwrap().name(), "Layer 1");
    assert!(editor.document().layer(layer).unwrap().is_visible());
    editor.redo().unwrap();
    assert_eq!(editor.document().layer(layer).unwrap().name(), "Renamed");
    assert!(!editor.document().layer(layer).unwrap().is_visible());

    editor
        .execute(Command::SetLayerVisibility {
            id: layer,
            visible: true,
        })
        .unwrap();
    assert!(!editor.can_redo());
    editor.undo().unwrap();
    assert!(!editor.can_undo(), "max_entries=1 must evict older groups");
}

#[test]
fn zero_memory_budget_disables_history_without_rejecting_edits() {
    let mut editor = Editor::with_history_config(
        Document::new(1, 1).unwrap(),
        HistoryConfig {
            max_entries: 1_024,
            memory_budget_bytes: 0,
        },
    )
    .unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(1, 2, 3, 4),
        })
        .unwrap();
    assert!(!editor.can_undo());
}

#[test]
fn project_and_png_roundtrip_preserve_expected_state() {
    let mut editor = Editor::new(Document::new(2, 1).unwrap()).unwrap();
    editor
        .execute(Command::SetMetadata {
            metadata: DocumentMetadata {
                title: "Roundtrip".into(),
                author: Some("Test Author".into()),
                properties: [("color_profile".into(), "sRGB".into())]
                    .into_iter()
                    .collect(),
            },
        })
        .unwrap();
    let first = editor.document().active_layer_id();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(12, 34, 56, 255),
        })
        .unwrap();
    let second = LayerId::new();
    editor
        .execute(Command::AddLayer {
            id: second,
            name: "Second".into(),
            index: 1,
        })
        .unwrap();
    editor
        .execute(Command::SelectRectangle {
            rect: Rect::new(1, 0, 1, 1),
            mode: SelectionMode::Replace,
        })
        .unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(200, 100, 50, 128),
        })
        .unwrap();

    let project = save_project(editor.document()).unwrap();
    let restored = load_project(&project).unwrap();
    assert_eq!(&restored, editor.document());
    assert_eq!(restored.layers()[0].id(), first);
    assert_eq!(restored.layers()[1].id(), second);
    assert!(restored.selection().is_active());

    let expected = editor.render_snapshot().unwrap().pixels().to_vec();
    let png = export_png(editor.document()).unwrap();
    let imported = import_png(&png).unwrap();
    assert_eq!(imported.width(), 2);
    assert_eq!(imported.height(), 1);
    assert_eq!(imported.layers()[0].pixels(), expected);
}

#[test]
fn malformed_project_dimensions_and_buffers_are_rejected() {
    let bytes = save_project(&Document::new(1, 1).unwrap()).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["document"]["width"] = serde_json::json!(u32::MAX);
    let malformed = serde_json::to_vec(&value).unwrap();
    assert!(matches!(
        load_project(&malformed),
        Err(CoreError::InvalidDimensions { .. })
    ));

    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["document"]["nodes"][0]["content"]["cels"][0]["pixels"] = serde_json::json!([0, 0, 0]);
    let malformed = serde_json::to_vec(&value).unwrap();
    assert!(matches!(
        load_project(&malformed),
        Err(CoreError::InvalidBufferLength { .. })
    ));
}

#[test]
fn grayscale_brightness_contrast_and_blur_execute() {
    let mut editor = Editor::new(Document::new(3, 1).unwrap()).unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(30, 90, 180, 255),
        })
        .unwrap();
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Grayscale,
        })
        .unwrap();
    let grayscale = editor.document().active_layer().pixel(3, 0, 0).unwrap();
    assert_eq!(grayscale.r, grayscale.g);
    assert_eq!(grayscale.g, grayscale.b);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::BrightnessContrast {
                brightness: 20,
                contrast: 10.0,
            },
        })
        .unwrap();
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::GaussianBlur { sigma: 1.0 },
        })
        .unwrap();
    assert_eq!(editor.document().active_layer().pixels().len(), 12);
}

#[test]
fn new_commands_roundtrip_and_legacy_brush_json_uses_defaults() {
    let commands = vec![
        Command::SelectEllipse {
            rect: Rect::new(-1, 2, 7, 9),
            mode: SelectionMode::Add,
        },
        Command::SelectAll,
        Command::InvertSelection,
        Command::FeatherSelection { radius: 3 },
        Command::GrowSelection { radius: 2 },
        Command::ShrinkSelection { radius: 1 },
        Command::GradientFill {
            kind: GradientKind::Radial {
                center_x: 1.0,
                center_y: 2.0,
                radius: 4.0,
            },
            stops: vec![
                GradientStop::new(0.0, Pixel::rgba(0, 0, 0, 255)),
                GradientStop::new(1.0, Pixel::rgba(255, 255, 255, 0)),
            ],
        },
        Command::CropCanvas {
            rect: Rect::new(-2, 1, 8, 6),
        },
        Command::ResizeCanvas {
            width: 9,
            height: 7,
            sampling: SamplingMode::Bilinear,
        },
        Command::FlipActive {
            horizontal: true,
            vertical: false,
        },
        Command::RotateActive90 { clockwise: true },
        Command::TransformActive {
            transform: Affine2D::IDENTITY,
            sampling: SamplingMode::Nearest,
        },
    ];
    for command in commands {
        let encoded = serde_json::to_string(&command).unwrap();
        assert_eq!(serde_json::from_str::<Command>(&encoded).unwrap(), command);
    }

    let legacy = r#"{
        "type":"brush_stroke",
        "points":[{"x":1.0,"y":2.0,"pressure":0.5}],
        "color":{"r":1,"g":2,"b":3,"a":4},
        "size":5.0,
        "opacity":0.75
    }"#;
    let decoded: Command = serde_json::from_str(legacy).unwrap();
    assert!(matches!(
        decoded,
        Command::BrushStroke {
            settings: BrushSettings {
                smoothing: BrushSmoothing::None,
                mirror_x: None,
                mirror_y: None,
                ..
            },
            ..
        }
    ));

    let old_changes = r#"{"generation":4,"document_changed":true,"structure_changed":false,"selection_changed":false,"changed_layers":[]}"#;
    let changes: redrob_core::ChangeSet = serde_json::from_str(old_changes).unwrap();
    assert!(!changes.canvas_changed);
}

#[test]
fn ellipse_select_all_invert_feather_grow_and_shrink_are_grayscale_and_bounded() {
    let mut editor = Editor::new(Document::new(7, 7).unwrap()).unwrap();
    editor
        .execute(Command::SelectEllipse {
            rect: Rect::new(1, 1, 5, 5),
            mode: SelectionMode::Replace,
        })
        .unwrap();
    let mask = editor.document().selection().mask();
    assert_eq!(mask[3 * 7 + 3], 255);
    assert_eq!(mask[0], 0);
    let edge = mask[7 + 1];
    assert!((1..255).contains(&edge));
    assert!(mask.iter().any(|value| (1..255).contains(value)));

    editor
        .execute(Command::SelectEllipse {
            rect: Rect::new(1, 1, 5, 5),
            mode: SelectionMode::Add,
        })
        .unwrap();
    assert_eq!(
        editor.document().selection().mask()[7 + 1],
        edge.saturating_mul(2)
    );
    editor
        .execute(Command::SelectEllipse {
            rect: Rect::new(1, 1, 5, 5),
            mode: SelectionMode::Subtract,
        })
        .unwrap();
    assert_eq!(editor.document().selection().mask()[7 + 1], edge);
    editor
        .execute(Command::SelectEllipse {
            rect: Rect::new(1, 1, 5, 5),
            mode: SelectionMode::Intersect,
        })
        .unwrap();
    assert_eq!(editor.document().selection().mask()[7 + 1], edge);

    editor.execute(Command::SelectAll).unwrap();
    assert!(editor.is_selection_active());
    assert!(
        editor
            .selection_mask_snapshot()
            .iter()
            .all(|&value| value == 255)
    );
    editor.execute(Command::InvertSelection).unwrap();
    assert!(
        editor
            .selection_mask_snapshot()
            .iter()
            .all(|&value| value == 0)
    );

    editor
        .execute(Command::SelectRectangle {
            rect: Rect::new(3, 3, 1, 1),
            mode: SelectionMode::Replace,
        })
        .unwrap();
    editor
        .execute(Command::FeatherSelection { radius: 1 })
        .unwrap();
    let feathered = editor.selection_mask_snapshot();
    assert!((1..255).contains(&feathered[3 * 7 + 3]));
    assert!((1..255).contains(&feathered[2 * 7 + 2]));
    let stable_snapshot = feathered.clone();

    editor
        .execute(Command::SelectRectangle {
            rect: Rect::new(3, 3, 1, 1),
            mode: SelectionMode::Replace,
        })
        .unwrap();
    editor
        .execute(Command::GrowSelection { radius: 1 })
        .unwrap();
    assert_eq!(editor.document().selection().coverage(2, 2), 255);
    assert_eq!(editor.document().selection().coverage(4, 4), 255);
    editor
        .execute(Command::ShrinkSelection { radius: 1 })
        .unwrap();
    assert_eq!(editor.document().selection().coverage(3, 3), 255);
    assert_eq!(editor.document().selection().coverage(2, 2), 0);

    let generation = editor.generation();
    assert!(matches!(
        editor.execute(Command::FeatherSelection { radius: 4_097 }),
        Err(CoreError::InvalidSelectionRadius(4_097))
    ));
    assert_eq!(editor.generation(), generation);
    assert_ne!(editor.selection_mask_snapshot(), stable_snapshot);
}

#[test]
fn linear_and_radial_gradients_clamp_interpolate_and_respect_selection() {
    let mut editor = Editor::new(Document::new(5, 1).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    editor
        .execute(Command::GradientFill {
            kind: GradientKind::Linear {
                start_x: 0.5,
                start_y: 0.5,
                end_x: 4.5,
                end_y: 0.5,
            },
            stops: vec![
                GradientStop::new(0.0, Pixel::rgba(0, 0, 0, 255)),
                GradientStop::new(0.5, Pixel::rgba(255, 0, 0, 255)),
                GradientStop::new(1.0, Pixel::rgba(255, 255, 255, 255)),
            ],
        })
        .unwrap();
    assert_eq!(pixel(&editor, layer, 0, 0), Pixel::rgba(0, 0, 0, 255));
    assert_eq!(pixel(&editor, layer, 2, 0), Pixel::rgba(255, 0, 0, 255));
    assert_eq!(pixel(&editor, layer, 4, 0), Pixel::rgba(255, 255, 255, 255));

    let mut radial = Editor::new(Document::new(5, 1).unwrap()).unwrap();
    let radial_layer = radial.document().active_layer_id();
    radial
        .execute(Command::SelectRectangle {
            rect: Rect::new(1, 0, 3, 1),
            mode: SelectionMode::Replace,
        })
        .unwrap();
    radial
        .execute(Command::GradientFill {
            kind: GradientKind::Radial {
                center_x: 2.5,
                center_y: 0.5,
                radius: 1.0,
            },
            stops: vec![
                GradientStop::new(0.0, Pixel::rgba(0, 0, 255, 255)),
                GradientStop::new(1.0, Pixel::rgba(255, 0, 0, 128)),
            ],
        })
        .unwrap();
    assert_eq!(pixel(&radial, radial_layer, 0, 0), Pixel::TRANSPARENT);
    assert_eq!(
        pixel(&radial, radial_layer, 2, 0),
        Pixel::rgba(0, 0, 255, 255)
    );
    assert_eq!(
        pixel(&radial, radial_layer, 1, 0),
        Pixel::rgba(255, 0, 0, 128)
    );

    let generation = radial.generation();
    for command in [
        Command::GradientFill {
            kind: GradientKind::Radial {
                center_x: 0.0,
                center_y: 0.0,
                radius: 0.0,
            },
            stops: vec![
                GradientStop::new(0.0, Pixel::TRANSPARENT),
                GradientStop::new(1.0, Pixel::TRANSPARENT),
            ],
        },
        Command::GradientFill {
            kind: GradientKind::Linear {
                start_x: 0.0,
                start_y: 0.0,
                end_x: 1.0,
                end_y: 0.0,
            },
            stops: vec![
                GradientStop::new(0.7, Pixel::TRANSPARENT),
                GradientStop::new(0.2, Pixel::TRANSPARENT),
            ],
        },
    ] {
        assert!(matches!(
            radial.execute(command),
            Err(CoreError::InvalidGradient)
        ));
    }
    assert_eq!(radial.generation(), generation);
}

fn paint_pixel(editor: &mut Editor, x: u32, y: u32, color: Pixel) {
    editor
        .execute(Command::SelectRectangle {
            rect: Rect::new(x as i32, y as i32, 1, 1),
            mode: SelectionMode::Replace,
        })
        .unwrap();
    editor.execute(Command::Fill { color }).unwrap();
}

#[test]
fn crop_extends_transparently_updates_all_layers_selection_and_undo() {
    let mut editor = Editor::new(Document::new(2, 2).unwrap()).unwrap();
    let bottom = editor.document().active_layer_id();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(200, 0, 0, 255),
        })
        .unwrap();
    let top = LayerId::new();
    editor
        .execute(Command::AddLayer {
            id: top,
            name: "Top".into(),
            index: 1,
        })
        .unwrap();
    paint_pixel(&mut editor, 1, 0, Pixel::rgba(0, 0, 200, 255));
    let changes = editor
        .execute(Command::CropCanvas {
            rect: Rect::new(-1, -1, 4, 4),
        })
        .unwrap();
    assert!(changes.canvas_changed);
    assert!(changes.selection_changed);
    assert_eq!(changes.changed_layers.len(), 2);
    assert_eq!(
        (editor.document().width(), editor.document().height()),
        (4, 4)
    );
    assert_eq!(pixel(&editor, bottom, 0, 0), Pixel::TRANSPARENT);
    assert_eq!(pixel(&editor, bottom, 1, 1), Pixel::rgba(200, 0, 0, 255));
    assert_eq!(pixel(&editor, top, 2, 1), Pixel::rgba(0, 0, 200, 255));
    assert_eq!(editor.document().selection().coverage(2, 1), 255);
    assert_eq!(editor.document().selection().coverage(1, 1), 0);

    let undo = editor.undo().unwrap();
    assert!(undo.canvas_changed);
    assert_eq!(
        (editor.document().width(), editor.document().height()),
        (2, 2)
    );
    editor.redo().unwrap();
    assert_eq!(
        (editor.document().width(), editor.document().height()),
        (4, 4)
    );
}

#[test]
fn resize_flip_rotate_and_affine_transform_have_deterministic_mapping() {
    let mut editor = Editor::new(Document::new(2, 2).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    paint_pixel(&mut editor, 0, 0, Pixel::rgba(10, 0, 0, 255));
    paint_pixel(&mut editor, 1, 0, Pixel::rgba(20, 0, 0, 255));
    paint_pixel(&mut editor, 0, 1, Pixel::rgba(30, 0, 0, 255));
    paint_pixel(&mut editor, 1, 1, Pixel::rgba(40, 0, 0, 255));
    editor.execute(Command::ClearSelection).unwrap();

    editor
        .execute(Command::RotateActive90 { clockwise: true })
        .unwrap();
    assert_eq!(pixel(&editor, layer, 0, 0).r, 30);
    assert_eq!(pixel(&editor, layer, 1, 0).r, 10);
    assert_eq!(pixel(&editor, layer, 0, 1).r, 40);
    assert_eq!(pixel(&editor, layer, 1, 1).r, 20);
    editor
        .execute(Command::FlipActive {
            horizontal: true,
            vertical: false,
        })
        .unwrap();
    assert_eq!(pixel(&editor, layer, 0, 0).r, 10);
    assert_eq!(pixel(&editor, layer, 1, 0).r, 30);

    editor
        .execute(Command::TransformActive {
            transform: Affine2D::new(1.0, 0.0, 0.0, 1.0, 1.0, 0.0),
            sampling: SamplingMode::Nearest,
        })
        .unwrap();
    assert_eq!(pixel(&editor, layer, 0, 0), Pixel::TRANSPARENT);
    assert_eq!(pixel(&editor, layer, 1, 0).r, 10);

    editor
        .execute(Command::ResizeCanvas {
            width: 4,
            height: 4,
            sampling: SamplingMode::Nearest,
        })
        .unwrap();
    assert_eq!(
        (editor.document().width(), editor.document().height()),
        (4, 4)
    );
    assert_eq!(pixel(&editor, layer, 2, 0).r, 10);

    let identity_pixels = editor.render_snapshot().unwrap().pixels().to_vec();
    editor
        .execute(Command::TransformActive {
            transform: Affine2D::IDENTITY,
            sampling: SamplingMode::Bilinear,
        })
        .unwrap();
    assert_eq!(editor.render_snapshot().unwrap().pixels(), identity_pixels);

    let before = editor.render_snapshot().unwrap().pixels().to_vec();
    let generation = editor.generation();
    for transform in [
        Affine2D::new(1.0, 2.0, 2.0, 4.0, 0.0, 0.0),
        Affine2D::new(f32::NAN, 0.0, 0.0, 1.0, 0.0, 0.0),
    ] {
        assert!(matches!(
            editor.execute(Command::TransformActive {
                transform,
                sampling: SamplingMode::Bilinear,
            }),
            Err(CoreError::InvalidTransform)
        ));
    }
    assert_eq!(editor.generation(), generation);
    assert_eq!(editor.render_snapshot().unwrap().pixels(), before);
    assert!(matches!(
        editor.execute(Command::ResizeCanvas {
            width: 0,
            height: 1,
            sampling: SamplingMode::Nearest,
        }),
        Err(CoreError::InvalidDimensions { .. })
    ));

    let mut finite = Editor::new(Document::new(1, 1).unwrap()).unwrap();
    for transform in [
        Affine2D::new(1.0e-4, 0.0, 0.0, 1.0e-4, 0.0, 0.0),
        Affine2D::new(f32::MAX, 0.0, 0.0, f32::MAX, 0.0, 0.0),
    ] {
        finite
            .execute(Command::TransformActive {
                transform,
                sampling: SamplingMode::Nearest,
            })
            .unwrap();
    }
}

#[test]
fn all_new_filters_execute_respect_selection_and_validate_strictly() {
    let mut editor = Editor::new(Document::new(3, 1).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(200, 100, 50, 255),
        })
        .unwrap();
    editor
        .execute(Command::SelectRectangle {
            rect: Rect::new(1, 0, 1, 1),
            mode: SelectionMode::Replace,
        })
        .unwrap();
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Threshold {
                threshold: 128,
                channel: redrob_core::HistogramChannel::Value,
            },
        })
        .unwrap();
    assert_eq!(pixel(&editor, layer, 0, 0), Pixel::rgba(200, 100, 50, 255));
    // K.16 changed this value, deliberately. The canvas is (200, 100, 50): under the old Rec. 709
    // luminance that measured 117.65 and fell below 128, giving black. Upstream's default channel is
    // `GIMP_HISTOGRAM_VALUE`, which is the MAXIMUM of red, green and blue -- 200, which clears 128 and
    // gives white. The filter was wrong before, not now.
    assert_eq!(pixel(&editor, layer, 1, 0), Pixel::rgba(255, 255, 255, 255));

    editor.execute(Command::ClearSelection).unwrap();
    for filter in [
        Filter::Posterize { levels: 4 },
        Filter::Levels {
            input_black: 0,
            input_white: 255,
            gamma: 1.2,
            output_black: 10,
            output_white: 240,
        },
        Filter::HueSaturation {
            hue_degrees: 120.0,
            saturation: 25.0,
            lightness: -10.0,
            hue_sectors: [0.0; 6],
            saturation_sectors: [0.0; 6],
            lightness_sectors: [0.0; 6],
            overlap: 0.0,
        },
        Filter::BoxBlur { radius: 1 },
        Filter::Sharpen { amount: 1.5 },
    ] {
        editor.execute(Command::ApplyFilter { filter }).unwrap();
    }
    assert_eq!(editor.document().active_layer().pixels().len(), 12);

    let before = editor.document().active_layer().pixels().to_vec();
    let generation = editor.generation();
    let invalid = [
        Filter::Posterize { levels: 1 },
        Filter::Levels {
            input_black: 100,
            input_white: 100,
            gamma: 1.0,
            output_black: 0,
            output_white: 255,
        },
        Filter::Levels {
            input_black: 0,
            input_white: 255,
            gamma: f32::NAN,
            output_black: 0,
            output_white: 255,
        },
        Filter::HueSaturation {
            hue_degrees: 181.0,
            saturation: 0.0,
            lightness: 0.0,
            hue_sectors: [0.0; 6],
            saturation_sectors: [0.0; 6],
            lightness_sectors: [0.0; 6],
            overlap: 0.0,
        },
        Filter::BoxBlur { radius: 0 },
        Filter::Sharpen {
            amount: f32::INFINITY,
        },
    ];
    for filter in invalid {
        assert!(matches!(
            editor.execute(Command::ApplyFilter { filter }),
            Err(CoreError::InvalidFilterParameter)
        ));
    }
    assert_eq!(editor.generation(), generation);
    assert_eq!(editor.document().active_layer().pixels(), before);
}

#[test]
fn brush_smoothing_and_mirror_settings_are_deterministic_and_validated() {
    let command = Command::BrushStroke {
        points: vec![
            BrushPoint::new(2.5, 1.5, 1.0),
            BrushPoint::new(3.5, 2.5, 0.5),
            BrushPoint::new(4.5, 3.5, 1.0),
        ],
        color: Pixel::rgba(20, 200, 40, 255),
        size: 1.0,
        opacity: 1.0,
        settings: BrushSettings {
            smoothing: BrushSmoothing::MovingAverage { window: 2 },
            mirror_x: Some(5.5),
            mirror_y: Some(2.5),
            ..Default::default()
        },
        tip: None,
        pipe: Vec::new(),
    };
    let mut first = Editor::new(Document::new(11, 5).unwrap()).unwrap();
    let mut second = Editor::new(Document::new(11, 5).unwrap()).unwrap();
    first.execute(command.clone()).unwrap();
    second.execute(command).unwrap();
    assert_eq!(
        first.render_snapshot().unwrap().pixels(),
        second.render_snapshot().unwrap().pixels()
    );
    let layer = first.document().active_layer_id();
    assert!(pixel(&first, layer, 2, 1).a > 0);
    assert!(pixel(&first, layer, 8, 1).a > 0);
    assert!(pixel(&first, layer, 2, 3).a > 0);
    assert!(pixel(&first, layer, 8, 3).a > 0);

    first
        .execute(Command::BrushStroke {
            points: vec![BrushPoint::new(f32::MAX, 1.0, 1.0)],
            color: Pixel::rgba(0, 0, 0, 255),
            size: 1.0,
            opacity: 1.0,
            settings: BrushSettings {
                mirror_x: Some(f32::MAX),
                ..BrushSettings::default()
            },
            tip: None,
            pipe: Vec::new(),
        })
        .unwrap();

    let generation = first.generation();
    for settings in [
        BrushSettings {
            smoothing: BrushSmoothing::MovingAverage { window: 1 },
            ..BrushSettings::default()
        },
        BrushSettings {
            mirror_x: Some(f32::NAN),
            ..BrushSettings::default()
        },
    ] {
        assert!(matches!(
            first.execute(Command::BrushStroke {
                points: vec![BrushPoint::new(1.0, 1.0, 1.0)],
                color: Pixel::rgba(0, 0, 0, 255),
                size: 1.0,
                opacity: 1.0,
                settings,
                tip: None,
                pipe: Vec::new(),
            }),
            Err(CoreError::InvalidBrushSettings)
        ));
    }
    assert_eq!(first.generation(), generation);
}

#[test]
fn crop_resize_project_roundtrip_preserves_new_document_state() {
    let mut editor = Editor::new(Document::new(3, 2).unwrap()).unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(11, 22, 33, 200),
        })
        .unwrap();
    editor
        .execute(Command::SelectEllipse {
            rect: Rect::new(0, 0, 3, 2),
            mode: SelectionMode::Replace,
        })
        .unwrap();
    editor
        .execute(Command::CropCanvas {
            rect: Rect::new(-1, 0, 5, 3),
        })
        .unwrap();
    editor
        .execute(Command::ResizeCanvas {
            width: 7,
            height: 4,
            sampling: SamplingMode::Bilinear,
        })
        .unwrap();
    let bytes = save_project(editor.document()).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["version"], 2);
    let restored = load_project(&bytes).unwrap();
    assert_eq!(restored, *editor.document());
    assert_eq!((restored.width(), restored.height()), (7, 4));
}

#[test]
fn small_dimension_parameter_sweep_is_deterministic() {
    let mut state = 0x1357_9bdf_u32;
    for case in 0..24_u32 {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let width = state % 7 + 1;
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let height = state % 7 + 1;
        let document = Document::new(width, height).unwrap();
        let mut first = Editor::new(document.clone()).unwrap();
        let mut second = Editor::new(document).unwrap();
        let commands = [
            Command::SelectEllipse {
                rect: Rect::new(-1, 0, width + 1, height),
                mode: SelectionMode::Replace,
            },
            Command::GrowSelection { radius: case % 3 },
            Command::FeatherSelection { radius: case % 2 },
            Command::GradientFill {
                kind: GradientKind::Linear {
                    start_x: 0.0,
                    start_y: 0.0,
                    end_x: width as f32 + 1.0,
                    end_y: height as f32 + 1.0,
                },
                stops: vec![
                    GradientStop::new(0.0, Pixel::rgba(3, 5, 7, 255)),
                    GradientStop::new(1.0, Pixel::rgba(251, 127, 63, 192)),
                ],
            },
            Command::ResizeCanvas {
                width: height + 1,
                height: width + 1,
                sampling: if case % 2 == 0 {
                    SamplingMode::Nearest
                } else {
                    SamplingMode::Bilinear
                },
            },
            Command::ApplyFilter {
                filter: Filter::Posterize {
                    levels: (case % 7 + 2) as u16,
                },
            },
        ];
        for command in commands {
            first.execute(command.clone()).unwrap();
            second.execute(command).unwrap();
        }
        assert_eq!(first.document(), second.document());
        assert_eq!(
            first.render_snapshot().unwrap().pixels(),
            second.render_snapshot().unwrap().pixels()
        );
        assert_eq!(
            save_project(first.document()).unwrap(),
            save_project(second.document()).unwrap()
        );
    }
}

#[test]
fn new_filter_channel_math_has_expected_reference_outputs() {
    let apply_to = |color: Pixel, filter: Filter| {
        let mut editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
        editor.execute(Command::Fill { color }).unwrap();
        editor.execute(Command::ApplyFilter { filter }).unwrap();
        editor.document().active_layer().pixel(1, 0, 0).unwrap()
    };
    assert_eq!(
        apply_to(
            Pixel::rgba(100, 170, 250, 123),
            Filter::Posterize { levels: 2 }
        ),
        Pixel::rgba(0, 255, 255, 123)
    );
    assert_eq!(
        apply_to(
            Pixel::rgba(128, 128, 128, 255),
            Filter::Levels {
                input_black: 0,
                input_white: 255,
                gamma: 1.0,
                output_black: 10,
                output_white: 110,
            },
        ),
        Pixel::rgba(60, 60, 60, 255)
    );
    let shifted = apply_to(
        Pixel::rgba(255, 0, 0, 255),
        Filter::HueSaturation {
            hue_degrees: 120.0,
            saturation: 0.0,
            lightness: 0.0,
            hue_sectors: [0.0; 6],
            saturation_sectors: [0.0; 6],
            lightness_sectors: [0.0; 6],
            overlap: 0.0,
        },
    );
    // 120 degrees of MASTER hue rotates 60, not 120 -- and that is upstream's arithmetic, not a
    // regression. `map_hue` in app/operations/gimpoperationhuesaturation.c:93 reads
    //     value += (config->hue[GIMP_HUE_RANGE_ALL] + config->hue[range]) / 2.0f;
    // so the master value is AVERAGED with the per-sector value rather than added to it. The
    // divisor exists so that setting both the master and a sector does not double-count; with the
    // sector at zero it halves the master.
    //
    // This assertion previously read `(0, 255, 0)` and pinned our own approximation, which applied
    // the master shift in full. Cycle 44 replaced that approximation with the ported arithmetic,
    // so the reference output moved. Verified against the source rather than adjusted until it
    // passed: `map_hue` is called exactly once per pixel (line 291).
    assert_eq!(shifted, Pixel::rgba(255, 255, 0, 255));

    let mut blurred = Editor::new(Document::new(3, 1).unwrap()).unwrap();
    paint_pixel(&mut blurred, 1, 0, Pixel::rgba(255, 255, 255, 255));
    blurred.execute(Command::ClearSelection).unwrap();
    blurred
        .execute(Command::ApplyFilter {
            filter: Filter::BoxBlur { radius: 1 },
        })
        .unwrap();
    let layer = blurred.document().active_layer_id();
    assert_eq!(pixel(&blurred, layer, 1, 0), Pixel::rgba(255, 255, 255, 85));

    let mut sharpened = Editor::new(Document::new(3, 1).unwrap()).unwrap();
    sharpened
        .execute(Command::Fill {
            color: Pixel::rgba(100, 100, 100, 255),
        })
        .unwrap();
    paint_pixel(&mut sharpened, 1, 0, Pixel::rgba(150, 150, 150, 255));
    sharpened.execute(Command::ClearSelection).unwrap();
    let before = pixel(&sharpened, sharpened.document().active_layer_id(), 1, 0).r;
    sharpened
        .execute(Command::ApplyFilter {
            filter: Filter::Sharpen { amount: 1.0 },
        })
        .unwrap();
    assert!(pixel(&sharpened, sharpened.document().active_layer_id(), 1, 0).r > before);
}

#[test]
fn literal_v1_project_remains_load_compatible() {
    let fixture = br#"{
        "magic":"REDROB_CANVAS_PROJECT",
        "version":1,
        "document":{
            "id":"00000000-0000-0000-0000-000000000001",
            "width":1,
            "height":1,
            "metadata":{"title":"v1","author":null,"properties":{}},
            "layers":[{
                "id":"00000000-0000-0000-0000-000000000002",
                "name":"Layer 1",
                "visible":true,
                "opacity":1.0,
                "blend_mode":"normal",
                "pixels":[1,2,3,4]
            }],
            "active_layer":"00000000-0000-0000-0000-000000000002",
            "selection":{"width":1,"height":1,"active":false,"mask":[0]}
        }
    }"#;
    let document = load_project(fixture).unwrap();
    assert_eq!((document.width(), document.height()), (1, 1));
    assert_eq!(document.layers()[0].pixels(), [1, 2, 3, 4]);
    assert!(!document.is_selection_active());
    let saved: serde_json::Value =
        serde_json::from_slice(&save_project(&document).unwrap()).unwrap();
    assert_eq!(saved["version"], 2);
}

#[test]
fn every_new_filter_preserves_pixels_outside_selection() {
    let filters = [
        Filter::Threshold {
            threshold: 100,
            channel: redrob_core::HistogramChannel::Value,
        },
        Filter::Posterize { levels: 3 },
        Filter::Levels {
            input_black: 10,
            input_white: 240,
            gamma: 1.5,
            output_black: 5,
            output_white: 250,
        },
        Filter::HueSaturation {
            hue_degrees: -45.0,
            saturation: 50.0,
            lightness: 20.0,
            hue_sectors: [0.0; 6],
            saturation_sectors: [0.0; 6],
            lightness_sectors: [0.0; 6],
            overlap: 0.0,
        },
        Filter::BoxBlur { radius: 2 },
        Filter::Sharpen { amount: 2.0 },
    ];
    for filter in filters {
        let mut editor = Editor::new(Document::new(5, 1).unwrap()).unwrap();
        editor
            .execute(Command::Fill {
                color: Pixel::rgba(80, 120, 160, 255),
            })
            .unwrap();
        paint_pixel(&mut editor, 2, 0, Pixel::rgba(220, 30, 60, 255));
        editor
            .execute(Command::SelectRectangle {
                rect: Rect::new(2, 0, 1, 1),
                mode: SelectionMode::Replace,
            })
            .unwrap();
        let layer = editor.document().active_layer_id();
        let left = pixel(&editor, layer, 1, 0);
        let right = pixel(&editor, layer, 3, 0);
        editor.execute(Command::ApplyFilter { filter }).unwrap();
        assert_eq!(pixel(&editor, layer, 1, 0), left);
        assert_eq!(pixel(&editor, layer, 3, 0), right);
    }
}

#[test]
fn grouped_new_operations_undo_and_redo_as_one_atomic_snapshot() {
    let mut editor = Editor::new(Document::new(4, 4).unwrap()).unwrap();
    let before = editor.document().clone();
    editor.begin_group("parity slice").unwrap();
    editor
        .execute(Command::SelectEllipse {
            rect: Rect::new(0, 0, 4, 4),
            mode: SelectionMode::Replace,
        })
        .unwrap();
    editor
        .execute(Command::GrowSelection { radius: 1 })
        .unwrap();
    editor
        .execute(Command::GradientFill {
            kind: GradientKind::Linear {
                start_x: 0.0,
                start_y: 0.0,
                end_x: 4.0,
                end_y: 4.0,
            },
            stops: vec![
                GradientStop::new(0.0, Pixel::rgba(0, 0, 0, 255)),
                GradientStop::new(1.0, Pixel::rgba(255, 128, 64, 255)),
            ],
        })
        .unwrap();
    editor
        .execute(Command::BrushStroke {
            points: vec![
                BrushPoint::new(1.0, 1.0, 1.0),
                BrushPoint::new(3.0, 3.0, 1.0),
            ],
            color: Pixel::rgba(10, 240, 30, 255),
            size: 2.0,
            opacity: 0.5,
            settings: BrushSettings {
                smoothing: BrushSmoothing::MovingAverage { window: 2 },
                mirror_x: Some(2.0),
                mirror_y: None,
                ..Default::default()
            },
            tip: None,
            pipe: Vec::new(),
        })
        .unwrap();
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Posterize { levels: 5 },
        })
        .unwrap();
    editor
        .execute(Command::FlipActive {
            horizontal: true,
            vertical: false,
        })
        .unwrap();
    editor
        .execute(Command::RotateActive90 { clockwise: false })
        .unwrap();
    editor
        .execute(Command::TransformActive {
            transform: Affine2D::IDENTITY,
            sampling: SamplingMode::Bilinear,
        })
        .unwrap();
    editor
        .execute(Command::ResizeCanvas {
            width: 5,
            height: 3,
            sampling: SamplingMode::Bilinear,
        })
        .unwrap();
    editor.end_group().unwrap();
    let after = editor.document().clone();
    assert_ne!(after, before);

    let undo = editor.undo().unwrap();
    assert!(undo.canvas_changed);
    assert!(undo.selection_changed);
    assert_eq!(editor.document(), &before);
    let redo = editor.redo().unwrap();
    assert!(redo.canvas_changed);
    assert_eq!(editor.document(), &after);
}

fn default_v2_value() -> serde_json::Value {
    serde_json::from_slice(&save_project(&Document::new(1, 1).unwrap()).unwrap()).unwrap()
}

#[test]
fn v1_migrates_to_default_timeline_and_v2_raster_cel() {
    let fixture = br#"{
        "magic":"REDROB_CANVAS_PROJECT","version":1,
        "document":{
            "id":"00000000-0000-0000-0000-000000000011","width":1,"height":1,
            "metadata":{"title":"migration","author":null,"properties":{}},
            "layers":[{"id":"00000000-0000-0000-0000-000000000012","name":"Ink",
                "visible":true,"opacity":0.5,"blend_mode":"multiply","pixels":[9,8,7,6]}],
            "active_layer":"00000000-0000-0000-0000-000000000012",
            "selection":{"width":1,"height":1,"active":true,"mask":[255]}
        }
    }"#;
    let document = load_project(fixture).unwrap();
    assert_eq!(document.current_frame_id(), FrameId::DEFAULT);
    assert_eq!(document.timeline().frames().len(), 1);
    assert_eq!(document.nodes()[0].parent_id(), None);
    assert_eq!(document.nodes()[0].mask(), None);
    assert_eq!(document.stored_raster_bytes(), 5);
    assert_eq!(
        document.nodes()[0].raster_pixels(FrameId::DEFAULT).unwrap(),
        [9, 8, 7, 6]
    );
    let saved: serde_json::Value =
        serde_json::from_slice(&save_project(&document).unwrap()).unwrap();
    assert_eq!(saved["version"], 2);
    assert!(saved["document"]["nodes"].is_array());
    assert_eq!(saved["document"]["nodes"][0]["content"]["kind"], "raster");
}

#[test]
fn v2_roundtrip_and_cow_raster_storage_are_deterministic() {
    let document = Document::new(2, 2).unwrap();
    let clone = document.clone();
    let left = document.nodes()[0].raster_cels().unwrap()[0].storage();
    let right = clone.nodes()[0].raster_cels().unwrap()[0].storage();
    assert!(left.shares_storage_with(right));

    let mut editor = Editor::new(clone).unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(1, 2, 3, 4),
        })
        .unwrap();
    let changed = editor.document().nodes()[0].raster_cels().unwrap()[0].storage();
    assert!(!left.shares_storage_with(changed));
    assert_eq!(document.nodes()[0].pixels(), [0; 16]);

    let first = save_project(editor.document()).unwrap();
    let restored = load_project(&first).unwrap();
    let second = save_project(&restored).unwrap();
    assert_eq!(first, second);
    assert_eq!(restored, *editor.document());
}

#[test]
fn unsupported_version_is_rejected_before_typed_document_decode() {
    let bytes = br#"{"magic":"REDROB_CANVAS_PROJECT","version":99,"document":"not a document"}"#;
    assert!(matches!(
        load_project(bytes),
        Err(CoreError::UnsupportedProjectVersion(99))
    ));
}

#[test]
fn malformed_hierarchy_masks_and_node_limits_are_rejected() {
    let mut missing_parent = default_v2_value();
    missing_parent["document"]["nodes"][0]["parent"] =
        serde_json::json!("00000000-0000-0000-0000-00000000ffff");
    assert!(matches!(
        load_project(&serde_json::to_vec(&missing_parent).unwrap()),
        Err(CoreError::MalformedProject(_))
    ));

    let mut cycle = default_v2_value();
    let id = cycle["document"]["nodes"][0]["id"].clone();
    cycle["document"]["nodes"][0]["content"] = serde_json::json!({"kind":"group"});
    cycle["document"]["nodes"][0]["parent"] = id;
    assert!(matches!(
        load_project(&serde_json::to_vec(&cycle).unwrap()),
        Err(CoreError::MalformedProject(message)) if message.contains("cycle")
    ));

    let mut too_deep = default_v2_value();
    let mut chain = Vec::new();
    for index in 0..=MAX_HIERARCHY_DEPTH + 1 {
        let id = format!("00000000-0000-0000-0000-{index:012x}");
        let parent = (index != 0)
            .then(|| format!("00000000-0000-0000-0000-{:012x}", index.saturating_sub(1)));
        chain.push(serde_json::json!({
            "id":id,
            "parent":parent,
            "name":format!("Group {index}"),
            "visible":true,
            "opacity":1.0,
            "blend_mode":"normal",
            "mask":null,
            "content":{"kind":"group"}
        }));
    }
    too_deep["document"]["active_node"] = chain[0]["id"].clone();
    too_deep["document"]["nodes"] = serde_json::Value::Array(chain);
    assert!(matches!(
        load_project(&serde_json::to_vec(&too_deep).unwrap()),
        Err(CoreError::DocumentLimitExceeded("hierarchy depth"))
    ));

    let mut invalid_mask = default_v2_value();
    invalid_mask["document"]["nodes"][0]["mask"] = serde_json::json!({"pixels":[1,2]});
    assert!(matches!(
        load_project(&serde_json::to_vec(&invalid_mask).unwrap()),
        Err(CoreError::InvalidBufferLength {
            expected: 1,
            actual: 2
        })
    ));

    let mut too_many = default_v2_value();
    let node = too_many["document"]["nodes"][0].clone();
    too_many["document"]["nodes"] =
        serde_json::Value::Array(vec![node; MAX_NODES.saturating_add(1)]);
    assert!(matches!(
        load_project(&serde_json::to_vec(&too_many).unwrap()),
        Err(CoreError::Json(error)) if error.to_string().contains("node count exceeds 4096")
    ));
}

#[test]
fn malformed_timeline_and_cels_are_rejected() {
    for mutate in 0..4 {
        let mut value = default_v2_value();
        match mutate {
            0 => value["document"]["timeline"]["fps"] = serde_json::json!(0),
            1 => value["document"]["timeline"]["current_frame"] = serde_json::json!(12),
            2 => value["document"]["timeline"]["frames"][0]["duration_ms"] = serde_json::json!(0),
            3 => value["document"]["timeline"]["playback"]["range_end"] = serde_json::json!(12),
            _ => unreachable!(),
        }
        assert!(matches!(
            load_project(&serde_json::to_vec(&value).unwrap()),
            Err(CoreError::MalformedProject(_))
        ));
    }

    let mut mismatched_duration = default_v2_value();
    mismatched_duration["document"]["timeline"]["fps"] = serde_json::json!(24.0);
    mismatched_duration["document"]["timeline"]["frames"][0]["duration_ms"] =
        serde_json::json!(100);
    assert!(matches!(
        load_project(&serde_json::to_vec(&mismatched_duration).unwrap()),
        Err(CoreError::MalformedProject(message)) if message.contains("disagrees")
    ));

    let mut too_many_frames = default_v2_value();
    too_many_frames["document"]["timeline"]["frames"] = serde_json::Value::Array(vec![
        serde_json::json!({"id":0,"duration_ms":100});
        MAX_FRAMES
            + 1
    ]);
    assert!(matches!(
        load_project(&serde_json::to_vec(&too_many_frames).unwrap()),
        Err(CoreError::DocumentLimitExceeded("frame count"))
    ));

    let mut unknown_frame = default_v2_value();
    unknown_frame["document"]["nodes"][0]["content"]["cels"][0]["frame"] = serde_json::json!(12);
    assert!(matches!(
        load_project(&serde_json::to_vec(&unknown_frame).unwrap()),
        Err(CoreError::MalformedProject(_))
    ));

    let mut duplicate_cel = default_v2_value();
    let cel = duplicate_cel["document"]["nodes"][0]["content"]["cels"][0].clone();
    duplicate_cel["document"]["nodes"][0]["content"]["cels"] =
        serde_json::json!([cel.clone(), cel]);
    assert!(matches!(
        load_project(&serde_json::to_vec(&duplicate_cel).unwrap()),
        Err(CoreError::DocumentLimitExceeded("raster cel count"))
            | Err(CoreError::MalformedProject(_))
    ));

    let mut bad_length = default_v2_value();
    bad_length["document"]["nodes"][0]["content"]["cels"][0]["pixels"] =
        serde_json::json!([1, 2, 3]);
    assert!(matches!(
        load_project(&serde_json::to_vec(&bad_length).unwrap()),
        Err(CoreError::InvalidBufferLength {
            expected: 4,
            actual: 3
        })
    ));
}

#[test]
fn semantic_complexity_and_active_raster_operations_fail_safely() {
    let mut oversized = default_v2_value();
    oversized["document"]["nodes"][0]["content"] = serde_json::json!({
        "kind":"text",
        "text":{
            "text":"x".repeat(MAX_TEXT_CONTENT_BYTES + 1),
            "font_family":"Sans",
            "font_size":12.0,
            "color":{"r":0,"g":0,"b":0,"a":255}
        }
    });
    assert!(matches!(
        load_project(&serde_json::to_vec(&oversized).unwrap()),
        Err(CoreError::Json(_))
    ));

    let mut too_many_paths = default_v2_value();
    too_many_paths["document"]["nodes"][0]["content"] = serde_json::json!({
        "kind":"vector",
        "vector":{"paths":vec![serde_json::json!({"commands":[]}); MAX_VECTOR_PATHS + 1]}
    });
    assert!(matches!(
        load_project(&serde_json::to_vec(&too_many_paths).unwrap()),
        Err(CoreError::Json(_))
    ));

    let mut semantic = default_v2_value();
    semantic["document"]["nodes"][0]["content"] = serde_json::json!({
        "kind":"text",
        "text":{
            "text":"hello",
            "font_family":"Sans",
            "font_size":12.0,
            "color":{"r":0,"g":0,"b":0,"a":255}
        }
    });
    let document = load_project(&serde_json::to_vec(&semantic).unwrap()).unwrap();
    let before = document.clone();
    let mut editor = Editor::new(document).unwrap();
    assert!(matches!(
        editor.execute(Command::Clear),
        Err(CoreError::UnsupportedNodeContent(NodeKind::Text))
    ));
    assert_eq!(editor.generation(), 0);
    assert_eq!(*editor.document(), before);
    let render = editor.try_render_snapshot().unwrap();
    assert!(render.pixels().iter().any(|byte| *byte != 0));
    assert!(!export_png(editor.document()).unwrap().is_empty());
}

#[test]
fn raster_commands_target_only_the_current_frame_cel() {
    let mut value = default_v2_value();
    value["document"]["timeline"]["frames"] = serde_json::json!([
        {"id":0,"duration_ms":100},
        {"id":1,"duration_ms":100}
    ]);
    value["document"]["timeline"]["current_frame"] = serde_json::json!(1);
    value["document"]["timeline"]["playback"]["range_end"] = serde_json::json!(1);
    value["document"]["nodes"][0]["content"]["cels"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"frame":1,"pixels":[10,20,30,255]}));
    let document = load_project(&serde_json::to_vec(&value).unwrap()).unwrap();
    let mut editor = Editor::new(document).unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(200, 100, 50, 255),
        })
        .unwrap();
    let node = &editor.document().nodes()[0];
    assert_eq!(node.raster_pixels(FrameId::DEFAULT).unwrap(), [0, 0, 0, 0]);
    assert_eq!(
        node.raster_pixels(FrameId::new(1)).unwrap(),
        [200, 100, 50, 255]
    );
    assert_eq!(
        editor.try_render_snapshot().unwrap().pixels(),
        [200, 100, 50, 255]
    );
}

#[test]
fn render_work_budget_accepts_exact_limit_and_rejects_valid_wide_tree() {
    let mut editor = Editor::new(Document::new(1_024, 1_024).unwrap()).unwrap();
    for index in 0..127 {
        editor
            .execute(Command::AddGroup {
                id: LayerId::new(),
                name: format!("Wide group {index}"),
                parent: None,
                sibling_index: index + 1,
            })
            .unwrap();
    }

    // One output clear, one raster composite, and two visits per group land
    // exactly on the exported aggregate budget.
    assert_eq!(MAX_RENDER_PIXEL_VISITS, 256 * 1_024 * 1_024);
    assert!(editor.try_render_snapshot().is_ok());

    let before = editor.document().clone();
    let generation = editor.generation();
    let error = editor
        .execute(Command::AddGroup {
            id: LayerId::new(),
            name: "First over budget".into(),
            parent: None,
            sibling_index: 128,
        })
        .unwrap_err();
    assert!(matches!(
        error,
        CoreError::RenderWorkLimitExceeded { max_pixel_visits }
            if max_pixel_visits == MAX_RENDER_PIXEL_VISITS
    ));
    assert_eq!(editor.generation(), generation);
    assert_eq!(editor.document(), &before);
    assert!(editor.try_render_snapshot().is_ok());
}

#[test]
fn resize_preflights_aggregate_cel_bytes_before_allocating() {
    let mut value = default_v2_value();
    value["document"]["timeline"]["frames"] = serde_json::Value::Array(
        (0..5)
            .map(|id| serde_json::json!({"id":id,"duration_ms":100}))
            .collect(),
    );
    value["document"]["timeline"]["playback"]["range_end"] = serde_json::json!(4);
    value["document"]["nodes"][0]["content"]["cels"] = serde_json::Value::Array(
        (0..5)
            .map(|frame| serde_json::json!({"frame":frame,"pixels":[0,0,0,0]}))
            .collect(),
    );
    let document = load_project(&serde_json::to_vec(&value).unwrap()).unwrap();
    let before = document.clone();
    let mut editor = Editor::new(document).unwrap();
    assert!(matches!(
        editor.execute(Command::ResizeCanvas {
            width: 8_192,
            height: 8_192,
            sampling: SamplingMode::Nearest,
        }),
        Err(CoreError::DocumentLimitExceeded("stored raster bytes"))
    ));
    assert_eq!(editor.generation(), 0);
    assert_eq!(*editor.document(), before);
}

#[test]
fn semantic_complexity_limits_are_document_wide() {
    let mut value = default_v2_value();
    let text = "x".repeat(MAX_TEXT_CONTENT_BYTES);
    value["document"]["nodes"][0]["content"] = serde_json::json!({
        "kind":"text",
        "text":{
            "text":text,
            "font_family":"Sans",
            "font_size":12.0,
            "color":{"r":0,"g":0,"b":0,"a":255}
        }
    });
    let original = value["document"]["nodes"][0].clone();
    for suffix in 1..4 {
        let mut node = original.clone();
        node["id"] = serde_json::json!(format!("00000000-0000-0000-0000-{suffix:012}"));
        value["document"]["nodes"]
            .as_array_mut()
            .unwrap()
            .push(node);
    }
    assert!(matches!(
        load_project(&serde_json::to_vec(&value).unwrap()),
        Err(CoreError::Json(error)) if error.to_string().contains("text complexity")
    ));
}

#[test]
fn hierarchy_moves_cycles_deletion_and_active_fallback_are_transactional() {
    let mut editor = Editor::new(Document::new(2, 1).unwrap()).unwrap();
    let raster = editor.document().active_layer_id();
    let outer = LayerId::new();
    let inner = LayerId::new();
    editor
        .execute(Command::AddGroup {
            id: outer,
            name: "Outer".into(),
            parent: None,
            sibling_index: 1,
        })
        .unwrap();
    editor
        .execute(Command::AddGroup {
            id: inner,
            name: "Inner".into(),
            parent: Some(outer),
            sibling_index: 0,
        })
        .unwrap();
    editor
        .execute(Command::MoveNode {
            id: raster,
            parent: Some(inner),
            sibling_index: 0,
        })
        .unwrap();
    assert_eq!(editor.document().node_depth(raster), Some(2));
    assert_eq!(editor.document().nodes().last().unwrap().id(), outer);

    let before = editor.document().clone();
    let generation = editor.generation();
    assert!(matches!(
        editor.execute(Command::MoveNode {
            id: outer,
            parent: Some(inner),
            sibling_index: 0,
        }),
        Err(CoreError::HierarchyCycle { .. })
    ));
    assert_eq!(editor.generation(), generation);
    assert_eq!(editor.document(), &before);
    assert!(matches!(
        editor.execute(Command::RemoveLayer { id: outer }),
        Err(CoreError::NonEmptyGroup(id)) if id == outer
    ));

    editor
        .execute(Command::MoveNode {
            id: raster,
            parent: None,
            sibling_index: 0,
        })
        .unwrap();
    editor.execute(Command::RemoveLayer { id: inner }).unwrap();
    assert_eq!(editor.document().active_layer_id(), outer);
    editor.execute(Command::RemoveLayer { id: outer }).unwrap();
    assert_eq!(editor.document().active_layer_id(), raster);
    assert!(
        editor
            .execute(Command::Fill {
                color: Pixel::rgba(1, 2, 3, 255),
            })
            .is_ok()
    );
}

#[test]
fn same_parent_moves_accept_exact_top_and_bottom_boundaries_and_reject_one_over() {
    let mut editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
    let first = editor.document().active_layer_id();
    let second = LayerId::new();
    let third = LayerId::new();
    editor
        .execute(Command::AddLayer {
            id: second,
            name: "Second".into(),
            index: 1,
        })
        .unwrap();
    editor
        .execute(Command::AddLayer {
            id: third,
            name: "Third".into(),
            index: 2,
        })
        .unwrap();

    editor
        .execute(Command::MoveNode {
            id: first,
            parent: None,
            sibling_index: 2,
        })
        .unwrap();
    assert_eq!(
        editor
            .document()
            .nodes()
            .iter()
            .map(|node| node.id())
            .collect::<Vec<_>>(),
        [second, third, first]
    );
    editor
        .execute(Command::MoveNode {
            id: first,
            parent: None,
            sibling_index: 0,
        })
        .unwrap();
    assert_eq!(
        editor
            .document()
            .nodes()
            .iter()
            .map(|node| node.id())
            .collect::<Vec<_>>(),
        [first, second, third]
    );

    let before = editor.document().clone();
    let generation = editor.generation();
    assert!(matches!(
        editor.execute(Command::MoveNode {
            id: first,
            parent: None,
            sibling_index: 3,
        }),
        Err(CoreError::SiblingIndexOutOfBounds { index: 3, len: 2 })
    ));
    assert_eq!(editor.generation(), generation);
    assert_eq!(editor.document(), &before);
}

#[test]
fn exact_maximum_group_depth_executes_and_renders_while_one_over_is_rejected() {
    let mut editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
    let mut parent = None;
    for depth in 0..=MAX_HIERARCHY_DEPTH {
        let id = LayerId::new();
        editor
            .execute(Command::AddGroup {
                id,
                name: format!("Depth {depth}"),
                parent,
                sibling_index: if parent.is_none() { 1 } else { 0 },
            })
            .unwrap();
        parent = Some(id);
    }
    assert_eq!(
        editor.document().node_depth(parent.unwrap()),
        Some(MAX_HIERARCHY_DEPTH)
    );
    assert!(editor.try_render_snapshot().is_ok());

    let before = editor.document().clone();
    let generation = editor.generation();
    assert!(matches!(
        editor.execute(Command::AddGroup {
            id: LayerId::new(),
            name: "Too deep".into(),
            parent,
            sibling_index: 0,
        }),
        Err(CoreError::DocumentLimitExceeded("hierarchy depth"))
    ));
    assert_eq!(editor.generation(), generation);
    assert_eq!(editor.document(), &before);
}

#[test]
fn group_raster_editing_is_typed_and_group_compositing_masks_exactly_once() {
    let mut editor = Editor::new(Document::new(2, 1).unwrap()).unwrap();
    let bottom = editor.document().active_layer_id();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(0, 0, 255, 255),
        })
        .unwrap();
    let group = LayerId::new();
    editor
        .execute(Command::AddGroup {
            id: group,
            name: "Group".into(),
            parent: None,
            sibling_index: 1,
        })
        .unwrap();
    assert!(matches!(
        editor.execute(Command::Clear),
        Err(CoreError::UnsupportedNodeContent(NodeKind::Group))
    ));
    let child = LayerId::new();
    editor
        .execute(Command::AddLayer {
            id: child,
            name: "Red".into(),
            index: 2,
        })
        .unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(255, 0, 0, 255),
        })
        .unwrap();
    editor
        .execute(Command::MoveNode {
            id: child,
            parent: Some(group),
            sibling_index: 0,
        })
        .unwrap();
    editor
        .execute(Command::SetLayerOpacity {
            id: group,
            opacity: 0.5,
        })
        .unwrap();
    assert_eq!(
        editor.try_render_snapshot().unwrap().pixels(),
        [128, 0, 128, 255, 128, 0, 128, 255]
    );

    editor
        .execute(Command::AddRasterMask { id: group })
        .unwrap();
    editor
        .execute(Command::ReplaceRasterMask {
            id: group,
            rect: Rect::new(0, 0, 2, 1),
            pixels: vec![0, 255],
        })
        .unwrap();
    assert_eq!(
        editor.try_render_snapshot().unwrap().pixels(),
        [0, 0, 255, 255, 128, 0, 128, 255]
    );
    editor
        .execute(Command::SetRasterMaskEnabled {
            id: group,
            enabled: false,
        })
        .unwrap();
    assert_eq!(
        editor.try_render_snapshot().unwrap().pixels(),
        [128, 0, 128, 255, 128, 0, 128, 255]
    );
    editor
        .execute(Command::SetLayerOpacity {
            id: group,
            opacity: 1.0,
        })
        .unwrap();
    editor
        .execute(Command::SetLayerBlendMode {
            id: group,
            mode: BlendMode::Multiply,
        })
        .unwrap();
    assert_eq!(
        editor.try_render_snapshot().unwrap().pixels(),
        [0, 0, 0, 255, 0, 0, 0, 255]
    );
    editor
        .execute(Command::SetLayerVisibility {
            id: group,
            visible: false,
        })
        .unwrap();
    assert_eq!(
        editor.try_render_snapshot().unwrap().pixels(),
        [0, 0, 255, 255, 0, 0, 255, 255]
    );
    assert_eq!(editor.document().layer(bottom).unwrap().parent_id(), None);
}

#[test]
fn raster_mask_from_selection_requires_active_selection_and_replaces_or_attaches_enabled_mask() {
    let mut editor = Editor::new(Document::new(2, 1).unwrap()).unwrap();
    let raster = editor.document().active_layer_id();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(20, 40, 60, 255),
        })
        .unwrap();
    let before = editor.document().clone();
    let generation = editor.generation();
    assert!(matches!(
        editor.execute(Command::RasterMaskFromSelection { id: raster }),
        Err(CoreError::SelectionNotActive)
    ));
    assert_eq!(editor.generation(), generation);
    assert_eq!(editor.document(), &before);

    editor
        .execute(Command::SelectRectangle {
            rect: Rect::new(0, 0, 1, 1),
            mode: SelectionMode::Replace,
        })
        .unwrap();
    editor
        .execute(Command::RasterMaskFromSelection { id: raster })
        .unwrap();
    let mask = editor.document().layer(raster).unwrap().mask().unwrap();
    assert!(mask.is_enabled());
    assert_eq!(mask.pixels(), [255, 0]);
    assert_eq!(
        editor.try_render_snapshot().unwrap().pixels(),
        [20, 40, 60, 255, 0, 0, 0, 0]
    );

    editor
        .execute(Command::SetRasterMaskEnabled {
            id: raster,
            enabled: false,
        })
        .unwrap();
    editor
        .execute(Command::SelectRectangle {
            rect: Rect::new(1, 0, 1, 1),
            mode: SelectionMode::Replace,
        })
        .unwrap();
    editor
        .execute(Command::RasterMaskFromSelection { id: raster })
        .unwrap();
    let mask = editor.document().layer(raster).unwrap().mask().unwrap();
    assert!(mask.is_enabled());
    assert_eq!(mask.pixels(), [0, 255]);
    editor.undo().unwrap();
    assert!(
        !editor
            .document()
            .layer(raster)
            .unwrap()
            .mask()
            .unwrap()
            .is_enabled()
    );
}

#[test]
fn raster_mask_toggle_crop_resize_roundtrip_and_history_restore_exact_state() {
    let mut editor = Editor::new(Document::new(2, 2).unwrap()).unwrap();
    let raster = editor.document().active_layer_id();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(20, 40, 60, 255),
        })
        .unwrap();
    editor
        .execute(Command::AddRasterMask { id: raster })
        .unwrap();
    editor
        .execute(Command::ReplaceRasterMask {
            id: raster,
            rect: Rect::new(0, 0, 2, 2),
            pixels: vec![0, 64, 128, 255],
        })
        .unwrap();
    let masked = editor.try_render_snapshot().unwrap().pixels().to_vec();
    assert_eq!(&masked[0..4], &[0, 0, 0, 0]);
    editor
        .execute(Command::SetRasterMaskEnabled {
            id: raster,
            enabled: false,
        })
        .unwrap();
    assert!(
        editor
            .try_render_snapshot()
            .unwrap()
            .pixels()
            .chunks_exact(4)
            .all(|pixel| pixel == [20, 40, 60, 255])
    );
    editor.undo().unwrap();
    assert_eq!(editor.try_render_snapshot().unwrap().pixels(), masked);

    editor
        .execute(Command::CropCanvas {
            rect: Rect::new(1, 0, 1, 2),
        })
        .unwrap();
    assert_eq!(
        editor
            .document()
            .layer(raster)
            .unwrap()
            .mask()
            .unwrap()
            .pixels(),
        [64, 255]
    );
    editor
        .execute(Command::ResizeCanvas {
            width: 2,
            height: 2,
            sampling: SamplingMode::Nearest,
        })
        .unwrap();
    assert_eq!(
        editor
            .document()
            .layer(raster)
            .unwrap()
            .mask()
            .unwrap()
            .pixels(),
        [64, 64, 255, 255]
    );

    let encoded = save_project(editor.document()).unwrap();
    let restored = load_project(&encoded).unwrap();
    assert_eq!(restored, *editor.document());
    assert!(restored.layer(raster).unwrap().mask().unwrap().is_enabled());
    editor.undo().unwrap();
    editor.redo().unwrap();
    assert_eq!(editor.document(), &restored);
}

#[test]
fn hierarchy_and_mask_failures_leave_generation_and_document_unchanged() {
    let mut editor = Editor::new(Document::new(2, 2).unwrap()).unwrap();
    let raster = editor.document().active_layer_id();
    let duplicate = raster;
    for command in [
        Command::AddGroup {
            id: duplicate,
            name: "Duplicate".into(),
            parent: None,
            sibling_index: 1,
        },
        Command::MoveNode {
            id: raster,
            parent: Some(raster),
            sibling_index: 0,
        },
        Command::ReplaceRasterMask {
            id: raster,
            rect: Rect::new(0, 0, 2, 2),
            pixels: vec![0; 4],
        },
    ] {
        let before = editor.document().clone();
        let generation = editor.generation();
        assert!(editor.execute(command).is_err());
        assert_eq!(editor.generation(), generation);
        assert_eq!(editor.document(), &before);
    }
    editor
        .execute(Command::AddRasterMask { id: raster })
        .unwrap();
    let before = editor.document().clone();
    let generation = editor.generation();
    assert!(matches!(
        editor.execute(Command::ReplaceRasterMask {
            id: raster,
            rect: Rect::new(1, 1, 2, 2),
            pixels: vec![0; 4],
        }),
        Err(CoreError::InvalidMaskOperation)
    ));
    assert_eq!(editor.generation(), generation);
    assert_eq!(editor.document(), &before);
}

#[test]
fn v2_rejects_noncontiguous_hierarchy_order() {
    let mut editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
    let raster = editor.document().active_layer_id();
    let group = LayerId::new();
    editor
        .execute(Command::AddGroup {
            id: group,
            name: "Group".into(),
            parent: None,
            sibling_index: 1,
        })
        .unwrap();
    editor
        .execute(Command::MoveNode {
            id: raster,
            parent: Some(group),
            sibling_index: 0,
        })
        .unwrap();
    let mut value: serde_json::Value =
        serde_json::from_slice(&save_project(editor.document()).unwrap()).unwrap();
    value["document"]["nodes"].as_array_mut().unwrap().reverse();
    assert!(matches!(
        load_project(&serde_json::to_vec(&value).unwrap()),
        Err(CoreError::InvalidHierarchyOrder)
    ));
}

#[test]
fn bounded_semantic_deserializers_accept_exact_limits_and_reject_one_over() {
    let text_value = |length| {
        serde_json::json!({
            "text": "x".repeat(length),
            "font_family": "font8x8 Basic Latin",
            "font_size": 12.0,
            "color": {"r": 1, "g": 2, "b": 3, "a": 255}
        })
    };
    assert!(serde_json::from_value::<TextContent>(text_value(MAX_TEXT_CONTENT_BYTES)).is_ok());
    assert!(serde_json::from_value::<TextContent>(text_value(MAX_TEXT_CONTENT_BYTES + 1)).is_err());

    let escaped = format!(
        "{{\"text\":\"{}\",\"font_family\":\"f\",\"font_size\":12.0,\"color\":{{\"r\":0,\"g\":0,\"b\":0,\"a\":255}}}}",
        "\\u0078".repeat(MAX_TEXT_CONTENT_BYTES + 1)
    );
    assert!(serde_json::from_str::<TextContent>(&escaped).is_err());

    let paths = |count| {
        serde_json::json!({
            "paths": (0..count).map(|_| serde_json::json!({"commands": []})).collect::<Vec<_>>()
        })
    };
    assert!(serde_json::from_value::<VectorContent>(paths(MAX_VECTOR_PATHS)).is_ok());
    assert!(serde_json::from_value::<VectorContent>(paths(MAX_VECTOR_PATHS + 1)).is_err());

    let commands = |count: usize| {
        format!(
            "{{\"paths\":[{{\"commands\":[{}]}}]}}",
            std::iter::repeat_n("{\"type\":\"close\"}", count)
                .collect::<Vec<_>>()
                .join(",")
        )
    };
    assert!(serde_json::from_str::<VectorContent>(&commands(MAX_PATH_COMMANDS_PER_PATH)).is_ok());
    assert!(
        serde_json::from_str::<VectorContent>(&commands(MAX_PATH_COMMANDS_PER_PATH + 1)).is_err()
    );
}

#[test]
fn semantic_memory_admission_accepts_exact_limit_and_rejects_one_over() {
    let exact = SemanticUsage {
        memory_bytes: MAX_SEMANTIC_MEMORY_BYTES,
        ..SemanticUsage::default()
    };
    assert_eq!(
        admit_semantic_replacement(SemanticUsage::default(), SemanticUsage::default(), exact,)
            .unwrap(),
        exact
    );
    let over = SemanticUsage {
        memory_bytes: MAX_SEMANTIC_MEMORY_BYTES + 1,
        ..SemanticUsage::default()
    };
    assert!(matches!(
        admit_semantic_replacement(SemanticUsage::default(), SemanticUsage::default(), over,),
        Err(CoreError::DocumentLimitExceeded("semantic memory bytes"))
    ));
}

fn decode_fixture_node(id: String, content: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "parent": null,
        "name": "Decode fixture",
        "visible": true,
        "opacity": 1.0,
        "blend_mode": "normal",
        "mask": null,
        "content": content
    })
}

fn decode_fixture_id(namespace: u16, index: usize) -> String {
    format!("00000000-0000-0000-{namespace:04x}-{index:012x}")
}

#[test]
fn document_node_decoder_accepts_exact_limit_and_rejects_untyped_one_over() {
    let nodes = (0..MAX_NODES)
        .map(|index| {
            decode_fixture_node(
                decode_fixture_id(1, index),
                serde_json::json!({ "kind": "group" }),
            )
        })
        .collect::<Vec<_>>();
    let mut exact = default_v2_value();
    exact["document"]["nodes"] = serde_json::Value::Array(nodes.clone());
    exact["document"]["active_node"] = serde_json::json!(decode_fixture_id(1, 0));
    assert_eq!(
        load_project(&serde_json::to_vec(&exact).unwrap())
            .unwrap()
            .nodes()
            .len(),
        MAX_NODES
    );

    let mut over = exact;
    over["document"]["nodes"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({ "not": "a typed layer" }));
    assert!(matches!(
        load_project(&serde_json::to_vec(&over).unwrap()),
        Err(CoreError::Json(error)) if error.to_string().contains("node count exceeds 4096")
    ));
}

#[test]
fn v1_layer_decoder_accepts_exact_node_limit_and_rejects_one_over() {
    let layer = |index| {
        serde_json::json!({
            "id": decode_fixture_id(2, index),
            "name": format!("Layer {index}"),
            "visible": true,
            "opacity": 1.0,
            "blend_mode": "normal",
            "pixels": [0, 0, 0, 0]
        })
    };
    let exact_layers = (0..MAX_NODES).map(layer).collect::<Vec<_>>();
    let exact = serde_json::json!({
        "magic": "REDROB_CANVAS_PROJECT",
        "version": 1,
        "document": {
            "id": "00000000-0000-0000-0000-000000000001",
            "width": 1,
            "height": 1,
            "metadata": { "title": "v1 limit", "author": null, "properties": {} },
            "layers": exact_layers,
            "active_layer": decode_fixture_id(2, 0),
            "selection": { "width": 1, "height": 1, "active": false, "mask": [0] }
        }
    });
    assert_eq!(
        load_project(&serde_json::to_vec(&exact).unwrap())
            .unwrap()
            .nodes()
            .len(),
        MAX_NODES
    );

    let mut over = exact;
    over["document"]["layers"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({ "not": "a typed version-1 layer" }));
    assert!(matches!(
        load_project(&serde_json::to_vec(&over).unwrap()),
        Err(CoreError::Json(error))
            if error.to_string().contains("version-1 layer count exceeds 4096")
    ));
}

#[test]
fn vector_path_decoder_charges_multi_path_command_totals_incrementally() {
    const PATHS: usize = 16;
    let commands_per_path = MAX_PATH_COMMANDS / PATHS;
    let mut vector = VectorContent {
        paths: (0..PATHS)
            .map(|_| VectorPath {
                commands: vec![PathCommand::Close; commands_per_path],
                ..VectorPath::default()
            })
            .collect(),
    };
    let exact = serde_json::to_vec(&vector).unwrap();
    let decoded: VectorContent = serde_json::from_slice(&exact).unwrap();
    assert_eq!(
        decoded
            .paths
            .iter()
            .map(|path| path.commands.len())
            .sum::<usize>(),
        MAX_PATH_COMMANDS
    );
    drop(decoded);

    vector
        .paths
        .last_mut()
        .unwrap()
        .commands
        .push(PathCommand::Close);
    let error =
        serde_json::from_slice::<VectorContent>(&serde_json::to_vec(&vector).unwrap()).unwrap_err();
    assert!(error.to_string().contains("vector command complexity"));
}

#[test]
fn document_decoder_charges_multi_node_text_and_path_totals_incrementally() {
    let text_overhead = 1 + EMBEDDED_FONT_ID.len();
    let payload_total = MAX_TEXT_BYTES - 4 * text_overhead;
    let base = payload_total / 4;
    let remainder = payload_total % 4;
    let text_nodes = (0..4)
        .map(|index| {
            let length = base + usize::from(index < remainder);
            decode_fixture_node(
                decode_fixture_id(3, index),
                serde_json::json!({
                    "kind": "text",
                    "text": {
                        "text": "x".repeat(length),
                        "font_family": "f",
                        "font_size": 1.0,
                        "color": { "r": 1, "g": 2, "b": 3, "a": 255 },
                        "origin_x": 1_000_000.0,
                        "origin_y": 1_000_000.0
                    }
                }),
            )
        })
        .collect::<Vec<_>>();
    let mut exact_text = default_v2_value();
    exact_text["document"]["nodes"] = serde_json::Value::Array(text_nodes);
    exact_text["document"]["active_node"] = serde_json::json!(decode_fixture_id(3, 0));
    assert_eq!(
        load_project(&serde_json::to_vec(&exact_text).unwrap())
            .unwrap()
            .semantic_usage()
            .unwrap()
            .text_bytes,
        MAX_TEXT_BYTES
    );
    let mut over_text = exact_text;
    let replacement = format!(
        "{}x",
        over_text["document"]["nodes"][3]["content"]["text"]["text"]
            .as_str()
            .unwrap()
    );
    over_text["document"]["nodes"][3]["content"]["text"]["text"] = serde_json::json!(replacement);
    assert!(matches!(
        load_project(&serde_json::to_vec(&over_text).unwrap()),
        Err(CoreError::Json(error)) if error.to_string().contains("text complexity")
    ));

    let path = serde_json::json!({
        "commands": [
            { "type": "move_to", "x": 1_000_000.0, "y": 1_000_000.0 },
            { "type": "line_to", "x": 1_000_001.0, "y": 1_000_000.0 }
        ],
        "stroke": { "color": { "r": 1, "g": 2, "b": 3, "a": 255 }, "width": 1.0 }
    });
    let vector_nodes = (0..4)
        .map(|index| {
            decode_fixture_node(
                decode_fixture_id(4, index),
                serde_json::json!({
                    "kind": "vector",
                    "vector": { "paths": vec![path.clone(); MAX_VECTOR_PATHS / 4] }
                }),
            )
        })
        .collect::<Vec<_>>();
    let mut exact_paths = default_v2_value();
    exact_paths["document"]["nodes"] = serde_json::Value::Array(vector_nodes);
    exact_paths["document"]["active_node"] = serde_json::json!(decode_fixture_id(4, 0));
    assert_eq!(
        load_project(&serde_json::to_vec(&exact_paths).unwrap())
            .unwrap()
            .semantic_usage()
            .unwrap()
            .vector_paths,
        MAX_VECTOR_PATHS
    );
    let mut over_paths = exact_paths;
    over_paths["document"]["nodes"][3]["content"]["vector"]["paths"]
        .as_array_mut()
        .unwrap()
        .push(path);
    assert!(matches!(
        load_project(&serde_json::to_vec(&over_paths).unwrap()),
        Err(CoreError::Json(error)) if error.to_string().contains("vector path count")
    ));
}

/// The Curves filter must reach pixels through the command path, not merely exist.
///
/// A curve that only passed its own unit tests would be the mesh mistake in miniature: correct
/// mathematics with nothing calling it. This drives it the way the product does.
#[test]
fn curves_filter_remaps_pixels_through_the_editor() {
    let points = vec![
        CurvePoint::smooth(0.0, 0.0),
        CurvePoint::smooth(0.5, 0.75),
        CurvePoint::smooth(1.0, 1.0),
    ];
    let table = ToneCurve::new(points.clone())
        .unwrap()
        .transfer_table_8bit();

    let mut editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(128, 64, 255, 255),
        })
        .unwrap();

    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Curves {
                points: points.clone(),
            },
        })
        .unwrap();

    // Each channel must land exactly where the transfer table says, and independently.
    assert_eq!(
        pixel(&editor, layer, 0, 0),
        Pixel::rgba(table[128], table[64], table[255], 255),
        "every channel goes through the transfer table"
    );
    assert!(
        table[128] > 128,
        "this curve lifts, so the midtone must rise"
    );
}

/// The identity curve must leave every channel value untouched.
///
/// This is what catches an off-by-one in the table's step: `1/size` instead of `1/(size - 1)` leaves the
/// identity almost right and wrong only near white, which no spot check would notice.
#[test]
fn the_identity_curve_changes_no_channel_value() {
    let identity = ToneCurve::identity().transfer_table_8bit();
    for (value, mapped) in identity.iter().enumerate() {
        assert_eq!(
            *mapped, value as u8,
            "the identity table must be exactly the identity at {value}"
        );
    }

    // And through the editor, on a filled layer.
    let mut editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(255, 0, 137, 255),
        })
        .unwrap();
    let before = pixel(&editor, layer, 0, 0);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Curves {
                points: vec![CurvePoint::smooth(0.0, 0.0), CurvePoint::smooth(1.0, 1.0)],
            },
        })
        .unwrap();
    assert_eq!(
        pixel(&editor, layer, 0, 0),
        before,
        "the identity curve must change nothing, including the white channel"
    );
}

/// An unusable point list is refused, and a refused filter leaves the layer alone.
#[test]
fn an_invalid_curve_is_refused_and_changes_nothing() {
    let mut editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(128, 128, 128, 255),
        })
        .unwrap();
    let before = pixel(&editor, layer, 0, 0);

    for points in [
        Vec::new(),
        // Two points sharing an x: the spline would divide by a zero-width interval.
        vec![
            CurvePoint::smooth(0.0, 0.0),
            CurvePoint::smooth(0.5, 0.2),
            CurvePoint::smooth(0.5, 0.9),
            CurvePoint::smooth(1.0, 1.0),
        ],
        vec![
            CurvePoint::smooth(0.0, 0.0),
            CurvePoint::smooth(f32::NAN, 1.0),
        ],
    ] {
        assert!(
            editor
                .execute(Command::ApplyFilter {
                    filter: Filter::Curves { points },
                })
                .is_err(),
            "an unusable curve must be refused"
        );
    }
    assert_eq!(
        pixel(&editor, layer, 0, 0),
        before,
        "a refused filter leaves the pixels alone"
    );
}

/// A corner point survives the round trip through a serialised command.
///
/// The points ARE the document's record of the curve, so the flag has to serialise. A dropped `corner`
/// would reopen the file as a visibly different curve.
#[test]
fn a_curve_with_a_corner_survives_command_serialisation() {
    let command = Command::ApplyFilter {
        filter: Filter::Curves {
            points: vec![
                CurvePoint::smooth(0.0, 0.0),
                CurvePoint::corner(0.5, 0.7),
                CurvePoint::smooth(1.0, 1.0),
            ],
        },
    };
    let json = serde_json::to_string(&command).unwrap();
    assert!(json.contains("corner"), "the flag must be written: {json}");
    let restored: Command = serde_json::from_str(&json).unwrap();
    assert_eq!(restored, command);

    if let Command::ApplyFilter {
        filter: Filter::Curves { points },
    } = restored
    {
        let curve = ToneCurve::new(points).unwrap();
        assert!(
            (curve.value(0.25) - 0.35).abs() < 2e-6,
            "the corner curve's value at 0.25 is Krita's 0.35, got {}",
            curve.value(0.25)
        );
    } else {
        panic!("the command changed shape");
    }
}

/// Hardness must change painted pixels, not merely exist as a field.
///
/// The previous three blocks each found translated mathematics with nothing to call it. This asserts the
/// opposite for this one: the same stroke at two hardnesses paints measurably different edges.
#[test]
fn dab_hardness_changes_the_painted_edge() {
    fn paint(shape: DabShape) -> Vec<Pixel> {
        let mut editor = Editor::new(Document::new(41, 41).unwrap()).unwrap();
        editor
            .execute(Command::BrushStroke {
                points: vec![BrushPoint {
                    x: 20.5,
                    y: 20.5,
                    pressure: 1.0,
                }],
                color: Pixel::rgba(0, 0, 0, 255),
                size: 30.0,
                opacity: 1.0,
                settings: BrushSettings {
                    shape,
                    ..Default::default()
                },
                tip: None,
                pipe: Vec::new(),
            })
            .unwrap();
        let layer = editor.document().active_layer_id();
        (0..41)
            .map(|x| pixel(&editor, layer, x, 20))
            .collect::<Vec<_>>()
    }

    let hard = paint(DabShape {
        hardness: 1.0,
        softness: 1.0,
        ratio: 1.0,
        antialias_edges: false,
        pencil: false,
    });
    let soft = paint(DabShape {
        hardness: 0.2,
        softness: 1.0,
        ratio: 1.0,
        antialias_edges: false,
        pencil: false,
    });

    // The centre is solid under both.
    assert_eq!(hard[20].a, 255, "a hard dab covers its centre");
    assert_eq!(soft[20].a, 255, "and so does a soft one");

    // Midway out, the soft dab is measurably more transparent. This is the assertion that fails if the
    // shape is carried and ignored.
    let midway = 20 + 8;
    assert_eq!(
        hard[midway].a, 255,
        "the hard dab is still solid at 8px out"
    );
    assert!(
        soft[midway].a < 200,
        "the soft dab must have faded by 8px out, got alpha {}",
        soft[midway].a
    );

    // And the soft dab's alpha falls monotonically from the centre outward.
    let mut previous = 255u8;
    for (offset, painted) in soft.iter().enumerate().skip(20).take(16) {
        assert!(
            painted.a <= previous,
            "soft dab alpha rose from {previous} to {} at x={offset}",
            painted.a
        );
        previous = painted.a;
    }
}

/// An elliptical dab must paint an ellipse.
#[test]
fn dab_ratio_paints_an_ellipse() {
    let mut editor = Editor::new(Document::new(41, 41).unwrap()).unwrap();
    editor
        .execute(Command::BrushStroke {
            points: vec![BrushPoint {
                x: 20.5,
                y: 20.5,
                pressure: 1.0,
            }],
            color: Pixel::rgba(0, 0, 0, 255),
            size: 30.0,
            opacity: 1.0,
            settings: BrushSettings {
                shape: DabShape {
                    hardness: 1.0,
                    softness: 1.0,
                    // Half as tall as it is wide.
                    ratio: 0.5,
                    antialias_edges: false,
                    pencil: false,
                },
                ..Default::default()
            },
            tip: None,
            pipe: Vec::new(),
        })
        .unwrap();
    let layer = editor.document().active_layer_id();

    // 12 pixels right of centre is inside a 30-wide dab; 12 below is outside a 15-tall one.
    assert_eq!(
        pixel(&editor, layer, 32, 20).a,
        255,
        "inside along the long axis"
    );
    assert_eq!(
        pixel(&editor, layer, 20, 32).a,
        0,
        "outside along the short axis"
    );
}

/// An out-of-range shape is refused and paints nothing.
#[test]
fn an_invalid_dab_shape_is_refused() {
    let mut editor = Editor::new(Document::new(9, 9).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    let before = pixel(&editor, layer, 4, 4);

    for shape in [
        DabShape {
            hardness: 1.5,
            ..DabShape::default()
        },
        DabShape {
            hardness: f32::NAN,
            ..DabShape::default()
        },
        DabShape {
            softness: 0.0,
            ..DabShape::default()
        },
        DabShape {
            ratio: 0.0,
            ..DabShape::default()
        },
    ] {
        assert!(
            editor
                .execute(Command::BrushStroke {
                    points: vec![BrushPoint {
                        x: 4.5,
                        y: 4.5,
                        pressure: 1.0,
                    }],
                    color: Pixel::rgba(0, 0, 0, 255),
                    size: 6.0,
                    opacity: 1.0,
                    settings: BrushSettings {
                        shape,
                        ..Default::default()
                    },
                    tip: None,
                    pipe: Vec::new(),
                })
                .is_err(),
            "{shape:?} must be refused"
        );
    }
    assert_eq!(
        pixel(&editor, layer, 4, 4),
        before,
        "a refused stroke paints nothing"
    );
}

/// A default shape is omitted from the serialised command, and a non-default one is carried.
///
/// The omission is what keeps every document and agent proposal written before this field existed
/// byte-identical. The carrying is what makes a saved shape reopen as itself.
#[test]
fn a_dab_shape_round_trips_and_a_default_one_is_omitted() {
    let with_default = Command::BrushStroke {
        points: vec![BrushPoint {
            x: 1.0,
            y: 1.0,
            pressure: 1.0,
        }],
        color: Pixel::rgba(0, 0, 0, 255),
        size: 4.0,
        opacity: 1.0,
        settings: BrushSettings::default(),
        tip: None,
        pipe: Vec::new(),
    };
    let json = serde_json::to_string(&with_default).unwrap();
    assert!(
        !json.contains("shape"),
        "a default shape must not be written: {json}"
    );
    assert_eq!(
        serde_json::from_str::<Command>(&json).unwrap(),
        with_default,
        "and it must come back as the default"
    );

    let shaped = Command::BrushStroke {
        points: vec![BrushPoint {
            x: 1.0,
            y: 1.0,
            pressure: 1.0,
        }],
        color: Pixel::rgba(0, 0, 0, 255),
        size: 4.0,
        opacity: 1.0,
        settings: BrushSettings {
            shape: DabShape {
                hardness: 0.25,
                softness: 1.5,
                ratio: 0.75,
                antialias_edges: false,
                pencil: false,
            },
            ..Default::default()
        },
        tip: None,
        pipe: Vec::new(),
    };
    let json = serde_json::to_string(&shaped).unwrap();
    assert!(
        json.contains("hardness"),
        "a real shape must be written: {json}"
    );
    assert_eq!(serde_json::from_str::<Command>(&json).unwrap(), shaped);

    // A command written before the field existed still loads.
    let legacy = r#"{"type":"brush_stroke","points":[{"x":1.0,"y":1.0,"pressure":1.0}],
        "color":{"r":0,"g":0,"b":0,"a":255},"size":4.0,"opacity":1.0,
        "settings":{"smoothing":{"kind":"none"},"mirror_x":null,"mirror_y":null}}"#;
    let restored: Command = serde_json::from_str(legacy).unwrap();
    assert_eq!(
        restored, with_default,
        "a pre-shape command must load as the default shape"
    );
}

/// Pressure scales the dab's extent, so its falloff must scale with it too.
///
/// This is the regression test for a bug introduced while wiring the shape in: the mask was first
/// resolved once per stroke from `size` alone, ignoring that `radius` is `size * pressure * 0.5`. A
/// light-pressure dab then received the centre of a full-size mask and came out uniformly solid, with no
/// falloff of its own. Clippy surfaced it by reporting `BrushDabRaster::radius` as never read.
#[test]
fn pressure_scales_the_dab_falloff_not_just_its_size() {
    fn row(pressure: f32) -> Vec<u8> {
        let mut editor = Editor::new(Document::new(41, 41).unwrap()).unwrap();
        editor
            .execute(Command::BrushStroke {
                points: vec![BrushPoint {
                    x: 20.5,
                    y: 20.5,
                    pressure,
                }],
                color: Pixel::rgba(0, 0, 0, 255),
                size: 32.0,
                opacity: 1.0,
                settings: BrushSettings {
                    shape: DabShape {
                        hardness: 0.2,
                        softness: 1.0,
                        ratio: 1.0,
                        antialias_edges: false,
                        pencil: false,
                    },
                    ..Default::default()
                },
                tip: None,
                pipe: Vec::new(),
            })
            .unwrap();
        let layer = editor.document().active_layer_id();
        (20..41).map(|x| pixel(&editor, layer, x, 20).a).collect()
    }

    let full = row(1.0);
    let half = row(0.5);

    // Pressure has TWO effects and they are separate. It scales the radius, and it also multiplies the
    // dab's alpha directly in the paint loop -- so a half-pressure dab is both smaller and half as
    // opaque. The first version of this test asserted a solid 255 centre for both and failed on the
    // second effect, which predates this change entirely.
    assert_eq!(full[0], 255, "a full-pressure dab is opaque at its centre");
    assert_eq!(
        half[0], 128,
        "a half-pressure dab is half-opaque at its centre"
    );

    // The half-pressure dab must END sooner: its radius is 8 where the full one's is 16.
    assert_eq!(
        half[12], 0,
        "a half-pressure dab of size 32 stops before 12px"
    );
    assert!(
        full[12] > 0,
        "while the full-pressure one still covers 12px"
    );

    // And it must FALL OFF within its own extent rather than being solid to its edge. This is the part
    // that was broken: a mask sized for the full radius is still inside its solid core at 4px, so the
    // dab came out flat at its own centre value all the way to its rim.
    assert!(
        half[4] < 128 && half[4] > 0,
        "a soft half-pressure dab must be partially covered at 4px, got {}",
        half[4]
    );

    // Normalised by each dab's own centre value, the two falloffs must agree at the same fraction of
    // their own radii. That is what "the mask scales with the dab" means, with pressure's opacity effect
    // divided back out.
    let full_fraction = f32::from(full[8]) / f32::from(full[0]);
    let half_fraction = f32::from(half[4]) / f32::from(half[0]);
    assert!(
        (full_fraction - half_fraction).abs() < 0.15,
        "at half of its own radius each dab should read alike once normalised: {full_fraction} against {half_fraction}"
    );
}

/// A GBR tip must reach pixels, not merely decode.
///
/// The blocks before this one each found translated code with nothing to call it, so the decoder gets the
/// same test the tone curve and the dab shape got: drive it as a command and read the canvas.
#[test]
fn a_decoded_gbr_tip_paints_its_own_shape() {
    // A 4x4 tip covering only its right half, so the painted result is asymmetric in a way no generated
    // round dab could produce. Built to the layout the reference decoder printed.
    let mut payload = vec![0u8; 16];
    for y in 0..4 {
        for x in 2..4 {
            payload[y * 4 + x] = 255;
        }
    }
    let mut data = Vec::new();
    for field in [(28u32 + 4), 2u32, 4u32, 4u32, 1u32] {
        data.extend_from_slice(&field.to_be_bytes());
    }
    data.extend_from_slice(b"GIMP");
    data.extend_from_slice(&10u32.to_be_bytes());
    data.extend_from_slice(b"hal\0");
    data.extend_from_slice(&payload);

    let tip = BrushTip::from_gbr(&data).expect("the tip must decode");
    assert_eq!((tip.width(), tip.height()), (4, 4));
    assert_eq!(tip.pixel(0, 0), 0, "the left half is empty");
    assert_eq!(tip.pixel(3, 0), 255, "the right half is solid");

    let mut editor = Editor::new(Document::new(41, 41).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    editor
        .execute(Command::BrushStroke {
            points: vec![BrushPoint {
                x: 20.5,
                y: 20.5,
                pressure: 1.0,
            }],
            color: Pixel::rgba(0, 0, 0, 255),
            size: 20.0,
            opacity: 1.0,
            settings: BrushSettings::default(),
            tip: Some(tip),
            pipe: Vec::new(),
        })
        .unwrap();

    // The right of centre is painted and the left is not -- which a round generated dab cannot do, so
    // this also proves the tip REPLACED the shape rather than being ignored.
    let right = pixel(&editor, layer, 26, 20).a;
    let left = pixel(&editor, layer, 14, 20).a;
    assert!(
        right > 200,
        "the tip's solid half must paint, got alpha {right}"
    );
    assert_eq!(left, 0, "and its empty half must not, got alpha {left}");
}

/// A tip whose coverage length disagrees with its dimensions is refused before anything indexes it.
#[test]
fn an_inconsistent_tip_is_refused_by_the_command() {
    let mut data = Vec::new();
    for field in [(28u32 + 2), 2u32, 3u32, 2u32, 1u32] {
        data.extend_from_slice(&field.to_be_bytes());
    }
    data.extend_from_slice(b"GIMP");
    data.extend_from_slice(&10u32.to_be_bytes());
    data.extend_from_slice(b"t\0");
    data.extend_from_slice(&[10, 20, 30, 40, 50, 60]);
    let good = BrushTip::from_gbr(&data).unwrap();
    assert!(good.is_valid());

    // Round-trip through JSON with a shortened coverage array, which is what a hostile or corrupt document
    // would carry. The decoder never produces this; the command boundary is the only thing that can catch
    // it.
    let json = serde_json::to_string(&good).unwrap();
    let tampered = json.replace("[10,20,30,40,50,60]", "[10,20]");
    assert_ne!(tampered, json, "the substitution must have applied");
    let bad: BrushTip = serde_json::from_str(&tampered).unwrap();

    let mut editor = Editor::new(Document::new(9, 9).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    let before = pixel(&editor, layer, 4, 4);
    assert!(
        editor
            .execute(Command::BrushStroke {
                points: vec![BrushPoint {
                    x: 4.5,
                    y: 4.5,
                    pressure: 1.0,
                }],
                color: Pixel::rgba(0, 0, 0, 255),
                size: 6.0,
                opacity: 1.0,
                settings: BrushSettings::default(),
                tip: Some(bad),
                pipe: Vec::new(),
            })
            .is_err(),
        "a tip lying about its own size must be refused"
    );
    assert_eq!(
        pixel(&editor, layer, 4, 4),
        before,
        "and the layer must be untouched"
    );
}

/// A stroke without a tip serialises exactly as it did before the field existed.
#[test]
fn a_tipless_stroke_serialises_unchanged_and_a_tip_round_trips() {
    let plain = Command::BrushStroke {
        points: vec![BrushPoint {
            x: 1.0,
            y: 2.0,
            pressure: 0.5,
        }],
        color: Pixel::rgba(1, 2, 3, 255),
        size: 8.0,
        opacity: 1.0,
        settings: BrushSettings::default(),
        tip: None,
        pipe: Vec::new(),
    };
    let json = serde_json::to_string(&plain).unwrap();
    assert!(
        !json.contains("tip"),
        "an absent tip must not be written: {json}"
    );
    assert!(
        !json.contains("shape"),
        "nor a default shape, so old documents stay byte-identical: {json}"
    );
    assert_eq!(serde_json::from_str::<Command>(&json).unwrap(), plain);

    // A command written before either field existed still loads.
    let legacy = r#"{"type":"brush_stroke","points":[{"x":1.0,"y":2.0,"pressure":0.5}],
        "color":{"r":1,"g":2,"b":3,"a":255},"size":8.0,"opacity":1.0,
        "settings":{"smoothing":{"kind":"none"},"mirror_x":null,"mirror_y":null}}"#;
    assert_eq!(serde_json::from_str::<Command>(legacy).unwrap(), plain);

    // And a tip survives the round trip intact.
    let mut data = Vec::new();
    for field in [(28u32 + 2), 2u32, 2u32, 1u32, 1u32] {
        data.extend_from_slice(&field.to_be_bytes());
    }
    data.extend_from_slice(b"GIMP");
    data.extend_from_slice(&50u32.to_be_bytes());
    data.extend_from_slice(b"r\0");
    data.extend_from_slice(&[77, 88]);
    let tip = BrushTip::from_gbr(&data).unwrap();
    let with_tip = Command::BrushStroke {
        points: vec![BrushPoint {
            x: 1.0,
            y: 2.0,
            pressure: 0.5,
        }],
        color: Pixel::rgba(1, 2, 3, 255),
        size: 8.0,
        opacity: 1.0,
        settings: BrushSettings::default(),
        tip: Some(tip.clone()),
        pipe: Vec::new(),
    };
    let json = serde_json::to_string(&with_tip).unwrap();
    assert!(json.contains("coverage"), "a real tip must be written");
    let restored: Command = serde_json::from_str(&json).unwrap();
    assert_eq!(restored, with_tip);
    if let Command::BrushStroke {
        tip: Some(back), ..
    } = restored
    {
        assert_eq!(back.pixel(0, 0), 77);
        assert_eq!(back.pixel(1, 0), 88);
        assert!((back.spacing() - 0.5).abs() < 1e-6);
        assert_eq!(back.name(), "r");
    } else {
        panic!("the tip did not survive");
    }
}

/// The bucket tool must reach pixels through the command path.
#[test]
fn flood_fill_fills_a_region_and_stops_at_a_barrier() {
    let mut editor = Editor::new(Document::new(9, 3).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(255, 255, 255, 255),
        })
        .unwrap();
    // A black barrier down the middle column, drawn with a one-pixel hard brush.
    for y in 0..3 {
        editor
            .execute(Command::BrushStroke {
                points: vec![BrushPoint {
                    x: 4.5,
                    y: y as f32 + 0.5,
                    pressure: 1.0,
                }],
                color: Pixel::rgba(0, 0, 0, 255),
                size: 1.0,
                opacity: 1.0,
                settings: BrushSettings {
                    shape: DabShape {
                        hardness: 1.0,
                        softness: 1.0,
                        ratio: 1.0,
                        antialias_edges: false,
                        pencil: false,
                    },
                    ..Default::default()
                },
                tip: None,
                pipe: Vec::new(),
            })
            .unwrap();
    }
    assert_eq!(
        pixel(&editor, layer, 4, 1).r,
        0,
        "the barrier must be there"
    );

    editor
        .execute(Command::FloodFill {
            x: 1,
            y: 1,
            color: Pixel::rgba(255, 0, 0, 255),
            options: FloodFillOptions {
                tolerance: 10,
                opacity_spread: 100,
            },
        })
        .unwrap();

    // Left of the barrier is red.
    for y in 0..3 {
        for x in 0..4 {
            assert_eq!(
                pixel(&editor, layer, x, y),
                Pixel::rgba(255, 0, 0, 255),
                "left of the barrier at {x},{y}"
            );
        }
    }
    // The barrier and the far side are untouched.
    for y in 0..3 {
        assert_eq!(pixel(&editor, layer, 4, y).r, 0, "the barrier at row {y}");
        for x in 5..9 {
            assert_eq!(
                pixel(&editor, layer, x, y),
                Pixel::rgba(255, 255, 255, 255),
                "right of the barrier at {x},{y}"
            );
        }
    }
}

/// The fill reads a snapshot, so a fill colour within tolerance of the old one does not run away.
///
/// Filling white with near-white at a generous tolerance is the case that breaks a naive implementation:
/// if the region is re-read while being written, each filled pixel answers the colour test and the fill
/// spreads through pixels it should never have reached. Here there is no barrier to prove that with, so
/// the assertion is that a bounded region stays bounded.
#[test]
fn flood_fill_reads_a_snapshot_rather_than_its_own_output() {
    let mut editor = Editor::new(Document::new(7, 1).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(100, 100, 100, 255),
        })
        .unwrap();
    // One pixel of a clearly different colour, splitting the row.
    {
        editor
            .execute(Command::BrushStroke {
                points: vec![BrushPoint {
                    x: 3.5,
                    y: 0.5,
                    pressure: 1.0,
                }],
                color: Pixel::rgba(255, 255, 255, 255),
                size: 1.0,
                opacity: 1.0,
                settings: BrushSettings {
                    shape: DabShape {
                        hardness: 1.0,
                        softness: 1.0,
                        ratio: 1.0,
                        antialias_edges: false,
                        pencil: false,
                    },
                    ..Default::default()
                },
                tip: None,
                pipe: Vec::new(),
            })
            .unwrap();
    }

    // Fill the left side with a colour very close to the WHITE barrier. A naive re-reading fill would then
    // step across it.
    editor
        .execute(Command::FloodFill {
            x: 0,
            y: 0,
            color: Pixel::rgba(250, 250, 250, 255),
            options: FloodFillOptions {
                tolerance: 10,
                opacity_spread: 100,
            },
        })
        .unwrap();

    for x in 0..3 {
        assert_eq!(
            pixel(&editor, layer, x, 0),
            Pixel::rgba(250, 250, 250, 255),
            "the left side is filled at {x}"
        );
    }
    assert_eq!(
        pixel(&editor, layer, 3, 0),
        Pixel::rgba(255, 255, 255, 255),
        "the barrier is untouched"
    );
    for x in 4..7 {
        assert_eq!(
            pixel(&editor, layer, x, 0),
            Pixel::rgba(100, 100, 100, 255),
            "and the far side never saw the fill at {x}"
        );
    }
}

/// The selection gates the fill, exactly as it gates every other paint command.
#[test]
fn flood_fill_respects_the_selection() {
    let mut editor = Editor::new(Document::new(6, 1).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(255, 255, 255, 255),
        })
        .unwrap();
    editor
        .execute(Command::SelectRectangle {
            rect: Rect::new(0, 0, 3, 1),
            mode: SelectionMode::Replace,
        })
        .unwrap();
    editor
        .execute(Command::FloodFill {
            x: 0,
            y: 0,
            color: Pixel::rgba(0, 0, 255, 255),
            options: FloodFillOptions::default(),
        })
        .unwrap();

    for x in 0..3 {
        assert_eq!(
            pixel(&editor, layer, x, 0),
            Pixel::rgba(0, 0, 255, 255),
            "inside the selection at {x}"
        );
    }
    for x in 3..6 {
        assert_eq!(
            pixel(&editor, layer, x, 0),
            Pixel::rgba(255, 255, 255, 255),
            "outside it at {x}, even though the region is contiguous"
        );
    }
}

/// A seed the tolerance excludes leaves the layer alone and is not an error.
#[test]
fn an_unfillable_seed_is_a_no_op_rather_than_a_failure() {
    let mut editor = Editor::new(Document::new(4, 1).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(255, 255, 255, 255),
        })
        .unwrap();
    let before = pixel(&editor, layer, 0, 0);

    // A soft fill at zero tolerance fills nothing at all, including its own seed -- Krita's arithmetic.
    editor
        .execute(Command::FloodFill {
            x: 0,
            y: 0,
            color: Pixel::rgba(0, 0, 0, 255),
            options: FloodFillOptions {
                tolerance: 0,
                opacity_spread: 0,
            },
        })
        .expect("clicking an excluded pixel is an ordinary thing to do, not an error");
    assert_eq!(
        pixel(&editor, layer, 0, 0),
        before,
        "and it changes nothing"
    );
}

/// An out-of-range seed or spread is refused, and the layer is untouched.
#[test]
fn flood_fill_refuses_bad_arguments() {
    let mut editor = Editor::new(Document::new(4, 2).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(255, 255, 255, 255),
        })
        .unwrap();
    let before = pixel(&editor, layer, 0, 0);

    for command in [
        Command::FloodFill {
            x: 4,
            y: 0,
            color: Pixel::rgba(0, 0, 0, 255),
            options: FloodFillOptions::default(),
        },
        Command::FloodFill {
            x: 0,
            y: 2,
            color: Pixel::rgba(0, 0, 0, 255),
            options: FloodFillOptions::default(),
        },
        Command::FloodFill {
            x: 0,
            y: 0,
            color: Pixel::rgba(0, 0, 0, 255),
            options: FloodFillOptions {
                tolerance: 10,
                opacity_spread: 101,
            },
        },
    ] {
        assert!(
            editor.execute(command).is_err(),
            "bad arguments must be refused"
        );
    }
    assert_eq!(pixel(&editor, layer, 0, 0), before);
}

/// The command round-trips, and its options are omitted when default.
#[test]
fn a_flood_fill_command_round_trips() {
    let plain = Command::FloodFill {
        x: 3,
        y: 4,
        color: Pixel::rgba(1, 2, 3, 255),
        options: FloodFillOptions::default(),
    };
    let json = serde_json::to_string(&plain).unwrap();
    assert_eq!(serde_json::from_str::<Command>(&json).unwrap(), plain);

    let custom = Command::FloodFill {
        x: 3,
        y: 4,
        color: Pixel::rgba(1, 2, 3, 255),
        options: FloodFillOptions {
            tolerance: 200,
            opacity_spread: 25,
        },
    };
    let json = serde_json::to_string(&custom).unwrap();
    assert!(
        json.contains("200"),
        "a non-default tolerance must be written: {json}"
    );
    assert_eq!(serde_json::from_str::<Command>(&json).unwrap(), custom);

    // A command written without options loads with the defaults.
    let without = r#"{"type":"flood_fill","x":3,"y":4,"color":{"r":1,"g":2,"b":3,"a":255}}"#;
    assert_eq!(serde_json::from_str::<Command>(without).unwrap(), plain);
}

/// Spacing must change what is painted, not merely exist as a setting.
#[test]
fn dab_spacing_changes_how_many_dabs_a_stroke_paints() {
    fn painted_columns(spacing: f32) -> usize {
        let mut editor = Editor::new(Document::new(61, 3).unwrap()).unwrap();
        let layer = editor.document().active_layer_id();
        editor
            .execute(Command::BrushStroke {
                points: vec![
                    BrushPoint::new(1.5, 1.5, 1.0),
                    BrushPoint::new(59.5, 1.5, 1.0),
                ],
                color: Pixel::rgba(0, 0, 0, 255),
                size: 2.0,
                opacity: 1.0,
                settings: BrushSettings {
                    shape: DabShape {
                        hardness: 1.0,
                        softness: 1.0,
                        ratio: 1.0,
                        antialias_edges: false,
                        pencil: false,
                    },
                    spacing: SpacingOptions {
                        spacing,
                        isotropic: false,
                    },
                    ..Default::default()
                },
                tip: None,
                pipe: Vec::new(),
            })
            .unwrap();
        (0..61)
            .filter(|x| pixel(&editor, layer, *x, 1).a > 0)
            .count()
    }

    // A 2-pixel brush: spacing 0.25 gives a 0.5-pixel ellipse, so the stroke is solid. A spacing of 5
    // gives a 10-pixel ellipse, so it is a dotted line.
    let dense = painted_columns(0.25);
    let sparse = painted_columns(5.0);
    assert!(
        dense > sparse,
        "a tighter spacing must paint more: {dense} against {sparse}"
    );
    assert!(
        sparse < 30,
        "and a spacing of five times the brush must leave gaps, got {sparse} of 61 columns"
    );
    assert!(dense > 50, "while a tight one is nearly solid, got {dense}");
}

/// An elliptical dab spaces along its own axes, which is what the scalar rule could not express.
///
/// Measured in Krita for a 20x5 dab at spacing 0.25: every 5.0 horizontally and every 1.25 vertically. A
/// horizontal stroke therefore places a quarter as many dabs as a vertical one of the same length.
#[test]
fn an_elliptical_dab_spaces_per_axis_through_the_editor() {
    fn dab_count(horizontal: bool) -> usize {
        let (width, height) = if horizontal { (81, 21) } else { (21, 81) };
        let mut editor = Editor::new(Document::new(width, height).unwrap()).unwrap();
        let layer = editor.document().active_layer_id();
        let points = if horizontal {
            vec![
                BrushPoint::new(10.5, 10.5, 1.0),
                BrushPoint::new(70.5, 10.5, 1.0),
            ]
        } else {
            vec![
                BrushPoint::new(10.5, 10.5, 1.0),
                BrushPoint::new(10.5, 70.5, 1.0),
            ]
        };
        editor
            .execute(Command::BrushStroke {
                points,
                color: Pixel::rgba(0, 0, 0, 255),
                size: 20.0,
                opacity: 1.0,
                settings: BrushSettings {
                    shape: DabShape {
                        hardness: 1.0,
                        softness: 1.0,
                        // A quarter as tall as it is wide, so the vertical spacing is a quarter too.
                        ratio: 0.25,
                        antialias_edges: false,
                        pencil: false,
                    },
                    spacing: SpacingOptions {
                        spacing: 0.25,
                        isotropic: false,
                    },
                    ..Default::default()
                },
                tip: None,
                pipe: Vec::new(),
            })
            .unwrap();
        // Count painted pixels rather than dabs: more, closer dabs cover more of the stroke's length.
        let mut covered = 0;
        for y in 0..height {
            for x in 0..width {
                if pixel(&editor, layer, x, y).a > 0 {
                    covered += 1;
                }
            }
        }
        covered
    }

    // Both strokes are 60 pixels long with the same dab, so any difference is the spacing.
    let horizontal = dab_count(true);
    let vertical = dab_count(false);
    assert!(horizontal > 0 && vertical > 0, "both strokes must paint");
    assert!(
        vertical > horizontal,
        "the short axis spaces four times tighter, so a vertical stroke covers more: {vertical} \
         against {horizontal}"
    );
}

/// Isotropic spacing makes the two directions agree again.
#[test]
fn isotropic_spacing_removes_the_directional_difference() {
    fn axes_for(isotropic: bool) -> (f32, f32) {
        SpacingOptions {
            spacing: 0.25,
            isotropic,
        }
        .axes(20.0, 5.0)
    }
    assert_eq!(axes_for(false), (5.0, 1.25), "per axis by default");
    assert_eq!(
        axes_for(true),
        (5.0, 5.0),
        "and the larger axis on both when isotropic"
    );
}

/// An invalid spacing is refused and the layer is untouched.
#[test]
fn an_invalid_spacing_is_refused() {
    let mut editor = Editor::new(Document::new(9, 3).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    let before = pixel(&editor, layer, 4, 1);

    for spacing in [0.0, -1.0, f32::NAN, f32::INFINITY, 10.5] {
        assert!(
            editor
                .execute(Command::BrushStroke {
                    points: vec![
                        BrushPoint::new(1.5, 1.5, 1.0),
                        BrushPoint::new(7.5, 1.5, 1.0),
                    ],
                    color: Pixel::rgba(0, 0, 0, 255),
                    size: 3.0,
                    opacity: 1.0,
                    settings: BrushSettings {
                        spacing: SpacingOptions {
                            spacing,
                            isotropic: false,
                        },
                        ..Default::default()
                    },
                    tip: None,
                    pipe: Vec::new(),
                })
                .is_err(),
            "a spacing of {spacing} must be refused"
        );
    }
    assert_eq!(
        pixel(&editor, layer, 4, 1),
        before,
        "and nothing is painted"
    );
}

/// A stroke that does not move paints one dab, not one per repeated point.
///
/// Krita's own rule: `if (start == end) return -1`, no dab. The previous placer emitted
/// `steps.max(1.0)` per segment, so a hundred identical points painted a hundred dabs on the same spot --
/// wasted work whose only visible effect was on a semi-transparent brush, where it compounded the alpha.
#[test]
fn repeated_identical_points_paint_once() {
    let mut editor = Editor::new(Document::new(5, 5).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    editor
        .execute(Command::BrushStroke {
            points: vec![BrushPoint::new(2.5, 2.5, 1.0); 64],
            color: Pixel::rgba(0, 0, 0, 64),
            size: 1.0,
            opacity: 1.0,
            settings: BrushSettings {
                shape: DabShape {
                    hardness: 1.0,
                    softness: 1.0,
                    ratio: 1.0,
                    antialias_edges: false,
                    pencil: false,
                },
                ..Default::default()
            },
            tip: None,
            pipe: Vec::new(),
        })
        .unwrap();
    let painted = pixel(&editor, layer, 2, 2);
    assert_eq!(
        painted.a, 64,
        "sixty-four identical points must lay down one dab's alpha, not compound it"
    );
}

/// The spacing is omitted from a serialised command when it is the default.
#[test]
fn a_default_spacing_is_omitted_and_a_custom_one_round_trips() {
    let plain = Command::BrushStroke {
        points: vec![BrushPoint::new(1.0, 1.0, 1.0)],
        color: Pixel::rgba(0, 0, 0, 255),
        size: 4.0,
        opacity: 1.0,
        settings: BrushSettings::default(),
        tip: None,
        pipe: Vec::new(),
    };
    let json = serde_json::to_string(&plain).unwrap();
    assert!(
        !json.contains("spacing"),
        "a default spacing must not be written: {json}"
    );
    assert_eq!(serde_json::from_str::<Command>(&json).unwrap(), plain);

    let custom = Command::BrushStroke {
        points: vec![BrushPoint::new(1.0, 1.0, 1.0)],
        color: Pixel::rgba(0, 0, 0, 255),
        size: 4.0,
        opacity: 1.0,
        settings: BrushSettings {
            spacing: SpacingOptions {
                spacing: 1.5,
                isotropic: true,
            },
            ..Default::default()
        },
        tip: None,
        pipe: Vec::new(),
    };
    let json = serde_json::to_string(&custom).unwrap();
    assert!(
        json.contains("isotropic"),
        "a custom spacing must be written: {json}"
    );
    assert_eq!(serde_json::from_str::<Command>(&json).unwrap(), custom);

    // A command written before the field existed loads with the previous behaviour.
    let legacy = r#"{"type":"brush_stroke","points":[{"x":1.0,"y":1.0,"pressure":1.0}],
        "color":{"r":0,"g":0,"b":0,"a":255},"size":4.0,"opacity":1.0,
        "settings":{"smoothing":{"kind":"none"},"mirror_x":null,"mirror_y":null}}"#;
    assert_eq!(serde_json::from_str::<Command>(legacy).unwrap(), plain);
}

/// A bilinear downscale must average the source, not sample one phase of it.
///
/// MEASURED before the filter existed: a one-pixel checkerboard shrunk by four came out with alpha 255
/// everywhere -- fifteen of every sixteen source pixels discarded. The area average is 128.
///
/// The checkerboard is white against TRANSPARENT rather than black, so the aliasing appears in alpha. My
/// first version of this test read the red channel, which is legitimately 255 either way: averaging white
/// with transparent in premultiplied space gives white at half alpha.
#[test]
fn a_bilinear_downscale_averages_instead_of_aliasing() {
    fn shrink_checkerboard(sampling: SamplingMode) -> (u8, u8, usize) {
        const SIZE: u32 = 64;
        let mut editor = Editor::new(Document::new(SIZE, SIZE).unwrap()).unwrap();
        let layer = editor.document().active_layer_id();
        for y in 0..SIZE {
            for x in 0..SIZE {
                if (x + y) % 2 == 0 {
                    editor
                        .execute(Command::BrushStroke {
                            points: vec![BrushPoint::new(x as f32 + 0.5, y as f32 + 0.5, 1.0)],
                            color: Pixel::rgba(255, 255, 255, 255),
                            size: 1.0,
                            opacity: 1.0,
                            settings: BrushSettings {
                                shape: DabShape {
                                    hardness: 1.0,
                                    softness: 1.0,
                                    ratio: 1.0,
                                    antialias_edges: false,
                                    pencil: false,
                                },
                                ..Default::default()
                            },
                            tip: None,
                            pipe: Vec::new(),
                        })
                        .unwrap();
                }
            }
        }
        editor
            .execute(Command::TransformActive {
                transform: Affine2D::new(0.25, 0.0, 0.0, 0.25, 0.0, 0.0),
                sampling,
            })
            .unwrap();

        let mut lowest = 255u8;
        let mut highest = 0u8;
        let mut distinct = std::collections::BTreeSet::new();
        for y in 0..16 {
            for x in 0..16 {
                let alpha = pixel(&editor, layer, x, y).a;
                lowest = lowest.min(alpha);
                highest = highest.max(alpha);
                distinct.insert(alpha);
            }
        }
        (lowest, highest, distinct.len())
    }

    let (low, high, distinct) = shrink_checkerboard(SamplingMode::Bilinear);
    assert!(
        (127..=129).contains(&low) && (127..=129).contains(&high),
        "a filtered quarter-scale of a checkerboard is the area average, about 128; got {low}..{high}"
    );
    assert!(
        distinct <= 2,
        "and it should be nearly uniform, not banded; got {distinct} distinct alphas"
    );

    // Nearest is the contrast, and it must still do what it says: pick one source pixel. That is what the
    // mode is for -- a caller asking for it wants exactly one pixel, usually for pixel art.
    //
    // Measured, and it is a sharper demonstration than a wide range would be: a stride of four over a
    // two-pixel period lands on the SAME phase every time, so nearest returns 255 everywhere. The image is
    // not noisy, it is uniformly wrong -- every white pixel kept and every transparent one discarded. My
    // first version of this test asserted a wide spread and failed against exactly that.
    let (near_low, near_high, _) = shrink_checkerboard(SamplingMode::Nearest);
    assert_eq!(
        (near_low, near_high),
        (255, 255),
        "nearest samples one phase of the checkerboard, so it reports a fully opaque block"
    );
    assert!(
        u32::from(near_high) - u32::from(high) > 100,
        "which is 255 against the filter's {high} -- the error the filter removes"
    );
}

/// Upscaling must not be filtered into mush: the support only widens when shrinking.
#[test]
fn an_upscale_is_not_widened() {
    let mut editor = Editor::new(Document::new(16, 16).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    // A single white pixel at the origin.
    editor
        .execute(Command::BrushStroke {
            points: vec![BrushPoint::new(0.5, 0.5, 1.0)],
            color: Pixel::rgba(255, 255, 255, 255),
            size: 1.0,
            opacity: 1.0,
            settings: BrushSettings {
                shape: DabShape {
                    hardness: 1.0,
                    softness: 1.0,
                    ratio: 1.0,
                    antialias_edges: false,
                    pencil: false,
                },
                ..Default::default()
            },
            tip: None,
            pipe: Vec::new(),
        })
        .unwrap();
    editor
        .execute(Command::TransformActive {
            transform: Affine2D::new(4.0, 0.0, 0.0, 4.0, 0.0, 0.0),
            sampling: SamplingMode::Bilinear,
        })
        .unwrap();
    // The pixel spreads over roughly a 4x4 block; what matters is that it is still opaque somewhere, which
    // a wrongly widened support would have averaged away to nothing.
    let strongest = (0..8)
        .flat_map(|y| (0..8).map(move |x| (x, y)))
        .map(|(x, y)| pixel(&editor, layer, x, y).a)
        .max()
        .unwrap();
    // Measured at 195: a bilinear upscale interpolates the white pixel against its transparent neighbours,
    // so even the strongest destination pixel is short of 255. That is ordinary bilinear behaviour and
    // predates this change; what matters is that the pixel SURVIVES, where a wrongly widened support would
    // have averaged it down to a faint smudge. My first threshold of 200 failed against the correct 195.
    assert!(
        strongest > 150,
        "an upscaled opaque pixel must stay substantially opaque, got {strongest}"
    );
}

/// A shrink of a solid colour is still that colour, at full alpha.
///
/// The normalisation step is what this checks: a triangle over a widened support does not sum to one, so
/// without normalising, a solid region would come out darker or lighter than it was.
#[test]
fn shrinking_a_solid_colour_preserves_it() {
    let mut editor = Editor::new(Document::new(48, 48).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(60, 120, 180, 255),
        })
        .unwrap();
    editor
        .execute(Command::TransformActive {
            transform: Affine2D::new(1.0 / 3.0, 0.0, 0.0, 1.0 / 3.0, 0.0, 0.0),
            sampling: SamplingMode::Bilinear,
        })
        .unwrap();
    // Well inside the shrunk region, away from its edges.
    for (x, y) in [(4u32, 4u32), (8, 8), (11, 11)] {
        let sample = pixel(&editor, layer, x, y);
        assert_eq!(
            sample.a, 255,
            "a solid region must stay fully opaque at {x},{y}"
        );
        assert!(
            sample.r.abs_diff(60) <= 1
                && sample.g.abs_diff(120) <= 1
                && sample.b.abs_diff(180) <= 1,
            "and keep its colour at {x},{y}, got {sample:?}"
        );
    }
}

/// A transparent pixel's colour must not bleed into its neighbours.
///
/// The weights are applied in premultiplied space for this reason. Averaging straight alpha would drag every
/// edge toward whatever colour happens to sit in the fully transparent pixels beside it -- black, in a
/// freshly allocated layer, so every shrunk edge would darken.
#[test]
fn transparent_neighbours_do_not_darken_an_edge() {
    let mut editor = Editor::new(Document::new(32, 32).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    // A bright red block in the top-left quarter, transparent elsewhere.
    editor
        .execute(Command::SelectRectangle {
            rect: Rect::new(0, 0, 16, 16),
            mode: SelectionMode::Replace,
        })
        .unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(255, 0, 0, 255),
        })
        .unwrap();
    editor.execute(Command::ClearSelection).unwrap();
    editor
        .execute(Command::TransformActive {
            transform: Affine2D::new(0.25, 0.0, 0.0, 0.25, 0.0, 0.0),
            sampling: SamplingMode::Bilinear,
        })
        .unwrap();

    // Inside the shrunk block the colour must be the original red, not a darkened one.
    let inside = pixel(&editor, layer, 1, 1);
    assert_eq!(inside.a, 255, "the block's interior stays opaque");
    assert_eq!(
        (inside.r, inside.g, inside.b),
        (255, 0, 0),
        "and keeps its colour rather than mixing with transparent black"
    );
}

/// An incrementally updated projection must be byte-identical to a full render.
///
/// This is the test the whole change rests on. A damage region that is too small leaves stale pixels from an
/// earlier frame, and nothing else in the suite would notice: every other test renders once, where the bug
/// only appears on the SECOND render after a bounded change.
///
/// It walks a long mixed sequence and compares against a freshly built editor replaying the same commands,
/// which has no projection to reuse. Any pixel the incremental path failed to recompute differs here.
#[test]
fn an_incremental_projection_equals_a_full_render() {
    fn sequence(editor: &mut Editor, step: usize) {
        let x = 8.0 + (step % 5) as f32 * 9.0;
        let y = 8.0 + (step % 7) as f32 * 6.0;
        match step % 6 {
            0 => {
                editor
                    .execute(Command::BrushStroke {
                        points: vec![
                            BrushPoint::new(x, y, 0.9),
                            BrushPoint::new(x + 11.0, y + 7.0, 0.5),
                        ],
                        color: Pixel::rgba(220, 40, 40, 255),
                        size: 7.0,
                        opacity: 1.0,
                        settings: BrushSettings::default(),
                        tip: None,
                        pipe: Vec::new(),
                    })
                    .unwrap();
            }
            1 => {
                // A second stroke elsewhere, so two disjoint regions are outstanding at once.
                editor
                    .execute(Command::BrushStroke {
                        points: vec![BrushPoint::new(56.0 - x, 56.0 - y, 1.0)],
                        color: Pixel::rgba(30, 90, 220, 200),
                        size: 5.0,
                        opacity: 0.7,
                        settings: BrushSettings::default(),
                        tip: None,
                        pipe: Vec::new(),
                    })
                    .unwrap();
            }
            2 => {
                // A whole-canvas command, which must report no region and so force a full recomposite.
                editor
                    .execute(Command::ApplyFilter {
                        filter: Filter::Invert,
                    })
                    .unwrap();
            }
            3 => {
                editor
                    .execute(Command::FloodFill {
                        x: 2,
                        y: 2,
                        color: Pixel::rgba(10, 200, 90, 255),
                        options: FloodFillOptions::default(),
                    })
                    .unwrap();
            }
            4 => {
                // Undo, which restores a whole document and must not be trusted to be local.
                let _ = editor.undo();
            }
            _ => {
                editor
                    .execute(Command::SetLayerOpacity {
                        id: editor.document().active_layer_id(),
                        opacity: 0.55,
                    })
                    .unwrap();
            }
        }
    }

    let mut incremental = Editor::new(Document::new(64, 64).unwrap()).unwrap();
    let mut steps = Vec::new();
    for step in 0..24 {
        sequence(&mut incremental, step);
        steps.push(step);
        // Render after EVERY step, which is what builds up a reused projection.
        let live = incremental.render_snapshot().unwrap();

        // A fresh editor replaying the same steps has no projection, so its render is necessarily full.
        let mut fresh = Editor::new(Document::new(64, 64).unwrap()).unwrap();
        for &replay in &steps {
            sequence(&mut fresh, replay);
        }
        let reference = fresh.render_snapshot().unwrap();

        assert_eq!(
            live.pixels(),
            reference.pixels(),
            "step {step}: the incremental projection diverged from a full render"
        );
    }
}

/// Rendering twice with nothing in between must not change the frame.
///
/// The damage is cleared by a successful render, so the second render has nothing to recompute and hands back
/// the projection. If it instead re-cleared the region without recompositing, the frame would go blank.
#[test]
fn a_second_render_with_no_edits_is_unchanged() {
    let mut editor = Editor::new(Document::new(32, 32).unwrap()).unwrap();
    editor
        .execute(Command::BrushStroke {
            points: vec![BrushPoint::new(16.0, 16.0, 1.0)],
            color: Pixel::rgba(255, 128, 0, 255),
            size: 9.0,
            opacity: 1.0,
            settings: BrushSettings::default(),
            tip: None,
            pipe: Vec::new(),
        })
        .unwrap();
    let first = editor.render_snapshot().unwrap();
    let second = editor.render_snapshot().unwrap();
    assert_eq!(
        first.pixels(),
        second.pixels(),
        "an idle render must reproduce the frame, not blank it"
    );
    let third = editor.render_snapshot().unwrap();
    assert_eq!(first.pixels(), third.pixels(), "and stay stable");
}

/// A caller holding the previous frame must not see it change under them.
///
/// The projection is reused in place when it is unshared, so this is the copy-on-write branch: with a retained
/// snapshot the buffer has two owners and must be cloned before the new frame is composited into it.
#[test]
fn a_retained_snapshot_is_not_overwritten() {
    let mut editor = Editor::new(Document::new(32, 32).unwrap()).unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(20, 20, 20, 255),
        })
        .unwrap();
    let held = editor.render_snapshot().unwrap();
    let held_before: Vec<u8> = held.pixels().to_vec();

    // Paint over it and render again while still holding the first frame.
    editor
        .execute(Command::BrushStroke {
            points: vec![BrushPoint::new(16.0, 16.0, 1.0)],
            color: Pixel::rgba(250, 10, 10, 255),
            size: 11.0,
            opacity: 1.0,
            settings: BrushSettings::default(),
            tip: None,
            pipe: Vec::new(),
        })
        .unwrap();
    let next = editor.render_snapshot().unwrap();

    assert_eq!(
        held.pixels(),
        held_before.as_slice(),
        "the retained frame must be exactly what it was when it was taken"
    );
    assert_ne!(
        held.pixels(),
        next.pixels(),
        "and the new frame must actually differ, or this test proves nothing"
    );
}

/// A resize must not reuse a projection sized for the old canvas.
#[test]
fn a_resize_does_not_reuse_the_old_projection() {
    let mut editor = Editor::new(Document::new(32, 32).unwrap()).unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(90, 90, 90, 255),
        })
        .unwrap();
    let before = editor.render_snapshot().unwrap();
    assert_eq!(before.pixels().len(), 32 * 32 * 4);

    editor
        .execute(Command::ResizeCanvas {
            width: 48,
            height: 20,
            sampling: SamplingMode::Nearest,
        })
        .unwrap();
    let after = editor.render_snapshot().unwrap();
    assert_eq!(
        after.pixels().len(),
        48 * 20 * 4,
        "the frame must match the new canvas"
    );

    let mut fresh = Editor::new(Document::new(32, 32).unwrap()).unwrap();
    fresh
        .execute(Command::Fill {
            color: Pixel::rgba(90, 90, 90, 255),
        })
        .unwrap();
    fresh
        .execute(Command::ResizeCanvas {
            width: 48,
            height: 20,
            sampling: SamplingMode::Nearest,
        })
        .unwrap();
    assert_eq!(
        after.pixels(),
        fresh.render_snapshot().unwrap().pixels(),
        "and equal a render that never had an old projection"
    );
}

/// A stroke entirely off the canvas damages nothing and must not blank the frame.
#[test]
fn an_off_canvas_stroke_leaves_the_frame_alone() {
    let mut editor = Editor::new(Document::new(32, 32).unwrap()).unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(200, 200, 40, 255),
        })
        .unwrap();
    let before = editor.render_snapshot().unwrap().pixels().to_vec();

    editor
        .execute(Command::BrushStroke {
            points: vec![BrushPoint::new(-400.0, -400.0, 1.0)],
            color: Pixel::rgba(0, 0, 0, 255),
            size: 4.0,
            opacity: 1.0,
            settings: BrushSettings::default(),
            tip: None,
            pipe: Vec::new(),
        })
        .unwrap();
    let after = editor.render_snapshot().unwrap();
    assert_eq!(
        after.pixels(),
        before.as_slice(),
        "a stroke that touched nothing must leave the frame exactly as it was"
    );
}

/// Brush strokes into an existing cel keep only the damaged rectangle for undo instead of whole
/// documents. This drives the same edits through that path and through the snapshot path (a group
/// per command forces it), and requires the two documents to be identical after every step, undo and
/// redo -- including a stroke onto a frame with no cel, which falls back to the snapshot path, and a
/// navigation away from the stroked frame before undoing it.
#[test]
fn region_undo_of_brush_strokes_matches_snapshot_undo_exactly() {
    let base = Document::new(64, 48).unwrap();
    let mut patched = Editor::new(base.clone()).unwrap();
    let mut snapshot = Editor::new(base).unwrap();
    let stroke = |x: f32, settings: BrushSettings| Command::BrushStroke {
        points: vec![
            BrushPoint::new(x, 10.0, 1.0),
            BrushPoint::new(x + 20.0, 30.0, 0.6),
        ],
        color: Pixel::rgba(30, 140, 220, 200),
        size: 9.0,
        opacity: 0.8,
        settings,
        tip: None,
        pipe: Vec::new(),
    };
    let mirrored = BrushSettings {
        mirror_x: Some(32.0),
        shape: DabShape {
            hardness: 0.2,
            ratio: 0.5,
            ..DabShape::default()
        },
        ..BrushSettings::default()
    };
    let second_frame = FrameId::new(2);
    let edits = vec![
        stroke(4.0, BrushSettings::default()),
        stroke(10.0, mirrored.clone()),
        Command::Fill {
            color: Pixel::rgba(0, 0, 0, 40),
        },
        stroke(20.0, BrushSettings::default()),
        Command::AddFrame {
            id: second_frame,
            index: 1,
        },
    ];
    let run = |editor: &mut Editor, command: Command, grouped: bool| {
        if grouped {
            editor.begin_group("snapshot").unwrap();
        }
        editor.execute(command).unwrap();
        if grouped {
            editor.end_group().unwrap();
        }
    };
    for command in edits {
        run(&mut patched, command.clone(), false);
        run(&mut snapshot, command, true);
        assert_eq!(patched.document(), snapshot.document());
    }
    // A stroke onto the new frame, which has no cel yet: must create it, so it takes the snapshot path.
    for editor in [&mut patched, &mut snapshot] {
        editor
            .navigate(redrob_core::Navigation::SetCurrentFrame { id: second_frame })
            .unwrap();
    }
    run(&mut patched, stroke(30.0, BrushSettings::default()), false);
    run(&mut snapshot, stroke(30.0, BrushSettings::default()), true);
    assert_eq!(patched.document(), snapshot.document());
    // And one more into that now-existing cel, which takes the patch path again.
    run(&mut patched, stroke(2.0, mirrored.clone()), false);
    run(&mut snapshot, stroke(2.0, mirrored), true);
    assert_eq!(patched.document(), snapshot.document());

    // Undo while viewing frame 2 reaches back to strokes on frame 1: a patch is written into the cel
    // it was taken from, not the one on screen.
    let mut undone = 0;
    while snapshot.can_undo() {
        patched.undo().unwrap();
        snapshot.undo().unwrap();
        undone += 1;
        assert_eq!(
            patched.document(),
            snapshot.document(),
            "after undo {undone}"
        );
    }
    assert!(!patched.can_undo());
    let mut redone = 0;
    while snapshot.can_redo() {
        patched.redo().unwrap();
        snapshot.redo().unwrap();
        redone += 1;
        assert_eq!(
            patched.document(),
            snapshot.document(),
            "after redo {redone}"
        );
    }
    assert_eq!(undone, 7);
    assert_eq!(redone, 7);
}

#[test]
fn eraser_mode_removes_paint_by_coverage_and_undoes() {
    let stroke = |x: f32, color: Pixel, opacity: f32, erase: bool| Command::BrushStroke {
        points: vec![BrushPoint::new(x, 8.0, 1.0)],
        color,
        size: 8.0,
        opacity,
        settings: BrushSettings {
            erase,
            ..BrushSettings::default()
        },
        tip: None,
        pipe: Vec::new(),
    };
    let mut editor = Editor::new(Document::new(32, 16).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    editor
        .execute(stroke(8.0, Pixel::rgba(200, 40, 90, 255), 1.0, false))
        .unwrap();
    editor
        .execute(stroke(24.0, Pixel::rgba(200, 40, 90, 255), 1.0, false))
        .unwrap();
    assert_eq!(pixel(&editor, layer, 8, 8), Pixel::rgba(200, 40, 90, 255));

    // A full-strength eraser dab clears the centre completely. The colour it carries is ignored,
    // including a transparent one.
    editor
        .execute(stroke(8.0, Pixel::rgba(0, 0, 0, 0), 1.0, true))
        .unwrap();
    assert_eq!(pixel(&editor, layer, 8, 8), Pixel::TRANSPARENT);
    // Half opacity halves the alpha and keeps the colour (straight alpha).
    editor
        .execute(stroke(24.0, Pixel::rgba(0, 0, 0, 255), 0.5, true))
        .unwrap();
    let half = pixel(&editor, layer, 24, 8);
    assert_eq!((half.r, half.g, half.b), (200, 40, 90));
    assert!(
        (126..=129).contains(&half.a),
        "half-erased alpha {}",
        half.a
    );

    editor.undo().unwrap();
    editor.undo().unwrap();
    assert_eq!(pixel(&editor, layer, 8, 8), Pixel::rgba(200, 40, 90, 255));
    assert_eq!(pixel(&editor, layer, 24, 8), Pixel::rgba(200, 40, 90, 255));
}

#[test]
fn eraser_flag_is_omitted_from_serialised_strokes_when_off() {
    let json = serde_json::to_string(&BrushSettings::default()).unwrap();
    assert!(!json.contains("erase"), "{json}");
    let on = BrushSettings {
        erase: true,
        ..BrushSettings::default()
    };
    let json = serde_json::to_string(&on).unwrap();
    assert!(json.contains("\"erase\":true"), "{json}");
    let back: BrushSettings = serde_json::from_str(&json).unwrap();
    assert!(back.erase);
}

#[test]
fn airbrush_flow_builds_up_with_repeated_dabs() {
    // One point painted N times at a low flow builds alpha up toward opaque: more repeats, more
    // alpha, and a single full-flow dab is darker than one low-flow dab.
    // The airbrush deposits over time as the pointer is held. Model that as `repeats` separate
    // low-flow strokes at the same spot (what the UI's repeat timer produces), and measure the
    // build-up at the centre.
    let stroke = |repeats: usize, flow: Option<f32>| {
        let mut editor = Editor::new(Document::new(16, 16).unwrap()).unwrap();
        let layer = editor.document().active_layer_id();
        for _ in 0..repeats.max(1) {
            editor
                .execute(Command::BrushStroke {
                    points: vec![BrushPoint::new(8.0, 8.0, 1.0)],
                    color: Pixel::rgba(0, 0, 0, 255),
                    size: 8.0,
                    opacity: 1.0,
                    settings: BrushSettings {
                        flow,
                        ..BrushSettings::default()
                    },
                    tip: None,
                    pipe: Vec::new(),
                })
                .unwrap();
        }
        pixel(&editor, layer, 8, 8).a
    };
    let one = stroke(1, Some(0.1));
    let five = stroke(5, Some(0.1));
    let full = stroke(1, None);
    assert!(
        one > 0 && one < full,
        "one low-flow dab is faint: {one} vs {full}"
    );
    assert!(five > one, "repeats build up: {five} vs {one}");
}

#[test]
fn flow_is_omitted_from_serialised_strokes_when_absent() {
    let json = serde_json::to_string(&BrushSettings::default()).unwrap();
    assert!(!json.contains("flow"), "{json}");
    let on = BrushSettings {
        flow: Some(0.1),
        ..BrushSettings::default()
    };
    let json = serde_json::to_string(&on).unwrap();
    assert!(json.contains("flow"), "{json}");
    let back: BrushSettings = serde_json::from_str(&json).unwrap();
    assert_eq!(back.flow, Some(0.1));
}

#[test]
fn smudge_drags_colour_along_the_stroke() {
    // Fill the left half red, then smudge rightward from inside the red into the empty right half.
    // A pixel in the formerly-empty area picks up red that the dab carried from the red region.
    let mut editor = Editor::new(Document::new(40, 12).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    editor
        .execute(Command::BrushStroke {
            points: vec![
                BrushPoint::new(2.0, 6.0, 1.0),
                BrushPoint::new(12.0, 6.0, 1.0),
            ],
            color: Pixel::rgba(220, 20, 20, 255),
            size: 10.0,
            opacity: 1.0,
            settings: BrushSettings::default(),
            tip: None,
            pipe: Vec::new(),
        })
        .unwrap();
    assert_eq!(pixel(&editor, layer, 30, 6).a, 0, "right half starts empty");

    editor
        .execute(Command::BrushStroke {
            points: (0..12)
                .map(|i| BrushPoint::new(10.0 + i as f32 * 3.0, 6.0, 1.0))
                .collect(),
            color: Pixel::rgba(0, 0, 0, 255),
            size: 10.0,
            opacity: 1.0,
            settings: BrushSettings {
                smudge: Some(0.25),
                ..BrushSettings::default()
            },
            tip: None,
            pipe: Vec::new(),
        })
        .unwrap();
    let dragged = pixel(&editor, layer, 20, 6);
    assert!(dragged.a > 0, "smudge carried paint into the empty area");
    assert!(
        dragged.r > dragged.g && dragged.r > dragged.b,
        "and it is reddish: {dragged:?}"
    );
}

#[test]
fn smudge_is_omitted_from_serialised_strokes_when_absent() {
    let json = serde_json::to_string(&BrushSettings::default()).unwrap();
    assert!(!json.contains("smudge"), "{json}");
    let on = BrushSettings {
        smudge: Some(0.25),
        ..BrushSettings::default()
    };
    let json = serde_json::to_string(&on).unwrap();
    assert!(json.contains("smudge"), "{json}");
    let back: BrushSettings = serde_json::from_str(&json).unwrap();
    assert_eq!(back.smudge, Some(0.25));
}

#[test]
fn clone_copies_from_the_source_offset() {
    // Paint a red block on the left. Set a clone offset of 20px right, then paint in the empty right
    // half: each dab copies the pixel 20px to its left, so the red block is reproduced there.
    let mut editor = Editor::new(Document::new(48, 12).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    editor
        .execute(Command::BrushStroke {
            points: vec![BrushPoint::new(6.0, 6.0, 1.0)],
            color: Pixel::rgba(220, 20, 20, 255),
            size: 10.0,
            opacity: 1.0,
            settings: BrushSettings::default(),
            tip: None,
            pipe: Vec::new(),
        })
        .unwrap();
    assert_eq!(
        pixel(&editor, layer, 26, 6).a,
        0,
        "the clone target starts empty"
    );

    editor
        .execute(Command::BrushStroke {
            points: vec![BrushPoint::new(26.0, 6.0, 1.0)],
            color: Pixel::rgba(0, 0, 0, 255),
            size: 10.0,
            opacity: 1.0,
            settings: BrushSettings {
                // Copy from 20px to the left (26 - 20 = 6, the red block's centre).
                clone_offset: Some((20.0, 0.0)),
                ..BrushSettings::default()
            },
            tip: None,
            pipe: Vec::new(),
        })
        .unwrap();
    let cloned = pixel(&editor, layer, 26, 6);
    assert!(cloned.a > 0, "clone reproduced the source");
    assert!(
        cloned.r > 200 && cloned.g < 60,
        "and it is the source red: {cloned:?}"
    );
}

#[test]
fn clone_offset_is_omitted_from_serialised_strokes_when_absent() {
    let json = serde_json::to_string(&BrushSettings::default()).unwrap();
    assert!(!json.contains("clone_offset"), "{json}");
    let on = BrushSettings {
        clone_offset: Some((20.0, -5.0)),
        ..BrushSettings::default()
    };
    let json = serde_json::to_string(&on).unwrap();
    assert!(json.contains("clone_offset"), "{json}");
    let back: BrushSettings = serde_json::from_str(&json).unwrap();
    assert_eq!(back.clone_offset, Some((20.0, -5.0)));
}

#[test]
fn heal_matches_the_patch_to_local_colour() {
    // Source: a green block with a lighter speckle (texture). Destination to heal: a red block.
    // A plain clone would stamp green over red; heal shifts the patch so its mean matches the red,
    // so the healed pixels are reddish (local colour) while keeping the source's light/dark variation.
    let mut editor = Editor::new(Document::new(48, 12).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    // Green source block on the left (x ~ 2..12).
    editor
        .execute(Command::BrushStroke {
            points: vec![BrushPoint::new(6.0, 6.0, 1.0)],
            color: Pixel::rgba(40, 180, 40, 255),
            size: 12.0,
            opacity: 1.0,
            settings: BrushSettings::default(),
            tip: None,
            pipe: Vec::new(),
        })
        .unwrap();
    // Red destination block on the right (x ~ 20..30).
    editor
        .execute(Command::BrushStroke {
            points: vec![BrushPoint::new(26.0, 6.0, 1.0)],
            color: Pixel::rgba(200, 40, 40, 255),
            size: 12.0,
            opacity: 1.0,
            settings: BrushSettings::default(),
            tip: None,
            pipe: Vec::new(),
        })
        .unwrap();

    // Heal the red block from the green source 20px to the left.
    editor
        .execute(Command::BrushStroke {
            points: vec![BrushPoint::new(26.0, 6.0, 1.0)],
            color: Pixel::rgba(0, 0, 0, 255),
            size: 10.0,
            opacity: 1.0,
            settings: BrushSettings {
                clone_offset: Some((20.0, 0.0)),
                heal: true,
                ..BrushSettings::default()
            },
            tip: None,
            pipe: Vec::new(),
        })
        .unwrap();
    let healed = pixel(&editor, layer, 26, 6);
    // Healed with local (red) colour, not the raw green source: red channel dominates.
    assert!(
        healed.r > healed.g,
        "heal took the local red colour, not raw green: {healed:?}"
    );
    assert!(healed.r > 120, "and it is clearly reddish: {healed:?}");
}

#[test]
fn heal_is_omitted_from_serialised_strokes_when_off() {
    let json = serde_json::to_string(&BrushSettings::default()).unwrap();
    assert!(!json.contains("heal"), "{json}");
    let on = BrushSettings {
        heal: true,
        ..BrushSettings::default()
    };
    assert!(serde_json::to_string(&on).unwrap().contains("heal"));
}

#[test]
fn convolve_blurs_and_sharpens_under_the_dab() {
    // A hard edge: left half mid-grey 100, right half mid-grey 160, both opaque. Blur over the
    // boundary pulls the two toward each other; sharpen pushes them apart.
    let make = || {
        let mut editor = Editor::new(Document::new(24, 12).unwrap()).unwrap();
        let layer = editor.document().active_layer_id();
        for y in 0..12 {
            for x in 0..24 {
                let v = if x < 12 { 100 } else { 160 };
                editor
                    .execute(Command::BrushStroke {
                        points: vec![BrushPoint::new(x as f32 + 0.5, y as f32 + 0.5, 1.0)],
                        color: Pixel::rgba(v, v, v, 255),
                        size: 1.5,
                        opacity: 1.0,
                        settings: BrushSettings {
                            shape: redrob_core::DabShape {
                                pencil: true,
                                ..redrob_core::DabShape::default()
                            },
                            ..BrushSettings::default()
                        },
                        tip: None,
                        pipe: Vec::new(),
                    })
                    .unwrap();
            }
        }
        (editor, layer)
    };
    let convolve = |amount: f32| {
        let (mut editor, layer) = make();
        editor
            .execute(Command::BrushStroke {
                points: vec![BrushPoint::new(12.0, 6.0, 1.0)],
                color: Pixel::rgba(0, 0, 0, 255),
                size: 10.0,
                opacity: 1.0,
                settings: BrushSettings {
                    convolve: Some(amount),
                    ..BrushSettings::default()
                },
                tip: None,
                pipe: Vec::new(),
            })
            .unwrap();
        // Just left of the boundary (dark side) and just right (light side).
        (
            pixel(&editor, layer, 11, 6).r,
            pixel(&editor, layer, 12, 6).r,
        )
    };
    let (orig_l, orig_r) = {
        let (editor, layer) = make();
        (
            pixel(&editor, layer, 11, 6).r,
            pixel(&editor, layer, 12, 6).r,
        )
    };
    let (blur_l, blur_r) = convolve(-0.8);
    let (sharp_l, sharp_r) = convolve(0.8);
    let orig_gap = orig_r as i32 - orig_l as i32;
    let blur_gap = blur_r as i32 - blur_l as i32;
    let sharp_gap = sharp_r as i32 - sharp_l as i32;
    assert!(
        blur_gap < orig_gap,
        "blur softens the edge: {blur_gap} < {orig_gap}"
    );
    assert!(
        sharp_gap > orig_gap,
        "sharpen hardens the edge: {sharp_gap} > {orig_gap}"
    );
}

#[test]
fn convolve_is_omitted_from_serialised_strokes_when_absent() {
    let json = serde_json::to_string(&BrushSettings::default()).unwrap();
    assert!(!json.contains("convolve"), "{json}");
    let on = BrushSettings {
        convolve: Some(-0.5),
        ..BrushSettings::default()
    };
    assert!(serde_json::to_string(&on).unwrap().contains("convolve"));
}

#[test]
fn dodge_lightens_and_burn_darkens_in_range() {
    // A flat mid-grey (128). Dodge midtones lightens it; burn midtones darkens it.
    let make = || {
        let mut editor = Editor::new(Document::new(16, 16).unwrap()).unwrap();
        let layer = editor.document().active_layer_id();
        editor
            .execute(Command::Fill {
                color: Pixel::rgba(128, 128, 128, 255),
            })
            .unwrap();
        (editor, layer)
    };
    let apply = |exposure: f32, range: u8| {
        let (mut editor, layer) = make();
        editor
            .execute(Command::BrushStroke {
                points: vec![BrushPoint::new(8.0, 8.0, 1.0)],
                color: Pixel::rgba(0, 0, 0, 255),
                size: 10.0,
                opacity: 1.0,
                settings: BrushSettings {
                    dodge_burn: Some(exposure),
                    dodge_range: Some(range),
                    ..BrushSettings::default()
                },
                tip: None,
                pipe: Vec::new(),
            })
            .unwrap();
        pixel(&editor, layer, 8, 8).r
    };
    assert!(apply(0.5, 1) > 128, "dodge midtones lightens");
    assert!(apply(-0.5, 1) < 128, "burn midtones darkens");
    // Midtone grey (128) is barely touched by the shadows or highlights range (its tonal weight is
    // near zero there), so a midtone dodge moves it more than a shadows dodge does.
    assert!(
        apply(0.5, 1) > apply(0.5, 0),
        "midtone range moves a mid-grey more than shadows range"
    );
}

#[test]
fn dodge_burn_is_omitted_from_serialised_strokes_when_absent() {
    let json = serde_json::to_string(&BrushSettings::default()).unwrap();
    assert!(!json.contains("dodge_burn"), "{json}");
    let on = BrushSettings {
        dodge_burn: Some(0.3),
        dodge_range: Some(2),
        ..BrushSettings::default()
    };
    let json = serde_json::to_string(&on).unwrap();
    assert!(json.contains("dodge_burn") && json.contains("dodge_range"));
}

#[test]
fn ink_thins_the_line_with_speed() {
    // Two strokes over the same path length. The "slow" one has many closely-spaced points (low
    // speed per step); the "fast" one has few far-apart points (high speed). With ink on, the fast
    // stroke's dabs are thinner, so a pixel just off the stroke's centre line is painted by the slow
    // stroke but not the fast one.
    let paint = |step: f32| {
        let mut editor = Editor::new(Document::new(64, 24).unwrap()).unwrap();
        let layer = editor.document().active_layer_id();
        let n = (50.0 / step) as usize;
        let points: Vec<_> = (0..=n)
            .map(|i| BrushPoint::new(6.0 + i as f32 * step, 12.0, 1.0))
            .collect();
        editor
            .execute(Command::BrushStroke {
                points,
                color: Pixel::rgba(0, 0, 0, 255),
                size: 12.0,
                opacity: 1.0,
                settings: BrushSettings {
                    ink: Some(0.9),
                    ..BrushSettings::default()
                },
                tip: None,
                pipe: Vec::new(),
            })
            .unwrap();
        // Alpha 3.5px off the centre line. A full-width dab (radius 6) covers it solidly; a thinned
        // one does not reach. Sampled here rather than at the dab's rim, where BOTH strokes land on
        // the antialiased edge and round to zero — which says nothing about the thinning.
        pixel(&editor, layer, 30, 15).a
    };
    let slow = paint(1.0);
    let fast = paint(10.0);
    assert!(
        slow > fast,
        "ink thins the fast stroke: slow {slow} vs fast {fast}"
    );
}

#[test]
fn ink_is_omitted_from_serialised_strokes_when_absent() {
    let json = serde_json::to_string(&BrushSettings::default()).unwrap();
    assert!(!json.contains("\"ink\""), "{json}");
    let on = BrushSettings {
        ink: Some(0.7),
        ..BrushSettings::default()
    };
    assert!(serde_json::to_string(&on).unwrap().contains("\"ink\""));
}

#[test]
fn mypaint_scatters_dabs_beyond_the_clean_footprint() {
    // A single-point stroke. A clean dab of radius ~5 (size 10) leaves pixel (8,20) -- 14px below
    // the centre -- untouched. With MyPaint offset jitter, sub-dabs scatter outward, so some pixel
    // in a ring just outside the clean radius gets paint. Deterministic, so this is stable.
    let clean = {
        let mut editor = Editor::new(Document::new(32, 32).unwrap()).unwrap();
        let layer = editor.document().active_layer_id();
        editor
            .execute(Command::BrushStroke {
                points: vec![BrushPoint::new(16.0, 16.0, 1.0)],
                color: Pixel::rgba(0, 0, 0, 255),
                size: 10.0,
                opacity: 1.0,
                settings: BrushSettings::default(),
                tip: None,
                pipe: Vec::new(),
            })
            .unwrap();
        // Count painted pixels in a ring 8..12 px from the centre (outside the ~5px clean radius).
        let mut n = 0;
        for y in 0..32 {
            for x in 0..32 {
                let d = (((x as i32 - 16).pow(2) + (y as i32 - 16).pow(2)) as f32).sqrt();
                if (8.0..12.0).contains(&d) && pixel(&editor, layer, x, y).a > 0 {
                    n += 1;
                }
            }
        }
        n
    };
    let scattered = {
        let mut editor = Editor::new(Document::new(32, 32).unwrap()).unwrap();
        let layer = editor.document().active_layer_id();
        editor
            .execute(Command::BrushStroke {
                points: vec![BrushPoint::new(16.0, 16.0, 1.0)],
                color: Pixel::rgba(0, 0, 0, 255),
                size: 10.0,
                opacity: 1.0,
                settings: BrushSettings {
                    mypaint: Some(redrob_core::MyPaintSurface {
                        dabs_per_step: 6,
                        radius_jitter: 0.4,
                        offset_jitter: 1.0,
                    }),
                    ..BrushSettings::default()
                },
                tip: None,
                pipe: Vec::new(),
            })
            .unwrap();
        let mut n = 0;
        for y in 0..32 {
            for x in 0..32 {
                let d = (((x as i32 - 16).pow(2) + (y as i32 - 16).pow(2)) as f32).sqrt();
                if (8.0..12.0).contains(&d) && pixel(&editor, layer, x, y).a > 0 {
                    n += 1;
                }
            }
        }
        n
    };
    assert!(
        scattered > clean,
        "MyPaint scatters paint into the outer ring: {scattered} > {clean}"
    );
}

#[test]
fn mypaint_is_omitted_from_serialised_strokes_when_absent() {
    let json = serde_json::to_string(&BrushSettings::default()).unwrap();
    assert!(!json.contains("mypaint"), "{json}");
    let on = BrushSettings {
        mypaint: Some(redrob_core::MyPaintSurface {
            dabs_per_step: 4,
            radius_jitter: 0.4,
            offset_jitter: 0.6,
        }),
        ..BrushSettings::default()
    };
    assert!(serde_json::to_string(&on).unwrap().contains("mypaint"));
}

#[test]
fn size_dynamics_bind_a_sensor_to_the_dab_size() {
    use redrob_core::{SizeDynamic, SizeSensor};
    // Two single dabs at pressure 0.9 and 0.2, with a strong pressure->size binding. The dab must be
    // wider at high pressure: a pixel 5px off-centre is painted by the high-pressure dab, not the
    // low one.
    let paint = |pressure: f32| {
        let mut editor = Editor::new(Document::new(40, 40).unwrap()).unwrap();
        let layer = editor.document().active_layer_id();
        editor
            .execute(Command::BrushStroke {
                points: vec![BrushPoint::new(20.0, 20.0, pressure)],
                color: Pixel::rgba(0, 0, 0, 255),
                size: 14.0,
                opacity: 1.0,
                settings: BrushSettings {
                    dynamics: vec![SizeDynamic {
                        sensor: SizeSensor::Pressure,
                        amount: 0.9,
                    }],
                    ..BrushSettings::default()
                },
                tip: None,
                pipe: Vec::new(),
            })
            .unwrap();
        pixel(&editor, layer, 25, 20).a
    };
    let high = paint(0.9);
    let low = paint(0.2);
    assert!(
        high > low,
        "pressure dynamic makes the high-pressure dab wider: {high} vs {low}"
    );
}

#[test]
fn dynamics_are_omitted_from_serialised_strokes_when_empty() {
    use redrob_core::{SizeDynamic, SizeSensor};
    let json = serde_json::to_string(&BrushSettings::default()).unwrap();
    assert!(!json.contains("dynamics"), "{json}");
    let on = BrushSettings {
        dynamics: vec![SizeDynamic {
            sensor: SizeSensor::Speed,
            amount: -0.5,
        }],
        ..BrushSettings::default()
    };
    let json = serde_json::to_string(&on).unwrap();
    assert!(json.contains("dynamics") && json.contains("speed"));
}

/// I.1: an opacity binding must change the dab's ALPHA without changing its size. This is the whole
/// point of a separate channel — pressure already drives the diameter, so a size binding alone cannot
/// express "darker when I press harder, same width".
#[test]
fn an_opacity_binding_changes_alpha_without_changing_width() {
    use redrob_core::{BrushDynamic, DynamicSensor};
    // Centre alpha and a 5px-off-centre alpha, for one dab at a given pressure.
    let paint = |pressure: f32, bind_opacity: bool| {
        let mut editor = Editor::new(Document::new(40, 40).unwrap()).unwrap();
        let layer = editor.document().active_layer_id();
        editor
            .execute(Command::BrushStroke {
                points: vec![BrushPoint::new(20.0, 20.0, pressure)],
                color: Pixel::rgba(0, 0, 0, 255),
                size: 14.0,
                opacity: 1.0,
                settings: BrushSettings {
                    opacity_dynamics: if bind_opacity {
                        vec![BrushDynamic {
                            sensor: DynamicSensor::Pressure,
                            amount: 0.9,
                        }]
                    } else {
                        Vec::new()
                    },
                    ..BrushSettings::default()
                },
                tip: None,
                pipe: Vec::new(),
            })
            .unwrap();
        (
            pixel(&editor, layer, 20, 20).a,
            pixel(&editor, layer, 25, 20).a,
        )
    };
    // With the binding on, a hard press is darker at the centre than a light press.
    let (hard_centre, hard_edge) = paint(0.9, true);
    let (soft_centre, _) = paint(0.2, true);
    assert!(
        hard_centre > soft_centre,
        "opacity binding darkens the hard press: {hard_centre} vs {soft_centre}"
    );
    // And the WIDTH is untouched: the same off-centre pixel is covered at the same pressure whether
    // the opacity binding is on or off. If the binding leaked into the size channel this would differ.
    let (_, unbound_edge) = paint(0.9, false);
    assert_eq!(
        hard_edge > 0,
        unbound_edge > 0,
        "an opacity binding must not change which pixels the dab covers"
    );
}

/// I.1: flow and opacity are different quantities. Flow scales what each dab deposits; two passes at
/// half flow build up darker than one, which is exactly what an opacity cap would prevent.
#[test]
fn a_flow_binding_scales_deposition_and_builds_up_over_passes() {
    use redrob_core::{BrushDynamic, DynamicSensor};
    let stroke = |editor: &mut Editor| {
        editor
            .execute(Command::BrushStroke {
                points: vec![BrushPoint::new(20.0, 20.0, 0.5)],
                color: Pixel::rgba(0, 0, 0, 255),
                size: 14.0,
                opacity: 1.0,
                settings: BrushSettings {
                    flow_dynamics: vec![BrushDynamic {
                        sensor: DynamicSensor::Pressure,
                        amount: -0.8,
                    }],
                    ..BrushSettings::default()
                },
                tip: None,
                pipe: Vec::new(),
            })
            .unwrap();
    };
    let mut editor = Editor::new(Document::new(40, 40).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    stroke(&mut editor);
    let one = pixel(&editor, layer, 20, 20).a;
    stroke(&mut editor);
    let two = pixel(&editor, layer, 20, 20).a;
    assert!(one > 0, "a reduced flow still deposits paint: {one}");
    assert!(
        two > one,
        "a second pass at reduced flow builds up: {two} vs {one}"
    );
}

/// I.1: the three channel lists are independent, and each is capped on its own. A binding list that is
/// too long must be refused rather than silently truncated.
#[test]
fn each_channel_list_is_capped_separately() {
    use redrob_core::{BrushDynamic, DynamicSensor};
    let nine = || {
        (0..9)
            .map(|_| BrushDynamic {
                sensor: DynamicSensor::Pressure,
                amount: 0.1,
            })
            .collect::<Vec<_>>()
    };
    for settings in [
        BrushSettings {
            opacity_dynamics: nine(),
            ..BrushSettings::default()
        },
        BrushSettings {
            flow_dynamics: nine(),
            ..BrushSettings::default()
        },
    ] {
        let mut editor = Editor::new(Document::new(8, 8).unwrap()).unwrap();
        let result = editor.execute(Command::BrushStroke {
            points: vec![BrushPoint::new(4.0, 4.0, 1.0)],
            color: Pixel::rgba(0, 0, 0, 255),
            size: 4.0,
            opacity: 1.0,
            settings,
            tip: None,
            pipe: Vec::new(),
        });
        assert!(result.is_err(), "a tenth binding in one channel is refused");
    }
    // Eight in EACH list is fine: the cap is per channel, so adding an opacity binding cannot push an
    // existing size binding out of range.
    let eight = || {
        (0..8)
            .map(|_| BrushDynamic {
                sensor: DynamicSensor::Pressure,
                amount: 0.1,
            })
            .collect::<Vec<_>>()
    };
    let mut editor = Editor::new(Document::new(8, 8).unwrap()).unwrap();
    editor
        .execute(Command::BrushStroke {
            points: vec![BrushPoint::new(4.0, 4.0, 1.0)],
            color: Pixel::rgba(0, 0, 0, 255),
            size: 4.0,
            opacity: 1.0,
            settings: BrushSettings {
                dynamics: eight(),
                opacity_dynamics: eight(),
                flow_dynamics: eight(),
                ..BrushSettings::default()
            },
            tip: None,
            pipe: Vec::new(),
        })
        .unwrap();
}

/// I.1: the new lists are omitted from a serialised stroke when empty, like `dynamics`, so an old
/// project file and a new one are byte-identical when no binding is used.
#[test]
fn channel_bindings_are_omitted_when_empty() {
    use redrob_core::{BrushDynamic, DynamicSensor};
    let json = serde_json::to_string(&BrushSettings::default()).unwrap();
    assert!(!json.contains("opacity_dynamics"), "{json}");
    assert!(!json.contains("flow_dynamics"), "{json}");
    let on = BrushSettings {
        flow_dynamics: vec![BrushDynamic {
            sensor: DynamicSensor::Random,
            amount: 0.25,
        }],
        ..BrushSettings::default()
    };
    let json = serde_json::to_string(&on).unwrap();
    assert!(
        json.contains("flow_dynamics") && json.contains("random"),
        "{json}"
    );
    assert!(!json.contains("opacity_dynamics"), "{json}");
}

/// I.1: a size binding must not change what the opacity channel READS. Both channels read the raw
/// pressure, computed before the size channel overwrites it.
///
/// This cannot be checked by comparing final alphas, and the reason is the engine's own design:
/// `dab.pressure` is a factor in the alpha product as well as in the diameter, so a size binding
/// changes the alpha no matter what the opacity channel does. What must hold is that the opacity
/// binding's own CONTRIBUTION — the ratio it introduces — is the same with and without a size
/// binding. If the opacity channel read the remapped pressure, that ratio would move.
#[test]
fn a_size_binding_does_not_shift_what_the_opacity_channel_reads() {
    use redrob_core::{BrushDynamic, DynamicSensor};
    let centre_alpha = |with_size_binding: bool, with_opacity_binding: bool| {
        let mut editor = Editor::new(Document::new(40, 40).unwrap()).unwrap();
        let layer = editor.document().active_layer_id();
        editor
            .execute(Command::BrushStroke {
                points: vec![BrushPoint::new(20.0, 20.0, 0.8)],
                color: Pixel::rgba(0, 0, 0, 255),
                size: 14.0,
                opacity: 1.0,
                settings: BrushSettings {
                    dynamics: if with_size_binding {
                        vec![BrushDynamic {
                            sensor: DynamicSensor::Pressure,
                            amount: -0.5,
                        }]
                    } else {
                        Vec::new()
                    },
                    // Negative, so the channel stays inside 0..=1 and actually varies: a positive
                    // amount at pressure 0.8 would clamp at 1.0 and the test would prove nothing.
                    opacity_dynamics: if with_opacity_binding {
                        vec![BrushDynamic {
                            sensor: DynamicSensor::Pressure,
                            amount: -0.5,
                        }]
                    } else {
                        Vec::new()
                    },
                    ..BrushSettings::default()
                },
                tip: None,
                pipe: Vec::new(),
            })
            .unwrap();
        f64::from(pixel(&editor, layer, 20, 20).a)
    };
    // The opacity binding's effect, measured twice: once on a stroke with no size binding, once on a
    // stroke whose pressure the size channel has pushed from 0.8 down to 0.65.
    let plain = centre_alpha(false, false);
    let plain_with_opacity = centre_alpha(false, true);
    let sized = centre_alpha(true, false);
    let sized_with_opacity = centre_alpha(true, true);
    assert!(plain > 0.0 && sized > 0.0, "both strokes paint the centre");
    let effect_alone = plain_with_opacity / plain;
    let effect_with_size = sized_with_opacity / sized;
    assert!(
        (effect_alone - effect_with_size).abs() < 0.02,
        "the opacity channel must read raw pressure: its effect was {effect_alone} alone \
         but {effect_with_size} beside a size binding"
    );
    // And the binding is doing something at all, so the assertion above is not satisfied by two
    // identical no-ops.
    assert!(
        effect_alone < 0.95,
        "a negative opacity binding lightens the dab, got ratio {effect_alone}"
    );
}

#[test]
fn gih_pipe_cycles_tip_frames_per_dab() {
    // Build two 4x4 GBR tips: frame A solid on its LEFT half, frame B solid on its RIGHT half.
    // A pipe of [A, B] stamped along a stroke alternates them per dab, so both a left-covering and a
    // right-covering dab appear -- which a single tip could not produce.
    let gbr = |left: bool| {
        let mut payload = [0u8; 16];
        for y in 0..4 {
            for x in 0..4 {
                let solid = if left { x < 2 } else { x >= 2 };
                if solid {
                    payload[y * 4 + x] = 255;
                }
            }
        }
        let mut data = Vec::new();
        for field in [(28u32 + 4), 2u32, 4u32, 4u32, 1u32] {
            data.extend_from_slice(&field.to_be_bytes());
        }
        data.extend_from_slice(b"GIMP");
        data.extend_from_slice(&10u32.to_be_bytes());
        data.extend_from_slice(b"hal\0");
        data.extend_from_slice(&payload);
        redrob_core::BrushTip::from_gbr(&data).expect("tip decodes")
    };
    let frame_a = gbr(true);
    let frame_b = gbr(false);

    let mut editor = Editor::new(Document::new(64, 16).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    // Several dabs across the row; the pipe [A, B] cycles A,B,A,B,...
    editor
        .execute(Command::BrushStroke {
            points: (0..6)
                .map(|i| BrushPoint::new(8.0 + i as f32 * 8.0, 8.0, 1.0))
                .collect(),
            color: Pixel::rgba(0, 0, 0, 255),
            size: 8.0,
            opacity: 1.0,
            settings: BrushSettings::default(),
            tip: None,
            pipe: vec![frame_a, frame_b],
        })
        .unwrap();
    // Somewhere a left-half-covering dab (frame A) and a right-half-covering dab (frame B) both
    // painted, so the row has painted pixels from both frame orientations. The row should not be
    // empty, which it would be if the pipe were ignored and no single tip was set.
    let painted = (0..64)
        .filter(|&x| pixel(&editor, layer, x, 8).a > 0)
        .count();
    assert!(painted > 0, "the pipe stamped its frames");
}

#[test]
fn pipe_is_omitted_from_serialised_strokes_when_empty() {
    let stroke = Command::BrushStroke {
        points: vec![BrushPoint::new(1.0, 1.0, 1.0)],
        color: Pixel::rgba(0, 0, 0, 255),
        size: 4.0,
        opacity: 1.0,
        settings: BrushSettings::default(),
        tip: None,
        pipe: Vec::new(),
    };
    assert!(!serde_json::to_string(&stroke).unwrap().contains("pipe"));
}

#[test]
fn free_polygon_selection_fills_the_lasso_region() {
    // A triangle covering the lower-left of a 20x20 canvas. A point well inside is selected, a point
    // outside is not.
    let mut editor = Editor::new(Document::new(20, 20).unwrap()).unwrap();
    editor
        .execute(Command::SelectPolygon {
            points: vec![(1.0, 1.0), (18.0, 1.0), (1.0, 18.0)],
            mode: SelectionMode::Replace,
        })
        .unwrap();
    let sel = editor.document().selection();
    // (3,3) is well inside the triangle; (17,17) is outside it.
    let coverage = |x: u32, y: u32| sel.coverage(x, y);
    assert!(
        coverage(3, 3) > 200,
        "inside the lasso is selected: {}",
        coverage(3, 3)
    );
    assert_eq!(coverage(17, 17), 0, "outside the lasso is not");
}

#[test]
fn free_polygon_with_fewer_than_three_points_selects_nothing() {
    let mut editor = Editor::new(Document::new(10, 10).unwrap()).unwrap();
    editor
        .execute(Command::SelectPolygon {
            points: vec![(1.0, 1.0), (5.0, 5.0)],
            mode: SelectionMode::Replace,
        })
        .unwrap();
    let sel = editor.document().selection();
    for y in 0..10 {
        for x in 0..10 {
            assert_eq!(sel.coverage(x, y), 0);
        }
    }
}

#[test]
fn magic_wand_selects_by_colour_contiguous_and_global() {
    // Left third red, middle third green, right third red again (two disconnected red regions).
    let mut editor = Editor::new(Document::new(30, 10).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    let paint = |ed: &mut Editor, x: u32, color: Pixel| {
        for px in x..x + 10 {
            for py in 0..10 {
                ed.execute(Command::BrushStroke {
                    points: vec![BrushPoint::new(px as f32 + 0.5, py as f32 + 0.5, 1.0)],
                    color,
                    size: 1.5,
                    opacity: 1.0,
                    settings: BrushSettings {
                        shape: redrob_core::DabShape {
                            pencil: true,
                            ..redrob_core::DabShape::default()
                        },
                        ..BrushSettings::default()
                    },
                    tip: None,
                    pipe: Vec::new(),
                })
                .unwrap();
            }
        }
    };
    let red = Pixel::rgba(220, 20, 20, 255);
    let green = Pixel::rgba(20, 200, 20, 255);
    paint(&mut editor, 0, red);
    paint(&mut editor, 10, green);
    paint(&mut editor, 20, red);
    let _ = layer;

    // Contiguous wand from the LEFT red block selects only it, not the right red block.
    editor
        .execute(Command::SelectByColor {
            x: 5,
            y: 5,
            tolerance: 20,
            contiguous: true,
            mode: SelectionMode::Replace,
        })
        .unwrap();
    let sel = editor.document().selection();
    assert!(sel.coverage(5, 5) > 0, "left red selected");
    assert_eq!(
        sel.coverage(25, 5),
        0,
        "right red NOT selected (not contiguous)"
    );

    // Global by-colour from the left red selects BOTH red blocks.
    editor
        .execute(Command::SelectByColor {
            x: 5,
            y: 5,
            tolerance: 20,
            contiguous: false,
            mode: SelectionMode::Replace,
        })
        .unwrap();
    let sel = editor.document().selection();
    assert!(
        sel.coverage(5, 5) > 0 && sel.coverage(25, 5) > 0,
        "both red blocks selected"
    );
    assert_eq!(sel.coverage(15, 5), 0, "the green middle is not");
}

#[test]
fn intelligent_scissors_traces_a_boundary_and_selects() {
    // An image split by a hard vertical edge at x=15 (left black, right white). Anchors around a
    // box; the scissors boundary snaps to the edge and the enclosed region selects. We assert the
    // trace produced a non-empty selection that includes a point inside the anchor box.
    let mut editor = Editor::new(Document::new(30, 20).unwrap()).unwrap();
    let _layer = editor.document().active_layer_id();
    for px in 0..30u32 {
        for py in 0..20u32 {
            let v: u8 = if px < 15 { 20 } else { 230 };
            editor
                .execute(Command::BrushStroke {
                    points: vec![BrushPoint::new(px as f32 + 0.5, py as f32 + 0.5, 1.0)],
                    color: Pixel::rgba(v, v, v, 255),
                    size: 1.5,
                    opacity: 1.0,
                    settings: BrushSettings {
                        shape: redrob_core::DabShape {
                            pencil: true,
                            ..redrob_core::DabShape::default()
                        },
                        ..BrushSettings::default()
                    },
                    tip: None,
                    pipe: Vec::new(),
                })
                .unwrap();
        }
    }
    editor
        .execute(Command::SelectScissors {
            anchors: vec![(5, 3), (25, 3), (25, 16), (5, 16)],
            mode: SelectionMode::Replace,
        })
        .unwrap();
    let sel = editor.document().selection();
    assert!(sel.is_active(), "scissors produced a selection");
    assert!(
        sel.coverage(15, 10) > 0,
        "a point inside the anchor box is selected"
    );
}

#[test]
fn magnetic_boundary_follows_a_strong_edge() {
    // A 20x20 image with a hard edge at x=10. A live wire between two anchors on the SAME side of the
    // edge, offset vertically, should stay cheap by running near the edge; the returned path is
    // non-empty and starts/ends at the anchors.
    let mut px = vec![0u8; 20 * 20 * 4];
    for y in 0..20usize {
        for x in 0..20usize {
            let v: u8 = if x < 10 { 10 } else { 240 };
            let o = (y * 20 + x) * 4;
            px[o] = v;
            px[o + 1] = v;
            px[o + 2] = v;
            px[o + 3] = 255;
        }
    }
    let path = redrob_core::scissors_magnetic_boundary(&px, 20, 20, &[(10, 2), (10, 17)], 100_000);
    assert!(path.len() >= 2, "a path was traced");
    assert_eq!(path.first().copied(), Some((10.0, 2.0)));
}

#[test]
fn foreground_select_classifies_by_sampled_colour() {
    // Left half red (subject), right half blue (background). Scribble fg on the red, bg on the blue;
    // red pixels are selected, blue pixels are not.
    let mut editor = Editor::new(Document::new(20, 10).unwrap()).unwrap();
    let _layer = editor.document().active_layer_id();
    for px in 0..20u32 {
        for py in 0..10u32 {
            let color = if px < 10 {
                Pixel::rgba(220, 20, 20, 255)
            } else {
                Pixel::rgba(20, 20, 220, 255)
            };
            editor
                .execute(Command::BrushStroke {
                    points: vec![BrushPoint::new(px as f32 + 0.5, py as f32 + 0.5, 1.0)],
                    color,
                    size: 1.5,
                    opacity: 1.0,
                    settings: BrushSettings {
                        shape: redrob_core::DabShape {
                            pencil: true,
                            ..redrob_core::DabShape::default()
                        },
                        ..BrushSettings::default()
                    },
                    tip: None,
                    pipe: Vec::new(),
                })
                .unwrap();
        }
    }
    editor
        .execute(Command::SelectForeground {
            fg: vec![(3, 5), (5, 5)],
            bg: vec![(15, 5), (17, 5)],
            mode: SelectionMode::Replace,
        })
        .unwrap();
    let sel = editor.document().selection();
    assert!(
        sel.coverage(5, 5) > 200,
        "the red subject is selected: {}",
        sel.coverage(5, 5)
    );
    assert_eq!(sel.coverage(15, 5), 0, "the blue background is not");
}

#[test]
fn align_layers_centres_a_block_on_the_canvas() {
    // A 4x4 opaque block painted in the top-left of a 40x40 canvas. Align it to the canvas centre:
    // its bounds move so the block straddles the middle (around x=18..22).
    let mut editor = Editor::new(Document::new(40, 40).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    for px in 2..6u32 {
        for py in 2..6u32 {
            editor
                .execute(Command::BrushStroke {
                    points: vec![BrushPoint::new(px as f32 + 0.5, py as f32 + 0.5, 1.0)],
                    color: Pixel::rgba(10, 10, 10, 255),
                    size: 1.5,
                    opacity: 1.0,
                    settings: BrushSettings {
                        shape: redrob_core::DabShape {
                            pencil: true,
                            ..redrob_core::DabShape::default()
                        },
                        ..BrushSettings::default()
                    },
                    tip: None,
                    pipe: Vec::new(),
                })
                .unwrap();
        }
    }
    // Confirm it starts in the corner, not the middle.
    assert_eq!(
        pixel(&editor, layer, 20, 20).a,
        0,
        "the block starts off-centre"
    );
    editor
        .execute(Command::AlignLayers {
            ids: vec![layer],
            h: 2,
            v: 2,
            to_canvas: true,
        })
        .unwrap();
    // After centring, the 4x4 block covers roughly the canvas middle (18..22).
    assert!(
        pixel(&editor, layer, 19, 19).a > 0,
        "the block is now centred"
    );
    assert_eq!(
        pixel(&editor, layer, 3, 3).a,
        0,
        "and no longer in the corner"
    );
}

#[test]
fn perspective_identity_is_a_no_op_and_keystone_warps() {
    // Fill the layer solid, then an identity perspective (corners = the full rect) leaves a sampled
    // pixel unchanged, while a keystone (top edge pulled inward) leaves the top-left corner empty.
    let make = || {
        let mut editor = Editor::new(Document::new(40, 40).unwrap()).unwrap();
        let layer = editor.document().active_layer_id();
        editor
            .execute(Command::Fill {
                color: Pixel::rgba(10, 120, 200, 255),
            })
            .unwrap();
        (editor, layer)
    };
    let (mut editor, layer) = make();
    editor
        .execute(Command::PerspectiveActive {
            corners: [(0.0, 0.0), (40.0, 0.0), (40.0, 40.0), (0.0, 40.0)],
            sampling: redrob_core::SamplingMode::Nearest,
        })
        .unwrap();
    assert!(
        pixel(&editor, layer, 20, 20).a > 0,
        "identity keeps the fill"
    );

    let (mut editor, layer) = make();
    editor
        .execute(Command::PerspectiveActive {
            // Top edge pulled in to 10..30; the top-left corner of the canvas is now outside the quad.
            corners: [(10.0, 0.0), (30.0, 0.0), (40.0, 40.0), (0.0, 40.0)],
            sampling: redrob_core::SamplingMode::Nearest,
        })
        .unwrap();
    assert_eq!(
        pixel(&editor, layer, 1, 1).a,
        0,
        "the keystone emptied the top-left corner"
    );
    assert!(
        pixel(&editor, layer, 20, 38).a > 0,
        "the wide bottom stays filled"
    );
}

#[test]
fn cage_identity_preserves_pixels_and_stretch_moves_content() {
    // A solid fill. An identity cage (dst == src) leaves the interior unchanged; stretching the cage
    // to the right pulls content past the old right edge.
    let make = || {
        let mut editor = Editor::new(Document::new(40, 40).unwrap()).unwrap();
        let layer = editor.document().active_layer_id();
        editor
            .execute(Command::Fill {
                color: Pixel::rgba(30, 180, 90, 255),
            })
            .unwrap();
        (editor, layer)
    };
    let square = vec![(8.0, 8.0), (24.0, 8.0), (24.0, 24.0), (8.0, 24.0)];

    let (mut editor, layer) = make();
    editor
        .execute(Command::CageTransform {
            src_cage: square.clone(),
            dst_cage: square.clone(),
            sampling: redrob_core::SamplingMode::Bilinear,
        })
        .unwrap();
    assert!(
        pixel(&editor, layer, 16, 16).a > 0,
        "identity cage keeps the centre filled"
    );

    let (mut editor, layer) = make();
    // Push the two right vertices out to x=34; the cage interior now reaches past x=24.
    let stretched = vec![(8.0, 8.0), (34.0, 8.0), (34.0, 24.0), (8.0, 24.0)];
    editor
        .execute(Command::CageTransform {
            src_cage: square.clone(),
            dst_cage: stretched,
            sampling: redrob_core::SamplingMode::Bilinear,
        })
        .unwrap();
    assert!(
        pixel(&editor, layer, 30, 16).a > 0,
        "the stretched cage carries fill past the old edge"
    );
}

#[test]
fn warp_grow_expands_an_edge_outward() {
    // A grow warp magnifies ABOUT ITS CENTRE: a destination pixel reads its source from closer in, so
    // content moves outward. The subject therefore has to be a shape the centre sits INSIDE.
    //
    // Centring the grow on a straight boundary does nothing to that boundary, and it is worth saying
    // why rather than rediscovering it: the displacement is radial, so every point ON a line through
    // the centre moves ALONG that line. A dest pixel right of the edge would need its source from
    // left of the edge, which means a negative radius. No strength reaches it.
    //
    // So: an opaque band from x=16..24 in a transparent field, grown about its own centre. A pixel
    // outside the old band becomes opaque because it now samples from inside it.
    let mut editor = Editor::new(Document::new(40, 40).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    editor
        .execute(Command::SelectRectangle {
            rect: redrob_core::Rect {
                x: 16,
                y: 0,
                width: 8,
                height: 40,
            },
            mode: redrob_core::SelectionMode::Replace,
        })
        .unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(200, 40, 40, 255),
        })
        .unwrap();
    editor.execute(Command::SelectAll).unwrap();
    assert_eq!(
        pixel(&editor, layer, 26, 20).a,
        0,
        "outside the band starts transparent"
    );
    editor
        .execute(Command::WarpBrush {
            points: vec![(20.0, 20.0)],
            mode: redrob_core::WarpMode::Grow,
            radius: 16.0,
            strength: 0.8,
            sampling: redrob_core::SamplingMode::Bilinear,
        })
        .unwrap();
    assert!(
        pixel(&editor, layer, 26, 20).a > 0,
        "grow magnified the band past its old edge"
    );
}

#[test]
fn npoint_corners_pinned_centre_moved_warps() {
    // Four corners pinned (src == dst) and a centre control point dragged down-right pulls the
    // centre content. A small dark mark at the centre moves off its original spot.
    let mut editor = Editor::new(Document::new(40, 40).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(220, 220, 220, 255),
        })
        .unwrap();
    // A 2x2 dark mark at (19,19)-(20,20).
    for px in 19..21u32 {
        for py in 19..21u32 {
            editor
                .execute(Command::BrushStroke {
                    points: vec![BrushPoint::new(px as f32 + 0.5, py as f32 + 0.5, 1.0)],
                    color: Pixel::rgba(0, 0, 0, 255),
                    size: 1.5,
                    opacity: 1.0,
                    settings: BrushSettings {
                        shape: redrob_core::DabShape {
                            pencil: true,
                            ..redrob_core::DabShape::default()
                        },
                        ..BrushSettings::default()
                    },
                    tip: None,
                    pipe: Vec::new(),
                })
                .unwrap();
        }
    }
    let src = vec![
        (0.0, 0.0),
        (40.0, 0.0),
        (40.0, 40.0),
        (0.0, 40.0),
        (20.0, 20.0),
    ];
    // Centre control moves to (26,26); corners stay.
    let dst = vec![
        (0.0, 0.0),
        (40.0, 0.0),
        (40.0, 40.0),
        (0.0, 40.0),
        (26.0, 26.0),
    ];
    editor
        .execute(Command::NPointTransform {
            src_pts: src,
            dst_pts: dst,
            sampling: redrob_core::SamplingMode::Bilinear,
        })
        .unwrap();
    // The dark mark now sits near the dragged destination, not the original centre.
    assert!(
        pixel(&editor, layer, 26, 26).r < 128,
        "the mark followed the dragged control point"
    );
}

#[test]
fn transform3d_zero_is_identity_and_y_rotation_warps() {
    // Zero rotation leaves a solid fill unchanged; a Y-axis rotation projects the layer into a
    // trapezoid, so a corner near the receding edge empties.
    let make = || {
        let mut editor = Editor::new(Document::new(40, 40).unwrap()).unwrap();
        let layer = editor.document().active_layer_id();
        editor
            .execute(Command::Fill {
                color: Pixel::rgba(60, 120, 200, 255),
            })
            .unwrap();
        (editor, layer)
    };
    let (mut editor, layer) = make();
    editor
        .execute(Command::Transform3d {
            rot_x: 0.0,
            rot_y: 0.0,
            rot_z: 0.0,
            distance: 2.0,
            sampling: redrob_core::SamplingMode::Bilinear,
        })
        .unwrap();
    assert!(
        pixel(&editor, layer, 20, 20).a > 0,
        "no rotation keeps the fill"
    );

    let (mut editor, layer) = make();
    editor
        .execute(Command::Transform3d {
            rot_x: 0.0,
            rot_y: 0.9,
            rot_z: 0.0,
            distance: 2.0,
            sampling: redrob_core::SamplingMode::Bilinear,
        })
        .unwrap();
    // The receding side of the rotated plane no longer covers the far corners.
    let far_empty = pixel(&editor, layer, 1, 1).a == 0 || pixel(&editor, layer, 38, 1).a == 0;
    assert!(far_empty, "the Y rotation left a receding corner uncovered");
    assert!(
        pixel(&editor, layer, 20, 20).a > 0,
        "but the centre is still covered"
    );
}

#[test]
fn multihand_symmetry_paints_rotated_copies() {
    // A single dab near the top with 4-fold symmetry about the canvas centre should also mark the
    // bottom (180 deg rotation of the top point).
    let mut editor = Editor::new(Document::new(40, 40).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    editor
        .execute(Command::BrushStroke {
            points: vec![BrushPoint::new(20.0, 6.0, 1.0)],
            color: Pixel::rgba(0, 0, 0, 255),
            size: 4.0,
            opacity: 1.0,
            settings: BrushSettings {
                symmetry_center: Some((20.0, 20.0)),
                symmetry_order: 4,
                ..BrushSettings::default()
            },
            tip: None,
            pipe: Vec::new(),
        })
        .unwrap();
    // The original top mark.
    assert!(
        pixel(&editor, layer, 20, 6).a > 0,
        "the original dab painted"
    );
    // The 180-degree copy at the bottom (y = 2*20 - 6 = 34).
    assert!(
        pixel(&editor, layer, 20, 34).a > 0,
        "a rotated copy painted at the bottom"
    );
    // And the two side copies (90 and 270 deg) at (6,20) and (34,20).
    assert!(
        pixel(&editor, layer, 6, 20).a > 0,
        "a rotated copy painted on the left"
    );
}

#[test]
fn assistant_parallel_ruler_snaps_the_stroke_straight() {
    // A ruler along the horizontal line y=10. A stroke that wanders in y should paint only on that
    // line: a point drawn at (20, 30) snaps to y=10, so (20,30) stays empty but (20,10) is painted.
    let mut editor = Editor::new(Document::new(40, 40).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    editor
        .execute(Command::BrushStroke {
            points: vec![
                BrushPoint::new(5.0, 10.0, 1.0),
                BrushPoint::new(20.0, 30.0, 1.0),
                BrushPoint::new(35.0, 10.0, 1.0),
            ],
            color: Pixel::rgba(0, 0, 0, 255),
            size: 3.0,
            opacity: 1.0,
            settings: BrushSettings {
                assistant: Some(redrob_core::BrushAssistant::ParallelRuler {
                    ax: 0.0,
                    ay: 10.0,
                    bx: 40.0,
                    by: 10.0,
                }),
                ..BrushSettings::default()
            },
            tip: None,
            pipe: Vec::new(),
        })
        .unwrap();
    assert!(
        pixel(&editor, layer, 20, 10).a > 0,
        "the wandering point snapped onto the ruler"
    );
    assert_eq!(
        pixel(&editor, layer, 20, 30).a,
        0,
        "and nothing painted off the ruler"
    );
}

#[test]
fn enclose_and_fill_fills_a_closed_ring_interior() {
    // Draw an opaque ring (a hollow box outline), then enclose-and-fill the bounding rectangle: the
    // transparent hole inside the ring fills, but the transparent area outside the ring does not.
    let mut editor = Editor::new(Document::new(40, 40).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    // Opaque border of a 20x20 box from (10,10) to (29,29): four one-pixel edges.
    let edges = |x: u32, y: u32| x == 10 || x == 29 || y == 10 || y == 29;
    for x in 10..30u32 {
        for y in 10..30u32 {
            if !edges(x, y) {
                continue;
            }
            editor
                .execute(Command::BrushStroke {
                    points: vec![BrushPoint::new(x as f32 + 0.5, y as f32 + 0.5, 1.0)],
                    color: Pixel::rgba(0, 0, 0, 255),
                    size: 1.5,
                    opacity: 1.0,
                    settings: BrushSettings {
                        shape: redrob_core::DabShape {
                            pencil: true,
                            ..redrob_core::DabShape::default()
                        },
                        ..BrushSettings::default()
                    },
                    tip: None,
                    pipe: Vec::new(),
                })
                .unwrap();
        }
    }
    editor
        .execute(Command::EncloseAndFill {
            rect: redrob_core::Rect {
                x: 0,
                y: 0,
                width: 40,
                height: 40,
            },
            color: Pixel::rgba(200, 0, 0, 255),
            alpha_threshold: 8,
        })
        .unwrap();
    assert!(
        pixel(&editor, layer, 20, 20).a > 0,
        "the inside of the ring filled"
    );
    assert_eq!(pixel(&editor, layer, 2, 2).a, 0, "the outside stayed empty");
}

#[test]
fn smart_patch_fills_a_selected_hole_from_surroundings() {
    // Solid fill, select a small square, clear it to a hole, then smart-patch pulls the surrounding
    // colour back into the hole.
    let mut editor = Editor::new(Document::new(40, 40).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(40, 160, 90, 255),
        })
        .unwrap();
    editor
        .execute(Command::SelectRectangle {
            rect: redrob_core::Rect {
                x: 18,
                y: 18,
                width: 4,
                height: 4,
            },
            mode: redrob_core::SelectionMode::Replace,
        })
        .unwrap();
    editor.execute(Command::Clear).unwrap();
    assert_eq!(pixel(&editor, layer, 19, 19).a, 0, "the hole is cleared");
    editor
        .execute(Command::SmartPatch { search_radius: 32 })
        .unwrap();
    let p = pixel(&editor, layer, 19, 19);
    assert!(
        p.a > 0 && p.g > p.r,
        "the hole was patched with surrounding green"
    );
}

#[test]
fn lazybrush_colours_two_regions_split_by_a_line() {
    // A vertical black line down the middle splits the canvas. A red scribble on the left and a blue
    // scribble on the right should colour their own sides without bleeding across the line.
    let mut editor = Editor::new(Document::new(40, 40).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(255, 255, 255, 255),
        })
        .unwrap();
    // Black divider at x=20.
    for y in 0..40u32 {
        editor
            .execute(Command::BrushStroke {
                points: vec![BrushPoint::new(20.5, y as f32 + 0.5, 1.0)],
                color: Pixel::rgba(0, 0, 0, 255),
                size: 1.5,
                opacity: 1.0,
                settings: BrushSettings {
                    shape: redrob_core::DabShape {
                        pencil: true,
                        ..redrob_core::DabShape::default()
                    },
                    ..BrushSettings::default()
                },
                tip: None,
                pipe: Vec::new(),
            })
            .unwrap();
    }
    editor
        .execute(Command::Lazybrush {
            scribbles: vec![
                (6, 20, Pixel::rgba(220, 0, 0, 255)),
                (33, 20, Pixel::rgba(0, 0, 220, 255)),
            ],
        })
        .unwrap();
    let left = pixel(&editor, layer, 6, 20);
    let right = pixel(&editor, layer, 33, 20);
    assert!(left.r > left.b, "the left side took the red scribble");
    assert!(right.b > right.r, "the right side took the blue scribble");
}

#[test]
fn dyna_brush_rounds_a_sharp_corner() {
    // A right-angle stroke (down then right). With the dyna brush the dab lags, so the inner corner
    // pixel is NOT painted the way a rigid brush would paint it — the mass rounds the turn.
    let stroke = |dyna: Option<(f32, f32)>| {
        let mut editor = Editor::new(Document::new(60, 60).unwrap()).unwrap();
        let layer = editor.document().active_layer_id();
        editor
            .execute(Command::BrushStroke {
                points: vec![
                    BrushPoint::new(10.0, 10.0, 1.0),
                    BrushPoint::new(10.0, 40.0, 1.0),
                    BrushPoint::new(40.0, 40.0, 1.0),
                ],
                color: Pixel::rgba(0, 0, 0, 255),
                size: 3.0,
                opacity: 1.0,
                settings: BrushSettings {
                    dyna,
                    ..BrushSettings::default()
                },
                tip: None,
                pipe: Vec::new(),
            })
            .unwrap();
        (editor, layer)
    };
    // Rigid stroke hits the exact corner (10,40).
    let (rigid, rl) = stroke(None);
    assert!(
        pixel(&rigid, rl, 10, 40).a > 0,
        "the rigid brush paints the sharp corner"
    );
    // Dyna stroke overshoots/rounds, so the exact corner is lighter or empty.
    let (dynb, dl) = stroke(Some((0.9, 0.1)));
    assert!(
        pixel(&dynb, dl, 10, 40).a <= pixel(&rigid, rl, 10, 40).a,
        "the dyna brush does not paint the corner harder than the rigid one"
    );
}

#[test]
fn motion_blur_smears_horizontally_and_lens_blur_spreads_a_disc() {
    // A single bright dot. Motion blur at 0 degrees smears it along x (a pixel to the side lights up)
    // but not along y; lens blur spreads it in both directions.
    let dot = || {
        let mut editor = Editor::new(Document::new(40, 40).unwrap()).unwrap();
        let layer = editor.document().active_layer_id();
        editor
            .execute(Command::BrushStroke {
                points: vec![BrushPoint::new(20.5, 20.5, 1.0)],
                color: Pixel::rgba(255, 255, 255, 255),
                size: 2.0,
                opacity: 1.0,
                settings: BrushSettings::default(),
                tip: None,
                pipe: Vec::new(),
            })
            .unwrap();
        (editor, layer)
    };
    let (mut editor, layer) = dot();
    editor
        .execute(Command::ApplyFilter {
            filter: redrob_core::Filter::MotionBlur {
                angle_degrees: 0.0,
                distance: 12,
            },
        })
        .unwrap();
    assert!(
        pixel(&editor, layer, 26, 20).a > 0,
        "motion blur smeared along x"
    );

    let (mut editor, layer) = dot();
    editor
        .execute(Command::ApplyFilter {
            filter: redrob_core::Filter::LensBlur { radius: 6 },
        })
        .unwrap();
    assert!(
        pixel(&editor, layer, 24, 20).a > 0 && pixel(&editor, layer, 20, 24).a > 0,
        "lens blur spread the dot in both directions"
    );
}

#[test]
fn edge_laplace_light_boundaries_emboss_greys_flat() {
    // Left half black, right half white: the only edge is the vertical boundary at x=20.
    let make = || {
        let mut editor = Editor::new(Document::new(40, 40).unwrap()).unwrap();
        let layer = editor.document().active_layer_id();
        editor
            .execute(Command::Fill {
                color: Pixel::rgba(0, 0, 0, 255),
            })
            .unwrap();
        editor
            .execute(Command::SelectRectangle {
                rect: redrob_core::Rect {
                    x: 20,
                    y: 0,
                    width: 20,
                    height: 40,
                },
                mode: redrob_core::SelectionMode::Replace,
            })
            .unwrap();
        editor
            .execute(Command::Fill {
                color: Pixel::rgba(255, 255, 255, 255),
            })
            .unwrap();
        editor.execute(Command::SelectAll).unwrap();
        (editor, layer)
    };
    let (mut e, l) = make();
    e.execute(Command::ApplyFilter {
        filter: redrob_core::Filter::EdgeDetect { amount: 1.0 },
    })
    .unwrap();
    assert!(pixel(&e, l, 20, 20).r > 40, "edge detect lit the boundary");
    assert!(
        pixel(&e, l, 5, 20).r < 40,
        "flat black stayed dark under edge detect"
    );

    let (mut e, l) = make();
    e.execute(Command::ApplyFilter {
        filter: redrob_core::Filter::Laplace,
    })
    .unwrap();
    assert!(pixel(&e, l, 20, 20).r > 40, "laplace lit the boundary");

    let (mut e, l) = make();
    e.execute(Command::ApplyFilter {
        filter: redrob_core::Filter::Emboss {
            angle_degrees: 135.0,
        },
    })
    .unwrap();
    // A flat interior pixel embosses to mid-grey.
    let g = pixel(&e, l, 5, 20);
    assert!(
        (100..=160).contains(&g.r),
        "emboss greyed the flat area to mid-tone"
    );
}

#[test]
fn pixelize_blocks_and_distortions_run() {
    // Pixelize: a sharp 1px checker becomes uniform within each block.
    let mut editor = Editor::new(Document::new(16, 16).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    for y in 0..16u32 {
        for x in 0..16u32 {
            let on = (x + y) % 2 == 0;
            let c = if on {
                Pixel::rgba(255, 255, 255, 255)
            } else {
                Pixel::rgba(0, 0, 0, 255)
            };
            editor
                .execute(Command::BrushStroke {
                    points: vec![BrushPoint::new(x as f32 + 0.5, y as f32 + 0.5, 1.0)],
                    color: c,
                    size: 1.5,
                    opacity: 1.0,
                    settings: BrushSettings {
                        shape: redrob_core::DabShape {
                            pencil: true,
                            ..redrob_core::DabShape::default()
                        },
                        ..BrushSettings::default()
                    },
                    tip: None,
                    pipe: Vec::new(),
                })
                .unwrap();
        }
    }
    editor
        .execute(Command::ApplyFilter {
            filter: redrob_core::Filter::Pixelize { block: 4 },
        })
        .unwrap();
    // Two pixels in the same 4x4 block now share a colour (the checker averaged to mid-grey).
    let a = pixel(&editor, layer, 0, 0);
    let b = pixel(&editor, layer, 1, 0);
    assert_eq!(a.r, b.r, "pixelize made the block uniform");
    assert!(
        (100..=160).contains(&a.r),
        "the checker averaged to mid-grey"
    );

    // Whirl-pinch and lens distortion should run without panicking and keep the canvas populated.
    let mut e2 = Editor::new(Document::new(32, 32).unwrap()).unwrap();
    let l2 = e2.document().active_layer_id();
    e2.execute(Command::Fill {
        color: Pixel::rgba(50, 150, 220, 255),
    })
    .unwrap();
    e2.execute(Command::ApplyFilter {
        filter: redrob_core::Filter::WhirlPinch {
            whirl_degrees: 120.0,
            pinch: 0.3,
        },
    })
    .unwrap();
    e2.execute(Command::ApplyFilter {
        filter: redrob_core::Filter::LensDistortion { main_amount: 40.0 },
    })
    .unwrap();
    assert!(
        pixel(&e2, l2, 16, 16).a > 0,
        "the centre is still painted after the distortions"
    );
}

#[test]
fn noise_filters_are_deterministic_and_jitter() {
    // RGB noise perturbs a flat fill but keeps it in range; the same seed gives the same result.
    let flat = || {
        let mut editor = Editor::new(Document::new(16, 16).unwrap()).unwrap();
        editor
            .execute(Command::Fill {
                color: Pixel::rgba(128, 128, 128, 255),
            })
            .unwrap();
        editor
    };
    let mut a = flat();
    let la = a.document().active_layer_id();
    a.execute(Command::ApplyFilter {
        filter: redrob_core::Filter::RgbNoise {
            amount: 0.3,
            seed: 7,
        },
    })
    .unwrap();
    let mut b = flat();
    let lb = b.document().active_layer_id();
    b.execute(Command::ApplyFilter {
        filter: redrob_core::Filter::RgbNoise {
            amount: 0.3,
            seed: 7,
        },
    })
    .unwrap();
    let pa = pixel(&a, la, 5, 5);
    let pb = pixel(&b, lb, 5, 5);
    assert_eq!(
        (pa.r, pa.g, pa.b),
        (pb.r, pb.g, pb.b),
        "same seed is reproducible"
    );

    // Across the layer at least one pixel moved off the flat 128.
    let moved = (0..16).any(|x| pixel(&a, la, x, 5).r != 128);
    assert!(moved, "rgb noise jittered the fill");

    // Hurl with amount 1.0 replaces (almost) everything with random colour.
    let mut h = flat();
    let lh = h.document().active_layer_id();
    h.execute(Command::ApplyFilter {
        filter: redrob_core::Filter::Hurl {
            amount: 1.0,
            seed: 3,
        },
    })
    .unwrap();
    let changed = (0..16).filter(|&x| pixel(&h, lh, x, 8).r != 128).count();
    assert!(changed >= 8, "hurl randomised most of the row");

    // Spread 0 is a no-op; spread N jitters positions.
    let mut s = flat();
    let ls = s.document().active_layer_id();
    s.execute(Command::ApplyFilter {
        filter: redrob_core::Filter::Spread { amount: 0, seed: 1 },
    })
    .unwrap();
    assert_eq!(pixel(&s, ls, 8, 8).r, 128, "spread 0 leaves the pixel put");
}

#[test]
fn render_filters_checker_gradient_map_and_noise() {
    // Checkerboard: adjacent cells differ.
    let mut e = Editor::new(Document::new(32, 32).unwrap()).unwrap();
    let l = e.document().active_layer_id();
    e.execute(Command::ApplyFilter {
        filter: redrob_core::Filter::Checkerboard {
            size: 8,
            color_a: Pixel::rgba(0, 0, 0, 255),
            color_b: Pixel::rgba(255, 255, 255, 255),
        },
    })
    .unwrap();
    assert_ne!(
        pixel(&e, l, 2, 2).r,
        pixel(&e, l, 10, 2).r,
        "neighbouring checker cells differ"
    );

    // Gradient map: a black→red map turns dark pixels dark-red-ish and white pixels red.
    let mut g = Editor::new(Document::new(8, 8).unwrap()).unwrap();
    let lg = g.document().active_layer_id();
    g.execute(Command::Fill {
        color: Pixel::rgba(255, 255, 255, 255),
    })
    .unwrap();
    g.execute(Command::ApplyFilter {
        filter: redrob_core::Filter::GradientMap {
            low: Pixel::rgba(0, 0, 0, 255),
            high: Pixel::rgba(255, 0, 0, 255),
        },
    })
    .unwrap();
    let p = pixel(&g, lg, 4, 4);
    assert!(
        p.r > 200 && p.g < 40,
        "white mapped to the high colour (red)"
    );

    // Solid noise and cell noise fill opaque and are not uniform.
    let mut n = Editor::new(Document::new(32, 32).unwrap()).unwrap();
    let ln = n.document().active_layer_id();
    n.execute(Command::ApplyFilter {
        filter: redrob_core::Filter::SolidNoise { detail: 4, seed: 2 },
    })
    .unwrap();
    assert_eq!(pixel(&n, ln, 0, 0).a, 255, "solid noise is opaque");
    let varied = (0..32).any(|x| pixel(&n, ln, x, 0).r != pixel(&n, ln, 0, 0).r);
    assert!(varied, "solid noise varies across the row");

    let mut c = Editor::new(Document::new(32, 32).unwrap()).unwrap();
    let lc = c.document().active_layer_id();
    c.execute(Command::ApplyFilter {
        filter: redrob_core::Filter::CellNoise {
            density: 6,
            seed: 2,
        },
    })
    .unwrap();
    assert_eq!(pixel(&c, lc, 0, 0).a, 255, "cell noise is opaque");
}

#[test]
fn colour_filters_behave() {
    let fill = |c: Pixel| {
        let mut e = Editor::new(Document::new(8, 8).unwrap()).unwrap();
        e.execute(Command::Fill { color: c }).unwrap();
        e
    };
    // Saturation 0 => grey (r==g==b).
    let mut s = fill(Pixel::rgba(200, 50, 50, 255));
    let ls = s.document().active_layer_id();
    s.execute(Command::ApplyFilter {
        filter: redrob_core::Filter::Saturation { scale: 0.0 },
    })
    .unwrap();
    let p = pixel(&s, ls, 4, 4);
    assert_eq!(p.r, p.g, "saturation 0 greys the pixel");
    assert_eq!(p.g, p.b, "saturation 0 greys the pixel");

    // Exposure +1 stop roughly doubles (clamped).
    let mut x = fill(Pixel::rgba(60, 60, 60, 255));
    let lx = x.document().active_layer_id();
    x.execute(Command::ApplyFilter {
        filter: redrob_core::Filter::Exposure { stops: 1.0 },
    })
    .unwrap();
    assert!(pixel(&x, lx, 4, 4).r > 100, "exposure brightened");

    // Warm temperature raises red, lowers blue.
    let mut t = fill(Pixel::rgba(120, 120, 120, 255));
    let lt = t.document().active_layer_id();
    t.execute(Command::ApplyFilter {
        filter: redrob_core::Filter::ColorTemperature { amount: 100.0 },
    })
    .unwrap();
    let pt = pixel(&t, lt, 4, 4);
    assert!(
        pt.r > 120 && pt.b < 120,
        "warm temperature pushed red up and blue down"
    );

    // Dither to 2 levels yields only 0 or 255 per channel.
    let mut d = fill(Pixel::rgba(100, 100, 100, 255));
    let ld = d.document().active_layer_id();
    d.execute(Command::ApplyFilter {
        filter: redrob_core::Filter::Dither { levels: 2 },
    })
    .unwrap();
    let pd = pixel(&d, ld, 4, 4);
    assert!(
        pd.r == 0 || pd.r == 255,
        "dither 2 quantised to the extremes"
    );
}

#[test]
fn artistic_filters_run_and_shape_output() {
    // Photocopy yields a greyscale sketch.
    let mut p = Editor::new(Document::new(16, 16).unwrap()).unwrap();
    let lp = p.document().active_layer_id();
    p.execute(Command::Fill {
        color: Pixel::rgba(180, 90, 40, 255),
    })
    .unwrap();
    p.execute(Command::ApplyFilter {
        filter: redrob_core::Filter::Photocopy { amount: 1.0 },
    })
    .unwrap();
    let pc = pixel(&p, lp, 8, 8);
    assert_eq!(pc.r, pc.g, "photocopy is greyscale");
    assert_eq!(pc.g, pc.b, "photocopy is greyscale");

    // Oilify, soft glow, cartoon, apply-canvas and cubism just need to run opaque without panic.
    for f in [
        redrob_core::Filter::Oilify { radius: 3 },
        redrob_core::Filter::Cartoon { amount: 1.5 },
        redrob_core::Filter::SoftGlow {
            radius: 4,
            amount: 0.5,
        },
        redrob_core::Filter::ApplyCanvas { depth: 0.5 },
        redrob_core::Filter::Cubism { tile: 6, seed: 2 },
    ] {
        let mut e = Editor::new(Document::new(24, 24).unwrap()).unwrap();
        let l = e.document().active_layer_id();
        e.execute(Command::Fill {
            color: Pixel::rgba(120, 160, 90, 255),
        })
        .unwrap();
        e.execute(Command::ApplyFilter { filter: f }).unwrap();
        assert!(
            pixel(&e, l, 12, 12).a > 0,
            "the artistic filter kept the pixel opaque"
        );
    }
}

#[test]
fn map_filters_run_opaque() {
    for f in [
        redrob_core::Filter::BumpMap {
            azimuth_degrees: 135.0,
            elevation_degrees: 45.0,
            depth: 4.0,
            map: None,
        },
        redrob_core::Filter::Displace {
            amount: 10.0,
            map: None,
        },
        redrob_core::Filter::FractalTrace {
            depth: 3,
            scale: 1.0,
            map: None,
        },
        redrob_core::Filter::WarpMap {
            amount: 15.0,
            steps: 4,
            map: None,
        },
    ] {
        let mut e = Editor::new(Document::new(24, 24).unwrap()).unwrap();
        let l = e.document().active_layer_id();
        // A gradient fill so the gradient-driven maps have something to follow.
        e.execute(Command::ApplyFilter {
            filter: redrob_core::Filter::GradientMap {
                low: Pixel::rgba(0, 0, 0, 255),
                high: Pixel::rgba(255, 255, 255, 255),
            },
        })
        .unwrap();
        e.execute(Command::Fill {
            color: Pixel::rgba(120, 120, 120, 255),
        })
        .unwrap();
        e.execute(Command::ApplyFilter { filter: f }).unwrap();
        assert!(
            pixel(&e, l, 12, 12).a > 0,
            "the map filter kept the pixel opaque"
        );
    }
}

#[test]
fn krita_filters_palettize_normal_halftone() {
    // Palettize to 2 levels snaps a mid grey to an extreme.
    let mut p = Editor::new(Document::new(8, 8).unwrap()).unwrap();
    let lp = p.document().active_layer_id();
    p.execute(Command::Fill {
        color: Pixel::rgba(100, 100, 100, 255),
    })
    .unwrap();
    p.execute(Command::ApplyFilter {
        filter: redrob_core::Filter::Palettize { levels: 2 },
    })
    .unwrap();
    let pc = pixel(&p, lp, 4, 4);
    assert!(pc.r == 0 || pc.r == 255, "palettize snapped to an extreme");

    // Normal map of a flat area is ~ (128,128,255): flat normal pointing up.
    let mut n = Editor::new(Document::new(8, 8).unwrap()).unwrap();
    let ln = n.document().active_layer_id();
    n.execute(Command::Fill {
        color: Pixel::rgba(120, 120, 120, 255),
    })
    .unwrap();
    n.execute(Command::ApplyFilter {
        filter: redrob_core::Filter::NormalMap { strength: 4.0 },
    })
    .unwrap();
    let nn = pixel(&n, ln, 4, 4);
    assert!(
        (120..=136).contains(&nn.r) && (120..=136).contains(&nn.g) && nn.b > 240,
        "flat area normal points up (128,128,255)"
    );

    // Halftone and phong bump run opaque.
    for f in [
        redrob_core::Filter::Halftone { cell: 4 },
        redrob_core::Filter::PhongBump {
            azimuth_degrees: 135.0,
            elevation_degrees: 45.0,
            depth: 4.0,
            shininess: 16.0,
        },
    ] {
        let mut e = Editor::new(Document::new(16, 16).unwrap()).unwrap();
        let l = e.document().active_layer_id();
        e.execute(Command::Fill {
            color: Pixel::rgba(150, 150, 150, 255),
        })
        .unwrap();
        e.execute(Command::ApplyFilter { filter: f }).unwrap();
        assert!(
            pixel(&e, l, 8, 8).a > 0,
            "the Krita filter kept the pixel opaque"
        );
    }
}

#[test]
fn channel_mixer_swaps_red_and_blue() {
    // A matrix that maps output R from input B and output B from input R swaps the two channels.
    let mut e = Editor::new(Document::new(4, 4).unwrap()).unwrap();
    let l = e.document().active_layer_id();
    e.execute(Command::Fill {
        color: Pixel::rgba(200, 50, 10, 255),
    })
    .unwrap();
    e.execute(Command::ApplyFilter {
        filter: redrob_core::Filter::ChannelMixer {
            matrix: [0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0],
            offset: [0.0, 0.0, 0.0],
            // false is the pre-existing behaviour, so this test measures exactly what it did
            // before the flag existed -- and each row of this swap matrix already sums to 1, so
            // even turning it on would change nothing here.
            preserve_luminosity: false,
        },
    })
    .unwrap();
    let p = pixel(&e, l, 2, 2);
    assert_eq!(p.r, 10, "output red took input blue");
    assert_eq!(p.b, 200, "output blue took input red");
    assert_eq!(p.g, 50, "green unchanged");
}

#[test]
fn undo_redo_depth_tracks_the_stack() {
    let mut e = Editor::new(Document::new(4, 4).unwrap()).unwrap();
    assert_eq!(e.undo_depth(), 0);
    assert_eq!(e.redo_depth(), 0);
    e.execute(Command::Fill {
        color: Pixel::rgba(10, 20, 30, 255),
    })
    .unwrap();
    e.execute(Command::Fill {
        color: Pixel::rgba(40, 50, 60, 255),
    })
    .unwrap();
    assert_eq!(e.undo_depth(), 2, "two fills on the undo stack");
    e.undo().unwrap();
    assert_eq!(e.undo_depth(), 1);
    assert_eq!(e.redo_depth(), 1, "one step available to redo");
    e.redo().unwrap();
    assert_eq!(e.undo_depth(), 2);
    assert_eq!(e.redo_depth(), 0);
}

#[test]
fn op_graph_applies_a_chain_with_amount() {
    use redrob_core::{OpGraph, OpNode};
    // A node's amount blends in LINEAR LIGHT (H.19), not over the display-encoded bytes. 200 inverts to
    // 55; their linear half-mix encodes near 155, where a byte-wise average would give 127. The
    // difference is the point: a byte average makes a half-strength effect look heavier than half.
    let mut e = Editor::new(Document::new(2, 2).unwrap()).unwrap();
    let l = e.document().active_layer_id();
    e.execute(Command::Fill {
        color: Pixel::rgba(200, 200, 200, 255),
    })
    .unwrap();
    let mut half = OpNode::new(redrob_core::Filter::Invert);
    half.amount = 0.5;
    e.execute(Command::ApplyGraph {
        graph: OpGraph { nodes: vec![half] },
    })
    .unwrap();
    let p = pixel(&e, l, 1, 1);
    assert!(
        (148..=162).contains(&p.r),
        "a linear half-mix of 200 and 55 lands near 155, got {}",
        p.r
    );
    assert!(
        p.r > 135,
        "a byte-wise average would have landed near 127: {}",
        p.r
    );

    // A two-op chain: grayscale then full invert runs both in order.
    let mut e2 = Editor::new(Document::new(2, 2).unwrap()).unwrap();
    let l2 = e2.document().active_layer_id();
    e2.execute(Command::Fill {
        color: Pixel::rgba(255, 0, 0, 255),
    })
    .unwrap();
    e2.execute(Command::ApplyGraph {
        graph: OpGraph {
            nodes: vec![
                OpNode::new(redrob_core::Filter::Grayscale),
                OpNode::new(redrob_core::Filter::Invert),
            ],
        },
    })
    .unwrap();
    let q = pixel(&e2, l2, 1, 1);
    // Grayscale of pure red (lum ~54) then invert (~201); channels equal.
    assert_eq!(q.r, q.g, "grayscale made the channels equal");
    assert!(q.r > 150, "then invert lifted the low grey");
}

#[test]
fn color_lab_roundtrip_and_lab_adjust() {
    // sRGB -> Lab -> sRGB is near-identity.
    let (l, a, b) = redrob_core::color::srgb8_to_lab(180, 90, 40);
    let (r, g, bl) = redrob_core::color::lab_to_srgb8(l, a, b);
    assert!((r as i32 - 180).abs() <= 2, "red round-trips through Lab");
    assert!((g as i32 - 90).abs() <= 2, "green round-trips through Lab");
    assert!((bl as i32 - 40).abs() <= 2, "blue round-trips through Lab");

    // LabAdjust chroma 0 greys (channels converge); lightness +20 brightens.
    let mut e = Editor::new(Document::new(2, 2).unwrap()).unwrap();
    let layer = e.document().active_layer_id();
    e.execute(Command::Fill {
        color: Pixel::rgba(180, 90, 40, 255),
    })
    .unwrap();
    e.execute(Command::ApplyFilter {
        filter: redrob_core::Filter::LabAdjust {
            lightness: 0.0,
            chroma: 0.0,
        },
    })
    .unwrap();
    let p = pixel(&e, layer, 1, 1);
    assert!(
        (p.r as i32 - p.g as i32).abs() <= 6 && (p.g as i32 - p.b as i32).abs() <= 6,
        "chroma 0 greys the pixel"
    );
}

#[test]
fn layer_style_drop_shadow_fills_offset_area() {
    use redrob_core::{Bevel, DropShadow, LayerStyle, OuterGlow};
    // Build a small opaque square in the middle; a drop shadow offset down-right should put some
    // alpha below-right of it where it was transparent.
    let w = 32u32;
    let h = 32u32;
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    for y in 8..16u32 {
        for x in 8..16u32 {
            let o = ((y * w + x) * 4) as usize;
            pixels[o..o + 4].copy_from_slice(&[200, 60, 60, 255]);
        }
    }
    let style = LayerStyle {
        drop_shadow: Some(DropShadow {
            color: Pixel::rgba(0, 0, 0, 255),
            offset_x: 6,
            offset_y: 6,
            blur: 2,
            opacity: 0.8,
        }),
        outer_glow: None,
        bevel: None,
    };
    redrob_core::layer_style::apply_layer_style(&mut pixels, w, h, &style).unwrap();
    // A pixel down-right of the square (was transparent) now has shadow alpha.
    let o = ((20 * w + 20) * 4) as usize;
    assert!(pixels[o + 3] > 0, "the drop shadow filled the offset area");
    // The original square is still opaque.
    let s = ((12 * w + 12) * 4) as usize;
    assert_eq!(pixels[s + 3], 255, "the layer still sits on top");

    // Smoke: glow + bevel together run without panic.
    let style2 = LayerStyle {
        drop_shadow: None,
        outer_glow: Some(OuterGlow {
            color: Pixel::rgba(0, 255, 0, 255),
            blur: 4,
            opacity: 0.7,
        }),
        bevel: Some(Bevel {
            azimuth_degrees: 135.0,
            depth: 6.0,
            blur: 2,
        }),
    };
    let mut p2 = pixels.clone();
    redrob_core::layer_style::apply_layer_style(&mut p2, w, h, &style2).unwrap();
}

/// H.1: `BrushSettings` owns a list (`dynamics`), so it is `Clone` and NOT `Copy`. The paint path reads
/// it by reference, so one settings value drives two strokes without the caller copying configuration.
/// A guard rather than a behaviour test: if someone puts `Copy` back, the `Vec` makes the derive fail,
/// and if someone takes settings by value again, this call site stops compiling at the second use.
#[test]
fn brush_settings_are_cloneable_and_reusable_across_strokes() {
    use redrob_core::{SizeDynamic, SizeSensor};

    let mut editor = Editor::new(Document::new(16, 16).unwrap()).unwrap();
    let settings = BrushSettings {
        dynamics: vec![
            SizeDynamic {
                sensor: SizeSensor::Pressure,
                amount: 0.5,
            },
            SizeDynamic {
                sensor: SizeSensor::Speed,
                amount: -0.25,
            },
        ],
        ..BrushSettings::default()
    };
    // Same settings, two strokes: the first takes a clone, the second takes the original.
    let stroke = |x: f32, settings: BrushSettings| Command::BrushStroke {
        points: vec![BrushPoint::new(x, 4.0, 1.0), BrushPoint::new(x, 12.0, 0.4)],
        color: Pixel::rgba(200, 40, 40, 255),
        size: 5.0,
        opacity: 1.0,
        settings,
        tip: None,
        pipe: Vec::new(),
    };
    editor.execute(stroke(4.0, settings.clone())).unwrap();
    editor.execute(stroke(10.0, settings)).unwrap();
    assert_eq!(editor.undo_depth(), 2);
}

/// H.2: the editor's onion-skin render is the shell's entry point, so it must agree with the plain
/// render when no neighbours are asked for. With `before = after = 0` there is no ghost to composite,
/// so the result is the current frame exactly -- which is what makes the ghosted case attributable to
/// the neighbours rather than to the onion path itself.
#[test]
fn onion_skin_without_neighbours_matches_the_plain_render() {
    let mut editor = Editor::new(Document::new(8, 6).unwrap()).unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(20, 90, 200, 255),
        })
        .unwrap();
    let plain = editor.render_snapshot().unwrap();
    let onion = editor
        .render_onion_skin_snapshot(
            0,
            0,
            Pixel::rgba(255, 80, 80, 255),
            Pixel::rgba(80, 255, 120, 255),
            0.4,
        )
        .unwrap();
    assert_eq!(onion.width(), plain.width());
    assert_eq!(onion.height(), plain.height());
    assert_eq!(onion.pixels(), plain.pixels());
}

/// H.18: a map filter reads its height field from ANOTHER layer when one is named. This is the whole
/// point of the feature — a bump map is a separate grey image, and shading a picture by its own
/// brightness lights its content rather than its surface.
#[test]
fn a_map_filter_reads_its_height_field_from_the_named_layer() {
    use redrob_core::{DocumentImportBuilder, ImportNode, RasterCel};

    // Two layers over a 4x1 canvas: a FLAT target, and a map with a hard step in the middle.
    let flat = vec![128u8; 4 * 4];
    let mut stepped = Vec::new();
    for x in 0..4 {
        let value = if x < 2 { 0 } else { 255 };
        stepped.extend_from_slice(&[value, value, value, 255]);
    }
    let mut builder = DocumentImportBuilder::new(4, 1).unwrap();
    builder
        .push_node(ImportNode::raster(
            "target",
            vec![RasterCel::new(FrameId::DEFAULT, flat.clone())],
        ))
        .unwrap();
    builder
        .push_node(ImportNode::raster(
            "heights",
            vec![RasterCel::new(FrameId::DEFAULT, stepped)],
        ))
        .unwrap();
    let document = builder.build().unwrap();
    let map_id = document.nodes()[1].id();
    let target_id = document.nodes()[0].id();

    // Self-map first: a flat layer has no gradient, so bump mapping it cannot change anything except
    // uniformly. This is the control that makes the next assertion mean something.
    let mut editor = Editor::new(document.clone()).unwrap();
    editor
        .execute(Command::SetActiveLayer { id: target_id })
        .unwrap();
    editor
        .execute(Command::ApplyFilter {
            filter: redrob_core::Filter::BumpMap {
                azimuth_degrees: 135.0,
                elevation_degrees: 45.0,
                depth: 4.0,
                map: None,
            },
        })
        .unwrap();
    let self_mapped = editor.document().layers()[0].pixels().to_vec();
    assert!(
        self_mapped
            .chunks_exact(4)
            .map(|p| p[0])
            .collect::<std::collections::HashSet<_>>()
            .len()
            == 1,
        "a flat self-map must shade uniformly: {self_mapped:?}"
    );

    // Now with the stepped layer as the map: the step is an edge, so the pixels beside it must shade
    // differently from the ones away from it.
    let mut editor = Editor::new(document).unwrap();
    editor
        .execute(Command::SetActiveLayer { id: target_id })
        .unwrap();
    editor
        .execute(Command::ApplyFilter {
            filter: redrob_core::Filter::BumpMap {
                azimuth_degrees: 135.0,
                elevation_degrees: 45.0,
                depth: 4.0,
                map: Some(map_id),
            },
        })
        .unwrap();
    let mapped = editor.document().layers()[0].pixels().to_vec();
    let values: Vec<u8> = mapped.chunks_exact(4).map(|pixel| pixel[0]).collect();
    assert!(
        values
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            > 1,
        "the map's edge must produce varying shade: {values:?}"
    );
    // And the target layer's own flatness is irrelevant: the variation came from the map alone.
    assert_ne!(values[0], values[1], "{values:?}");
}

/// A named layer that does not exist is an ERROR, not a silent fall back to the self-map: the command
/// asked for a specific map, and quietly shading by the layer's own brightness would look like the
/// filter working badly rather than like a missing layer.
#[test]
fn a_map_filter_refuses_a_map_layer_that_does_not_exist() {
    let mut editor = Editor::new(Document::new(2, 2).unwrap()).unwrap();
    let missing = redrob_core::NodeId::new();
    let error = editor
        .execute(Command::ApplyFilter {
            filter: redrob_core::Filter::Displace {
                amount: 5.0,
                map: Some(missing),
            },
        })
        .unwrap_err();
    assert!(matches!(error, CoreError::LayerNotFound(_)), "{error:?}");
}

/// J.1b. A precision-native filter has ONE implementation that is correct at every sample width,
/// and the first thing that must be true of it is that it did not change 8-bit output.
///
/// This test is the reason to stage the migration at all. If the native path disagreed with the
/// byte arm it replaced, the right move would be to roll it back rather than port more filters onto
/// it — so the 8-bit answer is pinned first, then the same answer is required at 16-bit and float.
#[test]
fn invert_is_identical_at_every_precision() {
    use redrob_core::precision::Precision;

    // Values chosen so a wrong complement is visible and asymmetric: 1 inverts to 254, so a
    // formula that mixed up 255 and 256, or that inverted alpha, lands somewhere else.
    let fill = Pixel::rgba(1, 128, 200, 255);
    let expected = Pixel::rgba(254, 127, 55, 255);

    let mut answers = Vec::new();
    for precision in [Precision::U8, Precision::U16, Precision::F32] {
        let mut editor = Editor::new(Document::new(2, 2).unwrap()).unwrap();
        let layer = editor.document().active_layer_id();
        editor.execute(Command::Fill { color: fill }).unwrap();
        editor
            .execute(Command::SetDocumentPrecision { precision })
            .unwrap();
        assert_eq!(editor.document().precision(), precision);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Invert,
            })
            .unwrap();
        // Read back at 8-bit so the three runs are comparable at all.
        editor
            .execute(Command::SetDocumentPrecision {
                precision: Precision::U8,
            })
            .unwrap();
        answers.push((precision, pixel(&editor, layer, 0, 0)));
    }

    for (precision, got) in &answers {
        assert_eq!(
            *got, expected,
            "invert at {precision:?} must give the same colour as at 8-bit"
        );
    }
}

/// A filter still written against 8-bit bytes is REFUSED on a deeper document, by name.
///
/// The alternative — narrow to 8-bit, run the old arm, widen back — would make every filter appear
/// to work at 16-bit while discarding the depth on each application, with nothing reported. A user
/// chose 16-bit to avoid precisely that, so the error is the feature.
#[test]
fn a_filter_not_yet_ported_is_refused_by_name_rather_than_narrowing_the_document() {
    use redrob_core::precision::Precision;

    let mut editor = Editor::new(Document::new(2, 2).unwrap()).unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(10, 20, 30, 255),
        })
        .unwrap();
    editor
        .execute(Command::SetDocumentPrecision {
            precision: Precision::U16,
        })
        .unwrap();

    let error = editor
        .execute(Command::ApplyFilter {
            filter: Filter::Grayscale,
        })
        .expect_err("a filter that is not precision-native must refuse a 16-bit document");
    match error {
        CoreError::FilterPrecisionUnsupported(name) => assert_eq!(
            name, "grayscale",
            "the message must name the filter that objected, not say 'a filter'"
        ),
        other => panic!("expected a precision refusal, got {other:?}"),
    }

    // Refused means refused: the document is still 16-bit and still holds its pixels.
    assert_eq!(editor.document().precision(), Precision::U16);
    assert!(
        editor.can_undo(),
        "the fill and the precision change are still the only history; the refusal added none"
    );
}

/// Every filter on the precision-native list has an arm in the native implementation.
///
/// The list and the implementation are separate pieces of code, so adding a filter to the list and
/// forgetting the arm is a live possibility — and it would fail at RUN time, on a user's document,
/// as a refusal for a filter the list says is supported. The list is read from the crate rather
/// than copied here, because a copy keeps passing after the real one changes.
#[test]
fn native_filters_all_have_an_implementation() {
    use redrob_core::precision::Precision;

    let native = redrob_core::precision_native_filter_tags();
    assert!(
        !native.is_empty(),
        "the native list is empty; J.1b landed nothing"
    );
    for tag in native {
        let filter: Filter = serde_json::from_value(serde_json::json!({ "kind": tag }))
            .unwrap_or_else(|error| panic!("'{tag}' is not a filter wire tag: {error}"));
        let mut editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
        editor
            .execute(Command::SetDocumentPrecision {
                precision: Precision::F32,
            })
            .unwrap();
        editor
            .execute(Command::ApplyFilter { filter })
            .unwrap_or_else(|error| {
                panic!("'{tag}' is listed as precision-native but failed: {error:?}")
            });
    }
}

/// Every filter resolves to its OWN name, not the generic fallback.
///
/// This exists because the name lookup reads the enum's serde tag by key, and the key was wrong at
/// first — which returns "filter" for every variant. That reads as a cosmetic message problem and
/// is actually the entire lookup being dead, so it needs a test that fails on the generic word
/// rather than on one example. It also catches a `FILTER_NAMES` entry that a rename left behind.
#[test]
fn every_filter_variant_resolves_to_its_own_name() {
    use redrob_core::precision::Precision;

    // One filter per wire tag, built from the tag itself so this cannot drift from the enum: a
    // variant added without a name-table entry refuses with the generic word and fails here.
    for tag in redrob_core::filter_wire_tags() {
        // A deeper document makes the refusal path the one that reports the name.
        let mut editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
        editor
            .execute(Command::SetDocumentPrecision {
                precision: Precision::U16,
            })
            .unwrap();
        let Ok(filter) = serde_json::from_value::<Filter>(serde_json::json!({ "kind": tag }))
        else {
            // Filters that carry required parameters cannot be built from a bare tag; their names
            // are covered by the table assertion below.
            continue;
        };
        if filter.is_precision_native_for_test() {
            continue;
        }
        let error = editor
            .execute(Command::ApplyFilter { filter })
            .expect_err("a non-native filter must refuse a 16-bit document");
        match error {
            CoreError::FilterPrecisionUnsupported(name) => assert_eq!(
                name, *tag,
                "filter '{tag}' reported the name '{name}'; the generic word means the tag lookup \
                 is reading the wrong key and every filter's name is wrong"
            ),
            other => panic!("expected a precision refusal for '{tag}', got {other:?}"),
        }
    }

    // And the table covers the enum: one entry per variant, no leftovers.
    let tags: Vec<&str> = redrob_core::filter_wire_tags().to_vec();
    let mut sorted = tags.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        tags.len(),
        "the filter name table has a duplicate entry"
    );
}

/// J.1d. Compositing happens at the document's precision, so a STACK of blends does not accumulate
/// the rounding error of one 8-bit step per layer.
///
/// The acceptance originally written for this item -- "a gradient shows measurably fewer banding
/// steps at 16-bit" -- cannot be measured on the display surface, and that was a mistake in the
/// plan rather than in the code: the display path is 8-bit by definition, so a single layer has 256
/// steps at any document precision. The win is in ACCUMULATION. Each 8-bit blend rounds to the
/// nearest of 255 levels; stack eight of them and the error compounds into a value visibly away
/// from the exact answer, while the same stack at 16-bit stays on it.
#[test]
fn a_stack_of_blends_keeps_its_accuracy_at_sixteen_bits() {
    use redrob_core::precision::Precision;

    // Eight layers of 50%-opacity white over black. The exact result approaches 1 - 1/2^8 of full
    // white; what matters is that the deep composite lands closer to it than the byte one.
    fn stack(precision: Precision) -> u8 {
        let mut editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
        editor
            .execute(Command::Fill {
                color: Pixel::rgba(0, 0, 0, 255),
            })
            .unwrap();
        editor
            .execute(Command::SetDocumentPrecision { precision })
            .unwrap();
        for index in 0..8 {
            let id = LayerId::new();
            editor
                .execute(Command::AddLayer {
                    id,
                    name: format!("over {index}"),
                    index: index + 1,
                })
                .unwrap();
            editor
                .execute(Command::Fill {
                    color: Pixel::rgba(255, 255, 255, 255),
                })
                .unwrap();
            editor
                .execute(Command::SetLayerOpacity { id, opacity: 0.5 })
                .unwrap();
        }
        let snapshot = editor.try_render_snapshot().unwrap();
        assert_eq!(
            snapshot.precision(),
            precision,
            "the projection is kept at the document's precision"
        );
        // Read through the display boundary, which is 8-bit at every document precision.
        snapshot.rgba8()[0]
    }

    let eight = stack(Precision::U8);
    let sixteen = stack(Precision::U16);
    // 1 - 2^-8 of 255 is 254.004..., so 254 is the exact answer rounded to a byte.
    assert_eq!(
        sixteen, 254,
        "a 16-bit composite of eight 50% layers should land on the exact value"
    );
    // MEASURED: the byte path lands on 255 and the deep path on 254. One step, and it is the step
    // between "the stack is still not quite white" and "the stack is white".
    assert_eq!(eight, 255, "the 8-bit stack rounds its way to full white");
    assert!(
        eight != sixteen,
        "if the byte path gave the same answer, this test could not tell the two apart and would \
         prove nothing"
    );
    assert!(
        (i16::from(eight) - 254).abs() > (i16::from(sixteen) - 254).abs(),
        "the 8-bit stack must be FURTHER from the exact value: got 8-bit {eight}, 16-bit {sixteen}"
    );
}

/// J.2a. A channel is a named coverage mask the document keeps, and it is VISIBLE as an overlay —
/// which is what makes its visibility and opacity mean anything.
///
/// Three things here would be silently wrong and are each pinned:
///
/// 1. `show_masked` decides which side the overlay paints. The same channel with the flag flipped
///    is the negative of itself on screen, so an implementation that picked the other convention
///    looks correct until someone compares it with a stored selection they recognise.
/// 2. The display colour's ALPHA participates in the overlay strength alongside the channel's own
///    opacity. Reading one and ignoring the other gives a control that appears dead.
/// 3. The overlay is drawn over the WHOLE layer stack. Compositing it among the layers would let a
///    layer above hide the marking the user turned on in order to see it.
#[test]
fn a_visible_channel_tints_the_canvas_on_the_side_show_masked_selects() {
    let mut editor = Editor::new(Document::new(2, 1).unwrap()).unwrap();
    // White image, so a red overlay is unmistakable in the red and blue channels.
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(255, 255, 255, 255),
        })
        .unwrap();
    // Select the left pixel only, then store it as a channel.
    editor
        .execute(Command::SelectRectangle {
            rect: Rect::new(0, 0, 1, 1),
            mode: SelectionMode::Replace,
        })
        .unwrap();
    let channel = redrob_core::ChannelId::new_v4();
    editor
        .execute(Command::AddChannel {
            id: channel,
            name: "Stored".into(),
            from_selection: true,
        })
        .unwrap();
    assert_eq!(editor.document().channels().len(), 1);
    assert_eq!(editor.document().channels()[0].name(), "Stored");

    let rendered = |editor: &Editor| editor.try_render_snapshot().unwrap().rgba8().to_vec();

    // Default is show_masked: the overlay marks what is HELD BACK, so the selected left pixel stays
    // white and the unselected right pixel is tinted.
    let masked = rendered(&editor);
    assert_eq!(
        &masked[0..3],
        &[255, 255, 255],
        "the selected side is clear"
    );
    assert!(
        masked[4] > masked[6],
        "the masked side is tinted red, got {:?}",
        &masked[4..8]
    );

    // Flip the side: now the selected pixel is the tinted one. This is the assertion that would
    // pass for an implementation using either convention if it only checked "something is tinted".
    editor
        .execute(Command::SetChannelShowMasked {
            id: channel,
            show_masked: false,
        })
        .unwrap();
    let selected = rendered(&editor);
    assert!(
        selected[0] > selected[2],
        "the selected side is now the tinted one, got {:?}",
        &selected[0..4]
    );
    assert_eq!(
        &selected[4..7],
        &[255, 255, 255],
        "and the masked side is clear"
    );

    // Hiding the channel removes the overlay entirely.
    editor
        .execute(Command::SetChannelVisible {
            id: channel,
            visible: false,
        })
        .unwrap();
    let hidden = rendered(&editor);
    assert_eq!(
        &hidden[0..8],
        &[255, 255, 255, 255, 255, 255, 255, 255],
        "a hidden channel draws nothing"
    );
}

/// The channel's opacity AND its colour's alpha both scale the overlay.
///
/// Two separate controls that multiply. An implementation reading one and ignoring the other gives
/// a slider that appears dead, which is the kind of defect that survives a demo.
#[test]
fn channel_opacity_and_colour_alpha_both_scale_the_overlay() {
    let strength = |opacity: f32, alpha: u8| {
        let mut editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
        editor
            .execute(Command::Fill {
                color: Pixel::rgba(255, 255, 255, 255),
            })
            .unwrap();
        let channel = redrob_core::ChannelId::new_v4();
        editor
            .execute(Command::AddChannel {
                id: channel,
                name: "Mask".into(),
                // No selection, so coverage is 0 everywhere and `show_masked` makes the whole
                // canvas the masked side -- a full-strength overlay to measure against.
                from_selection: false,
            })
            .unwrap();
        editor
            .execute(Command::SetChannelOpacity {
                id: channel,
                opacity,
            })
            .unwrap();
        editor
            .execute(Command::SetChannelColor {
                id: channel,
                color: Pixel::rgba(255, 0, 0, alpha),
            })
            .unwrap();
        // How far the blue channel was pulled down from white measures the overlay's strength.
        255 - editor.try_render_snapshot().unwrap().rgba8()[2]
    };

    let full = strength(1.0, 255);
    assert!(
        full > 200,
        "a full-strength overlay should be strong, got {full}"
    );
    let half_opacity = strength(0.5, 255);
    let half_alpha = strength(1.0, 128);
    assert!(
        half_opacity < full,
        "the channel's opacity must scale the overlay: {half_opacity} vs {full}"
    );
    assert!(
        half_alpha < full,
        "the colour's alpha must scale the overlay too: {half_alpha} vs {full}"
    );
    // Both at half must be weaker than either alone, which is what "they multiply" means.
    let both = strength(0.5, 128);
    assert!(
        both < half_opacity && both < half_alpha,
        "the two controls multiply: both={both}, opacity-only={half_opacity}, alpha-only={half_alpha}"
    );
}

/// A channel is one byte per pixel at every document precision, and the validator measures it that
/// way.
///
/// Coverage is not colour: sixteen bits of "how selected is this pixel" buys nothing a user can see.
/// Measuring a channel against the document's RGBA stride instead would reject every channel in a
/// deep document.
#[test]
fn channels_stay_one_byte_per_pixel_in_a_deep_document() {
    use redrob_core::precision::Precision;

    let mut editor = Editor::new(Document::new(2, 2).unwrap()).unwrap();
    editor
        .execute(Command::SetDocumentPrecision {
            precision: Precision::U16,
        })
        .unwrap();
    let channel = redrob_core::ChannelId::new_v4();
    editor
        .execute(Command::AddChannel {
            id: channel,
            name: "Mask".into(),
            from_selection: false,
        })
        .unwrap();
    assert_eq!(
        editor.document().channels()[0].pixels().len(),
        4,
        "four pixels, one coverage byte each, regardless of the document's sample width"
    );
    // And the document still validates, which is what a render depends on.
    editor.try_render_snapshot().unwrap();
}

/// J.2b. Quick mask turns the selection into a paintable channel and back.
///
/// The round trip is the feature, and each half has one detail that would be silently wrong:
///
/// - Entering CLEARS the selection. While the mode is on the user paints the mask, and a live
///   selection would confine those strokes to the very region they are meant to redraw — so a
///   stroke outside the original selection would do nothing, which looks like a broken brush.
/// - Leaving REPLACES the selection with the mask. Combining instead (add, intersect) would make
///   the edited mask a modifier to the selection it came from rather than the answer.
#[test]
fn quick_mask_round_trips_the_selection_through_a_paintable_channel() {
    let mut editor = Editor::new(Document::new(4, 1).unwrap()).unwrap();
    // Select the leftmost pixel.
    editor
        .execute(Command::SelectRectangle {
            rect: Rect::new(0, 0, 1, 1),
            mode: SelectionMode::Replace,
        })
        .unwrap();
    assert_eq!(editor.document().selection().coverage(0, 0), 255);
    assert_eq!(editor.document().selection().coverage(3, 0), 0);

    // Enter: a channel appears carrying that coverage, and the selection is emptied.
    editor
        .execute(Command::SetQuickMask { active: true })
        .unwrap();
    let channel = editor
        .document()
        .quick_mask()
        .expect("the mode records which channel it is editing");
    assert_eq!(editor.document().channels().len(), 1);
    assert_eq!(
        editor.document().channels()[0].name(),
        redrob_core::QUICK_MASK_NAME
    );
    assert_eq!(editor.document().channels()[0].pixels()[0], 255);
    assert_eq!(editor.document().channels()[0].pixels()[3], 0);
    // Asserted as INACTIVE rather than as zero coverage: an inactive selection reports 255 from
    // `coverage` on purpose (no selection means every pixel is available), so reading coverage here
    // cannot tell "cleared" from "everything selected".
    assert!(
        !editor.document().selection().is_active(),
        "the selection is cleared on entering, or strokes would be confined to it"
    );

    // Paint white at the far right: that is OUTSIDE the original selection, which is exactly the
    // case a surviving selection would have blocked.
    editor
        .execute(Command::BrushStroke {
            points: vec![BrushPoint::new(3.0, 0.0, 1.0)],
            color: Pixel::rgba(255, 255, 255, 255),
            // Wide enough that the painted pixel is near the dab centre; at size 2 a soft dab only
            // reaches about 0.56 there, and a threshold tuned to that would test the dab shape.
            size: 4.0,
            opacity: 1.0,
            settings: BrushSettings::default(),
            tip: None,
            pipe: Vec::new(),
        })
        .unwrap();
    let painted = editor.document().channels()[0].pixels().to_vec();
    assert!(
        painted[3] > 200,
        "a white stroke adds coverage at the far right, got {painted:?}"
    );
    // And the stroke went into the MASK, not the image: the layer is still empty.
    assert_eq!(
        pixel(&editor, editor.document().active_layer_id(), 3, 0),
        Pixel::TRANSPARENT,
        "while quick mask is on, a stroke must not reach the layer"
    );

    // Leave: the mask replaces the selection, and the channel is gone.
    editor
        .execute(Command::SetQuickMask { active: false })
        .unwrap();
    assert_eq!(editor.document().quick_mask(), None);
    assert!(editor.document().channels().is_empty());
    assert!(
        editor.document().selection().coverage(3, 0) > 200,
        "the painted area is now selected"
    );
    assert_eq!(
        editor.document().selection().coverage(0, 0),
        255,
        "and the original selection survived the round trip"
    );
    let _ = channel;
}

/// A black stroke in quick mask SUBTRACTS coverage.
///
/// The brush colour is read for its brightness because a channel has nowhere to put a hue. If the
/// stroke were applied as "paint coverage wherever the brush lands", black and white would both add
/// and the mode would be unable to erase — which is half of what it is for.
#[test]
fn a_black_stroke_in_quick_mask_removes_coverage() {
    let mut editor = Editor::new(Document::new(4, 1).unwrap()).unwrap();
    editor.execute(Command::SelectAll).unwrap();
    editor
        .execute(Command::SetQuickMask { active: true })
        .unwrap();
    assert_eq!(
        editor.document().channels()[0].pixels(),
        [255, 255, 255, 255]
    );

    editor
        .execute(Command::BrushStroke {
            points: vec![BrushPoint::new(0.0, 0.0, 1.0)],
            color: Pixel::rgba(0, 0, 0, 255),
            // Wide enough that the leftmost pixel is near the dab centre. At size 2 the centre
            // coverage of a soft dab is about 0.56, so the stroke lands at 112 -- correct for that
            // brush, and a threshold tuned to it would be testing the dab shape, not the mode.
            size: 4.0,
            opacity: 1.0,
            settings: BrushSettings::default(),
            tip: None,
            pipe: Vec::new(),
        })
        .unwrap();
    let painted = editor.document().channels()[0].pixels().to_vec();
    assert!(
        painted[0] < 60,
        "a black stroke takes coverage away, got {painted:?}"
    );
    assert_eq!(painted[3], 255, "and leaves the rest alone");
}

/// Toggling to the state it is already in does nothing.
///
/// A toggle bound to a keyboard shortcut gets pressed twice; stacking a second mask channel, or
/// converting a selection that is already empty, is the shape of bug that finds fast.
#[test]
fn asking_for_the_quick_mask_state_it_is_already_in_is_a_no_op() {
    let mut editor = Editor::new(Document::new(2, 1).unwrap()).unwrap();
    editor
        .execute(Command::SetQuickMask { active: false })
        .unwrap();
    assert!(editor.document().channels().is_empty());

    editor
        .execute(Command::SetQuickMask { active: true })
        .unwrap();
    let id = editor.document().quick_mask().unwrap();
    editor
        .execute(Command::SetQuickMask { active: true })
        .unwrap();
    assert_eq!(editor.document().channels().len(), 1, "no second mask");
    assert_eq!(
        editor.document().quick_mask(),
        Some(id),
        "and still the same one"
    );
}
