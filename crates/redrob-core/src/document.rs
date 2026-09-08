// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

use serde::de::{Error as DeError, IgnoredAny, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use uuid::Uuid;

use crate::command::{
    Affine2D, BrushPoint, BrushSettings, BrushSmoothing, GradientKind, GradientStop,
    MAX_BRUSH_DABS, MAX_BRUSH_PIXEL_VISITS, MAX_BRUSH_POINTS, MAX_BRUSH_SIZE, SamplingMode,
};
use crate::render::source_over;
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
}

/// Version-2 node payload. Semantic payloads are data-only foundations for later tasks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NodeContent {
    Raster { cels: Vec<RasterCel> },
    Group,
    Text { text: TextContent },
    Vector { vector: VectorContent },
}

impl NodeContent {
    pub const fn kind(&self) -> NodeKind {
        match self {
            Self::Raster { .. } => NodeKind::Raster,
            Self::Group => NodeKind::Group,
            Self::Text { .. } => NodeKind::Text,
            Self::Vector { .. } => NodeKind::Vector,
        }
    }

    pub fn raster_cels(&self) -> Option<&[RasterCel]> {
        match self {
            Self::Raster { cels } => Some(cels),
            _ => None,
        }
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

    pub(crate) fn transparent(id: LayerId, name: String, pixel_count: usize) -> Result<Self> {
        Self::transparent_at(id, name, pixel_count, FrameId::DEFAULT)
    }

    fn transparent_at(
        id: LayerId,
        name: String,
        pixel_count: usize,
        frame: FrameId,
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
            content: NodeContent::Raster {
                cels: vec![RasterCel {
                    frame,
                    pixels: RasterBytes::zeroed(pixel_count * 4),
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
            active_layer: self.active_node.ok_or(CoreError::LastLayer)?,
            layers,
            timeline,
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
    metadata: DocumentMetadata,
    #[serde(rename = "nodes", deserialize_with = "deserialize_document_nodes")]
    layers: Vec<Layer>,
    #[serde(rename = "active_node")]
    active_layer: LayerId,
    timeline: Timeline,
    selection: Selection,
}

impl Document {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        let count = pixel_count(width, height)?;
        let id = LayerId::new();
        Ok(Self {
            id: Uuid::new_v4(),
            width,
            height,
            metadata: DocumentMetadata::default(),
            layers: vec![Layer::transparent(id, "Layer 1".into(), count)?],
            active_layer: id,
            timeline: Timeline::default(),
            selection: Selection::new(width, height)?,
        })
    }

    pub fn id(&self) -> Uuid {
        self.id
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
        let bytes = pixel_count(self.width, self.height)?
            .checked_mul(4)
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

    fn active_raster_pixels_mut(&mut self) -> Result<&mut RasterBytes> {
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
        let cel_bytes = pixel_count(self.width, self.height)?
            .checked_mul(4)
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
        let layer = Layer::transparent_at(id, name, pixels, self.current_frame_id())?;
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

    pub(crate) fn brush_stroke(
        &mut self,
        points: &[BrushPoint],
        color: Pixel,
        size: f32,
        opacity: f32,
        settings: BrushSettings,
    ) -> Result<()> {
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
            if settings.mirror_x.is_some_and(|axis| {
                (f64::from(axis) * 2.0 - f64::from(point.x)).abs() > f64::from(f32::MAX)
            }) || settings.mirror_y.is_some_and(|axis| {
                (f64::from(axis) * 2.0 - f64::from(point.y)).abs() > f64::from(f32::MAX)
            }) {
                return Err(CoreError::InvalidBrushSettings);
            }
        }
        let processed = smooth_points(points, settings.smoothing);
        let paths = mirrored_paths(&processed, settings);
        let max_dabs = (pixel_count(self.width, self.height)?
            .saturating_mul(16)
            .saturating_add(points.len()))
        .min(MAX_BRUSH_DABS);
        let mut dabs = Vec::new();
        for path in paths {
            append_dabs(&mut dabs, &path, size, max_dabs)?;
        }
        preflight_brush_pixel_visits(&dabs, size, self.width, self.height)?;

        let mask = self.selection.clone();
        let width = self.width;
        let height = self.height;
        let pixels = self.active_raster_pixels_mut()?;
        for dab in dabs {
            if dab.pressure <= 0.0 {
                continue;
            }
            let raster = brush_dab_raster(dab, size, width, height);
            for y in raster.y0..raster.y1 {
                for x in raster.x0..raster.x1 {
                    let distance = ((x as f32 + 0.5 - dab.x).powi(2)
                        + (y as f32 + 0.5 - dab.y).powi(2))
                    .sqrt();
                    let edge = (raster.radius + 0.5 - distance).clamp(0.0, 1.0);
                    let selection = f32::from(mask.coverage(x, y)) / 255.0;
                    let alpha =
                        f32::from(color.a) / 255.0 * opacity * dab.pressure * edge * selection;
                    if alpha <= 0.0 {
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
        Ok(())
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
        let bytes_per_pixel = rgba_cels
            .checked_mul(4)
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
                        crop_bytes(&cel.pixels, old_width, old_height, rect, 4, count).into();
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
                let sampled = sample_rgba(
                    &original,
                    width,
                    height,
                    source_center_x - 0.5,
                    source_center_y - 0.5,
                    sampling,
                    false,
                );
                let offset = (y as usize * width as usize + x as usize) * 4;
                sampled.write_to(&mut output[offset..offset + 4]);
            }
        }
        self.replace_active_pixels(output)
    }

    pub(crate) fn replace_active_pixels(&mut self, pixels: Vec<u8>) -> Result<()> {
        let expected = pixel_count(self.width, self.height)? * 4;
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
            layers: vec![Layer::from_rgba(
                id,
                "Imported image".into(),
                pixels,
                count,
            )?],
            active_layer: id,
            timeline: Timeline::default(),
            selection: Selection::new(width, height)?,
        })
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
            layers,
            active_layer,
            timeline: Timeline::default(),
            selection,
        }
    }

    pub(crate) fn validate(&self) -> Result<()> {
        let count = pixel_count(self.width, self.height)?;
        let expected_rgba = count.checked_mul(4).ok_or(CoreError::InvalidDimensions {
            width: self.width,
            height: self.height,
        })?;
        validate_timeline(&self.timeline)?;
        validate_metadata(&self.metadata)?;
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
        NodeContent::Raster { .. } | NodeContent::Group => Ok(SemanticUsage::default()),
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
    }
    Ok(())
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

fn validate_brush_settings(settings: BrushSettings) -> Result<()> {
    if matches!(
        settings.smoothing,
        BrushSmoothing::MovingAverage { window } if !(2..=64).contains(&window)
    ) || settings.mirror_x.is_some_and(|axis| !axis.is_finite())
        || settings.mirror_y.is_some_and(|axis| !axis.is_finite())
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
        output.push(BrushPoint::new(x as f32, y as f32, pressure as f32));
    }
    output
}

fn mirrored_paths(points: &[BrushPoint], settings: BrushSettings) -> Vec<Vec<BrushPoint>> {
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
    paths
}

fn append_dabs(
    output: &mut Vec<BrushPoint>,
    points: &[BrushPoint],
    size: f32,
    max_dabs: usize,
) -> Result<()> {
    if output.len() >= max_dabs {
        return Err(CoreError::InvalidBrushSettings);
    }
    output.push(points[0]);
    for pair in points.windows(2) {
        let start = pair[0];
        let end = pair[1];
        let dx = f64::from(end.x) - f64::from(start.x);
        let dy = f64::from(end.y) - f64::from(start.y);
        let distance = dx.hypot(dy);
        let spacing =
            (f64::from(size) * f64::from(start.pressure.max(end.pressure)) * 0.25).max(0.5);
        let steps = (distance / spacing).ceil().max(1.0);
        if !steps.is_finite() || steps > (max_dabs - output.len()) as f64 {
            return Err(CoreError::InvalidBrushSettings);
        }
        let steps = steps as usize;
        for step in 1..=steps {
            let amount = step as f64 / steps as f64;
            output.push(BrushPoint::new(
                (f64::from(start.x) + dx * amount) as f32,
                (f64::from(start.y) + dy * amount) as f32,
                (f64::from(start.pressure) + f64::from(end.pressure - start.pressure) * amount)
                    as f32,
            ));
        }
    }
    Ok(())
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
