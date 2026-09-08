// SPDX-License-Identifier: GPL-3.0-or-later

//! Deterministic, UI-independent raster graphics editor core.

mod codec;
mod command;
mod document;
mod editor;
mod error;
mod filters;
mod formats;
mod ora;
mod raster;
mod render;
mod selection;
mod semantic;
mod svg;

pub use codec::{MAX_PROJECT_JSON_BYTES, export_png, import_png, load_project, save_project};
pub use command::{
    Affine2D, BrushPoint, BrushSettings, BrushSmoothing, Command, Filter, GradientKind,
    GradientStop, MAX_BRUSH_DABS, MAX_BRUSH_PIXEL_VISITS, MAX_BRUSH_POINTS, MAX_BRUSH_SIZE,
    MAX_MASK_COMMAND_PIXELS, SamplingMode,
};
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
pub use formats::{
    AlphaPolicy, EffectiveFormatMetadata, ExportOptions, ExportOutcome, FileFormat, FormatError,
    FormatWarning, ImportOptions, ImportOutcome, LossPolicy, MAX_FORMAT_INPUT_BYTES,
    MAX_FORMAT_OUTPUT_BYTES, detect_format, export_document, import_document,
};
pub use raster::RasterBytes;
pub use render::{MAX_RENDER_PIXEL_VISITS, RenderSnapshot};
pub use selection::{Selection, SelectionMode};
pub use semantic::{
    CUBIC_STEPS, FIXED_SCALE, MAX_SEMANTIC_COORDINATE, MAX_SEMANTIC_SAMPLE_EDGE_VISITS,
    MAX_SEMANTIC_SEGMENTS, MAX_TEXT_WORK, validate_semantic_content,
};
