// SPDX-License-Identifier: GPL-3.0-or-later

use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use redrob_agent::{Error, FunctionTool, Result, ToolCall, ToolExecutor, ToolOutput};
use redrob_core::{
    Affine2D, BrushPoint, BrushSettings, BrushSmoothing, Command, Document, EMBEDDED_FONT_ID,
    Editor, FillRule, Filter, FrameId, GradientKind, GradientStop, LayerId, MAX_BRUSH_POINTS,
    MAX_BRUSH_SIZE, MAX_FRAMES, MAX_HIERARCHY_DEPTH, MAX_MASK_COMMAND_PIXELS, MAX_PATH_COMMANDS,
    MAX_PATH_COMMANDS_PER_PATH, MAX_SEMANTIC_COORDINATE, MAX_TIMELINE_FPS, MAX_VECTOR_PATHS,
    NodeContent, NodeKind, PathCommand, Pixel, Rect, SamplingMode, SelectionMode, SemanticUsage,
    StrokeStyle, TextContent, VectorContent, VectorPath, admit_semantic_replacement,
    semantic_usage, validate_semantic_content,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Thread-safe editor ownership shared by native and agent integration layers.
pub type SharedEditor = Arc<Mutex<Editor>>;

/// Maps a fixed set of Redrob graphics tools onto the core command bus.
///
/// There is intentionally no generic command, shell, or filesystem tool.
#[derive(Clone)]
pub struct GraphicsToolExecutor {
    editor: SharedEditor,
}

impl GraphicsToolExecutor {
    pub fn new(editor: Editor) -> Self {
        Self::from_shared(Arc::new(Mutex::new(editor)))
    }

    pub fn from_shared(editor: SharedEditor) -> Self {
        Self { editor }
    }

    pub fn shared_editor(&self) -> SharedEditor {
        Arc::clone(&self.editor)
    }

    fn lock(&self) -> MutexGuard<'_, Editor> {
        self.editor
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

fn layer_id_property() -> Value {
    json!({ "type": "string", "format": "uuid", "description": "Stable node identifier" })
}

fn optional_parent_property() -> Value {
    json!({
        "oneOf": [
            { "type": "string", "format": "uuid" },
            { "type": "null" }
        ],
        "description": "Destination group identifier, or null for the document root"
    })
}

fn color_property() -> Value {
    object(
        json!({
            "r": { "type": "integer", "minimum": 0, "maximum": 255 },
            "g": { "type": "integer", "minimum": 0, "maximum": 255 },
            "b": { "type": "integer", "minimum": 0, "maximum": 255 },
            "a": { "type": "integer", "minimum": 0, "maximum": 255 }
        }),
        &["r", "g", "b", "a"],
    )
}

fn semantic_coordinate_property() -> Value {
    json!({
        "type": "number",
        "minimum": -MAX_SEMANTIC_COORDINATE,
        "maximum": MAX_SEMANTIC_COORDINATE
    })
}

fn text_content_property() -> Value {
    object(
        json!({
            "text": { "type": "string", "maxLength": 262144 },
            "font_family": { "type": "string", "minLength": 1, "maxLength": 256 },
            "font_size": { "type": "number", "exclusiveMinimum": 0.0, "maximum": 4096.0 },
            "color": color_property(),
            "origin_x": semantic_coordinate_property(),
            "origin_y": semantic_coordinate_property()
        }),
        &["text", "font_size", "color", "origin_x", "origin_y"],
    )
}

fn vector_content_property() -> Value {
    let coordinate = semantic_coordinate_property;
    let command = json!({
        "oneOf": [
            object(json!({ "type": { "const": "move_to" }, "x": coordinate(), "y": coordinate() }), &["type", "x", "y"]),
            object(json!({ "type": { "const": "line_to" }, "x": coordinate(), "y": coordinate() }), &["type", "x", "y"]),
            object(json!({
                "type": { "const": "cubic_to" },
                "control1_x": coordinate(), "control1_y": coordinate(),
                "control2_x": coordinate(), "control2_y": coordinate(),
                "x": coordinate(), "y": coordinate()
            }), &["type", "control1_x", "control1_y", "control2_x", "control2_y", "x", "y"]),
            object(json!({ "type": { "const": "close" } }), &["type"])
        ]
    });
    let stroke = object(
        json!({
            "color": color_property(),
            "width": { "type": "number", "exclusiveMinimum": 0.0, "maximum": 4096.0 }
        }),
        &["color", "width"],
    );
    object(
        json!({
            "paths": {
                "type": "array",
                "maxItems": MAX_VECTOR_PATHS,
                "items": object(
                    json!({
                        "commands": { "type": "array", "minItems": 2, "maxItems": MAX_PATH_COMMANDS_PER_PATH, "items": command },
                        "fill": { "oneOf": [color_property(), { "type": "null" }] },
                        "stroke": { "oneOf": [stroke, { "type": "null" }] },
                        "fill_rule": { "type": "string", "enum": ["non_zero", "even_odd"] }
                    }),
                    &["commands"],
                )
            }
        }),
        &["paths"],
    )
}

fn rect_property() -> Value {
    object(
        json!({
            "x": { "type": "integer", "minimum": i32::MIN, "maximum": i32::MAX },
            "y": { "type": "integer", "minimum": i32::MIN, "maximum": i32::MAX },
            "width": { "type": "integer", "minimum": 1, "maximum": 32768 },
            "height": { "type": "integer", "minimum": 1, "maximum": 32768 }
        }),
        &["x", "y", "width", "height"],
    )
}

fn selection_mode_property() -> Value {
    json!({
        "type": "string",
        "enum": ["replace", "add", "subtract", "intersect"]
    })
}

fn sampling_property() -> Value {
    json!({ "type": "string", "enum": ["nearest", "bilinear"] })
}

fn declaration(name: &str, description: &str, parameters: Value, mutating: bool) -> FunctionTool {
    FunctionTool::new(name, parameters)
        .expect("built-in tool schemas are objects")
        .with_description(description)
        .mutating(mutating)
}

/// Declarations for the complete, least-privilege graphics tool set.
///
/// JSON Schema publishes context-free scalar limits such as point count and
/// brush size. It cannot express the aggregate work bound because that bound
/// depends on the current canvas and the clipped rectangles of generated dabs;
/// redrob-core therefore preflights and authoritatively enforces that budget.
pub fn graphics_tool_declarations() -> Vec<FunctionTool> {
    let empty = || object(json!({}), &[]);
    let radius = || json!({ "type": "integer", "minimum": 0, "maximum": 4096 });
    let selection_shape = || {
        object(
            json!({ "rect": rect_property(), "mode": selection_mode_property() }),
            &["rect", "mode"],
        )
    };
    vec![
        declaration(
            "inspect_document",
            "Inspect dimensions, generation, active layer, ordered timeline, and playback metadata.",
            empty(),
            false,
        ),
        declaration(
            "add_frame",
            "Add a blank sparse raster frame at an optional timeline index.",
            object(
                json!({ "index": { "type": "integer", "minimum": 0, "maximum": MAX_FRAMES } }),
                &[],
            ),
            true,
        ),
        declaration(
            "duplicate_frame",
            "Duplicate all raster cels present at a source frame using clone-on-write storage.",
            object(
                json!({
                    "source_id": { "type": "integer", "minimum": 0, "maximum": u32::MAX },
                    "index": { "type": "integer", "minimum": 0, "maximum": MAX_FRAMES }
                }),
                &["source_id"],
            ),
            true,
        ),
        declaration(
            "remove_frame",
            "Remove a frame and its frame-keyed raster cels; the last frame cannot be removed.",
            object(
                json!({ "id": { "type": "integer", "minimum": 0, "maximum": u32::MAX } }),
                &["id"],
            ),
            true,
        ),
        declaration(
            "move_frame",
            "Move a stable frame identifier to a zero-based timeline index.",
            object(
                json!({
                    "id": { "type": "integer", "minimum": 0, "maximum": u32::MAX },
                    "new_index": { "type": "integer", "minimum": 0, "maximum": MAX_FRAMES - 1 }
                }),
                &["id", "new_index"],
            ),
            true,
        ),
        declaration(
            "set_timeline_fps",
            "Set the authoritative playback FPS and normalize all frame durations.",
            object(
                json!({ "fps": { "type": "number", "exclusiveMinimum": 0.0, "maximum": MAX_TIMELINE_FPS } }),
                &["fps"],
            ),
            true,
        ),
        declaration(
            "set_playback_range",
            "Set inclusive playback range endpoints in current timeline order.",
            object(
                json!({
                    "start": { "type": "integer", "minimum": 0, "maximum": u32::MAX },
                    "end": { "type": "integer", "minimum": 0, "maximum": u32::MAX }
                }),
                &["start", "end"],
            ),
            true,
        ),
        declaration(
            "set_looping",
            "Enable or disable playback range looping.",
            object(json!({ "looping": { "type": "boolean" } }), &["looping"]),
            true,
        ),
        declaration(
            "add_layer",
            "Add and activate a transparent root raster layer.",
            object(
                json!({
                    "name": { "type": "string", "minLength": 1 },
                    "index": { "type": "integer", "minimum": 0 }
                }),
                &["name"],
            ),
            true,
        ),
        declaration(
            "add_group",
            "Add and activate a layer group at an explicit hierarchy position.",
            object(
                json!({
                    "name": { "type": "string", "minLength": 1 },
                    "parent_id": optional_parent_property(),
                    "sibling_index": { "type": "integer", "minimum": 0 }
                }),
                &["name"],
            ),
            true,
        ),
        declaration(
            "add_text_node",
            "Add deterministic printable-ASCII text using the embedded font8x8 font.",
            object(
                json!({
                    "name": { "type": "string", "minLength": 1 },
                    "parent_id": optional_parent_property(),
                    "sibling_index": { "type": "integer", "minimum": 0 },
                    "text": text_content_property()
                }),
                &["name", "text"],
            ),
            true,
        ),
        declaration(
            "set_text_content",
            "Replace the bounded semantic content of an existing text node.",
            object(
                json!({ "id": layer_id_property(), "text": text_content_property() }),
                &["id", "text"],
            ),
            true,
        ),
        declaration(
            "add_vector_node",
            "Add bounded solid-fill/stroke vector paths rendered by the deterministic core.",
            object(
                json!({
                    "name": { "type": "string", "minLength": 1 },
                    "parent_id": optional_parent_property(),
                    "sibling_index": { "type": "integer", "minimum": 0 },
                    "vector": vector_content_property()
                }),
                &["name", "vector"],
            ),
            true,
        ),
        declaration(
            "set_vector_content",
            "Replace the bounded semantic paths of an existing vector node.",
            object(
                json!({ "id": layer_id_property(), "vector": vector_content_property() }),
                &["id", "vector"],
            ),
            true,
        ),
        declaration(
            "rasterize_semantic_node",
            "Convert one text or vector node to a current-frame-only raster cel while preserving node properties.",
            object(json!({ "id": layer_id_property() }), &["id"]),
            true,
        ),
        declaration(
            "move_node",
            "Move a node under a group or the document root at an explicit sibling position.",
            object(
                json!({
                    "id": layer_id_property(),
                    "parent_id": optional_parent_property(),
                    "sibling_index": { "type": "integer", "minimum": 0 }
                }),
                &["id", "parent_id", "sibling_index"],
            ),
            true,
        ),
        declaration(
            "add_raster_mask",
            "Attach an enabled white raster mask to a raster node or group.",
            object(json!({ "id": layer_id_property() }), &["id"]),
            true,
        ),
        declaration(
            "raster_mask_from_selection",
            "Copy the active selection coverage into an enabled mask, attaching one if needed.",
            object(json!({ "id": layer_id_property() }), &["id"]),
            true,
        ),
        declaration(
            "remove_raster_mask",
            "Remove an attached raster mask from a raster node or group.",
            object(json!({ "id": layer_id_property() }), &["id"]),
            true,
        ),
        declaration(
            "set_raster_mask_enabled",
            "Enable or disable an attached raster mask.",
            object(
                json!({ "id": layer_id_property(), "enabled": { "type": "boolean" } }),
                &["id", "enabled"],
            ),
            true,
        ),
        declaration(
            "replace_raster_mask",
            "Replace a bounded in-canvas mask rectangle with row-major 8-bit coverage.",
            object(
                json!({
                    "id": layer_id_property(),
                    "rect": rect_property(),
                    "pixels": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": MAX_MASK_COMMAND_PIXELS,
                        "items": { "type": "integer", "minimum": 0, "maximum": 255 }
                    }
                }),
                &["id", "rect", "pixels"],
            ),
            true,
        ),
        declaration(
            "rename_layer",
            "Rename a layer by stable identifier.",
            object(
                json!({ "id": layer_id_property(), "name": { "type": "string", "minLength": 1 } }),
                &["id", "name"],
            ),
            true,
        ),
        declaration(
            "set_layer_opacity",
            "Set layer opacity from zero through one.",
            object(
                json!({ "id": layer_id_property(), "opacity": { "type": "number", "minimum": 0.0, "maximum": 1.0 } }),
                &["id", "opacity"],
            ),
            true,
        ),
        declaration(
            "set_layer_visibility",
            "Show or hide a layer.",
            object(
                json!({ "id": layer_id_property(), "visible": { "type": "boolean" } }),
                &["id", "visible"],
            ),
            true,
        ),
        declaration(
            "fill",
            "Fill the active layer or selection with an RGBA color.",
            object(json!({ "color": color_property() }), &["color"]),
            true,
        ),
        declaration(
            "brush_stroke",
            "Paint a pressure-sensitive stroke with optional smoothing and mirror symmetry.",
            object(
                json!({
                    "points": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": MAX_BRUSH_POINTS,
                        "items": object(
                            json!({
                                "x": { "type": "number" },
                                "y": { "type": "number" },
                                "pressure": { "type": "number", "minimum": 0.0, "maximum": 1.0 }
                            }),
                            &["x", "y", "pressure"],
                        )
                    },
                    "color": color_property(),
                    "size": {
                        "type": "number",
                        "exclusiveMinimum": 0.0,
                        "maximum": MAX_BRUSH_SIZE,
                        "description": "Brush diameter in canvas pixels; aggregate clipped raster work is enforced by redrob-core."
                    },
                    "opacity": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                    "settings": object(
                        json!({
                            "smoothing": {
                                "oneOf": [
                                    object(json!({ "kind": { "const": "none" } }), &["kind"]),
                                    object(
                                        json!({
                                            "kind": { "const": "moving_average" },
                                            "window": { "type": "integer", "minimum": 2, "maximum": 64 }
                                        }),
                                        &["kind", "window"],
                                    )
                                ]
                            },
                            "mirror_x": { "type": "number" },
                            "mirror_y": { "type": "number" }
                        }),
                        &[],
                    )
                }),
                &["points", "color", "size", "opacity"],
            ),
            true,
        ),
        declaration(
            "select_rectangle",
            "Combine a rectangular region with the current selection.",
            selection_shape(),
            true,
        ),
        declaration(
            "select_ellipse",
            "Combine an elliptical region with the current selection.",
            selection_shape(),
            true,
        ),
        declaration("select_all", "Select every canvas pixel.", empty(), true),
        declaration(
            "invert_selection",
            "Invert the current selection mask.",
            empty(),
            true,
        ),
        declaration(
            "feather_selection",
            "Soften the selection boundary by a pixel radius.",
            object(json!({ "radius": radius() }), &["radius"]),
            true,
        ),
        declaration(
            "grow_selection",
            "Expand the selection by a pixel radius.",
            object(json!({ "radius": radius() }), &["radius"]),
            true,
        ),
        declaration(
            "shrink_selection",
            "Contract the selection by a pixel radius.",
            object(json!({ "radius": radius() }), &["radius"]),
            true,
        ),
        declaration(
            "clear_selection",
            "Clear the selection so edits affect the full canvas.",
            empty(),
            true,
        ),
        declaration(
            "gradient_fill",
            "Fill the active layer or selection with a linear or radial RGBA gradient.",
            object(
                json!({
                    "kind": {
                        "oneOf": [
                            object(
                                json!({
                                    "kind": { "const": "linear" },
                                    "start_x": { "type": "number" },
                                    "start_y": { "type": "number" },
                                    "end_x": { "type": "number" },
                                    "end_y": { "type": "number" }
                                }),
                                &["kind", "start_x", "start_y", "end_x", "end_y"],
                            ),
                            object(
                                json!({
                                    "kind": { "const": "radial" },
                                    "center_x": { "type": "number" },
                                    "center_y": { "type": "number" },
                                    "radius": { "type": "number", "exclusiveMinimum": 0.0 }
                                }),
                                &["kind", "center_x", "center_y", "radius"],
                            )
                        ]
                    },
                    "stops": {
                        "type": "array",
                        "minItems": 2,
                        "maxItems": 32,
                        "items": object(
                            json!({
                                "position": { "type": "number", "minimum": 0.0, "maximum": 1.0 },
                                "color": color_property()
                            }),
                            &["position", "color"],
                        )
                    }
                }),
                &["kind", "stops"],
            ),
            true,
        ),
        declaration(
            "crop_canvas",
            "Crop or pad the canvas to a rectangle.",
            object(json!({ "rect": rect_property() }), &["rect"]),
            true,
        ),
        declaration(
            "resize_canvas",
            "Resize the canvas and all layers with the selected sampling mode.",
            object(
                json!({
                    "width": { "type": "integer", "minimum": 1, "maximum": 32768 },
                    "height": { "type": "integer", "minimum": 1, "maximum": 32768 },
                    "sampling": sampling_property()
                }),
                &["width", "height", "sampling"],
            ),
            true,
        ),
        declaration(
            "flip_active",
            "Flip the active layer horizontally, vertically, or both.",
            object(
                json!({ "horizontal": { "type": "boolean" }, "vertical": { "type": "boolean" } }),
                &["horizontal", "vertical"],
            ),
            true,
        ),
        declaration(
            "rotate_active_90",
            "Rotate the active layer 90 degrees around the canvas center.",
            object(
                json!({ "clockwise": { "type": "boolean" } }),
                &["clockwise"],
            ),
            true,
        ),
        declaration(
            "transform_active",
            "Apply an invertible affine transform to the active layer.",
            object(
                json!({
                    "transform": object(
                        json!({
                            "m11": { "type": "number" },
                            "m12": { "type": "number" },
                            "m21": { "type": "number" },
                            "m22": { "type": "number" },
                            "tx": { "type": "number" },
                            "ty": { "type": "number" }
                        }),
                        &["m11", "m12", "m21", "m22", "tx", "ty"],
                    ),
                    "sampling": sampling_property()
                }),
                &["transform", "sampling"],
            ),
            true,
        ),
        declaration(
            "apply_filter",
            "Apply a typed raster filter to the active layer.",
            object(
                json!({
                    "filter": {
                        "oneOf": [
                            object(json!({ "kind": { "const": "invert" } }), &["kind"]),
                            object(json!({ "kind": { "const": "grayscale" } }), &["kind"]),
                            object(json!({
                                "kind": { "const": "brightness_contrast" },
                                "brightness": { "type": "integer", "minimum": -255, "maximum": 255 },
                                "contrast": { "type": "number", "minimum": -100.0, "maximum": 100.0 }
                            }), &["kind", "brightness", "contrast"]),
                            object(json!({
                                "kind": { "const": "gaussian_blur" },
                                "sigma": { "type": "number", "exclusiveMinimum": 0.0, "maximum": 1024.0 }
                            }), &["kind", "sigma"]),
                            object(json!({
                                "kind": { "const": "threshold" },
                                "threshold": { "type": "integer", "minimum": 0, "maximum": 255 }
                            }), &["kind", "threshold"]),
                            object(json!({
                                "kind": { "const": "posterize" },
                                "levels": { "type": "integer", "minimum": 2, "maximum": 256 }
                            }), &["kind", "levels"]),
                            object(json!({
                                "kind": { "const": "levels" },
                                "input_black": { "type": "integer", "minimum": 0, "maximum": 255 },
                                "input_white": { "type": "integer", "minimum": 0, "maximum": 255 },
                                "gamma": { "type": "number", "minimum": 0.01, "maximum": 100.0 },
                                "output_black": { "type": "integer", "minimum": 0, "maximum": 255 },
                                "output_white": { "type": "integer", "minimum": 0, "maximum": 255 }
                            }), &["kind", "input_black", "input_white", "gamma", "output_black", "output_white"]),
                            object(json!({
                                "kind": { "const": "hue_saturation" },
                                "hue_degrees": { "type": "number", "minimum": -180.0, "maximum": 180.0 },
                                "saturation": { "type": "number", "minimum": -100.0, "maximum": 100.0 },
                                "lightness": { "type": "number", "minimum": -100.0, "maximum": 100.0 }
                            }), &["kind", "hue_degrees", "saturation", "lightness"]),
                            object(json!({
                                "kind": { "const": "box_blur" },
                                "radius": { "type": "integer", "minimum": 1, "maximum": 4096 }
                            }), &["kind", "radius"]),
                            object(json!({
                                "kind": { "const": "sharpen" },
                                "amount": { "type": "number", "minimum": 0.0, "maximum": 10.0 }
                            }), &["kind", "amount"])
                        ]
                    }
                }),
                &["filter"],
            ),
            true,
        ),
        declaration("undo", "Undo the latest committed edit.", empty(), true),
        declaration("redo", "Redo the latest undone edit.", empty(), true),
    ]
}

/// Mutating declarations suitable for one-shot, review-only proposal requests.
pub fn proposal_tool_declarations() -> Vec<FunctionTool> {
    graphics_tool_declarations()
        .into_iter()
        .filter(|tool| tool.mutating)
        .collect()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyArgs {}

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct ColorArgs {
    r: u8,
    g: u8,
    b: u8,
    a: u8,
}

impl From<ColorArgs> for Pixel {
    fn from(value: ColorArgs) -> Self {
        Pixel::rgba(value.r, value.g, value.b, value.a)
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct PointArgs {
    x: f32,
    y: f32,
    pressure: f32,
}

impl From<PointArgs> for BrushPoint {
    fn from(value: PointArgs) -> Self {
        BrushPoint::new(value.x, value.y, value.pressure)
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct RectArgs {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

impl From<RectArgs> for Rect {
    fn from(value: RectArgs) -> Self {
        Rect::new(value.x, value.y, value.width, value.height)
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ToolSelectionMode {
    Replace,
    Add,
    Subtract,
    Intersect,
}

impl From<ToolSelectionMode> for SelectionMode {
    fn from(value: ToolSelectionMode) -> Self {
        match value {
            ToolSelectionMode::Replace => Self::Replace,
            ToolSelectionMode::Add => Self::Add,
            ToolSelectionMode::Subtract => Self::Subtract,
            ToolSelectionMode::Intersect => Self::Intersect,
        }
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ToolSamplingMode {
    Nearest,
    Bilinear,
}

impl From<ToolSamplingMode> for SamplingMode {
    fn from(value: ToolSamplingMode) -> Self {
        match value {
            ToolSamplingMode::Nearest => Self::Nearest,
            ToolSamplingMode::Bilinear => Self::Bilinear,
        }
    }
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ToolBrushSmoothing {
    #[default]
    None,
    MovingAverage {
        window: u8,
    },
}

impl From<ToolBrushSmoothing> for BrushSmoothing {
    fn from(value: ToolBrushSmoothing) -> Self {
        match value {
            ToolBrushSmoothing::None => Self::None,
            ToolBrushSmoothing::MovingAverage { window } => Self::MovingAverage { window },
        }
    }
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolBrushSettings {
    #[serde(default)]
    smoothing: ToolBrushSmoothing,
    #[serde(default, deserialize_with = "deserialize_present_f32")]
    mirror_x: Option<f32>,
    #[serde(default, deserialize_with = "deserialize_present_f32")]
    mirror_y: Option<f32>,
}

fn deserialize_present_f32<'de, D>(deserializer: D) -> std::result::Result<Option<f32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    f32::deserialize(deserializer).map(Some)
}

impl From<ToolBrushSettings> for BrushSettings {
    fn from(value: ToolBrushSettings) -> Self {
        Self {
            smoothing: value.smoothing.into(),
            mirror_x: value.mirror_x,
            mirror_y: value.mirror_y,
        }
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ToolGradientKind {
    Linear {
        start_x: f32,
        start_y: f32,
        end_x: f32,
        end_y: f32,
    },
    Radial {
        center_x: f32,
        center_y: f32,
        radius: f32,
    },
}

impl From<ToolGradientKind> for GradientKind {
    fn from(value: ToolGradientKind) -> Self {
        match value {
            ToolGradientKind::Linear {
                start_x,
                start_y,
                end_x,
                end_y,
            } => Self::Linear {
                start_x,
                start_y,
                end_x,
                end_y,
            },
            ToolGradientKind::Radial {
                center_x,
                center_y,
                radius,
            } => Self::Radial {
                center_x,
                center_y,
                radius,
            },
        }
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct GradientStopArgs {
    position: f32,
    color: ColorArgs,
}

impl From<GradientStopArgs> for GradientStop {
    fn from(value: GradientStopArgs) -> Self {
        GradientStop::new(value.position, value.color.into())
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ToolFilter {
    Invert,
    Grayscale,
    BrightnessContrast {
        brightness: i16,
        contrast: f32,
    },
    GaussianBlur {
        sigma: f32,
    },
    Threshold {
        threshold: u8,
    },
    Posterize {
        levels: u16,
    },
    Levels {
        input_black: u8,
        input_white: u8,
        gamma: f32,
        output_black: u8,
        output_white: u8,
    },
    HueSaturation {
        hue_degrees: f32,
        saturation: f32,
        lightness: f32,
    },
    BoxBlur {
        radius: u32,
    },
    Sharpen {
        amount: f32,
    },
}

impl From<ToolFilter> for Filter {
    fn from(value: ToolFilter) -> Self {
        match value {
            ToolFilter::Invert => Self::Invert,
            ToolFilter::Grayscale => Self::Grayscale,
            ToolFilter::BrightnessContrast {
                brightness,
                contrast,
            } => Self::BrightnessContrast {
                brightness,
                contrast,
            },
            ToolFilter::GaussianBlur { sigma } => Self::GaussianBlur { sigma },
            ToolFilter::Threshold { threshold } => Self::Threshold { threshold },
            ToolFilter::Posterize { levels } => Self::Posterize { levels },
            ToolFilter::Levels {
                input_black,
                input_white,
                gamma,
                output_black,
                output_white,
            } => Self::Levels {
                input_black,
                input_white,
                gamma,
                output_black,
                output_white,
            },
            ToolFilter::HueSaturation {
                hue_degrees,
                saturation,
                lightness,
            } => Self::HueSaturation {
                hue_degrees,
                saturation,
                lightness,
            },
            ToolFilter::BoxBlur { radius } => Self::BoxBlur { radius },
            ToolFilter::Sharpen { amount } => Self::Sharpen { amount },
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddFrameArgs {
    index: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DuplicateFrameArgs {
    source_id: FrameId,
    index: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FrameArgs {
    id: FrameId,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveFrameArgs {
    id: FrameId,
    new_index: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TimelineFpsArgs {
    fps: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaybackRangeArgs {
    start: FrameId,
    end: FrameId,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoopingArgs {
    looping: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddLayerArgs {
    name: String,
    index: Option<usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddGroupArgs {
    name: String,
    #[serde(default)]
    parent_id: Option<LayerId>,
    sibling_index: Option<usize>,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ToolFillRule {
    #[default]
    NonZero,
    EvenOdd,
}

impl From<ToolFillRule> for FillRule {
    fn from(value: ToolFillRule) -> Self {
        match value {
            ToolFillRule::NonZero => Self::NonZero,
            ToolFillRule::EvenOdd => Self::EvenOdd,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolTextContent {
    text: String,
    #[serde(default = "default_tool_font_family")]
    font_family: String,
    font_size: f64,
    color: ColorArgs,
    origin_x: f64,
    origin_y: f64,
}

fn default_tool_font_family() -> String {
    "font8x8 Basic Latin".into()
}

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolStrokeStyle {
    color: ColorArgs,
    width: f64,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum ToolPathCommand {
    MoveTo {
        x: f64,
        y: f64,
    },
    LineTo {
        x: f64,
        y: f64,
    },
    CubicTo {
        control1_x: f64,
        control1_y: f64,
        control2_x: f64,
        control2_y: f64,
        x: f64,
        y: f64,
    },
    Close,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolVectorPath {
    commands: Vec<ToolPathCommand>,
    #[serde(default)]
    fill: Option<ColorArgs>,
    #[serde(default)]
    stroke: Option<ToolStrokeStyle>,
    #[serde(default)]
    fill_rule: ToolFillRule,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolVectorContent {
    paths: Vec<ToolVectorPath>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddTextArgs {
    name: String,
    #[serde(default)]
    parent_id: Option<LayerId>,
    sibling_index: Option<usize>,
    text: ToolTextContent,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetTextArgs {
    id: LayerId,
    text: ToolTextContent,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AddVectorArgs {
    name: String,
    #[serde(default)]
    parent_id: Option<LayerId>,
    sibling_index: Option<usize>,
    vector: ToolVectorContent,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetVectorArgs {
    id: LayerId,
    vector: ToolVectorContent,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MoveNodeArgs {
    id: LayerId,
    parent_id: Value,
    sibling_index: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeArgs {
    id: LayerId,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ToggleMaskArgs {
    id: LayerId,
    enabled: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplaceMaskArgs {
    id: LayerId,
    rect: RectArgs,
    pixels: Vec<u8>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RenameLayerArgs {
    id: LayerId,
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpacityArgs {
    id: LayerId,
    opacity: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VisibilityArgs {
    id: LayerId,
    visible: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FillArgs {
    color: ColorArgs,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StrokeArgs {
    points: Vec<PointArgs>,
    color: ColorArgs,
    // Keep provider precision until the schema boundary has been validated;
    // deserializing directly to f32 could round a just-over-limit value down.
    size: f64,
    opacity: f32,
    #[serde(default)]
    settings: ToolBrushSettings,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SelectionShapeArgs {
    rect: RectArgs,
    mode: ToolSelectionMode,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RadiusArgs {
    radius: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GradientArgs {
    kind: ToolGradientKind,
    stops: Vec<GradientStopArgs>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CropArgs {
    rect: RectArgs,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResizeArgs {
    width: u32,
    height: u32,
    sampling: ToolSamplingMode,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FlipArgs {
    horizontal: bool,
    vertical: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RotateArgs {
    clockwise: bool,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct AffineArgs {
    m11: f32,
    m12: f32,
    m21: f32,
    m22: f32,
    tx: f32,
    ty: f32,
}

impl From<AffineArgs> for Affine2D {
    fn from(value: AffineArgs) -> Self {
        Affine2D::new(
            value.m11, value.m12, value.m21, value.m22, value.tx, value.ty,
        )
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TransformArgs {
    transform: AffineArgs,
    sampling: ToolSamplingMode,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FilterArgs {
    filter: ToolFilter,
}

fn decode<T: for<'de> Deserialize<'de>>(call: &ToolCall) -> Result<T> {
    serde_json::from_value(call.arguments.clone())
        .map_err(|error| Error::Tool(format!("invalid `{}` arguments: {error}", call.name)))
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProposalNodeFact {
    id: LayerId,
    parent: Option<LayerId>,
    kind: NodeKind,
    depth: usize,
    has_mask: bool,
    mask_enabled: bool,
    cel_frames: Vec<FrameId>,
    semantic_usage: SemanticUsage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ProposalFrameFact {
    id: FrameId,
    duration_ms: u32,
}

/// Immutable document facts needed to turn a tool call into a typed action.
#[derive(Clone, Debug, PartialEq)]
pub struct ProposalContext {
    base_generation: u64,
    document: Option<Document>,
    width: u32,
    height: u32,
    active_node: LayerId,
    selection_active: bool,
    nodes: Vec<ProposalNodeFact>,
    frames: Vec<ProposalFrameFact>,
    current_frame: FrameId,
    fps: f32,
    range_start: FrameId,
    range_end: FrameId,
    looping: bool,
    can_undo: bool,
    can_redo: bool,
    semantic_usage: SemanticUsage,
}

impl ProposalContext {
    pub fn from_editor(editor: &Editor) -> Self {
        Self::from_editor_with_document(editor, Some(editor.document().clone()))
    }

    fn from_editor_for_direct_execution(editor: &Editor) -> Self {
        Self::from_editor_with_document(editor, None)
    }

    fn from_editor_with_document(editor: &Editor, document_snapshot: Option<Document>) -> Self {
        let document = editor.document();
        Self {
            base_generation: editor.generation(),
            document: document_snapshot,
            width: document.width(),
            height: document.height(),
            active_node: document.active_layer_id(),
            selection_active: document.is_selection_active(),
            nodes: document
                .nodes()
                .iter()
                .map(|node| ProposalNodeFact {
                    id: node.id(),
                    parent: node.parent_id(),
                    kind: node.kind(),
                    depth: document.node_depth(node.id()).unwrap_or(0),
                    has_mask: node.mask().is_some(),
                    mask_enabled: node.mask().is_some_and(|mask| mask.is_enabled()),
                    cel_frames: node.raster_cels().map_or_else(Vec::new, |cels| {
                        cels.iter().map(|cel| cel.frame()).collect()
                    }),
                    semantic_usage: semantic_usage(node.content())
                        .expect("validated document semantic usage"),
                })
                .collect(),
            frames: document
                .timeline()
                .frames()
                .iter()
                .map(|frame| ProposalFrameFact {
                    id: frame.id(),
                    duration_ms: frame.duration_ms(),
                })
                .collect(),
            current_frame: document.timeline().current_frame(),
            fps: document.timeline().fps(),
            range_start: document.timeline().playback().range_start,
            range_end: document.timeline().playback().range_end,
            looping: document.timeline().playback().looping,
            can_undo: editor.can_undo(),
            can_redo: editor.can_redo(),
            semantic_usage: document
                .semantic_usage()
                .expect("validated document semantic usage"),
        }
    }

    pub fn base_generation(&self) -> u64 {
        self.base_generation
    }

    fn root_count(&self) -> usize {
        self.nodes
            .iter()
            .filter(|node| node.parent.is_none())
            .count()
    }

    fn frame_index(&self, id: FrameId) -> Option<usize> {
        self.frames.iter().position(|frame| frame.id == id)
    }

    fn contains_frame(&self, id: FrameId) -> bool {
        self.frame_index(id).is_some()
    }

    fn node(&self, id: LayerId) -> Option<&ProposalNodeFact> {
        self.nodes.iter().find(|node| node.id == id)
    }

    fn sibling_count(&self, parent: Option<LayerId>, excluding: Option<LayerId>) -> usize {
        self.nodes
            .iter()
            .filter(|node| node.parent == parent && Some(node.id) != excluding)
            .count()
    }

    fn destination_depth(&self, parent: Option<LayerId>) -> Option<usize> {
        parent
            .map(|id| self.node(id)?.depth.checked_add(1))
            .unwrap_or(Some(0))
    }

    fn subtree_height(&self, root: LayerId) -> Option<usize> {
        let root_depth = self.node(root)?.depth;
        self.nodes
            .iter()
            .filter_map(|candidate| {
                let mut current = Some(candidate.id);
                while let Some(id) = current {
                    if id == root {
                        return candidate.depth.checked_sub(root_depth);
                    }
                    current = self.node(id).and_then(|node| node.parent);
                }
                None
            })
            .max()
    }

    fn validate_destination_depth(&self, parent: Option<LayerId>, subtree_height: usize) -> bool {
        self.destination_depth(parent)
            .and_then(|depth| depth.checked_add(subtree_height))
            .is_some_and(|deepest| deepest <= MAX_HIERARCHY_DEPTH)
    }
}

impl From<&Editor> for ProposalContext {
    fn from(editor: &Editor) -> Self {
        Self::from_editor(editor)
    }
}

/// A typed action that remains inert until a native host explicitly applies it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProposalAction {
    /// Exact JSON representation of a [`redrob_core::Command`].
    Command {
        command: Command,
    },
    Undo,
    Redo,
}

/// Reviewable, non-executing interpretation of one graphics tool call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProposalDto {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub base_generation: u64,
    pub action: ProposalAction,
}

#[derive(Clone, Debug, PartialEq)]
enum TypedActionKind {
    Inspect,
    Command(Command),
    Undo,
    Redo,
}

#[derive(Clone, Debug, PartialEq)]
struct TypedToolAction {
    title: String,
    summary: String,
    kind: TypedActionKind,
}

fn invalid_arguments(call: &ToolCall, detail: &str) -> Error {
    Error::Tool(format!("invalid `{}` arguments: {detail}", call.name))
}

fn validate_name(call: &ToolCall, name: &str) -> Result<()> {
    if name.trim().is_empty() {
        return Err(invalid_arguments(call, "name must not be blank"));
    }
    Ok(())
}

fn validate_layer_id(call: &ToolCall, context: &ProposalContext, id: LayerId) -> Result<()> {
    if context.node(id).is_none() {
        return Err(invalid_arguments(call, "node id does not exist"));
    }
    Ok(())
}

fn validate_group_parent(
    call: &ToolCall,
    context: &ProposalContext,
    parent: Option<LayerId>,
) -> Result<()> {
    if let Some(parent) = parent
        && context.node(parent).map(|node| node.kind) != Some(NodeKind::Group)
    {
        return Err(invalid_arguments(call, "parent id must identify a group"));
    }
    Ok(())
}

fn validate_active_raster(call: &ToolCall, context: &ProposalContext) -> Result<()> {
    if context.node(context.active_node).map(|node| node.kind) != Some(NodeKind::Raster) {
        return Err(invalid_arguments(call, "active node must be a raster node"));
    }
    Ok(())
}

fn validate_mask_capable<'a>(
    call: &ToolCall,
    context: &'a ProposalContext,
    id: LayerId,
) -> Result<&'a ProposalNodeFact> {
    let node = context
        .node(id)
        .ok_or_else(|| invalid_arguments(call, "node id does not exist"))?;
    if !matches!(node.kind, NodeKind::Raster | NodeKind::Group) {
        return Err(invalid_arguments(
            call,
            "node kind does not support raster masks",
        ));
    }
    Ok(node)
}

fn validate_mask_target<'a>(
    call: &ToolCall,
    context: &'a ProposalContext,
    id: LayerId,
    require_mask: bool,
) -> Result<&'a ProposalNodeFact> {
    let node = validate_mask_capable(call, context, id)?;
    if require_mask && !node.has_mask {
        return Err(invalid_arguments(call, "node has no raster mask"));
    }
    if !require_mask && node.has_mask {
        return Err(invalid_arguments(call, "node already has a raster mask"));
    }
    Ok(node)
}

fn validate_finite(call: &ToolCall, field: &str, value: f32) -> Result<()> {
    if !value.is_finite() {
        return Err(invalid_arguments(call, &format!("{field} must be finite")));
    }
    Ok(())
}

fn validate_unit(call: &ToolCall, field: &str, value: f32) -> Result<()> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(invalid_arguments(
            call,
            &format!("{field} must be between zero and one"),
        ));
    }
    Ok(())
}

fn validate_rect(call: &ToolCall, rect: Rect) -> Result<()> {
    validate_dimensions(call, rect.width, rect.height)
}

fn validate_dimensions(call: &ToolCall, width: u32, height: u32) -> Result<()> {
    const MAX_DIMENSION: u32 = 32_768;
    const MAX_PIXELS: u64 = 64 * 1024 * 1024;
    if width == 0
        || height == 0
        || width > MAX_DIMENSION
        || height > MAX_DIMENSION
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        return Err(invalid_arguments(call, "dimensions are out of range"));
    }
    Ok(())
}

fn validate_selection_radius(call: &ToolCall, radius: u32) -> Result<()> {
    if radius > 4_096 {
        return Err(invalid_arguments(call, "selection radius is out of range"));
    }
    Ok(())
}

fn validate_brush(
    call: &ToolCall,
    points: &[PointArgs],
    size: f64,
    opacity: f32,
    settings: BrushSettings,
) -> Result<()> {
    if points.is_empty() {
        return Err(invalid_arguments(call, "points must not be empty"));
    }
    if points.len() > MAX_BRUSH_POINTS {
        return Err(invalid_arguments(
            call,
            "point count exceeds the supported maximum",
        ));
    }
    if !size.is_finite() || size <= 0.0 || size > f64::from(MAX_BRUSH_SIZE) {
        return Err(invalid_arguments(
            call,
            "size must be greater than zero and no larger than the supported maximum",
        ));
    }
    validate_unit(call, "opacity", opacity)?;
    if matches!(
        settings.smoothing,
        BrushSmoothing::MovingAverage { window } if !(2..=64).contains(&window)
    ) {
        return Err(invalid_arguments(
            call,
            "smoothing window must be between 2 and 64",
        ));
    }
    if let Some(axis) = settings.mirror_x {
        validate_finite(call, "mirror_x", axis)?;
    }
    if let Some(axis) = settings.mirror_y {
        validate_finite(call, "mirror_y", axis)?;
    }
    for point in points {
        validate_finite(call, "point x", point.x)?;
        validate_finite(call, "point y", point.y)?;
        validate_unit(call, "point pressure", point.pressure)?;
        if settings.mirror_x.is_some_and(|axis| {
            (f64::from(axis) * 2.0 - f64::from(point.x)).abs() > f64::from(f32::MAX)
        }) || settings.mirror_y.is_some_and(|axis| {
            (f64::from(axis) * 2.0 - f64::from(point.y)).abs() > f64::from(f32::MAX)
        }) {
            return Err(invalid_arguments(
                call,
                "mirrored point coordinates are out of range",
            ));
        }
    }
    Ok(())
}

fn validate_gradient(call: &ToolCall, kind: GradientKind, stops: &[GradientStop]) -> Result<()> {
    if !(2..=32).contains(&stops.len()) {
        return Err(invalid_arguments(
            call,
            "gradient must contain 2 to 32 stops",
        ));
    }
    let mut previous = -1.0_f32;
    for stop in stops {
        if !stop.position.is_finite()
            || !(0.0..=1.0).contains(&stop.position)
            || stop.position <= previous
        {
            return Err(invalid_arguments(
                call,
                "gradient stop positions must be finite, increasing, and between zero and one",
            ));
        }
        previous = stop.position;
    }
    match kind {
        GradientKind::Linear {
            start_x,
            start_y,
            end_x,
            end_y,
        } => {
            for (field, value) in [
                ("start_x", start_x),
                ("start_y", start_y),
                ("end_x", end_x),
                ("end_y", end_y),
            ] {
                validate_finite(call, field, value)?;
            }
            let dx = end_x - start_x;
            let dy = end_y - start_y;
            let length_squared = dx * dx + dy * dy;
            if !length_squared.is_finite() || length_squared <= f32::EPSILON {
                return Err(invalid_arguments(
                    call,
                    "linear gradient endpoints must be distinct and in range",
                ));
            }
        }
        GradientKind::Radial {
            center_x,
            center_y,
            radius,
        } => {
            validate_finite(call, "center_x", center_x)?;
            validate_finite(call, "center_y", center_y)?;
            if !radius.is_finite() || radius <= 0.0 {
                return Err(invalid_arguments(
                    call,
                    "radial gradient radius must be greater than zero",
                ));
            }
        }
    }
    Ok(())
}

fn validate_transform(call: &ToolCall, transform: Affine2D) -> Result<()> {
    for (field, value) in [
        ("m11", transform.m11),
        ("m12", transform.m12),
        ("m21", transform.m21),
        ("m22", transform.m22),
        ("tx", transform.tx),
        ("ty", transform.ty),
    ] {
        validate_finite(call, field, value)?;
    }
    let determinant = f64::from(transform.m11) * f64::from(transform.m22)
        - f64::from(transform.m12) * f64::from(transform.m21);
    if determinant == 0.0 {
        return Err(invalid_arguments(call, "transform must be invertible"));
    }
    Ok(())
}

fn validate_filter(call: &ToolCall, filter: &Filter) -> Result<()> {
    match filter {
        Filter::Invert | Filter::Grayscale | Filter::Threshold { .. } => Ok(()),
        Filter::BrightnessContrast {
            brightness,
            contrast,
        } => {
            if !(-255..=255).contains(brightness)
                || !contrast.is_finite()
                || !(-100.0..=100.0).contains(contrast)
            {
                return Err(invalid_arguments(
                    call,
                    "brightness/contrast parameters are out of range",
                ));
            }
            Ok(())
        }
        Filter::GaussianBlur { sigma } => {
            if !sigma.is_finite() || *sigma <= 0.0 || *sigma > 1_024.0 {
                return Err(invalid_arguments(call, "blur sigma is out of range"));
            }
            Ok(())
        }
        Filter::Posterize { levels } => {
            if !(2..=256).contains(levels) {
                return Err(invalid_arguments(call, "posterize levels are out of range"));
            }
            Ok(())
        }
        Filter::Levels {
            input_black,
            input_white,
            gamma,
            output_black,
            output_white,
        } => {
            if input_black >= input_white
                || output_black > output_white
                || !gamma.is_finite()
                || !(0.01..=100.0).contains(gamma)
            {
                return Err(invalid_arguments(call, "levels parameters are invalid"));
            }
            Ok(())
        }
        Filter::HueSaturation {
            hue_degrees,
            saturation,
            lightness,
        } => {
            if !hue_degrees.is_finite()
                || !saturation.is_finite()
                || !lightness.is_finite()
                || !(-180.0..=180.0).contains(hue_degrees)
                || !(-100.0..=100.0).contains(saturation)
                || !(-100.0..=100.0).contains(lightness)
            {
                return Err(invalid_arguments(
                    call,
                    "hue/saturation parameters are out of range",
                ));
            }
            Ok(())
        }
        Filter::BoxBlur { radius } => {
            if !(1..=4_096).contains(radius) {
                return Err(invalid_arguments(call, "box blur radius is out of range"));
            }
            Ok(())
        }
        Filter::Sharpen { amount } => {
            if !amount.is_finite() || !(0.0..=10.0).contains(amount) {
                return Err(invalid_arguments(call, "sharpen amount is out of range"));
            }
            Ok(())
        }
    }
}

fn color_summary(color: Pixel) -> String {
    format!(
        "#{:02X}{:02X}{:02X}{:02X}",
        color.r, color.g, color.b, color.a
    )
}

fn rect_summary(rect: Rect) -> String {
    format!("{}x{} at ({}, {})", rect.width, rect.height, rect.x, rect.y)
}

fn selection_mode_summary(mode: SelectionMode) -> &'static str {
    match mode {
        SelectionMode::Replace => "replace",
        SelectionMode::Add => "add to",
        SelectionMode::Subtract => "subtract from",
        SelectionMode::Intersect => "intersect with",
    }
}

fn sampling_summary(sampling: SamplingMode) -> &'static str {
    match sampling {
        SamplingMode::Nearest => "nearest-neighbor",
        SamplingMode::Bilinear => "bilinear",
    }
}

fn filter_summary(filter: &Filter) -> String {
    match filter {
        Filter::Invert => "Invert the active layer's RGB channels.".into(),
        Filter::Grayscale => "Convert the active layer to grayscale.".into(),
        Filter::BrightnessContrast {
            brightness,
            contrast,
        } => format!("Adjust active-layer brightness by {brightness} and contrast by {contrast}."),
        Filter::GaussianBlur { sigma } => {
            format!("Apply a Gaussian blur with sigma {sigma} to the active layer.")
        }
        Filter::Threshold { threshold } => {
            format!("Threshold the active layer at luminance {threshold}.")
        }
        Filter::Posterize { levels } => {
            format!("Posterize the active layer to {levels} levels per RGB channel.")
        }
        Filter::Levels {
            input_black,
            input_white,
            gamma,
            output_black,
            output_white,
        } => format!(
            "Map active-layer levels from {input_black}..{input_white} through gamma {gamma} to {output_black}..{output_white}."
        ),
        Filter::HueSaturation {
            hue_degrees,
            saturation,
            lightness,
        } => format!(
            "Adjust active-layer hue by {hue_degrees} degrees, saturation by {saturation}, and lightness by {lightness}."
        ),
        Filter::BoxBlur { radius } => {
            format!("Apply a box blur with radius {radius} to the active layer.")
        }
        Filter::Sharpen { amount } => {
            format!("Sharpen the active layer by amount {amount}.")
        }
    }
}

fn semantic_number(call: &ToolCall, field: &str, value: f64) -> Result<f32> {
    if !value.is_finite() || value.abs() > f64::from(MAX_SEMANTIC_COORDINATE) {
        return Err(invalid_arguments(
            call,
            &format!("{field} is outside the semantic coordinate range"),
        ));
    }
    Ok(value as f32)
}

fn tool_text_content(call: &ToolCall, value: ToolTextContent) -> Result<TextContent> {
    let text = TextContent {
        text: value.text,
        font_family: value.font_family,
        font_size: semantic_number(call, "font_size", value.font_size)?,
        color: value.color.into(),
        origin_x: semantic_number(call, "origin_x", value.origin_x)?,
        origin_y: semantic_number(call, "origin_y", value.origin_y)?,
        font_id: EMBEDDED_FONT_ID.into(),
    };
    Ok(text)
}

fn tool_path_command(call: &ToolCall, value: ToolPathCommand) -> Result<PathCommand> {
    Ok(match value {
        ToolPathCommand::MoveTo { x, y } => PathCommand::MoveTo {
            x: semantic_number(call, "x", x)?,
            y: semantic_number(call, "y", y)?,
        },
        ToolPathCommand::LineTo { x, y } => PathCommand::LineTo {
            x: semantic_number(call, "x", x)?,
            y: semantic_number(call, "y", y)?,
        },
        ToolPathCommand::CubicTo {
            control1_x,
            control1_y,
            control2_x,
            control2_y,
            x,
            y,
        } => PathCommand::CubicTo {
            control1_x: semantic_number(call, "control1_x", control1_x)?,
            control1_y: semantic_number(call, "control1_y", control1_y)?,
            control2_x: semantic_number(call, "control2_x", control2_x)?,
            control2_y: semantic_number(call, "control2_y", control2_y)?,
            x: semantic_number(call, "x", x)?,
            y: semantic_number(call, "y", y)?,
        },
        ToolPathCommand::Close => PathCommand::Close,
    })
}

fn tool_vector_content(call: &ToolCall, value: ToolVectorContent) -> Result<VectorContent> {
    if value.paths.len() > MAX_VECTOR_PATHS {
        return Err(invalid_arguments(call, "vector path count is out of range"));
    }
    let mut total_commands = 0_usize;
    let mut paths = Vec::with_capacity(value.paths.len());
    for path in value.paths {
        if path.commands.len() > MAX_PATH_COMMANDS_PER_PATH {
            return Err(invalid_arguments(
                call,
                "per-path vector command count is out of range",
            ));
        }
        total_commands = total_commands
            .checked_add(path.commands.len())
            .ok_or_else(|| invalid_arguments(call, "vector command count overflow"))?;
        if total_commands > MAX_PATH_COMMANDS {
            return Err(invalid_arguments(
                call,
                "vector command count is out of range",
            ));
        }
        let stroke = path
            .stroke
            .map(|stroke| -> Result<StrokeStyle> {
                Ok(StrokeStyle {
                    color: stroke.color.into(),
                    width: semantic_number(call, "stroke width", stroke.width)?,
                })
            })
            .transpose()?;
        paths.push(VectorPath {
            commands: path
                .commands
                .into_iter()
                .map(|command| tool_path_command(call, command))
                .collect::<Result<Vec<_>>>()?,
            fill: path.fill.map(Pixel::from),
            stroke,
            fill_rule: path.fill_rule.into(),
        });
    }
    Ok(VectorContent { paths })
}

fn validate_tool_semantic(
    call: &ToolCall,
    context: &ProposalContext,
    content: &NodeContent,
) -> Result<()> {
    validate_semantic_content(content, context.width, context.height)
        .map_err(|error| invalid_arguments(call, &error.to_string()))
}

fn validate_projected_semantic(
    call: &ToolCall,
    context: &ProposalContext,
    removed: SemanticUsage,
    content: &NodeContent,
) -> Result<()> {
    let added =
        semantic_usage(content).map_err(|error| invalid_arguments(call, &error.to_string()))?;
    admit_semantic_replacement(context.semantic_usage, removed, added)
        .map(|_| ())
        .map_err(|error| invalid_arguments(call, &error.to_string()))
}

fn deterministic_node_id(call: &ToolCall, context: &ProposalContext) -> Result<LayerId> {
    // Fixed FNV-1a parameters make retries deterministic. A bounded linear
    // probe mirrors frame allocation and avoids collisions with immutable
    // document facts in fresh requests.
    const OFFSET: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
    const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013b;
    let mut hash = OFFSET;
    let namespace: &[u8] = match call.name.as_str() {
        "add_layer" => b"redrob-graphics:add-layer:",
        "add_group" => b"redrob-graphics:add-group:",
        "add_text_node" => b"redrob-graphics:add-text:",
        "add_vector_node" => b"redrob-graphics:add-vector:",
        _ => b"redrob-graphics:node:",
    };
    for byte in namespace.iter().chain(call.id.as_bytes()) {
        hash ^= u128::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    for probe in 0..=redrob_core::MAX_NODES {
        let mut bytes = hash.wrapping_add(probe as u128).to_be_bytes();
        bytes[6] = (bytes[6] & 0x0f) | 0x50;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        let encoded = format!(
            "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-\
             {:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            bytes[0],
            bytes[1],
            bytes[2],
            bytes[3],
            bytes[4],
            bytes[5],
            bytes[6],
            bytes[7],
            bytes[8],
            bytes[9],
            bytes[10],
            bytes[11],
            bytes[12],
            bytes[13],
            bytes[14],
            bytes[15]
        );
        let candidate: LayerId = serde_json::from_value(json!(encoded)).map_err(Error::Decode)?;
        if context.node(candidate).is_none() {
            return Ok(candidate);
        }
    }
    Err(invalid_arguments(
        call,
        "no deterministic node identifier is available",
    ))
}

fn deterministic_frame_id(call: &ToolCall, context: &ProposalContext) -> FrameId {
    const OFFSET: u32 = 0x811c_9dc5;
    const PRIME: u32 = 0x0100_0193;
    let mut hash = OFFSET;
    for byte in b"redrob-graphics:frame:"
        .iter()
        .chain(call.name.as_bytes())
        .chain(call.id.as_bytes())
    {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    for offset in 0..=u32::MAX {
        let candidate = FrameId::new(hash.wrapping_add(offset));
        if !context.contains_frame(candidate) {
            return candidate;
        }
    }
    unreachable!("validated timelines cannot occupy every u32 frame id")
}

fn command_action(title: &str, summary: String, command: Command) -> TypedToolAction {
    TypedToolAction {
        title: title.into(),
        summary,
        kind: TypedActionKind::Command(command),
    }
}

/// Decodes and validates a tool call exactly once for both proposal and direct
/// execution paths. No editor mutation occurs here.
fn typed_action_from_tool_call(
    call: &ToolCall,
    context: &ProposalContext,
) -> Result<TypedToolAction> {
    if call.id.trim().is_empty() {
        return Err(invalid_arguments(call, "tool call id must not be empty"));
    }
    if matches!(
        call.name.as_str(),
        "fill"
            | "brush_stroke"
            | "gradient_fill"
            | "flip_active"
            | "rotate_active_90"
            | "transform_active"
            | "apply_filter"
    ) {
        validate_active_raster(call, context)?;
    }
    match call.name.as_str() {
        "inspect_document" => {
            let _: EmptyArgs = decode(call)?;
            Ok(TypedToolAction {
                title: "Inspect document".into(),
                summary: "Inspect the current document without changing it.".into(),
                kind: TypedActionKind::Inspect,
            })
        }
        "add_frame" => {
            let args: AddFrameArgs = decode(call)?;
            if context.frames.len() >= MAX_FRAMES {
                return Err(invalid_arguments(call, "timeline is at the frame limit"));
            }
            let index = args.index.unwrap_or(context.frames.len());
            if index > context.frames.len() {
                return Err(invalid_arguments(call, "frame index is out of range"));
            }
            let id = deterministic_frame_id(call, context);
            Ok(command_action(
                "Add frame",
                format!(
                    "Add blank frame {} at timeline index {index}; current frame remains {}.",
                    id.get(),
                    context.current_frame.get()
                ),
                Command::AddFrame { id, index },
            ))
        }
        "duplicate_frame" => {
            let args: DuplicateFrameArgs = decode(call)?;
            if !context.contains_frame(args.source_id) {
                return Err(invalid_arguments(call, "source frame does not exist"));
            }
            if context.frames.len() >= MAX_FRAMES {
                return Err(invalid_arguments(call, "timeline is at the frame limit"));
            }
            let index = args.index.unwrap_or(context.frames.len());
            if index > context.frames.len() {
                return Err(invalid_arguments(call, "frame index is out of range"));
            }
            let id = deterministic_frame_id(call, context);
            let cel_count = context
                .nodes
                .iter()
                .filter(|node| node.cel_frames.contains(&args.source_id))
                .count();
            let source_duration = context.frames[context
                .frame_index(args.source_id)
                .expect("source validated")]
            .duration_ms;
            Ok(command_action(
                "Duplicate frame",
                format!(
                    "Duplicate {cel_count} sparse raster cels from frame {} ({} ms) into frame {} at index {index}.",
                    args.source_id.get(),
                    source_duration,
                    id.get()
                ),
                Command::DuplicateFrame {
                    source: args.source_id,
                    id,
                    index,
                },
            ))
        }
        "remove_frame" => {
            let args: FrameArgs = decode(call)?;
            if !context.contains_frame(args.id) {
                return Err(invalid_arguments(call, "frame does not exist"));
            }
            if context.frames.len() == 1 {
                return Err(invalid_arguments(call, "the last frame cannot be removed"));
            }
            Ok(command_action(
                "Remove frame",
                format!("Remove frame {} and its raster cels.", args.id.get()),
                Command::RemoveFrame { id: args.id },
            ))
        }
        "move_frame" => {
            let args: MoveFrameArgs = decode(call)?;
            let old_index = context
                .frame_index(args.id)
                .ok_or_else(|| invalid_arguments(call, "frame does not exist"))?;
            if args.new_index >= context.frames.len() {
                return Err(invalid_arguments(call, "frame index is out of range"));
            }
            let mut ordered = context
                .frames
                .iter()
                .map(|frame| frame.id)
                .collect::<Vec<_>>();
            let frame = ordered.remove(old_index);
            ordered.insert(args.new_index, frame);
            let range_start = ordered.iter().position(|id| *id == context.range_start);
            let range_end = ordered.iter().position(|id| *id == context.range_end);
            if range_start > range_end {
                return Err(invalid_arguments(
                    call,
                    "move would reverse the playback range",
                ));
            }
            Ok(command_action(
                "Move frame",
                format!("Move frame {} to index {}.", args.id.get(), args.new_index),
                Command::MoveFrame {
                    id: args.id,
                    new_index: args.new_index,
                },
            ))
        }
        "set_timeline_fps" => {
            let args: TimelineFpsArgs = decode(call)?;
            if !args.fps.is_finite() || args.fps <= 0.0 || args.fps > f64::from(MAX_TIMELINE_FPS) {
                return Err(invalid_arguments(call, "fps is out of range"));
            }
            Ok(command_action(
                "Set timeline FPS",
                format!(
                    "Set authoritative timeline FPS from {} to {}.",
                    context.fps, args.fps
                ),
                Command::SetTimelineFps {
                    fps: args.fps as f32,
                },
            ))
        }
        "set_playback_range" => {
            let args: PlaybackRangeArgs = decode(call)?;
            let start = context
                .frame_index(args.start)
                .ok_or_else(|| invalid_arguments(call, "range start frame does not exist"))?;
            let end = context
                .frame_index(args.end)
                .ok_or_else(|| invalid_arguments(call, "range end frame does not exist"))?;
            if start > end {
                return Err(invalid_arguments(call, "playback range is reversed"));
            }
            Ok(command_action(
                "Set playback range",
                format!(
                    "Set playback range to frames {} through {}.",
                    args.start.get(),
                    args.end.get()
                ),
                Command::SetPlaybackRange {
                    start: args.start,
                    end: args.end,
                },
            ))
        }
        "set_looping" => {
            let args: LoopingArgs = decode(call)?;
            Ok(command_action(
                if args.looping {
                    "Enable looping"
                } else {
                    "Disable looping"
                },
                format!(
                    "Set timeline looping from {} to {}.",
                    context.looping, args.looping
                ),
                Command::SetLooping {
                    looping: args.looping,
                },
            ))
        }
        "add_layer" => {
            let args: AddLayerArgs = decode(call)?;
            validate_name(call, &args.name)?;
            let index = args.index.unwrap_or(context.root_count());
            if index > context.root_count() {
                return Err(invalid_arguments(call, "root layer index is out of range"));
            }
            Ok(command_action(
                "Add layer",
                format!(
                    "Create and activate a transparent root layer named '{}' at index {index}.",
                    args.name
                ),
                Command::AddLayer {
                    id: deterministic_node_id(call, context)?,
                    name: args.name,
                    index,
                },
            ))
        }
        "add_group" => {
            let args: AddGroupArgs = decode(call)?;
            validate_name(call, &args.name)?;
            validate_group_parent(call, context, args.parent_id)?;
            if !context.validate_destination_depth(args.parent_id, 0) {
                return Err(invalid_arguments(
                    call,
                    "group destination exceeds maximum hierarchy depth",
                ));
            }
            let sibling_count = context.sibling_count(args.parent_id, None);
            let sibling_index = args.sibling_index.unwrap_or(sibling_count);
            if sibling_index > sibling_count {
                return Err(invalid_arguments(
                    call,
                    "group sibling index is out of range",
                ));
            }
            Ok(command_action(
                "Add group",
                format!(
                    "Create and activate group '{}' under {} at sibling index {sibling_index}.",
                    args.name,
                    args.parent_id
                        .map_or_else(|| "the document root".into(), |id| id.to_string())
                ),
                Command::AddGroup {
                    id: deterministic_node_id(call, context)?,
                    name: args.name,
                    parent: args.parent_id,
                    sibling_index,
                },
            ))
        }
        "add_text_node" => {
            let args: AddTextArgs = decode(call)?;
            validate_name(call, &args.name)?;
            validate_group_parent(call, context, args.parent_id)?;
            if !context.validate_destination_depth(args.parent_id, 0) {
                return Err(invalid_arguments(
                    call,
                    "text destination exceeds maximum hierarchy depth",
                ));
            }
            let sibling_count = context.sibling_count(args.parent_id, None);
            let sibling_index = args.sibling_index.unwrap_or(sibling_count);
            if sibling_index > sibling_count {
                return Err(invalid_arguments(
                    call,
                    "text sibling index is out of range",
                ));
            }
            let text = tool_text_content(call, args.text)?;
            let text_content = NodeContent::Text { text: text.clone() };
            validate_tool_semantic(call, context, &text_content)?;
            validate_projected_semantic(call, context, SemanticUsage::default(), &text_content)?;
            let character_count = text.text.chars().count();
            Ok(command_action(
                "Add text node",
                format!(
                    "Create text node '{}' with {character_count} printable/newline characters using embedded {EMBEDDED_FONT_ID}.",
                    args.name
                ),
                Command::AddTextNode {
                    id: deterministic_node_id(call, context)?,
                    name: args.name,
                    parent: args.parent_id,
                    sibling_index,
                    text,
                },
            ))
        }
        "set_text_content" => {
            let args: SetTextArgs = decode(call)?;
            if context.node(args.id).map(|node| node.kind) != Some(NodeKind::Text) {
                return Err(invalid_arguments(call, "id must identify a text node"));
            }
            let text = tool_text_content(call, args.text)?;
            let text_content = NodeContent::Text { text: text.clone() };
            validate_tool_semantic(call, context, &text_content)?;
            let removed = context
                .node(args.id)
                .expect("text node validated")
                .semantic_usage;
            validate_projected_semantic(call, context, removed, &text_content)?;
            let character_count = text.text.chars().count();
            Ok(command_action(
                "Edit text node",
                format!(
                    "Replace text node {} with {character_count} printable/newline characters.",
                    args.id
                ),
                Command::SetTextContent { id: args.id, text },
            ))
        }
        "add_vector_node" => {
            let args: AddVectorArgs = decode(call)?;
            validate_name(call, &args.name)?;
            validate_group_parent(call, context, args.parent_id)?;
            if !context.validate_destination_depth(args.parent_id, 0) {
                return Err(invalid_arguments(
                    call,
                    "vector destination exceeds maximum hierarchy depth",
                ));
            }
            let sibling_count = context.sibling_count(args.parent_id, None);
            let sibling_index = args.sibling_index.unwrap_or(sibling_count);
            if sibling_index > sibling_count {
                return Err(invalid_arguments(
                    call,
                    "vector sibling index is out of range",
                ));
            }
            let vector = tool_vector_content(call, args.vector)?;
            let vector_content = NodeContent::Vector {
                vector: vector.clone(),
            };
            validate_tool_semantic(call, context, &vector_content)?;
            validate_projected_semantic(call, context, SemanticUsage::default(), &vector_content)?;
            let command_count = vector
                .paths
                .iter()
                .map(|path| path.commands.len())
                .sum::<usize>();
            Ok(command_action(
                "Add vector node",
                format!(
                    "Create vector node '{}' with {} paths and {command_count} commands.",
                    args.name,
                    vector.paths.len()
                ),
                Command::AddVectorNode {
                    id: deterministic_node_id(call, context)?,
                    name: args.name,
                    parent: args.parent_id,
                    sibling_index,
                    vector,
                },
            ))
        }
        "set_vector_content" => {
            let args: SetVectorArgs = decode(call)?;
            if context.node(args.id).map(|node| node.kind) != Some(NodeKind::Vector) {
                return Err(invalid_arguments(call, "id must identify a vector node"));
            }
            let vector = tool_vector_content(call, args.vector)?;
            let vector_content = NodeContent::Vector {
                vector: vector.clone(),
            };
            validate_tool_semantic(call, context, &vector_content)?;
            let removed = context
                .node(args.id)
                .expect("vector node validated")
                .semantic_usage;
            validate_projected_semantic(call, context, removed, &vector_content)?;
            let command_count = vector
                .paths
                .iter()
                .map(|path| path.commands.len())
                .sum::<usize>();
            Ok(command_action(
                "Edit vector node",
                format!(
                    "Replace vector node {} with {} paths and {command_count} commands.",
                    args.id,
                    vector.paths.len()
                ),
                Command::SetVectorContent {
                    id: args.id,
                    vector,
                },
            ))
        }
        "rasterize_semantic_node" => {
            let args: NodeArgs = decode(call)?;
            let node = context
                .node(args.id)
                .ok_or_else(|| invalid_arguments(call, "node id does not exist"))?;
            if !matches!(node.kind, NodeKind::Text | NodeKind::Vector) {
                return Err(invalid_arguments(
                    call,
                    "id must identify a text or vector node",
                ));
            }
            Ok(command_action(
                "Rasterize semantic node",
                format!(
                    "Rasterize node {} on current frame {} only; all other frames will be blank and node opacity/blend remain editable.",
                    args.id,
                    context.current_frame.get()
                ),
                Command::RasterizeSemanticNode { id: args.id },
            ))
        }
        "move_node" => {
            let args: MoveNodeArgs = decode(call)?;
            let parent = if args.parent_id.is_null() {
                None
            } else {
                Some(
                    serde_json::from_value::<LayerId>(args.parent_id.clone())
                        .map_err(|_| invalid_arguments(call, "parent_id must be a UUID or null"))?,
                )
            };
            if context.node(args.id).is_none() {
                return Err(invalid_arguments(call, "node id does not exist"));
            }
            validate_group_parent(call, context, parent)?;
            let mut ancestor = parent;
            while let Some(id) = ancestor {
                if id == args.id {
                    return Err(invalid_arguments(
                        call,
                        "move would create a hierarchy cycle",
                    ));
                }
                ancestor = context.node(id).and_then(|entry| entry.parent);
            }
            let sibling_count = context.sibling_count(parent, Some(args.id));
            if args.sibling_index > sibling_count {
                return Err(invalid_arguments(
                    call,
                    "destination sibling index is out of range",
                ));
            }
            let subtree_height = context
                .subtree_height(args.id)
                .ok_or_else(|| invalid_arguments(call, "node subtree is invalid"))?;
            if !context.validate_destination_depth(parent, subtree_height) {
                return Err(invalid_arguments(
                    call,
                    "moved subtree exceeds maximum hierarchy depth",
                ));
            }
            Ok(command_action(
                "Move node",
                format!(
                    "Move node {} under {} at sibling index {}.",
                    args.id,
                    parent.map_or_else(|| "the document root".into(), |id| id.to_string()),
                    args.sibling_index
                ),
                Command::MoveNode {
                    id: args.id,
                    parent,
                    sibling_index: args.sibling_index,
                },
            ))
        }
        "add_raster_mask" => {
            let args: NodeArgs = decode(call)?;
            validate_mask_target(call, context, args.id, false)?;
            Ok(command_action(
                "Add raster mask",
                format!("Attach an enabled white raster mask to node {}.", args.id),
                Command::AddRasterMask { id: args.id },
            ))
        }
        "raster_mask_from_selection" => {
            let args: NodeArgs = decode(call)?;
            validate_mask_capable(call, context, args.id)?;
            if !context.selection_active {
                return Err(invalid_arguments(call, "selection must be active"));
            }
            Ok(command_action(
                "Mask from selection",
                format!(
                    "Copy active selection coverage into an enabled raster mask on node {}.",
                    args.id
                ),
                Command::RasterMaskFromSelection { id: args.id },
            ))
        }
        "remove_raster_mask" => {
            let args: NodeArgs = decode(call)?;
            validate_mask_target(call, context, args.id, true)?;
            Ok(command_action(
                "Remove raster mask",
                format!("Remove the raster mask from node {}.", args.id),
                Command::RemoveRasterMask { id: args.id },
            ))
        }
        "set_raster_mask_enabled" => {
            let args: ToggleMaskArgs = decode(call)?;
            let node = validate_mask_target(call, context, args.id, true)?;
            if node.mask_enabled == args.enabled {
                return Err(invalid_arguments(
                    call,
                    "raster mask already has the requested state",
                ));
            }
            Ok(command_action(
                if args.enabled {
                    "Enable raster mask"
                } else {
                    "Disable raster mask"
                },
                format!(
                    "Set node {} raster mask to {}.",
                    args.id,
                    if args.enabled { "enabled" } else { "disabled" }
                ),
                Command::SetRasterMaskEnabled {
                    id: args.id,
                    enabled: args.enabled,
                },
            ))
        }
        "replace_raster_mask" => {
            let args: ReplaceMaskArgs = decode(call)?;
            validate_mask_target(call, context, args.id, true)?;
            let rect = Rect::from(args.rect);
            let count = u64::from(rect.width) * u64::from(rect.height);
            let x1 = i64::from(rect.x) + i64::from(rect.width);
            let y1 = i64::from(rect.y) + i64::from(rect.height);
            if rect.x < 0
                || rect.y < 0
                || x1 > i64::from(context.width)
                || y1 > i64::from(context.height)
                || count == 0
                || count > MAX_MASK_COMMAND_PIXELS as u64
                || args.pixels.len() as u64 != count
            {
                return Err(invalid_arguments(
                    call,
                    "mask rectangle or payload is invalid",
                ));
            }
            Ok(command_action(
                "Replace raster mask",
                format!(
                    "Replace node {} mask coverage in {}.",
                    args.id,
                    rect_summary(rect)
                ),
                Command::ReplaceRasterMask {
                    id: args.id,
                    rect,
                    pixels: args.pixels,
                },
            ))
        }
        "rename_layer" => {
            let args: RenameLayerArgs = decode(call)?;
            validate_layer_id(call, context, args.id)?;
            validate_name(call, &args.name)?;
            Ok(command_action(
                "Rename layer",
                format!("Rename layer {} to '{}'.", args.id, args.name),
                Command::RenameLayer {
                    id: args.id,
                    name: args.name,
                },
            ))
        }
        "set_layer_opacity" => {
            let args: OpacityArgs = decode(call)?;
            validate_layer_id(call, context, args.id)?;
            validate_unit(call, "opacity", args.opacity)?;
            Ok(command_action(
                "Set layer opacity",
                format!(
                    "Set layer {} opacity to {:.0}%.",
                    args.id,
                    args.opacity * 100.0
                ),
                Command::SetLayerOpacity {
                    id: args.id,
                    opacity: args.opacity,
                },
            ))
        }
        "set_layer_visibility" => {
            let args: VisibilityArgs = decode(call)?;
            validate_layer_id(call, context, args.id)?;
            Ok(command_action(
                if args.visible {
                    "Show layer"
                } else {
                    "Hide layer"
                },
                format!(
                    "Set layer {} visibility to {}.",
                    args.id,
                    if args.visible { "visible" } else { "hidden" }
                ),
                Command::SetLayerVisibility {
                    id: args.id,
                    visible: args.visible,
                },
            ))
        }
        "fill" => {
            let args: FillArgs = decode(call)?;
            let color = Pixel::from(args.color);
            Ok(command_action(
                "Fill active layer",
                format!(
                    "Fill the active layer or selection with {}.",
                    color_summary(color)
                ),
                Command::Fill { color },
            ))
        }
        "brush_stroke" => {
            let args: StrokeArgs = decode(call)?;
            let settings = BrushSettings::from(args.settings);
            validate_brush(call, &args.points, args.size, args.opacity, settings)?;
            let size = args.size as f32;
            let point_count = args.points.len();
            let symmetry = match (settings.mirror_x, settings.mirror_y) {
                (Some(_), Some(_)) => " with horizontal and vertical mirror symmetry",
                (Some(_), None) => " with vertical-axis mirror symmetry",
                (None, Some(_)) => " with horizontal-axis mirror symmetry",
                (None, None) => "",
            };
            let smoothing = match settings.smoothing {
                BrushSmoothing::None => String::new(),
                BrushSmoothing::MovingAverage { window } => {
                    format!(" and moving-average smoothing window {window}")
                }
            };
            Ok(command_action(
                "Paint brush stroke",
                format!(
                    "Paint {point_count} pressure samples at size {} and {:.0}% opacity{smoothing}{symmetry}.",
                    size,
                    args.opacity * 100.0
                ),
                Command::BrushStroke {
                    points: args.points.into_iter().map(BrushPoint::from).collect(),
                    color: args.color.into(),
                    size,
                    opacity: args.opacity,
                    settings,
                },
            ))
        }
        "select_rectangle" | "select_ellipse" => {
            let args: SelectionShapeArgs = decode(call)?;
            let rect = Rect::from(args.rect);
            validate_rect(call, rect)?;
            let mode = SelectionMode::from(args.mode);
            let shape = if call.name == "select_rectangle" {
                "rectangle"
            } else {
                "ellipse"
            };
            let command = if call.name == "select_rectangle" {
                Command::SelectRectangle { rect, mode }
            } else {
                Command::SelectEllipse { rect, mode }
            };
            Ok(command_action(
                if shape == "rectangle" {
                    "Select rectangle"
                } else {
                    "Select ellipse"
                },
                format!(
                    "Use a {} {} to {} the current selection.",
                    rect_summary(rect),
                    shape,
                    selection_mode_summary(mode)
                ),
                command,
            ))
        }
        "select_all" => {
            let _: EmptyArgs = decode(call)?;
            Ok(command_action(
                "Select all",
                "Select every pixel in the canvas.".into(),
                Command::SelectAll,
            ))
        }
        "invert_selection" => {
            let _: EmptyArgs = decode(call)?;
            Ok(command_action(
                "Invert selection",
                "Invert selection coverage across the canvas.".into(),
                Command::InvertSelection,
            ))
        }
        "feather_selection" | "grow_selection" | "shrink_selection" => {
            let args: RadiusArgs = decode(call)?;
            validate_selection_radius(call, args.radius)?;
            let (title, verb, command) = match call.name.as_str() {
                "feather_selection" => (
                    "Feather selection",
                    "Soften",
                    Command::FeatherSelection {
                        radius: args.radius,
                    },
                ),
                "grow_selection" => (
                    "Grow selection",
                    "Expand",
                    Command::GrowSelection {
                        radius: args.radius,
                    },
                ),
                _ => (
                    "Shrink selection",
                    "Contract",
                    Command::ShrinkSelection {
                        radius: args.radius,
                    },
                ),
            };
            Ok(command_action(
                title,
                format!("{verb} the selection by {} pixels.", args.radius),
                command,
            ))
        }
        "clear_selection" => {
            let _: EmptyArgs = decode(call)?;
            Ok(command_action(
                "Clear selection",
                "Clear the selection so later edits affect the full canvas.".into(),
                Command::ClearSelection,
            ))
        }
        "gradient_fill" => {
            let args: GradientArgs = decode(call)?;
            let kind = GradientKind::from(args.kind);
            let stops = args
                .stops
                .into_iter()
                .map(GradientStop::from)
                .collect::<Vec<_>>();
            validate_gradient(call, kind, &stops)?;
            let geometry = match kind {
                GradientKind::Linear { .. } => "linear",
                GradientKind::Radial { .. } => "radial",
            };
            Ok(command_action(
                "Gradient fill",
                format!(
                    "Fill the active layer or selection with a {geometry} gradient containing {} stops.",
                    stops.len()
                ),
                Command::GradientFill { kind, stops },
            ))
        }
        "crop_canvas" => {
            let args: CropArgs = decode(call)?;
            let rect = Rect::from(args.rect);
            validate_rect(call, rect)?;
            Ok(command_action(
                "Crop canvas",
                format!("Crop or pad the canvas to {}.", rect_summary(rect)),
                Command::CropCanvas { rect },
            ))
        }
        "resize_canvas" => {
            let args: ResizeArgs = decode(call)?;
            validate_dimensions(call, args.width, args.height)?;
            let sampling = SamplingMode::from(args.sampling);
            Ok(command_action(
                "Resize canvas",
                format!(
                    "Resize the canvas to {}x{} using {} sampling.",
                    args.width,
                    args.height,
                    sampling_summary(sampling)
                ),
                Command::ResizeCanvas {
                    width: args.width,
                    height: args.height,
                    sampling,
                },
            ))
        }
        "flip_active" => {
            let args: FlipArgs = decode(call)?;
            if !args.horizontal && !args.vertical {
                return Err(invalid_arguments(
                    call,
                    "at least one flip direction must be enabled",
                ));
            }
            let direction = match (args.horizontal, args.vertical) {
                (true, true) => "horizontally and vertically",
                (true, false) => "horizontally",
                (false, true) => "vertically",
                (false, false) => unreachable!(),
            };
            Ok(command_action(
                "Flip active layer",
                format!("Flip the active layer {direction}."),
                Command::FlipActive {
                    horizontal: args.horizontal,
                    vertical: args.vertical,
                },
            ))
        }
        "rotate_active_90" => {
            let args: RotateArgs = decode(call)?;
            Ok(command_action(
                "Rotate active layer",
                format!(
                    "Rotate the active layer 90 degrees {}.",
                    if args.clockwise {
                        "clockwise"
                    } else {
                        "counterclockwise"
                    }
                ),
                Command::RotateActive90 {
                    clockwise: args.clockwise,
                },
            ))
        }
        "transform_active" => {
            let args: TransformArgs = decode(call)?;
            let transform = Affine2D::from(args.transform);
            validate_transform(call, transform)?;
            let sampling = SamplingMode::from(args.sampling);
            Ok(command_action(
                "Transform active layer",
                format!(
                    "Apply an affine transform to the active layer using {} sampling.",
                    sampling_summary(sampling)
                ),
                Command::TransformActive {
                    transform,
                    sampling,
                },
            ))
        }
        "apply_filter" => {
            let args: FilterArgs = decode(call)?;
            let filter = Filter::from(args.filter);
            validate_filter(call, &filter)?;
            Ok(command_action(
                "Apply filter",
                filter_summary(&filter),
                Command::ApplyFilter { filter },
            ))
        }
        "undo" => {
            let _: EmptyArgs = decode(call)?;
            if !context.can_undo {
                return Err(invalid_arguments(call, "there is no edit to undo"));
            }
            Ok(TypedToolAction {
                title: "Undo latest edit".into(),
                summary: "Undo the latest committed edit after approval.".into(),
                kind: TypedActionKind::Undo,
            })
        }
        "redo" => {
            let _: EmptyArgs = decode(call)?;
            if !context.can_redo {
                return Err(invalid_arguments(call, "there is no edit to redo"));
            }
            Ok(TypedToolAction {
                title: "Redo latest edit".into(),
                summary: "Redo the latest undone edit after approval.".into(),
                kind: TypedActionKind::Redo,
            })
        }
        _ => Err(Error::UnknownTool(call.name.clone())),
    }
}

fn is_semantic_proposal_command(command: &Command) -> bool {
    matches!(
        command,
        Command::AddTextNode { .. }
            | Command::SetTextContent { .. }
            | Command::AddVectorNode { .. }
            | Command::SetVectorContent { .. }
            | Command::RasterizeSemanticNode { .. }
    )
}

fn admit_semantic_proposal(
    call: &ToolCall,
    context: &ProposalContext,
    command: &Command,
) -> Result<()> {
    if !is_semantic_proposal_command(command) {
        return Ok(());
    }
    let snapshot = context
        .document
        .as_ref()
        .ok_or_else(|| invalid_arguments(call, "authoritative proposal snapshot is unavailable"))?;
    let mut editor = Editor::new(snapshot.clone())
        .map_err(|error| invalid_arguments(call, &error.to_string()))?;
    editor
        .execute(command.clone())
        .map(|_| ())
        .map_err(|error| invalid_arguments(call, &error.to_string()))
}

/// Converts a declared graphics call into an inert proposal.
///
/// `inspect_document` is validated but returns `None` because it is read-only.
/// This function never locks or mutates an editor.
pub fn proposal_from_tool_call(
    call: &ToolCall,
    context: &ProposalContext,
) -> Result<Option<ProposalDto>> {
    let typed = typed_action_from_tool_call(call, context)?;
    let action = match typed.kind {
        TypedActionKind::Inspect => return Ok(None),
        TypedActionKind::Command(command) => {
            admit_semantic_proposal(call, context, &command)?;
            ProposalAction::Command { command }
        }
        TypedActionKind::Undo => ProposalAction::Undo,
        TypedActionKind::Redo => ProposalAction::Redo,
    };
    Ok(Some(ProposalDto {
        id: call.id.clone(),
        title: typed.title,
        summary: typed.summary,
        base_generation: context.base_generation(),
        action,
    }))
}

fn semantic_summary(content: &NodeContent) -> Value {
    match content {
        NodeContent::Text { text } => json!({
            "kind": "text",
            "character_count": text.text.chars().count(),
            "byte_count": text.text.len(),
            "line_count": text.text.split('\n').count(),
            "font_id": text.font_id,
            "font_family": text.font_family,
            "font_size": text.font_size,
            "origin_x": text.origin_x,
            "origin_y": text.origin_y,
            "color": text.color
        }),
        NodeContent::Vector { vector } => json!({
            "kind": "vector",
            "path_count": vector.paths.len(),
            "command_count": vector.paths.iter().map(|path| path.commands.len()).sum::<usize>(),
            "filled_path_count": vector.paths.iter().filter(|path| path.fill.is_some()).count(),
            "stroked_path_count": vector.paths.iter().filter(|path| path.stroke.is_some()).count()
        }),
        _ => Value::Null,
    }
}

fn command_result_detail(operation: &str, command: &Command) -> Value {
    match command {
        Command::AddTextNode { id, text, .. } | Command::SetTextContent { id, text } => json!({
            "type": operation,
            "id": id,
            "character_count": text.text.chars().count()
        }),
        Command::AddVectorNode { id, vector, .. } | Command::SetVectorContent { id, vector } => {
            json!({
                "type": operation,
                "id": id,
                "path_count": vector.paths.len(),
                "command_count": vector.paths.iter().map(|path| path.commands.len()).sum::<usize>()
            })
        }
        Command::RasterizeSemanticNode { id } => json!({ "type": operation, "id": id }),
        _ => match serde_json::to_value(command) {
            Ok(value) => value,
            Err(_) => json!({ "type": operation }),
        },
    }
}

fn document_summary(editor: &Editor) -> Value {
    let document = editor.document();
    let range_start = document
        .timeline()
        .frame_index(document.timeline().playback().range_start)
        .unwrap_or(0);
    let range_end = document
        .timeline()
        .frame_index(document.timeline().playback().range_end)
        .unwrap_or(range_start);
    json!({
        "schema_version": 2,
        "operation": "inspect_document",
        "document_id": document.id(),
        "width": document.width(),
        "height": document.height(),
        "generation": editor.generation(),
        "can_undo": editor.can_undo(),
        "can_redo": editor.can_redo(),
        "active_layer_id": document.active_layer_id(),
        "active_node_id": document.active_layer_id(),
        "timeline": {
            "frames": document.timeline().frames().iter().enumerate().map(|(index, frame)| json!({
                "id": frame.id(),
                "index": index,
                "duration_ms": frame.duration_ms(),
                "current": frame.id() == document.timeline().current_frame(),
                "in_range": (range_start..=range_end).contains(&index)
            })).collect::<Vec<_>>(),
            "current_frame": document.timeline().current_frame(),
            "fps": document.timeline().fps(),
            "range_start": document.timeline().playback().range_start,
            "range_end": document.timeline().playback().range_end,
            "looping": document.timeline().playback().looping,
            "playing": document.timeline().playback().playing
        },
        "layers": document.layers().iter().enumerate().map(|(index, layer)| {
            let kind = layer.kind();
            json!({
                "index": index,
                "id": layer.id(),
                "name": layer.name(),
                "visible": layer.is_visible(),
                "opacity": layer.opacity(),
                "blend_mode": layer.blend_mode(),
                "kind": kind,
                "parent_id": layer.parent_id(),
                "depth": document.node_depth(layer.id()).unwrap_or(0),
                "has_mask": layer.mask().is_some(),
                "mask_enabled": layer.mask().is_some_and(|mask| mask.is_enabled()),
                "semantic": semantic_summary(layer.content()),
                "group": (kind == NodeKind::Group).then(|| json!({
                    "child_count": document.child_ids(Some(layer.id())).len()
                })),
                "capabilities": {
                    "can_have_children": kind == NodeKind::Group,
                    "can_have_mask": matches!(kind, NodeKind::Raster | NodeKind::Group),
                    "can_edit_raster": kind == NodeKind::Raster,
                    "can_edit_text": kind == NodeKind::Text,
                    "can_edit_vector": kind == NodeKind::Vector,
                    "can_rasterize": matches!(kind, NodeKind::Text | NodeKind::Vector),
                    "is_semantic": matches!(kind, NodeKind::Text | NodeKind::Vector),
                    "supports_pass_through": false
                }
            })
        }).collect::<Vec<_>>()
    })
}

fn mutation_summary(editor: &Editor, operation: &str, detail: Value) -> Value {
    json!({
        "operation": operation,
        "status": "applied",
        "generation": editor.generation(),
        "active_layer_id": editor.document().active_layer_id(),
        "can_undo": editor.can_undo(),
        "can_redo": editor.can_redo(),
        "detail": detail
    })
}

#[async_trait]
impl ToolExecutor for GraphicsToolExecutor {
    async fn execute(&self, call: &ToolCall) -> Result<ToolOutput> {
        let mut editor = self.lock();
        let context = ProposalContext::from_editor_for_direct_execution(&editor);
        let typed = typed_action_from_tool_call(call, &context)?;
        let summary = match typed.kind {
            TypedActionKind::Inspect => document_summary(&editor),
            TypedActionKind::Command(command) => {
                let detail = command_result_detail(&call.name, &command);
                editor
                    .execute(command)
                    .map_err(|error| Error::Tool(error.to_string()))?;
                mutation_summary(&editor, &call.name, detail)
            }
            TypedActionKind::Undo => {
                editor
                    .undo()
                    .map_err(|error| Error::Tool(error.to_string()))?;
                mutation_summary(&editor, &call.name, json!({}))
            }
            TypedActionKind::Redo => {
                editor
                    .redo()
                    .map_err(|error| Error::Tool(error.to_string()))?;
                mutation_summary(&editor, &call.name, json!({}))
            }
        };
        serde_json::to_string(&summary)
            .map(ToolOutput::new)
            .map_err(Error::Decode)
    }
}

#[cfg(test)]
mod tests {
    use redrob_agent::{Error, ToolCall, ToolExecutor};
    use redrob_core::{
        BrushSettings, BrushSmoothing, Command, Document, EMBEDDED_FONT_ID, Editor, Filter,
        FrameId, LayerId, MAX_BRUSH_PIXEL_VISITS, MAX_BRUSH_POINTS, MAX_BRUSH_SIZE,
        MAX_HIERARCHY_DEPTH, MAX_STORED_RASTER_BYTES, MAX_TEXT_BYTES, MAX_TEXT_CONTENT_BYTES,
        Pixel, TextContent,
    };
    use serde_json::{Value, json};

    use super::{
        GraphicsToolExecutor, ProposalAction, ProposalContext, graphics_tool_declarations,
        proposal_from_tool_call, proposal_tool_declarations,
    };

    fn call(name: &str, arguments: Value) -> ToolCall {
        ToolCall {
            id: format!("call-{name}"),
            name: name.into(),
            arguments,
        }
    }

    fn context() -> ProposalContext {
        let editor = Editor::new(Document::new(2, 2).unwrap()).unwrap();
        ProposalContext::from_editor(&editor)
    }

    fn brush_call(point_count: usize) -> ToolCall {
        brush_call_with_size(point_count, 1.0)
    }

    fn brush_call_with_size(point_count: usize, size: f64) -> ToolCall {
        call(
            "brush_stroke",
            json!({
                "points": vec![json!({ "x": 0.5, "y": 0.5, "pressure": 1.0 }); point_count],
                "color": { "r": 1, "g": 2, "b": 3, "a": 255 },
                "size": size,
                "opacity": 1.0
            }),
        )
    }

    #[test]
    fn declarations_cover_every_supported_tool_and_are_strict() {
        let tools = graphics_tool_declarations();
        let names = tools
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "inspect_document",
                "add_frame",
                "duplicate_frame",
                "remove_frame",
                "move_frame",
                "set_timeline_fps",
                "set_playback_range",
                "set_looping",
                "add_layer",
                "add_group",
                "add_text_node",
                "set_text_content",
                "add_vector_node",
                "set_vector_content",
                "rasterize_semantic_node",
                "move_node",
                "add_raster_mask",
                "raster_mask_from_selection",
                "remove_raster_mask",
                "set_raster_mask_enabled",
                "replace_raster_mask",
                "rename_layer",
                "set_layer_opacity",
                "set_layer_visibility",
                "fill",
                "brush_stroke",
                "select_rectangle",
                "select_ellipse",
                "select_all",
                "invert_selection",
                "feather_selection",
                "grow_selection",
                "shrink_selection",
                "clear_selection",
                "gradient_fill",
                "crop_canvas",
                "resize_canvas",
                "flip_active",
                "rotate_active_90",
                "transform_active",
                "apply_filter",
                "undo",
                "redo",
            ]
        );
        assert!(!tools[0].mutating);
        assert!(tools[1..].iter().all(|tool| tool.mutating));
        assert!(tools.iter().all(|tool| {
            tool.parameters["additionalProperties"] == json!(false)
                && !tool.name.contains("shell")
                && !tool.name.contains("file")
        }));
        let brush = tools
            .iter()
            .find(|tool| tool.name == "brush_stroke")
            .unwrap();
        assert_eq!(
            brush.parameters["properties"]["points"]["maxItems"],
            MAX_BRUSH_POINTS
        );
        assert_eq!(
            brush.parameters["properties"]["size"]["maximum"],
            MAX_BRUSH_SIZE
        );
        assert_eq!(
            brush.parameters["properties"]["settings"]["additionalProperties"],
            false
        );
        let replace_mask = tools
            .iter()
            .find(|tool| tool.name == "replace_raster_mask")
            .unwrap();
        assert_eq!(
            replace_mask.parameters["properties"]["pixels"]["maxItems"],
            super::MAX_MASK_COMMAND_PIXELS
        );
        let filters = tools
            .iter()
            .find(|tool| tool.name == "apply_filter")
            .unwrap();
        assert_eq!(
            filters.parameters["properties"]["filter"]["oneOf"]
                .as_array()
                .unwrap()
                .len(),
            10
        );
    }

    #[test]
    fn proposal_declarations_are_the_mutating_subset_without_inspection() {
        let all = graphics_tool_declarations();
        let proposal = proposal_tool_declarations();
        assert!(all.iter().any(|tool| tool.name == "inspect_document"));
        assert!(proposal.iter().all(|tool| tool.mutating));
        assert!(!proposal.iter().any(|tool| tool.name == "inspect_document"));
        assert_eq!(proposal.len() + 1, all.len());
        assert_eq!(
            proposal.iter().map(|tool| &tool.name).collect::<Vec<_>>(),
            all.iter()
                .filter(|tool| tool.mutating)
                .map(|tool| &tool.name)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn exact_advanced_proposal_json_is_stable() {
        let proposal = proposal_from_tool_call(
            &call(
                "brush_stroke",
                json!({
                    "points": [{ "x": 1.0, "y": 2.0, "pressure": 0.75 }],
                    "color": { "r": 5, "g": 6, "b": 7, "a": 255 },
                    "size": 3.0,
                    "opacity": 0.8,
                    "settings": {
                        "smoothing": { "kind": "moving_average", "window": 4 },
                        "mirror_x": 10.0,
                        "mirror_y": 12.0
                    }
                }),
            ),
            &context(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            serde_json::to_value(proposal).unwrap(),
            json!({
                "id": "call-brush_stroke",
                "title": "Paint brush stroke",
                "summary": "Paint 1 pressure samples at size 3 and 80% opacity and moving-average smoothing window 4 with horizontal and vertical mirror symmetry.",
                "base_generation": 0,
                "action": {
                    "type": "command",
                    "command": {
                        "type": "brush_stroke",
                        "points": [{ "x": 1.0, "y": 2.0, "pressure": 0.75 }],
                        "color": { "r": 5, "g": 6, "b": 7, "a": 255 },
                        "size": 3.0,
                        "opacity": 0.800000011920929,
                        "settings": {
                            "smoothing": { "kind": "moving_average", "window": 4 },
                            "mirror_x": 10.0,
                            "mirror_y": 12.0
                        }
                    }
                }
            })
        );
    }

    #[test]
    fn every_advanced_call_converts_to_the_exact_core_action() {
        let cases = [
            (
                call(
                    "select_rectangle",
                    json!({ "rect": { "x": 1, "y": 2, "width": 3, "height": 4 }, "mode": "add" }),
                ),
                json!({ "type": "select_rectangle", "rect": { "x": 1, "y": 2, "width": 3, "height": 4 }, "mode": "add" }),
            ),
            (
                call(
                    "select_ellipse",
                    json!({ "rect": { "x": -1, "y": 0, "width": 2, "height": 3 }, "mode": "intersect" }),
                ),
                json!({ "type": "select_ellipse", "rect": { "x": -1, "y": 0, "width": 2, "height": 3 }, "mode": "intersect" }),
            ),
            (
                call("select_all", json!({})),
                json!({ "type": "select_all" }),
            ),
            (
                call("invert_selection", json!({})),
                json!({ "type": "invert_selection" }),
            ),
            (
                call("feather_selection", json!({ "radius": 2 })),
                json!({ "type": "feather_selection", "radius": 2 }),
            ),
            (
                call("grow_selection", json!({ "radius": 3 })),
                json!({ "type": "grow_selection", "radius": 3 }),
            ),
            (
                call("shrink_selection", json!({ "radius": 1 })),
                json!({ "type": "shrink_selection", "radius": 1 }),
            ),
            (
                call("clear_selection", json!({})),
                json!({ "type": "clear_selection" }),
            ),
            (
                call(
                    "gradient_fill",
                    json!({
                        "kind": { "kind": "linear", "start_x": 0.0, "start_y": 0.0, "end_x": 4.0, "end_y": 0.0 },
                        "stops": [
                            { "position": 0.0, "color": { "r": 0, "g": 0, "b": 0, "a": 255 } },
                            { "position": 1.0, "color": { "r": 255, "g": 255, "b": 255, "a": 255 } }
                        ]
                    }),
                ),
                json!({
                    "type": "gradient_fill",
                    "kind": { "kind": "linear", "start_x": 0.0, "start_y": 0.0, "end_x": 4.0, "end_y": 0.0 },
                    "stops": [
                        { "position": 0.0, "color": { "r": 0, "g": 0, "b": 0, "a": 255 } },
                        { "position": 1.0, "color": { "r": 255, "g": 255, "b": 255, "a": 255 } }
                    ]
                }),
            ),
            (
                call(
                    "crop_canvas",
                    json!({ "rect": { "x": 1, "y": 1, "width": 4, "height": 5 } }),
                ),
                json!({ "type": "crop_canvas", "rect": { "x": 1, "y": 1, "width": 4, "height": 5 } }),
            ),
            (
                call(
                    "resize_canvas",
                    json!({ "width": 8, "height": 6, "sampling": "bilinear" }),
                ),
                json!({ "type": "resize_canvas", "width": 8, "height": 6, "sampling": "bilinear" }),
            ),
            (
                call(
                    "flip_active",
                    json!({ "horizontal": true, "vertical": false }),
                ),
                json!({ "type": "flip_active", "horizontal": true, "vertical": false }),
            ),
            (
                call("rotate_active_90", json!({ "clockwise": false })),
                json!({ "type": "rotate_active90", "clockwise": false }),
            ),
            (
                call(
                    "transform_active",
                    json!({
                        "transform": { "m11": 1.0, "m12": 0.0, "m21": 0.0, "m22": 1.0, "tx": 2.0, "ty": 3.0 },
                        "sampling": "nearest"
                    }),
                ),
                json!({
                    "type": "transform_active",
                    "transform": { "m11": 1.0, "m12": 0.0, "m21": 0.0, "m22": 1.0, "tx": 2.0, "ty": 3.0 },
                    "sampling": "nearest"
                }),
            ),
        ];
        for (call, expected_command) in cases {
            let proposal = proposal_from_tool_call(&call, &context()).unwrap().unwrap();
            let ProposalAction::Command { command } = proposal.action else {
                panic!("expected command for {}", call.name);
            };
            assert_eq!(
                serde_json::to_value(command).unwrap(),
                expected_command,
                "{}",
                call.name
            );
            assert!(!proposal.summary.is_empty());
        }
    }

    #[test]
    fn every_filter_converts_and_has_a_human_summary() {
        let filters = [
            json!({ "kind": "invert" }),
            json!({ "kind": "grayscale" }),
            json!({ "kind": "brightness_contrast", "brightness": 4, "contrast": -5.0 }),
            json!({ "kind": "gaussian_blur", "sigma": 1.5 }),
            json!({ "kind": "threshold", "threshold": 128 }),
            json!({ "kind": "posterize", "levels": 4 }),
            json!({ "kind": "levels", "input_black": 1, "input_white": 250, "gamma": 1.2, "output_black": 2, "output_white": 240 }),
            json!({ "kind": "hue_saturation", "hue_degrees": 30.0, "saturation": 20.0, "lightness": -10.0 }),
            json!({ "kind": "box_blur", "radius": 2 }),
            json!({ "kind": "sharpen", "amount": 1.5 }),
        ];
        for filter in filters {
            let expected: Filter = serde_json::from_value(filter.clone()).unwrap();
            let proposal = proposal_from_tool_call(
                &call("apply_filter", json!({ "filter": filter })),
                &context(),
            )
            .unwrap()
            .unwrap();
            assert!(!proposal.summary.is_empty());
            let ProposalAction::Command {
                command: Command::ApplyFilter { filter: decoded },
            } = proposal.action
            else {
                panic!("expected filter command");
            };
            assert_eq!(decoded, expected);
        }
    }

    #[test]
    fn legacy_brush_input_defaults_settings() {
        let proposal = proposal_from_tool_call(
            &call(
                "brush_stroke",
                json!({
                    "points": [{ "x": 0.5, "y": 0.5, "pressure": 1.0 }],
                    "color": { "r": 1, "g": 2, "b": 3, "a": 255 },
                    "size": 1.0,
                    "opacity": 1.0
                }),
            ),
            &context(),
        )
        .unwrap()
        .unwrap();
        assert!(matches!(
            proposal.action,
            ProposalAction::Command {
                command: Command::BrushStroke {
                    settings: BrushSettings {
                        smoothing: BrushSmoothing::None,
                        mirror_x: None,
                        mirror_y: None,
                    },
                    ..
                }
            }
        ));
    }

    #[test]
    fn brush_point_runtime_matches_schema_at_boundary() {
        assert!(
            proposal_from_tool_call(&brush_call(MAX_BRUSH_POINTS), &context())
                .unwrap()
                .is_some()
        );
        assert!(proposal_from_tool_call(&brush_call(MAX_BRUSH_POINTS + 1), &context()).is_err());
    }

    #[test]
    fn brush_size_runtime_matches_schema_at_boundary() {
        assert!(
            proposal_from_tool_call(
                &brush_call_with_size(1, f64::from(MAX_BRUSH_SIZE)),
                &context(),
            )
            .unwrap()
            .is_some()
        );
        assert!(
            proposal_from_tool_call(
                &brush_call_with_size(1, f64::from(MAX_BRUSH_SIZE) + 0.000_01),
                &context(),
            )
            .is_err()
        );
        assert!(
            proposal_from_tool_call(
                &brush_call_with_size(1, f64::from(MAX_BRUSH_SIZE) + 1.0),
                &context(),
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn direct_executor_runs_every_advanced_command_through_core() {
        let executor =
            GraphicsToolExecutor::new(Editor::new(Document::new(6, 6).unwrap()).unwrap());
        let calls = [
            call("select_all", json!({})),
            call(
                "select_rectangle",
                json!({ "rect": { "x": 1, "y": 1, "width": 4, "height": 4 }, "mode": "replace" }),
            ),
            call(
                "select_ellipse",
                json!({ "rect": { "x": 0, "y": 0, "width": 6, "height": 6 }, "mode": "add" }),
            ),
            call("invert_selection", json!({})),
            call("feather_selection", json!({ "radius": 1 })),
            call("grow_selection", json!({ "radius": 1 })),
            call("shrink_selection", json!({ "radius": 1 })),
            call("clear_selection", json!({})),
            call(
                "brush_stroke",
                json!({
                    "points": [{ "x": 1.5, "y": 1.5, "pressure": 1.0 }],
                    "color": { "r": 255, "g": 0, "b": 0, "a": 255 },
                    "size": 2.0,
                    "opacity": 1.0,
                    "settings": { "smoothing": { "kind": "moving_average", "window": 2 }, "mirror_x": 3.0 }
                }),
            ),
            call(
                "gradient_fill",
                json!({
                    "kind": { "kind": "radial", "center_x": 3.0, "center_y": 3.0, "radius": 3.0 },
                    "stops": [
                        { "position": 0.0, "color": { "r": 255, "g": 255, "b": 255, "a": 255 } },
                        { "position": 1.0, "color": { "r": 0, "g": 0, "b": 0, "a": 255 } }
                    ]
                }),
            ),
            call(
                "apply_filter",
                json!({ "filter": { "kind": "threshold", "threshold": 128 } }),
            ),
            call(
                "apply_filter",
                json!({ "filter": { "kind": "posterize", "levels": 4 } }),
            ),
            call(
                "apply_filter",
                json!({ "filter": { "kind": "levels", "input_black": 0, "input_white": 255, "gamma": 1.0, "output_black": 0, "output_white": 255 } }),
            ),
            call(
                "apply_filter",
                json!({ "filter": { "kind": "hue_saturation", "hue_degrees": 10.0, "saturation": 5.0, "lightness": -5.0 } }),
            ),
            call(
                "apply_filter",
                json!({ "filter": { "kind": "box_blur", "radius": 1 } }),
            ),
            call(
                "apply_filter",
                json!({ "filter": { "kind": "sharpen", "amount": 1.0 } }),
            ),
            call(
                "flip_active",
                json!({ "horizontal": true, "vertical": false }),
            ),
            call("rotate_active_90", json!({ "clockwise": true })),
            call(
                "transform_active",
                json!({
                    "transform": { "m11": 1.0, "m12": 0.0, "m21": 0.0, "m22": 1.0, "tx": 0.0, "ty": 0.0 },
                    "sampling": "bilinear"
                }),
            ),
            call(
                "crop_canvas",
                json!({ "rect": { "x": 1, "y": 1, "width": 4, "height": 4 } }),
            ),
            call(
                "resize_canvas",
                json!({ "width": 5, "height": 3, "sampling": "nearest" }),
            ),
        ];
        for (index, call) in calls.iter().enumerate() {
            let output = executor.execute(call).await.unwrap();
            let value: Value = serde_json::from_str(&output.content).unwrap();
            assert_eq!(value["operation"], call.name);
            assert_eq!(value["generation"], index + 1);
            assert_eq!(value["detail"]["type"], command_type_for_tool(&call.name));
        }
        let editor = executor.shared_editor();
        let guard = editor.lock().unwrap();
        assert_eq!(
            (guard.document().width(), guard.document().height()),
            (5, 3)
        );
        assert_eq!(guard.generation(), calls.len() as u64);
    }

    fn command_type_for_tool(tool: &str) -> Value {
        json!(match tool {
            "rotate_active_90" => "rotate_active90",
            _ => tool,
        })
    }

    #[tokio::test]
    async fn legacy_brush_defaults_also_execute_directly() {
        let executor =
            GraphicsToolExecutor::new(Editor::new(Document::new(2, 2).unwrap()).unwrap());
        let output = executor
            .execute(&call(
                "brush_stroke",
                json!({
                    "points": [{ "x": 0.5, "y": 0.5, "pressure": 1.0 }],
                    "color": { "r": 10, "g": 20, "b": 30, "a": 255 },
                    "size": 1.0,
                    "opacity": 1.0
                }),
            ))
            .await
            .unwrap();
        let value: Value = serde_json::from_str(&output.content).unwrap();
        assert_eq!(
            value["detail"]["settings"],
            json!({ "smoothing": { "kind": "none" }, "mirror_x": null, "mirror_y": null })
        );
        assert_eq!(value["generation"], 1);
    }

    #[test]
    fn malformed_null_range_and_relationship_inputs_are_rejected() {
        let malformed = [
            call("unknown", json!({})),
            call("inspect_document", json!({ "extra": true })),
            call("add_layer", json!({ "name": "   " })),
            call("add_layer", json!({ "name": "Ink", "index": 2 })),
            call("fill", Value::Null),
            call(
                "brush_stroke",
                json!({
                    "points": [],
                    "color": { "r": 0, "g": 0, "b": 0, "a": 255 },
                    "size": 1.0,
                    "opacity": 1.0
                }),
            ),
            call(
                "brush_stroke",
                json!({
                    "points": [{ "x": 0.0, "y": 0.0, "pressure": 1.0 }],
                    "color": { "r": 0, "g": 0, "b": 0, "a": 255 },
                    "size": 1.0,
                    "opacity": 1.0,
                    "settings": { "smoothing": { "kind": "moving_average", "window": 1 } }
                }),
            ),
            call(
                "brush_stroke",
                json!({
                    "points": [{ "x": 0.0, "y": 0.0, "pressure": 1.0 }],
                    "color": { "r": 0, "g": 0, "b": 0, "a": 255 },
                    "size": 1.0,
                    "opacity": 1.0,
                    "settings": { "mirror_x": null }
                }),
            ),
            call(
                "brush_stroke",
                json!({
                    "points": [{ "x": 0.0, "y": 0.0, "pressure": 1.0 }],
                    "color": { "r": 0, "g": 0, "b": 0, "a": 255 },
                    "size": 1.0,
                    "opacity": 1.0,
                    "settings": { "mirror_y": null }
                }),
            ),
            call(
                "select_rectangle",
                json!({ "rect": { "x": 0, "y": 0, "width": 0, "height": 1 }, "mode": "replace" }),
            ),
            call("feather_selection", json!({ "radius": 4097 })),
            call(
                "gradient_fill",
                json!({
                    "kind": { "kind": "linear", "start_x": 1.0, "start_y": 1.0, "end_x": 1.0, "end_y": 1.0 },
                    "stops": [
                        { "position": 0.8, "color": { "r": 0, "g": 0, "b": 0, "a": 255 } },
                        { "position": 0.2, "color": { "r": 255, "g": 255, "b": 255, "a": 255 } }
                    ]
                }),
            ),
            call(
                "resize_canvas",
                json!({ "width": 32768, "height": 32768, "sampling": "nearest" }),
            ),
            call(
                "flip_active",
                json!({ "horizontal": false, "vertical": false }),
            ),
            call(
                "transform_active",
                json!({
                    "transform": { "m11": 1.0, "m12": 2.0, "m21": 2.0, "m22": 4.0, "tx": 0.0, "ty": 0.0 },
                    "sampling": "nearest"
                }),
            ),
            call(
                "apply_filter",
                json!({ "filter": { "kind": "posterize", "levels": 1 } }),
            ),
            call(
                "apply_filter",
                json!({ "filter": { "kind": "levels", "input_black": 200, "input_white": 100, "gamma": 1.0, "output_black": 0, "output_white": 255 } }),
            ),
            call(
                "apply_filter",
                json!({ "filter": { "kind": "hue_saturation", "hue_degrees": 181.0, "saturation": 0.0, "lightness": 0.0 } }),
            ),
            call(
                "apply_filter",
                json!({ "filter": { "kind": "box_blur", "radius": 0 } }),
            ),
            call(
                "apply_filter",
                json!({ "filter": { "kind": "sharpen", "amount": 10.1 } }),
            ),
            call("undo", json!({ "unexpected": true })),
        ];
        for call in malformed {
            assert!(
                proposal_from_tool_call(&call, &context()).is_err(),
                "{} accepted {:?}",
                call.name,
                call.arguments
            );
        }
    }

    #[test]
    fn proposal_context_validates_layer_ids_and_history_capabilities() {
        let mut editor = Editor::new(Document::new(2, 2).unwrap()).unwrap();
        let existing = editor.document().active_layer_id();
        let missing = LayerId::new();
        let initial = ProposalContext::from_editor(&editor);
        assert!(
            proposal_from_tool_call(
                &call("rename_layer", json!({ "id": existing, "name": "Ink" })),
                &initial,
            )
            .is_ok()
        );
        assert!(
            proposal_from_tool_call(
                &call("rename_layer", json!({ "id": missing, "name": "Ink" })),
                &initial,
            )
            .is_err()
        );
        assert!(proposal_from_tool_call(&call("undo", json!({})), &initial).is_err());
        assert!(proposal_from_tool_call(&call("redo", json!({})), &initial).is_err());

        editor
            .execute(Command::Fill {
                color: Pixel::rgba(1, 2, 3, 255),
            })
            .unwrap();
        let after_edit = ProposalContext::from_editor(&editor);
        assert!(proposal_from_tool_call(&call("undo", json!({})), &after_edit).is_ok());
        editor.undo().unwrap();
        let after_undo = ProposalContext::from_editor(&editor);
        assert!(proposal_from_tool_call(&call("redo", json!({})), &after_undo).is_ok());
    }

    #[tokio::test]
    async fn direct_executor_validates_layer_ids_and_history_capabilities() {
        let executor =
            GraphicsToolExecutor::new(Editor::new(Document::new(2, 2).unwrap()).unwrap());
        let missing = LayerId::new();
        assert!(
            executor
                .execute(&call(
                    "rename_layer",
                    json!({ "id": missing, "name": "Ink" }),
                ))
                .await
                .is_err()
        );
        assert!(executor.execute(&call("undo", json!({}))).await.is_err());
        assert!(executor.execute(&call("redo", json!({}))).await.is_err());
        assert_eq!(executor.shared_editor().lock().unwrap().generation(), 0);

        executor
            .execute(&call(
                "fill",
                json!({ "color": { "r": 1, "g": 2, "b": 3, "a": 255 } }),
            ))
            .await
            .unwrap();
        executor.execute(&call("undo", json!({}))).await.unwrap();
        executor.execute(&call("redo", json!({}))).await.unwrap();
        assert_eq!(executor.shared_editor().lock().unwrap().generation(), 3);
    }

    #[tokio::test]
    async fn direct_executor_rejects_brush_work_amplification_transactionally() {
        const CANVAS_SIZE: u32 = 129;

        let executor = GraphicsToolExecutor::new(
            Editor::new(Document::new(CANVAS_SIZE, CANVAS_SIZE).unwrap()).unwrap(),
        );
        let shared = executor.shared_editor();
        let pixels = shared
            .lock()
            .unwrap()
            .render_snapshot()
            .unwrap()
            .pixels()
            .to_vec();
        let center = CANVAS_SIZE as f32 / 2.0;
        let amplified = call(
            "brush_stroke",
            json!({
                "points": vec![json!({ "x": center, "y": center, "pressure": 1.0 }); MAX_BRUSH_POINTS],
                "color": { "r": 255, "g": 0, "b": 0, "a": 255 },
                "size": MAX_BRUSH_SIZE,
                "opacity": 1.0
            }),
        );

        assert!(matches!(
            executor.execute(&amplified).await,
            Err(Error::Tool(message)) if message.contains(&MAX_BRUSH_PIXEL_VISITS.to_string())
        ));
        let editor = shared.lock().unwrap();
        assert_eq!(editor.generation(), 0);
        assert_eq!(editor.render_snapshot().unwrap().pixels(), pixels);
    }

    #[tokio::test]
    async fn direct_executor_uses_the_same_validation_path() {
        let executor =
            GraphicsToolExecutor::new(Editor::new(Document::new(2, 2).unwrap()).unwrap());
        let invalid = call(
            "apply_filter",
            json!({ "filter": { "kind": "levels", "input_black": 255, "input_white": 0, "gamma": 1.0, "output_black": 0, "output_white": 255 } }),
        );
        assert!(matches!(
            executor.execute(&invalid).await,
            Err(Error::Tool(_))
        ));
        let editor = executor.shared_editor();
        assert_eq!(editor.lock().unwrap().generation(), 0);
    }

    #[tokio::test]
    async fn mask_from_selection_tool_requires_active_selection_and_uses_typed_command() {
        let inactive = context();
        let raster = inactive.active_node;
        assert!(matches!(
            proposal_from_tool_call(
                &call("raster_mask_from_selection", json!({ "id": raster })),
                &inactive,
            ),
            Err(Error::Tool(message)) if message.contains("selection must be active")
        ));

        let mut editor = Editor::new(Document::new(2, 1).unwrap()).unwrap();
        let raster = editor.document().active_layer_id();
        editor
            .execute(Command::SelectRectangle {
                rect: redrob_core::Rect::new(0, 0, 1, 1),
                mode: redrob_core::SelectionMode::Replace,
            })
            .unwrap();
        let executor = GraphicsToolExecutor::new(editor);
        let tool_call = call("raster_mask_from_selection", json!({ "id": raster }));
        let context = {
            let shared = executor.shared_editor();
            let editor = shared.lock().unwrap();
            ProposalContext::from_editor(&editor)
        };
        let proposal = proposal_from_tool_call(&tool_call, &context)
            .unwrap()
            .unwrap();
        assert_eq!(
            proposal.action,
            ProposalAction::Command {
                command: Command::RasterMaskFromSelection { id: raster }
            }
        );
        executor.execute(&tool_call).await.unwrap();
        let shared = executor.shared_editor();
        let editor = shared.lock().unwrap();
        let mask = editor.document().layer(raster).unwrap().mask().unwrap();
        assert_eq!(mask.pixels(), [255, 0]);
        assert!(mask.is_enabled());
    }

    #[tokio::test]
    async fn hierarchy_and_mask_proposals_match_direct_execution_exactly() {
        let editor = Editor::new(Document::new(2, 2).unwrap()).unwrap();
        let raster = editor.document().active_layer_id();
        let executor = GraphicsToolExecutor::new(editor);

        let add_group = call(
            "add_group",
            json!({ "name": "Paint", "parent_id": null, "sibling_index": 1 }),
        );
        let context = {
            let shared = executor.shared_editor();
            let editor = shared.lock().unwrap();
            ProposalContext::from_editor(&editor)
        };
        let proposal = proposal_from_tool_call(&add_group, &context)
            .unwrap()
            .unwrap();
        let expected_command = match &proposal.action {
            ProposalAction::Command { command } => serde_json::to_value(command).unwrap(),
            _ => panic!("expected command proposal"),
        };
        let group = match &proposal.action {
            ProposalAction::Command {
                command: Command::AddGroup { id, .. },
            } => *id,
            _ => panic!("expected add-group command"),
        };
        let output: Value =
            serde_json::from_str(&executor.execute(&add_group).await.unwrap().content).unwrap();
        assert_eq!(output["detail"], expected_command);

        executor
            .execute(&call(
                "move_node",
                json!({ "id": raster, "parent_id": group, "sibling_index": 0 }),
            ))
            .await
            .unwrap();
        executor
            .execute(&call("add_raster_mask", json!({ "id": group })))
            .await
            .unwrap();
        executor
            .execute(&call(
                "replace_raster_mask",
                json!({
                    "id": group,
                    "rect": { "x": 0, "y": 0, "width": 2, "height": 2 },
                    "pixels": [0, 64, 128, 255]
                }),
            ))
            .await
            .unwrap();
        executor
            .execute(&call(
                "set_raster_mask_enabled",
                json!({ "id": group, "enabled": false }),
            ))
            .await
            .unwrap();
        let inspect: Value = serde_json::from_str(
            &executor
                .execute(&call("inspect_document", json!({})))
                .await
                .unwrap()
                .content,
        )
        .unwrap();
        assert_eq!(inspect["schema_version"], 2);
        let group_summary = inspect["layers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["id"] == json!(group))
            .unwrap();
        assert_eq!(group_summary["kind"], "group");
        assert_eq!(group_summary["has_mask"], true);
        assert_eq!(group_summary["mask_enabled"], false);
        assert_eq!(group_summary["group"]["child_count"], 1);

        assert!(
            proposal_from_tool_call(
                &call("move_node", json!({ "id": raster, "sibling_index": 0 })),
                &context,
            )
            .is_err()
        );
        assert!(
            executor
                .execute(&call(
                    "move_node",
                    json!({ "id": group, "parent_id": group, "sibling_index": 0 }),
                ))
                .await
                .is_err()
        );
        assert!(
            executor
                .execute(&call("add_raster_mask", json!({ "id": group })))
                .await
                .is_err()
        );
    }

    #[test]
    fn timeline_proposals_are_deterministic_strict_and_bounded() {
        let context = context();
        let add = call("add_frame", json!({ "index": 1 }));
        let first = proposal_from_tool_call(&add, &context).unwrap().unwrap();
        let second = proposal_from_tool_call(&add, &context).unwrap().unwrap();
        assert_eq!(first, second);
        assert!(matches!(
            first.action,
            ProposalAction::Command {
                command: Command::AddFrame { index: 1, .. }
            }
        ));
        assert!(
            proposal_from_tool_call(&call("add_frame", json!({ "extra": true })), &context)
                .is_err()
        );
        assert!(
            proposal_from_tool_call(&call("set_timeline_fps", json!({ "fps": 0 })), &context)
                .is_err()
        );
        assert!(
            proposal_from_tool_call(&call("remove_frame", json!({ "id": 0 })), &context).is_err()
        );
        assert!(
            proposal_from_tool_call(
                &call("set_timeline_fps", json!({ "fps": 240.0000001 })),
                &context,
            )
            .is_err()
        );
        assert!(
            proposal_from_tool_call(
                &call("set_playback_range", json!({ "start": 9, "end": 0 })),
                &context,
            )
            .is_err()
        );
    }

    #[test]
    fn every_timeline_tool_converts_and_rejects_relationship_boundaries() {
        let mut editor = Editor::new(Document::new(2, 2).unwrap()).unwrap();
        editor
            .execute(Command::AddFrame {
                id: FrameId::new(1),
                index: 1,
            })
            .unwrap();
        let context = ProposalContext::from_editor(&editor);
        for tool in [
            call("add_frame", json!({ "index": 2 })),
            call("duplicate_frame", json!({ "source_id": 0, "index": 2 })),
            call("remove_frame", json!({ "id": 1 })),
            call("move_frame", json!({ "id": 1, "new_index": 1 })),
            call("set_timeline_fps", json!({ "fps": 240.0 })),
            call("set_playback_range", json!({ "start": 0, "end": 1 })),
            call("set_looping", json!({ "looping": true })),
        ] {
            let proposal = proposal_from_tool_call(&tool, &context).unwrap().unwrap();
            assert!(matches!(proposal.action, ProposalAction::Command { .. }));
        }
        for tool in [
            call("duplicate_frame", json!({ "source_id": 99 })),
            call("move_frame", json!({ "id": 1, "new_index": 2 })),
            call("set_playback_range", json!({ "start": 1, "end": 0 })),
            call("set_looping", json!({ "looping": true, "extra": false })),
        ] {
            assert!(proposal_from_tool_call(&tool, &context).is_err());
        }
        let duplicate = call("duplicate_frame", json!({ "source_id": 0, "index": 2 }));
        assert_eq!(
            proposal_from_tool_call(&duplicate, &context).unwrap(),
            proposal_from_tool_call(&duplicate, &context).unwrap()
        );
    }

    #[tokio::test]
    async fn timeline_executor_and_inspection_expose_ordered_metadata() {
        let executor =
            GraphicsToolExecutor::new(Editor::new(Document::new(2, 2).unwrap()).unwrap());
        executor
            .execute(&call("add_frame", json!({ "index": 1 })))
            .await
            .unwrap();
        executor
            .execute(&call("set_timeline_fps", json!({ "fps": 24.0 })))
            .await
            .unwrap();
        executor
            .execute(&call("set_looping", json!({ "looping": true })))
            .await
            .unwrap();
        let inspect: Value = serde_json::from_str(
            &executor
                .execute(&call("inspect_document", json!({})))
                .await
                .unwrap()
                .content,
        )
        .unwrap();
        assert_eq!(inspect["timeline"]["frames"].as_array().unwrap().len(), 2);
        assert_eq!(inspect["timeline"]["frames"][0]["index"], 0);
        assert_eq!(inspect["timeline"]["frames"][0]["duration_ms"], 42);
        assert_eq!(inspect["timeline"]["fps"], 24.0);
        assert_eq!(inspect["timeline"]["looping"], true);
        assert_eq!(inspect["timeline"]["current_frame"], 0);
    }

    #[test]
    fn proposal_hierarchy_depth_accepts_exact_boundary_and_rejects_one_over() {
        let mut editor = Editor::new(Document::new(2, 2).unwrap()).unwrap();
        let leaf = editor.document().active_layer_id();
        let mut chain = Vec::new();
        let mut parent = None;
        for depth in 0..=MAX_HIERARCHY_DEPTH {
            let id = LayerId::new();
            editor
                .execute(Command::AddGroup {
                    id,
                    name: format!("Depth {depth}"),
                    parent,
                    sibling_index: usize::from(parent.is_none()),
                })
                .unwrap();
            chain.push(id);
            parent = Some(id);
        }
        let subtree = LayerId::new();
        editor
            .execute(Command::AddGroup {
                id: subtree,
                name: "Subtree".into(),
                parent: None,
                sibling_index: 2,
            })
            .unwrap();
        editor
            .execute(Command::AddGroup {
                id: LayerId::new(),
                name: "Subtree child".into(),
                parent: Some(subtree),
                sibling_index: 0,
            })
            .unwrap();
        let context = ProposalContext::from_editor(&editor);

        let add_at_limit = call(
            "add_group",
            json!({ "name": "At limit", "parent_id": chain[MAX_HIERARCHY_DEPTH - 1] }),
        );
        assert!(
            proposal_from_tool_call(&add_at_limit, &context)
                .unwrap()
                .is_some()
        );
        let add_over_limit = call(
            "add_group",
            json!({ "name": "Over limit", "parent_id": chain[MAX_HIERARCHY_DEPTH] }),
        );
        assert!(matches!(
            proposal_from_tool_call(&add_over_limit, &context),
            Err(Error::Tool(message)) if message.contains("maximum hierarchy depth")
        ));

        let move_leaf_at_limit = call(
            "move_node",
            json!({
                "id": leaf,
                "parent_id": chain[MAX_HIERARCHY_DEPTH - 1],
                "sibling_index": 1
            }),
        );
        assert!(
            proposal_from_tool_call(&move_leaf_at_limit, &context)
                .unwrap()
                .is_some()
        );

        let move_subtree_at_limit = call(
            "move_node",
            json!({
                "id": subtree,
                "parent_id": chain[MAX_HIERARCHY_DEPTH - 2],
                "sibling_index": 1
            }),
        );
        assert!(
            proposal_from_tool_call(&move_subtree_at_limit, &context)
                .unwrap()
                .is_some()
        );
        let move_subtree_over_limit = call(
            "move_node",
            json!({
                "id": subtree,
                "parent_id": chain[MAX_HIERARCHY_DEPTH - 1],
                "sibling_index": 1
            }),
        );
        assert!(matches!(
            proposal_from_tool_call(&move_subtree_over_limit, &context),
            Err(Error::Tool(message)) if message.contains("maximum hierarchy depth")
        ));
    }

    #[test]
    fn semantic_proposals_are_strict_deterministic_bounded_and_inert() {
        let context = context();
        let add = call(
            "add_text_node",
            json!({
                "name": "Caption",
                "text": {
                    "text": "Hello\nWorld",
                    "font_family": "metadata only",
                    "font_size": 8.0,
                    "color": { "r": 1, "g": 2, "b": 3, "a": 255 },
                    "origin_x": 0.0,
                    "origin_y": 0.0
                }
            }),
        );
        let first = proposal_from_tool_call(&add, &context).unwrap().unwrap();
        let second = proposal_from_tool_call(&add, &context).unwrap().unwrap();
        assert_eq!(first, second);
        assert!(first.summary.contains("11 printable/newline characters"));
        assert!(!first.summary.contains("Hello"));
        let ProposalAction::Command { command } = first.action else {
            panic!("command")
        };
        let Command::AddTextNode { text, .. } = command else {
            panic!("text")
        };
        assert_eq!(text.font_id, redrob_core::EMBEDDED_FONT_ID);
        assert_eq!(context.base_generation(), 0);

        let unknown = call(
            "add_text_node",
            json!({
                "name": "Bad",
                "extra": true,
                "text": {
                    "text": "A", "font_size": 8.0,
                    "color": { "r": 0, "g": 0, "b": 0, "a": 255 },
                    "origin_x": 0.0, "origin_y": 0.0
                }
            }),
        );
        assert!(proposal_from_tool_call(&unknown, &context).is_err());
        let glyph = call(
            "add_text_node",
            json!({
                "name": "Bad", "text": {
                    "text": "é", "font_size": 8.0,
                    "color": { "r": 0, "g": 0, "b": 0, "a": 255 },
                    "origin_x": 0.0, "origin_y": 0.0
                }
            }),
        );
        assert!(proposal_from_tool_call(&glyph, &context).is_err());
    }

    #[tokio::test]
    async fn semantic_executor_projects_counts_and_current_frame_rasterize_warning() {
        let executor =
            GraphicsToolExecutor::new(Editor::new(Document::new(16, 16).unwrap()).unwrap());
        let add = call(
            "add_vector_node",
            json!({
                "name": "Box",
                "vector": { "paths": [{
                    "commands": [
                        { "type": "move_to", "x": 1.0, "y": 1.0 },
                        { "type": "line_to", "x": 8.0, "y": 1.0 },
                        { "type": "line_to", "x": 8.0, "y": 8.0 },
                        { "type": "close" }
                    ],
                    "fill": { "r": 10, "g": 20, "b": 30, "a": 255 },
                    "fill_rule": "non_zero"
                }] }
            }),
        );
        let proposal =
            proposal_from_tool_call(&add, &ProposalContext::from_editor(&executor.lock()))
                .unwrap()
                .unwrap();
        let ProposalAction::Command { command } = proposal.action else {
            panic!("command")
        };
        let Command::AddVectorNode { id, .. } = command else {
            panic!("vector")
        };
        executor.execute(&add).await.unwrap();
        let inspect: Value = serde_json::from_str(
            &executor
                .execute(&call("inspect_document", json!({})))
                .await
                .unwrap()
                .content,
        )
        .unwrap();
        let vector = inspect["layers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["id"] == id.to_string())
            .unwrap();
        assert_eq!(vector["semantic"]["path_count"], 1);
        assert_eq!(vector["semantic"]["command_count"], 4);
        assert_eq!(vector["capabilities"]["can_rasterize"], true);

        let rasterize = ToolCall {
            id: "rasterize".into(),
            name: "rasterize_semantic_node".into(),
            arguments: json!({ "id": id }),
        };
        let context = ProposalContext::from_editor(&executor.lock());
        let warning = proposal_from_tool_call(&rasterize, &context)
            .unwrap()
            .unwrap();
        assert!(warning.summary.contains("current frame 0 only"));
        assert!(warning.summary.contains("all other frames will be blank"));
        executor.execute(&rasterize).await.unwrap();
        let inspect: Value = serde_json::from_str(
            &executor
                .execute(&call("inspect_document", json!({})))
                .await
                .unwrap()
                .content,
        )
        .unwrap();
        let raster = inspect["layers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["id"] == id.to_string())
            .unwrap();
        assert_eq!(raster["kind"], "raster");
    }

    #[test]
    fn repeated_semantic_call_ids_probe_existing_node_ids_deterministically() {
        let mut editor = Editor::new(Document::new(8, 8).unwrap()).unwrap();
        let add = call(
            "add_text_node",
            json!({
                "name": "Caption",
                "text": {
                    "text": "A", "font_size": 8.0,
                    "color": { "r": 1, "g": 2, "b": 3, "a": 255 },
                    "origin_x": 0.0, "origin_y": 0.0
                }
            }),
        );
        let first = proposal_from_tool_call(&add, &ProposalContext::from_editor(&editor))
            .unwrap()
            .unwrap();
        let ProposalAction::Command {
            command: first_command,
        } = first.action
        else {
            panic!("command")
        };
        let Command::AddTextNode { id: first_id, .. } = first_command.clone() else {
            panic!("text")
        };
        editor.execute(first_command).unwrap();

        let second = proposal_from_tool_call(&add, &ProposalContext::from_editor(&editor))
            .unwrap()
            .unwrap();
        let ProposalAction::Command {
            command: second_command,
        } = second.action
        else {
            panic!("command")
        };
        let Command::AddTextNode { id: second_id, .. } = second_command.clone() else {
            panic!("text")
        };
        assert_ne!(first_id, second_id);
        editor.execute(second_command).unwrap();
    }

    #[test]
    fn semantic_proposals_enforce_aggregate_add_and_set_replacement_arithmetic() {
        let mut editor = Editor::new(Document::new(2, 2).unwrap()).unwrap();
        let overhead = 1 + EMBEDDED_FONT_ID.len();
        let payload_total = MAX_TEXT_BYTES - 4 * overhead;
        let base = payload_total / 4;
        let remainder = payload_total % 4;
        let mut ids = Vec::new();
        let mut lengths = Vec::new();
        for index in 0..4 {
            let length = base + usize::from(index < remainder);
            assert!(length <= MAX_TEXT_CONTENT_BYTES);
            let id = LayerId::new();
            ids.push(id);
            lengths.push(length);
            editor
                .execute(Command::AddTextNode {
                    id,
                    name: format!("Text {index}"),
                    parent: None,
                    sibling_index: index + 1,
                    text: TextContent {
                        text: "x".repeat(length),
                        font_family: "f".into(),
                        font_size: 1.0,
                        color: Pixel::rgba(1, 2, 3, 255),
                        origin_x: 100.0,
                        origin_y: 100.0,
                        font_id: EMBEDDED_FONT_ID.into(),
                    },
                })
                .unwrap();
        }
        assert_eq!(
            editor.document().semantic_usage().unwrap().text_bytes,
            MAX_TEXT_BYTES
        );
        let context = ProposalContext::from_editor(&editor);
        let add_over = call(
            "add_text_node",
            json!({
                "name": "Over",
                "text": {
                    "text": "x", "font_family": "f", "font_size": 1.0,
                    "color": { "r": 1, "g": 2, "b": 3, "a": 255 },
                    "origin_x": 100.0, "origin_y": 100.0
                }
            }),
        );
        assert!(matches!(
            proposal_from_tool_call(&add_over, &context),
            Err(Error::Tool(message)) if message.contains("text complexity")
        ));

        let set_exact = call(
            "set_text_content",
            json!({
                "id": ids[0],
                "text": {
                    "text": "y".repeat(lengths[0]), "font_family": "f", "font_size": 1.0,
                    "color": { "r": 4, "g": 5, "b": 6, "a": 255 },
                    "origin_x": 100.0, "origin_y": 100.0
                }
            }),
        );
        assert!(
            proposal_from_tool_call(&set_exact, &context)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn proposal_filter_variants_are_exhaustive() {
        let filters = [
            Filter::Threshold { threshold: 1 },
            Filter::Posterize { levels: 2 },
            Filter::Levels {
                input_black: 0,
                input_white: 255,
                gamma: 1.0,
                output_black: 0,
                output_white: 255,
            },
            Filter::HueSaturation {
                hue_degrees: 0.0,
                saturation: 0.0,
                lightness: 0.0,
            },
            Filter::BoxBlur { radius: 1 },
            Filter::Sharpen { amount: 0.0 },
        ];
        assert_eq!(filters.len(), 6);
    }

    fn proposal_command(editor: &Editor, call: &ToolCall) -> Command {
        let proposal = proposal_from_tool_call(call, &ProposalContext::from_editor(editor))
            .unwrap()
            .unwrap();
        assert_eq!(proposal.base_generation, editor.generation());
        let ProposalAction::Command { command } = proposal.action else {
            panic!("expected command proposal")
        };
        command
    }

    fn text_arguments(content: &str) -> Value {
        json!({
            "text": content,
            "font_family": "metadata only",
            "font_size": 8.0,
            "color": { "r": 1, "g": 2, "b": 3, "a": 255 },
            "origin_x": 1_000_000.0,
            "origin_y": 1_000_000.0
        })
    }

    fn vector_arguments(offset: f32) -> Value {
        json!({ "paths": [{
            "commands": [
                { "type": "move_to", "x": offset, "y": 1_000_000.0 },
                { "type": "line_to", "x": offset + 1.0, "y": 1_000_000.0 }
            ],
            "stroke": { "color": { "r": 4, "g": 5, "b": 6, "a": 255 }, "width": 1.0 }
        }] })
    }

    #[test]
    fn every_semantic_proposal_applies_to_its_authoritative_snapshot() {
        let mut editor = Editor::new(Document::new(16, 16).unwrap()).unwrap();

        let add_text = call(
            "add_text_node",
            json!({ "name": "Text", "text": text_arguments("A") }),
        );
        let command = proposal_command(&editor, &add_text);
        let Command::AddTextNode { id: text_id, .. } = command.clone() else {
            panic!("add text command")
        };
        editor.execute(command).unwrap();

        let set_text = call(
            "set_text_content",
            json!({ "id": text_id, "text": text_arguments("B") }),
        );
        editor
            .execute(proposal_command(&editor, &set_text))
            .unwrap();

        let add_vector = call(
            "add_vector_node",
            json!({ "name": "Vector", "vector": vector_arguments(1_000_000.0) }),
        );
        let command = proposal_command(&editor, &add_vector);
        let Command::AddVectorNode { id: vector_id, .. } = command.clone() else {
            panic!("add vector command")
        };
        editor.execute(command).unwrap();

        let set_vector = call(
            "set_vector_content",
            json!({ "id": vector_id, "vector": vector_arguments(999_999.0) }),
        );
        editor
            .execute(proposal_command(&editor, &set_vector))
            .unwrap();

        let rasterize = call("rasterize_semantic_node", json!({ "id": vector_id }));
        editor
            .execute(proposal_command(&editor, &rasterize))
            .unwrap();
    }

    fn add_root_groups(editor: &mut Editor, count: usize) {
        for index in 0..count {
            editor
                .execute(Command::AddGroup {
                    id: LayerId::new(),
                    name: format!("Render group {index}"),
                    parent: None,
                    sibling_index: index + 1,
                })
                .unwrap();
        }
    }

    #[test]
    fn add_text_proposal_respects_exact_and_one_over_render_work_boundaries() {
        let add = call(
            "add_text_node",
            json!({ "name": "Off canvas", "text": text_arguments("A") }),
        );

        let mut exact = Editor::new(Document::new(1_024, 1_024).unwrap()).unwrap();
        add_root_groups(&mut exact, 126);
        let generation = exact.generation();
        let command = proposal_command(&exact, &add);
        assert_eq!(exact.generation(), generation);
        exact.execute(command).unwrap();

        let mut over = Editor::new(Document::new(1_024, 1_024).unwrap()).unwrap();
        add_root_groups(&mut over, 127);
        let before = over.document().clone();
        let generation = over.generation();
        assert!(matches!(
            proposal_from_tool_call(&add, &ProposalContext::from_editor(&over)),
            Err(Error::Tool(message)) if message.contains("aggregate work budget")
        ));
        assert_eq!(over.generation(), generation);
        assert_eq!(over.document(), &before);
    }

    fn raster_storage_editor(mask_count: usize) -> (Editor, LayerId) {
        let width = 4_096;
        let height = 2_048;
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
        for index in 1..mask_count {
            let group = LayerId::new();
            editor
                .execute(Command::AddGroup {
                    id: group,
                    name: format!("Storage group {index}"),
                    parent: None,
                    sibling_index: index,
                })
                .unwrap();
            editor
                .execute(Command::AddRasterMask { id: group })
                .unwrap();
        }
        let semantic_id = LayerId::new();
        editor
            .execute(Command::AddTextNode {
                id: semantic_id,
                name: "Storage semantic".into(),
                parent: None,
                sibling_index: mask_count,
                text: TextContent {
                    text: "A".into(),
                    font_family: "metadata only".into(),
                    font_size: 8.0,
                    color: Pixel::rgba(1, 2, 3, 255),
                    origin_x: 1_000_000.0,
                    origin_y: 1_000_000.0,
                    font_id: EMBEDDED_FONT_ID.into(),
                },
            })
            .unwrap();
        (editor, semantic_id)
    }

    #[test]
    fn rasterize_proposal_respects_exact_and_one_over_storage_boundaries() {
        let raster_bytes = 4_096_u64 * 2_048 * 4;
        {
            let (mut exact, semantic_id) = raster_storage_editor(3);
            assert_eq!(
                exact.document().stored_raster_bytes(),
                MAX_STORED_RASTER_BYTES - raster_bytes
            );
            let call = call("rasterize_semantic_node", json!({ "id": semantic_id }));
            let command = proposal_command(&exact, &call);
            exact.execute(command).unwrap();
            assert_eq!(
                exact.document().stored_raster_bytes(),
                MAX_STORED_RASTER_BYTES
            );
        }

        let (over, semantic_id) = raster_storage_editor(4);
        let call = call("rasterize_semantic_node", json!({ "id": semantic_id }));
        let before = over.document().clone();
        let generation = over.generation();
        assert!(matches!(
            proposal_from_tool_call(&call, &ProposalContext::from_editor(&over)),
            Err(Error::Tool(message)) if message.contains("stored raster bytes")
        ));
        assert_eq!(over.generation(), generation);
        assert_eq!(over.document(), &before);
    }
}
