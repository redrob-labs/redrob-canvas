// SPDX-License-Identifier: GPL-3.0-or-later

use serde::{Deserialize, Serialize};

use crate::{
    BlendMode, DocumentMetadata, FrameId, LayerId, NodeId, Pixel, Rect, SelectionMode, Shape,
    TextContent, VectorContent, VectorPath,
};

/// Maximum number of mask samples accepted by one replacement command.
///
/// Mask edits use bounded rectangular payloads rather than whole-document
/// buffers so command decoding, cloning, history, and agent review remain
/// predictably bounded even on the largest supported canvas.
pub const MAX_MASK_COMMAND_PIXELS: usize = 256 * 1024;

/// Maximum pressure samples accepted in one brush command.
///
/// Four thousand ninety-six points supports detailed strokes while bounding
/// smoothing, mirroring, interpolation, and validation work.
pub const MAX_BRUSH_POINTS: usize = 4_096;

/// Maximum brush diameter, in canvas pixels.
///
/// The 1,000-pixel ceiling follows the pinned Krita painting baseline and is
/// lower than the generic 4,096-pixel fallback used by other bounded radii.
pub const MAX_BRUSH_SIZE: f32 = 1_000.0;

/// Absolute maximum number of interpolated and mirrored dabs in one stroke.
pub const MAX_BRUSH_DABS: usize = 4_000_000;

/// Maximum sum of canvas-clipped per-dab raster rectangle areas.
///
/// This permits one complete visit of the largest supported canvas while
/// preventing many individually valid dabs from amplifying raster work.
pub const MAX_BRUSH_PIXEL_VISITS: u64 = 64 * 1024 * 1024;

/// A pressure sample in canvas coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrushPoint {
    pub x: f32,
    pub y: f32,
    pub pressure: f32,
}

impl BrushPoint {
    pub const fn new(x: f32, y: f32, pressure: f32) -> Self {
        Self { x, y, pressure }
    }
}

/// Deterministic preprocessing applied to brush samples.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BrushSmoothing {
    #[default]
    None,
    MovingAverage {
        window: u8,
    },
}

/// Optional brush point processing and symmetry settings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BrushSettings {
    #[serde(default)]
    pub smoothing: BrushSmoothing,
    /// Mirrors the stroke across the vertical line at this x coordinate.
    #[serde(default)]
    pub mirror_x: Option<f32>,
    /// Mirrors the stroke across the horizontal line at this y coordinate.
    #[serde(default)]
    pub mirror_y: Option<f32>,
    /// How far apart dabs are placed along the stroke.
    ///
    /// Defaults to a quarter of the dab's size, which is what this product used when the spacing was
    /// hard-coded, so a document saved before this field existed reopens spaced as it was drawn.
    #[serde(default, skip_serializing_if = "crate::spacing::is_default_spacing")]
    pub spacing: crate::SpacingOptions,
    /// The dab's shape: hardness, softness and aspect.
    ///
    /// `default` with a default that reproduces the previous fixed round dab, so a document saved before
    /// this field existed reopens drawn the way it was drawn.
    ///
    /// Omitted from the output when it IS the default, which keeps every existing serialised command and
    /// agent proposal byte-identical. Two FFI tests assert exact proposal JSON and caught this field
    /// appearing in all of them -- noise about a shape the tool surface cannot set yet.
    #[serde(default, skip_serializing_if = "crate::dab_shape::is_default_shape")]
    pub shape: crate::DabShape,
    /// Removes paint instead of adding it: each dab lowers the layer's alpha by its own coverage
    /// (destination-out), as Krita's eraser mode does with the same brush. Colour is ignored.
    ///
    /// Omitted when false, so every existing serialised stroke and proposal stays byte-identical.
    #[serde(default, skip_serializing_if = "is_false")]
    pub erase: bool,
    /// Airbrush flow: a per-dab alpha multiplier below 1 so repeated dabs over one spot build up
    /// gradually toward the brush opacity, the way GIMP's airbrush deposits paint while held. `None`
    /// (the default) means a normal brush — each dab paints at full strength.
    ///
    /// Omitted when absent, so every existing serialised stroke stays byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<f32>,
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde's skip_serializing_if passes a reference
fn is_false(value: &bool) -> bool {
    !*value
}

/// One color stop in a gradient. Positions are in the inclusive range 0..=1.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradientStop {
    pub position: f32,
    pub color: Pixel,
}

impl GradientStop {
    pub const fn new(position: f32, color: Pixel) -> Self {
        Self { position, color }
    }
}

/// Gradient geometry in canvas coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GradientKind {
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

/// Pixel reconstruction used by raster transforms.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SamplingMode {
    #[default]
    Nearest,
    Bilinear,
}

/// A forward 2D affine transform.
///
/// The transformed point is `(m11*x + m12*y + tx, m21*x + m22*y + ty)`.
/// Rasterization inverse-maps destination pixel centers into the source.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Affine2D {
    pub m11: f32,
    pub m12: f32,
    pub m21: f32,
    pub m22: f32,
    pub tx: f32,
    pub ty: f32,
}

impl Affine2D {
    pub const IDENTITY: Self = Self {
        m11: 1.0,
        m12: 0.0,
        m21: 0.0,
        m22: 1.0,
        tx: 0.0,
        ty: 0.0,
    };

    pub const fn new(m11: f32, m12: f32, m21: f32, m22: f32, tx: f32, ty: f32) -> Self {
        Self {
            m11,
            m12,
            m21,
            m22,
            tx,
            ty,
        }
    }
}

/// A destructive filter operation applied to the active layer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Filter {
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
    /// An arbitrary transfer curve through user-placed control points.
    ///
    /// `Levels` above expresses a black point, a white point and a gamma, which cannot describe a curve
    /// that rises and falls. This can. The points are the curve's definition rather than a sampled
    /// table, so a document stays editable and re-samples at whatever precision it renders at.
    Curves {
        points: Vec<crate::CurvePoint>,
    },
}

/// Serializable mutations accepted by [`crate::Editor`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    SetMetadata {
        metadata: DocumentMetadata,
    },
    AddFrame {
        id: FrameId,
        index: usize,
    },
    DuplicateFrame {
        source: FrameId,
        id: FrameId,
        index: usize,
    },
    RemoveFrame {
        id: FrameId,
    },
    MoveFrame {
        id: FrameId,
        new_index: usize,
    },
    SetTimelineFps {
        fps: f32,
    },
    SetPlaybackRange {
        start: FrameId,
        end: FrameId,
    },
    SetLooping {
        looping: bool,
    },
    AddLayer {
        id: LayerId,
        name: String,
        index: usize,
    },
    /// Adds a group at an explicit sibling position. `parent` must be a group.
    AddGroup {
        id: NodeId,
        name: String,
        #[serde(default)]
        parent: Option<NodeId>,
        sibling_index: usize,
    },
    AddTextNode {
        id: NodeId,
        name: String,
        #[serde(default)]
        parent: Option<NodeId>,
        sibling_index: usize,
        text: TextContent,
    },
    SetTextContent {
        id: NodeId,
        text: TextContent,
    },
    AddVectorNode {
        id: NodeId,
        name: String,
        #[serde(default)]
        parent: Option<NodeId>,
        sibling_index: usize,
        vector: VectorContent,
    },
    SetVectorContent {
        id: NodeId,
        vector: VectorContent,
    },
    /// Adds a vector node whose single path is a constructed shape.
    ///
    /// Distinct from `AddVectorNode` with a hand-built path on purpose: the SHAPE is what is
    /// recorded, so the undo history and the project file keep the intent -- an ellipse of
    /// this size -- rather than the twenty-odd coordinates it expanded into.
    AddShapeNode {
        id: NodeId,
        name: String,
        #[serde(default)]
        parent: Option<NodeId>,
        sibling_index: usize,
        shape: Shape,
        /// Fill, stroke and fill rule for the constructed path. Its `commands` are ignored:
        /// the shape supplies them.
        #[serde(default)]
        paint: VectorPath,
    },
    RasterizeSemanticNode {
        id: NodeId,
    },
    /// Moves a node to an explicit parent and sibling position.
    MoveNode {
        id: NodeId,
        #[serde(default)]
        parent: Option<NodeId>,
        sibling_index: usize,
    },
    AddRasterMask {
        id: NodeId,
    },
    RemoveRasterMask {
        id: NodeId,
    },
    SetRasterMaskEnabled {
        id: NodeId,
        enabled: bool,
    },
    /// Copies the active selection's complete coverage into a target mask.
    /// If no mask exists, one is attached; an existing mask is overwritten.
    /// The resulting mask is always enabled. An inactive selection is rejected.
    RasterMaskFromSelection {
        id: NodeId,
    },
    /// Replaces one fully in-bounds mask rectangle with row-major coverage.
    ReplaceRasterMask {
        id: NodeId,
        rect: Rect,
        pixels: Vec<u8>,
    },
    RemoveLayer {
        id: LayerId,
    },
    SetActiveLayer {
        id: LayerId,
    },
    RenameLayer {
        id: LayerId,
        name: String,
    },
    SetLayerVisibility {
        id: LayerId,
        visible: bool,
    },
    SetLayerOpacity {
        id: LayerId,
        opacity: f32,
    },
    SetLayerBlendMode {
        id: LayerId,
        mode: BlendMode,
    },
    ReorderLayer {
        id: LayerId,
        new_index: usize,
    },
    SelectRectangle {
        rect: Rect,
        mode: SelectionMode,
    },
    SelectEllipse {
        rect: Rect,
        mode: SelectionMode,
    },
    SelectAll,
    InvertSelection,
    FeatherSelection {
        radius: u32,
    },
    GrowSelection {
        radius: u32,
    },
    ShrinkSelection {
        radius: u32,
    },
    ClearSelection,
    BrushStroke {
        points: Vec<BrushPoint>,
        color: Pixel,
        size: f32,
        opacity: f32,
        #[serde(default)]
        settings: BrushSettings,
        /// An image tip, which replaces the generated shape when present.
        ///
        /// On the command rather than inside `BrushSettings` because settings are small copyable
        /// configuration and a tip is bulk data -- a tip in there would cost `BrushSettings` its `Copy`,
        /// and every call site would clone config to carry pixels.
        ///
        /// Carried BY VALUE and bounded, the way vector paths and gradient stops already are here, because
        /// this product has no resource store to reference one from. A 64-square tip is 4 KB and the
        /// decoder caps a tip at 512 square. A resource store would be the better home; that is
        /// architecture rather than translation, so it is not invented here.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tip: Option<crate::BrushTip>,
    },
    GradientFill {
        kind: GradientKind,
        stops: Vec<GradientStop>,
    },
    Fill {
        color: Pixel,
    },
    /// Fills the contiguous region of similar colour around a seed point.
    ///
    /// `Fill` above paints the whole layer or the whole selection; this is the bucket tool. Its tolerance
    /// is a **Lab** distance, matching Krita, so the same numeric setting includes the same pixels there
    /// as here -- which is what the golden-output harness will compare.
    FloodFill {
        x: u32,
        y: u32,
        color: Pixel,
        #[serde(default)]
        options: crate::FloodFillOptions,
    },
    Clear,
    ApplyFilter {
        filter: Filter,
    },
    CropCanvas {
        rect: Rect,
    },
    ResizeCanvas {
        width: u32,
        height: u32,
        sampling: SamplingMode,
    },
    FlipActive {
        horizontal: bool,
        vertical: bool,
    },
    RotateActive90 {
        clockwise: bool,
    },
    TransformActive {
        transform: Affine2D,
        sampling: SamplingMode,
    },
}

impl Command {
    /// Constructs an add-layer command with a stable ID generated up front.
    pub fn add_layer(name: impl Into<String>, index: usize) -> Self {
        Self::AddLayer {
            id: LayerId::new(),
            name: name.into(),
            index,
        }
    }

    /// Constructs an add-group command with a stable ID generated up front.
    pub fn add_group(
        name: impl Into<String>,
        parent: Option<NodeId>,
        sibling_index: usize,
    ) -> Self {
        Self::AddGroup {
            id: NodeId::new(),
            name: name.into(),
            parent,
            sibling_index,
        }
    }
}
