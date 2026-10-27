// SPDX-License-Identifier: MIT OR Apache-2.0
//! Option enums for the third filter batch: map filters (Fractal Trace, Illusion, Little
//! Planet, Recursive Transform, Map Object, Small Tiles), pattern and fractal generators and the
//! decor filters.

use serde::{Deserialize, Serialize};

/// Fractal Trace: the set the image is traced through.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum FractalKind {
    #[default]
    Mandelbrot,
    Julia,
}

/// Fractal Trace: what a sample that falls outside the image shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum FractalAbyss {
    #[default]
    Loop,
    Clamp,
    Black,
    White,
    Transparent,
}

/// Illusion: how the copies are tinted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum IllusionType {
    #[default]
    Type1,
    Type2,
}

/// Map Object: the solid the image is wrapped on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum MapShape {
    #[default]
    Plane,
    Sphere,
    Box,
    Cylinder,
}

/// Small Tiles: which axes the flipped tiles are mirrored on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TileFlip {
    #[default]
    None,
    Horizontal,
    Vertical,
    Both,
}

/// Small Tiles: which tiles are flipped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TileSelection {
    #[default]
    All,
    Alternate,
    Explicit,
}

/// Fractal Explorer: the fractal family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ExplorerFractal {
    #[default]
    Mandelbrot,
    Julia,
    Barnsley1,
    Barnsley2,
    Barnsley3,
    Spider,
    ManOWar,
    Lambda,
    Sierpinski,
}

/// IFS Fractal: the built-in transform set the composer starts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum IfsPreset {
    #[default]
    Fern,
    Sierpinski,
    Dragon,
    Spiral,
    Tree,
    Snowflake,
}

/// Bayer Matrix: rotation of the pattern in quarter turns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum QuarterTurn {
    #[default]
    None,
    Quarter,
    Half,
    ThreeQuarters,
}

/// Maze: the generating algorithm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum MazeAlgorithm {
    #[default]
    DepthFirst,
    Prim,
}

/// Sinus: how the two colours are blended along the wave.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SinusBlend {
    Linear,
    Bilinear,
    #[default]
    Sinusoidal,
}

/// Spiral: how the radius grows with the angle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SpiralType {
    #[default]
    Linear,
    Logarithmic,
}

/// Spiral / Spinning Globe: sense of rotation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SpinDirection {
    #[default]
    Clockwise,
    CounterClockwise,
}

/// Jigsaw: the piece outline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum JigsawStyle {
    #[default]
    Square,
    Curved,
}

/// CML Explorer: the coupled-map function one of the HSV channels evolves by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum CmlFunction {
    KeepImage,
    KeepFirstValue,
    FillWithK,
    #[default]
    Logistic,
    DeltaSinCurve,
    SinCurve,
    SinCurveStep,
    Random,
}

/// Sphere Designer: the surface texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SphereTexture {
    #[default]
    Solid,
    Checker,
    Marble,
    Lizard,
    Phong,
    Noise,
    Wood,
    Spiral,
    Spots,
}

/// Spirograph: the family of curves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SpyroCurve {
    #[default]
    Spirograph,
    Epitrochoid,
    Sine,
    Lissajous,
}

/// Spirograph: the fixed gear's outline the moving gear rolls along.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SpyroShape {
    #[default]
    Circle,
    Polygon,
    Star,
    Sine,
    Bumps,
    Rack,
    Frame,
}

/// Lava: the built-in gradient the texture is coloured with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum LavaGradient {
    #[default]
    Incandescent,
    GermanFlag,
    CaribbeanBlues,
    Golden,
    FullSpectrum,
}

/// Rippling / Waves animation: what fills the edge the distortion exposes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum AnimEdge {
    #[default]
    Wrap,
    Smear,
    Black,
}
