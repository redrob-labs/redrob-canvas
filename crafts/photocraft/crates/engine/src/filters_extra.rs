//! Third filter batch: Blur (Mean Curvature, Tileable, Symmetric Nearest Neighbor), Other
//! (Antialias, Distance Map, Normal Map, Operation Graph), Noise (NL Filter, CIE lch, HSV, Hurl,
//! Pick, Slur), Distort (Engrave, Kaleidoscope, Shift, Value Propagate, Video Degradation, Blinds,
//! Page Curl), Light and Shadow (Supernova, Gradient Flare, Sparkle, Long Shadow, Perspective
//! Shadow, Xach-Effect), Stylize (Difference of Gaussians, Laplace, Image Gradient), Artistic
//! (Cubism, SLIC, Waterpixels, Impressionist, Van Gogh, Weave) and Decor (Filmstrip, Fuzzy
//! Border).
//!
//! They run through [`crate::filters::run_filter`] like every other filter; the ones that read
//! the foreground/background colours get them injected by [`crate::filters_ext::prepare`].

use redrob_photo_algo::{
    Axis, CurlColors, CurlCorner, DistanceMetric, FilterParams, GradientOutput, LicChannel, LicConvolve, LicOperator, NlMode, NormalComponent, PropagateMode,
    ShadowComposition, ShadowInterpolation, ShadowStyle, SparkleColor, StrokeBackground, StrokeColor, StrokeOrientation, StrokePlacement, StrokeShape,
    SuperpixelColor, TileableBlurType, VideoPattern,
};
use serde_json::Value;

use crate::commands::CommandSpec;

fn f(p: &Value, k: &str, d: f32) -> f32 {
    p.get(k).and_then(Value::as_f64).map_or(d, |v| v as f32)
}
fn i(p: &Value, k: &str, d: i64) -> i64 {
    p.get(k).and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f.round() as i64))).unwrap_or(d)
}
fn b(p: &Value, k: &str, d: bool) -> bool {
    p.get(k).and_then(Value::as_bool).unwrap_or(d)
}
fn s<'a>(p: &'a Value, k: &str, d: &'a str) -> &'a str {
    p.get(k).and_then(Value::as_str).unwrap_or(d)
}
fn seed(p: &Value) -> u32 {
    i(p, "seed", 0).clamp(0, u32::MAX as i64) as u32
}
/// Straight RGBA colour from `[r, g, b(, a)]` (0–1) or `#rrggbb`.
fn colour(p: &Value, k: &str, d: [f32; 4]) -> [f32; 4] {
    match p.get(k) {
        Some(Value::Array(a)) if a.len() >= 3 => {
            let c = |j: usize, dv: f32| a.get(j).and_then(Value::as_f64).map_or(dv, |v| v as f32);
            [c(0, 0.0), c(1, 0.0), c(2, 0.0), c(3, 1.0)]
        }
        Some(Value::String(h)) if h.len() == 7 && h.starts_with('#') => {
            let ch = |j: usize| h.get(j..j + 2).and_then(|s| u8::from_str_radix(s, 16).ok()).map_or(0.0, |v| v as f32 / 255.0);
            [ch(1), ch(3), ch(5), 1.0]
        }
        _ => d,
    }
}
fn axis(p: &Value, k: &str) -> Axis {
    if s(p, k, "horizontal") == "vertical" { Axis::Vertical } else { Axis::Horizontal }
}

const BLACK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// Ids whose rendering uses the foreground/background colours.
pub(crate) fn uses_colours(id: &str) -> bool {
    matches!(id, "filter.distort.pageCurl" | "filter.lightShadow.sparkle")
}

/// Algorithm parameters for the ids this module owns (dialog units).
pub fn params_for(id: &str, p: &Value) -> Option<FilterParams> {
    Some(match id {
        // ---- Blur ----
        "filter.blur.meanCurvatureBlur" => FilterParams::MeanCurvatureBlur { iterations: i(p, "iterations", 20).clamp(1, 500) as u32 },
        "filter.blur.tileableBlur" => FilterParams::TileableBlur {
            radius: f(p, "radius", 5.0).clamp(0.0, 128.0),
            horizontal: b(p, "blurHorizontally", true),
            vertical: b(p, "blurVertically", true),
            blur_type: if s(p, "blurType", "gaussian") == "box" { TileableBlurType::Box } else { TileableBlurType::Gaussian },
        },
        "filter.other.antialias" => FilterParams::Antialias,
        "filter.blur.symmetricNearestNeighbor" => {
            FilterParams::SymmetricNearestNeighbor { radius: f(p, "radius", 8.0).clamp(1.0, 100.0), pairs: i(p, "pairs", 2).clamp(1, 2) as u32 }
        }
        "filter.noise.nlFilter" => FilterParams::NlFilter {
            alpha: f(p, "alpha", 0.3).clamp(0.0, 1.0),
            radius: f(p, "radius", 0.3).clamp(0.0, 1.0),
            mode: match s(p, "mode", "alphaTrimmedMean") {
                "optimalEstimation" => NlMode::OptimalEstimation,
                "edgeEnhancement" => NlMode::EdgeEnhancement,
                _ => NlMode::AlphaTrimmedMean,
            },
        },
        // ---- Distort ----
        "filter.distort.engrave" => FilterParams::Engrave { height: f(p, "height", 10.0).clamp(2.0, 16.0), limit_line_width: b(p, "limitLineWidth", false) },
        "filter.distort.kaleidoscope" => FilterParams::Kaleidoscope {
            mirrors: i(p, "mirrors", 6).clamp(1, 24) as u32,
            mirror_rotation: f(p, "mirrorRotation", 0.0).clamp(0.0, 360.0),
            result_rotation: f(p, "resultRotation", 0.0).clamp(0.0, 360.0),
            offset_x: f(p, "offsetX", 0.5).clamp(0.0, 1.0),
            offset_y: f(p, "offsetY", 0.5).clamp(0.0, 1.0),
            zoom: f(p, "zoom", 100.0).clamp(1.0, 1000.0),
            trim_x: f(p, "trimX", 0.0).clamp(0.0, 0.9),
            trim_y: f(p, "trimY", 0.0).clamp(0.0, 0.9),
            clip: b(p, "clipResult", false),
        },
        "filter.distort.shift" => FilterParams::Shift { amount: f(p, "amount", 5.0).clamp(0.0, 200.0), axis: axis(p, "direction"), seed: seed(p) },
        "filter.distort.valuePropagate" => FilterParams::ValuePropagate {
            mode: match s(p, "mode", "moreWhite") {
                "moreBlack" => PropagateMode::MoreBlack,
                "middleValue" => PropagateMode::MiddleValue,
                "foregroundToPeaks" => PropagateMode::ForegroundToPeaks,
                "onlyForeground" => PropagateMode::OnlyForeground,
                "onlyBackground" => PropagateMode::OnlyBackground,
                "moreOpaque" => PropagateMode::MoreOpaque,
                "moreTransparent" => PropagateMode::MoreTransparent,
                _ => PropagateMode::MoreWhite,
            },
            lower_threshold: f(p, "lowerThreshold", 0.0).clamp(0.0, 1.0),
            upper_threshold: f(p, "upperThreshold", 1.0).clamp(0.0, 1.0),
            rate: f(p, "propagatingRate", 1.0).clamp(0.0, 1.0),
            color: colour(p, "color", BLACK),
            top: b(p, "toTop", true),
            left: b(p, "toLeft", true),
            right: b(p, "toRight", true),
            bottom: b(p, "toBottom", true),
        },
        "filter.distort.videoDegradation" => FilterParams::VideoDegradation {
            pattern: match s(p, "pattern", "staggered") {
                "largeStaggered" => VideoPattern::LargeStaggered,
                "striped" => VideoPattern::Striped,
                "wideStriped" => VideoPattern::WideStriped,
                "longStaggered" => VideoPattern::LongStaggered,
                "threeByThree" => VideoPattern::ThreeByThree,
                "largeThreeByThree" => VideoPattern::LargeThreeByThree,
                "hex" => VideoPattern::Hex,
                "dots" => VideoPattern::Dots,
                _ => VideoPattern::Staggered,
            },
            additive: b(p, "additive", true),
            rotated: b(p, "rotated", false),
        },
        "filter.distort.blinds" => FilterParams::Blinds {
            displacement: f(p, "displacement", 30.0).clamp(1.0, 90.0),
            segments: i(p, "segments", 3).clamp(1, 10) as u32,
            axis: axis(p, "orientation"),
            transparent: b(p, "transparent", false),
            background: colour(p, "background", WHITE),
        },
        "filter.distort.pageCurl" => FilterParams::PageCurl {
            corner: match s(p, "location", "lowerRight") {
                "upperLeft" => CurlCorner::UpperLeft,
                "upperRight" => CurlCorner::UpperRight,
                "lowerLeft" => CurlCorner::LowerLeft,
                _ => CurlCorner::LowerRight,
            },
            axis: axis(p, "orientation"),
            shade: b(p, "shade", true),
            colors: match s(p, "colors", "foregroundBackground") {
                "currentGradient" => CurlColors::CurrentGradient,
                "currentGradientReversed" => CurlColors::CurrentGradientReversed,
                _ => CurlColors::ForegroundBackground,
            },
            opacity: f(p, "opacity", 100.0).clamp(0.0, 100.0),
            foreground: colour(p, "foreground", BLACK),
            background: colour(p, "background", WHITE),
        },
        // ---- Light and Shadow ----
        "filter.lightShadow.supernova" => FilterParams::Supernova {
            center_x: f(p, "centerX", 0.5).clamp(0.0, 1.0),
            center_y: f(p, "centerY", 0.5).clamp(0.0, 1.0),
            radius: f(p, "radius", 20.0).clamp(1.0, 3000.0),
            spokes: i(p, "spokes", 100).clamp(1, 1024) as u32,
            random_hue: f(p, "randomHue", 0.0).clamp(0.0, 360.0),
            color: colour(p, "color", [0.35, 0.4, 1.0, 1.0]),
            seed: seed(p),
        },
        "filter.lightShadow.gradientFlare" => FilterParams::GradientFlare {
            center_x: f(p, "centerX", 0.5).clamp(0.0, 1.0),
            center_y: f(p, "centerY", 0.5).clamp(0.0, 1.0),
            radius: f(p, "radius", 100.0).clamp(1.0, 3000.0),
            rotation: f(p, "rotation", 0.0).clamp(-180.0, 180.0),
            hue_rotation: f(p, "hueRotation", 0.0).clamp(-180.0, 180.0),
            vector_angle: f(p, "vectorAngle", 60.0).clamp(0.0, 360.0),
            vector_length: f(p, "vectorLength", 400.0).clamp(0.0, 10000.0),
            glow: b(p, "glow", true),
            rays: b(p, "rays", true),
            second_flares: b(p, "secondFlares", true),
        },
        "filter.lightShadow.sparkle" => FilterParams::Sparkle {
            threshold: f(p, "luminosityThreshold", 0.001).clamp(0.0, 0.1),
            flare_intensity: f(p, "flareIntensity", 0.5).clamp(0.0, 1.0),
            spike_length: f(p, "spikeLength", 20.0).clamp(1.0, 100.0),
            spike_points: i(p, "spikePoints", 4).clamp(0, 16) as u32,
            spike_angle: f(p, "spikeAngle", 15.0).clamp(-1.0, 360.0),
            density: f(p, "spikeDensity", 1.0).clamp(0.0, 1.0),
            transparency: f(p, "transparency", 0.0).clamp(0.0, 1.0),
            random_hue: f(p, "randomHue", 0.0).clamp(0.0, 1.0),
            random_saturation: f(p, "randomSaturation", 0.0).clamp(0.0, 1.0),
            preserve_luminosity: b(p, "preserveLuminosity", false),
            inverse: b(p, "inverse", false),
            border: b(p, "addBorder", false),
            color: match s(p, "colorType", "naturalColor") {
                "foregroundColor" => SparkleColor::ForegroundColor,
                "backgroundColor" => SparkleColor::BackgroundColor,
                _ => SparkleColor::NaturalColor,
            },
            foreground: colour(p, "foreground", BLACK),
            background: colour(p, "background", WHITE),
            seed: seed(p),
        },
        "filter.lightShadow.longShadow" => FilterParams::LongShadow {
            style: match s(p, "style", "finite") {
                "infinite" => ShadowStyle::Infinite,
                "fading" => ShadowStyle::Fading,
                "fadingFixedLength" => ShadowStyle::FadingFixedLength,
                _ => ShadowStyle::Finite,
            },
            angle: f(p, "angle", 45.0).clamp(-180.0, 180.0),
            length: f(p, "length", 100.0).clamp(0.0, 1000.0),
            midpoint: f(p, "midpoint", 100.0).clamp(0.0, 1000.0),
            color: colour(p, "color", BLACK),
            composition: match s(p, "composition", "shadowPlusImage") {
                "shadowOnly" => ShadowComposition::ShadowOnly,
                "shadowMinusImage" => ShadowComposition::ShadowMinusImage,
                _ => ShadowComposition::ShadowPlusImage,
            },
        },
        "filter.lightShadow.perspectiveShadow" => FilterParams::PerspectiveShadow {
            angle: f(p, "angle", 45.0).clamp(0.0, 180.0),
            distance: f(p, "relativeDistance", 5.0).clamp(0.1, 24.0),
            length: f(p, "shadowLength", 1.0).clamp(0.1, 24.0),
            blur: f(p, "blurRadius", 3.0).clamp(0.0, 1024.0),
            color: colour(p, "color", BLACK),
            opacity: f(p, "opacity", 80.0).clamp(0.0, 100.0),
            interpolation: match s(p, "interpolation", "linear") {
                "none" => ShadowInterpolation::None,
                "cubic" => ShadowInterpolation::Cubic,
                _ => ShadowInterpolation::Linear,
            },
            allow_resizing: b(p, "allowResizing", false),
        },
        "filter.lightShadow.xachEffect" => FilterParams::XachEffect {
            highlight_x: f(p, "highlightXOffset", -1.0).clamp(-100.0, 100.0),
            highlight_y: f(p, "highlightYOffset", -1.0).clamp(-100.0, 100.0),
            highlight_color: colour(p, "highlightColor", WHITE),
            highlight_opacity: f(p, "highlightOpacity", 66.0).clamp(0.0, 255.0),
            shadow_color: colour(p, "dropShadowColor", BLACK),
            shadow_opacity: f(p, "dropShadowOpacity", 100.0).clamp(0.0, 255.0),
            blur: f(p, "dropShadowBlurRadius", 12.0).clamp(0.0, 255.0),
            shadow_x: f(p, "dropShadowXOffset", 5.0).clamp(-100.0, 100.0),
            shadow_y: f(p, "dropShadowYOffset", 5.0).clamp(-100.0, 100.0),
        },
        // ---- Noise ----
        "filter.noise.cieLchNoise" => FilterParams::LchNoise {
            dulling: i(p, "dulling", 2).clamp(1, 8) as u32,
            lightness: f(p, "lightness", 40.0).clamp(0.0, 100.0),
            chroma: f(p, "chroma", 40.0).clamp(0.0, 100.0),
            hue: f(p, "hue", 3.0).clamp(0.0, 180.0),
            seed: seed(p),
        },
        "filter.noise.hsvNoise" => FilterParams::HsvNoise {
            dulling: i(p, "dulling", 2).clamp(1, 8) as u32,
            hue: f(p, "hue", 3.0).clamp(0.0, 180.0),
            saturation: f(p, "saturation", 0.04).clamp(0.0, 1.0),
            value: f(p, "value", 0.04).clamp(0.0, 1.0),
            seed: seed(p),
        },
        "filter.noise.hurl" => {
            FilterParams::Hurl { randomization: f(p, "randomization", 50.0).clamp(0.0, 100.0), repeat: i(p, "repeat", 1).clamp(1, 100) as u32, seed: seed(p) }
        }
        "filter.noise.pick" => {
            FilterParams::Pick { randomization: f(p, "randomization", 50.0).clamp(0.0, 100.0), repeat: i(p, "repeat", 1).clamp(1, 100) as u32, seed: seed(p) }
        }
        "filter.noise.slur" => {
            FilterParams::Slur { randomization: f(p, "randomization", 50.0).clamp(0.0, 100.0), repeat: i(p, "repeat", 1).clamp(1, 100) as u32, seed: seed(p) }
        }
        // ---- Stylize (edges) ----
        "filter.stylize.differenceOfGaussians" => FilterParams::DifferenceOfGaussians {
            radius1: f(p, "radius1", 1.0).clamp(0.0, 10.0),
            radius2: f(p, "radius2", 2.0).clamp(0.0, 10.0),
            normalize: b(p, "normalize", true),
            invert: b(p, "invert", false),
        },
        "filter.stylize.laplace" => FilterParams::Laplace,
        "filter.stylize.imageGradient" => FilterParams::ImageGradient {
            output: match s(p, "output", "magnitude") {
                "direction" => GradientOutput::Direction,
                "both" => GradientOutput::Both,
                _ => GradientOutput::Magnitude,
            },
        },
        // ---- Other ----
        "filter.other.distanceMap" => FilterParams::DistanceMap {
            metric: match s(p, "metric", "euclidean") {
                "manhattan" => DistanceMetric::Manhattan,
                "chebyshev" => DistanceMetric::Chebyshev,
                _ => DistanceMetric::Euclidean,
            },
            threshold_low: f(p, "thresholdLow", 0.0001).clamp(0.0, 1.0),
            threshold_high: f(p, "thresholdHigh", 1.0).clamp(0.0, 1.0),
            averaging: f(p, "grayscaleAveraging", 0.0).clamp(0.0, 1000.0),
            normalize: b(p, "normalize", true),
        },
        "filter.other.normalMap" => {
            let comp = |k: &str, d: &str| match s(p, k, d) {
                "red" => NormalComponent::Red,
                "blue" => NormalComponent::Blue,
                _ => NormalComponent::Green,
            };
            FilterParams::NormalMap {
                scale: f(p, "scale", 10.0).clamp(0.0, 255.0),
                x_component: comp("xComponent", "red"),
                y_component: comp("yComponent", "green"),
                flip_x: b(p, "flipX", false),
                flip_y: b(p, "flipY", false),
                full_z: b(p, "fullZ", false),
                tileable: b(p, "tileable", false),
            }
        }
        "filter.other.operationGraph" => FilterParams::OperationGraph { source: s(p, "graph", "").to_owned() },
        // ---- Artistic ----
        "filter.artistic.cubism" => FilterParams::Cubism {
            tile_size: f(p, "tileSize", 10.0).clamp(0.0, 256.0),
            saturation: f(p, "tileSaturation", 2.5).clamp(0.0, 10.0),
            background: colour(p, "background", BLACK),
            seed: seed(p),
        },
        "filter.artistic.slic" => FilterParams::Slic {
            cluster_size: f(p, "regionsSize", 32.0).clamp(2.0, 1024.0),
            compactness: f(p, "compactness", 20.0).clamp(1.0, 40.0),
            iterations: i(p, "iterations", 1).clamp(1, 30) as u32,
        },
        "filter.artistic.waterpixels" => FilterParams::Waterpixels {
            size: f(p, "superpixelsSize", 32.0).clamp(8.0, 256.0),
            smoothness: f(p, "gradientSmoothness", 1.0).clamp(0.0, 1000.0),
            regularization: f(p, "spatialRegularization", 0.0).clamp(0.0, 50.0),
            color: if s(p, "superpixelsColor", "average") == "random" { SuperpixelColor::Random } else { SuperpixelColor::Average },
            seed: seed(p),
        },
        "filter.artistic.impressionist" => FilterParams::Impressionist {
            brush_size: f(p, "brushSize", 20.0).clamp(1.0, 200.0),
            size_variation: f(p, "sizeVariation", 0.3).clamp(0.0, 1.0),
            orientation: f(p, "orientation", 45.0).clamp(0.0, 360.0),
            orientation_type: match s(p, "orientationType", "value") {
                "radius" => StrokeOrientation::Radius,
                "random" => StrokeOrientation::Random,
                "radial" => StrokeOrientation::Radial,
                "flowing" => StrokeOrientation::Flowing,
                "hue" => StrokeOrientation::Hue,
                "adaptive" => StrokeOrientation::Adaptive,
                _ => StrokeOrientation::Value,
            },
            density: f(p, "strokeDensity", 20.0).clamp(1.0, 50.0),
            shape: match s(p, "brushShape", "ellipse") {
                "square" => StrokeShape::Square,
                "line" => StrokeShape::Line,
                "splotch" => StrokeShape::Splotch,
                _ => StrokeShape::Ellipse,
            },
            paper_relief: f(p, "paperRelief", 0.0).clamp(0.0, 100.0),
            placement: if s(p, "placement", "randomly") == "evenly" { StrokePlacement::Evenly } else { StrokePlacement::Randomly },
            color: if s(p, "strokeColor", "average") == "center" { StrokeColor::Center } else { StrokeColor::Average },
            background: match s(p, "background", "keepOriginal") {
                "solidColor" => StrokeBackground::SolidColor,
                "transparent" => StrokeBackground::Transparent,
                _ => StrokeBackground::KeepOriginal,
            },
            background_color: colour(p, "backgroundColor", WHITE),
            seed: seed(p),
        },
        "filter.artistic.vanGogh" => FilterParams::VanGogh {
            channel: match s(p, "effectChannel", "brightness") {
                "hue" => LicChannel::Hue,
                "saturation" => LicChannel::Saturation,
                _ => LicChannel::Brightness,
            },
            operator: if s(p, "effectOperator", "derivative") == "gradient" { LicOperator::Gradient } else { LicOperator::Derivative },
            convolve: if s(p, "convolve", "withWhiteNoise") == "withSourceImage" { LicConvolve::WithSourceImage } else { LicConvolve::WithWhiteNoise },
            filter_length: f(p, "filterLength", 5.0).clamp(0.1, 64.0),
            noise_magnitude: f(p, "noiseMagnitude", 2.0).clamp(1.0, 5.0),
            integration_steps: i(p, "integrationSteps", 25).clamp(1, 40) as u32,
            minimum_value: f(p, "minimumValue", -25.0).clamp(-100.0, 0.0),
            maximum_value: f(p, "maximumValue", 25.0).clamp(0.0, 100.0),
            seed: seed(p),
        },
        "filter.artistic.weave" => FilterParams::Weave {
            ribbon_width: f(p, "ribbonWidth", 30.0).clamp(0.0, 256.0),
            ribbon_spacing: f(p, "ribbonSpacing", 10.0).clamp(0.0, 256.0),
            shadow_darkness: f(p, "shadowDarkness", 75.0).clamp(0.0, 100.0),
            shadow_depth: f(p, "shadowDepth", 75.0).clamp(0.0, 100.0),
            thread_length: f(p, "threadLength", 200.0).clamp(0.0, 256.0),
            thread_density: f(p, "threadDensity", 50.0).clamp(0.0, 256.0),
            thread_intensity: f(p, "threadIntensity", 100.0).clamp(0.0, 100.0),
        },
        // ---- Decor ----
        "filter.decor.filmstrip" => FilterParams::Filmstrip {
            color: colour(p, "filmColor", BLACK),
            number_color: colour(p, "numberColor", [0.93, 0.61, 0.0, 1.0]),
            number_start: i(p, "startIndex", 1).clamp(0, 9999) as i32,
            numbers_top: b(p, "atTop", true),
            numbers_bottom: b(p, "atBottom", true),
            hole_offset: f(p, "holeOffset", 0.04).clamp(0.0, 0.3),
            hole_width: f(p, "holeWidth", 0.056).clamp(0.005, 0.5),
            hole_height: f(p, "holeHeight", 0.08).clamp(0.005, 0.3),
            hole_spacing: f(p, "holeSpacing", 0.08).clamp(0.01, 1.0),
        },
        "filter.decor.fuzzyBorder" => FilterParams::FuzzyBorder {
            color: colour(p, "color", WHITE),
            size: f(p, "borderSize", 16.0).clamp(1.0, 300.0),
            blur: b(p, "blurBorder", true),
            granularity: f(p, "granularity", 4.0).clamp(1.0, 16.0),
            shadow: b(p, "addShadow", false),
            shadow_weight: f(p, "shadowWeight", 100.0).clamp(0.0, 100.0),
            seed: seed(p),
        },
        _ => return None,
    })
}

macro_rules! cmd {
    ($id:literal, $label:literal, [$($m:literal),*], $params:literal) => {
        CommandSpec {
            id: $id,
            label: $label,
            menu: &[$($m),*],
            shortcut: None,
            params: $params,
            enabled: crate::filters::has_filterable_layer,
            run: |s, p| crate::filters::run_filter(s, $id, p),
            journal: true,
        }
    };
}

/// The command specs of this batch.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        // Blur
        cmd!("filter.blur.meanCurvatureBlur", "Mean Curvature Blur…", ["Filter", "Blur"], r##"{"iterations":1..500=20}"##),
        cmd!(
            "filter.blur.tileableBlur",
            "Tileable Blur…",
            ["Filter", "Blur"],
            r##"{"radius":0..128=5,"blurHorizontally":bool=true,"blurVertically":bool=true,"blurType":"gaussian|box"}"##
        ),
        cmd!("filter.blur.symmetricNearestNeighbor", "Symmetric Nearest Neighbor…", ["Filter", "Blur"], r##"{"radius":1..100=8,"pairs":1..2=2}"##),
        // Noise
        cmd!(
            "filter.noise.nlFilter",
            "NL Filter…",
            ["Filter", "Noise"],
            r##"{"alpha":0..1=0.3,"radius":0..1=0.3,"mode":"alphaTrimmedMean|optimalEstimation|edgeEnhancement"}"##
        ),
        cmd!(
            "filter.noise.cieLchNoise",
            "CIE lch Noise…",
            ["Filter", "Noise"],
            r##"{"dulling":1..8=2,"lightness":0..100=40,"chroma":0..100=40,"hue":0..180=3,"seed":u32=0}"##
        ),
        cmd!(
            "filter.noise.hsvNoise",
            "HSV Noise…",
            ["Filter", "Noise"],
            r##"{"dulling":1..8=2,"hue":0..180=3,"saturation":0..1=0.04,"value":0..1=0.04,"seed":u32=0}"##
        ),
        cmd!("filter.noise.hurl", "Hurl…", ["Filter", "Noise"], r##"{"randomization":0..100=50,"repeat":1..100=1,"seed":u32=0}"##),
        cmd!("filter.noise.pick", "Pick…", ["Filter", "Noise"], r##"{"randomization":0..100=50,"repeat":1..100=1,"seed":u32=0}"##),
        cmd!("filter.noise.slur", "Slur…", ["Filter", "Noise"], r##"{"randomization":0..100=50,"repeat":1..100=1,"seed":u32=0}"##),
        // Distort
        cmd!("filter.distort.engrave", "Engrave…", ["Filter", "Distort"], r##"{"height":2..16=10,"limitLineWidth":bool}"##),
        cmd!(
            "filter.distort.kaleidoscope",
            "Kaleidoscope…",
            ["Filter", "Distort"],
            r##"{"mirrors":1..24=6,"mirrorRotation":0..360=0,"resultRotation":0..360=0,"offsetX":0..1=0.5,"offsetY":0..1=0.5,"zoom":1..1000=100,"trimX":0..0.9=0,"trimY":0..0.9=0,"clipResult":bool}"##
        ),
        cmd!("filter.distort.shift", "Shift…", ["Filter", "Distort"], r##"{"amount":0..200=5,"direction":"horizontal|vertical","seed":u32=0}"##),
        cmd!(
            "filter.distort.valuePropagate",
            "Value Propagate…",
            ["Filter", "Distort"],
            r##"{"mode":"moreWhite|moreBlack|middleValue|foregroundToPeaks|onlyForeground|onlyBackground|moreOpaque|moreTransparent","lowerThreshold":0..1=0,"upperThreshold":0..1=1,"propagatingRate":0..1=1,"color":color=#000000,"toTop":bool=true,"toLeft":bool=true,"toRight":bool=true,"toBottom":bool=true}"##
        ),
        cmd!(
            "filter.distort.videoDegradation",
            "Video Degradation…",
            ["Filter", "Distort"],
            r##"{"pattern":"staggered|largeStaggered|striped|wideStriped|longStaggered|threeByThree|largeThreeByThree|hex|dots","additive":bool=true,"rotated":bool}"##
        ),
        cmd!(
            "filter.distort.blinds",
            "Blinds…",
            ["Filter", "Distort"],
            r##"{"displacement":1..90=30,"segments":1..10=3,"orientation":"horizontal|vertical","transparent":bool,"background":color=#ffffff}"##
        ),
        cmd!(
            "filter.distort.pageCurl",
            "Page Curl…",
            ["Filter", "Distort"],
            r##"{"location":"upperLeft|upperRight|lowerLeft|lowerRight"="lowerRight","orientation":"horizontal|vertical","shade":bool=true,"colors":"foregroundBackground|currentGradient|currentGradientReversed","opacity":0..100=100,"foreground":json,"background":json}"##
        ),
        // Light and Shadow
        cmd!(
            "filter.lightShadow.supernova",
            "Supernova…",
            ["Filter", "Light and Shadow"],
            r##"{"centerX":0..1=0.5,"centerY":0..1=0.5,"radius":1..3000=20,"spokes":1..1024=100,"randomHue":0..360=0,"color":color=#5966ff,"seed":u32=0}"##
        ),
        cmd!(
            "filter.lightShadow.gradientFlare",
            "Gradient Flare…",
            ["Filter", "Light and Shadow"],
            r##"{"centerX":0..1=0.5,"centerY":0..1=0.5,"radius":1..3000=100,"rotation":-180..180=0,"hueRotation":-180..180=0,"vectorAngle":0..360=60,"vectorLength":0..10000=400,"glow":bool=true,"rays":bool=true,"secondFlares":bool=true}"##
        ),
        cmd!(
            "filter.lightShadow.sparkle",
            "Sparkle…",
            ["Filter", "Light and Shadow"],
            r##"{"luminosityThreshold":0..0.1=0.001,"flareIntensity":0..1=0.5,"spikeLength":1..100=20,"spikePoints":0..16=4,"spikeAngle":-1..360=15,"spikeDensity":0..1=1,"transparency":0..1=0,"randomHue":0..1=0,"randomSaturation":0..1=0,"preserveLuminosity":bool,"inverse":bool,"addBorder":bool,"colorType":"naturalColor|foregroundColor|backgroundColor","foreground":json,"background":json,"seed":u32=0}"##
        ),
        cmd!(
            "filter.lightShadow.longShadow",
            "Long Shadow…",
            ["Filter", "Light and Shadow"],
            r##"{"style":"finite|infinite|fading|fadingFixedLength","angle":-180..180=45,"length":0..1000=100,"midpoint":0..1000=100,"color":color=#000000,"composition":"shadowPlusImage|shadowOnly|shadowMinusImage"}"##
        ),
        cmd!(
            "filter.lightShadow.perspectiveShadow",
            "Perspective Shadow…",
            ["Filter", "Light and Shadow"],
            r##"{"angle":0..180=45,"relativeDistance":0.1..24=5,"shadowLength":0.1..24=1,"blurRadius":0..1024=3,"color":color=#000000,"opacity":0..100=80,"interpolation":"none|linear|cubic"="linear","allowResizing":bool}"##
        ),
        cmd!(
            "filter.lightShadow.xachEffect",
            "Xach-Effect…",
            ["Filter", "Light and Shadow"],
            r##"{"highlightXOffset":-100..100=-1,"highlightYOffset":-100..100=-1,"highlightColor":color=#ffffff,"highlightOpacity":0..255=66,"dropShadowColor":color=#000000,"dropShadowOpacity":0..255=100,"dropShadowBlurRadius":0..255=12,"dropShadowXOffset":-100..100=5,"dropShadowYOffset":-100..100=5}"##
        ),
        // Stylize
        cmd!(
            "filter.stylize.differenceOfGaussians",
            "Difference of Gaussians…",
            ["Filter", "Stylize"],
            r##"{"radius1":0..10=1,"radius2":0..10=2,"normalize":bool=true,"invert":bool}"##
        ),
        cmd!("filter.stylize.laplace", "Laplace", ["Filter", "Stylize"], "{}"),
        cmd!("filter.stylize.imageGradient", "Image Gradient…", ["Filter", "Stylize"], r##"{"output":"magnitude|direction|both"}"##),
        // Other
        cmd!("filter.other.antialias", "Antialias", ["Filter", "Other"], "{}"),
        cmd!(
            "filter.other.distanceMap",
            "Distance Map…",
            ["Filter", "Other"],
            r##"{"metric":"euclidean|manhattan|chebyshev","thresholdLow":0..1=0.0001,"thresholdHigh":0..1=1,"grayscaleAveraging":0..1000=0,"normalize":bool=true}"##
        ),
        cmd!(
            "filter.other.normalMap",
            "Normal Map…",
            ["Filter", "Other"],
            r##"{"scale":0..255=10,"xComponent":"red|green|blue"="red","yComponent":"red|green|blue"="green","flipX":bool,"flipY":bool,"fullZ":bool,"tileable":bool}"##
        ),
        cmd!("filter.other.operationGraph", "Operation Graph…", ["Filter", "Other"], r##"{"graph":text}"##),
        // Artistic
        cmd!(
            "filter.artistic.cubism",
            "Cubism…",
            ["Filter", "Artistic"],
            r##"{"tileSize":0..256=10,"tileSaturation":0..10=2.5,"background":color=#000000,"seed":u32=0}"##
        ),
        cmd!("filter.artistic.slic", "SLIC…", ["Filter", "Artistic"], r##"{"regionsSize":2..1024=32,"compactness":1..40=20,"iterations":1..30=1}"##),
        cmd!(
            "filter.artistic.waterpixels",
            "Waterpixels…",
            ["Filter", "Artistic"],
            r##"{"superpixelsSize":8..256=32,"gradientSmoothness":0..1000=1,"spatialRegularization":0..50=0,"superpixelsColor":"average|random","seed":u32=0}"##
        ),
        cmd!(
            "filter.artistic.impressionist",
            "Impressionist…",
            ["Filter", "Artistic"],
            r##"{"brushSize":1..200=20,"sizeVariation":0..1=0.3,"orientation":0..360=45,"orientationType":"value|radius|random|radial|flowing|hue|adaptive","strokeDensity":1..50=20,"brushShape":"ellipse|square|line|splotch","paperRelief":0..100=0,"placement":"randomly|evenly","strokeColor":"average|center","background":"keepOriginal|solidColor|transparent","backgroundColor":color=#ffffff,"seed":u32=0}"##
        ),
        cmd!(
            "filter.artistic.vanGogh",
            "Van Gogh (LIC)…",
            ["Filter", "Artistic"],
            r##"{"effectChannel":"brightness|hue|saturation","effectOperator":"derivative|gradient","convolve":"withWhiteNoise|withSourceImage","filterLength":0.1..64=5,"noiseMagnitude":1..5=2,"integrationSteps":1..40=25,"minimumValue":-100..0=-25,"maximumValue":0..100=25,"seed":u32=0}"##
        ),
        cmd!(
            "filter.artistic.weave",
            "Weave…",
            ["Filter", "Artistic"],
            r##"{"ribbonWidth":0..256=30,"ribbonSpacing":0..256=10,"shadowDarkness":0..100=75,"shadowDepth":0..100=75,"threadLength":0..256=200,"threadDensity":0..256=50,"threadIntensity":0..100=100}"##
        ),
        // Decor
        cmd!(
            "filter.decor.filmstrip",
            "Filmstrip…",
            ["Filter", "Decor"],
            r##"{"filmColor":color=#000000,"numberColor":color=#ed9b00,"startIndex":0..9999=1,"atTop":bool=true,"atBottom":bool=true,"holeOffset":0..0.3=0.04,"holeWidth":0.005..0.5=0.056,"holeHeight":0.005..0.3=0.08,"holeSpacing":0.01..1=0.08}"##
        ),
        cmd!(
            "filter.decor.fuzzyBorder",
            "Fuzzy Border…",
            ["Filter", "Decor"],
            r##"{"color":color=#ffffff,"borderSize":1..300=16,"blurBorder":bool=true,"granularity":1..16=4,"addShadow":bool,"shadowWeight":0..100=100,"seed":u32=0}"##
        ),
    ]
}
