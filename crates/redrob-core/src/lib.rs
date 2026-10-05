// SPDX-License-Identifier: GPL-3.0-or-later

//! Deterministic, UI-independent raster graphics editor core.

pub mod abr;
mod anim;
mod assistants;
pub mod brush_tip;
mod channel;
mod codec;
pub mod color;
mod color_mode;
mod command;
mod curve_fit;
pub mod dab_shape;
mod dds;
mod dicom;
mod display_cms;
mod document;
mod editor;
mod error;
mod fattal;
mod filters;
mod fits;
pub mod flood_fill;
mod formats;
/// Curve and point geometry ported from Graphite.
///
/// `pub mod` rather than a private module with curated re-exports, which is how every
/// other module here is declared. The deviation is deliberate: this is a library of
/// algorithms rather than part of the document model, its surface is several dozen
/// functions across five files, and enumerating them in a `pub use` list would be
/// noise that drifts out of date. The curated surface exists to keep the document
/// model's invariants; geometry has none to protect.
pub mod geometry;
mod graph;
mod guides;
mod handle_transform;
pub mod icc;
mod icc_lut;
mod isobmff;
mod jxl;
mod kra;
mod kra_tiles;
pub mod layer_style;
mod mantiuk;
pub mod neighbourhood;
mod ora;
mod paint_select;
pub mod path;
pub mod pattern;
mod pdf;
mod postscript;
pub mod precision;
mod psd;
pub mod psp;
mod raster;
mod raw;
mod render;
pub mod scene;
mod scissors;
mod seamless_clone;
mod selection;
mod semantic;
mod serde_tag;
mod sgi;
pub mod spacing;
mod sunras;
mod svg;
pub mod telemetry;
mod text_caret;
pub mod tone_curve;
mod xbm;
mod xcf;
mod xpm;

pub use abr::{AbrError, MAX_ABR_BRUSHES, read_abr};
pub use assistants::BrushAssistant;
pub use brush_tip::{BrushTip, GbrError, MAX_BRUSH_TIP_EDGE, MAX_BRUSH_TIP_PIXELS};
pub use channel::{Channel, ChannelId, MAX_CHANNELS, QUICK_MASK_NAME};
pub use codec::{MAX_PROJECT_JSON_BYTES, export_png, import_png, load_project, save_project};
pub use color_mode::{
    ColorMode, DitherMode, INDEXED_ALPHA_THRESHOLD, MAX_PALETTE_COLORS, PaletteChoice,
    remap_indices_for_transparency, reserve_transparent_index,
};
pub use command::{
    Affine2D, AlienMapModel, BrushDynamic, BrushPoint, BrushSettings, BrushSmoothing,
    ColorComponent, Command, ConvolutionBorder, DeinterlaceField, DesaturateMode, DistanceMetric,
    DynamicSensor, Filter, FocusShape, FractionalPixels, GradientKind, GradientOutput,
    GradientStop, GrayMode, HalftoneColorModel, HistogramChannel, IllusionMode, LensSurroundings,
    LevelsSlot, MAX_BRUSH_DABS, MAX_BRUSH_PIXEL_VISITS, MAX_BRUSH_POINTS, MAX_BRUSH_SIZE,
    MAX_MASK_COMMAND_PIXELS, MazeAlgorithm, MyPaintSurface, OffsetType, PaperBackground,
    PropagateMode, SamplingMode, ShiftAxis, SinusBlend, SinusPerturbation, SizeDynamic, SizeSensor,
    SpiralType, TilingPrimitive, TrcType, VideoPattern, WarpMode, WindDirection, WindEdge,
    WindStyle,
};
pub use dab_shape::{DabMask, DabShape};
pub use display_cms::{ColorManagementMode, DisplaySettings, RenderingIntent};
pub use guides::{
    Guide, GuideId, GuideOrientation, GuideSettings, GuideStyle, SamplePoint, SamplePointId,
    snap_point, snap_x, snap_y,
};
pub use handle_transform::{HandleTransformClass, MAX_HANDLES};
pub use paint_select::{
    PAINT_SELECT_ADD_MASK_CUT, PAINT_SELECT_DEFAULT_STROKE_WIDTH, PAINT_SELECT_MAX_STROKE_WIDTH,
    PAINT_SELECT_MIN_STROKE_WIDTH, PAINT_SELECT_REMOVE_MASK_CUT,
};
pub use path::{MAX_PATHS, Path, PathId};
pub use pattern::{
    MAX_PATTERN_EDGE, MAX_PATTERN_NAME, MAX_PATTERN_PIXELS, PatError, Pattern, looks_like_pat,
};
pub use seamless_clone::{
    SEAMLESS_CLONE_DEFAULT_REFINE_SCALE, SEAMLESS_CLONE_MAX_REFINE_SCALE, samples_per_edge,
};
pub use text_caret::{CaretMovement, TextCaret, delete_at_caret, insert_at_caret, move_caret};

/// The wire tags of the filters that have a precision-native implementation (J.1b).
///
/// Public, and the single source the tests read, so "is this filter claimed native" and "does it
/// have an implementation" cannot be answered from two different lists. A copy of this list in a
/// test would keep passing after the real one changed.
pub fn precision_native_filter_tags() -> &'static [&'static str] {
    command::PRECISION_NATIVE_FILTERS
}

/// Every filter's wire tag, in enum order.
///
/// Public so the integration tests can assert the name lookup resolves each one to itself rather
/// than to the generic fallback — a failure mode that reads as a cosmetic message problem and is
/// actually the whole lookup being dead.
pub fn filter_wire_tags() -> &'static [&'static str] {
    command::FILTER_NAMES
}

/// The full parameter object a filter gets when only its `kind` is sent, or `None` when some
/// parameter has no default and a caller must supply it.
///
/// This is what the Qt bridge's minimal `{"kind": ...}` JSON actually applies, so a UI can show the
/// user the values before applying, and build a field per key, without a hand-kept table per
/// filter. Deserialise-then-serialise means the answer is the engine's own serde defaults.
pub fn filter_defaults(kind: &str) -> Option<serde_json::Value> {
    let filter: Filter = match serde_json::from_value(serde_json::json!({ "kind": kind })) {
        Ok(filter) => filter,
        Err(_) => {
            let mut object = required_filter_parameters(kind)?;
            object.insert("kind".into(), kind.into());
            serde_json::from_value(serde_json::Value::Object(object)).ok()?
        }
    };
    serde_json::to_value(filter).ok()
}

/// Starting values for the parameters a filter REQUIRES -- the ones its wire format gives no
/// default, so `{"kind": k}` alone is rejected. Upstream GEGL's declared defaults, converted to our
/// units where they differ (noted inline). These only seed the filter browser's form; the wire
/// format is unchanged, and a caller that omits them is still refused. Only the missing fields are
/// listed: everything else still comes from serde defaults.
fn required_filter_parameters(kind: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
    use serde_json::json;
    let white = json!({ "r": 255, "g": 255, "b": 255, "a": 255 });
    let black = json!({ "r": 0, "g": 0, "b": 0, "a": 255 });
    let value = match kind {
        "shadows_highlights" => json!({ "shadows": 0.0, "highlights": 0.0, "radius": 100.0 }),
        "color_exchange" => json!({ "from": white, "to": black }),
        "color_to_alpha" => json!({ "color": white }),
        "median_blur" => json!({ "radius": 3 }),
        "mean_curvature_blur" => json!({ "iterations": 20 }),
        "focus_blur" => json!({ "blur_radius": 25 }),
        "variable_blur" => json!({ "radius": 10 }),
        // Upstream's max_delta is 0.2 of the 0..1 range; ours is a u8 channel delta: 0.2 * 255.
        "selective_gaussian_blur" => json!({ "radius": 5, "max_delta": 51 }),
        "snn_mean" => json!({ "radius": 8 }),
        "difference_of_gaussians" => json!({ "radius1": 1.0, "radius2": 2.0 }),
        "edge_neon" => json!({ "radius": 5.0, "amount": 0.0 }),
        "engrave" => json!({ "height": 10 }),
        "illusion" => json!({ "divisions": 8 }),
        "mosaic" => json!({ "tile_size": 15 }),
        "tile_glass" => json!({ "tile_width": 25, "tile_height": 25 }),
        "tile_paper" => json!({ "tile_width": 155, "tile_height": 56 }),
        "wind" => json!({ "strength": 10 }),
        "spherize" => json!({ "curvature": 1.0 }),
        // Upstream's default transform string is the identity matrix.
        "recursive_transform" => {
            json!({ "transforms": [[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]] })
        }
        "shift" => json!({ "amount": 5 }),
        "apply_lens" => json!({ "refraction_index": 1.7 }),
        "high_pass" => json!({ "std_dev": 4.0, "contrast": 1.0 }),
        _ => return None,
    };
    match value {
        serde_json::Value::Object(map) => Some(map),
        _ => None,
    }
}
pub use document::{
    BlendMode, Document, DocumentImportBuilder, DocumentMetadata, EMBEDDED_FONT_ID, FillRule,
    Frame, FrameId, ImportMask, ImportNode, Layer, LayerId, MAX_FONT_FAMILY_BYTES,
    MAX_FONT_ID_BYTES, MAX_FRAME_DURATION_MS, MAX_FRAMES, MAX_HIERARCHY_DEPTH, MAX_METADATA_BYTES,
    MAX_METADATA_ENTRIES, MAX_NODE_NAME_BYTES, MAX_NODES, MAX_PATH_COMMANDS,
    MAX_PATH_COMMANDS_PER_PATH, MAX_SEMANTIC_MEMORY_BYTES, MAX_STORED_RASTER_BYTES, MAX_TEXT_BYTES,
    MAX_TEXT_CONTENT_BYTES, MAX_TIMELINE_FPS, MAX_VECTOR_PATHS, NodeContent, NodeId, NodeKind,
    PathCommand, Pixel, PlaybackMetadata, RasterCel, RasterMask, Rect, SemanticUsage, StrokeStyle,
    TextContent, Timeline, VectorContent, VectorPath, admit_semantic_replacement, semantic_usage,
    timeline_frame_duration_ms,
};
pub use editor::{ChangeSet, CommandBus, Editor, HistoryConfig, Navigation};
pub use error::{CoreError, Result};
pub use flood_fill::{FillMask, FloodFillOptions, colour_difference, flood_fill_mask};
pub use formats::{
    AlphaPolicy, EffectiveFormatMetadata, ExportOptions, ExportOutcome, FileFormat, FormatError,
    FormatWarning, ImportOptions, ImportOutcome, LossPolicy, MAX_FORMAT_INPUT_BYTES,
    MAX_FORMAT_OUTPUT_BYTES, TGA_FOOTER_SIGNATURE, detect_format, export_document, import_document,
};
pub use geometry::{
    MAX_SHAPE_SIDES, Shape, bezpath_to_vector_path, dvec2_to_point, point_to_dvec2,
    vector_path_to_bezpath,
};
pub use graph::{OpGraph, OpNode};
pub use layer_style::{Bevel, DropShadow, LayerStyle, OuterGlow};
pub use raster::RasterBytes;
pub use render::{MAX_RENDER_PIXEL_VISITS, RenderSnapshot, render_onion_skin};
pub use scissors::magnetic_boundary as scissors_magnetic_boundary;
pub use selection::{Selection, SelectionMode};
pub use semantic::{
    CUBIC_STEPS, FIXED_SCALE, MAX_SEMANTIC_COORDINATE, MAX_SEMANTIC_SAMPLE_EDGE_VISITS,
    MAX_SEMANTIC_SEGMENTS, MAX_TEXT_WORK, validate_semantic_content,
};
pub use spacing::{MIN_AXIS_PIXELS, MIN_SPACING, SpacingOptions, SpacingWalker};
pub use telemetry::{FilteredRollingMean, LatencyTracker, RollingMax, ScalarStats, ScalarTracker};
pub use tone_curve::{CurvePoint, MAX_CURVE_POINTS, ToneCurve, ToneCurveError};
