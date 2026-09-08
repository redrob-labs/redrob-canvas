// SPDX-License-Identifier: GPL-3.0-or-later

use thiserror::Error;

/// Errors returned by the editor core.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("invalid canvas dimensions {width}x{height}")]
    InvalidDimensions { width: u32, height: u32 },
    #[error("raster buffer length {actual} does not match expected length {expected}")]
    InvalidBufferLength { expected: usize, actual: usize },
    #[error("layer {0} was not found")]
    LayerNotFound(crate::LayerId),
    #[error("layer index {index} is out of bounds for {len} layers")]
    LayerIndexOutOfBounds { index: usize, len: usize },
    #[error("sibling index {index} is out of bounds for {len} siblings")]
    SiblingIndexOutOfBounds { index: usize, len: usize },
    #[error("node identifier {0} already exists")]
    DuplicateNodeId(crate::NodeId),
    #[error("node {0} is not a group and cannot be used as a parent")]
    ParentIsNotGroup(crate::NodeId),
    #[error("moving node {node} under {parent} would create a hierarchy cycle")]
    HierarchyCycle {
        node: crate::NodeId,
        parent: crate::NodeId,
    },
    #[error("node hierarchy order is noncontiguous or ambiguous")]
    InvalidHierarchyOrder,
    #[error("group {0} is not empty")]
    NonEmptyGroup(crate::NodeId),
    #[error("node {0} already has a raster mask")]
    RasterMaskAlreadyExists(crate::NodeId),
    #[error("node {0} has no raster mask")]
    RasterMaskNotFound(crate::NodeId),
    #[error("mask replacement must be fully inside the canvas and within the payload limit")]
    InvalidMaskOperation,
    #[error("mask-from-selection requires an active selection")]
    SelectionNotActive,
    #[error("a document must contain at least one layer")]
    LastLayer,
    #[error("layer opacity must be finite and between 0 and 1")]
    InvalidOpacity,
    #[error("layer name must not be empty")]
    EmptyLayerName,
    #[error("frame identifier {0:?} does not exist")]
    FrameNotFound(crate::FrameId),
    #[error("frame identifier {0:?} already exists")]
    DuplicateFrameId(crate::FrameId),
    #[error("frame index {index} is out of bounds for {len} frames")]
    FrameIndexOutOfBounds { index: usize, len: usize },
    #[error("the last timeline frame cannot be removed")]
    LastFrame,
    #[error("timeline fps must be finite, greater than zero, and at most 240")]
    InvalidTimelineFps,
    #[error("playback range endpoints must exist and be in timeline order")]
    InvalidPlaybackRange,
    #[error("moving the frame would reverse the playback range")]
    PlaybackRangeOrder,
    #[error("node content {0:?} is not supported by this raster operation")]
    UnsupportedNodeContent(crate::NodeKind),
    #[error(
        "text contains unsupported glyph {0:?}; only printable ASCII and newline are supported"
    )]
    UnsupportedTextGlyph(char),
    #[error("semantic geometry is non-finite or outside the bounded coordinate range")]
    InvalidSemanticGeometry,
    #[error("semantic path grammar is invalid")]
    InvalidSemanticPath,
    #[error("semantic fill, stroke, or embedded-font style is invalid")]
    InvalidSemanticStyle,
    #[error("semantic rasterization exceeds its deterministic work budget")]
    SemanticWorkLimitExceeded,
    #[error("raster node {node} has no cel for frame {frame:?}")]
    MissingRasterCel {
        node: crate::NodeId,
        frame: crate::FrameId,
    },
    #[error("document exceeds the supported {0} limit")]
    DocumentLimitExceeded(&'static str),
    #[error("render exceeds the aggregate work budget of {max_pixel_visits} pixel visits")]
    RenderWorkLimitExceeded { max_pixel_visits: u64 },
    #[error("brush stroke has {actual} points; expected 1 to {max}")]
    InvalidBrushPointCount { actual: usize, max: usize },
    #[error(
        "brush size must be finite, greater than zero, and no larger than the supported maximum"
    )]
    InvalidBrushSize,
    #[error("brush pressure must be finite and between 0 and 1")]
    InvalidPressure,
    #[error("brush processing settings are invalid or exceed deterministic limits")]
    InvalidBrushSettings,
    #[error("brush stroke exceeds the raster work budget of {max_pixel_visits} pixel visits")]
    BrushWorkLimitExceeded { max_pixel_visits: u64 },
    #[error("selection radius {0} exceeds the supported maximum")]
    InvalidSelectionRadius(u32),
    #[error("gradient stops or geometry are invalid")]
    InvalidGradient,
    #[error("affine or raster transform is invalid")]
    InvalidTransform,
    #[error("filter parameter is outside its supported range")]
    InvalidFilterParameter,
    #[error("a command group is already active")]
    GroupAlreadyActive,
    #[error("no command group is active")]
    NoActiveGroup,
    #[error("history cannot be traversed while a command group is active")]
    GroupInProgress,
    #[error("there is no command to undo")]
    NothingToUndo,
    #[error("there is no command to redo")]
    NothingToRedo,
    #[error("unsupported project version {0}")]
    UnsupportedProjectVersion(u32),
    #[error("invalid project magic")]
    InvalidProjectMagic,
    #[error("malformed project: {0}")]
    MalformedProject(String),
    #[error("input is not a PNG image")]
    UnsupportedImageFormat,
    #[error(transparent)]
    Format(#[from] crate::FormatError),
    #[error("image codec error: {0}")]
    Image(#[from] image::ImageError),
    #[error("project serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, CoreError>;
