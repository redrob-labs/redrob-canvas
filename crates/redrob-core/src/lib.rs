// SPDX-License-Identifier: GPL-3.0-or-later

//! Deterministic, UI-independent raster graphics editor core.

pub mod abr;
mod assistants;
pub mod brush_tip;
pub mod color;
mod codec;
mod command;
mod dds;
pub mod dab_shape;
mod document;
mod editor;
mod error;
mod filters;
pub mod flood_fill;
mod formats;
pub mod icc;
mod isobmff;
mod jxl;
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
pub mod layer_style;
mod ora;
mod pdf;
mod psd;
mod kra;
mod kra_tiles;
mod xcf;
mod anim;
mod raster;
mod raw;
mod render;
mod scissors;
mod selection;
mod semantic;
pub mod spacing;
mod svg;
pub mod telemetry;
pub mod tone_curve;

pub use abr::{AbrError, MAX_ABR_BRUSHES, read_abr};
pub use brush_tip::{BrushTip, GbrError, MAX_BRUSH_TIP_EDGE, MAX_BRUSH_TIP_PIXELS};
pub use codec::{MAX_PROJECT_JSON_BYTES, export_png, import_png, load_project, save_project};
pub use command::{
    Affine2D, BrushPoint, BrushSettings, BrushSmoothing, Command, Filter, GradientKind,
    GradientStop, MAX_BRUSH_DABS, MAX_BRUSH_PIXEL_VISITS, MAX_BRUSH_POINTS, MAX_BRUSH_SIZE,
    MAX_MASK_COMMAND_PIXELS, MyPaintSurface, SamplingMode, SizeDynamic, SizeSensor, WarpMode,
};
pub use dab_shape::{DabMask, DabShape};
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
pub use scissors::magnetic_boundary as scissors_magnetic_boundary;
pub use assistants::BrushAssistant;
pub use graph::{OpGraph, OpNode};
pub use layer_style::{Bevel, DropShadow, LayerStyle, OuterGlow};
pub use formats::{
    AlphaPolicy, EffectiveFormatMetadata, ExportOptions, ExportOutcome, FileFormat, FormatError,
    FormatWarning, ImportOptions, ImportOutcome, LossPolicy, MAX_FORMAT_INPUT_BYTES,
    MAX_FORMAT_OUTPUT_BYTES, detect_format, export_document, import_document,
};
pub use geometry::{
    MAX_SHAPE_SIDES, Shape, bezpath_to_vector_path, dvec2_to_point, point_to_dvec2,
    vector_path_to_bezpath,
};
pub use raster::RasterBytes;
pub use render::{MAX_RENDER_PIXEL_VISITS, RenderSnapshot, render_onion_skin};
pub use selection::{Selection, SelectionMode};
pub use semantic::{
    CUBIC_STEPS, FIXED_SCALE, MAX_SEMANTIC_COORDINATE, MAX_SEMANTIC_SAMPLE_EDGE_VISITS,
    MAX_SEMANTIC_SEGMENTS, MAX_TEXT_WORK, validate_semantic_content,
};
pub use spacing::{MIN_AXIS_PIXELS, MIN_SPACING, SpacingOptions, SpacingWalker};
pub use telemetry::{FilteredRollingMean, LatencyTracker, RollingMax, ScalarStats, ScalarTracker};
pub use tone_curve::{CurvePoint, MAX_CURVE_POINTS, ToneCurve, ToneCurveError};
