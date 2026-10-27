//! Option enums of the fourth filter batch (v0.11 parity): Variable Blur, Apply Lens, Mosaic,
//! Newsprint, Curve Bend, Bloom, Sobel, Cartoon, Clothify, Stencil Chrome, Bump Map, Panorama
//! Projection, Warp (map), Perlin Noise, Plasma, Solid Noise, plus the Gradient Flare designs,
//! the extra Impressionist stroke shapes and papers, and the Filmstrip number fonts.

use serde::{Deserialize, Serialize};

/// Where a per-pixel mask (Variable Blur's radius mask, Bump Map's height map) is read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum MaskSource {
    /// An external image (`mapPath` / `mapDocument`), stretched over the bounds.
    #[default]
    Map,
    /// The target layer's own layer mask (document coordinates).
    LayerMask,
    /// The layer's own pixels: luminance for a height map, alpha for a blur mask.
    SelfImage,
}

/// Mosaic tile shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum MosaicTile {
    #[default]
    Squares,
    Hexagons,
    Octagons,
    Triangles,
}

/// Newsprint colour model: which inks are screened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum NewsprintModel {
    WhiteOnBlack,
    #[default]
    BlackOnWhite,
    Rgb,
    Cmyk,
}

/// Newsprint halftone spot shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ScreenPattern {
    Line,
    #[default]
    Circle,
    Diamond,
    PsSquare,
    Cross,
}

/// Bump Map height curve.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum BumpType {
    #[default]
    Linear,
    Spherical,
    Sinusoidal,
}

/// Resampling of the projected panorama.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PanoSampler {
    Nearest,
    #[default]
    Linear,
    Cubic,
}

/// What Warp (map) samples past the edge of the bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum WarpEdge {
    #[default]
    Wrap,
    Smear,
    Black,
    ForegroundColor,
}

/// Gradient Flare built-in designs (glow palette, ray count and second-flare layout).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum FlareDesign {
    /// Warm white-yellow-red glow, fine rays, six mixed ghosts.
    #[default]
    Classic,
    /// A compact white star with long sharp rays and few ghosts.
    BrightStar,
    /// A low, wide orange sun with a halo and large faint discs.
    DistantSun,
    /// Cool blue-violet glow, soft rays, many small rings.
    ColdRings,
    /// A thin green-cyan glow whose ghosts dominate (lens reflections).
    HiddenPlanet,
    /// Only a soft white glow, no rays or ghosts unless re-enabled.
    SoftGlow,
}

/// Procedural paper textures Impressionist paints on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PaperTexture {
    /// Fine isotropic noise (the previous behaviour).
    #[default]
    Plain,
    /// Woven canvas: two crossed sinusoidal threads.
    Canvas,
    /// Coarse burlap weave.
    Burlap,
    /// Marbled veins (ridged fractal noise).
    Marble,
    /// Rough handmade paper (large fractal lumps).
    Rough,
    /// A supplied greyscale image (`paperPath`), tiled.
    Image,
}

/// Filmstrip frame-number font.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum FilmFont {
    /// 3×5 compact digits.
    #[default]
    Compact,
    /// 5×7 classic digits (the slide caption font).
    Classic,
}
