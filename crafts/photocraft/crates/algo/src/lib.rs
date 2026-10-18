//! # redrob-photo-algo
//!
//! CPU image filters (Photoshop's Filter menu): blur, sharpen, noise,
//! pixelate, stylize and distort. Every filter is
//!
//! * **depth-agnostic**: works on normalized `f32` samples read from any
//!   [`Surface`] (8/16-bit integer or 32-bit float) in the surface's own
//!   colour model;
//! * **tile/region aware**: [`FilterParams::halo`] declares how far outside an
//!   output tile the filter reads, so results are identical whatever the
//!   tiling (distortions declare [`Halo::Bounds`] and read the whole
//!   reference area);
//! * **selection aware**: output is mixed with the original by the
//!   selection's coverage.
//!
//! [`apply`] runs a filter over an area, tile by tile (in parallel with
//! rayon on native targets; single-threaded on wasm).
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod adjust_ext;
pub mod anim;
mod artistic;
mod artistic_fx;
mod blur;
mod blur2;
pub mod cage;
pub mod camera_raw;
pub mod color_to_alpha;
pub mod color_tools;
pub mod content_aware;
mod decor;
mod denoise;
pub mod dff;
pub mod direct;
mod distort;
mod distort2;
mod distort3;
pub mod erase;
pub mod exif;
mod extra;
mod extra2;
pub mod features;
pub mod fill_regions;
mod fxutil;
mod gallery;
mod gen_fractal;
mod gen_noise;
mod gen_pattern;
mod gen_util;
pub mod hdr;
pub mod histogram;
mod image;
pub mod inpaint;
pub mod lens;
pub mod liquify;
pub mod magnetic;
pub mod matting;
mod noise;
pub mod nonlocal;
mod oil;
mod other;
mod other2;
pub mod paint;
pub mod panorama;
mod par;
pub mod perspective;
mod photo_util;
mod pixelate;
pub mod poisson;
pub mod puppet;
pub mod pyramid;
pub mod quantize;
pub mod redeye;
mod relight;
pub mod remove;
mod render;
pub mod render2;
pub mod resample;
pub mod retouch;
pub mod scancrop;
pub mod seam;
pub mod segment;
pub mod selection;
mod selection_blur;
mod shape_blur;
mod sharpen;
pub mod stack;
mod stylize;
mod stylize2;
pub mod tone;
pub mod transform;
pub mod trap;
pub mod vanishing;
pub mod vectorscope;
mod video;
pub mod warp;
pub mod wideangle;

pub use artistic::{GALLERY_CATEGORIES, GalleryEffect, GalleryFilter, GalleryParam, GalleryParamKind};
pub use image::{Edge, Image};
pub use params_ext::*;
pub use params_extra::*;
pub use params_gen::*;
pub use params_more::*;

mod params_ext;
mod params_extra;
mod params_gen;
mod params_more;

use redrob_photo_color::ColorMode;
use redrob_photo_geom::Rect;
use redrob_photo_raster::Surface;
use serde::{Deserialize, Serialize};

/// How far outside an output tile a filter reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Halo {
    /// A margin of this many pixels.
    Radius(i32),
    /// The whole reference bounds (distortions, radial blur).
    Bounds,
}

/// Radial blur method.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum RadialMethod {
    #[default]
    Spin,
    Zoom,
}

/// Sampling quality for radial blur. Higher quality improves large-radius detail at greater cost.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum RadialQuality {
    /// Up to 64 intervals per pixel for a faster, coarser result.
    Draft,
    /// Up to 256 intervals per pixel; the default.
    #[default]
    Good,
    /// Up to 4096 intervals per pixel.
    Best,
}

/// Noise distribution.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Distribution {
    #[default]
    Uniform,
    Gaussian,
}

/// How pixels uncovered by a displacement are filled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum UndefinedAreas {
    #[default]
    Wrap,
    Repeat,
    Transparent,
}

/// Minimum / Maximum › Preserve: the shape the filter grows or shrinks by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Preserve {
    /// A square of `2·radius + 1` pixels (keeps corners square).
    #[default]
    Squareness,
    /// A disc of `radius` pixels (rounds corners off evenly).
    Roundness,
}

/// How Desaturate measures a colour's lightness.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum DesaturateMode {
    /// Rec. 709 luminance of the linear-light colour (perceptual grey of light).
    #[default]
    Luminance,
    /// Rec. 601 weights on the gamma-encoded values (the classic video grey).
    Luma,
    /// HSL lightness, (max + min) / 2.
    Lightness,
    /// Plain mean of the three channels.
    Average,
    /// HSV value, max of the three channels.
    Value,
}

/// Spherize mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SpherizeMode {
    #[default]
    Normal,
    HorizontalOnly,
    VerticalOnly,
}

/// Wave shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum WaveType {
    #[default]
    Sine,
    Triangle,
    Square,
}

/// Ripple size.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum RippleSize {
    Small,
    #[default]
    Medium,
    Large,
}

/// Polar coordinates direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum PolarMode {
    #[default]
    RectangularToPolar,
    PolarToRectangular,
}

/// A filter and its parameters, in Photoshop's dialog units (pixels,
/// percent, degrees, levels 0–255).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "filter", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum FilterParams {
    /// Radius = Gaussian standard deviation in pixels (0.1–1000).
    GaussianBlur {
        radius: f32,
    },
    /// Box of `2·radius + 1` pixels.
    BoxBlur {
        radius: f32,
    },
    MotionBlur {
        angle: f32,
        distance: f32,
    },
    /// Amount 1–100; centre as a fraction of the bounds (0.5, 0.5 = centre).
    RadialBlur {
        amount: f32,
        method: RadialMethod,
        #[serde(default)]
        quality: RadialQuality,
        center_x: f32,
        center_y: f32,
    },
    /// Radius px, threshold in levels.
    SurfaceBlur {
        radius: f32,
        threshold: f32,
    },
    /// Amount %, radius px, threshold levels.
    UnsharpMask {
        amount: f32,
        radius: f32,
        threshold: f32,
    },
    /// Amount %, radius px, noise reduction % (basic: Gaussian unsharp).
    SmartSharpen {
        amount: f32,
        radius: f32,
        reduce_noise: f32,
    },
    HighPass {
        radius: f32,
    },
    /// Amount 0–400 %.
    AddNoise {
        amount: f32,
        distribution: Distribution,
        monochromatic: bool,
        seed: u32,
    },
    Median {
        radius: f32,
    },
    DustAndScratches {
        radius: f32,
        threshold: f32,
    },
    /// Radius px; `preserve` is the dialog's Preserve (older params default to Squareness).
    Minimum {
        radius: f32,
        #[serde(default)]
        preserve: Preserve,
    },
    Maximum {
        radius: f32,
        #[serde(default)]
        preserve: Preserve,
    },
    Offset {
        horizontal: i32,
        vertical: i32,
        undefined: UndefinedAreas,
    },
    Mosaic {
        cell_size: f32,
    },
    /// Angle degrees, height px, amount %.
    Emboss {
        angle: f32,
        height: f32,
        amount: f32,
    },
    FindEdges,
    Solarize,
    Invert,
    /// Greyscale by one of several measures of a colour's lightness.
    Desaturate {
        mode: DesaturateMode,
    },
    /// Angle −999…999 degrees.
    Twirl {
        angle: f32,
    },
    /// Amount −100…100 %.
    Pinch {
        amount: f32,
    },
    /// Amount −100…100 %.
    Spherize {
        amount: f32,
        mode: SpherizeMode,
    },
    Wave {
        generators: u32,
        wavelength_min: f32,
        wavelength_max: f32,
        amplitude_min: f32,
        amplitude_max: f32,
        wave_type: WaveType,
        undefined: UndefinedAreas,
        seed: u32,
    },
    /// Amount −999…999 %.
    Ripple {
        amount: f32,
        size: RippleSize,
    },
    PolarCoordinates {
        mode: PolarMode,
    },

    // ---- Pixelate ----
    /// Max dot radius px (4–127); screen angles per channel in degrees.
    ColorHalftone {
        max_radius: f32,
        angles: [f32; 4],
    },
    /// Cell size px (3–300).
    Crystallize {
        cell_size: f32,
        seed: u32,
    },
    Facet,
    Fragment,
    Mezzotint {
        kind: MezzotintType,
        seed: u32,
    },
    /// Cell size px (3–300); `background` is straight sRGB RGBA.
    Pointillize {
        cell_size: f32,
        seed: u32,
        background: [f32; 4],
    },

    // ---- Stylize ----
    Diffuse {
        mode: DiffuseMode,
        seed: u32,
    },
    /// Size px (2–255), depth (1–255).
    Extrude {
        kind: ExtrudeType,
        size: f32,
        depth: f32,
        level_based: bool,
        solid_front: bool,
        mask_incomplete: bool,
        seed: u32,
    },
    /// Photoshop's Oil Paint units: stylization 0.1–10, cleanliness 0–10, scale 0.1–10,
    /// bristle detail 0–10, lighting angle degrees, shine 0–10.
    OilPaint {
        stylization: f32,
        cleanliness: f32,
        scale: f32,
        bristle_detail: f32,
        lighting: bool,
        angle: f32,
        shine: f32,
    },
    /// Tiles along the shorter side (1–99), max offset % (1–99).
    Tiles {
        count: u32,
        max_offset: f32,
        fill: TileFill,
        foreground: [f32; 4],
        background: [f32; 4],
        seed: u32,
    },
    /// Level 0–255; `upper` traces the side above the level.
    TraceContour {
        level: f32,
        upper: bool,
    },
    Wind {
        method: WindMethod,
        from_right: bool,
        seed: u32,
    },
    /// Edge: light edges on black by one of six operators, scaled by `amount` (1–10; 2 maps a
    /// full step to white).
    Edge {
        algorithm: EdgeAlgorithm,
        amount: f32,
    },
    /// Colour Temperature: the light source changes from `original` to `intended` kelvin.
    ColorTemperature {
        original: f32,
        intended: f32,
    },
    /// Hue-Chroma: hue rotation in degrees, chroma and lightness offsets (−100..100) in LCh(ab).
    HueChroma {
        hue: f32,
        chroma: f32,
        lightness: f32,
    },

    // ---- Distort ----
    /// Scales in % (−999…999; 100 % = 128 px at full map contrast). The map is
    /// supplied by the caller (another document, layer or file).
    Displace {
        horizontal: f32,
        vertical: f32,
        stretch: bool,
        undefined: UndefinedAreas,
        #[serde(skip)]
        map: Option<std::sync::Arc<Image>>,
    },
    /// Curve points `[t, offset]`: t 0–1 top→bottom, offset −1…1 of half the width.
    Shear {
        points: Vec<[f32; 2]>,
        undefined: UndefinedAreas,
    },
    /// Amount −100…100, ridges 0–20.
    ZigZag {
        amount: f32,
        ridges: f32,
        style: ZigZagStyle,
    },

    // ---- Render ----
    /// Variance 1–64, strength 1–64; colours straight sRGB RGBA.
    Fibers {
        variance: f32,
        strength: f32,
        seed: u32,
        foreground: [f32; 4],
        background: [f32; 4],
    },
    /// Brightness 10–300 %, centre as a fraction of the bounds.
    LensFlare {
        brightness: f32,
        center_x: f32,
        center_y: f32,
        lens: LensType,
    },
    /// Gloss/metallic/exposure/ambience −100…100; bump height 0–100.
    LightingEffects {
        lights: Vec<Light>,
        gloss: f32,
        metallic: f32,
        exposure: f32,
        ambience: f32,
        texture: TextureChannel,
        height: f32,
        white_is_high: bool,
    },
    /// Photographic relight: angle −180…180°, elevation 0…90°, intensity/ambient 0…100,
    /// warmth −100…100, softness 1…100. Intensity 0 is an identity copy.
    Relight {
        angle: f32,
        elevation: f32,
        intensity: f32,
        ambient: f32,
        warmth: f32,
        softness: f32,
    },

    // ---- Noise ----
    /// Strength 0–10, the rest 0–100 %.
    ReduceNoise {
        strength: f32,
        preserve_details: f32,
        reduce_color_noise: f32,
        sharpen_details: f32,
        remove_jpeg_artifact: bool,
    },

    // ---- Blur ----
    /// Radius 0.1–100 px, threshold 0.1–100 levels.
    SmartBlur {
        radius: f32,
        threshold: f32,
        quality: BlurQuality,
        mode: SmartBlurMode,
    },
    /// Iris radius 0–100 px, `blades` 3–8, curvature 0–100, rotation degrees;
    /// focal distance 0–255; specular brightness 0–100 / threshold 0–255; noise 0–100.
    LensBlur {
        radius: f32,
        blades: u32,
        curvature: f32,
        rotation: f32,
        depth: DepthSource,
        focal_distance: f32,
        invert_depth: bool,
        brightness: f32,
        threshold: f32,
        noise: f32,
        distribution: Distribution,
        monochromatic: bool,
        seed: u32,
        /// Depth for [`DepthSource::LayerMask`] (channel 0, document coordinates), supplied by the caller.
        #[serde(skip)]
        depth_map: Option<std::sync::Arc<Image>>,
    },
    /// Radius 5–1000 px.
    ShapeBlur {
        radius: f32,
        shape: BlurShape,
    },

    // ---- Blur Gallery ----
    /// Centre fractions, angle degrees, focus/transition as fractions of the bounds' shorter side.
    TiltShift {
        blur: f32,
        center_x: f32,
        center_y: f32,
        angle: f32,
        focus: f32,
        transition: f32,
    },
    IrisBlur {
        pins: Vec<IrisPin>,
    },
    FieldBlur {
        pins: Vec<FieldPin>,
    },
    SpinBlur {
        pins: Vec<SpinPin>,
    },
    PathBlur {
        paths: Vec<BlurPath>,
    },

    // ---- Other ----
    /// 5×5 kernel (row-major, centre = index 12), divided by `scale`, plus `offset` levels.
    Custom {
        kernel: Vec<f32>,
        scale: f32,
        offset: f32,
    },
    HsbHsl {
        input: HsbModel,
        output: HsbModel,
    },
    /// Removes `color` (straight sRGB RGBA, alpha ignored) by unmixing it from every pixel into
    /// transparency (see [`color_to_alpha`]). Thresholds 0–1: opacities at or below
    /// `transparency_threshold` become 0, at or above `opacity_threshold` 1, linear in between.
    ColorToAlpha {
        color: [f32; 4],
        transparency_threshold: f32,
        opacity_threshold: f32,
    },

    // ---- Video ----
    /// Removes the even (or odd) lines and rebuilds them by interpolation (or duplication).
    DeInterlace {
        eliminate_even: bool,
        interpolate: bool,
    },
    NtscColors,

    // ---- Third batch: blur ----
    /// Edge-preserving smoothing by `iterations` (1–500) steps of mean curvature flow.
    MeanCurvatureBlur {
        iterations: u32,
    },
    /// Blur that wraps around the bounds so the result tiles; radius 0–128 px.
    TileableBlur {
        radius: f32,
        horizontal: bool,
        vertical: bool,
        blur_type: TileableBlurType,
    },
    /// Scale3X-derived smoothing of jagged edges.
    Antialias,
    /// Radius 1–100 px; `pairs` 1 or 2.
    SymmetricNearestNeighbor {
        radius: f32,
        pairs: u32,
    },
    /// Alpha and radius 0–1.
    NlFilter {
        alpha: f32,
        radius: f32,
        mode: NlMode,
    },

    // ---- Third batch: distort ----
    /// Line height 2–16 px.
    Engrave {
        height: f32,
        limit_line_width: bool,
    },
    /// Mirrors 1–24; rotations in degrees; offset and trim as fractions of the bounds; zoom %.
    Kaleidoscope {
        mirrors: u32,
        mirror_rotation: f32,
        result_rotation: f32,
        offset_x: f32,
        offset_y: f32,
        zoom: f32,
        trim_x: f32,
        trim_y: f32,
        clip: bool,
    },
    /// Amount 0–200 px.
    Shift {
        amount: f32,
        axis: Axis,
        seed: u32,
    },
    /// Thresholds and rate 0–1; `color` straight sRGB RGBA; the directions values come from.
    ValuePropagate {
        mode: PropagateMode,
        lower_threshold: f32,
        upper_threshold: f32,
        rate: f32,
        color: [f32; 4],
        top: bool,
        left: bool,
        right: bool,
        bottom: bool,
    },
    VideoDegradation {
        pattern: VideoPattern,
        additive: bool,
        rotated: bool,
    },
    /// Displacement 1–90 degrees, 1–10 segments.
    Blinds {
        displacement: f32,
        segments: u32,
        axis: Axis,
        transparent: bool,
        background: [f32; 4],
    },
    /// Opacity 0–100 %; colours straight sRGB RGBA.
    PageCurl {
        corner: CurlCorner,
        axis: Axis,
        shade: bool,
        colors: CurlColors,
        opacity: f32,
        foreground: [f32; 4],
        background: [f32; 4],
    },

    // ---- Third batch: light and shadow ----
    /// Centre as fractions of the bounds, radius 1–3000 px, spokes 1–1024, random hue 0–360.
    Supernova {
        center_x: f32,
        center_y: f32,
        radius: f32,
        spokes: u32,
        random_hue: f32,
        color: [f32; 4],
        seed: u32,
    },
    /// Radius 1–3000 px, angles in degrees, vector length px.
    GradientFlare {
        center_x: f32,
        center_y: f32,
        radius: f32,
        rotation: f32,
        hue_rotation: f32,
        vector_angle: f32,
        vector_length: f32,
        glow: bool,
        rays: bool,
        second_flares: bool,
        /// Built-in design; ray count, glow size (%) and ghost count override it when non-zero
        /// (100 = the design's glow size).
        #[serde(default)]
        design: FlareDesign,
        #[serde(default)]
        ray_count: u32,
        #[serde(default = "default_glow_size")]
        glow_size: f32,
        #[serde(default)]
        ghost_count: u32,
    },
    /// Threshold 0–0.1, intensities 0–1, spike length 1–100 px, points 0–16, angle −1 (random)
    /// to 360.
    Sparkle {
        threshold: f32,
        flare_intensity: f32,
        spike_length: f32,
        spike_points: u32,
        spike_angle: f32,
        density: f32,
        transparency: f32,
        random_hue: f32,
        random_saturation: f32,
        preserve_luminosity: bool,
        inverse: bool,
        border: bool,
        color: SparkleColor,
        foreground: [f32; 4],
        background: [f32; 4],
        seed: u32,
    },
    /// Angle −180–180 degrees, length and midpoint px.
    LongShadow {
        style: ShadowStyle,
        angle: f32,
        length: f32,
        midpoint: f32,
        color: [f32; 4],
        composition: ShadowComposition,
    },
    /// Angle 0–180, distance and length relative to the object height, blur px, opacity %.
    PerspectiveShadow {
        angle: f32,
        distance: f32,
        length: f32,
        blur: f32,
        color: [f32; 4],
        opacity: f32,
        interpolation: ShadowInterpolation,
        allow_resizing: bool,
    },
    /// Offsets px, opacities 0–255, blur px.
    XachEffect {
        highlight_x: f32,
        highlight_y: f32,
        highlight_color: [f32; 4],
        highlight_opacity: f32,
        shadow_color: [f32; 4],
        shadow_opacity: f32,
        blur: f32,
        shadow_x: f32,
        shadow_y: f32,
    },

    // ---- Third batch: noise ----
    /// Dulling 1–8, lightness and chroma 0–100, hue 0–180 degrees.
    LchNoise {
        dulling: u32,
        lightness: f32,
        chroma: f32,
        hue: f32,
        seed: u32,
    },
    /// Dulling 1–8, hue 0–180 degrees, saturation and value 0–1.
    HsvNoise {
        dulling: u32,
        hue: f32,
        saturation: f32,
        value: f32,
        seed: u32,
    },
    /// Randomization 0–100 %, repeat 1–100.
    Hurl {
        randomization: f32,
        repeat: u32,
        seed: u32,
    },
    Pick {
        randomization: f32,
        repeat: u32,
        seed: u32,
    },
    Slur {
        randomization: f32,
        repeat: u32,
        seed: u32,
    },

    // ---- Third batch: edges and maps ----
    /// Radii 0–10 px.
    DifferenceOfGaussians {
        radius1: f32,
        radius2: f32,
        normalize: bool,
        invert: bool,
    },
    Laplace,
    ImageGradient {
        output: GradientOutput,
    },
    /// Thresholds 0–1, averaging 0–1000.
    DistanceMap {
        metric: DistanceMetric,
        threshold_low: f32,
        threshold_high: f32,
        averaging: f32,
        normalize: bool,
    },
    /// Scale 0–255.
    NormalMap {
        scale: f32,
        x_component: NormalComponent,
        y_component: NormalComponent,
        flip_x: bool,
        flip_y: bool,
        full_z: bool,
        tileable: bool,
    },
    /// One operation per line (`blur radius=4`, `invert`, `saturation value=1.5`, …).
    OperationGraph {
        source: String,
    },

    // ---- Third batch: artistic ----
    /// Tile size 0–256 px, saturation 0–10; background straight sRGB RGBA.
    Cubism {
        tile_size: f32,
        saturation: f32,
        background: [f32; 4],
        seed: u32,
    },
    /// Cluster size 2–1024 px, compactness 1–40, iterations 1–30.
    Slic {
        cluster_size: f32,
        compactness: f32,
        iterations: u32,
    },
    /// Superpixel size 8–256 px, smoothness 0–1000, regularization 0–50.
    Waterpixels {
        size: f32,
        smoothness: f32,
        regularization: f32,
        color: SuperpixelColor,
        seed: u32,
    },
    /// Brush size 1–200 px, variation 0–1, orientation degrees, density 1–50, relief 0–100 %.
    Impressionist {
        brush_size: f32,
        size_variation: f32,
        orientation: f32,
        orientation_type: StrokeOrientation,
        density: f32,
        shape: StrokeShape,
        paper_relief: f32,
        placement: StrokePlacement,
        color: StrokeColor,
        background: StrokeBackground,
        background_color: [f32; 4],
        seed: u32,
        /// Paper texture under the relief, its scale (px per weave), and the paper / brush
        /// images supplied by the caller (`paperPath` / `brushPath`).
        #[serde(default)]
        paper: PaperTexture,
        #[serde(default = "default_paper_scale")]
        paper_scale: f32,
        #[serde(skip)]
        paper_map: Option<std::sync::Arc<Image>>,
        #[serde(skip)]
        brush_map: Option<std::sync::Arc<Image>>,
    },
    /// Filter length 0.1–64, noise magnitude 1–5, steps 1–40, values −100–0 / 0–100.
    VanGogh {
        channel: LicChannel,
        operator: LicOperator,
        convolve: LicConvolve,
        filter_length: f32,
        noise_magnitude: f32,
        integration_steps: u32,
        minimum_value: f32,
        maximum_value: f32,
        seed: u32,
    },
    /// Widths, spacing and lengths px (0–256); darkness, depth and intensity %.
    Weave {
        ribbon_width: f32,
        ribbon_spacing: f32,
        shadow_darkness: f32,
        shadow_depth: f32,
        thread_length: f32,
        thread_density: f32,
        thread_intensity: f32,
    },

    // ---- Third batch: decor ----
    /// Hole geometry as fractions of the film height; colours straight sRGB RGBA.
    Filmstrip {
        color: [f32; 4],
        number_color: [f32; 4],
        number_start: i32,
        numbers_top: bool,
        numbers_bottom: bool,
        hole_offset: f32,
        hole_width: f32,
        hole_height: f32,
        hole_spacing: f32,
        /// Scale frames to the frame height; number font and height (fraction of the film
        /// height, 0 = automatic); the pictures (supplied by the caller; empty = the layer).
        #[serde(default = "default_true")]
        fit_height: bool,
        #[serde(default)]
        font: FilmFont,
        #[serde(default)]
        number_height: f32,
        #[serde(skip)]
        frames: Vec<std::sync::Arc<Image>>,
    },
    /// Border size 1–300 px, granularity 1–16, shadow weight 0–100 %.
    FuzzyBorder {
        color: [f32; 4],
        size: f32,
        blur: bool,
        granularity: f32,
        shadow: bool,
        shadow_weight: f32,
        seed: u32,
    },
    // ---- Map ----
    /// Fractal Trace: the image traced through a Mandelbrot or Julia set over the complex window
    /// `x1..x2` × `y1..y2`, `depth` iterations, `bailout` radius; `abyss` fills samples outside.
    FractalTrace {
        kind: FractalKind,
        x1: f32,
        x2: f32,
        y1: f32,
        y2: f32,
        jx: f32,
        jy: f32,
        depth: u32,
        bailout: f32,
        abyss: FractalAbyss,
    },
    /// Illusion: `division` kaleidoscopic copies.
    Illusion {
        division: u32,
        kind: IllusionType,
    },
    /// Little Planet: stereographic projection of an equirectangular panorama. Angles in
    /// degrees, zoom in %.
    LittlePlanet {
        pan: f32,
        tilt: f32,
        spin: f32,
        zoom: f32,
        inverse: bool,
    },
    /// Recursive Transform: scale %, angle degrees, offset as fractions of the bounds; copies
    /// `first..=last`; `fade` straight RGBA (its alpha is the fade strength).
    RecursiveTransform {
        scale: f32,
        angle: f32,
        offset_x: f32,
        offset_y: f32,
        first: u32,
        last: u32,
        fade: [f32; 4],
        paste_below: bool,
    },
    TileSeamless,
    /// Map Object: rotations in degrees, light position in bounds units, material 0…1,
    /// `highlight` the shininess exponent, `radius` of the solid in bounds units.
    MapObject {
        shape: MapShape,
        rot_x: f32,
        rot_y: f32,
        rot_z: f32,
        light_x: f32,
        light_y: f32,
        light_z: f32,
        ambient: f32,
        diffuse: f32,
        specular: f32,
        highlight: f32,
        radius: f32,
        transparent_background: bool,
    },
    /// Small Tiles: `segments` per side, flipped tiles, opacity %.
    SmallTiles {
        segments: u32,
        flip: TileFlip,
        which: TileSelection,
        opacity: f32,
    },

    // ---- Render: fractals ----
    /// Fractal Explorer: window, iterations, Julia seed, colour bands, RGB intensity 0…1 and
    /// inversion.
    FractalExplorer {
        kind: ExplorerFractal,
        x1: f32,
        x2: f32,
        y1: f32,
        y2: f32,
        iterations: u32,
        cx: f32,
        cy: f32,
        colors: u32,
        red: f32,
        green: f32,
        blue: f32,
        invert_red: bool,
        invert_green: bool,
        invert_blue: bool,
    },
    /// IFS Fractal: preset map set, iterations ×1000, spot radius px, placement.
    IfsFractal {
        preset: IfsPreset,
        iterations: u32,
        spot_radius: f32,
        x: f32,
        y: f32,
        scale: f32,
        angle: f32,
        color: [f32; 4],
        seed: u32,
    },
    /// Sphere Designer: ray-traced textured sphere.
    SphereDesigner {
        texture: SphereTexture,
        color1: [f32; 4],
        color2: [f32; 4],
        scale: f32,
        turbulence: f32,
        amount: f32,
        exponent: f32,
        light_x: f32,
        light_y: f32,
        light_z: f32,
        ambient: f32,
        bump: f32,
        radius: f32,
        transparent_background: bool,
    },

    // ---- Render: noise ----
    /// Cell Noise: feature size px, metric exponent, rank 1–3, octaves, palettize, seed.
    CellNoise {
        scale: f32,
        shape: f32,
        rank: u32,
        iterations: u32,
        palettize: bool,
        seed: u32,
    },
    /// Simplex Noise: feature size px, octaves, seed.
    SimplexNoise {
        scale: f32,
        iterations: u32,
        seed: u32,
    },
    /// Lava: seed, feature size px, roughness (octaves), gradient.
    Lava {
        seed: u32,
        size: f32,
        roughness: f32,
        gradient: LavaGradient,
    },
    /// Circuit: trace pitch px, seed, transparent gaps.
    Circuit {
        mask_size: f32,
        seed: u32,
        no_background: bool,
    },
    /// Qbist: random texture program chosen by `seed`.
    Qbist {
        seed: u32,
        antialias: bool,
        oversample: u32,
    },
    /// CML Explorer: per-channel maps, map parameter `k` 0–10, coupling 0–1, lattice zoom px,
    /// start row, seed.
    CmlExplorer {
        hue: CmlFunction,
        saturation: CmlFunction,
        value: CmlFunction,
        k: f32,
        coupling: f32,
        zoom: f32,
        start_offset: u32,
        seed: u32,
    },

    // ---- Render: pattern ----
    BayerMatrix {
        subdivisions: u32,
        x_scale: u32,
        y_scale: u32,
        rotation: QuarterTurn,
        reflect: bool,
        amplitude: f32,
        offset: f32,
        exponent: f32,
        x_offset: i32,
        y_offset: i32,
    },
    /// Checkerboard: cell size px, offsets px, two straight RGBA colours.
    Checkerboard {
        width: f32,
        height: f32,
        x_offset: f32,
        y_offset: f32,
        color1: [f32; 4],
        color2: [f32; 4],
    },
    DiffractionPatterns {
        red_frequency: f32,
        green_frequency: f32,
        blue_frequency: f32,
        red_contours: f32,
        green_contours: f32,
        blue_contours: f32,
        red_edges: f32,
        green_edges: f32,
        blue_edges: f32,
        brightness: f32,
        scattering: f32,
        polarization: f32,
    },
    /// Grid: spacings, offsets, line width and height in px; straight RGBA colour.
    Grid {
        x_spacing: f32,
        y_spacing: f32,
        x_offset: f32,
        y_offset: f32,
        line_width: f32,
        line_height: f32,
        color: [f32; 4],
    },
    LinearSinusoid {
        x_period: f32,
        y_period: f32,
        x_amplitude: f32,
        y_amplitude: f32,
        x_phase: f32,
        y_phase: f32,
        angle: f32,
        offset: f32,
        exponent: f32,
        x_offset: f32,
        y_offset: f32,
        rotation: f32,
        supersampling: u32,
    },
    Maze {
        cell_width: f32,
        cell_height: f32,
        algorithm: MazeAlgorithm,
        tileable: bool,
        seed: u32,
        foreground: [f32; 4],
        background: [f32; 4],
    },
    Sinus {
        x_scale: f32,
        y_scale: f32,
        complexity: f32,
        seed: u32,
        tileable: bool,
        perturbation: bool,
        blend: SinusBlend,
        exponent: f32,
        color1: [f32; 4],
        color2: [f32; 4],
    },
    /// Spiral: origin as fractions of the bounds, radius px per turn, log base, balance −1…1,
    /// rotation degrees.
    Spiral {
        kind: SpiralType,
        x: f32,
        y: f32,
        radius: f32,
        base: f32,
        balance: f32,
        rotation: f32,
        direction: SpinDirection,
        color1: [f32; 4],
        color2: [f32; 4],
    },
    Jigsaw {
        x_tiles: u32,
        y_tiles: u32,
        bevel_width: f32,
        highlight: f32,
        style: JigsawStyle,
    },
    /// Line Nova: rays from the centre in `color`.
    LineNova {
        lines: u32,
        sharpness: f32,
        offset_radius: f32,
        randomness: f32,
        seed: u32,
        color: [f32; 4],
    },
    /// Spirograph curve stroked in `color`.
    Spirograph {
        curve: SpyroCurve,
        shape: SpyroShape,
        fixed_teeth: u32,
        moving_teeth: u32,
        hole: f32,
        sides: u32,
        morph: f32,
        rotation: f32,
        line_width: f32,
        color: [f32; 4],
    },

    // ---- Decor / Web ----
    /// Slide: film-slide frame with a caption and a number in `color`; `font_size` px.
    Slide {
        text: String,
        number: String,
        font_size: f32,
        color: [f32; 4],
    },
    CoffeeStain {
        stains: u32,
        darken_only: bool,
        seed: u32,
    },
    /// Stencil Carve: the grey `map` (supplied by the caller) carved into the image.
    StencilCarve {
        carve_white: bool,
        #[serde(skip)]
        map: Option<std::sync::Arc<Image>>,
    },
    /// Semi-Flatten against `background` (straight sRGB RGBA).
    SemiFlatten {
        background: [f32; 4],
    },
    /// Filter Gallery: a stack of gallery effects applied in order.
    FilterGallery {
        effects: Vec<GalleryEffect>,
    },

    // ---- Fourth batch (v0.11 parity) ----
    /// Radius px scaled per pixel by a mask (external map, layer mask or the layer's alpha).
    VariableBlur {
        radius: f32,
        linear_mask: bool,
        high_quality: bool,
        invert: bool,
        source: MaskSource,
        #[serde(skip)]
        map: Option<std::sync::Arc<Image>>,
    },
    /// Refraction index 1–100; the surroundings stay or take `background`.
    ApplyLens {
        refraction: f32,
        keep_surroundings: bool,
        background: [f32; 4],
    },
    /// Tile size / height / spacing px, neatness and variation 0–1, light direction degrees.
    MosaicTiles {
        tile: MosaicTile,
        size: f32,
        height: f32,
        spacing: f32,
        neatness: f32,
        allow_splitting: bool,
        color_variation: f32,
        light_direction: f32,
        antialiasing: bool,
        color_averaging: bool,
        joints_color: [f32; 4],
        light_color: [f32; 4],
        seed: u32,
    },
    /// One halftone screen per ink: pattern, period px and angle degrees.
    Newsprint {
        model: NewsprintModel,
        pattern: [ScreenPattern; 4],
        period: [f32; 4],
        angle: [f32; 4],
        black_pullout: f32,
        antialiasing: u32,
        turbulence: f32,
        seed: u32,
    },
    /// Upper / lower edge curves as (x, y) in 0..1 (y 0.5 = straight), rotation degrees.
    CurveBend {
        upper: Vec<[f32; 2]>,
        lower: Vec<[f32; 2]>,
        rotation: f32,
        smoothing: bool,
        antialias: bool,
    },
    /// Threshold / softness / strength %, radius px.
    Bloom {
        threshold: f32,
        softness: f32,
        radius: f32,
        strength: f32,
        limit_exposure: bool,
    },
    Sobel {
        horizontal: bool,
        vertical: bool,
        keep_sign: bool,
    },
    /// Mask radius px, percent black 0–1.
    Cartoon {
        mask_radius: f32,
        pct_black: f32,
    },
    /// Blur px, azimuth / elevation degrees, depth.
    Clothify {
        blur_x: f32,
        blur_y: f32,
        azimuth: f32,
        elevation: f32,
        depth: f32,
        seed: u32,
    },
    /// Saturation / lightness / balances −100–100, factor 0–1; optional environment map.
    StencilChrome {
        saturation: f32,
        lightness: f32,
        factor: f32,
        highlight_balance: f32,
        chrome_balance: f32,
        #[serde(skip)]
        map: Option<std::sync::Arc<Image>>,
    },
    /// Azimuth / elevation degrees, depth 1–65, offsets px, water level and ambient 0–1.
    BumpMap {
        kind: BumpType,
        compensate: bool,
        invert: bool,
        tiled: bool,
        azimuth: f32,
        elevation: f32,
        depth: f32,
        offset_x: i32,
        offset_y: i32,
        waterlevel: f32,
        ambient: f32,
        source: MaskSource,
        #[serde(skip)]
        map: Option<std::sync::Arc<Image>>,
    },
    /// Pan / tilt / spin degrees, zoom % (100 = 90° field of view).
    PanoramaProjection {
        pan: f32,
        tilt: f32,
        spin: f32,
        zoom: f32,
        inverse: bool,
        sampler: PanoSampler,
    },
    /// Step px, iterations, rotation degrees; displacement and magnitude maps from the caller.
    WarpMap {
        step_size: f32,
        iterations: u32,
        on_edges: WarpEdge,
        dither: f32,
        rotation_angle: f32,
        substeps: u32,
        edge_color: [f32; 4],
        seed: u32,
        #[serde(skip)]
        map: Option<std::sync::Arc<Image>>,
        #[serde(skip)]
        magnitude: Option<std::sync::Arc<Image>>,
    },
    /// Alpha 0–4 (octave falloff), scale 0–20, z offset, iterations 1–8.
    PerlinNoise {
        alpha: f32,
        scale: f32,
        z_offset: f32,
        iterations: u32,
    },
    /// Turbulence 0.1–7.
    Plasma {
        turbulence: f32,
        seed: u32,
    },
    /// Sizes 0.1–16 cells, detail 0–15 octaves.
    SolidNoise {
        x_size: f32,
        y_size: f32,
        detail: u32,
        tileable: bool,
        turbulent: bool,
        seed: u32,
    },
}

fn default_glow_size() -> f32 {
    100.0
}
fn default_paper_scale() -> f32 {
    6.0
}
fn default_true() -> bool {
    true
}

impl FilterParams {
    /// Pixels read outside an output tile.
    pub fn halo(&self) -> Halo {
        let g = |sigma: f32| Halo::Radius((sigma.max(0.0) * 3.0).ceil() as i32 + 1);
        match self {
            FilterParams::GaussianBlur { radius } | FilterParams::HighPass { radius } => g(*radius),
            FilterParams::UnsharpMask { radius, .. } => Halo::Radius(sharpen::exact::halo(*radius)),
            FilterParams::SmartSharpen { radius, .. } => g(*radius),
            FilterParams::BoxBlur { radius }
            | FilterParams::SurfaceBlur { radius, .. }
            | FilterParams::Median { radius }
            | FilterParams::DustAndScratches { radius, .. }
            | FilterParams::Minimum { radius, .. }
            | FilterParams::Maximum { radius, .. } => Halo::Radius(radius.max(0.0).ceil() as i32 + 1),
            FilterParams::MotionBlur { distance, .. } => Halo::Radius((distance.abs() / 2.0).ceil() as i32 + 2),
            FilterParams::Mosaic { cell_size } => Halo::Radius(cell_size.max(1.0).ceil() as i32 + 1),
            FilterParams::Emboss { height, .. } => Halo::Radius(height.abs().ceil() as i32 + 2),
            FilterParams::FindEdges => Halo::Radius(1),
            FilterParams::AddNoise { .. } | FilterParams::Solarize | FilterParams::Invert | FilterParams::Desaturate { .. } => Halo::Radius(0),
            FilterParams::RadialBlur { .. }
            | FilterParams::Offset { .. }
            | FilterParams::Twirl { .. }
            | FilterParams::Pinch { .. }
            | FilterParams::Spherize { .. }
            | FilterParams::Wave { .. }
            | FilterParams::Ripple { .. }
            | FilterParams::PolarCoordinates { .. } => Halo::Bounds,
            other => halo_ext(other),
        }
    }

    /// [`Self::halo`] using the document/selection bounds so filters whose reach
    /// depends on size (Relight's shading blur) request the neighbourhood they actually read.
    pub fn halo_for(&self, bounds: Rect) -> Halo {
        match self {
            FilterParams::Relight { intensity, softness, .. } if *intensity != 0.0 => Halo::Radius(relight::halo_radius(*softness, bounds)),
            _ => self.halo(),
        }
    }

    /// Whether the filter's job is to make pixels transparent (Color to Alpha): it needs a layer
    /// that can hold transparency, so callers turn the Background into a normal layer first and
    /// refuse a layer whose transparency is locked instead of putting the old alpha back.
    pub fn makes_transparency(&self) -> bool {
        matches!(self, FilterParams::ColorToAlpha { .. } | FilterParams::PageCurl { .. } | FilterParams::CurveBend { .. })
    }

    /// Whether the filter moves pixels around the reference bounds (its
    /// output area is the bounds, not the layer's content grown by the halo).
    pub fn is_global(&self) -> bool {
        self.halo() == Halo::Bounds
    }

    /// Whether the filter renders new pixels without reading the layer's own (Fibers): it fills
    /// the reference bounds even where the layer is transparent, as Photoshop's does on an
    /// empty layer, while still reading no neighbourhood.
    pub fn renders_from_nothing(&self) -> bool {
        matches!(self, FilterParams::Fibers { .. }) || gen_renders_from_nothing(self)
    }

    /// Human-readable name.
    pub fn label(&self) -> &'static str {
        match self {
            FilterParams::GaussianBlur { .. } => "Gaussian Blur",
            FilterParams::BoxBlur { .. } => "Box Blur",
            FilterParams::MotionBlur { .. } => "Motion Blur",
            FilterParams::RadialBlur { .. } => "Radial Blur",
            FilterParams::SurfaceBlur { .. } => "Surface Blur",
            FilterParams::UnsharpMask { .. } => "Unsharp Mask",
            FilterParams::SmartSharpen { .. } => "Smart Sharpen",
            FilterParams::HighPass { .. } => "High Pass",
            FilterParams::AddNoise { .. } => "Add Noise",
            FilterParams::Median { .. } => "Median",
            FilterParams::DustAndScratches { .. } => "Dust & Scratches",
            FilterParams::Minimum { .. } => "Minimum",
            FilterParams::Maximum { .. } => "Maximum",
            FilterParams::Offset { .. } => "Offset",
            FilterParams::Mosaic { .. } => "Mosaic",
            FilterParams::Emboss { .. } => "Emboss",
            FilterParams::FindEdges => "Find Edges",
            FilterParams::Solarize => "Solarize",
            FilterParams::Invert => "Invert",
            FilterParams::Desaturate { .. } => "Desaturate",
            FilterParams::Twirl { .. } => "Twirl",
            FilterParams::Pinch { .. } => "Pinch",
            FilterParams::Spherize { .. } => "Spherize",
            FilterParams::Wave { .. } => "Wave",
            FilterParams::Ripple { .. } => "Ripple",
            FilterParams::PolarCoordinates { .. } => "Polar Coordinates",
            FilterParams::ColorHalftone { .. } => "Color Halftone",
            FilterParams::Crystallize { .. } => "Crystallize",
            FilterParams::Facet => "Facet",
            FilterParams::Fragment => "Fragment",
            FilterParams::Mezzotint { .. } => "Mezzotint",
            FilterParams::Pointillize { .. } => "Pointillize",
            FilterParams::Diffuse { .. } => "Diffuse",
            FilterParams::Extrude { .. } => "Extrude",
            FilterParams::OilPaint { .. } => "Oil Paint",
            FilterParams::Tiles { .. } => "Tiles",
            FilterParams::TraceContour { .. } => "Trace Contour",
            FilterParams::Edge { .. } => "Edge",
            FilterParams::ColorTemperature { .. } => "Color Temperature",
            FilterParams::HueChroma { .. } => "Hue-Chroma",
            FilterParams::Wind { .. } => "Wind",
            FilterParams::Displace { .. } => "Displace",
            FilterParams::Shear { .. } => "Shear",
            FilterParams::ZigZag { .. } => "ZigZag",
            FilterParams::Fibers { .. } => "Fibers",
            FilterParams::LensFlare { .. } => "Lens Flare",
            FilterParams::LightingEffects { .. } => "Lighting Effects",
            FilterParams::Relight { .. } => "Relight",
            FilterParams::ReduceNoise { .. } => "Reduce Noise",
            FilterParams::SmartBlur { .. } => "Smart Blur",
            FilterParams::LensBlur { .. } => "Lens Blur",
            FilterParams::ShapeBlur { .. } => "Shape Blur",
            FilterParams::TiltShift { .. } => "Tilt-Shift",
            FilterParams::IrisBlur { .. } => "Iris Blur",
            FilterParams::FieldBlur { .. } => "Field Blur",
            FilterParams::SpinBlur { .. } => "Spin Blur",
            FilterParams::PathBlur { .. } => "Path Blur",
            FilterParams::Custom { .. } => "Custom",
            FilterParams::HsbHsl { .. } => "HSB/HSL",
            FilterParams::ColorToAlpha { .. } => "Color to Alpha",
            FilterParams::DeInterlace { .. } => "De-Interlace",
            FilterParams::NtscColors => "NTSC Colors",
            FilterParams::MeanCurvatureBlur { .. } => "Mean Curvature Blur",
            FilterParams::TileableBlur { .. } => "Tileable Blur",
            FilterParams::Antialias => "Antialias",
            FilterParams::SymmetricNearestNeighbor { .. } => "Symmetric Nearest Neighbor",
            FilterParams::NlFilter { .. } => "NL Filter",
            FilterParams::Engrave { .. } => "Engrave",
            FilterParams::Kaleidoscope { .. } => "Kaleidoscope",
            FilterParams::Shift { .. } => "Shift",
            FilterParams::ValuePropagate { .. } => "Value Propagate",
            FilterParams::VideoDegradation { .. } => "Video Degradation",
            FilterParams::Blinds { .. } => "Blinds",
            FilterParams::PageCurl { .. } => "Page Curl",
            FilterParams::Supernova { .. } => "Supernova",
            FilterParams::GradientFlare { .. } => "Gradient Flare",
            FilterParams::Sparkle { .. } => "Sparkle",
            FilterParams::LongShadow { .. } => "Long Shadow",
            FilterParams::PerspectiveShadow { .. } => "Perspective Shadow",
            FilterParams::XachEffect { .. } => "Xach-Effect",
            FilterParams::LchNoise { .. } => "CIE lch Noise",
            FilterParams::HsvNoise { .. } => "HSV Noise",
            FilterParams::Hurl { .. } => "Hurl",
            FilterParams::Pick { .. } => "Pick",
            FilterParams::Slur { .. } => "Slur",
            FilterParams::DifferenceOfGaussians { .. } => "Difference of Gaussians",
            FilterParams::Laplace => "Laplace",
            FilterParams::ImageGradient { .. } => "Image Gradient",
            FilterParams::DistanceMap { .. } => "Distance Map",
            FilterParams::NormalMap { .. } => "Normal Map",
            FilterParams::OperationGraph { .. } => "Operation Graph",
            FilterParams::Cubism { .. } => "Cubism",
            FilterParams::Slic { .. } => "SLIC",
            FilterParams::Waterpixels { .. } => "Waterpixels",
            FilterParams::Impressionist { .. } => "Impressionist",
            FilterParams::VanGogh { .. } => "Van Gogh (LIC)",
            FilterParams::Weave { .. } => "Weave",
            FilterParams::Filmstrip { .. } => "Filmstrip",
            FilterParams::FuzzyBorder { .. } => "Fuzzy Border",
            FilterParams::FilterGallery { effects } => match effects.as_slice() {
                [one] => one.filter.name(),
                _ => "Filter Gallery",
            },
            other => gen_label(other),
        }
    }
}

/// Whether a third-batch filter renders the whole bounds without reading the layer's pixels
/// (generators), or paints over them from nothing (the drawing filters), so an empty layer
/// still gets the result.
fn gen_renders_from_nothing(p: &FilterParams) -> bool {
    matches!(
        p,
        FilterParams::FractalExplorer { .. }
            | FilterParams::IfsFractal { .. }
            | FilterParams::SphereDesigner { .. }
            | FilterParams::CellNoise { .. }
            | FilterParams::SimplexNoise { .. }
            | FilterParams::Lava { .. }
            | FilterParams::Circuit { .. }
            | FilterParams::Qbist { .. }
            | FilterParams::CmlExplorer { .. }
            | FilterParams::BayerMatrix { .. }
            | FilterParams::Checkerboard { .. }
            | FilterParams::DiffractionPatterns { .. }
            | FilterParams::Grid { .. }
            | FilterParams::LinearSinusoid { .. }
            | FilterParams::Maze { .. }
            | FilterParams::Sinus { .. }
            | FilterParams::Spiral { .. }
            | FilterParams::Jigsaw { .. }
            | FilterParams::LineNova { .. }
            | FilterParams::Spirograph { .. }
            | FilterParams::Slide { .. }
            | FilterParams::CoffeeStain { .. }
            | FilterParams::PerlinNoise { .. }
            | FilterParams::Plasma { .. }
            | FilterParams::SolidNoise { .. }
    )
}

/// Labels of the third filter batch (map, render, decor).
fn gen_label(p: &FilterParams) -> &'static str {
    match p {
        FilterParams::FractalTrace { .. } => "Fractal Trace",
        FilterParams::Illusion { .. } => "Illusion",
        FilterParams::LittlePlanet { .. } => "Little Planet",
        FilterParams::RecursiveTransform { .. } => "Recursive Transform",
        FilterParams::TileSeamless => "Tile Seamless",
        FilterParams::MapObject { .. } => "Map Object",
        FilterParams::SmallTiles { .. } => "Small Tiles",
        FilterParams::FractalExplorer { .. } => "Fractal Explorer",
        FilterParams::IfsFractal { .. } => "IFS Fractal",
        FilterParams::SphereDesigner { .. } => "Sphere Designer",
        FilterParams::CellNoise { .. } => "Cell Noise",
        FilterParams::SimplexNoise { .. } => "Simplex Noise",
        FilterParams::Lava { .. } => "Lava",
        FilterParams::Circuit { .. } => "Circuit",
        FilterParams::Qbist { .. } => "Qbist",
        FilterParams::CmlExplorer { .. } => "CML Explorer",
        FilterParams::BayerMatrix { .. } => "Bayer Matrix",
        FilterParams::Checkerboard { .. } => "Checkerboard",
        FilterParams::DiffractionPatterns { .. } => "Diffraction Patterns",
        FilterParams::Grid { .. } => "Grid",
        FilterParams::LinearSinusoid { .. } => "Linear Sinusoid",
        FilterParams::Maze { .. } => "Maze",
        FilterParams::Sinus { .. } => "Sinus",
        FilterParams::Spiral { .. } => "Spiral",
        FilterParams::Jigsaw { .. } => "Jigsaw",
        FilterParams::LineNova { .. } => "Line Nova",
        FilterParams::Spirograph { .. } => "Spirograph",
        FilterParams::Slide { .. } => "Slide",
        FilterParams::CoffeeStain { .. } => "Coffee Stain",
        FilterParams::StencilCarve { .. } => "Stencil Carve",
        FilterParams::SemiFlatten { .. } => "Semi-Flatten",
        other => extra2_label(other),
    }
}

/// Labels of the fourth batch.
fn extra2_label(p: &FilterParams) -> &'static str {
    match p {
        FilterParams::VariableBlur { .. } => "Variable Blur",
        FilterParams::ApplyLens { .. } => "Apply Lens",
        FilterParams::MosaicTiles { .. } => "Mosaic Tiles",
        FilterParams::Newsprint { .. } => "Newsprint",
        FilterParams::CurveBend { .. } => "Curve Bend",
        FilterParams::Bloom { .. } => "Bloom",
        FilterParams::Sobel { .. } => "Sobel",
        FilterParams::Cartoon { .. } => "Cartoon",
        FilterParams::Clothify { .. } => "Clothify",
        FilterParams::StencilChrome { .. } => "Stencil Chrome",
        FilterParams::BumpMap { .. } => "Bump Map",
        FilterParams::PanoramaProjection { .. } => "Panorama Projection",
        FilterParams::WarpMap { .. } => "Warp",
        FilterParams::PerlinNoise { .. } => "Perlin Noise",
        FilterParams::Plasma { .. } => "Plasma",
        FilterParams::SolidNoise { .. } => "Solid Noise",
        _ => "Filter",
    }
}

/// Halos of the second filter batch.
fn halo_ext(p: &FilterParams) -> Halo {
    let r = |v: f32| Halo::Radius(v.max(0.0).ceil() as i32 + 1);
    match p {
        FilterParams::ColorHalftone { max_radius, .. } => r(max_radius * 3.0),
        FilterParams::Crystallize { cell_size, .. } => r(cell_size.max(1.0) * 3.0),
        FilterParams::Pointillize { cell_size, .. } => r(cell_size.max(1.0) * 2.0),
        FilterParams::Facet => r(3.0),
        FilterParams::Fragment => r(4.0),
        FilterParams::Mezzotint { .. } | FilterParams::HsbHsl { .. } | FilterParams::ColorToAlpha { .. } | FilterParams::NtscColors => Halo::Radius(0),
        FilterParams::Diffuse { .. } => r(1.0),
        FilterParams::Extrude { size, depth, .. } => r(stylize2::extrude_reach(*depth) + size.clamp(2.0, 255.0) * 3.0 + 1.0),
        FilterParams::OilPaint { stylization, scale, .. } => r(oil::reach(*stylization, *scale)),
        FilterParams::Tiles { .. } => Halo::Bounds,
        FilterParams::TraceContour { .. } | FilterParams::Edge { .. } => r(1.0),
        FilterParams::ColorTemperature { .. } | FilterParams::HueChroma { .. } => Halo::Radius(0),
        FilterParams::Wind { method, .. } => r(stylize2::wind_reach(*method)),
        FilterParams::Displace { .. } | FilterParams::Shear { .. } | FilterParams::ZigZag { .. } | FilterParams::SpinBlur { .. } => Halo::Bounds,
        // Generators and per-pixel renders only need the bounds geometry (always in the context).
        FilterParams::Fibers { .. } | FilterParams::LensFlare { .. } => Halo::Radius(0),
        FilterParams::LightingEffects { .. } => r(1.0),
        FilterParams::Relight { intensity, .. } if *intensity == 0.0 => Halo::Radius(0),
        FilterParams::Relight { .. } => Halo::Radius(relight::halo_radius_max()),
        FilterParams::ReduceNoise { .. } => r(denoise::reach()),
        FilterParams::SmartBlur { radius, .. } => r(*radius),
        FilterParams::LensBlur { radius, .. } => r(*radius),
        FilterParams::ShapeBlur { radius, .. } => r(shape_blur::radius(*radius)),
        FilterParams::TiltShift { blur, .. } => r(gallery::reach(*blur)),
        FilterParams::IrisBlur { pins } => r(gallery::reach(pins.iter().map(|p| p.blur).fold(0.0, f32::max))),
        FilterParams::FieldBlur { pins } => r(gallery::reach(pins.iter().map(|p| p.blur).fold(0.0, f32::max))),
        FilterParams::PathBlur { paths } => r(paths.iter().map(|p| p.speed.abs()).fold(0.0, f32::max) / 2.0 + 1.0),
        FilterParams::Custom { .. } => r(2.0),
        FilterParams::DeInterlace { .. } => r(1.0),
        FilterParams::FilterGallery { effects } => Halo::Radius(artistic_fx::reach(effects)),
        other => halo_extra(other),
    }
}

/// Halos of the third filter batch.
fn halo_extra(p: &FilterParams) -> Halo {
    let r = |v: f32| Halo::Radius(v.max(0.0).ceil() as i32 + 1);
    match p {
        FilterParams::MeanCurvatureBlur { iterations } => Halo::Radius((*iterations).clamp(1, 500) as i32 + 2),
        FilterParams::TileableBlur { .. } | FilterParams::Kaleidoscope { .. } | FilterParams::Blinds { .. } | FilterParams::PageCurl { .. } => Halo::Bounds,
        FilterParams::Antialias | FilterParams::NlFilter { .. } | FilterParams::ValuePropagate { .. } => r(1.0),
        FilterParams::SymmetricNearestNeighbor { radius, .. } => r(radius.clamp(1.0, 100.0)),
        FilterParams::Engrave { height, .. } => r(height.clamp(2.0, 16.0)),
        FilterParams::Shift { amount, .. } => r(amount.clamp(0.0, 200.0)),
        FilterParams::VideoDegradation { .. } | FilterParams::Supernova { .. } | FilterParams::GradientFlare { .. } => Halo::Radius(0),
        FilterParams::Sparkle { spike_length, .. } => r(extra::light::sparkle_reach(*spike_length)),
        FilterParams::LongShadow { style, length, midpoint, .. } => match extra::light::long_shadow_reach(*style, *length, *midpoint) {
            Some(reach) => r(reach),
            None => Halo::Bounds,
        },
        FilterParams::PerspectiveShadow { .. } | FilterParams::DistanceMap { .. } | FilterParams::Filmstrip { .. } => Halo::Bounds,
        FilterParams::XachEffect { blur, highlight_x, highlight_y, shadow_x, shadow_y, .. } => {
            r(extra::light::xach_reach(*blur, (*highlight_x, *highlight_y), (*shadow_x, *shadow_y)))
        }
        FilterParams::LchNoise { .. } | FilterParams::HsvNoise { .. } | FilterParams::Hurl { .. } => Halo::Radius(0),
        FilterParams::Pick { repeat, .. } | FilterParams::Slur { repeat, .. } => Halo::Radius((*repeat).clamp(1, 100) as i32 + 1),
        FilterParams::DifferenceOfGaussians { radius1, radius2, .. } => r(extra::edge::dog_reach(*radius1, *radius2)),
        FilterParams::Laplace | FilterParams::ImageGradient { .. } => r(1.0),
        FilterParams::NormalMap { tileable, .. } => {
            if *tileable {
                Halo::Bounds
            } else {
                r(1.0)
            }
        }
        FilterParams::OperationGraph { source } => r(extra::graph::reach(source)),
        FilterParams::Cubism { tile_size, saturation, .. } => r(extra::artistic::cubism_reach(*tile_size, *saturation)),
        FilterParams::Slic { cluster_size, .. } => r(extra::artistic::slic_reach(*cluster_size)),
        FilterParams::Waterpixels { size, .. } => r(extra::artistic::waterpixels_reach(*size)),
        FilterParams::Impressionist { brush_size, size_variation, .. } => r(extra::artistic::impressionist_reach(*brush_size, *size_variation)),
        FilterParams::VanGogh { integration_steps, .. } => r(extra::artistic::lic_reach(*integration_steps) + 2.0),
        FilterParams::Weave { .. } | FilterParams::FuzzyBorder { .. } => Halo::Radius(0),
        FilterParams::FractalTrace { .. }
        | FilterParams::Illusion { .. }
        | FilterParams::LittlePlanet { .. }
        | FilterParams::RecursiveTransform { .. }
        | FilterParams::TileSeamless
        | FilterParams::MapObject { .. }
        | FilterParams::SmallTiles { .. }
        | FilterParams::SphereDesigner { .. }
        | FilterParams::CmlExplorer { .. } => Halo::Bounds,
        // Fourth batch.
        FilterParams::VariableBlur { radius, .. } => r(extra2::variable_blur_reach(*radius)),
        FilterParams::Bloom { radius, .. } => r(extra2::bloom_reach(*radius)),
        FilterParams::Sobel { .. } => r(1.0),
        FilterParams::Cartoon { mask_radius, .. } => r(extra2::cartoon_reach(*mask_radius)),
        FilterParams::Clothify { blur_x, blur_y, .. } => r(extra2::clothify_reach(*blur_x, *blur_y)),
        FilterParams::StencilChrome { .. } => r(extra2::surface::chrome_reach()),
        FilterParams::BumpMap { .. } => r(2.0),
        FilterParams::MosaicTiles { size, .. } => r(extra2::distort::mosaic_reach(*size)),
        FilterParams::ApplyLens { .. } | FilterParams::CurveBend { .. } | FilterParams::PanoramaProjection { .. } | FilterParams::WarpMap { .. } => {
            Halo::Bounds
        }
        // The other generators and the per-pixel decor filters only need the bounds geometry.
        _ => Halo::Radius(0),
    }
}

/// Context a filter runs in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ctx {
    /// Reference area: distortions are centred on it and sample within it
    /// (Photoshop uses the selection bounds, else the canvas).
    pub bounds: Rect,
    /// Colour model of the samples (the last channel is alpha if `alpha`).
    pub mode: ColorMode,
    /// Whether the last channel is alpha.
    pub alpha: bool,
}

/// Runs the filter kernel for `out` from `src` (which covers `out` grown by
/// the halo, or the bounds for global filters). Returns interleaved samples.
pub fn kernel(params: &FilterParams, src: &Image, out: Rect, ctx: &Ctx) -> Vec<f32> {
    match params {
        FilterParams::GaussianBlur { radius } => blur::gaussian(src, out, ctx, *radius),
        FilterParams::BoxBlur { radius } => blur::boxed(src, out, ctx, *radius),
        FilterParams::MotionBlur { angle, distance } => blur::motion(src, out, ctx, *angle, *distance),
        FilterParams::RadialBlur { amount, method, quality, center_x, center_y } => {
            blur::radial(src, out, ctx, *amount, *method, *quality, (*center_x, *center_y))
        }
        FilterParams::SurfaceBlur { radius, threshold } => blur::surface(src, out, ctx, *radius, *threshold),
        FilterParams::UnsharpMask { amount, radius, threshold } => sharpen::unsharp(src, out, ctx, *amount, *radius, *threshold),
        FilterParams::SmartSharpen { amount, radius, reduce_noise } => sharpen::smart(src, out, ctx, *amount, *radius, *reduce_noise),
        FilterParams::HighPass { radius } => sharpen::high_pass(src, out, ctx, *radius),
        FilterParams::AddNoise { amount, distribution, monochromatic, seed } => noise::add(src, out, ctx, *amount, *distribution, *monochromatic, *seed),
        FilterParams::Median { radius } => noise::median(src, out, *radius, None),
        FilterParams::DustAndScratches { radius, threshold } => noise::median(src, out, *radius, Some(*threshold)),
        FilterParams::Minimum { radius, preserve } => other::min_max_preserve(src, out, *radius, false, *preserve),
        FilterParams::Maximum { radius, preserve } => other::min_max_preserve(src, out, *radius, true, *preserve),
        FilterParams::Offset { horizontal, vertical, undefined } => other::offset(src, out, ctx, *horizontal, *vertical, *undefined),
        FilterParams::Mosaic { cell_size } => stylize::mosaic(src, out, ctx, *cell_size),
        FilterParams::Emboss { angle, height, amount } => stylize::emboss(src, out, ctx, *angle, *height, *amount),
        FilterParams::FindEdges => stylize::find_edges(src, out, ctx),
        FilterParams::Solarize => stylize::solarize(src, out, ctx),
        FilterParams::Invert => stylize::per_pixel(src, out, ctx, |v| 1.0 - v),
        FilterParams::Desaturate { mode } => stylize::desaturate(src, out, ctx, *mode),
        FilterParams::Twirl { angle } => distort::twirl(src, out, ctx, *angle),
        FilterParams::Pinch { amount } => distort::pinch(src, out, ctx, *amount),
        FilterParams::Spherize { amount, mode } => distort::spherize(src, out, ctx, *amount, *mode),
        FilterParams::Wave { generators, wavelength_min, wavelength_max, amplitude_min, amplitude_max, wave_type, undefined, seed } => distort::wave(
            src,
            out,
            ctx,
            distort::WaveSpec {
                generators: *generators,
                wavelength: (*wavelength_min, *wavelength_max),
                amplitude: (*amplitude_min, *amplitude_max),
                wave_type: *wave_type,
                undefined: *undefined,
                seed: *seed,
            },
        ),
        FilterParams::Ripple { amount, size } => distort::ripple(src, out, ctx, *amount, *size),
        FilterParams::PolarCoordinates { mode } => distort::polar(src, out, ctx, *mode),
        FilterParams::ColorHalftone { max_radius, angles } => pixelate::color_halftone(src, out, ctx, *max_radius, *angles),
        FilterParams::Crystallize { cell_size, seed } => pixelate::crystallize(src, out, ctx, *cell_size, *seed),
        FilterParams::Facet => pixelate::facet(src, out, ctx),
        FilterParams::Fragment => pixelate::fragment(src, out, ctx),
        FilterParams::Mezzotint { kind, seed } => pixelate::mezzotint(src, out, ctx, *kind, *seed),
        FilterParams::Pointillize { cell_size, seed, background } => pixelate::pointillize(src, out, ctx, *cell_size, *seed, *background),
        FilterParams::Diffuse { mode, seed } => stylize2::diffuse(src, out, ctx, *mode, *seed),
        FilterParams::Extrude { kind, size, depth, level_based, solid_front, mask_incomplete, seed } => stylize2::extrude(
            src,
            out,
            ctx,
            &stylize2::ExtrudeSpec {
                kind: *kind,
                size: *size,
                depth: *depth,
                level_based: *level_based,
                solid_front: *solid_front,
                mask_incomplete: *mask_incomplete,
                seed: *seed,
            },
        ),
        FilterParams::OilPaint { stylization, cleanliness, scale, bristle_detail, lighting, angle, shine } => oil::oil_paint(
            src,
            out,
            ctx,
            &oil::OilSpec {
                stylization: *stylization,
                cleanliness: *cleanliness,
                scale: *scale,
                bristle_detail: *bristle_detail,
                lighting: *lighting,
                angle: *angle,
                shine: *shine,
            },
        ),
        FilterParams::Tiles { count, max_offset, fill, foreground, background, seed } => {
            stylize2::tiles(src, out, ctx, *count, *max_offset, *fill, (*foreground, *background), *seed)
        }
        FilterParams::TraceContour { level, upper } => stylize2::trace_contour(src, out, ctx, *level, *upper),
        FilterParams::Edge { algorithm, amount } => stylize2::edge(src, out, ctx, *algorithm, *amount),
        FilterParams::ColorTemperature { original, intended } => adjust_ext::color_temperature(src, out, ctx, *original, *intended),
        FilterParams::HueChroma { hue, chroma, lightness } => adjust_ext::hue_chroma(src, out, ctx, *hue, *chroma, *lightness),
        FilterParams::Wind { method, from_right, seed } => stylize2::wind(src, out, ctx, *method, *from_right, *seed),
        FilterParams::Displace { horizontal, vertical, stretch, undefined, map } => {
            distort2::displace(src, out, ctx, (*horizontal, *vertical), *stretch, *undefined, map.as_deref())
        }
        FilterParams::Shear { points, undefined } => distort2::shear(src, out, ctx, points, *undefined),
        FilterParams::ZigZag { amount, ridges, style } => distort2::zigzag(src, out, ctx, *amount, *ridges, *style),
        FilterParams::Fibers { variance, strength, seed, foreground, background } => {
            render::fibers(src, out, ctx, *variance, *strength, *seed, (*foreground, *background))
        }
        FilterParams::LensFlare { brightness, center_x, center_y, lens } => render::lens_flare(src, out, ctx, *brightness, (*center_x, *center_y), *lens),
        FilterParams::LightingEffects { lights, gloss, metallic, exposure, ambience, texture, height, white_is_high } => render::lighting(
            src,
            out,
            ctx,
            &render::LightingSpec {
                lights,
                gloss: *gloss,
                metallic: *metallic,
                exposure: *exposure,
                ambience: *ambience,
                texture: *texture,
                height: *height,
                white_is_high: *white_is_high,
            },
        ),
        FilterParams::Relight { angle, elevation, intensity, ambient, warmth, softness } => relight::relight(
            src,
            out,
            ctx,
            relight::Params { angle: *angle, elevation: *elevation, intensity: *intensity, ambient: *ambient, warmth: *warmth, softness: *softness },
        ),
        FilterParams::ReduceNoise { strength, preserve_details, reduce_color_noise, sharpen_details, remove_jpeg_artifact } => denoise::reduce_noise(
            src,
            out,
            ctx,
            &denoise::DenoiseSpec {
                strength: *strength,
                preserve: *preserve_details,
                color: *reduce_color_noise,
                sharpen: *sharpen_details,
                jpeg: *remove_jpeg_artifact,
            },
        ),
        FilterParams::SmartBlur { radius, threshold, quality, mode } => blur2::smart_blur(src, out, ctx, *radius, *threshold, *quality, *mode),
        FilterParams::LensBlur {
            radius,
            blades,
            curvature,
            rotation,
            depth,
            focal_distance,
            invert_depth,
            brightness,
            threshold,
            noise,
            distribution,
            monochromatic,
            seed,
            depth_map,
        } => blur2::lens_blur(
            src,
            out,
            ctx,
            &blur2::LensSpec {
                radius: *radius,
                blades: *blades,
                curvature: *curvature,
                rotation: *rotation,
                depth: *depth,
                focal: *focal_distance,
                invert: *invert_depth,
                brightness: *brightness,
                threshold: *threshold,
                noise: *noise,
                distribution: *distribution,
                mono: *monochromatic,
                seed: *seed,
                depth_map: depth_map.as_deref(),
            },
        ),
        FilterParams::ShapeBlur { radius, shape } => blur2::shape_blur(src, out, ctx, *radius, *shape),
        FilterParams::TiltShift { blur, center_x, center_y, angle, focus, transition } => {
            gallery::tilt_shift(src, out, ctx, *blur, (*center_x, *center_y), *angle, *focus, *transition)
        }
        FilterParams::IrisBlur { pins } => gallery::iris(src, out, ctx, pins),
        FilterParams::FieldBlur { pins } => gallery::field(src, out, ctx, pins),
        FilterParams::SpinBlur { pins } => gallery::spin(src, out, ctx, pins),
        FilterParams::PathBlur { paths } => gallery::path(src, out, ctx, paths),
        FilterParams::Custom { kernel, scale, offset } => other2::custom(src, out, ctx, kernel, *scale, *offset),
        FilterParams::HsbHsl { input, output } => other2::hsb_hsl(src, out, ctx, *input, *output),
        FilterParams::ColorToAlpha { color, transparency_threshold, opacity_threshold } => {
            color_to_alpha::color_to_alpha(src, out, ctx, *color, *transparency_threshold, *opacity_threshold)
        }
        FilterParams::DeInterlace { eliminate_even, interpolate } => video::deinterlace(src, out, ctx, *eliminate_even, *interpolate),
        FilterParams::NtscColors => video::ntsc(src, out, ctx),
        FilterParams::MeanCurvatureBlur { iterations } => extra::blur::mean_curvature(src, out, ctx, *iterations),
        FilterParams::TileableBlur { radius, horizontal, vertical, blur_type } => {
            extra::blur::tileable(src, out, ctx, *radius, *horizontal, *vertical, *blur_type)
        }
        FilterParams::Antialias => extra::blur::antialias(src, out, ctx),
        FilterParams::SymmetricNearestNeighbor { radius, pairs } => extra::blur::snn(src, out, ctx, *radius, *pairs),
        FilterParams::NlFilter { alpha, radius, mode } => extra::blur::nl_filter(src, out, ctx, *alpha, *radius, *mode),
        FilterParams::Engrave { height, limit_line_width } => extra::distort::engrave(src, out, ctx, *height, *limit_line_width),
        FilterParams::Kaleidoscope { mirrors, mirror_rotation, result_rotation, offset_x, offset_y, zoom, trim_x, trim_y, clip } => {
            extra::distort::kaleidoscope(
                src,
                out,
                ctx,
                &extra::distort::KaleidoscopeSpec {
                    mirrors: *mirrors,
                    mirror_rotation: *mirror_rotation,
                    result_rotation: *result_rotation,
                    offset: (*offset_x, *offset_y),
                    zoom: *zoom,
                    trim: (*trim_x, *trim_y),
                    clip: *clip,
                },
            )
        }
        FilterParams::Shift { amount, axis, seed } => extra::distort::shift(src, out, ctx, *amount, *axis, *seed),
        FilterParams::ValuePropagate { mode, lower_threshold, upper_threshold, rate, color, top, left, right, bottom } => extra::distort::value_propagate(
            src,
            out,
            ctx,
            &extra::distort::PropagateSpec {
                mode: *mode,
                lower: *lower_threshold,
                upper: *upper_threshold,
                rate: *rate,
                color: *color,
                directions: [*top, *left, *right, *bottom],
            },
        ),
        FilterParams::VideoDegradation { pattern, additive, rotated } => extra::distort::video_degradation(src, out, ctx, *pattern, *additive, *rotated),
        FilterParams::Blinds { displacement, segments, axis, transparent, background } => {
            extra::distort::blinds(src, out, ctx, *displacement, *segments, *axis, *transparent, *background)
        }
        FilterParams::PageCurl { corner, axis, shade, colors, opacity, foreground, background } => extra::distort::page_curl(
            src,
            out,
            ctx,
            &extra::distort::CurlSpec {
                corner: *corner,
                axis: *axis,
                shade: *shade,
                colors: *colors,
                opacity: *opacity,
                foreground: *foreground,
                background: *background,
            },
        ),
        FilterParams::Supernova { center_x, center_y, radius, spokes, random_hue, color, seed } => extra::light::supernova(
            src,
            out,
            ctx,
            &extra::light::SupernovaSpec {
                center: (*center_x, *center_y),
                radius: *radius,
                spokes: *spokes,
                random_hue: *random_hue,
                color: *color,
                seed: *seed,
            },
        ),
        FilterParams::GradientFlare {
            center_x,
            center_y,
            radius,
            rotation,
            hue_rotation,
            vector_angle,
            vector_length,
            glow,
            rays,
            second_flares,
            design,
            ray_count,
            glow_size,
            ghost_count,
        } => extra::light::gradient_flare(
            src,
            out,
            ctx,
            &extra::light::FlareSpec {
                center: (*center_x, *center_y),
                radius: *radius,
                rotation: *rotation,
                hue_rotation: *hue_rotation,
                vector_angle: *vector_angle,
                vector_length: *vector_length,
                glow: *glow,
                rays: *rays,
                second_flares: *second_flares,
                design: *design,
                ray_count: *ray_count,
                glow_size: *glow_size,
                ghost_count: *ghost_count,
            },
        ),
        FilterParams::Sparkle {
            threshold,
            flare_intensity,
            spike_length,
            spike_points,
            spike_angle,
            density,
            transparency,
            random_hue,
            random_saturation,
            preserve_luminosity,
            inverse,
            border,
            color,
            foreground,
            background,
            seed,
        } => extra::light::sparkle(
            src,
            out,
            ctx,
            &extra::light::SparkleSpec {
                threshold: *threshold,
                flare_intensity: *flare_intensity,
                spike_length: *spike_length,
                spike_points: *spike_points,
                spike_angle: *spike_angle,
                density: *density,
                transparency: *transparency,
                random_hue: *random_hue,
                random_saturation: *random_saturation,
                preserve_luminosity: *preserve_luminosity,
                inverse: *inverse,
                border: *border,
                color: *color,
                foreground: *foreground,
                background: *background,
                seed: *seed,
            },
        ),
        FilterParams::LongShadow { style, angle, length, midpoint, color, composition } => extra::light::long_shadow(
            src,
            out,
            ctx,
            &extra::light::LongShadowSpec { style: *style, angle: *angle, length: *length, midpoint: *midpoint, color: *color, composition: *composition },
        ),
        FilterParams::PerspectiveShadow { angle, distance, length, blur, color, opacity, interpolation, allow_resizing: _ } => {
            extra::light::perspective_shadow(
                src,
                out,
                ctx,
                &extra::light::PerspectiveShadowSpec {
                    angle: *angle,
                    distance: *distance,
                    length: *length,
                    blur: *blur,
                    color: *color,
                    opacity: *opacity,
                    interpolation: *interpolation,
                },
            )
        }
        FilterParams::XachEffect { highlight_x, highlight_y, highlight_color, highlight_opacity, shadow_color, shadow_opacity, blur, shadow_x, shadow_y } => {
            extra::light::xach(
                src,
                out,
                ctx,
                &extra::light::XachSpec {
                    highlight_offset: (*highlight_x, *highlight_y),
                    highlight_color: *highlight_color,
                    highlight_opacity: *highlight_opacity,
                    shadow_color: *shadow_color,
                    shadow_opacity: *shadow_opacity,
                    blur: *blur,
                    shadow_offset: (*shadow_x, *shadow_y),
                },
            )
        }
        FilterParams::LchNoise { dulling, lightness, chroma, hue, seed } => extra::noise::lch_noise(src, out, ctx, *dulling, *lightness, *chroma, *hue, *seed),
        FilterParams::HsvNoise { dulling, hue, saturation, value, seed } => extra::noise::hsv_noise(src, out, ctx, *dulling, *hue, *saturation, *value, *seed),
        FilterParams::Hurl { randomization, repeat, seed } => extra::noise::hurl(src, out, ctx, *randomization, *repeat, *seed),
        FilterParams::Pick { randomization, repeat, seed } => extra::noise::pick(src, out, ctx, *randomization, *repeat, *seed),
        FilterParams::Slur { randomization, repeat, seed } => extra::noise::slur(src, out, ctx, *randomization, *repeat, *seed),
        FilterParams::DifferenceOfGaussians { radius1, radius2, normalize, invert } => {
            extra::edge::difference_of_gaussians(src, out, ctx, *radius1, *radius2, *normalize, *invert)
        }
        FilterParams::Laplace => extra::edge::laplace(src, out, ctx),
        FilterParams::ImageGradient { output } => extra::edge::image_gradient(src, out, ctx, *output),
        FilterParams::DistanceMap { metric, threshold_low, threshold_high, averaging, normalize } => {
            extra::edge::distance_map(src, out, ctx, *metric, *threshold_low, *threshold_high, *averaging, *normalize)
        }
        FilterParams::NormalMap { scale, x_component, y_component, flip_x, flip_y, full_z, tileable } => extra::edge::normal_map(
            src,
            out,
            ctx,
            &extra::edge::NormalSpec {
                scale: *scale,
                x_component: *x_component,
                y_component: *y_component,
                flip_x: *flip_x,
                flip_y: *flip_y,
                full_z: *full_z,
                tileable: *tileable,
            },
        ),
        FilterParams::OperationGraph { source } => extra::graph::run(src, out, ctx, source),
        FilterParams::Cubism { tile_size, saturation, background, seed } => extra::artistic::cubism(src, out, ctx, *tile_size, *saturation, *background, *seed),
        FilterParams::Slic { cluster_size, compactness, iterations } => extra::artistic::slic(src, out, ctx, *cluster_size, *compactness, *iterations),
        FilterParams::Waterpixels { size, smoothness, regularization, color, seed } => {
            extra::artistic::waterpixels(src, out, ctx, *size, *smoothness, *regularization, *color, *seed)
        }
        FilterParams::Impressionist {
            brush_size,
            size_variation,
            orientation,
            orientation_type,
            density,
            shape,
            paper_relief,
            placement,
            color,
            background,
            background_color,
            seed,
            paper,
            paper_scale,
            paper_map,
            brush_map,
        } => extra::artistic::impressionist(
            src,
            out,
            ctx,
            &extra::artistic::ImpressionistSpec {
                brush_size: *brush_size,
                size_variation: *size_variation,
                orientation: *orientation,
                orientation_type: *orientation_type,
                density: *density,
                shape: *shape,
                paper_relief: *paper_relief,
                placement: *placement,
                color: *color,
                background: *background,
                background_color: *background_color,
                seed: *seed,
                paper: *paper,
                paper_scale: *paper_scale,
                paper_map: paper_map.as_deref(),
                brush_map: brush_map.as_deref(),
            },
        ),
        FilterParams::VanGogh { channel, operator, convolve, filter_length, noise_magnitude, integration_steps, minimum_value, maximum_value, seed } => {
            extra::artistic::van_gogh(
                src,
                out,
                ctx,
                &extra::artistic::LicSpec {
                    channel: *channel,
                    operator: *operator,
                    convolve: *convolve,
                    filter_length: *filter_length,
                    noise_magnitude: *noise_magnitude,
                    steps: *integration_steps,
                    min_value: *minimum_value,
                    max_value: *maximum_value,
                    seed: *seed,
                },
            )
        }
        FilterParams::Weave { ribbon_width, ribbon_spacing, shadow_darkness, shadow_depth, thread_length, thread_density, thread_intensity } => {
            extra::artistic::weave(
                src,
                out,
                ctx,
                &extra::artistic::WeaveSpec {
                    ribbon_width: *ribbon_width,
                    ribbon_spacing: *ribbon_spacing,
                    shadow_darkness: *shadow_darkness,
                    shadow_depth: *shadow_depth,
                    thread_length: *thread_length,
                    thread_density: *thread_density,
                    thread_intensity: *thread_intensity,
                },
            )
        }
        FilterParams::Filmstrip {
            color,
            number_color,
            number_start,
            numbers_top,
            numbers_bottom,
            hole_offset,
            hole_width,
            hole_height,
            hole_spacing,
            fit_height,
            font,
            number_height,
            frames,
        } => extra::decor::filmstrip(
            src,
            out,
            ctx,
            &extra::decor::FilmstripSpec {
                color: *color,
                number_color: *number_color,
                number_start: *number_start,
                numbers_top: *numbers_top,
                numbers_bottom: *numbers_bottom,
                hole_offset: *hole_offset,
                hole_width: *hole_width,
                hole_height: *hole_height,
                hole_spacing: *hole_spacing,
                fit_height: *fit_height,
                font: *font,
                number_height: *number_height,
                frames,
            },
        ),
        FilterParams::FuzzyBorder { color, size, blur, granularity, shadow, shadow_weight, seed } => extra::decor::fuzzy_border(
            src,
            out,
            ctx,
            &extra::decor::FuzzyBorderSpec {
                color: *color,
                size: *size,
                blur: *blur,
                granularity: *granularity,
                shadow: *shadow,
                shadow_weight: *shadow_weight,
                seed: *seed,
            },
        ),
        FilterParams::FilterGallery { effects } => artistic_fx::run(effects, src, out, ctx),
        other => kernel_gen(other, src, out, ctx),
    }
}

/// Kernels of the third filter batch (map, render, decor).
fn kernel_gen(params: &FilterParams, src: &Image, out: Rect, ctx: &Ctx) -> Vec<f32> {
    match params {
        FilterParams::FractalTrace { kind, x1, x2, y1, y2, jx, jy, depth, bailout, abyss } => distort3::fractal_trace(
            src,
            out,
            ctx,
            &distort3::FractalTraceSpec { kind: *kind, x1: *x1, x2: *x2, y1: *y1, y2: *y2, jx: *jx, jy: *jy, depth: *depth, bailout: *bailout, abyss: *abyss },
        ),
        FilterParams::Illusion { division, kind } => distort3::illusion(src, out, ctx, *division, *kind),
        FilterParams::LittlePlanet { pan, tilt, spin, zoom, inverse } => {
            distort3::little_planet(src, out, ctx, &distort3::PlanetSpec { pan: *pan, tilt: *tilt, spin: *spin, zoom: *zoom, inverse: *inverse })
        }
        FilterParams::RecursiveTransform { scale, angle, offset_x, offset_y, first, last, fade, paste_below } => distort3::recursive_transform(
            src,
            out,
            ctx,
            &distort3::RecursiveSpec {
                scale: *scale,
                angle: *angle,
                offset_x: *offset_x,
                offset_y: *offset_y,
                first: *first,
                last: *last,
                fade: *fade,
                paste_below: *paste_below,
            },
        ),
        FilterParams::TileSeamless => distort3::tile_seamless(src, out, ctx),
        FilterParams::MapObject {
            shape,
            rot_x,
            rot_y,
            rot_z,
            light_x,
            light_y,
            light_z,
            ambient,
            diffuse,
            specular,
            highlight,
            radius,
            transparent_background,
        } => distort3::map_object(
            src,
            out,
            ctx,
            &distort3::MapObjectSpec {
                shape: *shape,
                rot_x: *rot_x,
                rot_y: *rot_y,
                rot_z: *rot_z,
                light_x: *light_x,
                light_y: *light_y,
                light_z: *light_z,
                ambient: *ambient,
                diffuse: *diffuse,
                specular: *specular,
                highlight: *highlight,
                radius: *radius,
                transparent_background: *transparent_background,
            },
        ),
        FilterParams::SmallTiles { segments, flip, which, opacity } => distort3::small_tiles(src, out, ctx, *segments, *flip, *which, *opacity),
        FilterParams::FractalExplorer { kind, x1, x2, y1, y2, iterations, cx, cy, colors, red, green, blue, invert_red, invert_green, invert_blue } => {
            gen_fractal::fractal_explorer(
                src,
                out,
                ctx,
                &gen_fractal::ExplorerSpec {
                    kind: *kind,
                    x1: *x1,
                    x2: *x2,
                    y1: *y1,
                    y2: *y2,
                    iterations: *iterations,
                    cx: *cx,
                    cy: *cy,
                    colors: *colors,
                    intensity: [*red, *green, *blue],
                    invert: [*invert_red, *invert_green, *invert_blue],
                },
            )
        }
        FilterParams::IfsFractal { preset, iterations, spot_radius, x, y, scale, angle, color, seed } => gen_fractal::ifs_fractal(
            src,
            out,
            ctx,
            &gen_fractal::IfsSpec {
                preset: *preset,
                iterations: *iterations,
                spot_radius: *spot_radius,
                x: *x,
                y: *y,
                scale: *scale,
                angle: *angle,
                color: *color,
                seed: *seed,
            },
        ),
        FilterParams::SphereDesigner {
            texture,
            color1,
            color2,
            scale,
            turbulence,
            amount,
            exponent,
            light_x,
            light_y,
            light_z,
            ambient,
            bump,
            radius,
            transparent_background,
        } => gen_fractal::sphere_designer(
            src,
            out,
            ctx,
            &gen_fractal::SphereSpec {
                texture: *texture,
                color1: *color1,
                color2: *color2,
                scale: *scale,
                turbulence: *turbulence,
                amount: *amount,
                exponent: *exponent,
                light_x: *light_x,
                light_y: *light_y,
                light_z: *light_z,
                ambient: *ambient,
                bump: *bump,
                radius: *radius,
                transparent_background: *transparent_background,
            },
        ),
        FilterParams::CellNoise { scale, shape, rank, iterations, palettize, seed } => gen_noise::cell_noise(
            src,
            out,
            ctx,
            &gen_noise::CellSpec { scale: *scale, shape: *shape, rank: *rank, iterations: *iterations, palettize: *palettize, seed: *seed },
        ),
        FilterParams::SimplexNoise { scale, iterations, seed } => gen_noise::simplex_noise(src, out, ctx, *scale, *iterations, *seed),
        FilterParams::Lava { seed, size, roughness, gradient } => gen_noise::lava(src, out, ctx, *seed, *size, *roughness, *gradient),
        FilterParams::Circuit { mask_size, seed, no_background } => gen_noise::circuit(src, out, ctx, *mask_size, *seed, *no_background),
        FilterParams::Qbist { seed, antialias, oversample } => gen_noise::qbist(src, out, ctx, *seed, *antialias, *oversample),
        FilterParams::CmlExplorer { hue, saturation, value, k, coupling, zoom, start_offset, seed } => gen_noise::cml_explorer(
            src,
            out,
            ctx,
            &gen_noise::CmlSpec {
                hue: *hue,
                saturation: *saturation,
                value: *value,
                k: *k,
                coupling: *coupling,
                zoom: *zoom,
                start_offset: *start_offset,
                seed: *seed,
            },
        ),
        FilterParams::BayerMatrix { subdivisions, x_scale, y_scale, rotation, reflect, amplitude, offset, exponent, x_offset, y_offset } => {
            gen_pattern::bayer_matrix(
                src,
                out,
                ctx,
                &gen_pattern::BayerSpec {
                    subdivisions: *subdivisions,
                    x_scale: *x_scale,
                    y_scale: *y_scale,
                    rotation: *rotation,
                    reflect: *reflect,
                    amplitude: *amplitude,
                    offset: *offset,
                    exponent: *exponent,
                    x_offset: *x_offset,
                    y_offset: *y_offset,
                },
            )
        }
        FilterParams::Checkerboard { width, height, x_offset, y_offset, color1, color2 } => {
            gen_pattern::checkerboard(src, out, ctx, *width, *height, *x_offset, *y_offset, *color1, *color2)
        }
        FilterParams::DiffractionPatterns {
            red_frequency,
            green_frequency,
            blue_frequency,
            red_contours,
            green_contours,
            blue_contours,
            red_edges,
            green_edges,
            blue_edges,
            brightness,
            scattering,
            polarization,
        } => gen_pattern::diffraction(
            src,
            out,
            ctx,
            &gen_pattern::DiffractionSpec {
                frequencies: [*red_frequency, *green_frequency, *blue_frequency],
                contours: [*red_contours, *green_contours, *blue_contours],
                edges: [*red_edges, *green_edges, *blue_edges],
                brightness: *brightness,
                scattering: *scattering,
                polarization: *polarization,
            },
        ),
        FilterParams::Grid { x_spacing, y_spacing, x_offset, y_offset, line_width, line_height, color } => {
            gen_pattern::grid(src, out, ctx, *x_spacing, *y_spacing, *x_offset, *y_offset, *line_width, *line_height, *color)
        }
        FilterParams::LinearSinusoid {
            x_period,
            y_period,
            x_amplitude,
            y_amplitude,
            x_phase,
            y_phase,
            angle,
            offset,
            exponent,
            x_offset,
            y_offset,
            rotation,
            supersampling,
        } => gen_pattern::linear_sinusoid(
            src,
            out,
            ctx,
            &gen_pattern::SinusoidSpec {
                x_period: *x_period,
                y_period: *y_period,
                x_amplitude: *x_amplitude,
                y_amplitude: *y_amplitude,
                x_phase: *x_phase,
                y_phase: *y_phase,
                angle: *angle,
                offset: *offset,
                exponent: *exponent,
                x_offset: *x_offset,
                y_offset: *y_offset,
                rotation: *rotation,
                supersampling: *supersampling,
            },
        ),
        FilterParams::Maze { cell_width, cell_height, algorithm, tileable, seed, foreground, background } => gen_pattern::maze(
            src,
            out,
            ctx,
            &gen_pattern::MazeSpec {
                cell_width: *cell_width,
                cell_height: *cell_height,
                algorithm: *algorithm,
                tileable: *tileable,
                seed: *seed,
                foreground: *foreground,
                background: *background,
            },
        ),
        FilterParams::Sinus { x_scale, y_scale, complexity, seed, tileable, perturbation, blend, exponent, color1, color2 } => gen_pattern::sinus(
            src,
            out,
            ctx,
            &gen_pattern::SinusSpec {
                x_scale: *x_scale,
                y_scale: *y_scale,
                complexity: *complexity,
                seed: *seed,
                tileable: *tileable,
                perturbation: *perturbation,
                blend: *blend,
                exponent: *exponent,
                color1: *color1,
                color2: *color2,
            },
        ),
        FilterParams::Spiral { kind, x, y, radius, base, balance, rotation, direction, color1, color2 } => gen_pattern::spiral(
            src,
            out,
            ctx,
            &gen_pattern::SpiralSpec {
                kind: *kind,
                x: *x,
                y: *y,
                radius: *radius,
                base: *base,
                balance: *balance,
                rotation: *rotation,
                direction: *direction,
                color1: *color1,
                color2: *color2,
            },
        ),
        FilterParams::Jigsaw { x_tiles, y_tiles, bevel_width, highlight, style } => gen_pattern::jigsaw(
            src,
            out,
            ctx,
            &gen_pattern::JigsawSpec { x_tiles: *x_tiles, y_tiles: *y_tiles, bevel_width: *bevel_width, highlight: *highlight, style: *style },
        ),
        FilterParams::LineNova { lines, sharpness, offset_radius, randomness, seed, color } => gen_pattern::line_nova(
            src,
            out,
            ctx,
            &gen_pattern::LineNovaSpec {
                lines: *lines,
                sharpness: *sharpness,
                offset_radius: *offset_radius,
                randomness: *randomness,
                seed: *seed,
                color: *color,
            },
        ),
        FilterParams::Spirograph { curve, shape, fixed_teeth, moving_teeth, hole, sides, morph, rotation, line_width, color } => gen_pattern::spyro(
            src,
            out,
            ctx,
            &gen_pattern::SpyroSpec {
                curve: *curve,
                shape: *shape,
                fixed_teeth: *fixed_teeth,
                moving_teeth: *moving_teeth,
                hole: *hole,
                sides: *sides,
                morph: *morph,
                rotation: *rotation,
                line_width: *line_width,
                color: *color,
            },
        ),
        FilterParams::Slide { text, number, font_size, color } => decor::slide(src, out, ctx, text, number, *font_size, *color),
        FilterParams::CoffeeStain { stains, darken_only, seed } => decor::coffee_stain(src, out, ctx, *stains, *darken_only, *seed),
        FilterParams::StencilCarve { carve_white, map } => decor::stencil_carve(src, out, ctx, *carve_white, map.as_deref()),
        FilterParams::SemiFlatten { background } => decor::semi_flatten(src, out, ctx, *background),
        other => extra2_kernel(other, src, out, ctx),
    }
}

/// Kernels of the fourth batch.
fn extra2_kernel(params: &FilterParams, src: &Image, out: Rect, ctx: &Ctx) -> Vec<f32> {
    match params {
        FilterParams::VariableBlur { radius, linear_mask, high_quality, invert, source, map } => extra2::variable_blur(
            src,
            out,
            ctx,
            &extra2::VariableBlurSpec {
                radius: *radius,
                linear_mask: *linear_mask,
                high_quality: *high_quality,
                invert: *invert,
                source: *source,
                map: map.as_deref(),
            },
        ),
        FilterParams::ApplyLens { refraction, keep_surroundings, background } => {
            extra2::distort::apply_lens(src, out, ctx, *refraction, *keep_surroundings, *background)
        }
        FilterParams::MosaicTiles {
            tile,
            size,
            height,
            spacing,
            neatness,
            allow_splitting,
            color_variation,
            light_direction,
            antialiasing,
            color_averaging,
            joints_color,
            light_color,
            seed,
        } => extra2::distort::mosaic(
            src,
            out,
            ctx,
            &extra2::distort::MosaicSpec {
                tile: *tile,
                size: *size,
                height: *height,
                spacing: *spacing,
                neatness: *neatness,
                allow_splitting: *allow_splitting,
                color_variation: *color_variation,
                light_direction: *light_direction,
                antialiasing: *antialiasing,
                color_averaging: *color_averaging,
                joints_color: *joints_color,
                light_color: *light_color,
                seed: *seed,
            },
        ),
        FilterParams::Newsprint { model, pattern, period, angle, black_pullout, antialiasing, turbulence, seed } => extra2::distort::newsprint(
            src,
            out,
            ctx,
            &extra2::distort::NewsprintSpec {
                model: *model,
                pattern: *pattern,
                period: *period,
                angle: *angle,
                black_pullout: *black_pullout,
                antialiasing: *antialiasing,
                turbulence: *turbulence,
                seed: *seed,
            },
        ),
        FilterParams::CurveBend { upper, lower, rotation, smoothing, antialias } => extra2::distort::curve_bend(
            src,
            out,
            ctx,
            &extra2::distort::CurveBendSpec { upper: upper.clone(), lower: lower.clone(), rotation: *rotation, smoothing: *smoothing, antialias: *antialias },
        ),
        FilterParams::Bloom { threshold, softness, radius, strength, limit_exposure } => extra2::bloom(
            src,
            out,
            ctx,
            &extra2::BloomSpec { threshold: *threshold, softness: *softness, radius: *radius, strength: *strength, limit_exposure: *limit_exposure },
        ),
        FilterParams::Sobel { horizontal, vertical, keep_sign } => extra2::sobel(src, out, ctx, *horizontal, *vertical, *keep_sign),
        FilterParams::Cartoon { mask_radius, pct_black } => extra2::cartoon(src, out, ctx, *mask_radius, *pct_black),
        FilterParams::Clothify { blur_x, blur_y, azimuth, elevation, depth, seed } => extra2::clothify(
            src,
            out,
            ctx,
            &extra2::ClothifySpec { blur_x: *blur_x, blur_y: *blur_y, azimuth: *azimuth, elevation: *elevation, depth: *depth, seed: *seed },
        ),
        FilterParams::StencilChrome { saturation, lightness, factor, highlight_balance, chrome_balance, map } => extra2::surface::stencil_chrome(
            src,
            out,
            ctx,
            &extra2::surface::ChromeSpec {
                saturation: *saturation,
                lightness: *lightness,
                factor: *factor,
                highlight_balance: *highlight_balance,
                chrome_balance: *chrome_balance,
                environment: map.as_deref(),
            },
        ),
        FilterParams::BumpMap { kind, compensate, invert, tiled, azimuth, elevation, depth, offset_x, offset_y, waterlevel, ambient, source, map } => {
            extra2::surface::bump_map(
                src,
                out,
                ctx,
                &extra2::surface::BumpMapSpec {
                    kind: *kind,
                    compensate: *compensate,
                    invert: *invert,
                    tiled: *tiled,
                    azimuth: *azimuth,
                    elevation: *elevation,
                    depth: *depth,
                    offset_x: *offset_x,
                    offset_y: *offset_y,
                    waterlevel: *waterlevel,
                    ambient: *ambient,
                    source: *source,
                    map: map.as_deref(),
                },
            )
        }
        FilterParams::PanoramaProjection { pan, tilt, spin, zoom, inverse, sampler } => extra2::distort::panorama(
            src,
            out,
            ctx,
            &extra2::distort::PanoramaSpec { pan: *pan, tilt: *tilt, spin: *spin, zoom: *zoom, inverse: *inverse, sampler: *sampler },
        ),
        FilterParams::WarpMap { step_size, iterations, on_edges, dither, rotation_angle, substeps, edge_color, seed, map, magnitude } => {
            extra2::distort::warp_map(
                src,
                out,
                ctx,
                &extra2::distort::WarpMapSpec {
                    step_size: *step_size,
                    iterations: *iterations,
                    on_edges: *on_edges,
                    dither: *dither,
                    rotation_angle: *rotation_angle,
                    substeps: *substeps,
                    edge_color: *edge_color,
                    seed: *seed,
                    map: map.as_deref(),
                    magnitude: magnitude.as_deref(),
                },
            )
        }
        FilterParams::PerlinNoise { alpha, scale, z_offset, iterations } => extra2::noise::perlin_noise(src, out, ctx, *alpha, *scale, *z_offset, *iterations),
        FilterParams::Plasma { turbulence, seed } => extra2::noise::plasma(src, out, ctx, *turbulence, *seed),
        FilterParams::SolidNoise { x_size, y_size, detail, tileable, turbulent, seed } => {
            extra2::noise::solid_noise(src, out, ctx, *x_size, *y_size, *detail, *tileable, *turbulent, *seed)
        }
        _ => src.crop(out),
    }
}

/// Output tile size used by [`apply`].
pub const TILE: i32 = 256;

/// The area a filter writes for a layer whose pixels cover `content`:
/// global filters and renders that ignore the layer's pixels (Fibers) write the reference
/// bounds; others the content grown by the halo (blurs spread into transparent areas).
/// Clipped to the selection's bounds when there is one.
pub fn output_area(params: &FilterParams, content: Rect, bounds: Rect, selection_bounds: Option<Rect>) -> Rect {
    let mut area = match params.halo_for(bounds) {
        Halo::Bounds => bounds,
        Halo::Radius(_) if params.renders_from_nothing() => bounds,
        Halo::Radius(r) => {
            if content.is_empty() {
                Rect::EMPTY
            } else {
                content.inflate(r)
            }
        }
    };
    if let Some(sb) = selection_bounds {
        area = area.intersect(&sb);
    }
    area
}

/// Applies a filter to `area` of `surface`, mixing with the original by the
/// selection coverage (channel 0 of `selection`). Returns the new surface.
pub fn apply(surface: &Surface, params: &FilterParams, area: Rect, bounds: Rect, selection: Option<&Surface>) -> Surface {
    apply_tiled(surface, params, area, bounds, selection, auto_tile(params, bounds), None)
}

/// [`apply`] for a layer in a document: neighbourhood filters repeat the edge pixels of `extent`
/// (the canvas plus any off-canvas pixels the layer has) instead of reading transparency beyond
/// it, and the output is clipped to `extent`, as Photoshop does at the canvas edge.
pub fn apply_in(surface: &Surface, params: &FilterParams, area: Rect, bounds: Rect, selection: Option<&Surface>, extent: Rect) -> Surface {
    apply_tiled(surface, params, area.intersect(&extent), bounds, selection, auto_tile(params, bounds), Some(extent))
}

/// [`apply_in`] that can be cancelled (checked before each tile) and reports progress per tile
/// group. `None` when cancelled, or when a checked filter rejects geometry/allocation;
/// the input surface is never modified.
pub fn apply_in_with(
    surface: &Surface,
    params: &FilterParams,
    area: Rect,
    bounds: Rect,
    selection: Option<&Surface>,
    extent: Rect,
    ctl: &redrob_photo_raster::Interrupt,
) -> Option<Surface> {
    apply_tiled_with(surface, params, area.intersect(&extent), bounds, selection, auto_tile(params, bounds), Some(extent), ctl)
}

/// Results do not depend on the tiling, so wide-halo filters use bigger tiles to keep the
/// re-read margin (and its cost) below ~2× the tile area.
fn auto_tile(params: &FilterParams, bounds: Rect) -> i32 {
    match params.halo_for(bounds) {
        Halo::Radius(r) => TILE.max((2 * r + 63) / 64 * 64).min(2048),
        Halo::Bounds => TILE,
    }
}

/// [`apply`] / [`apply_in`] with an explicit tile size (tests use it to check tile independence).
pub fn apply_tiled(
    surface: &Surface,
    params: &FilterParams,
    area: Rect,
    bounds: Rect,
    selection: Option<&Surface>,
    tile: i32,
    extent: Option<Rect>,
) -> Surface {
    // Preserve the input if a checked filter allocation cannot be completed.
    apply_tiled_with(surface, params, area, bounds, selection, tile, extent, &redrob_photo_raster::Interrupt::NONE).unwrap_or_else(|| surface.clone())
}

/// [`apply_tiled`] with cancellation (checked before each tile, so a cancel takes effect within
/// one tile's work) and progress (after each group of tiles). `None` when cancelled,
/// or when a checked filter rejects geometry or cannot allocate a tile.
#[allow(clippy::too_many_arguments)]
pub fn apply_tiled_with(
    surface: &Surface,
    params: &FilterParams,
    area: Rect,
    bounds: Rect,
    selection: Option<&Surface>,
    tile: i32,
    extent: Option<Rect>,
    ctl: &redrob_photo_raster::Interrupt,
) -> Option<Surface> {
    apply_tiled_with_impl(surface, params, area, bounds, selection, tile, extent, ctl, true)
}

#[allow(clippy::too_many_arguments)]
fn apply_tiled_with_impl(
    surface: &Surface,
    params: &FilterParams,
    area: Rect,
    bounds: Rect,
    selection: Option<&Surface>,
    tile: i32,
    extent: Option<Rect>,
    ctl: &redrob_photo_raster::Interrupt,
    prepare_fast_paths: bool,
) -> Option<Surface> {
    if ctl.cancelled() {
        return None;
    }
    if let FilterParams::UnsharpMask { amount, radius, threshold } = *params
        && (!sharpen::exact::valid_params(amount, radius, threshold)
            || !sharpen::exact::valid_area(area)
            || !sharpen::exact::valid_area(bounds)
            || extent.is_some_and(|e| !sharpen::exact::valid_area(e)))
    {
        return None;
    }
    let mut out = surface.clone();
    if area.is_empty() {
        return Some(out);
    }
    #[cfg(not(target_arch = "wasm32"))]
    if prepare_fast_paths && let Some(result) = sharpen::fft::apply(surface, params, area, selection, extent, ctl) {
        return result;
    }
    if let Some(boxes) = blur::box_widths(params) {
        return apply_box_blur(out, surface, area, extent, selection, &boxes, ctl);
    }
    if let Some(result) = blur::motion_apply::apply(surface, params, area, bounds, selection, extent, tile, ctl) {
        return result;
    }
    // The public tile override must not create a zero-step loop or an
    // unbounded scratch buffer for these checked filters. Automatic tiles are unchanged.
    let tile = if matches!(params, FilterParams::ShapeBlur { .. } | FilterParams::UnsharpMask { .. }) { tile.clamp(1, 2048) } else { tile };
    let fmt = surface.format();
    let ctx = Ctx { bounds, mode: fmt.mode, alpha: fmt.alpha };
    // Prepare the immutable sample transforms once for all output tiles. If the
    // optional table allocation fails, use the existing sampler; only a real
    // cancellation returns None. Source reads keep their upstream semantics.
    let radial = match params {
        FilterParams::RadialBlur { amount, method, quality, center_x, center_y } if prepare_fast_paths => {
            blur::radial::Plan::for_area(bounds, *amount, *method, *quality, (*center_x, *center_y), area, ctl)
        }
        _ => None,
    };
    if ctl.cancelled() {
        return None;
    }
    let halo = params.halo_for(bounds);
    if matches!(params, FilterParams::UnsharpMask { .. }) {
        let Halo::Radius(reach) = halo else { return None };
        let (w, h) = (area.width().min(tile as u32) as usize, area.height().min(tile as u32) as usize);
        let read_samples = w.checked_add(2 * reach as usize)?.checked_mul(h.checked_add(2 * reach as usize)?)?.checked_mul(fmt.channels())?;
        let tile_count = u64::from(area.width()).div_ceil(tile as u64).checked_mul(u64::from(area.height()).div_ceil(tile as u64))?;
        // Reject hostile source-window sizes before Image::read allocates them.
        // Ordinary filter parameters and automatic tiles fit these bounds.
        if read_samples > 512 * 1024 * 1024 || read_samples.checked_mul(std::mem::size_of::<f32>())? > isize::MAX as usize || tile_count > 1_048_576 {
            return None;
        }
    }
    let shared = (halo == Halo::Bounds).then(|| Image::read(surface, bounds.union(&area)));
    // Shape rasterization depends only on params, not on a tile's pixels.
    let shape = match params {
        FilterParams::ShapeBlur { radius, shape } => {
            if ctl.cancelled() {
                return None;
            }
            Some(shape_blur::Prepared::new(*radius, *shape))
        }
        _ => None,
    };
    let mut tiles = Vec::new();
    let mut y = area.y0;
    while y < area.y1 {
        let mut x = area.x0;
        while x < area.x1 {
            tiles.push(Rect::new(x, y, x.saturating_add(tile).min(area.x1), y.saturating_add(tile).min(area.y1)));
            x = x.saturating_add(tile);
        }
        y = y.saturating_add(tile);
    }
    // Tiles finished so far, for progress reported per tile (from any worker thread).
    let finished = std::sync::atomic::AtomicUsize::new(0);
    let total = tiles.len().max(1);
    let run = |t: &Rect| -> (Rect, Vec<f32>) {
        // Cancelled: skip the remaining tiles of the group (the result is discarded).
        if ctl.cancelled() {
            return (*t, Vec::new());
        }
        let tick = || {
            let n = finished.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            // The last few percent are the writes after each group.
            ctl.progress(0.98 * n as f32 / total as f32);
        };
        let owned;
        let src = match (&shared, halo) {
            (Some(s), _) => s,
            (None, Halo::Radius(r)) => {
                owned = match extent {
                    Some(e) => Image::read_clamped(surface, t.inflate(r), e),
                    None => Image::read(surface, t.inflate(r)),
                };
                &owned
            }
            // `shared` is always read for global filters; read the same image if it weren't.
            (None, Halo::Bounds) => {
                owned = Image::read(surface, bounds.union(&area));
                &owned
            }
        };
        let Some(mut data) = (match (&shape, &radial) {
            (Some(shape), _) => shape.checked_run(src, *t, &ctx),
            (None, Some(plan)) => match plan.filter(src, *t, &ctx, ctl) {
                Some(data) => Some(data),
                None if ctl.cancelled() => None,
                None => Some(kernel(params, src, *t, &ctx)),
            },
            (None, None) => match *params {
                FilterParams::UnsharpMask { amount, radius, threshold } => sharpen::exact::run(src, *t, &ctx, amount, radius, threshold, ctl),
                _ => Some(kernel(params, src, *t, &ctx)),
            },
        }) else {
            return (*t, Vec::new());
        };
        if let Some(sel) = selection {
            mix_selection(&mut data, *t, sel, src, fmt.alpha);
        }
        tick();
        (*t, data)
    };
    // Tiles are filtered in groups of about RESULT_BUDGET bytes (at least one per core) and each
    // group is written before the next starts, so a huge layer never holds all its results as
    // floats at once.
    #[cfg(not(target_arch = "wasm32"))]
    let threads = rayon::current_num_threads();
    #[cfg(target_arch = "wasm32")]
    let threads = 1;
    let per_tile = (tile.max(1) as usize).pow(2) * fmt.channels() * std::mem::size_of::<f32>();
    let group = (RESULT_BUDGET / per_tile.max(1)).max(threads).max(1);
    for chunk in tiles.chunks(group) {
        if ctl.cancelled() {
            return None;
        }
        #[cfg(not(target_arch = "wasm32"))]
        let results: Vec<(Rect, Vec<f32>)> = {
            use rayon::prelude::*;
            chunk.par_iter().map(run).collect()
        };
        #[cfg(target_arch = "wasm32")]
        let results: Vec<(Rect, Vec<f32>)> = chunk.iter().map(run).collect();
        if ctl.cancelled() {
            return None;
        }
        // Checked filters can reject malformed geometry or an allocation failure.
        // Do not hand an incomplete tile to Surface::write_region.
        if (shape.is_some() || matches!(params, FilterParams::UnsharpMask { .. }))
            && results.iter().any(|(t, data)| data.len() != t.width() as usize * t.height() as usize * fmt.channels())
        {
            return None;
        }
        for (t, data) in results {
            out.write_region(t, &data);
        }
    }
    out.prune();
    ctl.progress(1.0);
    Some(out)
}

/// [`apply_tiled_with`] for the blurs run as box passes ([`blur::box_widths`]): the whole area at
/// once (in column bands of about [`RESULT_BUDGET`] bytes of floats), so the wide halo of a large
/// radius is read once rather than around every tile. Pixels beyond `extent` repeat its edge, as
/// [`Image::read_clamped`] does for tiles; without one the surface is read (transparent where empty).
fn apply_box_blur(
    mut out: Surface,
    surface: &Surface,
    area: Rect,
    extent: Option<Rect>,
    selection: Option<&Surface>,
    boxes: &[usize],
    ctl: &redrob_photo_raster::Interrupt,
) -> Option<Surface> {
    let fmt = surface.format();
    let n = fmt.channels();
    let reach = i32::try_from(boxes.iter().map(|w| w / 2).sum::<usize>()).unwrap_or(i32::MAX);
    let grown = area.inflate(reach);
    let src = extent.map(|e| grown.intersect(&e)).filter(|r| !r.is_empty()).unwrap_or(grown);
    let read = |r: Rect, buf: &mut Vec<f32>| surface.read_region_into(r, buf);
    // Per output column: the horizontal results for every source row, plus the final samples.
    let column_bytes = (src.height() as usize + area.height() as usize) * n * std::mem::size_of::<f32>();
    let band = i32::try_from(RESULT_BUDGET / column_bytes.max(1)).unwrap_or(i32::MAX).max(blur::STRIP) / blur::STRIP * blur::STRIP;
    let bands: Vec<Rect> = (area.x0..area.x1).step_by(band as usize).map(|x| Rect::new(x, area.y0, x.saturating_add(band).min(area.x1), area.y1)).collect();
    for (i, b) in bands.iter().enumerate() {
        let strips = blur::box_blur(src, &read, *b, n, fmt.alpha, boxes, ctl)?;
        // Each strip lies in one tile column: the columns' tiles are written in parallel.
        let mut columns: Vec<(Surface, Vec<blur::Strip>)> = Vec::new();
        for (t, data) in strips {
            match columns.last_mut() {
                Some((_, v))
                    if v.first().is_some_and(|(r, _)| r.x0.div_euclid(redrob_photo_geom::TILE_SIZE) == t.x0.div_euclid(redrob_photo_geom::TILE_SIZE)) =>
                {
                    v.push((t, data))
                }
                _ => columns.push((Surface::with_default(fmt, &surface.default_pixel()), vec![(t, data)])),
            }
        }
        for (s, v) in &mut columns {
            let r = v.iter().fold(Rect::EMPTY, |acc, (t, _)| if acc.is_empty() { *t } else { acc.union(t) });
            s.put_tiles(out.take_tiles(r));
        }
        let write = |(s, v): &mut (Surface, Vec<blur::Strip>)| {
            for (t, data) in v.iter_mut() {
                if let Some(sel) = selection {
                    mix_selection(data, *t, sel, &Image::read(surface, *t), fmt.alpha);
                }
                s.write_region(*t, data);
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            use rayon::prelude::*;
            columns.par_iter_mut().for_each(write);
        }
        #[cfg(target_arch = "wasm32")]
        columns.iter_mut().for_each(write);
        for (s, _) in columns {
            out.put_tiles(s.tiles().map(|(c, t)| (*c, t.clone())));
        }
        ctl.progress(0.98 * (i + 1) as f32 / bands.len() as f32);
    }
    out.prune();
    ctl.progress(1.0);
    Some(out)
}

/// Mixes filtered `data` for `t` back toward the original pixels in `orig` by the selection's
/// coverage (channel 0 of `sel`). Changing alpha requires premultiplied colour interpolation;
/// otherwise hidden colours leak into feathered edges and alpha-removing filters darken them.
fn mix_selection(data: &mut [f32], t: Rect, sel: &Surface, orig: &Image, alpha: bool) {
    let w = t.width() as usize;
    // The selection over `t` read once, not a lookup and an allocation per pixel (#4533).
    let mask = sel.read_region(t);
    let sc = sel.channels().max(1);
    for (i, px) in data.chunks_exact_mut(orig.ch.max(1)).enumerate() {
        let (x, y) = (t.x0 + (i % w) as i32, t.y0 + (i / w) as i32);
        let k = mask.get(i * sc).copied().unwrap_or(0.0).clamp(0.0, 1.0);
        if k >= 1.0 {
            continue;
        }
        if k <= 0.0 {
            for (c, v) in px.iter_mut().enumerate() {
                *v = orig.get(x, y, c);
            }
            continue;
        }
        if alpha {
            let a = px.len() - 1;
            let original_alpha = orig.get(x, y, a);
            let filtered_alpha = px[a];
            if original_alpha != filtered_alpha {
                let mixed_alpha = original_alpha + (filtered_alpha - original_alpha) * k;
                let original_weight = original_alpha * (1.0 - k);
                let filtered_weight = filtered_alpha * k;
                for (c, v) in px[..a].iter_mut().enumerate() {
                    *v = if mixed_alpha > 0.0 { (orig.get(x, y, c) * original_weight + *v * filtered_weight) / mixed_alpha } else { 0.0 };
                }
                px[a] = mixed_alpha;
                continue;
            }
        }
        for (c, v) in px.iter_mut().enumerate() {
            let o = orig.get(x, y, c);
            *v = o + (*v - o) * k;
        }
    }
}

/// Bytes of float filter results held at once by [`apply_tiled`].
const RESULT_BUDGET: usize = 256 << 20;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_ext;

#[cfg(test)]
mod tests_mosaic;

#[cfg(test)]
mod tests_selection_mix;

/// The current upstream sampler through the identical source/selection/tile pipeline.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn apply_radial_reference(
    surface: &Surface,
    params: &FilterParams,
    area: Rect,
    bounds: Rect,
    selection: Option<&Surface>,
    extent: Rect,
    ctl: &redrob_photo_raster::Interrupt,
) -> Option<Surface> {
    apply_tiled_with_impl(surface, params, area.intersect(&extent), bounds, selection, auto_tile(params, bounds), Some(extent), ctl, false)
}
