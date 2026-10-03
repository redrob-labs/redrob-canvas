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
///
/// NOT `Copy`: `dynamics` is a list, so a stroke carries a variable number of sensor bindings and the
/// struct owns a heap allocation. Everything that reads settings takes `&BrushSettings`, so the lost
/// `Copy` costs no clone on the paint path -- a stroke's settings are read, never duplicated.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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
    /// Smudge rate: when set, the dab does not paint the brush colour but drags the colour already on
    /// the layer. A carried accumulator is blended toward each sampled pixel by this rate and written
    /// back, so colour smears along the stroke (GIMP's smudge). `None` (default) is a normal brush.
    ///
    /// Omitted when absent, so every existing serialised stroke stays byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smudge: Option<f32>,
    /// Clone source offset `(dx, dy)`: when set, each dab copies the pixel at `(x - dx, y - dy)` from
    /// the layer instead of painting the brush colour, so the stroke clones another region (GIMP's
    /// clone tool, aligned mode). With `clone_perspective` the offset point is first mapped through a
    /// 3x3 homography (perspective clone). `None` (default) is a normal brush.
    ///
    /// Omitted when absent, so every existing serialised stroke stays byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clone_offset: Option<(f32, f32)>,
    /// Row-major 3x3 homography applied to the clone source point before sampling (perspective
    /// clone). Ignored unless `clone_offset` is set. Omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clone_perspective: Option<[f32; 9]>,
    /// Heal: like clone (needs `clone_offset`), but the copied patch's texture is transplanted onto
    /// the destination's local colour -- each dab shifts the source so its mean matches the mean of
    /// the pixels it lands on, so a blemish is covered with surrounding colour but source detail.
    /// `false` (default) is a plain clone/brush.
    ///
    /// Omitted when false, so every existing serialised stroke stays byte-identical.
    #[serde(default, skip_serializing_if = "is_false")]
    pub heal: bool,
    /// Convolve brush (GIMP's blur/sharpen): processes the pixels under the dab in place instead of
    /// painting. Positive = sharpen (push each pixel away from its 3x3 neighbourhood mean), negative
    /// = blur (toward it); magnitude 0..=1 is the strength. `None` (default) is a normal brush.
    ///
    /// Omitted when absent, so every existing serialised stroke stays byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub convolve: Option<f32>,
    /// Dodge/Burn brush (GIMP's dodge-burn): lightens (positive) or darkens (negative) the pixels
    /// under the dab in place, by this exposure in -1..=1, scaled by the dab coverage. Applied in a
    /// tonal range selected by `dodge_range`. `None` (default) is a normal brush.
    ///
    /// Omitted when absent, so every existing serialised stroke stays byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dodge_burn: Option<f32>,
    /// Which tones the dodge/burn brush affects: 0 shadows, 1 midtones (default when dodge_burn is
    /// set), 2 highlights. Ignored unless `dodge_burn` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dodge_range: Option<u8>,
    /// Ink nib speed response (GIMP's ink): the faster the pen moves, the thinner the line. This is
    /// the sensitivity in 0..=1 -- 0 ignores speed (a constant nib), 1 lets a fast stroke taper to
    /// nearly nothing. Applied by scaling each point's pressure (which drives the dab diameter) down
    /// as the local speed rises. `None` (default) is a normal brush.
    ///
    /// Omitted when absent, so every existing serialised stroke stays byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ink: Option<f32>,
    /// MyPaint-style surface model (GIMP's MyPaint brush): instead of one clean dab per step, the
    /// stroke scatters several small dabs with jittered radius and position, so it builds a textured,
    /// grainy line rather than a solid one. `None` (default) is a normal brush.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mypaint: Option<MyPaintSurface>,
    /// Krita-style sensor/preset dynamics (B.10): bindings from an input sensor (pressure, speed,
    /// random) to the brush size, each with a response amount. Empty (default) means size follows
    /// raw pressure as before. Applied per point in plan_brush_stroke by remapping each point's
    /// pressure (which drives the dab diameter) through the combined sensor response.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dynamics: Vec<SizeDynamic>,
    /// Bindings that drive the dab's OPACITY (I.1). Separate from `dynamics` because pressure already
    /// drives the diameter: with only a size binding, pressing harder makes a dab both bigger and more
    /// opaque, and the two cannot be asked for independently. These scale the dab's alpha.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub opacity_dynamics: Vec<BrushDynamic>,
    /// Bindings that drive the dab's FLOW (I.1) — how much paint each dab deposits, as distinct from
    /// how opaque the stroke can become. A low flow with full opacity builds up over repeated passes;
    /// a low opacity caps the result however many passes are made.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flow_dynamics: Vec<BrushDynamic>,
    /// Multihand / radial symmetry (Krita's multibrush): the centre the stroke is mirrored and
    /// rotated about. `None` (default) means no radial symmetry (mirror_x / mirror_y still apply).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symmetry_center: Option<(f32, f32)>,
    /// How many rotational copies of the stroke to paint about `symmetry_center`, evenly spaced
    /// around the circle (2 = opposite, 6 = six-fold, etc). 0 or 1 means no rotational copies.
    /// Ignored unless `symmetry_center` is set. Omitted when 0.
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub symmetry_order: u8,
    /// Drawing assistant (Krita's assistants, C.15): a guide that snaps every stroke point before it
    /// is painted. `None` (default) is freehand. Omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant: Option<crate::BrushAssistant>,
    /// Dyna brush (GIMP's dynamic brush, C.16b): a mass-spring model where the dab chases the cursor
    /// through a weight and drag, so the stroke smooths and overshoots like an inked nib. `(mass,
    /// drag)` each in 0..=1; `None` (default) is a rigid brush. Omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dyna: Option<(f32, f32)>,
}

/// One Krita-style binding: how much an input sensor drives one brush channel.
///
/// The same (sensor, amount) pair drives size, opacity and flow — which channel it affects is decided
/// by WHICH list on `BrushSettings` it sits in, not by the binding itself.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrushDynamic {
    pub sensor: DynamicSensor,
    /// -1..=1: how strongly this sensor pushes its channel up (positive) or down (negative).
    pub amount: f32,
}

/// Previous name, from when size was the only channel a binding could drive (B.10). Kept so the FFI
/// and the Qt bridge keep naming the type they were written against.
pub type SizeDynamic = BrushDynamic;

/// Input sensors a dynamic binding can read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DynamicSensor {
    /// Pen pressure (the point's own pressure value).
    Pressure,
    /// Stroke speed (distance from the previous point, normalised against the brush size).
    Speed,
    /// A per-point deterministic pseudo-random value.
    Random,
}

impl BrushDynamic {
    pub fn is_valid(&self) -> bool {
        self.amount.is_finite() && (-1.0..=1.0).contains(&self.amount)
    }
}

/// Previous name of [`DynamicSensor`], kept for the same reason as [`SizeDynamic`].
pub type SizeSensor = DynamicSensor;

/// Parameters of the MyPaint-style scatter (B.9). All in 0..=1 except `dabs_per_step` (1..=8).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MyPaintSurface {
    /// How many jittered sub-dabs each stroke dab becomes (more = denser, grainier).
    pub dabs_per_step: u8,
    /// Fraction of the radius each sub-dab's size may randomly vary by.
    pub radius_jitter: f32,
    /// Fraction of the radius each sub-dab's centre may be randomly offset by.
    pub offset_jitter: f32,
}

impl MyPaintSurface {
    pub fn is_valid(&self) -> bool {
        (1..=8).contains(&self.dabs_per_step)
            && self.radius_jitter.is_finite()
            && (0.0..=1.0).contains(&self.radius_jitter)
            && self.offset_jitter.is_finite()
            && (0.0..=1.0).contains(&self.offset_jitter)
    }
}

#[allow(clippy::trivially_copy_pass_by_ref)] // serde's skip_serializing_if passes a reference
fn is_false(value: &bool) -> bool {
    !*value
}

fn is_zero_u8(value: &u8) -> bool {
    *value == 0
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

/// The deformation applied by the warp / liquify brush.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarpMode {
    /// Push pixels along the stroke direction.
    #[default]
    Move,
    /// Expand pixels away from the brush centre.
    Grow,
    /// Contract pixels toward the brush centre.
    Shrink,
    /// Rotate pixels clockwise about the brush centre.
    SwirlCw,
    /// Rotate pixels counter-clockwise about the brush centre.
    SwirlCcw,
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
    /// Motion blur (GEGL motion-blur-linear): average the pixels along a line of `distance` pixels at
    /// `angle` degrees, so the image smears in that direction.
    MotionBlur {
        angle_degrees: f32,
        distance: u32,
    },
    /// Lens blur (GEGL): average the pixels under a disc of `radius`, giving a round bokeh rather than
    /// the box/Gaussian spread.
    LensBlur {
        radius: u32,
    },
    /// Edge detect (GIMP edge, Sobel): replace each pixel with the magnitude of its luma gradient, so
    /// edges light up on black. `amount` scales the response.
    EdgeDetect {
        amount: f32,
    },
    /// Emboss (GIMP emboss): a directional relief where the gradient along `angle_degrees` becomes
    /// grey +/- shading, so the image looks stamped.
    Emboss {
        angle_degrees: f32,
    },
    /// Laplace (GIMP laplace): the second-derivative edge operator (the 3x3 Laplacian kernel), a
    /// thinner, sharper edge than Sobel.
    Laplace,
    /// Pixelize (GIMP pixelize): replace each `block`x`block` cell with its average colour.
    Pixelize {
        block: u32,
    },
    /// Waves (GIMP waves): a sinusoidal displacement of amplitude `amplitude` and wavelength
    /// `wavelength` radiating from the centre.
    Waves {
        amplitude: f32,
        wavelength: f32,
    },
    /// Ripple (GIMP ripple): shift each row (or column) by a sine of the other axis.
    Ripple {
        amplitude: f32,
        wavelength: f32,
        horizontal: bool,
    },
    /// Whirl-pinch (GIMP whirl-pinch): rotate (`whirl` degrees) and pull/push (`pinch` -1..1) pixels
    /// around the centre within a radius.
    WhirlPinch {
        whirl_degrees: f32,
        pinch: f32,
    },
    /// Lens distortion (GIMP lens-distortion): barrel (positive) or pincushion (negative) warp by
    /// `main_amount`, scaled from the centre.
    LensDistortion {
        main_amount: f32,
    },
    /// RGB noise (GIMP noise-rgb / Krita random noise): add independent random jitter of amplitude
    /// `amount` (0..1 scaled to +/-255) to each channel. `seed` makes it reproducible.
    RgbNoise {
        amount: f32,
        seed: u32,
    },
    /// HSV noise (GIMP noise-hsv): jitter hue/saturation/value instead of the raw channels, so the
    /// noise reads as colour grain rather than per-channel speckle.
    HsvNoise {
        hue: f32,
        saturation: f32,
        value: f32,
        seed: u32,
    },
    /// Hurl (GIMP noise-hurl): with probability `amount`, replace a pixel with a fully random colour.
    Hurl {
        amount: f32,
        seed: u32,
    },
    /// Pick (GIMP noise-pick): with probability `amount`, replace a pixel with a random one of its
    /// eight neighbours — a scattering that keeps the palette.
    Pick {
        amount: f32,
        seed: u32,
    },
    /// Spread (GIMP noise-spread): displace each pixel by a random offset up to `amount` pixels,
    /// jittering positions without changing colours.
    Spread {
        amount: u32,
        seed: u32,
    },
    /// Checkerboard (GIMP checkerboard): fill with a two-colour `size`-pixel checker.
    Checkerboard {
        size: u32,
        color_a: crate::Pixel,
        color_b: crate::Pixel,
    },
    /// Gradient map (GIMP gradient-map): remap each pixel's luma onto the gradient from `low` (dark)
    /// to `high` (light).
    GradientMap {
        low: crate::Pixel,
        high: crate::Pixel,
    },
    /// Plasma (GIMP plasma): fill with smooth fractal clouds built from layered value noise.
    Plasma {
        turbulence: f32,
        seed: u32,
    },
    /// Solid noise (GIMP noise-solid): a greyscale fractal cloud (summed value-noise octaves).
    SolidNoise {
        detail: u32,
        seed: u32,
    },
    /// Cell noise (GIMP/GEGL cell-noise, Worley): distance to the nearest of a scatter of random
    /// feature points, giving an organic cellular pattern. `density` is cells across the image.
    CellNoise {
        density: u32,
        seed: u32,
    },
    /// Colour balance (GIMP color-balance, simplified): add per-channel shifts in -100..100 (red,
    /// green, blue), weighted toward the midtones.
    ColorBalance {
        red: f32,
        green: f32,
        blue: f32,
    },
    /// Colour temperature (GIMP color-temperature): warm (positive) or cool (negative) the image by
    /// scaling red up and blue down (or vice versa), -100..100.
    ColorTemperature {
        amount: f32,
    },
    /// Exposure (GIMP exposure): multiply linear light by 2^stops, a photographic exposure stop.
    Exposure {
        stops: f32,
    },
    /// Hue-chroma (GIMP hue-chroma): rotate hue by degrees and scale chroma, in CIE LCh-ish terms via
    /// HSV (hue degrees, chroma -100..100).
    HueChroma {
        hue_degrees: f32,
        chroma: f32,
    },
    /// Saturation (GIMP saturation / GEGL): scale saturation around grey by `scale` (0 = greyscale,
    /// 1 = unchanged, >1 = more saturated).
    Saturation {
        scale: f32,
    },
    /// Dither (GIMP dither / Floyd-Steinberg): quantise to `levels` per channel with error diffusion,
    /// so banding becomes a stippled gradient.
    Dither {
        levels: u16,
    },
    /// Oilify (GIMP oilify): each pixel becomes the most common colour in its `radius` neighbourhood
    /// (binned), giving a painterly, flattened look.
    Oilify {
        radius: u32,
    },
    /// Cartoon (GIMP cartoon): darken edges onto the image so it reads as inked line art over flat
    /// colour. `amount` controls the darkening strength.
    Cartoon {
        amount: f32,
    },
    /// Soft glow (GIMP softglow): bloom the bright areas — a blurred, brightened copy screened back
    /// over the image.
    SoftGlow {
        radius: u32,
        amount: f32,
    },
    /// Photocopy (GIMP photocopy): a high-contrast black-and-white sketch from local brightness.
    Photocopy {
        amount: f32,
    },
    /// Apply canvas (GIMP apply-canvas): overlay a woven canvas texture (procedural) at `depth`.
    ApplyCanvas {
        depth: f32,
    },
    /// Cubism (GIMP cubism): break the image into scattered square tiles of its local colour, like
    /// cubist facets. `tile` is the tile size.
    Cubism {
        tile: u32,
        seed: u32,
    },
    /// Bump map (GIMP bump-map): shade the image as if lit from `azimuth`/`elevation`, using the
    /// layer's own luma as the height field — raised where it is bright.
    BumpMap {
        azimuth_degrees: f32,
        elevation_degrees: f32,
        depth: f32,
        /// Which layer supplies the height field. `None` (the default, and what every serialised
        /// command before this said) reads the layer's own luma, so an existing document is unchanged.
        /// A map layer is what makes these filters useful: a bump map is a SEPARATE grey image, and
        /// shading a picture by its own brightness lights its content rather than its surface.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        map: Option<crate::NodeId>,
    },
    /// Displace (GIMP displace): shift each pixel by the local luma gradient scaled by `amount`, so
    /// bright-to-dark edges push the image around. The gradient comes from `map` when set.
    Displace {
        amount: f32,
        /// Which layer supplies the height field. `None` (the default, and what every serialised
        /// command before this said) reads the layer's own luma, so an existing document is unchanged.
        /// A map layer is what makes these filters useful: a bump map is a SEPARATE grey image, and
        /// shading a picture by its own brightness lights its content rather than its surface.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        map: Option<crate::NodeId>,
    },
    /// Fractal trace (GIMP fractal-trace): remap coordinates through one Mandelbrot iteration, so the
    /// image is smeared along the fractal's flow. `depth` iterations, `scale` zoom.
    FractalTrace {
        depth: u32,
        scale: f32,
        /// Which layer supplies the height field. `None` (the default, and what every serialised
        /// command before this said) reads the layer's own luma, so an existing document is unchanged.
        /// A map layer is what makes these filters useful: a bump map is a SEPARATE grey image, and
        /// shading a picture by its own brightness lights its content rather than its surface.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        map: Option<crate::NodeId>,
    },
    /// Warp map (GIMP warp / GEGL): iteratively push pixels along the luma gradient `steps` times,
    /// smearing toward edges. The gradient comes from `map` when set.
    WarpMap {
        amount: f32,
        steps: u32,
        /// Which layer supplies the height field. `None` (the default, and what every serialised
        /// command before this said) reads the layer's own luma, so an existing document is unchanged.
        /// A map layer is what makes these filters useful: a bump map is a SEPARATE grey image, and
        /// shading a picture by its own brightness lights its content rather than its surface.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        map: Option<crate::NodeId>,
    },
    /// Halftone (Krita halftone): render the image as a grid of ink dots whose size follows local
    /// darkness, like newsprint. `cell` is the dot grid spacing.
    Halftone {
        cell: u32,
    },
    /// Phong bump map (Krita phong bumpmap): Phong-shaded relief from the luma height field with a
    /// specular highlight.
    PhongBump {
        azimuth_degrees: f32,
        elevation_degrees: f32,
        depth: f32,
        shininess: f32,
    },
    /// Index / palettize (Krita index colours): snap every pixel to the nearest of `colors` evenly
    /// quantised levels per channel — a reduced palette.
    Palettize {
        levels: u16,
    },
    /// Normal map (Krita height-to-normal): convert the luma height field to an RGB tangent-space
    /// normal map (x,y from the gradient, z up).
    NormalMap {
        strength: f32,
    },
    /// Channel mixer (GIMP/Krita channel-mixer): each output channel is a linear combination of the
    /// input R/G/B. `matrix` is row-major [rr, rg, rb, gr, gg, gb, br, bg, bb]; `offset` adds a bias
    /// per output channel (each -1..1, scaled to 0..255).
    ChannelMixer {
        matrix: [f32; 9],
        offset: [f32; 3],
    },
    /// CIE Lab adjustment (G.2 colour management): shift perceptual lightness by `lightness` (-100..
    /// 100 added to L) and scale chroma (a,b) by `chroma` (0..4), done in CIE Lab via the colour
    /// module so the change is perceptually even rather than per-channel.
    LabAdjust {
        lightness: f32,
        chroma: f32,
    },
}

impl Filter {
    /// Whether this filter has a single implementation that works at any document precision
    /// (J.1b).
    ///
    /// The list grows one filter per porting step. A filter NOT on it is written against 8-bit
    /// bytes and is refused at a wider precision rather than run through a narrowing round trip,
    /// so this list is also the honest record of how far the migration has got.
    pub(crate) fn is_precision_native(&self) -> bool {
        PRECISION_NATIVE_FILTERS.contains(&self.name())
    }

    /// [`Self::is_precision_native`] for the integration tests, which live outside this crate and
    /// otherwise could not skip the filters that are expected NOT to refuse.
    pub fn is_precision_native_for_test(&self) -> bool {
        self.is_precision_native()
    }

    /// A stable name for this filter, for messages a user reads.
    ///
    /// Derived from the serde tag rather than written twice: the discriminant name and the wire
    /// name cannot then drift apart, which is the usual way a hand-written table of names starts
    /// lying after a rename.
    pub(crate) fn name(&self) -> &'static str {
        // The tag KEY is whatever `#[serde(tag = "...")]` on this enum says — `kind`, not `type`.
        // Reading the wrong key returns the generic word for EVERY filter, so the whole lookup is
        // dead while looking like a cosmetic message problem. That is what happened here, and
        // `every_filter_variant_resolves_to_its_own_name` is what keeps a future tag rename from
        // doing it again silently.
        const TAG: &str = "kind";
        let tag = match serde_json::to_value(self) {
            Ok(serde_json::Value::String(tag)) => tag,
            Ok(serde_json::Value::Object(map)) => match map.get(TAG) {
                Some(serde_json::Value::String(tag)) => tag.clone(),
                _ => return "filter",
            },
            _ => return "filter",
        };
        FILTER_NAMES
            .iter()
            .find(|known| **known == tag)
            .copied()
            .unwrap_or("filter")
    }
}

/// The filters that have a precision-native implementation (J.1b), by wire tag.
///
/// One list, read by both the predicate and the tests. Grows by one entry per porting step, and is
/// therefore also the honest record of how far the migration has got: a filter absent from here is
/// refused on a deep document rather than quietly flattened.
pub(crate) const PRECISION_NATIVE_FILTERS: &[&str] = &["invert"];

/// Every filter's wire tag, interned so [`Filter::name`] can return `&'static str`.
///
/// A missing entry degrades to the generic word "filter" rather than failing: a name is for a
/// message, and an incomplete table must not be able to turn a working filter into an error.
/// `filter_names_cover_every_variant` keeps the table complete.
pub(crate) const FILTER_NAMES: &[&str] = &[
    "invert",
    "grayscale",
    "brightness_contrast",
    "gaussian_blur",
    "threshold",
    "posterize",
    "levels",
    "hue_saturation",
    "box_blur",
    "sharpen",
    "curves",
    "motion_blur",
    "lens_blur",
    "edge_detect",
    "emboss",
    "laplace",
    "pixelize",
    "waves",
    "ripple",
    "whirl_pinch",
    "lens_distortion",
    "rgb_noise",
    "hsv_noise",
    "hurl",
    "pick",
    "spread",
    "checkerboard",
    "gradient_map",
    "plasma",
    "solid_noise",
    "cell_noise",
    "color_balance",
    "color_temperature",
    "exposure",
    "hue_chroma",
    "saturation",
    "dither",
    "oilify",
    "cartoon",
    "soft_glow",
    "photocopy",
    "apply_canvas",
    "cubism",
    "bump_map",
    "displace",
    "fractal_trace",
    "warp_map",
    "halftone",
    "phong_bump",
    "palettize",
    "normal_map",
    "channel_mixer",
    "lab_adjust",
];

/// Serializable mutations accepted by [`crate::Editor`].
// The spread is real: a brush stroke carries its settings and point list while most variants carry
// an id. Boxing the big variant would not remove that payload, only move it behind a pointer at
// every construction site, and `Command` is the serde wire shape, so the indirection would be
// visible to anything that round-trips one.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    SetMetadata {
        metadata: DocumentMetadata,
    },
    /// Adds a named coverage mask, optionally seeded from the current selection (J.2a).
    ///
    /// `from_selection` is how content gets INTO a channel: an empty one would be addable and
    /// useless, so the two are one command rather than an add followed by a fill that does not
    /// exist yet.
    AddChannel {
        id: crate::ChannelId,
        name: String,
        #[serde(default)]
        from_selection: bool,
    },
    /// Stores a path the caller already has (J.4).
    AddPath {
        id: crate::PathId,
        name: String,
        commands: Vec<crate::PathCommand>,
    },
    RemovePath {
        id: crate::PathId,
    },
    RenamePath {
        id: crate::PathId,
        name: String,
    },
    /// Shows or hides a path's on-canvas editing outline. Not layer visibility — a path
    /// contributes no pixels either way (J.4).
    SetPathVisible {
        id: crate::PathId,
        visible: bool,
    },
    /// Stores the current selection's outline as a path (J.4).
    PathFromSelection {
        name: String,
    },
    /// Replaces or combines the selection with a stored path's interior (J.4).
    SelectionFromPath {
        id: crate::PathId,
        mode: SelectionMode,
    },
    /// Paints along a stored path with the brush (J.4).
    ///
    /// Reuses the brush rather than growing a second line renderer: "stroke this path" means the
    /// path drawn with the tool the user has set up, dynamics and all, which is what makes it
    /// useful instead of a thin hairline nobody wants.
    StrokePath {
        id: crate::PathId,
        color: Pixel,
        size: f32,
        opacity: f32,
        #[serde(default)]
        settings: BrushSettings,
    },
    /// Converts the document to a colour mode, rewriting every raster cel (J.3).
    ///
    /// `palette` is required for indexed and ignored otherwise. `dither` says where the error from
    /// snapping each colour goes; it only applies to indexed.
    ConvertColorMode {
        mode: crate::ColorMode,
        #[serde(default)]
        palette: Option<crate::PaletteChoice>,
        #[serde(default)]
        dither: crate::DitherMode,
    },
    /// Turns quick mask on or off (J.2b).
    ///
    /// On: the selection becomes a paintable channel and the selection itself is cleared — while
    /// the mode is on, a live selection would confine the strokes to the region they are meant to
    /// redraw. Off: the channel replaces the selection and is removed.
    SetQuickMask {
        active: bool,
    },
    RemoveChannel {
        id: crate::ChannelId,
    },
    SetChannelVisible {
        id: crate::ChannelId,
        visible: bool,
    },
    SetChannelOpacity {
        id: crate::ChannelId,
        opacity: f32,
    },
    /// The colour the channel's overlay is painted in. Its ALPHA participates in the overlay
    /// strength alongside the channel's own opacity.
    SetChannelColor {
        id: crate::ChannelId,
        color: Pixel,
    },
    /// `true` paints the masked-out area, `false` the selected area. The same channel with this
    /// flipped is the negative of itself on screen.
    SetChannelShowMasked {
        id: crate::ChannelId,
        show_masked: bool,
    },
    RenameChannel {
        id: crate::ChannelId,
        name: String,
    },
    /// Re-encodes every raster cel to a different sample width and records it on the document
    /// (J.1a).
    ///
    /// Narrowing is allowed and is not an error: a user converting a deep document down to 8-bit
    /// is doing it on purpose, usually to export. It is reported instead — the result carries a
    /// warning naming the loss — because the one unacceptable outcome is losing the depth without
    /// being told.
    SetDocumentPrecision {
        precision: crate::precision::Precision,
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
    /// Free-form polygon selection (lasso / polygon tool). Points are (x, y) in canvas pixels,
    /// implicitly closed.
    SelectPolygon {
        points: Vec<(f32, f32)>,
        mode: SelectionMode,
    },
    /// Magic wand: select pixels within `tolerance` of the colour at (x, y). `contiguous` floods the
    /// connected region; otherwise every matching pixel on the layer.
    SelectByColor {
        x: u32,
        y: u32,
        tolerance: u8,
        contiguous: bool,
        mode: SelectionMode,
    },
    /// Intelligent scissors / magnetic selection: trace an edge-snapping boundary through the
    /// anchors (implicitly closed) and select the enclosed polygon.
    SelectScissors {
        anchors: Vec<(u32, u32)>,
        mode: SelectionMode,
    },
    /// Foreground select: classify every pixel as foreground or background from scribbled samples.
    /// `fg` and `bg` are (x, y) sample points on the active layer.
    SelectForeground {
        fg: Vec<(u32, u32)>,
        bg: Vec<(u32, u32)>,
        mode: SelectionMode,
    },
    /// Align tool: move the named layers so their opaque bounds line up. h/v: 0 none, 1 min, 2
    /// centre, 3 max. `to_canvas` aligns to the canvas, else to the layers' combined bounds.
    AlignLayers {
        ids: Vec<LayerId>,
        h: u8,
        v: u8,
        to_canvas: bool,
    },
    /// Perspective / distort transform of the active layer: map its rect corners (TL, TR, BR, BL) to
    /// these four destination corners through a homography. The non-affine transform.
    PerspectiveActive {
        corners: [(f32, f32); 4],
        sampling: SamplingMode,
    },
    /// Cage transform of the active layer: pixels inside the source cage follow it to the destination
    /// cage via mean-value coordinates. Both cages are the same-length closed polygon.
    CageTransform {
        src_cage: Vec<(f32, f32)>,
        dst_cage: Vec<(f32, f32)>,
        sampling: SamplingMode,
    },
    /// Warp / liquify brush over the active layer: a stroke pushes, grows, shrinks or swirls pixels.
    WarpBrush {
        points: Vec<(f32, f32)>,
        mode: WarpMode,
        radius: f32,
        strength: f32,
        sampling: SamplingMode,
    },
    /// N-point deformation of the active layer: control points move from their source positions to
    /// their destination positions and the layer warps smoothly (thin-plate spline).
    NPointTransform {
        src_pts: Vec<(f32, f32)>,
        dst_pts: Vec<(f32, f32)>,
        sampling: SamplingMode,
    },
    /// 3D transform of the active layer: rotate about its centre (radians about X/Y/Z) and project
    /// through a pinhole camera at `distance` canvas-widths.
    Transform3d {
        rot_x: f32,
        rot_y: f32,
        rot_z: f32,
        distance: f32,
        sampling: SamplingMode,
    },
    /// Enclose-and-fill (Krita): fill the regions inside `rect` that existing opaque pixels close off
    /// from the rectangle border. `alpha_threshold` is the alpha below which a pixel counts as empty.
    EncloseAndFill {
        rect: Rect,
        color: Pixel,
        alpha_threshold: u8,
    },
    /// Smart patch (Krita): content-aware fill of the current selection from nearby pixels.
    SmartPatch {
        search_radius: u32,
    },
    /// Lazybrush (Krita): colour whole regions from a few colour scribbles, stopping at line art.
    /// Each scribble is (x, y, colour).
    Lazybrush {
        scribbles: Vec<(u32, u32, Pixel)>,
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
        /// On the command rather than inside `BrushSettings` because settings are configuration read
        /// by reference on the paint path and a tip is bulk data -- a tip in there would make every
        /// settings read carry pixels it does not look at.
        ///
        /// Carried BY VALUE and bounded, the way vector paths and gradient stops already are here, because
        /// this product has no resource store to reference one from. A 64-square tip is 4 KB and the
        /// decoder caps a tip at 512 square. A resource store would be the better home; that is
        /// architecture rather than translation, so it is not invented here.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tip: Option<crate::BrushTip>,
        /// GIH image pipe (B.11): extra tip frames cycled along the stroke. When non-empty, each dab
        /// uses the next frame in sequence (`tip` first, if present, then these), wrapping around --
        /// so a stroke stamps a repeating series of images rather than one. Bounded like `tip`.
        ///
        /// Omitted when empty, so every existing serialised stroke stays byte-identical.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        pipe: Vec<crate::BrushTip>,
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
    /// Apply a linear operation graph (GEGL-style op chain) to the active layer (G.1).
    ApplyGraph {
        graph: crate::OpGraph,
    },
    /// Bake layer styles (drop shadow / outer glow / bevel) into the active layer (G.3).
    ApplyLayerStyle {
        style: crate::LayerStyle,
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
