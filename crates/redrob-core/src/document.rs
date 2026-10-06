// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

use serde::de::{Error as DeError, IgnoredAny, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

use crate::channel::{Channel, ChannelId, MAX_CHANNELS};
use crate::color_mode::{ColorMode, DitherMode, MAX_PALETTE_COLORS, PaletteChoice};
use crate::command::{
    Affine2D, BrushPoint, BrushSettings, BrushSmoothing, GradientKind, GradientStop,
    MAX_BRUSH_DABS, MAX_BRUSH_PIXEL_VISITS, MAX_BRUSH_POINTS, MAX_BRUSH_SIZE, SamplingMode,
    WarpMode,
};
use crate::precision::{Converted, Precision};
use crate::render::source_over;
use crate::selection::SelectionMode;
use crate::{CoreError, RasterBytes, Result, Selection};

pub const MAX_NODES: usize = 4_096;
pub const MAX_HIERARCHY_DEPTH: usize = 256;
pub const MAX_FRAMES: usize = 10_000;
pub const MAX_STORED_RASTER_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_TEXT_BYTES: usize = 1024 * 1024;
pub const MAX_TEXT_CONTENT_BYTES: usize = 256 * 1024;
pub const MAX_FONT_FAMILY_BYTES: usize = 256;
pub const MAX_FONT_ID_BYTES: usize = 128;
pub const MAX_VECTOR_PATHS: usize = 4_096;
pub const MAX_PATH_COMMANDS: usize = 1_000_000;
pub const MAX_PATH_COMMANDS_PER_PATH: usize = 65_536;
pub const MAX_SEMANTIC_MEMORY_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_METADATA_ENTRIES: usize = 4_096;
pub const MAX_METADATA_BYTES: usize = 1024 * 1024;
pub const MAX_NODE_NAME_BYTES: usize = 4_096;
pub const MAX_FRAME_DURATION_MS: u32 = 86_400_000;
pub const MAX_TIMELINE_FPS: f32 = 240.0;
pub const EMBEDDED_FONT_ID: &str = crate::semantic::EMBEDDED_FONT_ID;

fn default_font_id() -> String {
    EMBEDDED_FONT_ID.to_owned()
}

fn deserialize_bounded_string<'de, D, const MAX: usize>(
    deserializer: D,
) -> std::result::Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    struct BoundedStringVisitor<const MAX: usize>;

    impl<'de, const MAX: usize> Visitor<'de> for BoundedStringVisitor<MAX> {
        type Value = String;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "a UTF-8 string of at most {MAX} bytes")
        }

        fn visit_borrowed_str<E>(self, value: &'de str) -> std::result::Result<String, E>
        where
            E: DeError,
        {
            self.visit_str(value)
        }

        fn visit_str<E>(self, value: &str) -> std::result::Result<String, E>
        where
            E: DeError,
        {
            if value.len() > MAX {
                return Err(E::custom(format_args!("string exceeds {MAX} bytes")));
            }
            Ok(value.to_owned())
        }

        fn visit_string<E>(self, value: String) -> std::result::Result<String, E>
        where
            E: DeError,
        {
            if value.len() > MAX {
                return Err(E::custom(format_args!("string exceeds {MAX} bytes")));
            }
            Ok(value)
        }
    }

    deserializer.deserialize_string(BoundedStringVisitor::<MAX>)
}

fn deserialize_text_string<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_bounded_string::<D, MAX_TEXT_CONTENT_BYTES>(deserializer)
}

fn deserialize_font_family<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_bounded_string::<D, MAX_FONT_FAMILY_BYTES>(deserializer)
}

fn deserialize_font_id<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_bounded_string::<D, MAX_FONT_ID_BYTES>(deserializer)
}

/// How the lines of a text node line up (paragraph text, P10).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

impl TextAlign {
    fn is_left(&self) -> bool {
        *self == Self::Left
    }
}

/// Text uses the compiled-in public-domain font8x8 Basic Latin bitmap only.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextContent {
    #[serde(deserialize_with = "deserialize_text_string")]
    pub text: String,
    /// Informational metadata only; it is never resolved through the OS.
    #[serde(deserialize_with = "deserialize_font_family")]
    pub font_family: String,
    pub font_size: f32,
    pub color: Pixel,
    #[serde(default)]
    pub origin_x: f32,
    #[serde(default)]
    pub origin_y: f32,
    #[serde(default = "default_font_id", deserialize_with = "deserialize_font_id")]
    pub font_id: String,
    /// Paragraph text: lines wrap at word boundaries to fit this width in canvas pixels. `None` is
    /// point text, where lines break only at `\n`. Omitted when absent, so existing documents stay
    /// byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub box_width: Option<f32>,
    /// Line alignment, inside `box_width` when set, else inside the longest line.
    #[serde(default, skip_serializing_if = "TextAlign::is_left")]
    pub align: TextAlign,
}

/// Deterministic path winding rule.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

/// Bounded solid vector stroke. Caps and joins are deliberately fixed to
/// round sample-distance behavior in the v2 native scope.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrokeStyle {
    pub color: Pixel,
    pub width: f32,
}

pub(crate) const MAX_DIMENSION: u32 = 32_768;
pub(crate) const MAX_PIXELS: u64 = 64 * 1024 * 1024;

pub(crate) fn pixel_count(width: u32, height: u32) -> Result<usize> {
    let count = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(CoreError::InvalidDimensions { width, height })?;
    if width == 0
        || height == 0
        || width > MAX_DIMENSION
        || height > MAX_DIMENSION
        || count > MAX_PIXELS
        || count > usize::MAX as u64
    {
        return Err(CoreError::InvalidDimensions { width, height });
    }
    Ok(count as usize)
}

/// A stable layer identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LayerId(Uuid);

impl LayerId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub const fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for LayerId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for LayerId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Stable identifier for any document node. `LayerId` remains the compatibility name.
pub type NodeId = LayerId;

/// Stable timeline frame identifier.
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct FrameId(u32);

impl FrameId {
    pub const DEFAULT: Self = Self(0);

    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

/// An eight-bit, straight-alpha RGBA pixel.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Pixel {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Pixel {
    pub const TRANSPARENT: Self = Self::rgba(0, 0, 0, 0);

    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    pub(crate) fn from_slice(bytes: &[u8]) -> Self {
        Self::rgba(bytes[0], bytes[1], bytes[2], bytes[3])
    }

    pub(crate) fn write_to(self, bytes: &mut [u8]) {
        bytes.copy_from_slice(&[self.r, self.g, self.b, self.a]);
    }
}

/// A rectangle in canvas coordinates.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub(crate) fn clipped_bounds(
        self,
        canvas_width: u32,
        canvas_height: u32,
    ) -> Option<(u32, u32, u32, u32)> {
        let x0 = i64::from(self.x).clamp(0, i64::from(canvas_width));
        let y0 = i64::from(self.y).clamp(0, i64::from(canvas_height));
        let x1 = (i64::from(self.x) + i64::from(self.width)).clamp(0, i64::from(canvas_width));
        let y1 = (i64::from(self.y) + i64::from(self.height)).clamp(0, i64::from(canvas_height));
        (x0 < x1 && y0 < y1).then_some((x0 as u32, y0 as u32, x1 as u32, y1 as u32))
    }
}

/// Per-document user metadata.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DocumentMetadata {
    pub title: String,
    pub author: Option<String>,
    pub properties: BTreeMap<String, String>,
}

/// Layer blend function.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlendMode {
    #[default]
    Normal,
    Multiply,
    Screen,
    Overlay,
    Add,
    // A.1 (GIMP gimpoperationlayermode-blend.c / Krita compositeops): the darken/lighten family.
    // Per-channel pairs pick the darker or lighter channel; the Luma pair pick the whole source or
    // destination pixel by its Rec. 709 luma, so a lighten by luma never mixes channels across layers.
    DarkenOnly,
    LightenOnly,
    LumaDarkenOnly,
    LumaLightenOnly,
    // A.2 (GIMP gimpoperationlayermode-blend.c): the dodge/burn and light family. All per-channel.
    Dodge,
    Burn,
    LinearBurn,
    LinearLight,
    VividLight,
    PinLight,
    HardMix,
    // A.3 (GIMP gimpoperationlayermode-blend.c): contrast and grain. All per-channel.
    HardLight,
    SoftLight,
    GrainExtract,
    GrainMerge,
    // A.4 (GIMP gimpoperationlayermode-blend.c): arithmetic. Add already covers GIMP ADDITION.
    Difference,
    Exclusion,
    Subtract,
    Divide,
    // A.5 (GIMP gimpoperationlayermode-blend.c / W3C non-separable): colour composition. These are
    // not per-channel -- they take some components (hue, saturation, value/lightness) from one layer
    // and the rest from the other, so they are resolved as whole pixels before the channel loop.
    HsvHue,
    HsvSaturation,
    HsvValue,
    HslColor,
    LchHue,
    LchChroma,
    LchColor,
    LchLightness,
    Luminance,
    // A.6 (GIMP gimpoperationlayermode-composite.c): composite ops that act on alpha/order, not the
    // colour formula. Behind/Replace/Overwrite/Erase/AntiErase/ColorErase are resolved in composite();
    // Dissolve needs the pixel coordinate and is resolved in composite_buffer(). PassThrough is a group
    // projection flag (a group with it composites its children straight onto the backdrop); at the
    // single-pixel level it behaves as Normal, and the group behaviour is a later backlog item.
    Dissolve,
    Behind,
    Erase,
    AntiErase,
    ColorErase,
    Replace,
    Overwrite,
    PassThrough,
    /// Coverage ADDED rather than composited, clamped so the two alphas cannot sum past full (J.6).
    ///
    /// Upstream's group default. Alpha arithmetic, not a colour formula — see `composite_unit`.
    Merge,
    /// The DIFFERENCE of the two coverages, carrying the colour of whichever had more (J.6).
    Split,
}

/// One frame in a document timeline.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    id: FrameId,
    duration_ms: u32,
}

impl Frame {
    pub const fn new(id: FrameId, duration_ms: u32) -> Self {
        Self { id, duration_ms }
    }

    pub const fn id(self) -> FrameId {
        self.id
    }

    pub const fn duration_ms(self) -> u32 {
        self.duration_ms
    }
}

impl Default for Frame {
    fn default() -> Self {
        Self::new(FrameId::DEFAULT, 100)
    }
}

/// Playback range and UI playback metadata. Playback itself is implemented later.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlaybackMetadata {
    pub range_start: FrameId,
    pub range_end: FrameId,
    pub looping: bool,
    pub playing: bool,
}

impl Default for PlaybackMetadata {
    fn default() -> Self {
        Self {
            range_start: FrameId::DEFAULT,
            range_end: FrameId::DEFAULT,
            looping: false,
            playing: false,
        }
    }
}

/// Ordered frames and playback metadata for a document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Timeline {
    frames: Vec<Frame>,
    current_frame: FrameId,
    fps: f32,
    playback: PlaybackMetadata,
}

impl Default for Timeline {
    fn default() -> Self {
        Self {
            frames: vec![Frame::default()],
            current_frame: FrameId::DEFAULT,
            fps: 10.0,
            playback: PlaybackMetadata::default(),
        }
    }
}

impl Timeline {
    pub fn frames(&self) -> &[Frame] {
        &self.frames
    }

    pub const fn current_frame(&self) -> FrameId {
        self.current_frame
    }

    pub const fn fps(&self) -> f32 {
        self.fps
    }

    pub const fn playback(&self) -> PlaybackMetadata {
        self.playback
    }

    pub fn frame_index(&self, id: FrameId) -> Option<usize> {
        self.frames.iter().position(|frame| frame.id == id)
    }

    pub fn contains(&self, id: FrameId) -> bool {
        self.frame_index(id).is_some()
    }
}

pub fn timeline_frame_duration_ms(fps: f32) -> Result<u32> {
    if !fps.is_finite() || fps <= 0.0 || fps > MAX_TIMELINE_FPS {
        return Err(CoreError::InvalidTimelineFps);
    }
    Ok((1000.0_f64 / f64::from(fps)).round().max(1.0) as u32)
}

/// Optional 8-bit node mask, stored with clone-on-write ownership.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RasterMask {
    #[serde(default = "default_enabled")]
    enabled: bool,
    pixels: RasterBytes,
}

const fn default_enabled() -> bool {
    true
}

impl RasterMask {
    pub fn new(pixels: Vec<u8>) -> Self {
        Self {
            enabled: true,
            pixels: pixels.into(),
        }
    }

    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn storage(&self) -> &RasterBytes {
        &self.pixels
    }
}

// Text payload is defined near the public semantic limits above so its serde
// defaults remain explicit and auditable.

/// A bounded semantic vector path command.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PathCommand {
    MoveTo {
        x: f32,
        y: f32,
    },
    LineTo {
        x: f32,
        y: f32,
    },
    CubicTo {
        control1_x: f32,
        control1_y: f32,
        control2_x: f32,
        control2_y: f32,
        x: f32,
        y: f32,
    },
    Close,
}

/// One semantic vector path with optional solid fill and bounded solid stroke.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VectorPath {
    #[serde(deserialize_with = "deserialize_path_commands")]
    pub commands: Vec<PathCommand>,
    #[serde(default)]
    pub fill: Option<Pixel>,
    #[serde(default)]
    pub stroke: Option<StrokeStyle>,
    #[serde(default)]
    pub fill_rule: FillRule,
}

fn deserialize_bounded_vec<'de, D, T, const MAX: usize>(
    deserializer: D,
    label: &'static str,
) -> std::result::Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct BoundedVecVisitor<T, const MAX: usize> {
        label: &'static str,
        marker: std::marker::PhantomData<T>,
    }

    impl<'de, T, const MAX: usize> Visitor<'de> for BoundedVecVisitor<T, MAX>
    where
        T: Deserialize<'de>,
    {
        type Value = Vec<T>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "{} array with at most {MAX} entries", self.label)
        }

        fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Vec<T>, A::Error>
        where
            A: SeqAccess<'de>,
        {
            // Deliberately ignore an untrusted size hint so a forged JSON
            // envelope cannot force a large speculative allocation.
            let mut values = Vec::new();
            while values.len() < MAX {
                let Some(value) = sequence.next_element()? else {
                    return Ok(values);
                };
                values.push(value);
            }
            if sequence.next_element::<IgnoredAny>()?.is_some() {
                return Err(A::Error::custom(format_args!(
                    "{} count exceeds {MAX}",
                    self.label
                )));
            }
            Ok(values)
        }
    }

    deserializer.deserialize_seq(BoundedVecVisitor::<T, MAX> {
        label,
        marker: std::marker::PhantomData,
    })
}

fn deserialize_path_commands<'de, D>(
    deserializer: D,
) -> std::result::Result<Vec<PathCommand>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_bounded_vec::<D, PathCommand, MAX_PATH_COMMANDS_PER_PATH>(
        deserializer,
        "path command",
    )
}

fn deserialize_vector_paths<'de, D>(
    deserializer: D,
) -> std::result::Result<Vec<VectorPath>, D::Error>
where
    D: Deserializer<'de>,
{
    struct VectorPathsVisitor;

    impl<'de> Visitor<'de> for VectorPathsVisitor {
        type Value = Vec<VectorPath>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(
                formatter,
                "vector path array with at most {MAX_VECTOR_PATHS} entries"
            )
        }

        fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Vec<VectorPath>, A::Error>
        where
            A: SeqAccess<'de>,
        {
            // Start with the fixed vector allocation, then charge each path
            // before retaining it. The sequence size hint is untrusted.
            let mut usage = SemanticUsage {
                memory_bytes: std::mem::size_of::<VectorContent>(),
                ..SemanticUsage::default()
            };
            let mut paths = Vec::new();
            while paths.len() < MAX_VECTOR_PATHS {
                let Some(path) = sequence.next_element()? else {
                    return Ok(paths);
                };
                usage = admit_semantic_replacement(
                    usage,
                    SemanticUsage::default(),
                    semantic_usage_for_vector_path(&path).map_err(A::Error::custom)?,
                )
                .map_err(A::Error::custom)?;
                paths.push(path);
            }
            if sequence.next_element::<IgnoredAny>()?.is_some() {
                return Err(A::Error::custom(format_args!(
                    "vector path count exceeds {MAX_VECTOR_PATHS}"
                )));
            }
            Ok(paths)
        }
    }

    deserializer.deserialize_seq(VectorPathsVisitor)
}

/// Semantic vector payload rasterized only by the deterministic core path.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct VectorContent {
    #[serde(deserialize_with = "deserialize_vector_paths")]
    pub paths: Vec<VectorPath>,
}

/// Raster pixels associated with one timeline frame.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RasterCel {
    frame: FrameId,
    pixels: RasterBytes,
}

impl RasterCel {
    pub fn new(frame: FrameId, pixels: Vec<u8>) -> Self {
        Self {
            frame,
            pixels: pixels.into(),
        }
    }

    pub const fn frame(&self) -> FrameId {
        self.frame
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn storage(&self) -> &RasterBytes {
        &self.pixels
    }
}

/// Coarse node kind, independent of a node's payload.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Raster,
    Group,
    Text,
    Vector,
    /// A non-destructive filter over everything below it in its parent (P11). It owns no pixels:
    /// the render runs its filter on a copy of the stack beneath and composites the result back
    /// with the node's own opacity, mask and blend mode.
    Adjustment,
}

/// Version-2 node payload. Semantic payloads are data-only foundations for later tasks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NodeContent {
    Raster { cels: Vec<RasterCel> },
    Group,
    Text { text: TextContent },
    Vector { vector: VectorContent },
    /// Boxed: `Filter` is the largest enum in the engine and every node would otherwise pay for it.
    Adjustment { filter: Box<crate::Filter> },
}

impl NodeContent {
    pub const fn kind(&self) -> NodeKind {
        match self {
            Self::Raster { .. } => NodeKind::Raster,
            Self::Group => NodeKind::Group,
            Self::Text { .. } => NodeKind::Text,
            Self::Vector { .. } => NodeKind::Vector,
            Self::Adjustment { .. } => NodeKind::Adjustment,
        }
    }

    pub fn adjustment_filter(&self) -> Option<&crate::Filter> {
        match self {
            Self::Adjustment { filter } => Some(filter),
            _ => None,
        }
    }

    pub fn raster_cels(&self) -> Option<&[RasterCel]> {
        match self {
            Self::Raster { cels } => Some(cels),
            _ => None,
        }
    }
}

/// M2: what a layer refuses. `pixels` stops every paint and filter; `transparent` keeps each
/// pixel's alpha, so painting only recolours what is already there; `position` stops moves and
/// transforms. Photoshop's "lock all" is all three.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct LayerLocks {
    #[serde(default, skip_serializing_if = "is_false")]
    pub transparent: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub pixels: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub position: bool,
}

impl LayerLocks {
    pub const fn is_empty(&self) -> bool {
        !self.transparent && !self.pixels && !self.position
    }
}

/// A version-2 document node. The `Layer` name is retained for API compatibility.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    id: LayerId,
    parent: Option<NodeId>,
    name: String,
    visible: bool,
    opacity: f32,
    blend_mode: BlendMode,
    mask: Option<RasterMask>,
    /// M1: a clipping mask -- this node shows only where the nearest unclipped sibling below
    /// it has pixels (Photoshop's Ctrl+Alt+G). Omitted when false, so old documents are unchanged.
    #[serde(default, skip_serializing_if = "is_false")]
    clipped: bool,
    /// M2: Photoshop's layer locks. Omitted when nothing is locked.
    #[serde(default, skip_serializing_if = "LayerLocks::is_empty")]
    locks: LayerLocks,
    /// M11: nodes sharing a link number move and transform together (Photoshop's linked
    /// layers). Omitted when unlinked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    link: Option<u32>,
    content: NodeContent,
}

impl Layer {
    pub fn id(&self) -> LayerId {
        self.id
    }

    pub const fn parent_id(&self) -> Option<NodeId> {
        self.parent
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn is_visible(&self) -> bool {
        self.visible
    }

    pub fn opacity(&self) -> f32 {
        self.opacity
    }

    pub fn blend_mode(&self) -> BlendMode {
        self.blend_mode
    }

    pub const fn kind(&self) -> NodeKind {
        self.content.kind()
    }

    pub fn content(&self) -> &NodeContent {
        &self.content
    }

    pub fn mask(&self) -> Option<&RasterMask> {
        self.mask.as_ref()
    }

    /// M1: whether this node is a clipping mask onto the sibling below it.
    pub const fn is_clipped(&self) -> bool {
        self.clipped
    }

    /// M2: this node's locks.
    pub const fn locks(&self) -> LayerLocks {
        self.locks
    }

    /// M11: the link group this node belongs to, if any.
    pub const fn link(&self) -> Option<u32> {
        self.link
    }

    /// Compatibility accessor for the default-frame raster cel.
    pub fn pixels(&self) -> &[u8] {
        self.raster_pixels(FrameId::DEFAULT).unwrap_or(&[])
    }

    pub fn raster_cels(&self) -> Option<&[RasterCel]> {
        self.content.raster_cels()
    }

    pub fn has_raster_cel(&self, frame: FrameId) -> bool {
        self.raster_cels()
            .is_some_and(|cels| cels.iter().any(|cel| cel.frame == frame))
    }

    pub fn raster_pixels(&self, frame: FrameId) -> Result<&[u8]> {
        let NodeContent::Raster { cels } = &self.content else {
            return Err(CoreError::UnsupportedNodeContent(self.kind()));
        };
        cels.iter()
            .find(|cel| cel.frame == frame)
            .map(RasterCel::pixels)
            .ok_or(CoreError::MissingRasterCel {
                node: self.id,
                frame,
            })
    }

    fn raster_pixels_mut(&mut self, frame: FrameId) -> Result<&mut RasterBytes> {
        let kind = self.kind();
        let NodeContent::Raster { cels } = &mut self.content else {
            return Err(CoreError::UnsupportedNodeContent(kind));
        };
        cels.iter_mut()
            .find(|cel| cel.frame == frame)
            .map(|cel| &mut cel.pixels)
            .ok_or(CoreError::MissingRasterCel {
                node: self.id,
                frame,
            })
    }

    pub fn pixel(&self, width: u32, x: u32, y: u32) -> Option<Pixel> {
        if x >= width {
            return None;
        }
        let pixels = self.pixels();
        let offset = (y as usize)
            .checked_mul(width as usize)?
            .checked_add(x as usize)?
            .checked_mul(4)?;
        (offset + 4 <= pixels.len()).then(|| Pixel::from_slice(&pixels[offset..offset + 4]))
    }

    pub(crate) fn transparent(
        id: LayerId,
        name: String,
        pixel_count: usize,
        precision: Precision,
    ) -> Result<Self> {
        Self::transparent_at(id, name, pixel_count, FrameId::DEFAULT, precision)
    }

    // A new layer's buffer is sized at the DOCUMENT's precision (J.1d). It was `pixel_count * 4`,
    // so adding a layer to a 16-bit document produced a half-length cel that the document's own
    // validator then rejected -- found by the first test that stacked layers on a deep document.
    fn transparent_at(
        id: LayerId,
        name: String,
        pixel_count: usize,
        frame: FrameId,
        precision: Precision,
    ) -> Result<Self> {
        validate_name(&name)?;
        Ok(Self {
            id,
            parent: None,
            name,
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: None,
            clipped: false,
            locks: LayerLocks::default(),
            link: None,
            content: NodeContent::Raster {
                cels: vec![RasterCel {
                    frame,
                    pixels: RasterBytes::zeroed(precision.buffer_len(pixel_count)),
                }],
            },
        })
    }

    pub(crate) fn from_rgba(
        id: LayerId,
        name: String,
        pixels: Vec<u8>,
        pixel_count: usize,
    ) -> Result<Self> {
        let expected = pixel_count * 4;
        if pixels.len() != expected {
            return Err(CoreError::InvalidBufferLength {
                expected,
                actual: pixels.len(),
            });
        }
        validate_name(&name)?;
        Ok(Self {
            id,
            parent: None,
            name,
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: None,
            clipped: false,
            locks: LayerLocks::default(),
            link: None,
            content: NodeContent::Raster {
                cels: vec![RasterCel::new(FrameId::DEFAULT, pixels)],
            },
        })
    }

    pub(crate) fn from_v1_parts(
        id: LayerId,
        name: String,
        visible: bool,
        opacity: f32,
        blend_mode: BlendMode,
        pixels: Vec<u8>,
    ) -> Self {
        Self {
            id,
            parent: None,
            name,
            visible,
            opacity,
            blend_mode,
            mask: None,
            clipped: false,
            locks: LayerLocks::default(),
            link: None,
            content: NodeContent::Raster {
                cels: vec![RasterCel::new(FrameId::DEFAULT, pixels)],
            },
        }
    }

    fn stored_raster_bytes(&self) -> u64 {
        let mask = self
            .mask
            .as_ref()
            .map_or(0, |mask| mask.pixels.len() as u64);
        let cels = self.content.raster_cels().map_or(0, |cels| {
            cels.iter().map(|cel| cel.pixels.len() as u64).sum()
        });
        mask.saturating_add(cels)
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// M6: what Select > Color Range picks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorRange {
    /// Pixels near a sampled colour.
    #[default]
    Sampled,
    Shadows,
    Midtones,
    Highlights,
}

/// M5: where Edit > Stroke puts its band relative to the selection edge.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrokeLocation {
    Inside,
    #[default]
    Center,
    Outside,
}

/// Two-pass 3-4 chamfer distance, in pixels, from every pixel to the nearest pixel whose `inside`
/// differs from its own. A pixel touching the other side is at 1.
fn chamfer(inside: &[bool], w: usize, h: usize) -> Vec<f32> {
    const FAR: u32 = u32::MAX / 4;
    let mut d = vec![FAR; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let differs = |nx: usize, ny: usize| inside[ny * w + nx] != inside[i];
            let edge = (x > 0 && differs(x - 1, y))
                || (x + 1 < w && differs(x + 1, y))
                || (y > 0 && differs(x, y - 1))
                || (y + 1 < h && differs(x, y + 1));
            if edge {
                d[i] = 3;
            }
        }
    }
    let relax = |d: &mut Vec<u32>, i: usize, j: usize, cost: u32| {
        if inside[i] == inside[j] && d[j] + cost < d[i] {
            d[i] = d[j] + cost;
        }
    };
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if x > 0 { relax(&mut d, i, i - 1, 3); }
            if y > 0 {
                relax(&mut d, i, i - w, 3);
                if x > 0 { relax(&mut d, i, i - w - 1, 4); }
                if x + 1 < w { relax(&mut d, i, i - w + 1, 4); }
            }
        }
    }
    for y in (0..h).rev() {
        for x in (0..w).rev() {
            let i = y * w + x;
            if x + 1 < w { relax(&mut d, i, i + 1, 3); }
            if y + 1 < h {
                relax(&mut d, i, i + w, 3);
                if x + 1 < w { relax(&mut d, i, i + w + 1, 4); }
                if x > 0 { relax(&mut d, i, i + w - 1, 4); }
            }
        }
    }
    d.into_iter().map(|v| v as f32 / 3.0).collect()
}

fn validate_name(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        Err(CoreError::EmptyLayerName)
    } else if name.len() > MAX_NODE_NAME_BYTES {
        Err(CoreError::DocumentLimitExceeded("node name bytes"))
    } else {
        Ok(())
    }
}

fn deserialize_document_nodes<'de, D>(deserializer: D) -> std::result::Result<Vec<Layer>, D::Error>
where
    D: Deserializer<'de>,
{
    struct DocumentNodesVisitor;

    impl<'de> Visitor<'de> for DocumentNodesVisitor {
        type Value = Vec<Layer>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "node array with at most {MAX_NODES} entries")
        }

        fn visit_seq<A>(self, mut sequence: A) -> std::result::Result<Vec<Layer>, A::Error>
        where
            A: SeqAccess<'de>,
        {
            // Ignore the untrusted size hint. Semantic usage is charged before
            // each decoded node becomes part of the document.
            let mut usage = SemanticUsage::default();
            let mut layers = Vec::new();
            while layers.len() < MAX_NODES {
                let Some(layer) = sequence.next_element::<Layer>()? else {
                    return Ok(layers);
                };
                usage = admit_semantic_replacement(
                    usage,
                    SemanticUsage::default(),
                    semantic_usage(layer.content()).map_err(A::Error::custom)?,
                )
                .map_err(A::Error::custom)?;
                layers.push(layer);
            }
            if sequence.next_element::<IgnoredAny>()?.is_some() {
                return Err(A::Error::custom(format_args!(
                    "node count exceeds {MAX_NODES}"
                )));
            }
            Ok(layers)
        }
    }

    deserializer.deserialize_seq(DocumentNodesVisitor)
}

/// Detached node description used by [`DocumentImportBuilder`].
///
/// The builder owns every payload and only produces a document after the full
/// document validator accepts the complete hierarchy.
#[derive(Clone, Debug, PartialEq)]
pub struct ImportNode {
    id: NodeId,
    parent: Option<NodeId>,
    name: String,
    visible: bool,
    opacity: f32,
    blend_mode: BlendMode,
    mask: Option<ImportMask>,
    clipped: bool,
    content: NodeContent,
}

/// Detached raster-mask description for atomic document import.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportMask {
    enabled: bool,
    pixels: Vec<u8>,
}

impl ImportMask {
    pub fn new(pixels: Vec<u8>) -> Self {
        Self {
            enabled: true,
            pixels,
        }
    }

    pub fn disabled(pixels: Vec<u8>) -> Self {
        Self {
            enabled: false,
            pixels,
        }
    }
}

impl ImportNode {
    fn new(name: impl Into<String>, content: NodeContent) -> Self {
        Self {
            id: NodeId::new(),
            parent: None,
            name: name.into(),
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: None,
            clipped: false,
            content,
        }
    }

    pub fn raster(name: impl Into<String>, cels: Vec<RasterCel>) -> Self {
        Self::new(name, NodeContent::Raster { cels })
    }

    pub fn group(name: impl Into<String>) -> Self {
        Self::new(name, NodeContent::Group)
    }

    pub fn text(name: impl Into<String>, text: TextContent) -> Self {
        Self::new(name, NodeContent::Text { text })
    }

    pub fn vector(name: impl Into<String>, vector: VectorContent) -> Self {
        Self::new(name, NodeContent::Vector { vector })
    }

    pub fn with_id(mut self, id: NodeId) -> Self {
        self.id = id;
        self
    }

    pub fn with_parent(mut self, parent: Option<NodeId>) -> Self {
        self.parent = parent;
        self
    }

    pub fn with_visibility(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }

    pub fn with_blend_mode(mut self, blend_mode: BlendMode) -> Self {
        self.blend_mode = blend_mode;
        self
    }

    /// M1: imported as a clipping mask (PSD's "clipping" byte, for one).
    pub fn with_clipped(mut self, clipped: bool) -> Self {
        self.clipped = clipped;
        self
    }

    pub fn with_mask(mut self, mask: Option<ImportMask>) -> Self {
        self.mask = mask;
        self
    }

    pub const fn id(&self) -> NodeId {
        self.id
    }
}

/// Safe, detached constructor for complete imported documents.
///
/// Nodes may be supplied in any parent-before/after order. Sibling encounter
/// order is interpreted as bottom-to-top and `build` canonicalizes the final
/// vector to contiguous bottom-to-top postorder before the unchanged document
/// validator is run.
#[derive(Debug)]
pub struct DocumentImportBuilder {
    id: Uuid,
    width: u32,
    height: u32,
    /// The sample width every node's pixels in this import are encoded at (J.1c). Defaults to
    /// 8-bit, which is what an importer that does not set it is handing over.
    precision: Precision,
    metadata: DocumentMetadata,
    nodes: Vec<ImportNode>,
    active_node: Option<NodeId>,
    frames: Vec<FrameId>,
    current_frame: FrameId,
    fps: f32,
    playback: PlaybackMetadata,
    selection_active: bool,
    selection_mask: Vec<u8>,
}

impl DocumentImportBuilder {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        let count = pixel_count(width, height)?;
        Ok(Self {
            id: Uuid::new_v4(),
            width,
            height,
            precision: Precision::U8,
            metadata: DocumentMetadata::default(),
            nodes: Vec::new(),
            active_node: None,
            frames: vec![FrameId::DEFAULT],
            current_frame: FrameId::DEFAULT,
            fps: 10.0,
            playback: PlaybackMetadata::default(),
            selection_active: false,
            selection_mask: vec![0; count],
        })
    }

    /// Declares the sample width of the pixels this import is handing over (J.1c).
    ///
    /// An importer that keeps a deep file's depth MUST call this: the buffers it provides are then
    /// longer than 8-bit ones, and the document's validator measures them against this value.
    pub fn precision(&mut self, precision: Precision) -> &mut Self {
        self.precision = precision;
        self
    }

    pub fn document_id(&mut self, id: Uuid) -> &mut Self {
        self.id = id;
        self
    }

    pub fn metadata(&mut self, metadata: DocumentMetadata) -> &mut Self {
        self.metadata = metadata;
        self
    }

    pub fn timeline(
        &mut self,
        frames: Vec<FrameId>,
        fps: f32,
        current_frame: FrameId,
        playback: PlaybackMetadata,
    ) -> Result<&mut Self> {
        if frames.is_empty() || frames.len() > MAX_FRAMES {
            return Err(CoreError::DocumentLimitExceeded("frame count"));
        }
        timeline_frame_duration_ms(fps)?;
        self.frames = frames;
        self.fps = fps;
        self.current_frame = current_frame;
        self.playback = playback;
        Ok(self)
    }

    pub fn selection(&mut self, active: bool, mask: Vec<u8>) -> Result<&mut Self> {
        let expected = pixel_count(self.width, self.height)?;
        if mask.len() != expected {
            return Err(CoreError::InvalidBufferLength {
                expected,
                actual: mask.len(),
            });
        }
        self.selection_active = active;
        self.selection_mask = mask;
        Ok(self)
    }

    pub fn active_node(&mut self, id: NodeId) -> &mut Self {
        self.active_node = Some(id);
        self
    }

    pub fn push_node(&mut self, node: ImportNode) -> Result<&mut Self> {
        if self.nodes.len() >= MAX_NODES {
            return Err(CoreError::DocumentLimitExceeded("node count"));
        }
        if self.active_node.is_none() {
            self.active_node = Some(node.id);
        }
        self.nodes.push(node);
        Ok(self)
    }

    pub fn build(self) -> Result<Document> {
        if self.nodes.is_empty() {
            return Err(CoreError::LastLayer);
        }
        let duration = timeline_frame_duration_ms(self.fps)?;
        let timeline = Timeline {
            frames: self
                .frames
                .into_iter()
                .map(|id| Frame::new(id, duration))
                .collect(),
            current_frame: self.current_frame,
            fps: self.fps,
            playback: self.playback,
        };
        let mut layers = self
            .nodes
            .into_iter()
            .map(|node| Layer {
                id: node.id,
                parent: node.parent,
                name: node.name,
                visible: node.visible,
                opacity: node.opacity,
                blend_mode: node.blend_mode,
                mask: node.mask.map(|mask| RasterMask {
                    enabled: mask.enabled,
                    pixels: mask.pixels.into(),
                }),
                clipped: node.clipped,
                locks: LayerLocks::default(),
                link: None,
                content: node.content,
            })
            .collect::<Vec<_>>();
        let parents = layers
            .iter()
            .map(|node| (node.id, node.parent))
            .collect::<HashMap<_, _>>();
        let kinds = layers
            .iter()
            .map(|node| (node.id, node.kind()))
            .collect::<HashMap<_, _>>();
        validate_hierarchy(&parents, &kinds)?;
        let siblings = hierarchy_siblings(&layers);
        reorder_nodes(&mut layers, &siblings)?;
        let document = Document {
            id: self.id,
            width: self.width,
            height: self.height,
            metadata: self.metadata,
            precision: self.precision,
            active_layer: self.active_node.ok_or(CoreError::LastLayer)?,
            layers,
            timeline,
            channels: Vec::new(),
            quick_mask: None,
            color_mode: ColorMode::Rgb,
            palette: Vec::new(),
            paths: Vec::new(),
            guides: Vec::new(),
            sample_points: Vec::new(),
            guide_settings: crate::GuideSettings::default(),
            selection: Selection::from_import_parts(
                self.width,
                self.height,
                self.selection_active,
                self.selection_mask,
            )?,
        };
        document.validate()?;
        Ok(document)
    }
}

/// A validated version-2 document. Node order is stable and root raster order is bottom-to-top.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Document {
    id: Uuid,
    width: u32,
    height: u32,
    /// Sample width of this document's stored pixels (J.1a).
    ///
    /// `#[serde(default)]` is what keeps every project written before this field existed loadable,
    /// and loadable as exactly what it is: an absent field reads as [`Precision::U8`], which is
    /// what those bytes always were. Writing it unconditionally would have been the other
    /// defensible choice and is worse — it changes the bytes of every existing project for a value
    /// that was already implied.
    #[serde(default)]
    precision: Precision,
    metadata: DocumentMetadata,
    #[serde(rename = "nodes", deserialize_with = "deserialize_document_nodes")]
    layers: Vec<Layer>,
    #[serde(rename = "active_node")]
    active_layer: LayerId,
    timeline: Timeline,
    selection: Selection,
    /// Named coverage masks stored with the document (J.2a).
    ///
    /// `#[serde(default)]` for the same reason as `precision`: a project written before channels
    /// existed has none, and an absent field is exactly that rather than a parse failure.
    #[serde(default)]
    channels: Vec<Channel>,
    /// The channel the selection is being edited AS, while quick mask is on (J.2b).
    ///
    /// Stored as the channel's id rather than a bool so the mode cannot drift from the list: a flag
    /// plus a by-name lookup would let a renamed or deleted channel leave the document claiming a
    /// mode it is not in.
    #[serde(default)]
    quick_mask: Option<ChannelId>,
    /// How this document's colour is constrained (J.3).
    #[serde(default)]
    color_mode: ColorMode,
    /// Stored paths: geometry that draws nothing by itself (J.4).
    ///
    /// Deliberately NOT layers. A path contributes no pixels, so putting it in the layer stack
    /// would give it group opacity, a blend mode and a place in the flatten order it has no use
    /// for — and the moment someone gave it a stroke to see what they were editing it would start
    /// painting into the image.
    #[serde(default)]
    paths: Vec<crate::Path>,
    /// The palette an indexed document's pixels are drawn from. Empty in any other mode.
    ///
    /// Stored even though the pixels are already snapped to it, because the palette is the
    /// DOCUMENT's property: an indexed file needs it written out, and re-deriving it from the
    /// pixels would silently drop any entry the image happens not to use.
    #[serde(default)]
    palette: Vec<Pixel>,
    /// Infinite alignment lines stored with the document (L.1).
    ///
    /// `#[serde(default)]` for the same reason as `channels`: a project written before guides
    /// existed has none.
    ///
    /// Order is observable, not incidental. The snap rule takes the FIRST candidate at the minimum
    /// distance, so two guides on the same coordinate are distinguishable and the list must not be
    /// silently reordered or deduplicated.
    #[serde(default)]
    guides: Vec<crate::Guide>,
    /// Stored positions whose composited colour the user watches (L.1).
    #[serde(default)]
    sample_points: Vec<crate::SamplePoint>,
    /// How those two are drawn, snapped to and locked (L.1).
    #[serde(default)]
    guide_settings: crate::GuideSettings,
}

impl Document {
    /// H4 (File > New): a one-layer document whose layer is filled with `background`; a fully
    /// transparent colour leaves it empty. Not an edit, so there is nothing to undo.
    pub fn new_filled(width: u32, height: u32, background: Pixel) -> Result<Self> {
        let mut document = Self::new(width, height)?;
        if background.a > 0 {
            document.fill_active(background)?;
        }
        Ok(document)
    }

    pub fn new(width: u32, height: u32) -> Result<Self> {
        let count = pixel_count(width, height)?;
        let id = LayerId::new();
        Ok(Self {
            id: Uuid::new_v4(),
            width,
            height,
            metadata: DocumentMetadata::default(),
            precision: Precision::default(),
            layers: vec![Layer::transparent(
                id,
                "Layer 1".into(),
                count,
                Precision::U8,
            )?],
            active_layer: id,
            timeline: Timeline::default(),
            selection: Selection::new(width, height)?,
            channels: Vec::new(),
            quick_mask: None,
            color_mode: ColorMode::Rgb,
            palette: Vec::new(),
            paths: Vec::new(),
            guides: Vec::new(),
            sample_points: Vec::new(),
            guide_settings: crate::GuideSettings::default(),
        })
    }

    pub fn id(&self) -> Uuid {
        self.id
    }

    /// Sample width of this document's stored pixels.
    pub fn precision(&self) -> Precision {
        self.precision
    }

    /// How this document's colour is constrained (J.3).
    pub fn color_mode(&self) -> ColorMode {
        self.color_mode
    }

    /// The palette an indexed document's pixels are drawn from. Empty in any other mode.
    pub fn palette(&self) -> &[Pixel] {
        &self.palette
    }

    /// Converts the document to `mode`, rewriting every raster cel.
    ///
    /// Refused at a precision other than 8-bit. Indexed and 16-bit are not a combination that means
    /// anything — a palette is at most 256 colours, so the extra sample width can only describe
    /// entries that are not in it — and greyscale at 16 bits would be defensible but is not written
    /// yet. Refusing is better than converting the precision as a side effect of a colour-mode
    /// change the user asked for.
    pub(crate) fn convert_color_mode(
        &mut self,
        mode: ColorMode,
        palette_choice: Option<&PaletteChoice>,
        dither: DitherMode,
    ) -> Result<()> {
        if self.precision != Precision::U8 {
            return Err(CoreError::UnsupportedColorModeConversion);
        }
        let width = self.width as usize;
        match mode {
            ColorMode::Rgb => {
                // Nothing to rewrite: every greyscale and indexed pixel is already a valid RGB one.
                // Converting back is dropping a constraint, not recovering the colour that was lost
                // when it was applied — and saying that here is better than implying a round trip.
                self.palette.clear();
            }
            ColorMode::Grayscale => {
                self.for_each_raster_cel(|pixels| {
                    crate::color_mode::to_grayscale(pixels);
                });
                self.palette.clear();
            }
            ColorMode::Indexed => {
                let choice = palette_choice.ok_or(CoreError::MissingPalette)?;
                // The palette is built from the FLATTENED image, not from one layer: a palette
                // chosen from the bottom layer alone would have no entry for anything painted above
                // it, and every such pixel would snap to the nearest wrong colour.
                let flattened = self.flattened_rgba()?;
                let palette = crate::color_mode::build_palette(&flattened, choice);
                if palette.is_empty() || palette.len() > MAX_PALETTE_COLORS {
                    return Err(CoreError::InvalidPalette(palette.len()));
                }
                self.for_each_raster_cel(|pixels| {
                    let (snapped, _) = crate::color_mode::quantize(pixels, width, &palette, dither);
                    pixels.copy_from_slice(&snapped);
                });
                self.palette = palette;
            }
        }
        self.color_mode = mode;
        Ok(())
    }

    /// Applies `edit` to every raster cel in the document.
    fn for_each_raster_cel(&mut self, mut edit: impl FnMut(&mut [u8])) {
        for node in &mut self.layers {
            if let NodeContent::Raster { cels } = &mut node.content {
                for cel in cels.iter_mut() {
                    let mut pixels = cel.pixels.to_vec();
                    edit(&mut pixels);
                    cel.pixels = RasterBytes::new(pixels);
                }
            }
        }
    }

    /// The composited image as 8-bit RGBA, for decisions that need to see the whole picture.
    fn flattened_rgba(&self) -> Result<Vec<u8>> {
        let snapshot = crate::RenderSnapshot::try_render_frame(self, 0, self.current_frame_id())?;
        Ok(snapshot.rgba8().into_owned())
    }

    /// Snaps the damaged region of an indexed document back onto its palette (J.3-b).
    ///
    /// Returns whether any pixel actually moved, so a caller can tell the user their colour was
    /// changed rather than letting it happen silently.
    ///
    /// # Why this exists at all
    ///
    /// Upstream needs no such function: an indexed drawable's buffer has an indexed FORMAT, so every
    /// write through it is snapped by the pixel library on the way in
    /// (`app/core/gimpdrawable.c` builds every destination buffer with
    /// `gimp_drawable_get_format`). The constraint is the destination, not a check.
    ///
    /// J.3 chose RGBA storage with the mode as a declared constraint, so there is no format to do
    /// the work and the snap has to be performed somewhere explicit. That is the cost of the storage
    /// decision, paid here rather than left as a defect.
    ///
    /// # Why the command boundary, and not each write
    ///
    /// `active_raster_pixels_mut` has well over a dozen callers; hooking each one means the next
    /// write primitive someone adds is off-palette and nothing says so. One snap after the command
    /// completes covers every path that exists and every path yet to be written.
    ///
    /// Only the damaged rectangle is visited: a full-canvas nearest-colour search per brush stroke
    /// would be charged on every stroke in the document, for pixels nobody touched.
    ///
    /// No dithering. Error diffusion is right when converting a whole image at once and wrong per
    /// edit: a solid stroke drawn in an unavailable colour would come out speckled, and the same
    /// stroke drawn twice would speckle differently.
    pub(crate) fn enforce_palette(&mut self, damage: Option<Rect>, layers: &[LayerId]) -> bool {
        if self.color_mode != ColorMode::Indexed || self.palette.is_empty() {
            return false;
        }
        let palette = self.palette.clone();
        let width = self.width as i32;
        let height = self.height as i32;
        // `None` damage means the whole canvas, the same reading the renderer gives it.
        let region = damage.unwrap_or_else(|| Rect::new(0, 0, self.width, self.height));
        let x0 = region.x.max(0);
        let y0 = region.y.max(0);
        let x1 = (region.x + region.width as i32).min(width);
        let y1 = (region.y + region.height as i32).min(height);
        if x0 >= x1 || y0 >= y1 {
            return false;
        }
        let mut snapped = false;
        for node in &mut self.layers {
            if !layers.contains(&node.id) {
                continue;
            }
            let NodeContent::Raster { cels } = &mut node.content else {
                continue;
            };
            for cel in cels.iter_mut() {
                let mut pixels = cel.pixels.to_vec();
                let mut touched = false;
                for y in y0..y1 {
                    for x in x0..x1 {
                        let base = (y as usize * width as usize + x as usize) * 4;
                        // A fully transparent pixel has no colour to constrain, and snapping it
                        // would write a palette colour under zero alpha -- invisible now, and wrong
                        // the moment anything raises that alpha.

                        if pixels[base + 3] == 0 {
                            continue;
                        }
                        let wanted = [
                            i32::from(pixels[base]),
                            i32::from(pixels[base + 1]),
                            i32::from(pixels[base + 2]),
                        ];
                        let entry = palette[crate::color_mode::nearest_index(&palette, wanted)];
                        if entry.r != pixels[base]
                            || entry.g != pixels[base + 1]
                            || entry.b != pixels[base + 2]
                        {
                            pixels[base] = entry.r;
                            pixels[base + 1] = entry.g;
                            pixels[base + 2] = entry.b;
                            touched = true;
                        }
                    }
                }
                if touched {
                    cel.pixels = RasterBytes::new(pixels);
                    snapped = true;
                }
            }
        }
        snapped
    }

    /// Stored paths, in list order (J.4).
    pub fn paths(&self) -> &[crate::Path] {
        &self.paths
    }

    /// Stored guides, in list order (L.1).
    ///
    /// The order is part of the behaviour: `snap_x`/`snap_y` keep the FIRST candidate at the
    /// minimum distance, so two guides on one coordinate are told apart by their place here.
    pub fn guides(&self) -> &[crate::Guide] {
        &self.guides
    }

    /// Stored sample points, in list order (L.1).
    pub fn sample_points(&self) -> &[crate::SamplePoint] {
        &self.sample_points
    }

    /// How this document's guides and sample points are drawn, snapped to and locked (L.1).
    pub fn guide_settings(&self) -> crate::GuideSettings {
        self.guide_settings
    }

    pub(crate) fn set_guide_settings(&mut self, settings: crate::GuideSettings) {
        self.guide_settings = settings;
    }

    /// Place a guide. Refused while the guides are locked.
    ///
    /// `position` is not validated against the canvas, which is upstream's own behaviour: its add
    /// and move paths check nothing, so a guide may sit outside the image and survive a crop.
    pub(crate) fn add_guide(
        &mut self,
        id: crate::GuideId,
        orientation: crate::GuideOrientation,
        position: i32,
        style: crate::GuideStyle,
    ) -> Result<()> {
        self.require_guides_unlocked()?;
        if self.guides.iter().any(|guide| guide.id() == id) {
            return Err(CoreError::DuplicateGuideId(id));
        }
        self.guides
            .push(crate::Guide::new(id, orientation, position, style));
        Ok(())
    }

    pub(crate) fn move_guide(&mut self, id: crate::GuideId, position: i32) -> Result<()> {
        self.require_guides_unlocked()?;
        self.guide_mut(id)?.set_position(position);
        Ok(())
    }

    pub(crate) fn remove_guide(&mut self, id: crate::GuideId) -> Result<()> {
        self.require_guides_unlocked()?;
        let index = self
            .guides
            .iter()
            .position(|guide| guide.id() == id)
            .ok_or(CoreError::GuideNotFound(id))?;
        self.guides.remove(index);
        Ok(())
    }

    pub(crate) fn add_sample_point(
        &mut self,
        id: crate::SamplePointId,
        x: i32,
        y: i32,
    ) -> Result<()> {
        if self.sample_points.iter().any(|point| point.id() == id) {
            return Err(CoreError::DuplicateSamplePointId(id));
        }
        self.sample_points.push(crate::SamplePoint::new(id, x, y));
        Ok(())
    }

    pub(crate) fn move_sample_point(
        &mut self,
        id: crate::SamplePointId,
        x: i32,
        y: i32,
    ) -> Result<()> {
        self.sample_point_mut(id)?.set_position(x, y);
        Ok(())
    }

    pub(crate) fn remove_sample_point(&mut self, id: crate::SamplePointId) -> Result<()> {
        let index = self
            .sample_points
            .iter()
            .position(|point| point.id() == id)
            .ok_or(CoreError::SamplePointNotFound(id))?;
        self.sample_points.remove(index);
        Ok(())
    }

    /// Snap a coordinate onto this document's guides and canvas edges (L.1).
    ///
    /// Reads the stored settings, so a caller cannot snap to guides a document has turned off. The
    /// LOCK is deliberately not consulted: locking stops a guide moving, not a tool aligning to it.
    pub fn snap_point(&self, x: f64, y: f64, epsilon_x: f64, epsilon_y: f64) -> (f64, f64, bool) {
        crate::snap_point(
            &self.guides,
            (self.width, self.height),
            (x, y),
            (epsilon_x, epsilon_y),
            &self.guide_settings,
        )
    }

    fn require_guides_unlocked(&self) -> Result<()> {
        if self.guide_settings.lock_guides {
            return Err(CoreError::GuidesLocked);
        }
        Ok(())
    }

    fn guide_mut(&mut self, id: crate::GuideId) -> Result<&mut crate::Guide> {
        self.guides
            .iter_mut()
            .find(|guide| guide.id() == id)
            .ok_or(CoreError::GuideNotFound(id))
    }

    fn sample_point_mut(&mut self, id: crate::SamplePointId) -> Result<&mut crate::SamplePoint> {
        self.sample_points
            .iter_mut()
            .find(|point| point.id() == id)
            .ok_or(CoreError::SamplePointNotFound(id))
    }

    pub fn path(&self, id: crate::PathId) -> Option<&crate::Path> {
        self.paths.iter().find(|path| path.id == id)
    }

    pub(crate) fn add_path(&mut self, path: crate::Path) -> Result<()> {
        if self.paths.len() >= crate::MAX_PATHS {
            return Err(CoreError::TooManyPaths);
        }
        self.paths.push(path);
        Ok(())
    }

    pub(crate) fn remove_path(&mut self, id: crate::PathId) -> Result<()> {
        let index = self
            .paths
            .iter()
            .position(|path| path.id == id)
            .ok_or(CoreError::UnknownPath(id))?;
        self.paths.remove(index);
        Ok(())
    }

    pub(crate) fn rename_path(&mut self, id: crate::PathId, name: String) -> Result<()> {
        self.path_mut(id)?.name = name;
        Ok(())
    }

    pub(crate) fn set_path_visible(&mut self, id: crate::PathId, visible: bool) -> Result<()> {
        self.path_mut(id)?.visible = visible;
        Ok(())
    }

    fn path_mut(&mut self, id: crate::PathId) -> Result<&mut crate::Path> {
        self.paths
            .iter_mut()
            .find(|path| path.id == id)
            .ok_or(CoreError::UnknownPath(id))
    }

    /// Stores the current selection's outline as a path (J.4).
    ///
    /// Refused when nothing is selected. An inactive selection reports full coverage by design —
    /// every pixel is available — so tracing it would produce a path around the whole canvas, which
    /// is not what anyone pressing this means.
    pub(crate) fn path_from_selection(&mut self, name: String, fit: bool) -> Result<crate::PathId> {
        if !self.selection.is_active() {
            return Err(CoreError::NoSelection);
        }
        let commands = if fit {
            crate::path::trace_mask_outline_fitted(self.selection.mask(), self.width, self.height)
        } else {
            crate::path::trace_mask_outline(self.selection.mask(), self.width, self.height)
        };
        if commands.is_empty() {
            return Err(CoreError::NoSelection);
        }
        let path = crate::Path::new(name, commands);
        let id = path.id;
        self.add_path(path)?;
        Ok(id)
    }

    /// Replaces or combines the selection with a stored path's interior (J.4).
    ///
    /// The path is rasterized through the same filler the vector layers use, so a path and a vector
    /// layer of the same geometry agree about which pixels are inside. A second point-in-polygon
    /// test written here would be a second answer to that question, and the two would drift.
    pub(crate) fn selection_from_path(
        &mut self,
        id: crate::PathId,
        mode: SelectionMode,
    ) -> Result<()> {
        let path = self.path(id).ok_or(CoreError::UnknownPath(id))?;
        let vector = VectorContent {
            paths: vec![VectorPath {
                commands: path.commands.clone(),
                // Filled opaque white: the rasterizer's alpha IS the coverage we want, and a
                // partial edge pixel carries its antialiasing straight into the selection.
                fill: Some(Pixel::rgba(255, 255, 255, 255)),
                stroke: None,
                fill_rule: FillRule::NonZero,
            }],
        };
        let rgba =
            crate::semantic::rasterize(&NodeContent::Vector { vector }, self.width, self.height)?;
        let coverage: Vec<u8> = rgba.chunks_exact(4).map(|pixel| pixel[3]).collect();
        self.selection.apply_mask_shape(coverage, mode);
        Ok(())
    }

    /// The points a brush should be dragged along to stroke a path (J.4).
    pub(crate) fn path_stroke_points(&self, id: crate::PathId) -> Result<Vec<(f32, f32)>> {
        let path = self.path(id).ok_or(CoreError::UnknownPath(id))?;
        crate::path::flatten_to_points(&path.commands)
    }

    /// The document's named coverage masks, in list order (J.2a).
    pub fn channels(&self) -> &[Channel] {
        &self.channels
    }

    /// The channel the selection is currently being edited AS, if quick mask is on (J.2b).
    pub fn quick_mask(&self) -> Option<ChannelId> {
        self.quick_mask
    }

    /// Turns quick mask on or off.
    ///
    /// On: the selection is copied into a channel and the selection is CLEARED. Clearing it is not
    /// tidiness — while quick mask is on the user paints the mask, and a live selection would
    /// confine those strokes to the very region they are meant to redraw.
    ///
    /// Off: the channel's coverage REPLACES the selection and the channel is removed. Replace
    /// rather than intersect or add, because the mask is what the user has just been editing: it is
    /// the answer, not a modifier to one.
    ///
    /// Idempotent. Asking for the state it is already in does nothing rather than stacking a second
    /// mask channel, which is the shape of bug a toggle bound to a keyboard shortcut finds fast.
    pub(crate) fn set_quick_mask(&mut self, active: bool) -> Result<()> {
        match (active, self.quick_mask) {
            (true, None) => {
                let id = ChannelId::new_v4();
                self.add_channel(id, crate::channel::QUICK_MASK_NAME.to_string(), true)?;
                self.selection.clear();
                self.quick_mask = Some(id);
            }
            (false, Some(id)) => {
                let index = self.channel_index(id)?;
                let coverage = self.channels[index].pixels().to_vec();
                // The existing Replace path, not a new one: a quick mask coming back IS a mask
                // shape replacing the selection, which is what this already means.
                self.selection
                    .apply_mask_shape(coverage, crate::SelectionMode::Replace);
                self.channels.remove(index);
                self.quick_mask = None;
            }
            // Already in the requested state.
            _ => {}
        }
        Ok(())
    }

    /// Paints coverage into the quick-mask channel, for a stroke made while the mode is on (J.2b).
    ///
    /// The brush colour's LUMINANCE is the coverage being painted: white adds to the mask, black
    /// takes away, grey lands in between. That is what makes the mode an editor rather than a
    /// viewer — and it is why the colour is read for its brightness rather than written as colour,
    /// which a channel has nowhere to put.
    pub(crate) fn paint_quick_mask(
        &mut self,
        dabs: &[BrushPoint],
        color: Pixel,
        size: f32,
        opacity: f32,
        shape: crate::DabShape,
        tip: Option<&crate::BrushTip>,
    ) -> Result<()> {
        let Some(id) = self.quick_mask else {
            return Ok(());
        };
        let index = self.channel_index(id)?;
        let width = self.width;
        let height = self.height;
        let target = f32::from(crate::channel::luminance_of(color)) / 255.0;
        let mut coverage = self.channels[index].pixels().to_vec();
        for &dab in dabs {
            if dab.pressure <= 0.0 {
                continue;
            }
            let raster = brush_dab_raster(dab, size, width, height);
            let diameter = raster.radius * 2.0;
            let dab_mask = crate::DabMask::new(shape, diameter);
            for y in raster.y0..raster.y1 {
                for x in raster.x0..raster.x1 {
                    let edge = match tip {
                        Some(tip) => tip.coverage_at(
                            x as f32 + 0.5 - dab.x,
                            y as f32 + 0.5 - dab.y,
                            diameter,
                        ),
                        None => {
                            dab_mask.coverage_at(x as f32 + 0.5 - dab.x, y as f32 + 0.5 - dab.y)
                        }
                    };
                    // The SELECTION is deliberately not consulted here. It was cleared on entering
                    // the mode, and consulting a channel's own coverage as a stroke limit would make
                    // the mask impossible to grow where it is currently empty.
                    let k = (edge * dab.pressure * opacity).clamp(0.0, 1.0);
                    if k <= 0.0 {
                        continue;
                    }
                    let at = y as usize * width as usize + x as usize;
                    let here = f32::from(coverage[at]) / 255.0;
                    let mixed = here + (target - here) * k;
                    coverage[at] = (mixed * 255.0).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        self.channels[index].replace_pixels(coverage);
        Ok(())
    }

    fn channel_index(&self, id: ChannelId) -> Result<usize> {
        self.channels
            .iter()
            .position(|channel| channel.id() == id)
            .ok_or(CoreError::ChannelNotFound(id))
    }

    /// Adds a channel, optionally seeded with the current selection's coverage.
    ///
    /// `from_selection` is not a convenience: an empty channel is a channel the user cannot put
    /// anything into from here, so without it the list would be addable and useless.
    pub(crate) fn add_channel(
        &mut self,
        id: ChannelId,
        name: String,
        from_selection: bool,
    ) -> Result<()> {
        if self.channels.len() >= MAX_CHANNELS {
            return Err(CoreError::DocumentLimitExceeded("channel count"));
        }
        if self.channels.iter().any(|channel| channel.id() == id) {
            return Err(CoreError::DuplicateChannelId(id));
        }
        validate_name(&name)?;
        let count = pixel_count(self.width, self.height)?;
        let pixels = if from_selection {
            // The selection's mask is already one byte per pixel over the whole canvas, which is
            // exactly a channel's storage — so this is a copy, not a conversion.
            self.selection.mask().to_vec()
        } else {
            vec![0u8; count]
        };
        self.channels.push(Channel::new(id, name, pixels));
        Ok(())
    }

    pub(crate) fn remove_channel(&mut self, id: ChannelId) -> Result<()> {
        let index = self.channel_index(id)?;
        self.channels.remove(index);
        Ok(())
    }

    pub(crate) fn set_channel_visible(&mut self, id: ChannelId, visible: bool) -> Result<()> {
        let index = self.channel_index(id)?;
        self.channels[index].set_visible(visible);
        Ok(())
    }

    pub(crate) fn set_channel_opacity(&mut self, id: ChannelId, opacity: f32) -> Result<()> {
        // Same bound as a layer's: a non-finite or out-of-range opacity is refused rather than
        // clamped, so a caller learns its value was wrong instead of silently getting another one.
        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
            return Err(CoreError::InvalidOpacity);
        }
        let index = self.channel_index(id)?;
        self.channels[index].set_opacity(opacity);
        Ok(())
    }

    pub(crate) fn set_channel_color(&mut self, id: ChannelId, color: Pixel) -> Result<()> {
        let index = self.channel_index(id)?;
        self.channels[index].set_color(color);
        Ok(())
    }

    pub(crate) fn set_channel_show_masked(
        &mut self,
        id: ChannelId,
        show_masked: bool,
    ) -> Result<()> {
        let index = self.channel_index(id)?;
        self.channels[index].set_show_masked(show_masked);
        Ok(())
    }

    pub(crate) fn rename_channel(&mut self, id: ChannelId, name: String) -> Result<()> {
        validate_name(&name)?;
        let index = self.channel_index(id)?;
        self.channels[index].set_name(name);
        Ok(())
    }

    /// Re-encodes every raster cel into `target` and records it as the document's precision.
    ///
    /// Returns whether the change DROPPED bits. A caller that ignores that is how a deep import
    /// came to be silently narrowed, so it is a return value rather than a log line.
    ///
    /// Every cel is converted before anything is stored. A partial conversion would leave the
    /// document holding a mixture of widths under one declared precision, which no reader could
    /// interpret — and unlike a refused edit, it is not recoverable by undo, because the bytes it
    /// would undo to are the ones already overwritten.
    pub(crate) fn set_precision(&mut self, target: Precision) -> bool {
        if self.precision == target {
            return false;
        }
        let source = self.precision;
        let mut narrowed = false;
        let mut rewritten: Vec<(usize, usize, RasterBytes)> = Vec::new();
        for (node_index, node) in self.layers.iter().enumerate() {
            if let NodeContent::Raster { cels } = node.content() {
                for (cel_index, cel) in cels.iter().enumerate() {
                    let converted: Converted = source.convert(cel.pixels.as_slice(), target);
                    narrowed |= converted.narrowed;
                    rewritten.push((node_index, cel_index, RasterBytes::new(converted.bytes)));
                }
            }
        }
        for (node_index, cel_index, bytes) in rewritten {
            if let NodeContent::Raster { cels } = &mut self.layers[node_index].content
                && let Some(cel) = cels.get_mut(cel_index)
            {
                cel.pixels = bytes;
            }
        }
        self.precision = target;
        narrowed
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn metadata(&self) -> &DocumentMetadata {
        &self.metadata
    }

    /// Compatibility view of all nodes. Existing flat documents contain raster layers only.
    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    pub fn nodes(&self) -> &[Layer] {
        &self.layers
    }

    pub fn timeline(&self) -> &Timeline {
        &self.timeline
    }

    pub const fn current_frame_id(&self) -> FrameId {
        self.timeline.current_frame
    }

    pub fn active_layer_id(&self) -> LayerId {
        self.active_layer
    }

    pub fn active_layer(&self) -> &Layer {
        // Validation guarantees this compatibility accessor has a target.
        self.active_node()
            .expect("validated document always has an active node")
    }

    pub fn active_node(&self) -> Option<&Layer> {
        self.layer(self.active_layer)
    }

    pub fn active_raster_pixels(&self) -> Result<&[u8]> {
        self.active_node()
            .ok_or(CoreError::LayerNotFound(self.active_layer))?
            .raster_pixels(self.current_frame_id())
    }

    pub fn layer(&self, id: LayerId) -> Option<&Layer> {
        self.layers.iter().find(|layer| layer.id == id)
    }

    /// Returns a node's validated hierarchy depth, where roots are depth zero.
    pub fn node_depth(&self, id: NodeId) -> Option<usize> {
        let mut current = self.layer(id)?.parent;
        let mut depth = 0_usize;
        while let Some(parent) = current {
            depth = depth.checked_add(1)?;
            if depth > MAX_HIERARCHY_DEPTH {
                return None;
            }
            current = self.layer(parent)?.parent;
        }
        Some(depth)
    }

    /// Returns direct children in deterministic bottom-to-top sibling order.
    pub fn child_ids(&self, parent: Option<NodeId>) -> Vec<NodeId> {
        self.sibling_ids(parent)
    }

    fn sibling_ids(&self, parent: Option<NodeId>) -> Vec<NodeId> {
        self.layers
            .iter()
            .filter(|node| node.parent == parent)
            .map(|node| node.id)
            .collect()
    }

    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// Returns owned selection mask bytes that remain valid across document edits.
    pub fn selection_mask_snapshot(&self) -> Vec<u8> {
        self.selection.snapshot_bytes()
    }

    pub fn is_selection_active(&self) -> bool {
        self.selection.is_active()
    }

    pub(crate) fn layer_mut(&mut self, id: LayerId) -> Result<&mut Layer> {
        self.layers
            .iter_mut()
            .find(|layer| layer.id == id)
            .ok_or(CoreError::LayerNotFound(id))
    }

    fn materialize_raster_cel(&mut self, id: LayerId, frame: FrameId) -> Result<()> {
        let node = self.layer(id).ok_or(CoreError::LayerNotFound(id))?;
        if node.kind() != NodeKind::Raster {
            return Err(CoreError::UnsupportedNodeContent(node.kind()));
        }
        if node.has_raster_cel(frame) {
            return Ok(());
        }
        // At the document's precision, not four bytes a pixel (J.1d): a cel materialized for a new
        // frame on a deep document would otherwise be half or a quarter length.
        let bytes = pixel_count(self.width, self.height)?
            .checked_mul(self.precision.bytes_per_pixel())
            .ok_or(CoreError::DocumentLimitExceeded("stored raster bytes"))?;
        if self.stored_raster_bytes().saturating_add(bytes as u64) > MAX_STORED_RASTER_BYTES {
            return Err(CoreError::DocumentLimitExceeded("stored raster bytes"));
        }
        let node = self.layer_mut(id)?;
        let NodeContent::Raster { cels } = &mut node.content else {
            unreachable!("raster kind checked above")
        };
        cels.push(RasterCel {
            frame,
            pixels: RasterBytes::zeroed(bytes),
        });
        Ok(())
    }

    pub(crate) fn prepare_active_raster_edit(&mut self) -> Result<()> {
        self.materialize_raster_cel(self.active_layer, self.current_frame_id())
    }

    /// A copy of the active raster cel at the current frame for READING (selection tools): no
    /// lock check and no cel materialised. A layer with no cel on this frame reads as empty.
    fn active_raster_copy(&self) -> Result<Vec<u8>> {
        let node = self.layer(self.active_layer).ok_or(CoreError::LayerNotFound(self.active_layer))?;
        if node.kind() != NodeKind::Raster {
            return Err(CoreError::UnsupportedNodeContent(node.kind()));
        }
        match node.raster_pixels(self.current_frame_id()) {
            Ok(pixels) => Ok(pixels.to_vec()),
            Err(_) => Ok(vec![0; self.precision.buffer_len(pixel_count(self.width, self.height)?)]),
        }
    }

    fn active_raster_pixels_mut(&mut self) -> Result<&mut RasterBytes> {
        // M2: every paint and filter on the active layer comes through here.
        if self.layer(self.active_layer).is_some_and(|node| node.locks.pixels) {
            return Err(CoreError::LayerLocked { id: self.active_layer, what: "pixels" });
        }
        let frame = self.current_frame_id();
        self.materialize_raster_cel(self.active_layer, frame)?;
        self.layer_mut(self.active_layer)?.raster_pixels_mut(frame)
    }

    pub(crate) fn set_metadata(&mut self, metadata: DocumentMetadata) {
        self.metadata = metadata;
    }

    pub(crate) fn add_frame(&mut self, id: FrameId, index: usize) -> Result<()> {
        if self.timeline.contains(id) {
            return Err(CoreError::DuplicateFrameId(id));
        }
        if self.timeline.frames.len() >= MAX_FRAMES {
            return Err(CoreError::DocumentLimitExceeded("frame count"));
        }
        if index > self.timeline.frames.len() {
            return Err(CoreError::FrameIndexOutOfBounds {
                index,
                len: self.timeline.frames.len(),
            });
        }
        let duration_ms = timeline_frame_duration_ms(self.timeline.fps)?;
        self.timeline
            .frames
            .insert(index, Frame::new(id, duration_ms));
        self.timeline.playback.playing = false;
        Ok(())
    }

    pub(crate) fn duplicate_frame(
        &mut self,
        source: FrameId,
        id: FrameId,
        index: usize,
    ) -> Result<()> {
        if self.timeline.contains(id) {
            return Err(CoreError::DuplicateFrameId(id));
        }
        if !self.timeline.contains(source) {
            return Err(CoreError::FrameNotFound(source));
        }
        if self.timeline.frames.len() >= MAX_FRAMES {
            return Err(CoreError::DocumentLimitExceeded("frame count"));
        }
        if index > self.timeline.frames.len() {
            return Err(CoreError::FrameIndexOutOfBounds {
                index,
                len: self.timeline.frames.len(),
            });
        }
        let additional = self
            .layers
            .iter()
            .filter_map(|node| {
                node.raster_cels()?
                    .iter()
                    .find(|cel| cel.frame == source)
                    .map(|cel| cel.pixels.len() as u64)
            })
            .sum::<u64>();
        if self.stored_raster_bytes().saturating_add(additional) > MAX_STORED_RASTER_BYTES {
            return Err(CoreError::DocumentLimitExceeded("stored raster bytes"));
        }
        let duration_ms = timeline_frame_duration_ms(self.timeline.fps)?;
        self.timeline
            .frames
            .insert(index, Frame::new(id, duration_ms));
        for node in &mut self.layers {
            let NodeContent::Raster { cels } = &mut node.content else {
                continue;
            };
            if let Some(source_cel) = cels.iter().find(|cel| cel.frame == source) {
                cels.push(RasterCel {
                    frame: id,
                    pixels: source_cel.pixels.clone(),
                });
            }
        }
        self.timeline.playback.playing = false;
        Ok(())
    }

    pub(crate) fn remove_frame(&mut self, id: FrameId) -> Result<()> {
        if self.timeline.frames.len() == 1 {
            return Err(CoreError::LastFrame);
        }
        let removed_index = self
            .timeline
            .frame_index(id)
            .ok_or(CoreError::FrameNotFound(id))?;
        let surviving_current = if self.timeline.current_frame == id {
            self.timeline.frames[if removed_index + 1 < self.timeline.frames.len() {
                removed_index + 1
            } else {
                removed_index - 1
            }]
            .id
        } else {
            self.timeline.current_frame
        };
        // Same reason as `materialize_raster_cel`: a replacement cel is sized at the document's
        // precision (J.1d).
        let cel_bytes = pixel_count(self.width, self.height)?
            .checked_mul(self.precision.bytes_per_pixel())
            .ok_or(CoreError::DocumentLimitExceeded("stored raster bytes"))?
            as u64;
        let removed_bytes = self
            .layers
            .iter()
            .filter_map(|node| {
                node.raster_cels()?
                    .iter()
                    .find(|cel| cel.frame == id)
                    .map(|cel| cel.pixels.len() as u64)
            })
            .sum::<u64>();
        let empty_rasters = self
            .layers
            .iter()
            .filter_map(|node| node.raster_cels())
            .filter(|cels| cels.len() == 1 && cels[0].frame == id)
            .count() as u64;
        let target_bytes = self
            .stored_raster_bytes()
            .saturating_sub(removed_bytes)
            .saturating_add(empty_rasters.saturating_mul(cel_bytes));
        if target_bytes > MAX_STORED_RASTER_BYTES {
            return Err(CoreError::DocumentLimitExceeded("stored raster bytes"));
        }

        let old_range_start = self.timeline.playback.range_start;
        let old_range_end = self.timeline.playback.range_end;
        self.timeline.frames.remove(removed_index);
        self.timeline.current_frame = surviving_current;
        if old_range_start == id {
            self.timeline.playback.range_start =
                self.timeline.frames[removed_index.min(self.timeline.frames.len() - 1)].id;
        }
        if old_range_end == id {
            self.timeline.playback.range_end = self.timeline.frames[removed_index
                .saturating_sub(1)
                .min(self.timeline.frames.len() - 1)]
            .id;
        }
        if self
            .timeline
            .frame_index(self.timeline.playback.range_start)
            > self.timeline.frame_index(self.timeline.playback.range_end)
        {
            let fallback =
                self.timeline.frames[removed_index.min(self.timeline.frames.len() - 1)].id;
            self.timeline.playback.range_start = fallback;
            self.timeline.playback.range_end = fallback;
        }
        for node in &mut self.layers {
            let NodeContent::Raster { cels } = &mut node.content else {
                continue;
            };
            cels.retain(|cel| cel.frame != id);
            if cels.is_empty() {
                cels.push(RasterCel {
                    frame: surviving_current,
                    pixels: RasterBytes::zeroed(cel_bytes as usize),
                });
            }
        }
        self.timeline.playback.playing = false;
        Ok(())
    }

    pub(crate) fn move_frame(&mut self, id: FrameId, new_index: usize) -> Result<()> {
        let old_index = self
            .timeline
            .frame_index(id)
            .ok_or(CoreError::FrameNotFound(id))?;
        if new_index >= self.timeline.frames.len() {
            return Err(CoreError::FrameIndexOutOfBounds {
                index: new_index,
                len: self.timeline.frames.len(),
            });
        }
        let mut frames = self.timeline.frames.clone();
        let frame = frames.remove(old_index);
        frames.insert(new_index, frame);
        let position = |target| frames.iter().position(|frame| frame.id == target);
        if position(self.timeline.playback.range_start) > position(self.timeline.playback.range_end)
        {
            return Err(CoreError::PlaybackRangeOrder);
        }
        self.timeline.frames = frames;
        self.timeline.playback.playing = false;
        Ok(())
    }

    pub(crate) fn set_timeline_fps(&mut self, fps: f32) -> Result<()> {
        let duration_ms = timeline_frame_duration_ms(fps)?;
        self.timeline.fps = fps;
        for frame in &mut self.timeline.frames {
            frame.duration_ms = duration_ms;
        }
        Ok(())
    }

    pub(crate) fn set_playback_range(&mut self, start: FrameId, end: FrameId) -> Result<()> {
        let start_index = self
            .timeline
            .frame_index(start)
            .ok_or(CoreError::FrameNotFound(start))?;
        let end_index = self
            .timeline
            .frame_index(end)
            .ok_or(CoreError::FrameNotFound(end))?;
        if start_index > end_index {
            return Err(CoreError::InvalidPlaybackRange);
        }
        self.timeline.playback.range_start = start;
        self.timeline.playback.range_end = end;
        Ok(())
    }

    pub(crate) fn set_looping(&mut self, looping: bool) {
        self.timeline.playback.looping = looping;
    }

    pub(crate) fn set_current_frame(&mut self, id: FrameId) -> Result<()> {
        if !self.timeline.contains(id) {
            return Err(CoreError::FrameNotFound(id));
        }
        self.timeline.current_frame = id;
        Ok(())
    }

    pub(crate) fn set_playing(&mut self, playing: bool) {
        if playing {
            let current = self.timeline.frame_index(self.timeline.current_frame);
            let start = self
                .timeline
                .frame_index(self.timeline.playback.range_start);
            let end = self.timeline.frame_index(self.timeline.playback.range_end);
            if !matches!((current, start, end), (Some(current), Some(start), Some(end)) if (start..=end).contains(&current))
            {
                self.timeline.current_frame = self.timeline.playback.range_start;
            }
        }
        self.timeline.playback.playing = playing;
    }

    pub(crate) fn advance_playback(&mut self) {
        if !self.timeline.playback.playing {
            return;
        }
        let current = self
            .timeline
            .frame_index(self.timeline.current_frame)
            .unwrap_or(0);
        let start = self
            .timeline
            .frame_index(self.timeline.playback.range_start)
            .unwrap_or(0);
        let end = self
            .timeline
            .frame_index(self.timeline.playback.range_end)
            .unwrap_or(start);
        if current < start || current > end {
            self.timeline.current_frame = self.timeline.playback.range_start;
        } else if current < end {
            self.timeline.current_frame = self.timeline.frames[current + 1].id;
        } else if self.timeline.playback.looping {
            self.timeline.current_frame = self.timeline.playback.range_start;
        } else {
            self.timeline.playback.playing = false;
        }
    }

    pub(crate) fn stop_playback(&mut self) {
        self.timeline.playback.playing = false;
    }

    pub(crate) fn add_layer(&mut self, id: LayerId, name: String, index: usize) -> Result<()> {
        let root_count = self.sibling_ids(None).len();
        if index > root_count {
            return Err(CoreError::LayerIndexOutOfBounds {
                index,
                len: root_count,
            });
        }
        let pixels = pixel_count(self.width, self.height)?;
        let additional = (pixels as u64)
            .checked_mul(4)
            .ok_or(CoreError::DocumentLimitExceeded("stored raster bytes"))?;
        if self.stored_raster_bytes().saturating_add(additional) > MAX_STORED_RASTER_BYTES {
            return Err(CoreError::DocumentLimitExceeded("stored raster bytes"));
        }
        let layer =
            Layer::transparent_at(id, name, pixels, self.current_frame_id(), self.precision)?;
        self.insert_node(layer, None, index)
    }

    pub(crate) fn add_group(
        &mut self,
        id: NodeId,
        name: String,
        parent: Option<NodeId>,
        sibling_index: usize,
    ) -> Result<()> {
        validate_name(&name)?;
        let group = Layer {
            id,
            parent,
            name,
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: None,
            clipped: false,
            locks: LayerLocks::default(),
            link: None,
            content: NodeContent::Group,
        };
        self.insert_node(group, parent, sibling_index)
    }

    pub(crate) fn add_text_node(
        &mut self,
        id: NodeId,
        name: String,
        parent: Option<NodeId>,
        sibling_index: usize,
        text: TextContent,
    ) -> Result<()> {
        validate_name(&name)?;
        crate::semantic::validate_text(&text)?;
        let node = Layer {
            id,
            parent,
            name,
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: None,
            clipped: false,
            locks: LayerLocks::default(),
            link: None,
            content: NodeContent::Text { text },
        };
        self.insert_node(node, parent, sibling_index)
    }

    pub(crate) fn add_vector_node(
        &mut self,
        id: NodeId,
        name: String,
        parent: Option<NodeId>,
        sibling_index: usize,
        vector: VectorContent,
    ) -> Result<()> {
        validate_name(&name)?;
        crate::semantic::validate_vector(&vector)?;
        let node = Layer {
            id,
            parent,
            name,
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: None,
            clipped: false,
            locks: LayerLocks::default(),
            link: None,
            content: NodeContent::Vector { vector },
        };
        self.insert_node(node, parent, sibling_index)
    }

    pub(crate) fn set_text_content(&mut self, id: NodeId, text: TextContent) -> Result<()> {
        crate::semantic::validate_text(&text)?;
        let node = self.layer_mut(id)?;
        if node.kind() != NodeKind::Text {
            return Err(CoreError::UnsupportedNodeContent(node.kind()));
        }
        node.content = NodeContent::Text { text };
        Ok(())
    }

    pub(crate) fn set_vector_content(&mut self, id: NodeId, vector: VectorContent) -> Result<()> {
        crate::semantic::validate_vector(&vector)?;
        let node = self.layer_mut(id)?;
        if node.kind() != NodeKind::Vector {
            return Err(CoreError::UnsupportedNodeContent(node.kind()));
        }
        node.content = NodeContent::Vector { vector };
        Ok(())
    }

    /// Adds an adjustment node (P11). The filter is checked here, at the command, so a filter the
    /// render could never run is refused when it is added instead of failing every later frame.
    pub(crate) fn add_adjustment_node(
        &mut self,
        id: NodeId,
        name: String,
        parent: Option<NodeId>,
        sibling_index: usize,
        filter: crate::Filter,
    ) -> Result<()> {
        validate_name(&name)?;
        crate::filters::validate_adjustment_filter(&filter)?;
        crate::filters::check_adjustment_precision(&filter, self.precision)?;
        let node = Layer {
            id,
            parent,
            name,
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: None,
            clipped: false,
            locks: LayerLocks::default(),
            link: None,
            content: NodeContent::Adjustment {
                filter: Box::new(filter),
            },
        };
        self.insert_node(node, parent, sibling_index)
    }

    pub(crate) fn set_adjustment_filter(&mut self, id: NodeId, filter: crate::Filter) -> Result<()> {
        crate::filters::validate_adjustment_filter(&filter)?;
        crate::filters::check_adjustment_precision(&filter, self.precision)?;
        let node = self.layer_mut(id)?;
        if node.kind() != NodeKind::Adjustment {
            return Err(CoreError::UnsupportedNodeContent(node.kind()));
        }
        node.content = NodeContent::Adjustment {
            filter: Box::new(filter),
        };
        Ok(())
    }

    pub(crate) fn rasterize_semantic_node(&mut self, id: NodeId) -> Result<()> {
        let node = self.layer(id).ok_or(CoreError::LayerNotFound(id))?;
        if !matches!(node.kind(), NodeKind::Text | NodeKind::Vector) {
            return Err(CoreError::UnsupportedNodeContent(node.kind()));
        }
        let bytes = pixel_count(self.width, self.height)?
            .checked_mul(4)
            .ok_or(CoreError::DocumentLimitExceeded("stored raster bytes"))?;
        if self.stored_raster_bytes().saturating_add(bytes as u64) > MAX_STORED_RASTER_BYTES {
            return Err(CoreError::DocumentLimitExceeded("stored raster bytes"));
        }
        crate::semantic::preflight(node.content(), self.width, self.height)?;
        let pixels = crate::semantic::rasterize(node.content(), self.width, self.height)?;
        let frame = self.current_frame_id();
        self.layer_mut(id)?.content = NodeContent::Raster {
            cels: vec![RasterCel::new(frame, pixels)],
        };
        Ok(())
    }

    fn insert_node(
        &mut self,
        mut node: Layer,
        parent: Option<NodeId>,
        sibling_index: usize,
    ) -> Result<()> {
        if self.layers.len() >= MAX_NODES {
            return Err(CoreError::DocumentLimitExceeded("node count"));
        }
        if self.layer(node.id).is_some() {
            return Err(CoreError::DuplicateNodeId(node.id));
        }
        self.validate_parent(parent)?;
        let mut siblings = hierarchy_siblings(&self.layers);
        let destination = siblings.entry(parent).or_default();
        if sibling_index > destination.len() {
            return Err(CoreError::SiblingIndexOutOfBounds {
                index: sibling_index,
                len: destination.len(),
            });
        }
        node.parent = parent;
        let id = node.id;
        destination.insert(sibling_index, id);
        siblings.entry(Some(id)).or_default();
        self.layers.push(node);
        reorder_nodes(&mut self.layers, &siblings)?;
        self.active_layer = id;
        Ok(())
    }

    /// H1 (Ctrl+J): copies `source` -- and, for a group, every node inside it -- to the sibling
    /// slot just above it, names the top copy "<name> copy" and makes it active. All limits are
    /// checked before anything is inserted, so a refusal leaves the document as it was.
    pub(crate) fn duplicate_node(&mut self, source: NodeId, id: NodeId) -> Result<()> {
        let root = self.layer(source).ok_or(CoreError::LayerNotFound(source))?;
        let parent = root.parent;
        // The subtree in document order: a parent always precedes its children, and children
        // keep their stacking order because `layers` is kept in sibling order.
        let mut subtree = vec![source];
        let mut next = 0;
        while next < subtree.len() {
            let owner = subtree[next];
            subtree.extend(self.layers.iter().filter(|n| n.parent == Some(owner)).map(|n| n.id));
            next += 1;
        }
        if self.layers.len() + subtree.len() > MAX_NODES {
            return Err(CoreError::DocumentLimitExceeded("node count"));
        }
        let added: u64 = subtree
            .iter()
            .filter_map(|n| self.layer(*n))
            .map(Layer::stored_raster_bytes)
            .sum();
        if self.stored_raster_bytes().saturating_add(added) > MAX_STORED_RASTER_BYTES {
            return Err(CoreError::DocumentLimitExceeded("stored raster bytes"));
        }
        let remap = |old: NodeId| -> NodeId {
            if old == source {
                return id;
            }
            // XOR with a fixed key is one-to-one, so distinct originals get distinct copies.
            let key = id.as_uuid().as_u128();
            LayerId::from_uuid(Uuid::from_u128(old.as_uuid().as_u128() ^ key))
        };
        let copies: Vec<NodeId> = subtree.iter().map(|n| remap(*n)).collect();
        for copy in &copies {
            if self.layer(*copy).is_some() {
                return Err(CoreError::DuplicateNodeId(*copy));
            }
        }
        let sibling_index = self
            .sibling_ids(parent)
            .iter()
            .position(|n| *n == source)
            .map_or(0, |i| i + 1);
        for (index, original) in subtree.iter().enumerate() {
            let mut node = self.layer(*original).expect("subtree nodes exist").clone();
            node.id = copies[index];
            if index == 0 {
                let renamed = format!("{} copy", node.name);
                if renamed.len() <= MAX_NODE_NAME_BYTES {
                    node.name = renamed;
                }
                self.insert_node(node, parent, sibling_index)?;
            } else {
                let new_parent = remap(node.parent.expect("inner nodes have a parent"));
                let at = self.sibling_ids(Some(new_parent)).len();
                self.insert_node(node, Some(new_parent), at)?;
            }
        }
        self.active_layer = id;
        Ok(())
    }

    /// H2 (Ctrl+E): composites raster layer `id` into the raster layer directly below it in the same
    /// group, with `id`'s opacity, blend mode and enabled mask, then removes `id`. The lower layer
    /// keeps its own name, opacity, blend mode and mask, as in Photoshop. Every frame where `id` has
    /// a cel is merged, so an animated layer does not lose frames. Checks run before any pixel is
    /// written.
    pub(crate) fn merge_down(&mut self, id: NodeId) -> Result<NodeId> {
        let upper = self.layer(id).ok_or(CoreError::LayerNotFound(id))?;
        if upper.kind() != NodeKind::Raster {
            return Err(CoreError::UnsupportedNodeContent(upper.kind()));
        }
        if !upper.is_visible() {
            return Err(CoreError::MergeHiddenLayer(id));
        }
        let siblings = self.sibling_ids(upper.parent);
        let position = siblings.iter().position(|n| *n == id).expect("a node is its parent's child");
        let lower_id = *position
            .checked_sub(1)
            .and_then(|i| siblings.get(i))
            .ok_or(CoreError::NothingBelowToMerge(id))?;
        let lower = self.layer(lower_id).expect("sibling exists");
        if lower.kind() != NodeKind::Raster {
            return Err(CoreError::NothingBelowToMerge(id));
        }
        if lower.locks.pixels || upper.locks.pixels {
            return Err(CoreError::LayerLocked { id: if lower.locks.pixels { lower_id } else { id }, what: "pixels" });
        }
        let frames: Vec<FrameId> = upper
            .raster_cels()
            .unwrap_or_default()
            .iter()
            .map(|cel| cel.frame)
            .collect();
        let missing = frames.iter().filter(|f| !lower.has_raster_cel(**f)).count();
        let cel_bytes = pixel_count(self.width, self.height)?
            .checked_mul(self.precision.bytes_per_pixel())
            .ok_or(CoreError::DocumentLimitExceeded("stored raster bytes"))?;
        if self
            .stored_raster_bytes()
            .saturating_add((missing * cel_bytes) as u64)
            > MAX_STORED_RASTER_BYTES
        {
            return Err(CoreError::DocumentLimitExceeded("stored raster bytes"));
        }
        let opacity = upper.opacity();
        let mode = upper.blend_mode();
        let mask: Option<Vec<u8>> = upper
            .mask()
            .filter(|mask| mask.is_enabled())
            .map(|mask| mask.pixels().to_vec());
        let sources: Vec<(FrameId, Vec<u8>)> = frames
            .iter()
            .map(|f| Ok((*f, upper.raster_pixels(*f)?.to_vec())))
            .collect::<Result<_>>()?;
        let (width, height, precision) = (self.width, self.height, self.precision);
        for (frame, source) in &sources {
            self.materialize_raster_cel(lower_id, *frame)?;
            let destination = self.layer_mut(lower_id)?.raster_pixels_mut(*frame)?;
            crate::render::composite_buffer(
                precision,
                destination,
                source,
                mask.as_deref(),
                opacity,
                mode,
                width,
                (0, 0, width, height),
            );
        }
        self.remove_layer(id)?;
        self.active_layer = lower_id;
        Ok(lower_id)
    }

    /// H3. Merge visible (Ctrl+Shift+E) when `background` is `None`; Flatten image when it is a
    /// colour. The visible stack is composited -- every frame, so animation survives -- into one
    /// raster layer `id`, which replaces the visible top-level nodes at the slot of the topmost of
    /// them. Merge visible keeps hidden top-level nodes where they were; Flatten removes them and
    /// lays the composite over `background`, as Photoshop fills a flattened image's transparency.
    /// A hidden node INSIDE a visible group goes with its group, as in Photoshop.
    pub(crate) fn merge_visible(&mut self, id: NodeId, background: Option<Pixel>) -> Result<()> {
        let roots = self.sibling_ids(None);
        let visible: Vec<NodeId> = roots
            .iter()
            .copied()
            .filter(|n| self.layer(*n).is_some_and(Layer::is_visible))
            .collect();
        if visible.is_empty() {
            return Err(CoreError::NothingVisibleToMerge);
        }
        if self.layer(id).is_some() {
            return Err(CoreError::DuplicateNodeId(id));
        }
        let frames: Vec<FrameId> = self.timeline.frames.iter().map(|f| f.id()).collect();
        let cel_bytes = pixel_count(self.width, self.height)?
            .checked_mul(self.precision.bytes_per_pixel())
            .ok_or(CoreError::DocumentLimitExceeded("stored raster bytes"))?;
        let mut cels = Vec::with_capacity(frames.len());
        for frame in &frames {
            let mut pixels = crate::render::composite_frame(self, *frame)?;
            if let Some(color) = background {
                let mut base = vec![0_u8; cel_bytes];
                let unit = [
                    f32::from(color.r) / 255.0,
                    f32::from(color.g) / 255.0,
                    f32::from(color.b) / 255.0,
                    f32::from(color.a) / 255.0,
                ];
                for pixel in 0..cel_bytes / self.precision.bytes_per_pixel() {
                    for (channel, value) in unit.iter().enumerate() {
                        self.precision.write_sample(&mut base, pixel * 4 + channel, *value);
                    }
                }
                crate::render::composite_buffer(
                    self.precision,
                    &mut base,
                    &pixels,
                    None,
                    1.0,
                    BlendMode::Normal,
                    self.width,
                    (0, 0, self.width, self.height),
                );
                pixels = base;
            }
            cels.push(RasterCel::new(*frame, pixels));
        }
        // Everything that goes: visible roots and their subtrees (and, flattening, all roots).
        let doomed_roots: Vec<NodeId> = if background.is_some() { roots.clone() } else { visible.clone() };
        let mut doomed = doomed_roots.clone();
        let mut next = 0;
        while next < doomed.len() {
            let owner = doomed[next];
            doomed.extend(self.layers.iter().filter(|n| n.parent == Some(owner)).map(|n| n.id));
            next += 1;
        }
        let kept_bytes: u64 = self
            .layers
            .iter()
            .filter(|n| !doomed.contains(&n.id))
            .map(Layer::stored_raster_bytes)
            .sum::<u64>()
            .saturating_add(self.selection.mask().len() as u64);
        if kept_bytes.saturating_add((cel_bytes * frames.len()) as u64) > MAX_STORED_RASTER_BYTES {
            return Err(CoreError::DocumentLimitExceeded("stored raster bytes"));
        }
        // The slot: where the topmost doomed root sits, counted among the roots that stay.
        let topmost = *doomed_roots.last().expect("at least one visible root");
        let slot = roots
            .iter()
            .take_while(|n| **n != topmost)
            .filter(|n| !doomed_roots.contains(n))
            .count();
        let name = if background.is_some() { "Background" } else { "Merged" };
        self.layers.retain(|n| !doomed.contains(&n.id));
        let node = Layer {
            id,
            parent: None,
            name: name.to_string(),
            visible: true,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            mask: None,
            clipped: false,
            locks: LayerLocks::default(),
            link: None,
            content: NodeContent::Raster { cels },
        };
        self.insert_node(node, None, slot)?;
        self.active_layer = id;
        Ok(())
    }

    /// H5 (Ctrl+C): the active raster layer at the current frame, cut to the selection's bounding
    /// box, as straight 8-bit RGBA. Partly selected pixels keep that share of their alpha and
    /// unselected ones are transparent. With no selection the whole layer is copied. A layer with
    /// no cel on this frame copies as transparent.
    pub fn copy_active_rgba(&self) -> Result<(Rect, Vec<u8>)> {
        let node = self.layer(self.active_layer).ok_or(CoreError::LayerNotFound(self.active_layer))?;
        if node.kind() != NodeKind::Raster {
            return Err(CoreError::UnsupportedNodeContent(node.kind()));
        }
        let width = self.width;
        let selection = &self.selection;
        let rect = if selection.is_active() {
            let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
            for (index, coverage) in selection.mask().iter().enumerate() {
                if *coverage > 0 {
                    let (x, y) = (index as u32 % width, index as u32 / width);
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x + 1);
                    y1 = y1.max(y + 1);
                }
            }
            if x0 == u32::MAX {
                return Err(CoreError::NoSelection);
            }
            Rect { x: x0 as i32, y: y0 as i32, width: x1 - x0, height: y1 - y0 }
        } else {
            Rect { x: 0, y: 0, width, height: self.height }
        };
        let mut out = vec![0_u8; rect.width as usize * rect.height as usize * 4];
        let Ok(pixels) = node.raster_pixels(self.current_frame_id()) else {
            return Ok((rect, out));
        };
        let precision = self.precision;
        let byte = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
        for row in 0..rect.height {
            for column in 0..rect.width {
                let (x, y) = (rect.x as u32 + column, rect.y as u32 + row);
                let coverage = f32::from(selection.coverage(x, y)) / 255.0;
                if coverage == 0.0 {
                    continue;
                }
                let source = (y as usize * width as usize + x as usize) * 4;
                let target = (row as usize * rect.width as usize + column as usize) * 4;
                for channel in 0..3 {
                    out[target + channel] = byte(precision.read_sample(pixels, source + channel));
                }
                out[target + 3] = byte(precision.read_sample(pixels, source + 3) * coverage);
            }
        }
        Ok((rect, out))
    }

    /// H5 (Ctrl+V): adds raster layer `id` just above the active node, holding `pixels` (straight
    /// 8-bit RGBA, `rect.width * rect.height * 4` bytes) at `rect`'s position. The part outside the
    /// canvas is dropped, as Photoshop clips a paste to the canvas; the layer is made active.
    pub(crate) fn paste_layer(&mut self, id: NodeId, name: String, rect: Rect, pixels: &[u8]) -> Result<()> {
        let expected = (rect.width as usize)
            .checked_mul(rect.height as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or(CoreError::DocumentLimitExceeded("pasted pixels"))?;
        if pixels.len() != expected {
            return Err(CoreError::InvalidBufferLength { expected, actual: pixels.len() });
        }
        let count = pixel_count(self.width, self.height)?;
        let additional = self.precision.buffer_len(count) as u64;
        if self.stored_raster_bytes().saturating_add(additional) > MAX_STORED_RASTER_BYTES {
            return Err(CoreError::DocumentLimitExceeded("stored raster bytes"));
        }
        let mut layer = Layer::transparent_at(id, name, count, self.current_frame_id(), self.precision)?;
        let precision = self.precision;
        let frame = self.current_frame_id();
        {
            let target = layer.raster_pixels_mut(frame)?;
            for row in 0..rect.height as i64 {
                let y = rect.y as i64 + row;
                if y < 0 || y >= i64::from(self.height) {
                    continue;
                }
                for column in 0..rect.width as i64 {
                    let x = rect.x as i64 + column;
                    if x < 0 || x >= i64::from(self.width) {
                        continue;
                    }
                    let source = ((row * rect.width as i64 + column) * 4) as usize;
                    let at = ((y * i64::from(self.width) + x) * 4) as usize;
                    for channel in 0..4 {
                        precision.write_sample(target, at + channel, f32::from(pixels[source + channel]) / 255.0);
                    }
                }
            }
        }
        let active = self.layer(self.active_layer).map(|n| (n.id, n.parent));
        let (parent, slot) = match active {
            Some((active_id, parent)) => {
                let siblings = self.sibling_ids(parent);
                (parent, siblings.iter().position(|n| *n == active_id).map_or(siblings.len(), |i| i + 1))
            }
            None => (None, self.sibling_ids(None).len()),
        };
        self.insert_node(layer, parent, slot)
    }

    pub(crate) fn set_layer_clipped(&mut self, id: NodeId, clipped: bool) -> Result<()> {
        self.layer_mut(id)?.clipped = clipped;
        Ok(())
    }

    pub(crate) fn set_layer_locks(&mut self, id: NodeId, locks: LayerLocks) -> Result<()> {
        self.layer_mut(id)?.locks = locks;
        Ok(())
    }

    /// M11: links `ids` into one new group (dropping any links they had), or unlinks them. A
    /// group left with a single member is dissolved, since a link of one means nothing.
    pub(crate) fn link_layers(&mut self, ids: &[NodeId], link: bool) -> Result<()> {
        for id in ids {
            self.layer(*id).ok_or(CoreError::LayerNotFound(*id))?;
        }
        let group = link.then(|| self.layers.iter().filter_map(|n| n.link).max().map_or(1, |m| m + 1));
        for id in ids {
            self.layer_mut(*id)?.link = group;
        }
        let mut counts = HashMap::<u32, usize>::new();
        for node in &self.layers {
            if let Some(g) = node.link {
                *counts.entry(g).or_default() += 1;
            }
        }
        for node in &mut self.layers {
            if node.link.is_some_and(|g| counts[&g] < 2) {
                node.link = None;
            }
        }
        Ok(())
    }

    /// M11: the other nodes linked to `id`.
    pub(crate) fn linked_with(&self, id: NodeId) -> Vec<NodeId> {
        let Some(group) = self.layer(id).and_then(|n| n.link) else {
            return Vec::new();
        };
        self.layers.iter().filter(|n| n.link == Some(group) && n.id != id).map(|n| n.id).collect()
    }

    /// M2: the active cel's alpha samples, when the active layer locks its transparency, so the
    /// command bus can put them back after an edit.
    pub(crate) fn locked_alpha_snapshot(&self) -> Option<(NodeId, FrameId, Vec<f32>)> {
        let node = self.layer(self.active_layer)?;
        if !node.locks.transparent || node.locks.pixels {
            return None;
        }
        let frame = self.current_frame_id();
        let pixels = node.raster_pixels(frame).ok()?;
        let count = self.width as usize * self.height as usize;
        let alpha = (0..count).map(|p| self.precision.read_sample(pixels, p * 4 + 3)).collect();
        Some((node.id, frame, alpha))
    }

    pub(crate) fn restore_locked_alpha(&mut self, id: NodeId, frame: FrameId, alpha: &[f32]) {
        let precision = self.precision;
        let count = self.width as usize * self.height as usize;
        if alpha.len() != count {
            return; // The canvas changed size; there is nothing to line the alpha up with.
        }
        let Ok(node) = self.layer_mut(id) else { return };
        let Ok(pixels) = node.raster_pixels_mut(frame) else { return };
        for (pixel, value) in alpha.iter().enumerate() {
            precision.write_sample(pixels, pixel * 4 + 3, *value);
        }
    }

    pub(crate) fn remove_layer(&mut self, id: LayerId) -> Result<()> {
        if self.layers.len() == 1 {
            return Err(CoreError::LastLayer);
        }
        let index = self
            .layers
            .iter()
            .position(|layer| layer.id == id)
            .ok_or(CoreError::LayerNotFound(id))?;
        if self.layers.iter().any(|layer| layer.parent == Some(id)) {
            return Err(CoreError::NonEmptyGroup(id));
        }
        self.layers.remove(index);
        if self.active_layer == id {
            self.active_layer = self.layers[index.min(self.layers.len() - 1)].id;
        }
        Ok(())
    }

    /// Compatibility root-level reorder. Hierarchical callers use `MoveNode`.
    pub(crate) fn reorder_layer(&mut self, id: LayerId, new_index: usize) -> Result<()> {
        let parent = self.layer(id).ok_or(CoreError::LayerNotFound(id))?.parent;
        if parent.is_some() {
            return Err(CoreError::InvalidHierarchyOrder);
        }
        let root_count = self.sibling_ids(None).len();
        if new_index >= root_count {
            return Err(CoreError::LayerIndexOutOfBounds {
                index: new_index,
                len: root_count,
            });
        }
        self.move_node(id, None, new_index)
    }

    pub(crate) fn move_node(
        &mut self,
        id: NodeId,
        parent: Option<NodeId>,
        sibling_index: usize,
    ) -> Result<()> {
        let current_parent = self.layer(id).ok_or(CoreError::LayerNotFound(id))?.parent;
        self.validate_parent(parent)?;
        if let Some(parent_id) = parent {
            let mut current = Some(parent_id);
            while let Some(ancestor) = current {
                if ancestor == id {
                    return Err(CoreError::HierarchyCycle {
                        node: id,
                        parent: parent_id,
                    });
                }
                current = self.layer(ancestor).and_then(|node| node.parent);
            }
        }

        let mut siblings = hierarchy_siblings(&self.layers);
        let source = siblings.entry(current_parent).or_default();
        let Some(source_index) = source.iter().position(|candidate| *candidate == id) else {
            return Err(CoreError::InvalidHierarchyOrder);
        };
        source.remove(source_index);
        let destination = siblings.entry(parent).or_default();
        if sibling_index > destination.len() {
            return Err(CoreError::SiblingIndexOutOfBounds {
                index: sibling_index,
                len: destination.len(),
            });
        }
        destination.insert(sibling_index, id);
        self.layer_mut(id)?.parent = parent;
        reorder_nodes(&mut self.layers, &siblings)
    }

    fn validate_parent(&self, parent: Option<NodeId>) -> Result<()> {
        let Some(parent) = parent else {
            return Ok(());
        };
        let node = self.layer(parent).ok_or(CoreError::LayerNotFound(parent))?;
        if node.kind() != NodeKind::Group {
            return Err(CoreError::ParentIsNotGroup(parent));
        }
        Ok(())
    }

    pub(crate) fn add_raster_mask(&mut self, id: NodeId) -> Result<()> {
        let count = pixel_count(self.width, self.height)?;
        if self.stored_raster_bytes().saturating_add(count as u64) > MAX_STORED_RASTER_BYTES {
            return Err(CoreError::DocumentLimitExceeded("stored raster bytes"));
        }
        let node = self.layer_mut(id)?;
        if !matches!(node.kind(), NodeKind::Raster | NodeKind::Group) {
            return Err(CoreError::UnsupportedNodeContent(node.kind()));
        }
        if node.mask.is_some() {
            return Err(CoreError::RasterMaskAlreadyExists(id));
        }
        node.mask = Some(RasterMask::new(vec![u8::MAX; count]));
        Ok(())
    }

    pub(crate) fn remove_raster_mask(&mut self, id: NodeId) -> Result<()> {
        let node = self.layer_mut(id)?;
        if node.mask.take().is_none() {
            return Err(CoreError::RasterMaskNotFound(id));
        }
        Ok(())
    }

    pub(crate) fn set_raster_mask_enabled(&mut self, id: NodeId, enabled: bool) -> Result<()> {
        let mask = self
            .layer_mut(id)?
            .mask
            .as_mut()
            .ok_or(CoreError::RasterMaskNotFound(id))?;
        mask.enabled = enabled;
        Ok(())
    }

    pub(crate) fn raster_mask_from_selection(&mut self, id: NodeId) -> Result<()> {
        if !self.selection.is_active() {
            return Err(CoreError::SelectionNotActive);
        }
        let node = self.layer(id).ok_or(CoreError::LayerNotFound(id))?;
        if !matches!(node.kind(), NodeKind::Raster | NodeKind::Group) {
            return Err(CoreError::UnsupportedNodeContent(node.kind()));
        }
        if node.mask.is_none() {
            let count = pixel_count(self.width, self.height)? as u64;
            let target_bytes = self
                .stored_raster_bytes()
                .checked_add(count)
                .ok_or(CoreError::DocumentLimitExceeded("stored raster bytes"))?;
            if target_bytes > MAX_STORED_RASTER_BYTES {
                return Err(CoreError::DocumentLimitExceeded("stored raster bytes"));
            }
        }

        let coverage = self.selection.snapshot_bytes();
        let node = self.layer_mut(id)?;
        node.mask = Some(RasterMask::new(coverage));
        Ok(())
    }

    pub(crate) fn replace_raster_mask(
        &mut self,
        id: NodeId,
        rect: Rect,
        pixels: &[u8],
    ) -> Result<()> {
        let count = u64::from(rect.width)
            .checked_mul(u64::from(rect.height))
            .and_then(|count| usize::try_from(count).ok())
            .ok_or(CoreError::InvalidMaskOperation)?;
        let x1 = i64::from(rect.x) + i64::from(rect.width);
        let y1 = i64::from(rect.y) + i64::from(rect.height);
        if rect.x < 0
            || rect.y < 0
            || x1 > i64::from(self.width)
            || y1 > i64::from(self.height)
            || count == 0
            || count > crate::MAX_MASK_COMMAND_PIXELS
            || pixels.len() != count
        {
            return Err(CoreError::InvalidMaskOperation);
        }
        let width = self.width as usize;
        let mask = self
            .layer_mut(id)?
            .mask
            .as_mut()
            .ok_or(CoreError::RasterMaskNotFound(id))?;
        for row in 0..rect.height as usize {
            let destination = (rect.y as usize + row) * width + rect.x as usize;
            let source = row * rect.width as usize;
            mask.pixels[destination..destination + rect.width as usize]
                .copy_from_slice(&pixels[source..source + rect.width as usize]);
        }
        Ok(())
    }

    pub(crate) fn set_active_layer(&mut self, id: LayerId) -> Result<()> {
        if self.layer(id).is_none() {
            return Err(CoreError::LayerNotFound(id));
        }
        self.active_layer = id;
        Ok(())
    }

    pub(crate) fn rename_layer(&mut self, id: LayerId, name: String) -> Result<()> {
        validate_name(&name)?;
        self.layer_mut(id)?.name = name;
        Ok(())
    }

    pub(crate) fn set_layer_visibility(&mut self, id: LayerId, visible: bool) -> Result<()> {
        self.layer_mut(id)?.visible = visible;
        Ok(())
    }

    pub(crate) fn set_layer_opacity(&mut self, id: LayerId, opacity: f32) -> Result<()> {
        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
            return Err(CoreError::InvalidOpacity);
        }
        self.layer_mut(id)?.opacity = opacity;
        Ok(())
    }

    pub(crate) fn set_layer_blend_mode(&mut self, id: LayerId, mode: BlendMode) -> Result<()> {
        self.layer_mut(id)?.blend_mode = mode;
        Ok(())
    }

    pub(crate) fn clear_selection(&mut self) {
        self.selection.clear();
    }

    pub(crate) fn select_rect(&mut self, rect: Rect, mode: crate::SelectionMode) {
        self.selection.apply_rect(rect, mode);
    }

    pub(crate) fn select_ellipse(&mut self, rect: Rect, mode: crate::SelectionMode) {
        self.selection.apply_ellipse(rect, mode);
    }

    pub(crate) fn select_polygon(&mut self, points: &[(f32, f32)], mode: crate::SelectionMode) {
        self.selection.apply_polygon(points, mode);
    }

    /// Magic wand: select pixels of the active layer whose colour is within `tolerance` of the pixel
    /// at (x, y). `contiguous` true floods only the connected region (GIMP's fuzzy select / Krita
    /// contiguous), false matches every pixel on the layer (GIMP by-colour / Krita similar). The Lab
    /// tolerance is the same metric the bucket fill uses, so a wand and a fill agree.
    /// M6 (Select > Color Range): selects every pixel of the active layer by how close it is to
    /// `color` (soft: full inside `fuzziness / 2`, fading to none at `fuzziness`), or by tone range,
    /// as Photoshop's Color Range dialog. Global, not contiguous; partial pixels get partial
    /// selection, which is what makes it different from the magic wand.
    pub(crate) fn select_color_range(
        &mut self,
        color: Pixel,
        fuzziness: u8,
        range: ColorRange,
        mode: crate::SelectionMode,
    ) -> Result<()> {
        let snapshot = self.active_raster_copy()?;
        let fuzz = f32::from(fuzziness.max(1));
        let ramp = |value: f32, full: f32, none: f32| -> f32 {
            // 1 at `full`, 0 at `none`, linear between (either direction).
            ((value - none) / (full - none)).clamp(0.0, 1.0)
        };
        let shape: Vec<u8> = snapshot
            .chunks_exact(4)
            .map(|p| {
                let pixel = Pixel::from_slice(p);
                let luma = 0.299 * f32::from(pixel.r) + 0.587 * f32::from(pixel.g) + 0.114 * f32::from(pixel.b);
                let amount = match range {
                    ColorRange::Sampled => {
                        let d = f32::from(crate::colour_difference(color, pixel));
                        ramp(d, fuzz / 2.0, fuzz)
                    }
                    ColorRange::Shadows => ramp(luma, 64.0, 128.0),
                    ColorRange::Highlights => ramp(luma, 192.0, 128.0),
                    ColorRange::Midtones => ramp(luma, 96.0, 32.0).min(ramp(luma, 160.0, 224.0)),
                };
                // Transparent pixels have no colour to match.
                (amount * f32::from(pixel.a)).round() as u8
            })
            .collect();
        self.selection.apply_mask_shape(shape, mode);
        Ok(())
    }

    pub(crate) fn select_by_color(
        &mut self,
        x: u32,
        y: u32,
        tolerance: u8,
        contiguous: bool,
        mode: crate::SelectionMode,
    ) -> Result<()> {
        let width = self.width;
        let height = self.height;
        if x >= width || y >= height {
            return Err(CoreError::InvalidFilterParameter);
        }
        let snapshot = self.active_raster_copy()?;
        let mut shape = vec![0_u8; (width as usize) * (height as usize)];
        if contiguous {
            let options = crate::FloodFillOptions {
                tolerance,
                opacity_spread: 100,
            };
            if let Some(fill) = crate::flood_fill_mask(
                &snapshot,
                width,
                height,
                x,
                y,
                options,
                MAX_BRUSH_PIXEL_VISITS,
            ) {
                for row in 0..fill.height {
                    for column in 0..fill.width {
                        let px = fill.x0 + column;
                        let py = fill.y0 + row;
                        shape[py as usize * width as usize + px as usize] =
                            fill.coverage_at(px, py);
                    }
                }
            }
        } else {
            let seed_offset = (y as usize * width as usize + x as usize) * 4;
            let seed = Pixel::from_slice(&snapshot[seed_offset..seed_offset + 4]);
            for (index, pixel) in snapshot.chunks_exact(4).enumerate() {
                if crate::colour_difference(seed, Pixel::from_slice(pixel)) <= tolerance {
                    shape[index] = u8::MAX;
                }
            }
        }
        self.selection.apply_mask_shape(shape, mode);
        Ok(())
    }

    /// Intelligent scissors / magnetic selection: trace a boundary that snaps to the strongest edges
    /// through the given anchors, then select the polygon it encloses. Anchors are (x, y) in canvas
    /// pixels; the boundary is implicitly closed.
    pub(crate) fn select_scissors(
        &mut self,
        anchors: &[(u32, u32)],
        mode: crate::SelectionMode,
    ) -> Result<()> {
        if anchors.len() < 2 {
            return Ok(());
        }
        let width = self.width;
        let height = self.height;
        if anchors.iter().any(|&(x, y)| x >= width || y >= height) {
            return Err(CoreError::InvalidFilterParameter);
        }
        let snapshot = self.active_raster_copy()?;
        // Bound the per-segment search so a huge canvas cannot make one trace unbounded. The constant
        // is u64 so it means the same on a 32-bit target; the search counts in usize.
        let budget = usize::try_from(MAX_BRUSH_PIXEL_VISITS).unwrap_or(usize::MAX);
        let polygon = crate::scissors::magnetic_boundary(&snapshot, width, height, anchors, budget);
        self.selection.apply_polygon(&polygon, mode);
        Ok(())
    }

    /// Seamless clone: copy a rectangle of the active layer to another place with its boundary
    /// made to disappear (L.6).
    ///
    /// **An adaptation, recorded as one**: upstream's patch comes from the CLIPBOARD
    /// (`gimpclipboard.h` is included by the tool and `sc->paste` is a `GimpBuffer`), and this
    /// command model has no clipboard, so the patch is named as a rectangle of the layer instead.
    /// The construction is untouched by that — it needs a patch and a place to put it.
    ///
    /// The correction is the boundary mismatch interpolated across the interior with mean-value
    /// coordinates. See `crate::seamless_clone` for why that, and not a Poisson solve, is what the
    /// readable parameter describes.
    pub(crate) fn seamless_clone(
        &mut self,
        src: Rect,
        dst_x: i32,
        dst_y: i32,
        max_refine_scale: u32,
    ) -> Result<()> {
        use crate::seamless_clone as sc;

        if max_refine_scale > sc::SEAMLESS_CLONE_MAX_REFINE_SCALE {
            return Err(CoreError::InvalidFilterParameter);
        }
        // A region narrower than three pixels has no interior to correct: every pixel is boundary.
        if src.width < 3 || src.height < 3 {
            return Err(CoreError::InvalidFilterParameter);
        }
        let width = self.width;
        let height = self.height;
        let inside = |x: i64, y: i64, w: u32, h: u32| -> bool {
            x >= 0
                && y >= 0
                && x + i64::from(w) <= i64::from(width)
                && y + i64::from(h) <= i64::from(height)
        };
        if !inside(i64::from(src.x), i64::from(src.y), src.width, src.height)
            || !inside(i64::from(dst_x), i64::from(dst_y), src.width, src.height)
        {
            return Err(CoreError::InvalidFilterParameter);
        }

        let per_edge = sc::samples_per_edge(max_refine_scale);
        let interior = u64::from(src.width - 2) * u64::from(src.height - 2);
        sc::check_budget(interior, u64::from(per_edge) * 4, MAX_BRUSH_PIXEL_VISITS)?;

        self.prepare_active_raster_edit()?;
        let original = self.active_raster_pixels()?.to_vec();
        let read = |x: u32, y: u32| -> Pixel {
            let o = (y as usize * width as usize + x as usize) * 4;
            Pixel {
                r: original[o],
                g: original[o + 1],
                b: original[o + 2],
                a: original[o + 3],
            }
        };

        // The boundary walk is in DESTINATION coordinates; the matching patch pixel is the same
        // point shifted back by the offset, so one walk serves both images.
        let shift_x = f64::from(dst_x) - f64::from(src.x);
        let shift_y = f64::from(dst_y) - f64::from(src.y);
        let polygon = sc::boundary_polygon(
            f64::from(dst_x),
            f64::from(dst_y),
            f64::from(src.width),
            f64::from(src.height),
            per_edge,
        );
        let mismatch: Vec<[f64; 3]> = polygon
            .iter()
            .map(|&(bx, by)| {
                let dx = (bx.round() as i64).clamp(0, i64::from(width) - 1) as u32;
                let dy = (by.round() as i64).clamp(0, i64::from(height) - 1) as u32;
                let px = ((bx - shift_x).round() as i64).clamp(0, i64::from(width) - 1) as u32;
                let py = ((by - shift_y).round() as i64).clamp(0, i64::from(height) - 1) as u32;
                sc::boundary_mismatch(read(dx, dy), read(px, py))
            })
            .collect();

        let mut output = original.clone();
        for row in 0..src.height {
            for col in 0..src.width {
                let dx = dst_x as u32 + col;
                let dy = dst_y as u32 + row;
                let patch = read(src.x as u32 + col, src.y as u32 + row);
                let point = (f64::from(dx), f64::from(dy));
                // Mean-value weights over the boundary, the same solve the cage transform uses.
                // `None` means the point is degenerate against this polygon; the patch then goes
                // down uncorrected rather than being skipped, so the region is never left with a
                // hole in it.
                let corrected = match mean_value_coords(point, &polygon) {
                    Some(weights) => {
                        let mut offset = [0.0_f64; 3];
                        for (w, m) in weights.iter().zip(mismatch.iter()) {
                            offset[0] += w * m[0];
                            offset[1] += w * m[1];
                            offset[2] += w * m[2];
                        }
                        Pixel {
                            r: (f64::from(patch.r) + offset[0]).round().clamp(0.0, 255.0) as u8,
                            g: (f64::from(patch.g) + offset[1]).round().clamp(0.0, 255.0) as u8,
                            b: (f64::from(patch.b) + offset[2]).round().clamp(0.0, 255.0) as u8,
                            a: patch.a,
                        }
                    }
                    None => patch,
                };
                let o = (dy as usize * width as usize + dx as usize) * 4;
                corrected.write_to(&mut output[o..o + 4]);
            }
        }
        self.replace_active_pixels(output)
    }

    /// Paint select: rough strokes REFINE the existing selection (L.5).
    ///
    /// Distinct from [`Self::select_foreground`], which is the neighbouring tool: that one takes
    /// foreground and background scribbles together and classifies every pixel by colour, ignoring
    /// whatever is already selected. This one carries ONE label per stroke and works against the
    /// selection as it stands — upstream resets its trimap to grey on every button press and
    /// latches the operation at that moment, so a single stroke can only write one value.
    ///
    /// `mode` decides that value: `Add` scribbles the object, anything else scribbles the
    /// background, which is upstream's `painting_op == GIMP_CHANNEL_OP_ADD ? 1.f : 0.f`.
    pub(crate) fn paint_select(
        &mut self,
        scribbles: &[(u32, u32)],
        stroke_width: u32,
        mode: crate::SelectionMode,
    ) -> Result<()> {
        use crate::paint_select as ps;

        if scribbles.is_empty() {
            return Ok(());
        }
        if !(ps::PAINT_SELECT_MIN_STROKE_WIDTH..=ps::PAINT_SELECT_MAX_STROKE_WIDTH)
            .contains(&stroke_width)
        {
            return Err(CoreError::InvalidFilterParameter);
        }
        let width = self.width;
        let height = self.height;
        if scribbles.iter().any(|&(x, y)| x >= width || y >= height) {
            return Err(CoreError::InvalidFilterParameter);
        }

        let growing = mode == crate::SelectionMode::Add;
        // The asymmetric `gegl:threshold` on the existing selection: 0.99 growing, 0.01 otherwise.
        let cut = if growing {
            ps::PAINT_SELECT_ADD_MASK_CUT
        } else {
            ps::PAINT_SELECT_REMOVE_MASK_CUT
        };
        let count = pixel_count(width, height)?;
        // An INACTIVE selection is read as an empty mask here, not as "everything selected".
        // Our convention is that an inactive selection means the whole canvas, which is right for
        // editing; upstream's selection channel is genuinely all-zero before anything is selected,
        // and this operation reads the MASK rather than asking what is editable. Treating an
        // inactive selection as full would make a growing stroke seed from every pixel, so the
        // whole canvas would come back selected regardless of where the stroke went.
        let selection_active = self.selection.is_active();
        let above_cut: Vec<bool> = (0..count)
            .map(|i| {
                if !selection_active {
                    return false;
                }
                let x = (i % width as usize) as u32;
                let y = (i / width as usize) as u32;
                f32::from(self.selection.coverage(x, y)) / 255.0 >= cut
            })
            .collect();

        let scribble = ps::scribble_mask(width, height, scribbles, stroke_width);
        let snapshot = self.active_raster_pixels()?.to_vec();
        let budget = usize::try_from(MAX_BRUSH_PIXEL_VISITS).unwrap_or(usize::MAX);

        // Growing: the thresholded CORE joins the stroke as an object seed, because a pixel already
        // confidently selected belongs to the object. Shrinking: the thresholded EXTENT is a bound,
        // so everything outside it is ruled out of the region being removed. That is what the two
        // different cuts are for.
        let (object, excluded) = if growing {
            let object: Vec<bool> = scribble
                .iter()
                .zip(above_cut.iter())
                .map(|(&a, &b)| a || b)
                .collect();
            (object, vec![false; count])
        } else {
            let excluded: Vec<bool> = above_cut.iter().map(|&inside| !inside).collect();
            (scribble, excluded)
        };

        let region = ps::region(&snapshot, width, height, &object, &excluded, budget);
        // Upstream hands the operation's output to `gimp_channel_select_buffer` with `painting_op`,
        // so the region is COMBINED with the selection rather than replacing it.
        self.selection.apply_mask_shape(region, mode);
        Ok(())
    }

    /// Foreground select: the user scribbles over the subject (fg) and the background (bg); every
    /// pixel is labelled by whether its colour is closer to the foreground samples or the background
    /// samples (nearest-sample classification in the same Lab metric the wand uses). Pixels nearer
    /// the foreground are selected. A coverage ramp near the decision boundary softens the edge.
    pub(crate) fn select_foreground(
        &mut self,
        fg: &[(u32, u32)],
        bg: &[(u32, u32)],
        mode: crate::SelectionMode,
    ) -> Result<()> {
        if fg.is_empty() {
            return Ok(());
        }
        let width = self.width;
        let height = self.height;
        if fg.iter().chain(bg).any(|&(x, y)| x >= width || y >= height) {
            return Err(CoreError::InvalidFilterParameter);
        }
        let snapshot = self.active_raster_copy()?;
        let sample = |marks: &[(u32, u32)]| -> Vec<Pixel> {
            marks
                .iter()
                .map(|&(x, y)| {
                    let o = (y as usize * width as usize + x as usize) * 4;
                    Pixel::from_slice(&snapshot[o..o + 4])
                })
                .collect()
        };
        let fg_colors = sample(fg);
        let bg_colors = sample(bg);
        let nearest = |p: Pixel, set: &[Pixel]| -> u16 {
            set.iter()
                .map(|&c| u16::from(crate::colour_difference(p, c)))
                .min()
                .unwrap_or(u16::MAX)
        };
        let mut shape = vec![0_u8; (width as usize) * (height as usize)];
        for (index, chunk) in snapshot.chunks_exact(4).enumerate() {
            let p = Pixel::from_slice(chunk);
            let df = nearest(p, &fg_colors);
            let db = if bg_colors.is_empty() {
                // No background marks: select whatever is within a generous distance of the fg.
                64
            } else {
                nearest(p, &bg_colors)
            };
            // Foreground wins when it is closer. A soft ramp around the tie makes the edge not a hard
            // 1-pixel step: coverage = clamp((db - df) scaled, 0..1).
            let diff = f32::from(db) - f32::from(df);
            let coverage = ((diff / 16.0) * 0.5 + 0.5).clamp(0.0, 1.0);
            shape[index] = (coverage * 255.0).round() as u8;
        }
        self.selection.apply_mask_shape(shape, mode);
        Ok(())
    }

    pub(crate) fn select_all(&mut self) {
        self.selection.select_all();
    }

    pub(crate) fn invert_selection(&mut self) {
        self.selection.invert();
    }

    pub(crate) fn feather_selection(&mut self, radius: u32) -> Result<()> {
        self.selection.feather(radius)
    }

    pub(crate) fn grow_selection(&mut self, radius: u32) -> Result<()> {
        self.selection.grow(radius)
    }

    pub(crate) fn shrink_selection(&mut self, radius: u32) -> Result<()> {
        self.selection.shrink(radius)
    }

    pub(crate) fn gradient_fill(
        &mut self,
        kind: GradientKind,
        stops: &[GradientStop],
    ) -> Result<()> {
        validate_gradient(kind, stops)?;
        let selection = self.selection.clone();
        let width = self.width;
        let pixels = self.active_raster_pixels_mut()?;
        for (index, destination) in pixels.chunks_exact_mut(4).enumerate() {
            let x = index as u32 % width;
            let y = index as u32 / width;
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let position = match kind {
                GradientKind::Linear {
                    start_x,
                    start_y,
                    end_x,
                    end_y,
                } => {
                    let dx = end_x - start_x;
                    let dy = end_y - start_y;
                    ((px - start_x) * dx + (py - start_y) * dy) / (dx * dx + dy * dy)
                }
                GradientKind::Radial {
                    center_x,
                    center_y,
                    radius,
                } => (px - center_x).hypot(py - center_y) / radius,
            }
            .clamp(0.0, 1.0);
            let mut source = interpolate_gradient(stops, position);
            source.a =
                ((u16::from(source.a) * u16::from(selection.coverage(x, y)) + 127) / 255) as u8;
            if source.a != 0 {
                source_over(Pixel::from_slice(destination), source).write_to(destination);
            }
        }
        Ok(())
    }

    /// Fills the contiguous region of similar colour around a seed, through the selection.
    ///
    /// The mask is computed from a snapshot of the layer BEFORE anything is written. Reading the region
    /// while filling it would let already-filled pixels answer the colour test -- so a fill whose new
    /// colour is within tolerance of the old one would spread across the whole layer, and one whose colour
    /// is outside it would not. Neither is what a bucket tool does.
    pub(crate) fn flood_fill_active(
        &mut self,
        x: u32,
        y: u32,
        color: Pixel,
        options: crate::FloodFillOptions,
    ) -> Result<()> {
        if !options.is_valid() {
            return Err(CoreError::InvalidFilterParameter);
        }
        let width = self.width;
        let height = self.height;
        if x >= width || y >= height {
            return Err(CoreError::InvalidFilterParameter);
        }
        let mask = self.selection.clone();
        let snapshot = self.active_raster_pixels_mut()?.to_vec();
        let fill = crate::flood_fill_mask(
            &snapshot,
            width,
            height,
            x,
            y,
            options,
            MAX_BRUSH_PIXEL_VISITS,
        );
        // A seed that cannot be filled is not an error -- clicking a pixel the tolerance excludes is an
        // ordinary thing to do, and it leaves the layer alone.
        let Some(fill) = fill else {
            return Ok(());
        };

        let pixels = self.active_raster_pixels_mut()?;
        for row in 0..fill.height {
            for column in 0..fill.width {
                let px = fill.x0 + column;
                let py = fill.y0 + row;
                let coverage = fill.coverage_at(px, py);
                if coverage == 0 {
                    continue;
                }
                // The selection gates the fill, exactly as it gates `fill_active`.
                let selected = mask.coverage(px, py);
                if selected == 0 {
                    continue;
                }
                let combined = (u16::from(coverage) * u16::from(selected) + 127) / 255;
                let mut source = color;
                source.a = ((u16::from(source.a) * combined + 127) / 255) as u8;
                if source.a == 0 {
                    continue;
                }
                let offset = ((py as usize * width as usize) + px as usize) * 4;
                let slice = &mut pixels[offset..offset + 4];
                source_over(Pixel::from_slice(slice), source).write_to(slice);
            }
        }
        Ok(())
    }

    /// Enclose-and-fill (Krita): within a rectangle, fill every region that is CLOSED OFF from the
    /// rectangle's border by existing opaque pixels. Re-derived from Krita's enclose-and-fill tool —
    /// a flood from the rectangle edge marks everything reachable through transparent pixels; the
    /// transparent pixels it could NOT reach are enclosed by a drawn boundary, and those get `color`.
    pub(crate) fn enclose_and_fill(
        &mut self,
        rect: Rect,
        color: Pixel,
        alpha_threshold: u8,
    ) -> Result<()> {
        let width = self.width;
        let height = self.height;
        // A rect's origin is SIGNED — it may start off-canvas to the left or above — while the canvas
        // extent is unsigned. Clamp into canvas space before any arithmetic: a negative origin becomes
        // 0 and the part that hung off the canvas is simply not covered. Widened to i64 first, because
        // an i32 origin plus a u32 width overflows i32 on its own.
        let x0 = rect.x.clamp(0, width as i32) as u32;
        let y0 = rect.y.clamp(0, height as i32) as u32;
        let x1 = (i64::from(rect.x) + i64::from(rect.width)).clamp(0, i64::from(width)) as u32;
        let y1 = (i64::from(rect.y) + i64::from(rect.height)).clamp(0, i64::from(height)) as u32;
        if x1 <= x0 || y1 <= y0 {
            return Ok(());
        }
        let rw = (x1 - x0) as usize;
        let rh = (y1 - y0) as usize;
        let snapshot = self.active_raster_pixels()?.to_vec();
        // `open[i]` = this cell is transparent AND reachable from the rectangle border through other
        // transparent cells. Flood-fill from the border inward.
        let is_transparent = |rx: usize, ry: usize| -> bool {
            let px = x0 as usize + rx;
            let py = y0 as usize + ry;
            snapshot[(py * width as usize + px) * 4 + 3] <= alpha_threshold
        };
        let mut reached = vec![false; rw * rh];
        let mut stack: Vec<(usize, usize)> = Vec::new();
        for rx in 0..rw {
            for &ry in &[0usize, rh - 1] {
                if ry < rh && is_transparent(rx, ry) && !reached[ry * rw + rx] {
                    reached[ry * rw + rx] = true;
                    stack.push((rx, ry));
                }
            }
        }
        for ry in 0..rh {
            for &rx in &[0usize, rw - 1] {
                if rx < rw && is_transparent(rx, ry) && !reached[ry * rw + rx] {
                    reached[ry * rw + rx] = true;
                    stack.push((rx, ry));
                }
            }
        }
        while let Some((rx, ry)) = stack.pop() {
            let neighbours = [
                (rx.wrapping_sub(1), ry),
                (rx + 1, ry),
                (rx, ry.wrapping_sub(1)),
                (rx, ry + 1),
            ];
            for &(nx, ny) in &neighbours {
                if nx < rw && ny < rh && !reached[ny * rw + nx] && is_transparent(nx, ny) {
                    reached[ny * rw + nx] = true;
                    stack.push((nx, ny));
                }
            }
        }
        // Fill the transparent-but-unreached cells (the enclosed interiors).
        let mask = self.selection.clone();
        let pixels = self.active_raster_pixels_mut()?;
        for ry in 0..rh {
            for rx in 0..rw {
                if reached[ry * rw + rx] {
                    continue;
                }
                let px = x0 as usize + rx;
                let py = y0 as usize + ry;
                if snapshot[(py * width as usize + px) * 4 + 3] > alpha_threshold {
                    continue; // an opaque boundary pixel, not an interior
                }
                let coverage = mask.coverage(px as u32, py as u32);
                if coverage == 0 {
                    continue;
                }
                let mut source = color;
                source.a = ((u16::from(source.a) * u16::from(coverage) + 127) / 255) as u8;
                let offset = (py * width as usize + px) * 4;
                let slice = &mut pixels[offset..offset + 4];
                source_over(Pixel::from_slice(slice), source).write_to(slice);
            }
        }
        Ok(())
    }

    /// M10 (Edit > Content-Aware Fill, Shift+F5): fills the selection with texture from around it
    /// by PatchMatch (`content_fill`), starting from the smart-patch average. Partly selected
    /// pixels blend between the original and the fill by their coverage, so a feathered selection
    /// gives a soft seam. 8-bit documents only.
    pub(crate) fn content_aware_fill(&mut self) -> Result<()> {
        if self.precision != crate::precision::Precision::U8 {
            return Err(CoreError::FilterPrecisionUnsupported("content_aware_fill"));
        }
        if !self.selection.is_active() {
            return Err(CoreError::NoSelection);
        }
        let original = self.active_raster_copy()?;
        let (w, h) = (self.width as usize, self.height as usize);
        let coverage = self.selection.mask().to_vec();
        let hole: Vec<bool> = coverage.iter().map(|c| *c > 0).collect();
        // A rough guess first, which also runs the lock checks on the active layer.
        self.smart_patch(32)?;
        let mut work = self.active_raster_copy()?;
        crate::content_fill::fill(&mut work, w, h, &hole, 3, 6)?;
        let pixels = self.active_raster_pixels_mut()?;
        for (index, c) in coverage.iter().enumerate() {
            if *c == 0 {
                continue;
            }
            let k = u32::from(*c);
            for channel in 0..4 {
                let at = index * 4 + channel;
                let (old, new) = (u32::from(original[at]), u32::from(work[at]));
                pixels[at] = ((old * (255 - k) + new * k + 127) / 255) as u8;
            }
        }
        Ok(())
    }

    /// Smart patch (Krita): content-aware fill of the current selection. Re-derived from Krita's
    /// smart-patch tool as a lightweight exemplar inpaint — each selected (hole) pixel is filled from
    /// the nearest UNselected pixels found by marching outward along eight directions, averaged with a
    /// distance weight. It covers a blemish with surrounding content rather than a flat colour. Not the
    /// full PatchMatch, but the same intent and far shorter.
    pub(crate) fn smart_patch(&mut self, search_radius: u32) -> Result<()> {
        let width = self.width;
        let height = self.height;
        let radius = search_radius.clamp(1, 256) as i64;
        let mask = self.selection.clone();
        let snapshot = self.active_raster_pixels()?.to_vec();
        let directions: [(i64, i64); 8] = [
            (1, 0),
            (-1, 0),
            (0, 1),
            (0, -1),
            (1, 1),
            (1, -1),
            (-1, 1),
            (-1, -1),
        ];
        let sample = |x: i64, y: i64| -> Option<[u8; 4]> {
            if x < 0 || y < 0 || x >= i64::from(width) || y >= i64::from(height) {
                return None;
            }
            // Only sample pixels OUTSIDE the hole.
            if mask.coverage(x as u32, y as u32) != 0 {
                return None;
            }
            let o = ((y as usize) * width as usize + x as usize) * 4;
            Some([
                snapshot[o],
                snapshot[o + 1],
                snapshot[o + 2],
                snapshot[o + 3],
            ])
        };
        let pixels = self.active_raster_pixels_mut()?;
        for y in 0..height {
            for x in 0..width {
                if mask.coverage(x, y) == 0 {
                    continue;
                }
                let (mut acc, mut weight) = ([0.0f64; 4], 0.0f64);
                for &(dx, dy) in &directions {
                    // March outward until the first non-hole pixel along this direction.
                    let mut step = 1i64;
                    while step <= radius {
                        if let Some(rgba) =
                            sample(i64::from(x) + dx * step, i64::from(y) + dy * step)
                        {
                            let w = 1.0 / step as f64;
                            for c in 0..4 {
                                acc[c] += f64::from(rgba[c]) * w;
                            }
                            weight += w;
                            break;
                        }
                        step += 1;
                    }
                }
                if weight <= 0.0 {
                    continue;
                }
                let offset = (y as usize * width as usize + x as usize) * 4;
                for c in 0..4 {
                    pixels[offset + c] = (acc[c] / weight).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        Ok(())
    }

    /// Lazybrush (Krita): colour whole regions from a few colour scribbles. Re-derived from Krita's
    /// lazybrush — a multi-source shortest-path flood where moving across a strong luma edge is
    /// expensive, so each pixel takes the colour of the scribble it can reach most cheaply, and the
    /// paint stops at line art. `scribbles` are (x, y, colour) seeds. The result is composited onto the
    /// active layer where the selection allows.
    pub(crate) fn lazybrush(&mut self, scribbles: &[(u32, u32, Pixel)]) -> Result<()> {
        if scribbles.is_empty() {
            return Ok(());
        }
        let width = self.width as usize;
        let height = self.height as usize;
        let n = width * height;
        let snapshot = self.active_raster_pixels()?.to_vec();
        // Luma gradient magnitude, 0..1, as the edge cost (same idea as the scissors cost map).
        let luma = |i: usize| -> f64 {
            let o = i * 4;
            0.299 * f64::from(snapshot[o])
                + 0.587 * f64::from(snapshot[o + 1])
                + 0.114 * f64::from(snapshot[o + 2])
        };
        let edge = |x: usize, y: usize| -> f64 {
            let xm = x.saturating_sub(1);
            let xp = (x + 1).min(width - 1);
            let ym = y.saturating_sub(1);
            let yp = (y + 1).min(height - 1);
            let gx = (luma(y * width + xp) - luma(y * width + xm)).abs();
            let gy = (luma(yp * width + x) - luma(ym * width + x)).abs();
            (gx + gy) / 510.0
        };
        // Multi-source Dijkstra: cost of entering a cell = 0.01 + edge(cell)^2 * 8 (crossing a sharp
        // line is dear). Each cell remembers which seed's colour reached it cheapest.
        let mut dist = vec![f64::INFINITY; n];
        let mut owner = vec![usize::MAX; n];
        use std::cmp::Ordering;
        #[derive(PartialEq)]
        struct Node(f64, usize);
        impl Eq for Node {}
        impl PartialOrd for Node {
            fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                Some(self.cmp(other))
            }
        }
        impl Ord for Node {
            fn cmp(&self, other: &Self) -> Ordering {
                // Reversed so BinaryHeap behaves as a min-heap on distance.
                other.0.partial_cmp(&self.0).unwrap_or(Ordering::Equal)
            }
        }
        let mut heap = std::collections::BinaryHeap::new();
        for (seed, &(sx, sy, _)) in scribbles.iter().enumerate() {
            if sx as usize >= width || sy as usize >= height {
                continue;
            }
            let i = sy as usize * width + sx as usize;
            dist[i] = 0.0;
            owner[i] = seed;
            heap.push(Node(0.0, i));
        }
        let mut visits = 0usize;
        while let Some(Node(d, i)) = heap.pop() {
            if d > dist[i] {
                continue;
            }
            visits += 1;
            // Same widening reason as the scissors budget: the cap is u64, the counter is usize.
            if visits as u64 > MAX_BRUSH_PIXEL_VISITS.saturating_mul(4) {
                break;
            }
            let x = i % width;
            let y = i / width;
            let neighbours = [
                (x.wrapping_sub(1), y),
                (x + 1, y),
                (x, y.wrapping_sub(1)),
                (x, y + 1),
            ];
            for &(nx, ny) in &neighbours {
                if nx >= width || ny >= height {
                    continue;
                }
                let e = edge(nx, ny);
                let step = 0.01 + e * e * 8.0;
                let j = ny * width + nx;
                let nd = d + step;
                if nd < dist[j] {
                    dist[j] = nd;
                    owner[j] = owner[i];
                    heap.push(Node(nd, j));
                }
            }
        }
        let mask = self.selection.clone();
        let pixels = self.active_raster_pixels_mut()?;
        for i in 0..n {
            let seed = owner[i];
            if seed == usize::MAX {
                continue;
            }
            let x = (i % width) as u32;
            let y = (i / width) as u32;
            let coverage = mask.coverage(x, y);
            if coverage == 0 {
                continue;
            }
            let mut source = scribbles[seed].2;
            source.a = ((u16::from(source.a) * u16::from(coverage) + 127) / 255) as u8;
            let slice = &mut pixels[i * 4..i * 4 + 4];
            source_over(Pixel::from_slice(slice), source).write_to(slice);
        }
        Ok(())
    }

    pub(crate) fn fill_active(&mut self, color: Pixel) -> Result<()> {
        let mask = self.selection.clone();
        let width = self.width;
        let pixels = self.active_raster_pixels_mut()?;
        for (index, pixel) in pixels.chunks_exact_mut(4).enumerate() {
            let x = index as u32 % width;
            let y = index as u32 / width;
            let coverage = mask.coverage(x, y);
            if coverage != 0 {
                let mut source = color;
                source.a = ((u16::from(source.a) * u16::from(coverage) + 127) / 255) as u8;
                source_over(Pixel::from_slice(pixel), source).write_to(pixel);
            }
        }
        Ok(())
    }

    /// M5 (Edit > Stroke): paints a band `width` pixels wide along the selection's edge in `color`,
    /// inside it, centred on it or outside it, as Photoshop's Stroke dialog does. The band is
    /// measured with a chamfer distance (3-4 weights, so within a few percent of Euclidean) and
    /// gets a one-pixel soft edge. It is painted regardless of the selection, which only says
    /// where the edge is.
    pub(crate) fn stroke_selection(
        &mut self,
        width: f32,
        color: Pixel,
        location: StrokeLocation,
    ) -> Result<()> {
        if !self.selection.is_active() {
            return Err(CoreError::NoSelection);
        }
        if !width.is_finite() || width <= 0.0 || width > 1000.0 {
            return Err(CoreError::InvalidSemanticStyle);
        }
        let (w, h) = (self.width as usize, self.height as usize);
        let inside: Vec<bool> = self.selection.mask().iter().map(|c| *c >= 128).collect();
        // Distance (in pixels) from each pixel to the nearest pixel on the OTHER side of the edge.
        let to_other = chamfer(&inside, w, h);
        let (reach_in, reach_out) = match location {
            StrokeLocation::Inside => (width, 0.0),
            StrokeLocation::Center => (width / 2.0, width / 2.0),
            StrokeLocation::Outside => (0.0, width),
        };
        let pixels = self.active_raster_pixels_mut()?;
        for (index, pixel) in pixels.chunks_exact_mut(4).enumerate() {
            let reach = if inside[index] { reach_in } else { reach_out };
            if reach <= 0.0 {
                continue;
            }
            // A pixel next to the edge is at distance 1; it is fully inside a band of width >= 1.
            let coverage = (reach - to_other[index] + 1.0).clamp(0.0, 1.0);
            if coverage <= 0.0 {
                continue;
            }
            let mut source = color;
            source.a = (f32::from(source.a) * coverage).round() as u8;
            source_over(Pixel::from_slice(pixel), source).write_to(pixel);
        }
        Ok(())
    }

    pub(crate) fn clear_active(&mut self) -> Result<()> {
        let mask = self.selection.clone();
        let width = self.width;
        let pixels = self.active_raster_pixels_mut()?;
        for (index, pixel) in pixels.chunks_exact_mut(4).enumerate() {
            let x = index as u32 % width;
            let y = index as u32 / width;
            let coverage = mask.coverage(x, y);
            if coverage == u8::MAX {
                pixel.fill(0);
            } else if coverage != 0 {
                let keep = 255 - u16::from(coverage);
                for channel in pixel {
                    *channel = ((u16::from(*channel) * keep + 127) / 255) as u8;
                }
            }
        }
        Ok(())
    }

    /// Paints a stroke and returns the region it damaged, for the renderer to bound its recomposite to.
    // A stroke's inputs are already grouped where grouping is meaningful (BrushSettings, the tip,
    // the pipe); the rest are the primitive stroke parameters and bundling them would only add a
    // struct that exists to satisfy a count.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn brush_stroke(
        &mut self,
        points: &[BrushPoint],
        color: Pixel,
        size: f32,
        opacity: f32,
        settings: &BrushSettings,
        tip: Option<&crate::BrushTip>,
        pipe: &[crate::BrushTip],
    ) -> Result<Rect> {
        let plan = self.plan_brush_stroke(points, color, size, opacity, settings, tip, pipe)?;
        self.paint_brush_plan(&plan)
    }

    /// Validates a stroke and resolves it to dabs and the exact region they will touch, without
    /// writing a pixel.
    // Same parameter list as `brush_stroke`, deliberately: the two must stay callable with the
    // identical arguments or the plan would not describe what the paint does.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn plan_brush_stroke<'t>(
        &self,
        points: &[BrushPoint],
        color: Pixel,
        size: f32,
        opacity: f32,
        settings: &BrushSettings,
        tip: Option<&'t crate::BrushTip>,
        pipe: &'t [crate::BrushTip],
    ) -> Result<BrushPlan<'t>> {
        // A tip arrives from a serialised command as well as from a file, so its declared dimensions and
        // its coverage length must be checked to agree before anything indexes it.
        if tip.is_some_and(|tip| !tip.is_valid()) || pipe.iter().any(|t| !t.is_valid()) {
            return Err(CoreError::InvalidBrushSettings);
        }
        // GIH pipe frames, in stamp order: the single tip first (if any), then the pipe. Empty means
        // the generated dab is used (frames is None in the plan).
        let mut frames: Vec<&'t crate::BrushTip> = Vec::new();
        if let Some(t) = tip {
            frames.push(t);
        }
        frames.extend(pipe.iter());
        if points.is_empty() || points.len() > MAX_BRUSH_POINTS {
            return Err(CoreError::InvalidBrushPointCount {
                actual: points.len(),
                max: MAX_BRUSH_POINTS,
            });
        }
        if !size.is_finite() || size <= 0.0 || size > MAX_BRUSH_SIZE {
            return Err(CoreError::InvalidBrushSize);
        }
        if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
            return Err(CoreError::InvalidOpacity);
        }
        validate_brush_settings(settings)?;
        for point in points {
            if !point.x.is_finite()
                || !point.y.is_finite()
                || !point.pressure.is_finite()
                || !(0.0..=1.0).contains(&point.pressure)
            {
                return Err(CoreError::InvalidPressure);
            }
            // P7. Tilt is an angle a tablet reports; anything outside a right angle is not one.
            if !point.tilt_x.is_finite()
                || !point.tilt_y.is_finite()
                || !(-90.0..=90.0).contains(&point.tilt_x)
                || !(-90.0..=90.0).contains(&point.tilt_y)
            {
                return Err(CoreError::InvalidPressure);
            }
            if settings.mirror_x.is_some_and(|axis| {
                (f64::from(axis) * 2.0 - f64::from(point.x)).abs() > f64::from(f32::MAX)
            }) || settings.mirror_y.is_some_and(|axis| {
                (f64::from(axis) * 2.0 - f64::from(point.y)).abs() > f64::from(f32::MAX)
            }) {
                return Err(CoreError::InvalidBrushSettings);
            }
        }
        let mut processed = smooth_points(points, settings.smoothing);
        // Drawing assistant (C.15): snap every point onto the guide before anything else uses the
        // path, so dab placement, spacing and mirroring all follow the snapped line. The anchor is the
        // first point, which the vanishing-point assistant needs to choose its ray.
        if let Some(assistant) = settings.assistant
            && let Some(&first) = processed.first()
        {
            let anchor = (first.x, first.y);
            for point in processed.iter_mut() {
                let (sx, sy) = assistant.snap(point.x, point.y, anchor);
                *point = BrushPoint {
                    x: sx,
                    y: sy,
                    ..*point
                };
            }
        }
        // Dyna brush (C.16b): a mass-spring that lets the dab lag the cursor. The dab position chases
        // each input point through a spring (stiffness from 1-drag) against a mass, so the stroke
        // rounds its corners and overshoots. Pressure rides along unchanged.
        if let Some((mass, drag)) = settings.dyna
            && processed.len() >= 2
        {
            let mass = f64::from(mass).clamp(0.05, 1.0);
            let drag = f64::from(drag).clamp(0.0, 1.0);
            let first = processed[0];
            let (mut px, mut py) = (f64::from(first.x), f64::from(first.y));
            let (mut vx, mut vy) = (0.0_f64, 0.0_f64);
            // Spring pulls the dab toward the cursor; higher mass = more lag, higher drag = more
            // damping. Stiffness scaled so a light brush still tracks closely.
            let stiffness = 0.6 / mass;
            let damping = 1.0 - 0.5 * drag;
            let mut out = Vec::with_capacity(processed.len());
            out.push(first);
            for point in &processed[1..] {
                let tx = f64::from(point.x);
                let ty = f64::from(point.y);
                vx = (vx + (tx - px) * stiffness) * damping;
                vy = (vy + (ty - py) * stiffness) * damping;
                px += vx;
                py += vy;
                out.push(BrushPoint {
                    x: px as f32,
                    y: py as f32,
                    ..*point
                });
            }
            processed = out;
        }
        // Ink (GIMP): the nib thins as the pen moves faster. Scale each point's pressure down by the
        // local speed (distance to the previous point) so a quick stroke tapers. speed is normalised
        // against the brush size, so the response does not depend on the dab's pixel scale.
        if let Some(sensitivity) = settings.ink {
            let reference = size.max(1.0);
            for i in 0..processed.len() {
                let speed = if i == 0 {
                    0.0
                } else {
                    let dx = processed[i].x - processed[i - 1].x;
                    let dy = processed[i].y - processed[i - 1].y;
                    (dx * dx + dy * dy).sqrt() / reference
                };
                // A fast point (speed >= 1 brush-width per step) is scaled toward (1 - sensitivity).
                let factor = 1.0 - sensitivity * speed.min(1.0);
                processed[i].pressure = (processed[i].pressure * factor).clamp(0.0, 1.0);
            }
        }
        // Krita-style sensor bindings (B.10 for size, I.1 for opacity and flow). All three channels
        // read the SAME per-point sensor values, computed once here from the RAW pressure -- before the
        // size channel overwrites it. Reading the remapped pressure instead would make a size binding
        // silently change what an opacity binding sees, so asking for "bigger with pressure, but
        // uniformly opaque" would not be expressible.
        let dab_opacity_scale;
        let dab_flow_scale;
        if settings.dynamics.is_empty()
            && settings.opacity_dynamics.is_empty()
            && settings.flow_dynamics.is_empty()
        {
            dab_opacity_scale = Vec::new();
            dab_flow_scale = Vec::new();
        } else {
            let reference = size.max(1.0);
            let mut opacity_points = Vec::with_capacity(processed.len());
            let mut flow_points = Vec::with_capacity(processed.len());
            // Each channel's bindings sum their nudges about a 0.5-centred sensor, so a positive amount
            // raises the channel on above-mid readings and lowers it below mid.
            let combine = |bindings: &[crate::BrushDynamic],
                           pressure: f32,
                           speed: f32,
                           random: f32,
                           tilt: f32| {
                let mut delta = 0.0_f32;
                for d in bindings {
                    let sensor = match d.sensor {
                        crate::DynamicSensor::Pressure => pressure,
                        crate::DynamicSensor::Speed => speed,
                        crate::DynamicSensor::Random => random,
                        crate::DynamicSensor::Tilt => tilt,
                    };
                    delta += d.amount * (sensor - 0.5);
                }
                delta
            };
            for i in 0..processed.len() {
                let speed = if i == 0 {
                    0.0
                } else {
                    let dx = processed[i].x - processed[i - 1].x;
                    let dy = processed[i].y - processed[i - 1].y;
                    ((dx * dx + dy * dy).sqrt() / reference).min(1.0)
                };
                let mut h = ((processed[i].x.to_bits() as u64) << 32
                    ^ processed[i].y.to_bits() as u64)
                    .wrapping_mul(0x9E37_79B9_7F4A_7C15);
                h ^= h >> 29;
                let random = (h & 0xFFFF) as f32 / 65535.0;
                let base = processed[i].pressure;
                let tilt = processed[i].tilt_amount();
                // Opacity and flow start at 1.0 (no scaling) and are nudged from there, so an empty
                // list leaves the stroke exactly as it was before this feature existed.
                opacity_points.push(
                    (1.0 + combine(&settings.opacity_dynamics, base, speed, random, tilt))
                        .clamp(0.0, 1.0),
                );
                flow_points.push(
                    (1.0 + combine(&settings.flow_dynamics, base, speed, random, tilt))
                        .clamp(0.0, 1.0),
                );
                // Size last, because it is the one that overwrites the pressure the other two read.
                if !settings.dynamics.is_empty() {
                    let delta = combine(&settings.dynamics, base, speed, random, tilt);
                    processed[i].pressure = (base + delta).clamp(0.0, 1.0);
                }
            }
            dab_opacity_scale = opacity_points;
            dab_flow_scale = flow_points;
        }
        let paths = mirrored_paths(&processed, settings);
        let max_dabs = (pixel_count(self.width, self.height)?
            .saturating_mul(16)
            .saturating_add(points.len()))
        .min(MAX_BRUSH_DABS);
        let mut dabs = Vec::new();
        // Per-dab channel values, parallel to `dabs`. Empty when no channel binding was asked for, and
        // every reader treats empty as "no scaling" rather than as zero.
        let mut dab_opacity: Vec<f32> = Vec::new();
        let mut dab_flow: Vec<f32> = Vec::new();
        let has_channels = !dab_opacity_scale.is_empty();
        for path in paths {
            let channels = if has_channels {
                Some(DabChannels {
                    opacity_in: &dab_opacity_scale,
                    flow_in: &dab_flow_scale,
                    opacity_out: &mut dab_opacity,
                    flow_out: &mut dab_flow,
                })
            } else {
                None
            };
            append_dabs(
                &mut dabs,
                &path,
                size,
                settings.shape,
                settings.spacing,
                max_dabs,
                channels,
            )?;
        }
        // MyPaint-style scatter (B.9): replace each clean dab with several jittered sub-dabs, so the
        // stroke builds a grainy, textured line. Deterministic in the dab index, so a re-render is
        // identical. Capped at max_dabs like append_dabs.
        if let Some(mp) = settings.mypaint {
            let per = mp.dabs_per_step.clamp(1, 8) as usize;
            let hash = |n: u64| {
                let mut h = n.wrapping_mul(0x9E37_79B9_7F4A_7C15);
                h ^= h >> 29;
                h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
                h ^= h >> 32;
                // -1.0..=1.0
                (h & 0xFFFF) as f32 / 32767.5 - 1.0
            };
            let mut scattered = Vec::with_capacity((dabs.len() * per).min(max_dabs));
            // The channels are indexed by dab, so a scatter that multiplies the dabs must multiply them
            // too — every sub-dab inherits its parent's opacity and flow. Left out, the arrays would be
            // shorter than `dabs` and the whole channel would be dropped as a mismatch.
            let mut scattered_opacity = Vec::with_capacity(scattered.capacity());
            let mut scattered_flow = Vec::with_capacity(scattered.capacity());
            for (i, dab) in dabs.iter().enumerate() {
                for k in 0..per {
                    if scattered.len() >= max_dabs {
                        break;
                    }
                    let seed = (i as u64) << 8 | k as u64;
                    let ox = hash(seed) * mp.offset_jitter * size * 0.5;
                    let oy = hash(seed ^ 0xABCD) * mp.offset_jitter * size * 0.5;
                    // Radius via pressure: a sub-dab is 1 +/- radius_jitter of the dab's pressure.
                    let rj = 1.0 + hash(seed ^ 0x1234) * mp.radius_jitter;
                    scattered.push(BrushPoint::new(
                        dab.x + ox,
                        dab.y + oy,
                        (dab.pressure * rj).clamp(0.0, 1.0),
                    ));
                    if has_channels {
                        scattered_opacity.push(dab_opacity.get(i).copied().unwrap_or(1.0));
                        scattered_flow.push(dab_flow.get(i).copied().unwrap_or(1.0));
                    }
                }
            }
            dabs = scattered;
            if has_channels {
                dab_opacity = scattered_opacity;
                dab_flow = scattered_flow;
            }
        }
        preflight_brush_pixel_visits(&dabs, size, self.width, self.height)?;

        // The damaged region is accumulated from the dab rasters themselves rather than guessed from the
        // input points. Smoothing MOVES points and a Catmull-Rom segment can overshoot its control points,
        // so a box derived from the request would not reliably contain the dabs it produced -- and a damage
        // region that is too small renders a stale frame.
        //
        // Computed here, before any pixel is written, so the editor can keep only this region's previous
        // pixels for undo instead of the whole layer (see `Editor::execute_brush_stroke`).
        let mut damaged: Option<(u32, u32, u32, u32)> = None;
        for dab in &dabs {
            if dab.pressure <= 0.0 {
                continue;
            }
            let raster = brush_dab_raster(*dab, size, self.width, self.height);
            if raster.x0 < raster.x1 && raster.y0 < raster.y1 {
                damaged = Some(match damaged {
                    None => (raster.x0, raster.y0, raster.x1, raster.y1),
                    Some((x0, y0, x1, y1)) => (
                        x0.min(raster.x0),
                        y0.min(raster.y0),
                        x1.max(raster.x1),
                        y1.max(raster.y1),
                    ),
                });
            }
        }
        // A stroke whose every dab fell outside the canvas, or was fully transparent, damages nothing. That
        // is reported as an empty region rather than as the whole canvas, so it costs no recomposite.
        let damage = damaged.map_or(Rect::new(0, 0, 0, 0), |(x0, y0, x1, y1)| {
            Rect::new(x0 as i32, y0 as i32, x1 - x0, y1 - y0)
        });
        Ok(BrushPlan {
            dabs,
            dab_opacity,
            dab_flow,
            color,
            size,
            opacity,
            shape: settings.shape,
            tip,
            frames,
            erase: settings.erase,
            flow: settings.flow,
            smudge: settings.smudge,
            clone_offset: settings.clone_offset,
            clone_perspective: settings.clone_perspective,
            heal: settings.heal,
            convolve: settings.convolve,
            dodge_burn: settings.dodge_burn,
            dodge_range: settings.dodge_range,
            damage,
        })
    }

    /// Paints a planned stroke onto the active layer's current cel and returns the region it damaged.
    ///
    /// Every pixel this writes lies inside `plan.damage`; `Editor::execute_brush_stroke` relies on that
    /// to restore the region alone on undo.
    pub(crate) fn paint_brush_plan(&mut self, plan: &BrushPlan<'_>) -> Result<Rect> {
        // Quick mask (J.2b): while the mode is on, a stroke edits the MASK, not the image. Routed
        // here rather than at the command layer because every stroke arrives through this one
        // function — a check further out would have to be repeated for each paint command, and the
        // one that was forgotten would silently paint colour onto the layer behind the mask.
        if self.quick_mask.is_some() {
            self.paint_quick_mask(
                &plan.dabs,
                plan.color,
                plan.size,
                plan.opacity,
                plan.shape,
                plan.tip,
            )?;
            return Ok(plan.damage);
        }
        let mask = self.selection.clone();
        let width = self.width;
        let height = self.height;
        let (color, size, opacity, shape, tip, erase, flow) = (
            plan.color,
            plan.size,
            plan.opacity,
            plan.shape,
            plan.tip,
            plan.erase,
            plan.flow,
        );
        // Per-dab channel lookup (I.1). An absent entry means the binding was never asked for, so the
        // scale is 1.0 — NOT 0.0, which would silently erase the stroke.
        let dab_opacity_at = |i: usize| plan.dab_opacity.get(i).copied().unwrap_or(1.0);
        let dab_flow_at = |i: usize| plan.dab_flow.get(i).copied().unwrap_or(1.0);
        let pixels = self.active_raster_pixels_mut()?;
        if let Some(exposure) = plan.dodge_burn {
            // Dodge/Burn (GIMP gimpdodgeburn.c): lighten (exposure > 0) or darken (< 0) the pixels
            // under the dab in a tonal range. Range 0 shadows, 1 midtones, 2 highlights -- the factor
            // is weighted so a dodge of the highlights barely touches the shadows, and vice versa.
            let range = plan.dodge_range.unwrap_or(1);
            let dodge = exposure >= 0.0;
            let mag = exposure.abs();
            for &dab in &plan.dabs {
                if dab.pressure <= 0.0 {
                    continue;
                }
                let raster = brush_dab_raster(dab, size, width, height);
                let diameter = raster.radius * 2.0;
                let dab_mask = crate::DabMask::new(shape, diameter);
                for y in raster.y0..raster.y1 {
                    for x in raster.x0..raster.x1 {
                        let edge = match tip {
                            Some(tip) => tip.coverage_at(
                                x as f32 + 0.5 - dab.x,
                                y as f32 + 0.5 - dab.y,
                                diameter,
                            ),
                            None => {
                                dab_mask.coverage_at(x as f32 + 0.5 - dab.x, y as f32 + 0.5 - dab.y)
                            }
                        };
                        let k =
                            mag * dab.pressure * edge * (f32::from(mask.coverage(x, y)) / 255.0);
                        if k <= 0.0 {
                            continue;
                        }
                        let offset = ((y as usize * width as usize) + x as usize) * 4;
                        for c in 0..3 {
                            let v = f32::from(pixels[offset + c]) / 255.0;
                            // Tonal weight: how much this range cares about value v (GIMP's shadow /
                            // midtone / highlight transfer, approximated with a smooth window).
                            let weight = match range {
                                0 => (1.0 - v).powi(2),             // shadows: strongest at dark
                                2 => v.powi(2), // highlights: strongest at light
                                _ => 1.0 - (2.0 * v - 1.0).powi(2), // midtones: strongest at 0.5
                            };
                            let step = k * weight;
                            let out = if dodge {
                                v + (1.0 - v) * step
                            } else {
                                v - v * step
                            };
                            pixels[offset + c] = (out * 255.0).round().clamp(0.0, 255.0) as u8;
                        }
                    }
                }
            }
            return Ok(plan.damage);
        }
        if let Some(amount) = plan.convolve {
            // Convolve (GIMP gimpconvolve.c): the dab does not paint, it blurs or sharpens the pixels
            // it covers. amount < 0 blurs (move each pixel toward its 3x3 neighbourhood mean), > 0
            // sharpens (move away from it), scaled by the dab coverage. Read from a snapshot so one
            // dab's output is not the next sample's input within the same pass.
            let snapshot = pixels.to_vec();
            let at = |x: i32, y: i32, c: usize| -> f32 {
                let xi = x.clamp(0, width as i32 - 1) as usize;
                let yi = y.clamp(0, height as i32 - 1) as usize;
                f32::from(snapshot[(yi * width as usize + xi) * 4 + c])
            };
            for &dab in &plan.dabs {
                if dab.pressure <= 0.0 {
                    continue;
                }
                let raster = brush_dab_raster(dab, size, width, height);
                let diameter = raster.radius * 2.0;
                let dab_mask = crate::DabMask::new(shape, diameter);
                for y in raster.y0..raster.y1 {
                    for x in raster.x0..raster.x1 {
                        let edge = match tip {
                            Some(tip) => tip.coverage_at(
                                x as f32 + 0.5 - dab.x,
                                y as f32 + 0.5 - dab.y,
                                diameter,
                            ),
                            None => {
                                dab_mask.coverage_at(x as f32 + 0.5 - dab.x, y as f32 + 0.5 - dab.y)
                            }
                        };
                        let selection = f32::from(mask.coverage(x, y)) / 255.0;
                        let k = amount * dab.pressure * edge * selection;
                        if k == 0.0 {
                            continue;
                        }
                        let offset = ((y as usize * width as usize) + x as usize) * 4;
                        for c in 0..4 {
                            // 3x3 box mean of the neighbourhood from the snapshot.
                            let mut sum = 0.0;
                            for dy in -1..=1 {
                                for dx in -1..=1 {
                                    sum += at(x as i32 + dx, y as i32 + dy, c);
                                }
                            }
                            let mean = sum / 9.0;
                            let here = f32::from(snapshot[offset + c]);
                            // k<0 blur: toward mean; k>0 sharpen: away from mean.
                            let out = if k < 0.0 {
                                here + (mean - here) * (-k)
                            } else {
                                here + (here - mean) * k
                            };
                            pixels[offset + c] = out.round().clamp(0.0, 255.0) as u8;
                        }
                    }
                }
            }
            return Ok(plan.damage);
        }
        if let Some((off_x, off_y)) = plan.clone_offset {
            // Clone (GIMP gimpclone.c): each dab copies the layer from a source region offset from the
            // stroke, rather than painting the brush colour. The source is read from a snapshot taken
            // before painting, so a stroke dragged over its own source does not feed back on itself.
            // Perspective clone maps the source point through a 3x3 homography first.
            let snapshot = pixels.to_vec();
            let persp = plan.clone_perspective;
            let sample = |sx: f32, sy: f32| -> Option<[f32; 4]> {
                // Apply the homography if present; otherwise the point is used directly.
                let (mx, my) = match persp {
                    Some(m) => {
                        let w = m[6] * sx + m[7] * sy + m[8];
                        if w.abs() < 1e-6 {
                            return None;
                        }
                        (
                            (m[0] * sx + m[1] * sy + m[2]) / w,
                            (m[3] * sx + m[4] * sy + m[5]) / w,
                        )
                    }
                    None => (sx, sy),
                };
                let ix = mx.floor() as i32;
                let iy = my.floor() as i32;
                if ix < 0 || iy < 0 || ix >= width as i32 || iy >= height as i32 {
                    return None;
                }
                let o = (iy as usize * width as usize + ix as usize) * 4;
                Some([
                    f32::from(snapshot[o]),
                    f32::from(snapshot[o + 1]),
                    f32::from(snapshot[o + 2]),
                    f32::from(snapshot[o + 3]),
                ])
            };
            for (dab_index, &dab) in plan.dabs.iter().enumerate() {
                if dab.pressure <= 0.0 {
                    continue;
                }
                let raster = brush_dab_raster(dab, size, width, height);
                let diameter = raster.radius * 2.0;
                let dab_mask = crate::DabMask::new(shape, diameter);
                // Heal: shift the whole source patch so its mean colour matches the mean of the
                // destination pixels under the dab, before compositing. This transplants the source's
                // texture (its deviations from its own mean) onto the destination's local colour --
                // the essence of GIMP's heal, without the full Poisson solve.
                let heal_shift = if plan.heal {
                    let mut src_sum = [0.0_f64; 3];
                    let mut dst_sum = [0.0_f64; 3];
                    let mut count = 0.0_f64;
                    for y in raster.y0..raster.y1 {
                        for x in raster.x0..raster.x1 {
                            let Some(s) = sample(x as f32 - off_x, y as f32 - off_y) else {
                                continue;
                            };
                            let o = ((y as usize * width as usize) + x as usize) * 4;
                            for c in 0..3 {
                                src_sum[c] += f64::from(s[c]);
                                dst_sum[c] += f64::from(snapshot[o + c]);
                            }
                            count += 1.0;
                        }
                    }
                    (count > 0.0).then(|| {
                        [
                            ((dst_sum[0] - src_sum[0]) / count) as f32,
                            ((dst_sum[1] - src_sum[1]) / count) as f32,
                            ((dst_sum[2] - src_sum[2]) / count) as f32,
                        ]
                    })
                } else {
                    None
                };
                for y in raster.y0..raster.y1 {
                    for x in raster.x0..raster.x1 {
                        let edge = match tip {
                            Some(tip) => tip.coverage_at(
                                x as f32 + 0.5 - dab.x,
                                y as f32 + 0.5 - dab.y,
                                diameter,
                            ),
                            None => {
                                dab_mask.coverage_at(x as f32 + 0.5 - dab.x, y as f32 + 0.5 - dab.y)
                            }
                        };
                        let selection = f32::from(mask.coverage(x, y)) / 255.0;
                        let strength = opacity
                            * dab_opacity_at(dab_index)
                            * dab.pressure
                            * edge
                            * selection
                            * flow.unwrap_or(1.0)
                            * dab_flow_at(dab_index);
                        if strength <= 0.0 {
                            continue;
                        }
                        let Some(src) = sample(x as f32 - off_x, y as f32 - off_y) else {
                            continue;
                        };
                        let shift = heal_shift.unwrap_or([0.0; 3]);
                        let source = Pixel::rgba(
                            (src[0] + shift[0]).round().clamp(0.0, 255.0) as u8,
                            (src[1] + shift[1]).round().clamp(0.0, 255.0) as u8,
                            (src[2] + shift[2]).round().clamp(0.0, 255.0) as u8,
                            (src[3] * strength).round().clamp(0.0, 255.0) as u8,
                        );
                        let offset = ((y as usize * width as usize) + x as usize) * 4;
                        let pixel = &mut pixels[offset..offset + 4];
                        source_over(Pixel::from_slice(pixel), source).write_to(pixel);
                    }
                }
            }
            return Ok(plan.damage);
        }
        if let Some(rate) = plan.smudge {
            // Smudge (GIMP gimpsmudge.c): the dab does not stamp the brush colour, it drags the colour
            // already on the layer. A carried accumulator (seeded from the first dab's centre) blends
            // toward the pixel under each dab by `rate`, then is written back under the dab coverage.
            // Low rate smears a long way; rate 1 just stamps the sampled colour.
            let sample = |px: &[u8], cx: f32, cy: f32| -> [f32; 4] {
                let ix = (cx as i32).clamp(0, width as i32 - 1) as usize;
                let iy = (cy as i32).clamp(0, height as i32 - 1) as usize;
                let o = (iy * width as usize + ix) * 4;
                [
                    f32::from(px[o]),
                    f32::from(px[o + 1]),
                    f32::from(px[o + 2]),
                    f32::from(px[o + 3]),
                ]
            };
            let mut accum = plan
                .dabs
                .first()
                .map_or([0.0; 4], |d| sample(pixels, d.x, d.y));
            for (dab_index, &dab) in plan.dabs.iter().enumerate() {
                if dab.pressure <= 0.0 {
                    continue;
                }
                let here = sample(pixels, dab.x, dab.y);
                for c in 0..4 {
                    accum[c] = accum[c] * (1.0 - rate) + here[c] * rate;
                }
                let carried = Pixel::rgba(
                    accum[0].round().clamp(0.0, 255.0) as u8,
                    accum[1].round().clamp(0.0, 255.0) as u8,
                    accum[2].round().clamp(0.0, 255.0) as u8,
                    accum[3].round().clamp(0.0, 255.0) as u8,
                );
                let raster = brush_dab_raster(dab, size, width, height);
                let diameter = raster.radius * 2.0;
                let dab_mask = crate::DabMask::new(shape, diameter);
                for y in raster.y0..raster.y1 {
                    for x in raster.x0..raster.x1 {
                        let edge = match tip {
                            Some(tip) => tip.coverage_at(
                                x as f32 + 0.5 - dab.x,
                                y as f32 + 0.5 - dab.y,
                                diameter,
                            ),
                            None => {
                                dab_mask.coverage_at(x as f32 + 0.5 - dab.x, y as f32 + 0.5 - dab.y)
                            }
                        };
                        let selection = f32::from(mask.coverage(x, y)) / 255.0;
                        let strength = opacity
                            * dab_opacity_at(dab_index)
                            * dab.pressure
                            * edge
                            * selection
                            * flow.unwrap_or(1.0)
                            * dab_flow_at(dab_index);
                        if strength <= 0.0 {
                            continue;
                        }
                        let mut source = carried;
                        source.a =
                            (f32::from(carried.a) * strength).round().clamp(0.0, 255.0) as u8;
                        let offset = ((y as usize * width as usize) + x as usize) * 4;
                        let pixel = &mut pixels[offset..offset + 4];
                        source_over(Pixel::from_slice(pixel), source).write_to(pixel);
                    }
                }
            }
            return Ok(plan.damage);
        }
        for (dab_index, &dab) in plan.dabs.iter().enumerate() {
            if dab.pressure <= 0.0 {
                continue;
            }
            // GIH pipe: cycle through the frames, one per dab; otherwise the single tip (or none).
            let frame: Option<&crate::BrushTip> = if plan.frames.is_empty() {
                tip
            } else {
                Some(plan.frames[dab_index % plan.frames.len()])
            };
            let raster = brush_dab_raster(dab, size, width, height);
            // Per DAB, not per stroke, because the radius is scaled by pressure. Resolving the mask once
            // from `size` alone gave a light-pressure dab the centre of a full-size mask -- uniformly
            // solid instead of falling off over its own smaller extent. Caught because clippy reported
            // `BrushDabRaster::radius` as never read once its only reader was replaced.
            //
            // The cost is a handful of divisions per dab against a per-pixel loop over the dab's box.
            // Krita pays this differently, with a pyramid of pre-scaled masks; that is its own block of
            // the port and is not needed to make the shape correct.
            let diameter = raster.radius * 2.0;
            let dab_mask = crate::DabMask::new(shape, diameter);
            for y in raster.y0..raster.y1 {
                for x in raster.x0..raster.x1 {
                    // The shape decides coverage now. The previous fixed rule was
                    // `(radius + 0.5 - distance).clamp(0, 1)`: a one-pixel linear feather with no
                    // hardness control at all, which the default shape reproduces closely enough that
                    // documents drawn before this field existed reopen looking as they did.
                    let offset_x = x as f32 + 0.5 - dab.x;
                    let offset_y = y as f32 + 0.5 - dab.y;
                    // An image tip replaces the generated shape entirely rather than multiplying with
                    // it. Multiplying would make every loaded brush softer than the file says, and a tip
                    // already carries its own edge.
                    let edge = match frame {
                        Some(tip) => tip.coverage_at(offset_x, offset_y, diameter),
                        None => dab_mask.coverage_at(offset_x, offset_y),
                    };
                    let selection = f32::from(mask.coverage(x, y)) / 255.0;
                    // An eraser's strength does not depend on the colour it would have painted.
                    let paint_alpha = if erase {
                        1.0
                    } else {
                        f32::from(color.a) / 255.0
                    };
                    let alpha = paint_alpha
                        * opacity
                        * dab_opacity_at(dab_index)
                        * dab.pressure
                        * edge
                        * selection
                        * flow.unwrap_or(1.0)
                        * dab_flow_at(dab_index);
                    if alpha <= 0.0 {
                        continue;
                    }
                    if erase {
                        let offset = ((y as usize * width as usize) + x as usize) * 4;
                        let remaining = f32::from(pixels[offset + 3]) * (1.0 - alpha.min(1.0));
                        let remaining = remaining.round().clamp(0.0, 255.0) as u8;
                        if remaining == 0 {
                            pixels[offset..offset + 4].copy_from_slice(&[0, 0, 0, 0]);
                        } else {
                            pixels[offset + 3] = remaining;
                        }
                        continue;
                    }
                    let mut source = color;
                    source.a = (alpha * 255.0).round().clamp(0.0, 255.0) as u8;
                    let offset = ((y as usize * width as usize) + x as usize) * 4;
                    let pixel = &mut pixels[offset..offset + 4];
                    source_over(Pixel::from_slice(pixel), source).write_to(pixel);
                }
            }
        }
        Ok(plan.damage)
    }

    /// Whether the active layer already has a raster cel on the current frame. A stroke on a frame
    /// without one creates the cel, which is a structural change a pixel patch cannot undo.
    pub(crate) fn active_cel_exists(&self) -> bool {
        self.layer(self.active_layer)
            .is_some_and(|layer| layer.has_raster_cel(self.current_frame_id()))
    }

    /// Copies a rectangle of the active layer's current cel out as packed RGBA rows.
    pub(crate) fn copy_active_region(&self, rect: Rect) -> Result<Vec<u8>> {
        let layer = self
            .layer(self.active_layer)
            .ok_or(CoreError::LayerNotFound(self.active_layer))?;
        copy_region(
            layer.raster_pixels(self.current_frame_id())?,
            self.width,
            rect,
        )
    }

    /// Writes packed RGBA rows back into a rectangle of one layer's cel on one frame.
    ///
    /// Addressed by layer and frame rather than "active" and "current" because undo runs after the
    /// viewer may have moved to another frame, and the patch belongs to the cel it was taken from.
    pub(crate) fn write_region(
        &mut self,
        layer: LayerId,
        frame: FrameId,
        rect: Rect,
        bytes: &[u8],
    ) -> Result<()> {
        let width = self.width;
        let pixels = self.layer_mut(layer)?.raster_pixels_mut(frame)?;
        paste_region(pixels, width, rect, bytes)
    }

    fn preflight_canvas_raster_bytes(&self, target_pixels: usize) -> Result<()> {
        let mut rgba_cels = 0_u64;
        let mut masks = 1_u64; // Selection mask.
        for layer in &self.layers {
            if layer.mask.is_some() {
                masks = masks
                    .checked_add(1)
                    .ok_or(CoreError::DocumentLimitExceeded("stored raster bytes"))?;
            }
            if let Some(cels) = layer.content.raster_cels() {
                rgba_cels = rgba_cels
                    .checked_add(cels.len() as u64)
                    .ok_or(CoreError::DocumentLimitExceeded("stored raster bytes"))?;
            }
        }
        // M13: a cel is the document's precision wide, not four bytes, at 16- and 32-bit.
        let bytes_per_pixel = rgba_cels
            .checked_mul(self.precision.bytes_per_pixel() as u64)
            .and_then(|bytes| bytes.checked_add(masks))
            .ok_or(CoreError::DocumentLimitExceeded("stored raster bytes"))?;
        let target_bytes = (target_pixels as u64)
            .checked_mul(bytes_per_pixel)
            .ok_or(CoreError::DocumentLimitExceeded("stored raster bytes"))?;
        if target_bytes > MAX_STORED_RASTER_BYTES {
            return Err(CoreError::DocumentLimitExceeded("stored raster bytes"));
        }
        Ok(())
    }

    pub(crate) fn crop_canvas(&mut self, rect: Rect) -> Result<()> {
        let count = pixel_count(rect.width, rect.height)?;
        let bpp = self.precision.bytes_per_pixel();
        self.preflight_canvas_raster_bytes(count)?;
        let old_width = self.width;
        let old_height = self.height;
        for layer in &mut self.layers {
            if let Some(mask) = &mut layer.mask {
                mask.pixels =
                    crop_bytes(&mask.pixels, old_width, old_height, rect, 1, count).into();
            }
            if let NodeContent::Raster { cels } = &mut layer.content {
                for cel in cels {
                    cel.pixels =
                        crop_bytes(&cel.pixels, old_width, old_height, rect, bpp, count).into();
                }
            }
        }
        self.selection.crop(rect)?;
        self.width = rect.width;
        self.height = rect.height;
        Ok(())
    }

    pub(crate) fn resize_canvas(
        &mut self,
        width: u32,
        height: u32,
        sampling: SamplingMode,
    ) -> Result<()> {
        let count = pixel_count(width, height)?;
        self.preflight_canvas_raster_bytes(count)?;
        let old_width = self.width;
        let old_height = self.height;
        let precision = self.precision;
        for layer in &mut self.layers {
            if let Some(mask) = &mut layer.mask {
                let mut output = vec![0; count];
                for y in 0..height {
                    for x in 0..width {
                        let source_x =
                            (f64::from(x) + 0.5) * f64::from(old_width) / f64::from(width) - 0.5;
                        let source_y =
                            (f64::from(y) + 0.5) * f64::from(old_height) / f64::from(height) - 0.5;
                        output[y as usize * width as usize + x as usize] = match sampling {
                            SamplingMode::Nearest => {
                                let sx =
                                    source_x.round().clamp(0.0, f64::from(old_width - 1)) as usize;
                                let sy =
                                    source_y.round().clamp(0.0, f64::from(old_height - 1)) as usize;
                                mask.pixels[sy * old_width as usize + sx]
                            }
                            SamplingMode::Bilinear => crate::selection::sample_mask_for_raster(
                                &mask.pixels,
                                old_width,
                                old_height,
                                source_x,
                                source_y,
                            ),
                        };
                    }
                }
                mask.pixels = output.into();
            }
            if let NodeContent::Raster { cels } = &mut layer.content {
                for cel in cels {
                    if precision != crate::precision::Precision::U8 {
                        cel.pixels = resample_deep(&cel.pixels, precision, old_width, old_height, width, height, sampling).into();
                        continue;
                    }
                    let mut output = vec![0; count * 4];
                    for y in 0..height {
                        for x in 0..width {
                            let source_x = (f64::from(x) + 0.5) * f64::from(old_width)
                                / f64::from(width)
                                - 0.5;
                            let source_y = (f64::from(y) + 0.5) * f64::from(old_height)
                                / f64::from(height)
                                - 0.5;
                            let sampled = sample_rgba(
                                &cel.pixels,
                                old_width,
                                old_height,
                                source_x,
                                source_y,
                                sampling,
                                true,
                            );
                            let offset = (y as usize * width as usize + x as usize) * 4;
                            sampled.write_to(&mut output[offset..offset + 4]);
                        }
                    }
                    cel.pixels = output.into();
                }
            }
        }
        self.selection.resize(width, height, sampling)?;
        self.width = width;
        self.height = height;
        Ok(())
    }

    /// The opaque bounding box of a layer's current cel, or None if it is fully transparent.
    fn layer_opaque_bounds(&self, id: LayerId) -> Option<(u32, u32, u32, u32)> {
        let frame = self.current_frame_id();
        let pixels = self.layer(id)?.raster_pixels(frame).ok()?;
        let w = self.width as usize;
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
        let mut any = false;
        for (i, chunk) in pixels.chunks_exact(4).enumerate() {
            if chunk[3] == 0 {
                continue;
            }
            any = true;
            let x = (i % w) as u32;
            let y = (i / w) as u32;
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
        }
        any.then_some((x0, y0, x1, y1))
    }

    /// Align tool: move each layer in `ids` so its opaque bounds line up on the chosen edges.
    /// `h` / `v` are 0 = none, 1 = min (left/top), 2 = centre/middle, 3 = max (right/bottom).
    /// `to_canvas` aligns to the canvas; otherwise to the combined bounds of all the layers.
    pub(crate) fn align_layers(
        &mut self,
        ids: &[LayerId],
        h: u8,
        v: u8,
        to_canvas: bool,
    ) -> Result<()> {
        if ids.is_empty() || (h == 0 && v == 0) {
            return Ok(());
        }
        // Reference box: the canvas, or the union of every target layer's opaque bounds.
        let reference = if to_canvas {
            (0u32, 0u32, self.width, self.height)
        } else {
            let mut r: Option<(u32, u32, u32, u32)> = None;
            for &id in ids {
                if let Some(b) = self.layer_opaque_bounds(id) {
                    r = Some(match r {
                        None => b,
                        Some((rx0, ry0, rx1, ry1)) => {
                            (rx0.min(b.0), ry0.min(b.1), rx1.max(b.2), ry1.max(b.3))
                        }
                    });
                }
            }
            match r {
                Some(b) => b,
                None => return Ok(()),
            }
        };
        let previous_active = self.active_layer;
        for &id in ids {
            let Some((bx0, by0, bx1, by1)) = self.layer_opaque_bounds(id) else {
                continue;
            };
            let bw = bx1 as f64 - bx0 as f64;
            let bh = by1 as f64 - by0 as f64;
            let (rx0, ry0, rx1, ry1) = reference;
            let dx = match h {
                1 => rx0 as f64 - bx0 as f64,
                2 => (rx0 as f64 + rx1 as f64) * 0.5 - (bx0 as f64 + bw * 0.5),
                3 => rx1 as f64 - bx1 as f64,
                _ => 0.0,
            };
            let dy = match v {
                1 => ry0 as f64 - by0 as f64,
                2 => (ry0 as f64 + ry1 as f64) * 0.5 - (by0 as f64 + bh * 0.5),
                3 => ry1 as f64 - by1 as f64,
                _ => 0.0,
            };
            if dx == 0.0 && dy == 0.0 {
                continue;
            }
            self.set_active_layer(id)?;
            self.transform_active(
                Affine2D {
                    m11: 1.0,
                    m12: 0.0,
                    m21: 0.0,
                    m22: 1.0,
                    tx: dx as f32,
                    ty: dy as f32,
                },
                SamplingMode::Nearest,
            )?;
        }
        self.set_active_layer(previous_active)?;
        Ok(())
    }

    pub(crate) fn flip_active(&mut self, horizontal: bool, vertical: bool) -> Result<()> {
        if !horizontal && !vertical {
            return Err(CoreError::InvalidTransform);
        }
        let width = self.width;
        let height = self.height;
        self.prepare_active_raster_edit()?;
        let original = self.active_raster_pixels()?.to_vec();
        let mut output = vec![0; original.len()];
        for y in 0..height {
            for x in 0..width {
                let source_x = if horizontal { width - 1 - x } else { x };
                let source_y = if vertical { height - 1 - y } else { y };
                let source = (source_y as usize * width as usize + source_x as usize) * 4;
                let destination = (y as usize * width as usize + x as usize) * 4;
                output[destination..destination + 4].copy_from_slice(&original[source..source + 4]);
            }
        }
        self.replace_active_pixels(output)
    }

    pub(crate) fn rotate_active_90(&mut self, clockwise: bool) -> Result<()> {
        let width = self.width;
        let height = self.height;
        let center_x = f64::from(width) * 0.5;
        let center_y = f64::from(height) * 0.5;
        self.prepare_active_raster_edit()?;
        let original = self.active_raster_pixels()?.to_vec();
        let mut output = vec![0; original.len()];
        for y in 0..height {
            for x in 0..width {
                let dx = f64::from(x) + 0.5 - center_x;
                let dy = f64::from(y) + 0.5 - center_y;
                let (source_x, source_y) = if clockwise {
                    (center_x + dy - 0.5, center_y - dx - 0.5)
                } else {
                    (center_x - dy - 0.5, center_y + dx - 0.5)
                };
                let sampled = sample_rgba(
                    &original,
                    width,
                    height,
                    source_x,
                    source_y,
                    SamplingMode::Nearest,
                    false,
                );
                let offset = (y as usize * width as usize + x as usize) * 4;
                sampled.write_to(&mut output[offset..offset + 4]);
            }
        }
        self.replace_active_pixels(output)
    }

    pub(crate) fn transform_active(
        &mut self,
        transform: Affine2D,
        sampling: SamplingMode,
    ) -> Result<()> {
        let values = [
            transform.m11,
            transform.m12,
            transform.m21,
            transform.m22,
            transform.tx,
            transform.ty,
        ];
        let determinant = f64::from(transform.m11) * f64::from(transform.m22)
            - f64::from(transform.m12) * f64::from(transform.m21);
        if values.iter().any(|value| !value.is_finite()) || determinant == 0.0 {
            return Err(CoreError::InvalidTransform);
        }
        let width = self.width;
        let height = self.height;
        self.prepare_active_raster_edit()?;
        let original = self.active_raster_pixels()?.to_vec();
        let mut output = vec![0; original.len()];
        let inverse = 1.0 / determinant;
        // How much source area one destination pixel covers. When the transform SHRINKS, a point sample
        // reads one phase of the source and discards the rest -- measured before this existed: a one-pixel
        // checkerboard shrunk by four came out 255 everywhere, where the area average is 128.
        //
        // Only the bilinear mode filters. Nearest is asked for when a caller wants exactly one source pixel,
        // usually for pixel art, and quietly averaging would be the opposite of what it requested.
        let (scale_x, scale_y) = transform_scales(transform);
        let filtering = matches!(sampling, SamplingMode::Bilinear)
            && (scale_x < 1.0 || scale_y < 1.0)
            && scale_x > 0.0
            && scale_y > 0.0;
        for y in 0..height {
            for x in 0..width {
                let destination_x = f64::from(x) + 0.5 - f64::from(transform.tx);
                let destination_y = f64::from(y) + 0.5 - f64::from(transform.ty);
                let source_center_x = (f64::from(transform.m22) * destination_x
                    - f64::from(transform.m12) * destination_y)
                    * inverse;
                let source_center_y = (-f64::from(transform.m21) * destination_x
                    + f64::from(transform.m11) * destination_y)
                    * inverse;
                let sampled = if filtering {
                    sample_rgba_filtered(
                        &original,
                        width,
                        height,
                        source_center_x - 0.5,
                        source_center_y - 0.5,
                        scale_x,
                        scale_y,
                    )
                } else {
                    sample_rgba(
                        &original,
                        width,
                        height,
                        source_center_x - 0.5,
                        source_center_y - 0.5,
                        sampling,
                        false,
                    )
                };
                let offset = (y as usize * width as usize + x as usize) * 4;
                sampled.write_to(&mut output[offset..offset + 4]);
            }
        }
        self.replace_active_pixels(output)
    }

    /// Perspective / distort transform: map the layer's rect corners (TL, TR, BR, BL in canvas
    /// pixels) to the four given destination corners through a homography, then inverse-sample. This
    /// is the non-affine transform the affine `transform_active` cannot express (perspective, and the
    /// distort/unified handles when they are not a parallelogram).
    /// Handle transform: 1 to 4 pinned handles carry their source positions to their
    /// destinations, and the layer follows (L.3).
    ///
    /// Distinct from [`Self::perspective_active`] in the two ways that matter. The source points
    /// are ARBITRARY rather than the layer's own corners, so a user can pin the features they care
    /// about instead of the frame; and the NUMBER of handles restricts the transform class —
    /// translation, similarity, affine, projective — which is what makes one or two handles useful
    /// at all rather than under-determined.
    ///
    /// The class map is applied to the layer's four corners and the result handed to
    /// `perspective_active`. That is exact, not a shortcut: a projective map is determined by the
    /// images of four points in general position, so the homography that path re-solves from the
    /// corners IS the class map. Every class here is projective, so it holds for all four.
    pub(crate) fn handle_transform_active(
        &mut self,
        src: &[(f32, f32)],
        dst: &[(f32, f32)],
        sampling: SamplingMode,
    ) -> Result<()> {
        if src.len() != dst.len() || src.is_empty() || src.len() > crate::MAX_HANDLES {
            return Err(CoreError::InvalidTransform);
        }
        if src
            .iter()
            .chain(dst.iter())
            .any(|&(x, y)| !x.is_finite() || !y.is_finite())
        {
            return Err(CoreError::InvalidTransform);
        }
        let to_f64 = |points: &[(f32, f32)]| -> Vec<(f64, f64)> {
            points
                .iter()
                .map(|&(x, y)| (f64::from(x), f64::from(y)))
                .collect()
        };
        let src64 = to_f64(src);
        let dst64 = to_f64(dst);
        let matrix = crate::handle_transform::handle_transform_matrix(&src64, &dst64)?;
        // Upstream's own validity test, against the points the matrix was solved from.
        crate::handle_transform::projective_validity(&matrix, &src64)?;

        let w = f64::from(self.width);
        let h = f64::from(self.height);
        let corners = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)];
        // The corners are also inputs to the same matrix, so they get the same test — a map that is
        // valid on the handles can still send a corner to infinity, and `project` below cannot
        // stand in for this: it rejects a near-zero `w` but not corners whose `w` differ in SIGN,
        // which is a quad folded through the camera plane with every coordinate finite.
        crate::handle_transform::projective_validity(&matrix, &corners)?;
        let mut projected = [(0.0_f32, 0.0_f32); 4];
        for (slot, &(cx, cy)) in projected.iter_mut().zip(corners.iter()) {
            let (px, py) = crate::handle_transform::project(&matrix, cx, cy)?;
            *slot = (px as f32, py as f32);
        }
        self.perspective_active(projected, sampling)
    }

    pub(crate) fn perspective_active(
        &mut self,
        dst: [(f32, f32); 4],
        sampling: SamplingMode,
    ) -> Result<()> {
        if dst.iter().any(|&(x, y)| !x.is_finite() || !y.is_finite()) {
            return Err(CoreError::InvalidTransform);
        }
        let width = self.width;
        let height = self.height;
        // Source quad is the whole layer rect.
        let src = [
            (0.0_f64, 0.0_f64),
            (f64::from(width), 0.0),
            (f64::from(width), f64::from(height)),
            (0.0, f64::from(height)),
        ];
        let dstf: [(f64, f64); 4] = [
            (f64::from(dst[0].0), f64::from(dst[0].1)),
            (f64::from(dst[1].0), f64::from(dst[1].1)),
            (f64::from(dst[2].0), f64::from(dst[2].1)),
            (f64::from(dst[3].0), f64::from(dst[3].1)),
        ];
        // Homography dst -> src (so each destination pixel reads its source). If it is singular the
        // handles are degenerate; leave the layer alone rather than divide by zero.
        let Some(inv) = homography(dstf, src) else {
            return Err(CoreError::InvalidTransform);
        };
        self.prepare_active_raster_edit()?;
        let original = self.active_raster_pixels()?.to_vec();
        let mut output = vec![0u8; original.len()];
        for y in 0..height {
            for x in 0..width {
                let px = f64::from(x) + 0.5;
                let py = f64::from(y) + 0.5;
                let w = inv[6] * px + inv[7] * py + inv[8];
                if w.abs() < 1e-9 {
                    continue;
                }
                let sx = (inv[0] * px + inv[1] * py + inv[2]) / w;
                let sy = (inv[3] * px + inv[4] * py + inv[5]) / w;
                let sampled = sample_rgba(
                    &original,
                    width,
                    height,
                    sx - 0.5,
                    sy - 0.5,
                    sampling,
                    false,
                );
                let offset = (y as usize * width as usize + x as usize) * 4;
                sampled.write_to(&mut output[offset..offset + 4]);
            }
        }
        self.replace_active_pixels(output)
    }

    /// Cage transform: a closed source cage polygon is dragged to a destination cage, and the pixels
    /// inside follow. Re-derived from GIMP's cage tool, but with mean-value coordinates (robust for
    /// any simple polygon) instead of Green coordinates. For each destination pixel we find its
    /// mean-value weights against the destination cage, then read the source position those same
    /// weights give on the source cage, and inverse-sample it.
    pub(crate) fn cage_transform(
        &mut self,
        src_cage: &[(f32, f32)],
        dst_cage: &[(f32, f32)],
        sampling: SamplingMode,
    ) -> Result<()> {
        if src_cage.len() < 3 || src_cage.len() != dst_cage.len() {
            return Err(CoreError::InvalidTransform);
        }
        if src_cage
            .iter()
            .chain(dst_cage.iter())
            .any(|&(x, y)| !x.is_finite() || !y.is_finite())
        {
            return Err(CoreError::InvalidTransform);
        }
        let width = self.width;
        let height = self.height;
        self.prepare_active_raster_edit()?;
        let original = self.active_raster_pixels()?.to_vec();
        let mut output = original.clone();
        let src: Vec<(f64, f64)> = src_cage
            .iter()
            .map(|&(x, y)| (f64::from(x), f64::from(y)))
            .collect();
        let dst: Vec<(f64, f64)> = dst_cage
            .iter()
            .map(|&(x, y)| (f64::from(x), f64::from(y)))
            .collect();
        // Only touch pixels inside the destination cage; everything else keeps its original value.
        let (minx, miny, maxx, maxy) = polygon_bounds(&dst, width, height);
        for y in miny..maxy {
            for x in minx..maxx {
                let p = (f64::from(x) + 0.5, f64::from(y) + 0.5);
                if !point_in_polygon(p, &dst) {
                    continue;
                }
                let Some(weights) = mean_value_coords(p, &dst) else {
                    continue;
                };
                // Reconstruct the source position from the same weights on the source cage.
                let mut sx = 0.0;
                let mut sy = 0.0;
                for (w, &(cx, cy)) in weights.iter().zip(src.iter()) {
                    sx += w * cx;
                    sy += w * cy;
                }
                let sampled = sample_rgba(
                    &original,
                    width,
                    height,
                    sx - 0.5,
                    sy - 0.5,
                    sampling,
                    false,
                );
                let offset = (y as usize * width as usize + x as usize) * 4;
                sampled.write_to(&mut output[offset..offset + 4]);
            }
        }
        self.replace_active_pixels(output)
    }

    /// Warp / liquify brush: drag over the layer to push, grow, shrink or swirl pixels. Re-derived
    /// from GIMP's warp transform (the IWarp successor) and Krita's liquify. `points` is the stroke in
    /// canvas pixels; `mode` picks the deformation; `radius` is the brush radius and `strength` scales
    /// it. We accumulate an inverse displacement field (for each destination pixel, where in the
    /// source to read) and sample once.
    pub(crate) fn warp_brush(
        &mut self,
        points: &[(f32, f32)],
        mode: WarpMode,
        radius: f32,
        strength: f32,
        sampling: SamplingMode,
    ) -> Result<()> {
        if points.is_empty() || !(radius.is_finite()) || radius <= 0.0 || !strength.is_finite() {
            return Err(CoreError::InvalidTransform);
        }
        let width = self.width;
        let height = self.height;
        self.prepare_active_raster_edit()?;
        let original = self.active_raster_pixels()?.to_vec();
        let n = (width as usize) * (height as usize);
        // Inverse displacement: disp[i] = (dx, dy) added to the destination to find the source.
        let mut disp = vec![(0.0_f32, 0.0_f32); n];
        let r = f64::from(radius);
        let r2 = r * r;
        for (k, &(cx, cy)) in points.iter().enumerate() {
            let cx = f64::from(cx);
            let cy = f64::from(cy);
            // Stroke direction, used by the "move" mode.
            let (mut vx, mut vy) = (0.0_f64, 0.0_f64);
            if k > 0 {
                vx = cx - f64::from(points[k - 1].0);
                vy = cy - f64::from(points[k - 1].1);
            }
            let x0 = (cx - r).floor().max(0.0) as u32;
            let y0 = (cy - r).floor().max(0.0) as u32;
            let x1 = ((cx + r).ceil() as i64).clamp(0, i64::from(width)) as u32;
            let y1 = ((cy + r).ceil() as i64).clamp(0, i64::from(height)) as u32;
            for y in y0..y1 {
                for x in x0..x1 {
                    let px = f64::from(x) + 0.5;
                    let py = f64::from(y) + 0.5;
                    let ox = px - cx;
                    let oy = py - cy;
                    let d2 = ox * ox + oy * oy;
                    if d2 > r2 {
                        continue;
                    }
                    // Smooth Gaussian-like falloff, 1 at the centre to 0 at the rim.
                    let falloff = (1.0 - d2 / r2).powi(2) * f64::from(strength);
                    let (ddx, ddy) = match mode {
                        // Move: pull the source backwards along the stroke, so pixels shift forward.
                        WarpMode::Move => (-vx * falloff, -vy * falloff),
                        // Grow: push the source outward (read from closer to centre) -> magnify.
                        WarpMode::Grow => (-ox * falloff, -oy * falloff),
                        // Shrink: pull the source inward -> minify.
                        WarpMode::Shrink => (ox * falloff, oy * falloff),
                        // Swirl: rotate the source sample about the centre.
                        WarpMode::SwirlCw | WarpMode::SwirlCcw => {
                            let sign = if matches!(mode, WarpMode::SwirlCw) {
                                1.0
                            } else {
                                -1.0
                            };
                            let angle = falloff * sign;
                            let (s, c) = angle.sin_cos();
                            let rx = c * ox - s * oy;
                            let ry = s * ox + c * oy;
                            (rx - ox, ry - oy)
                        }
                    };
                    let i = (y as usize) * (width as usize) + x as usize;
                    disp[i].0 += ddx as f32;
                    disp[i].1 += ddy as f32;
                }
            }
        }
        let mut output = original.clone();
        for y in 0..height {
            for x in 0..width {
                let i = (y as usize) * (width as usize) + x as usize;
                let (dx, dy) = disp[i];
                if dx == 0.0 && dy == 0.0 {
                    continue;
                }
                let sx = f64::from(x) + 0.5 + f64::from(dx);
                let sy = f64::from(y) + 0.5 + f64::from(dy);
                let sampled = sample_rgba(
                    &original,
                    width,
                    height,
                    sx - 0.5,
                    sy - 0.5,
                    sampling,
                    false,
                );
                let offset = i * 4;
                sampled.write_to(&mut output[offset..offset + 4]);
            }
        }
        self.replace_active_pixels(output)
    }

    /// N-point deformation: N control points are dragged from their source positions to destination
    /// positions and the whole layer warps smoothly to follow. Re-derived from Krita's n-point
    /// transform, implemented as a thin-plate spline (TPS) — the standard smooth interpolant for
    /// scattered point pairs. We fit the dst->src spline (so each destination pixel reads its source)
    /// and inverse-sample once.
    pub(crate) fn npoint_transform(
        &mut self,
        src_pts: &[(f32, f32)],
        dst_pts: &[(f32, f32)],
        sampling: SamplingMode,
    ) -> Result<()> {
        if src_pts.len() < 2 || src_pts.len() != dst_pts.len() || src_pts.len() > 64 {
            return Err(CoreError::InvalidTransform);
        }
        if src_pts
            .iter()
            .chain(dst_pts.iter())
            .any(|&(x, y)| !x.is_finite() || !y.is_finite())
        {
            return Err(CoreError::InvalidTransform);
        }
        // Fit TPS from the DESTINATION control points to the SOURCE coordinates, so evaluating at a
        // destination pixel yields where to read in the source.
        let ctrl: Vec<(f64, f64)> = dst_pts
            .iter()
            .map(|&(x, y)| (f64::from(x), f64::from(y)))
            .collect();
        let target_x: Vec<f64> = src_pts.iter().map(|&(x, _)| f64::from(x)).collect();
        let target_y: Vec<f64> = src_pts.iter().map(|&(_, y)| f64::from(y)).collect();
        let (Some(wx), Some(wy)) = (tps_weights(&ctrl, &target_x), tps_weights(&ctrl, &target_y))
        else {
            return Err(CoreError::InvalidTransform);
        };
        let width = self.width;
        let height = self.height;
        self.prepare_active_raster_edit()?;
        let original = self.active_raster_pixels()?.to_vec();
        let mut output = vec![0u8; original.len()];
        for y in 0..height {
            for x in 0..width {
                let px = f64::from(x) + 0.5;
                let py = f64::from(y) + 0.5;
                let sx = tps_eval(&ctrl, &wx, px, py);
                let sy = tps_eval(&ctrl, &wy, px, py);
                let sampled = sample_rgba(
                    &original,
                    width,
                    height,
                    sx - 0.5,
                    sy - 0.5,
                    sampling,
                    false,
                );
                let offset = (y as usize * width as usize + x as usize) * 4;
                sampled.write_to(&mut output[offset..offset + 4]);
            }
        }
        self.replace_active_pixels(output)
    }

    /// 3D transform: rotate the layer in space about its centre (angles in radians about the X, Y and
    /// Z axes) and project through a simple pinhole camera at `distance` layer-widths away. Re-derived
    /// from GIMP's transform3d: it reduces to projecting the four layer corners and warping to that
    /// quad, so it reuses `perspective_active`.
    pub(crate) fn transform3d_active(
        &mut self,
        rot_x: f32,
        rot_y: f32,
        rot_z: f32,
        distance: f32,
        sampling: SamplingMode,
    ) -> Result<()> {
        if [rot_x, rot_y, rot_z, distance]
            .iter()
            .any(|v| !v.is_finite())
            || distance <= 0.0
        {
            return Err(CoreError::InvalidTransform);
        }
        let w = f64::from(self.width);
        let h = f64::from(self.height);
        let cx = w * 0.5;
        let cy = h * 0.5;
        // Camera is `distance` canvas-widths in front; the focal length keeps the un-rotated layer the
        // same size (so distance only bends it, not zooms it).
        let d = f64::from(distance) * w.max(1.0);
        let focal = d;
        let (sx, cxr) = f64::from(rot_x).sin_cos();
        let (sy, cyr) = f64::from(rot_y).sin_cos();
        let (sz, czr) = f64::from(rot_z).sin_cos();
        // Row-major rotation Rz * Ry * Rx.
        let rotate = |x: f64, y: f64, z: f64| -> (f64, f64, f64) {
            // Rx
            let (y1, z1) = (y * cxr - z * sx, y * sx + z * cxr);
            // Ry
            let (x2, z2) = (x * cyr + z1 * sy, -x * sy + z1 * cyr);
            // Rz
            let (x3, y3) = (x2 * czr - y1 * sz, x2 * sz + y1 * czr);
            (x3, y3, z2)
        };
        let corners = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)];
        let mut projected = [(0.0_f32, 0.0_f32); 4];
        for (i, &(px, py)) in corners.iter().enumerate() {
            // Centre, rotate, then pinhole-project back to canvas pixels.
            let (rx, ry, rz) = rotate(px - cx, py - cy, 0.0);
            let denom = d + rz;
            if denom.abs() < 1e-6 {
                return Err(CoreError::InvalidTransform);
            }
            let scale = focal / denom;
            projected[i] = ((cx + rx * scale) as f32, (cy + ry * scale) as f32);
        }
        self.perspective_active(projected, sampling)
    }

    pub(crate) fn replace_active_pixels(&mut self, pixels: Vec<u8>) -> Result<()> {
        // Four samples per pixel at the document's declared width (J.1a). This said `* 4`, so a
        // precision-native filter writing correct 16-bit bytes was rejected by the store it was
        // writing to — found by the first such filter, not by reading.
        let expected = pixel_count(self.width, self.height)? * self.precision.bytes_per_pixel();
        if pixels.len() != expected {
            return Err(CoreError::InvalidBufferLength {
                expected,
                actual: pixels.len(),
            });
        }
        *self.active_raster_pixels_mut()? = pixels.into();
        Ok(())
    }

    pub(crate) fn from_single_layer(
        width: u32,
        height: u32,
        pixels: Vec<u8>,
        title: String,
    ) -> Result<Self> {
        let count = pixel_count(width, height)?;
        let id = LayerId::new();
        Ok(Self {
            id: Uuid::new_v4(),
            width,
            height,
            metadata: DocumentMetadata {
                title,
                ..DocumentMetadata::default()
            },
            // `pixels` is 8-bit RGBA by this function's signature.
            precision: Precision::U8,
            layers: vec![Layer::from_rgba(
                id,
                "Imported image".into(),
                pixels,
                count,
            )?],
            active_layer: id,
            timeline: Timeline::default(),
            selection: Selection::new(width, height)?,
            channels: Vec::new(),
            quick_mask: None,
            color_mode: ColorMode::Rgb,
            palette: Vec::new(),
            paths: Vec::new(),
            guides: Vec::new(),
            sample_points: Vec::new(),
            guide_settings: crate::GuideSettings::default(),
        })
    }

    /// A one-layer working document an adjustment node's filter runs in (P11). Precision and colour
    /// mode are the real document's, so a filter sees the same samples and the same `!gray` guard it
    /// would see applied destructively. Selection is empty, so the filter covers the whole canvas.
    pub(crate) fn adjustment_scratch(
        width: u32,
        height: u32,
        precision: Precision,
        color_mode: ColorMode,
        pixels: Vec<u8>,
    ) -> Result<Self> {
        let count = pixel_count(width, height)?;
        let mut scratch = Self::from_single_layer(width, height, vec![0; count * 4], String::new())?;
        scratch.set_precision(precision);
        scratch.color_mode = color_mode;
        scratch.replace_active_pixels(pixels)?;
        Ok(scratch)
    }

    pub(crate) fn from_v1_parts(
        id: Uuid,
        width: u32,
        height: u32,
        metadata: DocumentMetadata,
        layers: Vec<Layer>,
        active_layer: LayerId,
        selection: Selection,
    ) -> Self {
        Self {
            id,
            width,
            height,
            metadata,
            // A version-1 document predates the field entirely, and its bytes are 8-bit.
            precision: Precision::U8,
            layers,
            active_layer,
            timeline: Timeline::default(),
            selection,
            channels: Vec::new(),
            quick_mask: None,
            color_mode: ColorMode::Rgb,
            palette: Vec::new(),
            paths: Vec::new(),
            guides: Vec::new(),
            sample_points: Vec::new(),
            guide_settings: crate::GuideSettings::default(),
        }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        let count = pixel_count(self.width, self.height)?;
        // Four SAMPLES per pixel, whose width the document declares (J.1a). This read
        // `count * 4` while one byte per sample was the only possibility; left that way, a
        // widened document fails its own validator — which is how this line was found.
        let expected_rgba = count.checked_mul(self.precision.bytes_per_pixel()).ok_or(
            CoreError::InvalidDimensions {
                width: self.width,
                height: self.height,
            },
        )?;
        validate_timeline(&self.timeline)?;
        validate_metadata(&self.metadata)?;
        // A channel is ONE byte per pixel at every precision -- coverage, not colour (J.2a). So it
        // is measured against the pixel count, not against `expected_rgba`: using the latter would
        // make every channel in a deep document fail validation, and using four bytes a pixel would
        // accept a buffer four times the size it should be.
        if self.channels.len() > MAX_CHANNELS {
            return Err(CoreError::DocumentLimitExceeded("channel count"));
        }
        let mut channel_ids = HashSet::with_capacity(self.channels.len());
        for channel in &self.channels {
            if !channel_ids.insert(channel.id()) {
                return Err(CoreError::DuplicateChannelId(channel.id()));
            }
            if channel.pixels().len() != count {
                return Err(CoreError::InvalidBufferLength {
                    expected: count,
                    actual: channel.pixels().len(),
                });
            }
            if !channel.opacity().is_finite() || !(0.0..=1.0).contains(&channel.opacity()) {
                return Err(CoreError::InvalidOpacity);
            }
            validate_name(channel.name())?;
        }
        if self.layers.is_empty() {
            return Err(CoreError::LastLayer);
        }
        if self.layers.len() > MAX_NODES {
            return Err(CoreError::DocumentLimitExceeded("node count"));
        }

        let frame_ids = self
            .timeline
            .frames
            .iter()
            .map(|frame| frame.id)
            .collect::<HashSet<_>>();
        let mut ids = HashSet::with_capacity(self.layers.len());
        let mut parents = HashMap::with_capacity(self.layers.len());
        let mut kinds = HashMap::with_capacity(self.layers.len());
        let mut stored_raster_bytes = self.selection.mask().len() as u64;
        let mut complexity = SemanticUsage::default();

        for layer in &self.layers {
            if !ids.insert(layer.id) {
                return Err(CoreError::DuplicateNodeId(layer.id));
            }
            parents.insert(layer.id, layer.parent);
            kinds.insert(layer.id, layer.kind());
            validate_name(&layer.name)?;
            if !layer.opacity.is_finite() || !(0.0..=1.0).contains(&layer.opacity) {
                return Err(CoreError::InvalidOpacity);
            }
            if let Some(mask) = &layer.mask {
                if !matches!(layer.kind(), NodeKind::Raster | NodeKind::Group) {
                    return Err(CoreError::UnsupportedNodeContent(layer.kind()));
                }
                if mask.pixels.len() != count {
                    return Err(CoreError::InvalidBufferLength {
                        expected: count,
                        actual: mask.pixels.len(),
                    });
                }
            }
            validate_node_content(layer, &frame_ids, expected_rgba, &mut complexity)?;
            if matches!(layer.kind(), NodeKind::Text | NodeKind::Vector) {
                crate::semantic::preflight(layer.content(), self.width, self.height)?;
            }
            stored_raster_bytes = stored_raster_bytes
                .checked_add(layer.stored_raster_bytes())
                .ok_or(CoreError::DocumentLimitExceeded("stored raster bytes"))?;
            if stored_raster_bytes > MAX_STORED_RASTER_BYTES {
                return Err(CoreError::DocumentLimitExceeded("stored raster bytes"));
            }
        }

        if !ids.contains(&self.active_layer) {
            return Err(CoreError::MalformedProject(
                "active node does not exist".into(),
            ));
        }
        validate_hierarchy(&parents, &kinds)?;
        let mut canonical = self.layers.clone();
        let siblings = hierarchy_siblings(&canonical);
        reorder_nodes(&mut canonical, &siblings)?;
        if canonical
            .iter()
            .map(|node| node.id)
            .ne(self.layers.iter().map(|node| node.id))
        {
            return Err(CoreError::InvalidHierarchyOrder);
        }
        self.selection.validate(self.width, self.height)?;
        crate::render::preflight_document(self)
    }

    pub fn semantic_usage(&self) -> Result<SemanticUsage> {
        self.layers
            .iter()
            .try_fold(SemanticUsage::default(), |total, layer| {
                admit_semantic_replacement(
                    total,
                    SemanticUsage::default(),
                    semantic_usage(layer.content())?,
                )
            })
    }

    pub fn stored_raster_bytes(&self) -> u64 {
        self.layers
            .iter()
            .fold(self.selection.mask().len() as u64, |total, layer| {
                total.saturating_add(layer.stored_raster_bytes())
            })
    }

    pub(crate) fn memory_bytes(&self, shared_allocations: &mut HashSet<usize>) -> usize {
        let mut bytes = std::mem::size_of::<Self>()
            .saturating_add(
                self.layers
                    .capacity()
                    .saturating_mul(std::mem::size_of::<Layer>()),
            )
            .saturating_add(
                self.timeline
                    .frames
                    .capacity()
                    .saturating_mul(std::mem::size_of::<Frame>()),
            )
            .saturating_add(self.metadata.title.capacity())
            .saturating_add(self.metadata.author.as_ref().map_or(0, String::capacity));
        for (key, value) in &self.metadata.properties {
            bytes = bytes
                .saturating_add(std::mem::size_of::<(String, String)>())
                .saturating_add(key.capacity())
                .saturating_add(value.capacity());
        }

        add_shared_memory(
            &mut bytes,
            shared_allocations,
            self.selection.mask_storage(),
        );
        for layer in &self.layers {
            bytes = bytes.saturating_add(layer.name.capacity());
            if let Some(mask) = &layer.mask {
                add_shared_memory(&mut bytes, shared_allocations, &mask.pixels);
            }
            match &layer.content {
                NodeContent::Raster { cels } => {
                    bytes = bytes.saturating_add(
                        cels.capacity()
                            .saturating_mul(std::mem::size_of::<RasterCel>()),
                    );
                    for cel in cels {
                        add_shared_memory(&mut bytes, shared_allocations, &cel.pixels);
                    }
                }
                NodeContent::Group => {}
                NodeContent::Adjustment { .. } => {
                    bytes = bytes.saturating_add(std::mem::size_of::<crate::Filter>());
                }
                NodeContent::Text { text } => {
                    bytes = bytes
                        .saturating_add(text.text.capacity())
                        .saturating_add(text.font_family.capacity())
                        .saturating_add(text.font_id.capacity());
                }
                NodeContent::Vector { vector } => {
                    bytes = bytes.saturating_add(
                        vector
                            .paths
                            .capacity()
                            .saturating_mul(std::mem::size_of::<VectorPath>()),
                    );
                    for path in &vector.paths {
                        bytes = bytes.saturating_add(
                            path.commands
                                .capacity()
                                .saturating_mul(std::mem::size_of::<PathCommand>()),
                        );
                    }
                }
            }
        }
        bytes
    }
}

fn hierarchy_siblings(nodes: &[Layer]) -> HashMap<Option<NodeId>, Vec<NodeId>> {
    let mut siblings = HashMap::<Option<NodeId>, Vec<NodeId>>::new();
    for node in nodes {
        siblings.entry(node.parent).or_default().push(node.id);
        siblings.entry(Some(node.id)).or_default();
    }
    siblings
}

fn append_postorder(
    id: NodeId,
    siblings: &HashMap<Option<NodeId>, Vec<NodeId>>,
    nodes: &HashMap<NodeId, Layer>,
    ordered: &mut Vec<Layer>,
    depth: usize,
) -> Result<()> {
    if depth > MAX_HIERARCHY_DEPTH {
        return Err(CoreError::DocumentLimitExceeded("hierarchy depth"));
    }
    for child in siblings.get(&Some(id)).into_iter().flatten() {
        append_postorder(*child, siblings, nodes, ordered, depth + 1)?;
    }
    ordered.push(
        nodes
            .get(&id)
            .cloned()
            .ok_or(CoreError::InvalidHierarchyOrder)?,
    );
    Ok(())
}

fn reorder_nodes(
    nodes: &mut Vec<Layer>,
    siblings: &HashMap<Option<NodeId>, Vec<NodeId>>,
) -> Result<()> {
    let by_id = nodes
        .iter()
        .cloned()
        .map(|node| (node.id, node))
        .collect::<HashMap<_, _>>();
    let mut ordered = Vec::with_capacity(nodes.len());
    for root in siblings.get(&None).into_iter().flatten() {
        append_postorder(*root, siblings, &by_id, &mut ordered, 0)?;
    }
    if ordered.len() != nodes.len() {
        return Err(CoreError::InvalidHierarchyOrder);
    }
    *nodes = ordered;
    Ok(())
}

fn add_shared_memory(
    bytes: &mut usize,
    shared_allocations: &mut HashSet<usize>,
    storage: &RasterBytes,
) {
    if shared_allocations.insert(storage.allocation_identity()) {
        *bytes = (*bytes).saturating_add(storage.allocation_bytes());
    }
}

fn validate_metadata(metadata: &DocumentMetadata) -> Result<()> {
    if metadata.properties.len() > MAX_METADATA_ENTRIES {
        return Err(CoreError::DocumentLimitExceeded("metadata entries"));
    }
    let mut bytes = metadata.title.len();
    bytes = bytes
        .checked_add(metadata.author.as_ref().map_or(0, String::len))
        .ok_or(CoreError::DocumentLimitExceeded("metadata bytes"))?;
    for (key, value) in &metadata.properties {
        bytes = bytes
            .checked_add(key.len())
            .and_then(|total| total.checked_add(value.len()))
            .ok_or(CoreError::DocumentLimitExceeded("metadata bytes"))?;
    }
    if bytes > MAX_METADATA_BYTES {
        return Err(CoreError::DocumentLimitExceeded("metadata bytes"));
    }
    Ok(())
}

fn validate_timeline(timeline: &Timeline) -> Result<()> {
    if timeline.frames.is_empty() || timeline.frames.len() > MAX_FRAMES {
        return Err(CoreError::DocumentLimitExceeded("frame count"));
    }
    if !timeline.fps.is_finite() || timeline.fps <= 0.0 || timeline.fps > MAX_TIMELINE_FPS {
        return Err(CoreError::MalformedProject(
            "timeline fps is invalid".into(),
        ));
    }
    let authoritative_duration = timeline_frame_duration_ms(timeline.fps)
        .map_err(|_| CoreError::MalformedProject("timeline fps is invalid".into()))?;
    let mut positions = HashMap::with_capacity(timeline.frames.len());
    for (position, frame) in timeline.frames.iter().enumerate() {
        if frame.duration_ms == 0 || frame.duration_ms > MAX_FRAME_DURATION_MS {
            return Err(CoreError::MalformedProject(
                "frame duration is invalid".into(),
            ));
        }
        if frame.duration_ms != authoritative_duration {
            return Err(CoreError::MalformedProject(
                "frame duration disagrees with timeline fps".into(),
            ));
        }
        if positions.insert(frame.id, position).is_some() {
            return Err(CoreError::MalformedProject(
                "duplicate frame identifier".into(),
            ));
        }
    }
    let current = positions
        .get(&timeline.current_frame)
        .copied()
        .ok_or_else(|| {
            CoreError::MalformedProject("current timeline frame does not exist".into())
        })?;
    let range_start = positions
        .get(&timeline.playback.range_start)
        .copied()
        .ok_or_else(|| CoreError::MalformedProject("playback range start does not exist".into()))?;
    let range_end = positions
        .get(&timeline.playback.range_end)
        .copied()
        .ok_or_else(|| CoreError::MalformedProject("playback range end does not exist".into()))?;
    if range_start > range_end || current >= timeline.frames.len() {
        return Err(CoreError::MalformedProject(
            "playback range is invalid".into(),
        ));
    }
    Ok(())
}

fn validate_hierarchy(
    parents: &HashMap<NodeId, Option<NodeId>>,
    kinds: &HashMap<NodeId, NodeKind>,
) -> Result<()> {
    for (&node, &parent) in parents {
        if let Some(parent) = parent {
            if !parents.contains_key(&parent) {
                return Err(CoreError::MalformedProject(format!(
                    "parent {parent} of node {node} does not exist"
                )));
            }
            if kinds.get(&parent) != Some(&NodeKind::Group) {
                return Err(CoreError::MalformedProject(format!(
                    "parent {parent} is not a group"
                )));
            }
        }
    }

    for &start in parents.keys() {
        let mut current = Some(start);
        let mut visited = HashSet::new();
        let mut depth = 0_usize;
        while let Some(node) = current {
            if !visited.insert(node) {
                return Err(CoreError::MalformedProject(
                    "node hierarchy contains a cycle".into(),
                ));
            }
            current = parents.get(&node).copied().flatten();
            if current.is_some() {
                depth = depth.saturating_add(1);
                if depth > MAX_HIERARCHY_DEPTH {
                    return Err(CoreError::DocumentLimitExceeded("hierarchy depth"));
                }
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SemanticUsage {
    pub text_bytes: usize,
    pub vector_paths: usize,
    pub path_commands: usize,
    pub memory_bytes: usize,
}

impl SemanticUsage {
    fn checked_add(self, other: Self) -> Result<Self> {
        Ok(Self {
            text_bytes: self
                .text_bytes
                .checked_add(other.text_bytes)
                .ok_or(CoreError::DocumentLimitExceeded("text complexity"))?,
            vector_paths: self
                .vector_paths
                .checked_add(other.vector_paths)
                .ok_or(CoreError::DocumentLimitExceeded("vector path count"))?,
            path_commands: self.path_commands.checked_add(other.path_commands).ok_or(
                CoreError::DocumentLimitExceeded("vector command complexity"),
            )?,
            memory_bytes: self
                .memory_bytes
                .checked_add(other.memory_bytes)
                .ok_or(CoreError::DocumentLimitExceeded("semantic memory bytes"))?,
        })
    }

    fn checked_sub(self, other: Self) -> Result<Self> {
        Ok(Self {
            text_bytes: self
                .text_bytes
                .checked_sub(other.text_bytes)
                .ok_or_else(|| CoreError::MalformedProject("semantic usage underflow".into()))?,
            vector_paths: self
                .vector_paths
                .checked_sub(other.vector_paths)
                .ok_or_else(|| CoreError::MalformedProject("semantic usage underflow".into()))?,
            path_commands: self
                .path_commands
                .checked_sub(other.path_commands)
                .ok_or_else(|| CoreError::MalformedProject("semantic usage underflow".into()))?,
            memory_bytes: self
                .memory_bytes
                .checked_sub(other.memory_bytes)
                .ok_or_else(|| CoreError::MalformedProject("semantic usage underflow".into()))?,
        })
    }
}

fn semantic_usage_for_vector_path(path: &VectorPath) -> Result<SemanticUsage> {
    Ok(SemanticUsage {
        vector_paths: 1,
        path_commands: path.commands.len(),
        memory_bytes: std::mem::size_of::<VectorPath>()
            .checked_add(
                path.commands
                    .len()
                    .checked_mul(std::mem::size_of::<PathCommand>())
                    .ok_or(CoreError::DocumentLimitExceeded("semantic memory bytes"))?,
            )
            .ok_or(CoreError::DocumentLimitExceeded("semantic memory bytes"))?,
        ..SemanticUsage::default()
    })
}

pub fn semantic_usage(content: &NodeContent) -> Result<SemanticUsage> {
    match content {
        NodeContent::Text { text } => {
            let text_bytes = text
                .text
                .len()
                .checked_add(text.font_family.len())
                .and_then(|bytes| bytes.checked_add(text.font_id.len()))
                .ok_or(CoreError::DocumentLimitExceeded("text complexity"))?;
            Ok(SemanticUsage {
                text_bytes,
                memory_bytes: std::mem::size_of::<TextContent>()
                    .checked_add(text_bytes)
                    .ok_or(CoreError::DocumentLimitExceeded("semantic memory bytes"))?,
                ..SemanticUsage::default()
            })
        }
        NodeContent::Vector { vector } => vector.paths.iter().try_fold(
            SemanticUsage {
                memory_bytes: std::mem::size_of::<VectorContent>(),
                ..SemanticUsage::default()
            },
            |total, path| total.checked_add(semantic_usage_for_vector_path(path)?),
        ),
        NodeContent::Raster { .. } | NodeContent::Group | NodeContent::Adjustment { .. } => {
            Ok(SemanticUsage::default())
        }
    }
}

pub fn admit_semantic_replacement(
    current: SemanticUsage,
    removed: SemanticUsage,
    added: SemanticUsage,
) -> Result<SemanticUsage> {
    let projected = current.checked_sub(removed)?.checked_add(added)?;
    if projected.text_bytes > MAX_TEXT_BYTES {
        return Err(CoreError::DocumentLimitExceeded("text complexity"));
    }
    if projected.vector_paths > MAX_VECTOR_PATHS {
        return Err(CoreError::DocumentLimitExceeded("vector path count"));
    }
    if projected.path_commands > MAX_PATH_COMMANDS {
        return Err(CoreError::DocumentLimitExceeded(
            "vector command complexity",
        ));
    }
    if projected.memory_bytes > MAX_SEMANTIC_MEMORY_BYTES {
        return Err(CoreError::DocumentLimitExceeded("semantic memory bytes"));
    }
    Ok(projected)
}

fn validate_node_content(
    layer: &Layer,
    frame_ids: &HashSet<FrameId>,
    expected_rgba: usize,
    totals: &mut SemanticUsage,
) -> Result<()> {
    match &layer.content {
        NodeContent::Raster { cels } => {
            if cels.is_empty() || cels.len() > frame_ids.len() {
                return Err(CoreError::DocumentLimitExceeded("raster cel count"));
            }
            let mut cel_frames = HashSet::with_capacity(cels.len());
            for cel in cels {
                if !frame_ids.contains(&cel.frame) {
                    return Err(CoreError::MalformedProject(format!(
                        "raster cel references unknown frame {}",
                        cel.frame.get()
                    )));
                }
                if !cel_frames.insert(cel.frame) {
                    return Err(CoreError::MalformedProject(
                        "duplicate raster cel frame".into(),
                    ));
                }
                if cel.pixels.len() != expected_rgba {
                    return Err(CoreError::InvalidBufferLength {
                        expected: expected_rgba,
                        actual: cel.pixels.len(),
                    });
                }
            }
        }
        NodeContent::Group => {}
        NodeContent::Text { text } => {
            *totals = admit_semantic_replacement(
                *totals,
                SemanticUsage::default(),
                semantic_usage(layer.content())?,
            )?;
            crate::semantic::validate_text(text)?;
        }
        NodeContent::Vector { vector } => {
            *totals = admit_semantic_replacement(
                *totals,
                SemanticUsage::default(),
                semantic_usage(layer.content())?,
            )?;
            crate::semantic::validate_vector(vector)?;
        }
        NodeContent::Adjustment { filter } => crate::filters::validate_adjustment_filter(filter)?,
    }
    Ok(())
}

/// M13: resample a cel stored at a 16- or 32-bit precision. Nearest copies samples; bilinear
/// blends the four neighbours in premultiplied alpha, so transparent pixels do not bleed their
/// colour. Same pixel-centre mapping as the 8-bit path.
fn resample_deep(
    input: &[u8],
    precision: crate::precision::Precision,
    old_width: u32,
    old_height: u32,
    width: u32,
    height: u32,
    sampling: SamplingMode,
) -> Vec<u8> {
    let bpp = precision.bytes_per_pixel();
    let mut output = vec![0_u8; width as usize * height as usize * bpp];
    let read = |x: i64, y: i64| -> [f32; 4] {
        let x = x.clamp(0, i64::from(old_width) - 1) as usize;
        let y = y.clamp(0, i64::from(old_height) - 1) as usize;
        let base = (y * old_width as usize + x) * 4;
        [0, 1, 2, 3].map(|c| precision.read_sample(input, base + c))
    };
    for y in 0..height {
        for x in 0..width {
            let sx = (f64::from(x) + 0.5) * f64::from(old_width) / f64::from(width) - 0.5;
            let sy = (f64::from(y) + 0.5) * f64::from(old_height) / f64::from(height) - 0.5;
            let value = match sampling {
                SamplingMode::Nearest => read(sx.round() as i64, sy.round() as i64),
                SamplingMode::Bilinear => {
                    let (x0, y0) = (sx.floor() as i64, sy.floor() as i64);
                    let (fx, fy) = ((sx - sx.floor()) as f32, (sy - sy.floor()) as f32);
                    let mut acc = [0.0_f32; 4];
                    for (dx, dy, w) in [(0, 0, (1.0 - fx) * (1.0 - fy)), (1, 0, fx * (1.0 - fy)), (0, 1, (1.0 - fx) * fy), (1, 1, fx * fy)] {
                        let p = read(x0 + dx, y0 + dy);
                        for c in 0..3 {
                            acc[c] += p[c] * p[3] * w;
                        }
                        acc[3] += p[3] * w;
                    }
                    if acc[3] > 0.0 {
                        [acc[0] / acc[3], acc[1] / acc[3], acc[2] / acc[3], acc[3]]
                    } else {
                        [0.0; 4]
                    }
                }
            };
            let base = (y as usize * width as usize + x as usize) * 4;
            for (c, v) in value.iter().enumerate() {
                precision.write_sample(&mut output, base + c, *v);
            }
        }
    }
    output
}

fn crop_bytes(
    input: &[u8],
    old_width: u32,
    old_height: u32,
    rect: Rect,
    channels: usize,
    output_pixels: usize,
) -> Vec<u8> {
    let mut output = vec![0; output_pixels * channels];
    for y in 0..rect.height {
        for x in 0..rect.width {
            let source_x = i64::from(rect.x) + i64::from(x);
            let source_y = i64::from(rect.y) + i64::from(y);
            if source_x >= 0
                && source_y >= 0
                && source_x < i64::from(old_width)
                && source_y < i64::from(old_height)
            {
                let source =
                    (source_y as usize * old_width as usize + source_x as usize) * channels;
                let destination = (y as usize * rect.width as usize + x as usize) * channels;
                output[destination..destination + channels]
                    .copy_from_slice(&input[source..source + channels]);
            }
        }
    }
    output
}

fn validate_gradient(kind: GradientKind, stops: &[GradientStop]) -> Result<()> {
    if !(2..=32).contains(&stops.len()) {
        return Err(CoreError::InvalidGradient);
    }
    let mut previous = -1.0_f32;
    for stop in stops {
        if !stop.position.is_finite()
            || !(0.0..=1.0).contains(&stop.position)
            || stop.position <= previous
        {
            return Err(CoreError::InvalidGradient);
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
            let values = [start_x, start_y, end_x, end_y];
            let dx = end_x - start_x;
            let dy = end_y - start_y;
            let length_squared = dx * dx + dy * dy;
            if values.iter().any(|value| !value.is_finite())
                || !dx.is_finite()
                || !dy.is_finite()
                || !length_squared.is_finite()
                || length_squared <= f32::EPSILON
            {
                return Err(CoreError::InvalidGradient);
            }
        }
        GradientKind::Radial {
            center_x,
            center_y,
            radius,
        } => {
            if !center_x.is_finite()
                || !center_y.is_finite()
                || !radius.is_finite()
                || radius <= 0.0
            {
                return Err(CoreError::InvalidGradient);
            }
        }
    }
    Ok(())
}

fn interpolate_gradient(stops: &[GradientStop], position: f32) -> Pixel {
    if position <= stops[0].position {
        return stops[0].color;
    }
    if position >= stops[stops.len() - 1].position {
        return stops[stops.len() - 1].color;
    }
    let upper = stops.partition_point(|stop| stop.position < position);
    let left = stops[upper - 1];
    let right = stops[upper];
    let amount = (position - left.position) / (right.position - left.position);
    let interpolate = |start: u8, end: u8| {
        (f32::from(start) + (f32::from(end) - f32::from(start)) * amount)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Pixel::rgba(
        interpolate(left.color.r, right.color.r),
        interpolate(left.color.g, right.color.g),
        interpolate(left.color.b, right.color.b),
        interpolate(left.color.a, right.color.a),
    )
}

/// A validated stroke, resolved to dabs, with the region it will damage known before painting.
pub(crate) struct BrushPlan<'t> {
    dabs: Vec<BrushPoint>,
    /// Per-dab opacity and flow multipliers (I.1), parallel to `dabs`. Empty means no binding was
    /// asked for; readers must treat that as 1.0 and NOT as 0.0.
    dab_opacity: Vec<f32>,
    dab_flow: Vec<f32>,
    color: Pixel,
    size: f32,
    opacity: f32,
    shape: crate::DabShape,
    tip: Option<&'t crate::BrushTip>,
    // GIH pipe frames in stamp order; empty means use the generated dab or the single `tip`.
    frames: Vec<&'t crate::BrushTip>,
    erase: bool,
    flow: Option<f32>,
    smudge: Option<f32>,
    clone_offset: Option<(f32, f32)>,
    clone_perspective: Option<[f32; 9]>,
    heal: bool,
    convolve: Option<f32>,
    dodge_burn: Option<f32>,
    dodge_range: Option<u8>,
    pub(crate) damage: Rect,
}

/// Byte range of one row of `rect` inside a packed RGBA buffer `width` pixels wide.
fn region_row(width: u32, rect: Rect, row: u32) -> std::ops::Range<usize> {
    let start = ((rect.y as usize + row as usize) * width as usize + rect.x as usize) * 4;
    start..start + rect.width as usize * 4
}

fn region_is_inside(pixels: &[u8], width: u32, rect: Rect) -> bool {
    rect.x >= 0
        && rect.y >= 0
        && width > 0
        && (rect.x as u64 + u64::from(rect.width)) <= u64::from(width)
        && ((rect.y as u64 + u64::from(rect.height)) * u64::from(width) * 4) <= pixels.len() as u64
}

pub(crate) fn copy_region(pixels: &[u8], width: u32, rect: Rect) -> Result<Vec<u8>> {
    if !region_is_inside(pixels, width, rect) {
        return Err(CoreError::DocumentLimitExceeded(
            "undo region outside the raster",
        ));
    }
    let mut out = Vec::with_capacity(rect.width as usize * rect.height as usize * 4);
    for row in 0..rect.height {
        out.extend_from_slice(&pixels[region_row(width, rect, row)]);
    }
    Ok(out)
}

fn paste_region(pixels: &mut [u8], width: u32, rect: Rect, bytes: &[u8]) -> Result<()> {
    let row_bytes = rect.width as usize * 4;
    if !region_is_inside(pixels, width, rect) || bytes.len() != row_bytes * rect.height as usize {
        return Err(CoreError::DocumentLimitExceeded(
            "undo region outside the raster",
        ));
    }
    for row in 0..rect.height {
        let source = row as usize * row_bytes;
        pixels[region_row(width, rect, row)].copy_from_slice(&bytes[source..source + row_bytes]);
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct BrushDabRaster {
    radius: f32,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

fn brush_dab_raster(dab: BrushPoint, size: f32, width: u32, height: u32) -> BrushDabRaster {
    let radius = (size * dab.pressure * 0.5).max(0.25);
    let canvas_width = width as f32;
    let canvas_height = height as f32;
    BrushDabRaster {
        radius,
        x0: (dab.x - radius - 1.0).floor().clamp(0.0, canvas_width) as u32,
        y0: (dab.y - radius - 1.0).floor().clamp(0.0, canvas_height) as u32,
        x1: (dab.x + radius + 1.0).ceil().clamp(0.0, canvas_width) as u32,
        y1: (dab.y + radius + 1.0).ceil().clamp(0.0, canvas_height) as u32,
    }
}

fn preflight_brush_pixel_visits(
    dabs: &[BrushPoint],
    size: f32,
    width: u32,
    height: u32,
) -> Result<()> {
    let mut pixel_visits = 0_u64;
    for &dab in dabs.iter().filter(|dab| dab.pressure > 0.0) {
        let raster = brush_dab_raster(dab, size, width, height);
        let dab_visits = u64::from(raster.x1 - raster.x0)
            .checked_mul(u64::from(raster.y1 - raster.y0))
            .ok_or(CoreError::BrushWorkLimitExceeded {
                max_pixel_visits: MAX_BRUSH_PIXEL_VISITS,
            })?;
        pixel_visits =
            pixel_visits
                .checked_add(dab_visits)
                .ok_or(CoreError::BrushWorkLimitExceeded {
                    max_pixel_visits: MAX_BRUSH_PIXEL_VISITS,
                })?;
        if pixel_visits > MAX_BRUSH_PIXEL_VISITS {
            return Err(CoreError::BrushWorkLimitExceeded {
                max_pixel_visits: MAX_BRUSH_PIXEL_VISITS,
            });
        }
    }
    Ok(())
}

fn validate_brush_settings(settings: &BrushSettings) -> Result<()> {
    if matches!(
        settings.smoothing,
        BrushSmoothing::MovingAverage { window } if !(2..=64).contains(&window)
    ) || settings.mirror_x.is_some_and(|axis| !axis.is_finite())
        || settings.mirror_y.is_some_and(|axis| !axis.is_finite())
        // Refused rather than clamped: a silently clamped brush draws something the caller did not ask
        // for and gives them no way to notice. The mask clamps internally as well, so a bypass of this
        // check still cannot divide by zero.
        || !settings.shape.is_valid()
        || !settings.spacing.is_valid()
        || settings
            .flow
            .is_some_and(|flow| !flow.is_finite() || !(0.0..=1.0).contains(&flow))
        || settings
            .smudge
            .is_some_and(|rate| !rate.is_finite() || !(0.0..=1.0).contains(&rate))
        || settings
            .clone_offset
            .is_some_and(|(dx, dy)| !dx.is_finite() || !dy.is_finite())
        || settings
            .clone_perspective
            .is_some_and(|m| m.iter().any(|v| !v.is_finite()))
        || settings
            .convolve
            .is_some_and(|v| !v.is_finite() || !(-1.0..=1.0).contains(&v))
        || settings
            .dodge_burn
            .is_some_and(|v| !v.is_finite() || !(-1.0..=1.0).contains(&v))
        || settings.dodge_range.is_some_and(|r| r > 2)
        || settings
            .ink
            .is_some_and(|v| !v.is_finite() || !(0.0..=1.0).contains(&v))
        || settings.mypaint.is_some_and(|m| !m.is_valid())
        || settings.dynamics.len() > 8
        || settings.dynamics.iter().any(|d| !d.is_valid())
        // The opacity and flow lists carry the same cap and the same per-binding rule (I.1). Capped
        // separately rather than on the total, so adding an opacity binding cannot push an existing
        // size binding out of range.
        || settings.opacity_dynamics.len() > 8
        || settings.opacity_dynamics.iter().any(|d| !d.is_valid())
        || settings.flow_dynamics.len() > 8
        || settings.flow_dynamics.iter().any(|d| !d.is_valid())
        || settings
            .symmetry_center
            .is_some_and(|(x, y)| !x.is_finite() || !y.is_finite())
        || settings.symmetry_order > 32
        || settings.assistant.is_some_and(|a| !a.is_valid())
        || settings.dyna.is_some_and(|(m, d)| {
            !m.is_finite() || !d.is_finite() || !(0.0..=1.0).contains(&m) || !(0.0..=1.0).contains(&d)
        })
    {
        return Err(CoreError::InvalidBrushSettings);
    }
    Ok(())
}

fn smooth_points(points: &[BrushPoint], smoothing: BrushSmoothing) -> Vec<BrushPoint> {
    let BrushSmoothing::MovingAverage { window } = smoothing else {
        return points.to_vec();
    };
    let window = usize::from(window);
    let mut output = Vec::with_capacity(points.len());
    for end in 0..points.len() {
        let start = (end + 1).saturating_sub(window);
        let samples = &points[start..=end];
        let divisor = samples.len() as f64;
        let x = samples.iter().map(|point| f64::from(point.x)).sum::<f64>() / divisor;
        let y = samples.iter().map(|point| f64::from(point.y)).sum::<f64>() / divisor;
        let pressure = samples
            .iter()
            .map(|point| f64::from(point.pressure))
            .sum::<f64>()
            / divisor;
        // P7: tilt is averaged like pressure, so smoothing does not drop a tablet's lean.
        let mean = |read: fn(&BrushPoint) -> f32| {
            samples.iter().map(|point| f64::from(read(point))).sum::<f64>() / divisor
        };
        output.push(
            BrushPoint::new(x as f32, y as f32, pressure as f32).with_tilt(
                mean(|point| point.tilt_x) as f32,
                mean(|point| point.tilt_y) as f32,
            ),
        );
    }
    output
}

fn mirrored_paths(points: &[BrushPoint], settings: &BrushSettings) -> Vec<Vec<BrushPoint>> {
    let mut paths = vec![points.to_vec()];
    if let Some(axis) = settings.mirror_x {
        let mirrored = points
            .iter()
            .map(|point| {
                BrushPoint::new(
                    (f64::from(axis) * 2.0 - f64::from(point.x)) as f32,
                    point.y,
                    point.pressure,
                )
            })
            .collect::<Vec<_>>();
        if !paths.contains(&mirrored) {
            paths.push(mirrored);
        }
    }
    if let Some(axis) = settings.mirror_y {
        let existing = paths.clone();
        for path in existing {
            let mirrored = path
                .iter()
                .map(|point| {
                    BrushPoint::new(
                        point.x,
                        (f64::from(axis) * 2.0 - f64::from(point.y)) as f32,
                        point.pressure,
                    )
                })
                .collect::<Vec<_>>();
            if !paths.contains(&mirrored) {
                paths.push(mirrored);
            }
        }
    }
    // Multihand radial symmetry: rotate every path so far around the centre into `order` evenly
    // spaced copies (Krita's multibrush). order<=1 adds nothing.
    if let Some((cx, cy)) = settings.symmetry_center {
        let order = settings.symmetry_order;
        if order >= 2 {
            let cx = f64::from(cx);
            let cy = f64::from(cy);
            let existing = paths.clone();
            for step in 1..order {
                let angle = std::f64::consts::TAU * f64::from(step) / f64::from(order);
                let (s, c) = angle.sin_cos();
                for path in &existing {
                    let rotated = path
                        .iter()
                        .map(|point| {
                            let dx = f64::from(point.x) - cx;
                            let dy = f64::from(point.y) - cy;
                            BrushPoint::new(
                                (cx + dx * c - dy * s) as f32,
                                (cy + dx * s + dy * c) as f32,
                                point.pressure,
                            )
                        })
                        .collect::<Vec<_>>();
                    if !paths.contains(&rotated) {
                        paths.push(rotated);
                    }
                }
            }
        }
    }
    paths
}

/// Places dabs along a path at the configured spacing.
///
/// The spacing is an ELLIPSE, not a scalar distance. Translated from Krita: the travelled |dx| and |dy| are
/// accumulated separately and a dab lands where the accumulation crosses an ellipse whose semi-axes are the
/// dab's own dimensions times the spacing fraction. For a round dab that reduces to Euclidean distance; for
/// an elliptical one -- which this product has had since dabs gained an aspect ratio -- it spaces along each
/// axis, which a scalar cannot express.
///
/// The previous rule was `(size * pressure * 0.25).max(0.5)` divided into the segment length: a hard-coded
/// quarter, no setting, and no way for an elliptical dab or a loaded tip's own spacing to matter.
/// Per-point channel values carried alongside the dab walk (I.1), and where the interpolated per-dab
/// results go. Both input slices are indexed by POINT and must be as long as `points`; a mirrored copy
/// of a stroke is a 1:1 map of its points, so the same slices serve every mirrored path.
struct DabChannels<'a> {
    opacity_in: &'a [f32],
    flow_in: &'a [f32],
    opacity_out: &'a mut Vec<f32>,
    flow_out: &'a mut Vec<f32>,
}

fn append_dabs(
    output: &mut Vec<BrushPoint>,
    points: &[BrushPoint],
    size: f32,
    shape: crate::DabShape,
    spacing: crate::SpacingOptions,
    max_dabs: usize,
    mut channels: Option<DabChannels<'_>>,
) -> Result<()> {
    if output.len() >= max_dabs {
        return Err(CoreError::InvalidBrushSettings);
    }
    // A channel list that does not cover every point is dropped rather than read past its end: a
    // partial list would scale the first dabs and silently leave the rest at full strength.
    if let Some(ch) = &channels
        && (ch.opacity_in.len() != points.len() || ch.flow_in.len() != points.len())
    {
        channels = None;
    }
    output.push(points[0]);
    if let Some(ch) = &mut channels {
        ch.opacity_out.push(ch.opacity_in[0]);
        ch.flow_out.push(ch.flow_in[0]);
    }

    let ratio = if shape.ratio.is_finite() {
        shape.ratio.clamp(0.01, 100.0)
    } else {
        1.0
    };

    // ONE walker for the whole stroke, not one per segment. The walker's accumulation is the distance
    // since the last dab, so recreating it at every segment boundary threw that distance away — and a
    // stroke whose samples are closer together than the dab spacing then never reached its second
    // dab. It painted a single dab at the first point and nothing else, which is precisely what a
    // graphics tablet's own densely-sampled stroke looks like. `spacing.rs`'s own test walks a
    // polyline this way; this caller was the one that got it wrong.
    let mut walker = crate::SpacingWalker::new(0.5, 0.5);
    for (segment, pair) in points.windows(2).enumerate() {
        let mut start = pair[0];
        let end = pair[1];
        // The dab's size follows pressure, so the spacing ellipse does too -- a light-pressure dab is
        // smaller and its dabs sit closer together, which is what keeps a tapering stroke solid.
        let pressure = start.pressure.max(end.pressure).clamp(0.0, 1.0);
        let diameter = (size * pressure).max(0.5);
        let (axis_x, axis_y) = spacing.axes(diameter, diameter * ratio);
        // Re-aims the ellipse at this segment's size while KEEPING the walked distance.
        walker.set_axes(axis_x, axis_y);

        loop {
            if output.len() >= max_dabs {
                return Err(CoreError::InvalidBrushSettings);
            }
            let dx = end.x - start.x;
            let dy = end.y - start.y;
            let Some(t) = walker.next_dab(dx, dy) else {
                break;
            };
            if !t.is_finite() {
                return Err(CoreError::InvalidBrushSettings);
            }
            let x = start.x + dx * t;
            let y = start.y + dy * t;
            // Pressure is interpolated along the ORIGINAL segment, so a dab's pressure does not drift as
            // the walk advances its own start point.
            let span_x = end.x - pair[0].x;
            let span_y = end.y - pair[0].y;
            let along = if span_x.abs() > span_y.abs() {
                if span_x.abs() < 1e-9 {
                    1.0
                } else {
                    (x - pair[0].x) / span_x
                }
            } else if span_y.abs() < 1e-9 {
                1.0
            } else {
                (y - pair[0].y) / span_y
            };
            let along = along.clamp(0.0, 1.0);
            output.push(BrushPoint::new(
                x,
                y,
                pair[0].pressure + (end.pressure - pair[0].pressure) * along,
            ));
            // The channels ride the same `along` as the pressure, so a dab's opacity and flow come from
            // the same place on the segment as its size. Using a separately-derived position would let
            // them disagree about where on the stroke the dab is.
            if let Some(ch) = &mut channels {
                let a0 = ch.opacity_in[segment];
                let a1 = ch.opacity_in[segment + 1];
                ch.opacity_out.push(a0 + (a1 - a0) * along);
                let f0 = ch.flow_in[segment];
                let f1 = ch.flow_in[segment + 1];
                ch.flow_out.push(f0 + (f1 - f0) * along);
            }
            start = BrushPoint::new(x, y, start.pressure);
            if (end.x - x).abs() < 1e-9 && (end.y - y).abs() < 1e-9 {
                break;
            }
        }
    }
    Ok(())
}

/// The scale a transform applies along each axis, from the matrix's column norms.
///
/// For a rotation or a shear the columns are not the principal axes, but their norms are the standard
/// estimate and are what a filter needs: how much source area one destination pixel covers.
fn transform_scales(transform: Affine2D) -> (f64, f64) {
    let sx = f64::from(transform.m11).hypot(f64::from(transform.m21));
    let sy = f64::from(transform.m12).hypot(f64::from(transform.m22));
    (sx, sy)
}

/// Samples with a triangle filter whose support widens as the transform shrinks.
///
/// TRANSLATED from Krita's `KisFilterWeightsBuffer` and `KisBilinearFilterStrategy`
/// (`libs/image/kis_filter_weights_buffer.h`, `libs/image/kis_filter_strategy.cc`), GPL-2.0-or-later.
///
/// A plain bilinear sample reads four texels whatever the scale factor, so shrinking reads one phase of the
/// source and discards the rest. MEASURED before this existed: a one-pixel checkerboard shrunk by four came
/// out **255 everywhere** -- pure white, where the area average is 128. Fifteen of every sixteen source
/// pixels were thrown away.
///
/// Krita's fix is to widen the filter's support in SOURCE space by `1 / scale` while evaluating its weights
/// in DESTINATION space, then normalise them. At a quarter scale that gathers four source pixels either side
/// of the centre per axis rather than one.
fn sample_rgba_filtered(
    input: &[u8],
    width: u32,
    height: u32,
    centre_x: f64,
    centre_y: f64,
    scale_x: f64,
    scale_y: f64,
) -> Pixel {
    // Krita widens only when shrinking, and stops widening past a 1/256 scale -- beyond that the support
    // would cover the whole image for every destination pixel.
    let widen_x = if scale_x < 1.0 && scale_x > 1.0 / 256.0 {
        1.0 / scale_x
    } else {
        1.0
    };
    let widen_y = if scale_y < 1.0 && scale_y > 1.0 / 256.0 {
        1.0 / scale_y
    } else {
        1.0
    };
    // The weights are evaluated in destination space, so the position step is the scale itself.
    let step_x = if widen_x > 1.0 { scale_x } else { 1.0 };
    let step_y = if widen_y > 1.0 { scale_y } else { 1.0 };

    let first_x = (centre_x - widen_x).ceil() as i64;
    let last_x = (centre_x + widen_x).floor() as i64;
    let first_y = (centre_y - widen_y).ceil() as i64;
    let last_y = (centre_y + widen_y).floor() as i64;

    let mut red = 0.0;
    let mut green = 0.0;
    let mut blue = 0.0;
    let mut alpha = 0.0;
    let mut total = 0.0;

    for source_y in first_y..=last_y {
        if source_y < 0 || source_y >= i64::from(height) {
            continue;
        }
        // Krita's bilinear strategy is the triangle `1 - |t|`, and its `weightsPositionScale` argument is
        // deliberately unused: the scaling is applied to the POSITION before the weight is evaluated.
        let weight_y = 1.0 - ((source_y as f64 - centre_y) * step_y).abs();
        if weight_y <= 0.0 {
            continue;
        }
        for source_x in first_x..=last_x {
            if source_x < 0 || source_x >= i64::from(width) {
                continue;
            }
            let weight_x = 1.0 - ((source_x as f64 - centre_x) * step_x).abs();
            if weight_x <= 0.0 {
                continue;
            }
            let weight = weight_x * weight_y;
            let offset = (source_y as usize * width as usize + source_x as usize) * 4;
            let pixel = &input[offset..offset + 4];
            // Weighted in premultiplied space, so a transparent pixel's colour cannot bleed into the
            // result. Averaging straight alpha would drag every edge toward whatever colour happens to sit
            // in the fully transparent pixels beside it.
            let pixel_alpha = f64::from(pixel[3]) / 255.0;
            red += f64::from(pixel[0]) * pixel_alpha * weight;
            green += f64::from(pixel[1]) * pixel_alpha * weight;
            blue += f64::from(pixel[2]) * pixel_alpha * weight;
            alpha += pixel_alpha * weight;
            total += weight;
        }
    }

    if total <= 0.0 {
        return Pixel::TRANSPARENT;
    }
    // Krita normalises its weight table to sum to 255. Normalising here is the same step: a triangle over a
    // widened support does not sum to one by itself.
    let out_alpha = alpha / total;
    if out_alpha <= 0.0 {
        return Pixel::TRANSPARENT;
    }
    Pixel::rgba(
        ((red / total) / out_alpha).round().clamp(0.0, 255.0) as u8,
        ((green / total) / out_alpha).round().clamp(0.0, 255.0) as u8,
        ((blue / total) / out_alpha).round().clamp(0.0, 255.0) as u8,
        (out_alpha * 255.0).round().clamp(0.0, 255.0) as u8,
    )
}

/// The 3x3 homography (row-major, 9 elements) mapping the four `src` points to the four `dst`
/// points, or None if the system is singular. Solves the standard 8x8 linear system for a projective
/// transform with h22 fixed to 1.
/// The pixel bounding box of a polygon, clamped to the canvas (exclusive max).
fn polygon_bounds(poly: &[(f64, f64)], width: u32, height: u32) -> (u32, u32, u32, u32) {
    let mut minx = f64::MAX;
    let mut miny = f64::MAX;
    let mut maxx = f64::MIN;
    let mut maxy = f64::MIN;
    for &(x, y) in poly {
        minx = minx.min(x);
        miny = miny.min(y);
        maxx = maxx.max(x);
        maxy = maxy.max(y);
    }
    let x0 = minx.floor().clamp(0.0, f64::from(width)) as u32;
    let y0 = miny.floor().clamp(0.0, f64::from(height)) as u32;
    let x1 = (maxx.ceil().clamp(0.0, f64::from(width)) as u32).max(x0);
    let y1 = (maxy.ceil().clamp(0.0, f64::from(height)) as u32).max(y0);
    (x0, y0, x1, y1)
}

/// Even-odd point-in-polygon test.
fn point_in_polygon(p: (f64, f64), poly: &[(f64, f64)]) -> bool {
    let (px, py) = p;
    let mut inside = false;
    let n = poly.len();
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi) = poly[i];
        let (xj, yj) = poly[j];
        if (yi > py) != (yj > py) {
            let t = (px - xi) < (xj - xi) * (py - yi) / (yj - yi);
            if t {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// Mean-value coordinates of `p` with respect to the closed polygon `poly` (Floater 2003). Returns
/// one normalized weight per vertex, or None when the point sits on a vertex and weights blow up (the
/// caller then just keeps the original pixel). The weights sum to 1 and reproduce `p` as their
/// weighted sum of the polygon's vertices.
fn mean_value_coords(p: (f64, f64), poly: &[(f64, f64)]) -> Option<Vec<f64>> {
    let n = poly.len();
    let (px, py) = p;
    let mut dist = Vec::with_capacity(n);
    let mut unit = Vec::with_capacity(n);
    for &(vx, vy) in poly {
        let dx = vx - px;
        let dy = vy - py;
        let d = (dx * dx + dy * dy).sqrt();
        if d < 1e-9 {
            // On a vertex: that vertex takes all the weight.
            let mut w = vec![0.0; n];
            let idx = dist.len();
            w[idx] = 1.0;
            return Some(w);
        }
        dist.push(d);
        unit.push((dx / d, dy / d));
    }
    let mut weights = vec![0.0_f64; n];
    let mut total = 0.0;
    for i in 0..n {
        let prev = if i == 0 { n - 1 } else { i - 1 };
        let next = (i + 1) % n;
        // tan(alpha/2) for the two triangles sharing vertex i.
        let t_prev = half_angle_tangent(unit[prev], unit[i]);
        let t_next = half_angle_tangent(unit[i], unit[next]);
        let w = (t_prev + t_next) / dist[i];
        weights[i] = w;
        total += w;
    }
    if !total.is_finite() || total.abs() < 1e-12 {
        return None;
    }
    for w in &mut weights {
        *w /= total;
    }
    Some(weights)
}

/// tan(theta/2) between two unit vectors, from the stable half-angle identity
/// tan(t/2) = sin(t) / (1 + cos(t)); falls back to 0 at a straight edge.
fn half_angle_tangent(a: (f64, f64), b: (f64, f64)) -> f64 {
    let cos = (a.0 * b.0 + a.1 * b.1).clamp(-1.0, 1.0);
    let sin = a.0 * b.1 - a.1 * b.0;
    let denom = 1.0 + cos;
    if denom.abs() < 1e-9 { 0.0 } else { sin / denom }
}

/// Thin-plate-spline radial kernel U(r) = r^2 log(r), with U(0) = 0.
fn tps_kernel(r2: f64) -> f64 {
    if r2 <= 1e-12 { 0.0 } else { 0.5 * r2 * r2.ln() }
}

/// Fit a thin-plate spline through `ctrl` control points so it maps each to the scalar `target[i]`.
/// Returns N+3 weights (N radial + the affine a0 + a1*x + a2*y tail), or None if the system is
/// singular. Call once per output coordinate (x and y are fitted independently).
fn tps_weights(ctrl: &[(f64, f64)], target: &[f64]) -> Option<Vec<f64>> {
    let n = ctrl.len();
    let m = n + 3;
    // Build the (N+3) x (N+3) system L * w = b.
    let mut a = vec![vec![0.0_f64; m]; m];
    let mut b = vec![0.0_f64; m];
    for i in 0..n {
        for j in 0..n {
            let dx = ctrl[i].0 - ctrl[j].0;
            let dy = ctrl[i].1 - ctrl[j].1;
            a[i][j] = tps_kernel(dx * dx + dy * dy);
        }
        a[i][n] = 1.0;
        a[i][n + 1] = ctrl[i].0;
        a[i][n + 2] = ctrl[i].1;
        // Symmetric affine constraints block.
        a[n][i] = 1.0;
        a[n + 1][i] = ctrl[i].0;
        a[n + 2][i] = ctrl[i].1;
        b[i] = target[i];
    }
    solve_linear(a, b)
}

/// Evaluate a fitted thin-plate spline at (x, y).
fn tps_eval(ctrl: &[(f64, f64)], weights: &[f64], x: f64, y: f64) -> f64 {
    let n = ctrl.len();
    let mut value = weights[n] + weights[n + 1] * x + weights[n + 2] * y;
    for i in 0..n {
        let dx = x - ctrl[i].0;
        let dy = y - ctrl[i].1;
        value += weights[i] * tps_kernel(dx * dx + dy * dy);
    }
    value
}

/// Solve the dense linear system A x = b by Gaussian elimination with partial pivoting. Returns None
/// when the matrix is singular. A and b are consumed.
fn solve_linear(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Option<Vec<f64>> {
    let n = b.len();
    for col in 0..n {
        let mut pivot = col;
        for row in (col + 1)..n {
            if a[row][col].abs() > a[pivot][col].abs() {
                pivot = row;
            }
        }
        if a[pivot][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, pivot);
        b.swap(col, pivot);
        for row in 0..n {
            if row == col {
                continue;
            }
            let factor = a[row][col] / a[col][col];
            if factor == 0.0 {
                continue;
            }
            // The pivot row is copied out first: the row being reduced and the pivot row are both
            // rows of `a`, so an index loop over the two is what clippy flags and a split borrow
            // would only obscure. `n` here is the small system size, so the copy is cheap.
            let pivot: Vec<f64> = a[col][col..n].to_vec();
            for (target, p) in a[row][col..n].iter_mut().zip(pivot.iter()) {
                *target -= factor * p;
            }
            b[row] -= factor * b[col];
        }
    }
    let mut x = vec![0.0_f64; n];
    for i in 0..n {
        x[i] = b[i] / a[i][i];
    }
    Some(x)
}

pub(crate) fn homography(src: [(f64, f64); 4], dst: [(f64, f64); 4]) -> Option<[f64; 9]> {
    // Build A (8x8) and b (8) so A * [a b c d e f g h]^T = b, with the map
    //   x' = (a x + b y + c) / (g x + h y + 1), y' = (d x + e y + f) / (g x + h y + 1).
    let mut a = [[0.0_f64; 8]; 8];
    let mut b = [0.0_f64; 8];
    for i in 0..4 {
        let (x, y) = src[i];
        let (u, v) = dst[i];
        let r = 2 * i;
        a[r] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y];
        b[r] = u;
        a[r + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y];
        b[r + 1] = v;
    }
    // Gaussian elimination with partial pivoting.
    for col in 0..8 {
        let mut pivot = col;
        for row in (col + 1)..8 {
            if a[row][col].abs() > a[pivot][col].abs() {
                pivot = row;
            }
        }
        if a[pivot][col].abs() < 1e-12 {
            return None;
        }
        a.swap(col, pivot);
        b.swap(col, pivot);
        for row in 0..8 {
            if row == col {
                continue;
            }
            let factor = a[row][col] / a[col][col];
            // Same shape as the general solver above: the pivot row is copied out so the
            // reduction is an iterator pair rather than two indexes into `a`.
            let pivot = a[col];
            for (target, p) in a[row][col..8].iter_mut().zip(pivot[col..8].iter()) {
                *target -= factor * p;
            }
            b[row] -= factor * b[col];
        }
    }
    let mut h = [0.0_f64; 9];
    for i in 0..8 {
        h[i] = b[i] / a[i][i];
    }
    h[8] = 1.0;
    Some(h)
}

fn sample_rgba(
    input: &[u8],
    width: u32,
    height: u32,
    x: f64,
    y: f64,
    sampling: SamplingMode,
    clamp: bool,
) -> Pixel {
    match sampling {
        SamplingMode::Nearest => {
            let mut source_x = x.round();
            let mut source_y = y.round();
            if clamp {
                source_x = source_x.clamp(0.0, f64::from(width - 1));
                source_y = source_y.clamp(0.0, f64::from(height - 1));
            }
            if source_x < 0.0
                || source_y < 0.0
                || source_x >= f64::from(width)
                || source_y >= f64::from(height)
            {
                return Pixel::TRANSPARENT;
            }
            let offset = (source_y as usize * width as usize + source_x as usize).saturating_mul(4);
            Pixel::from_slice(&input[offset..offset + 4])
        }
        SamplingMode::Bilinear => sample_rgba_bilinear(input, width, height, x, y, clamp),
    }
}

fn sample_rgba_bilinear(
    input: &[u8],
    width: u32,
    height: u32,
    mut x: f64,
    mut y: f64,
    clamp: bool,
) -> Pixel {
    if clamp {
        x = x.clamp(0.0, f64::from(width - 1));
        y = y.clamp(0.0, f64::from(height - 1));
    }
    let x0 = x.floor() as i64;
    let y0 = y.floor() as i64;
    let tx = x - x.floor();
    let ty = y - y.floor();
    let sample = |source_x: i64, source_y: i64| -> [f64; 4] {
        if source_x < 0
            || source_y < 0
            || source_x >= i64::from(width)
            || source_y >= i64::from(height)
        {
            return [0.0; 4];
        }
        let offset = (source_y as usize * width as usize + source_x as usize) * 4;
        let alpha = f64::from(input[offset + 3]) / 255.0;
        [
            f64::from(input[offset]) * alpha,
            f64::from(input[offset + 1]) * alpha,
            f64::from(input[offset + 2]) * alpha,
            f64::from(input[offset + 3]),
        ]
    };
    let samples = [
        (sample(x0, y0), (1.0 - tx) * (1.0 - ty)),
        (sample(x0 + 1, y0), tx * (1.0 - ty)),
        (sample(x0, y0 + 1), (1.0 - tx) * ty),
        (sample(x0 + 1, y0 + 1), tx * ty),
    ];
    let mut result = [0.0; 4];
    for (sample, weight) in samples {
        for channel in 0..4 {
            result[channel] += sample[channel] * weight;
        }
    }
    let alpha = result[3].round().clamp(0.0, 255.0) as u8;
    if alpha == 0 {
        return Pixel::TRANSPARENT;
    }
    let alpha_fraction = result[3] / 255.0;
    Pixel::rgba(
        (result[0] / alpha_fraction).round().clamp(0.0, 255.0) as u8,
        (result[1] / alpha_fraction).round().clamp(0.0, 255.0) as u8,
        (result[2] / alpha_fraction).round().clamp(0.0, 255.0) as u8,
        alpha,
    )
}
