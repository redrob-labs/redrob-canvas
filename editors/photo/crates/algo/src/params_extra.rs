//! Option enums of the third filter batch (edge-preserving blurs, distortions, light and
//! shadow, noise, edge detection, artistic renders, decorations and the operation graph).

use serde::{Deserialize, Serialize};

/// Which way a line-based filter runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Axis {
    #[default]
    Horizontal,
    Vertical,
}

/// Tileable Blur kernel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum TileableBlurType {
    #[default]
    Gaussian,
    Box,
}

/// NL Filter estimator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum NlMode {
    #[default]
    AlphaTrimmedMean,
    OptimalEstimation,
    EdgeEnhancement,
}

/// What Value Propagate spreads into its neighbours.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PropagateMode {
    #[default]
    MoreWhite,
    MoreBlack,
    MiddleValue,
    ForegroundToPeaks,
    OnlyForeground,
    OnlyBackground,
    MoreOpaque,
    MoreTransparent,
}

/// Sub-pixel layout of Video Degradation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum VideoPattern {
    #[default]
    Staggered,
    LargeStaggered,
    Striped,
    WideStriped,
    LongStaggered,
    ThreeByThree,
    LargeThreeByThree,
    Hex,
    Dots,
}

/// Corner Page Curl lifts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum CurlCorner {
    UpperLeft,
    UpperRight,
    LowerLeft,
    #[default]
    LowerRight,
}

/// Colours on the back of the curled page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum CurlColors {
    #[default]
    ForegroundBackground,
    CurrentGradient,
    CurrentGradientReversed,
}

/// Long Shadow extrusion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ShadowStyle {
    #[default]
    Finite,
    Infinite,
    Fading,
    FadingFixedLength,
}

/// Long Shadow output composition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ShadowComposition {
    #[default]
    ShadowPlusImage,
    ShadowOnly,
    ShadowMinusImage,
}

/// Resampling of the projected Perspective Shadow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ShadowInterpolation {
    None,
    #[default]
    Linear,
    Cubic,
}

/// Where Sparkle takes its spike colour from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SparkleColor {
    #[default]
    NaturalColor,
    ForegroundColor,
    BackgroundColor,
}

/// Image Gradient output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum GradientOutput {
    #[default]
    Magnitude,
    Direction,
    Both,
}

/// Distance Map metric.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum DistanceMetric {
    #[default]
    Euclidean,
    Manhattan,
    Chebyshev,
}

/// Which output channel carries a normal component.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum NormalComponent {
    #[default]
    Red,
    Green,
    Blue,
}

/// Channel that drives the Van Gogh flow field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum LicChannel {
    Hue,
    Saturation,
    #[default]
    Brightness,
}

/// How the flow field is derived from the channel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum LicOperator {
    #[default]
    Derivative,
    Gradient,
}

/// What Van Gogh convolves along the field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum LicConvolve {
    #[default]
    WithWhiteNoise,
    WithSourceImage,
}

/// Waterpixels fill colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SuperpixelColor {
    #[default]
    Average,
    Random,
}

/// Impressionist stroke orientation source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum StrokeOrientation {
    #[default]
    Value,
    Radius,
    Random,
    Radial,
    Flowing,
    Hue,
    Adaptive,
}

/// Impressionist brush shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum StrokeShape {
    #[default]
    Ellipse,
    Square,
    Line,
    Splotch,
    Circle,
    Triangle,
    Diamond,
    Star,
    /// A supplied greyscale brush image (`brushPath`): bright pixels paint.
    Image,
}

/// Impressionist stroke placement.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum StrokePlacement {
    Evenly,
    #[default]
    Randomly,
}

/// Impressionist stroke colour source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum StrokeColor {
    #[default]
    Average,
    Center,
}

/// What Impressionist paints under the strokes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum StrokeBackground {
    #[default]
    KeepOriginal,
    SolidColor,
    Transparent,
}
