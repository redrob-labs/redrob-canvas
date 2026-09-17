// SPDX-License-Identifier: GPL-3.0-or-later

use redrob_core::{
    Affine2D, BlendMode, BrushPoint, BrushSettings, BrushSmoothing, Command, CoreError, Document,
    DocumentMetadata, EMBEDDED_FONT_ID, Editor, Filter, FrameId, GradientKind, GradientStop,
    HistoryConfig, LayerId, MAX_BRUSH_PIXEL_VISITS, MAX_BRUSH_POINTS, MAX_BRUSH_SIZE, MAX_FRAMES,
    MAX_HIERARCHY_DEPTH, MAX_NODES, MAX_PATH_COMMANDS, MAX_PATH_COMMANDS_PER_PATH,
    MAX_RENDER_PIXEL_VISITS, MAX_SEMANTIC_MEMORY_BYTES, MAX_TEXT_BYTES, MAX_TEXT_CONTENT_BYTES,
    MAX_VECTOR_PATHS, NodeKind, PathCommand, Pixel, Rect, SamplingMode, SelectionMode,
    SemanticUsage, TextContent, VectorContent, VectorPath, admit_semantic_replacement, export_png,
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
        (BlendMode::Multiply, [78, 47, 39, 255]),
        (BlendMode::Screen, [222, 173, 211, 255]),
        (BlendMode::Overlay, [157, 94, 167, 255]),
        (BlendMode::Add, [255, 220, 250, 255]),
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

    let error = editor
        .execute(Command::BrushStroke {
            points: vec![BrushPoint::new(center, center, 1.0); MAX_BRUSH_POINTS],
            color: Pixel::rgba(255, 0, 0, 255),
            size: MAX_BRUSH_SIZE,
            opacity: 1.0,
            settings: BrushSettings::default(),
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
            filter: Filter::Threshold { threshold: 128 },
        })
        .unwrap();
    assert_eq!(pixel(&editor, layer, 0, 0), Pixel::rgba(200, 100, 50, 255));
    assert_eq!(pixel(&editor, layer, 1, 0), Pixel::rgba(0, 0, 0, 255));

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
        },
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
        },
    );
    assert_eq!(shifted, Pixel::rgba(0, 255, 0, 255));

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
        Filter::Threshold { threshold: 100 },
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
            },
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
