//! Option enums of the fifth filter batch (v0.11 Paint-mode parity): Dodge / Burn, Index
//! Colors, the polygon Blur, Edge Detection, the Emboss family, Phong Bump Map, Palettize,
//! Cross-Channel Adjustment, Slope / Offset / Power, Raindrops, Height to Normal Map, Random Pick
//! and the single-step channel filters.

use serde::{Deserialize, Serialize};

/// Tone range a Dodge or Burn filter acts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ToneRange {
    Shadows,
    #[default]
    Midtones,
    Highlights,
}

/// Dither pattern of Index Colors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum IndexDither {
    None,
    /// 8×8 Bayer threshold matrix.
    #[default]
    Ordered,
    /// Position-hashed white noise.
    Noise,
}

/// Kernel outline of the polygon Blur.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PolyShape {
    #[default]
    Circle,
    Triangle,
    Quadrilateral,
    Pentagon,
    Hexagon,
    Octagon,
}

/// Derivative operator of Edge Detection and Height to Normal Map.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum EdgeMethod {
    /// Uniform cross-smoothing.
    Prewitt,
    /// Triangular cross-smoothing.
    #[default]
    Sobel,
    /// No cross-smoothing: the central difference only.
    Simple,
}

/// What Edge Detection writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum EdgeOutput {
    /// Gradient magnitude.
    #[default]
    Pythagorean,
    XOnly,
    YOnly,
    /// Gradient direction, 0..2π mapped to 0..1.
    Radians,
}

/// Which emboss of the family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum EmbossDirection {
    Horizontal,
    Vertical,
    /// Horizontal and vertical.
    Both,
    #[default]
    AllDirections,
    Laplacian,
    /// Lit relief at a fixed light direction, `depth` scaling the slopes.
    VariableDepth,
}

/// Source channel of a height field (Phong Bump Map, Height to Normal Map).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum HeightChannel {
    Red,
    Green,
    Blue,
    Alpha,
    #[default]
    Luminance,
}

/// One light of the Phong Bump Map.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhongLight {
    pub enabled: bool,
    /// Degrees, counter-clockwise from three o'clock on screen.
    pub azimuth: f32,
    /// Degrees above the surface.
    pub inclination: f32,
    /// Straight sRGB.
    pub color: [f32; 3],
}

impl Default for PhongLight {
    fn default() -> Self {
        PhongLight { enabled: false, azimuth: 135.0, inclination: 45.0, color: [1.0, 1.0, 1.0] }
    }
}

/// Colour space Palettize measures distances in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PaletteSpace {
    #[default]
    Rgb,
    Lab,
}

/// Palettize dithering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PaletteDither {
    #[default]
    None,
    /// 8×8 Bayer threshold.
    Pattern,
    /// Floyd–Steinberg error diffusion (serial; the output area is one tile).
    Diffusion,
    Noise,
}

/// How a Palettize pattern dither picks the colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PaletteColorMode {
    /// The threshold is added to every channel before the nearest colour is looked up.
    #[default]
    PerChannelOffset,
    /// The two nearest colours are chosen between by the threshold and the pixel's position
    /// between them.
    NearestColors,
}

/// What Palettize does with transparency.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PaletteAlphaMode {
    /// Alpha below ½ becomes 0, otherwise 1.
    #[default]
    Clip,
    /// Pixels below the alpha threshold take the palette entry `alpha_index`, opaque.
    Index,
    /// Alpha is dithered to 0 or 1 with the pattern.
    Dither,
    Unchanged,
}

/// A colour channel the Cross-Channel Adjustment reads or writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum CrossChannel {
    Red,
    Green,
    Blue,
    Alpha,
    #[default]
    Hue,
    Saturation,
    Lightness,
}

/// One curve of the Cross-Channel Adjustment: `driven` is offset by the curve evaluated at the
/// pixel's `driver` value (curve y 0.5 = no change; points `[x, y]` in 0..1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrossCurve {
    pub driven: CrossChannel,
    pub driver: CrossChannel,
    pub points: Vec<[f32; 2]>,
}

/// Where a Height to Normal Map output channel takes its value from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum NormalSwizzle {
    #[default]
    XPlus,
    XMinus,
    YPlus,
    YMinus,
    ZPlus,
    ZMinus,
}
