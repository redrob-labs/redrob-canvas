//! The brush model: tip shape plus Photoshop-style dynamics sections, all plain serde data.
//!
//! Every section is optional (an `enabled` flag, or a neutral default), so a partial JSON object
//! such as `{"size": 40, "scattering": {"enabled": true, "scatter": {"jitter": 2.0}}}` deserialises
//! onto the defaults. Field names are camelCase in JSON, matching the command parameters.
//!
//! Units: sizes are pixels, angles are degrees, every percentage is a 0..1 fraction (scatter and
//! texture scale may exceed 1, like Photoshop's 1000 % scatter).

use redrob_photo_color::BlendMode;
use serde::{Deserialize, Serialize};

use crate::curve::ResponseCurve;
use crate::mixer::MixerSettings;
use crate::tile::{ColorTile, GrayTile, StoredTile};

/// Largest brush diameter accepted by the rasterizer and brush controls.
pub const MAX_BRUSH_SIZE: f32 = 5000.0;

/// Largest Scatter amount (primary and Dual Brush): 10 = 1000 %, the Brush Settings slider's top.
/// Beyond it a dab could land arbitrarily far away, and an unbounded amount overflowed the dab
/// centre to infinity (#977).
pub const MAX_SCATTER: f32 = 10.0;

/// What drives a dynamic parameter (Photoshop's "Control" pop-ups).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Control {
    #[default]
    Off,
    /// Ramp over `fadeSteps` spacing steps (from full to the minimum).
    Fade,
    PenPressure,
    PenTilt,
    StylusWheel,
    /// Barrel rotation of the stylus.
    Rotation,
    /// Pointer speed: 0 at rest, 1 from [`crate::dynamics::VELOCITY_FULL_PX_PER_MS`] on (or from
    /// [`BrushSettings::speed_max`], smoothed by [`BrushSettings::speed_smoothing`]). Input
    /// without timestamps reads 0 (the dynamic's minimum). Also accepted as `speed`.
    #[serde(alias = "speed")]
    Velocity,
    /// Angle only: the direction of the first stroke segment.
    InitialDirection,
    /// Angle only: the current stroke direction.
    Direction,
    /// The stroke direction as a 0..1 scalar (a full turn, after [`DrawingAngle::offset`]), usable
    /// on every option; with [`DrawingAngle::lock`] the direction of the first segment.
    DrawingAngle,
    /// A fresh random value per dab.
    Random,
    /// The stylus tilt about the X axis alone: -90° reads 0, upright 0.5, +90° reads 1.
    TiltX,
    /// The stylus tilt about the Y axis alone, the same way.
    TiltY,
    /// The compass direction the stylus leans in: -180..180° mapped to 0..1 (an upright or
    /// mouse pointer reads 0.5). Tablet settings may add a left-handed offset (see
    /// [`BrushSettings::tilt_direction_offset`]).
    TiltDirection,
    /// How upright the stylus is: 0 lying flat, 1 vertical (a mouse reads 1).
    TiltElevation,
    /// The distance drawn so far, in pixels, over [`Dynamic::length`] (0 at the start, 1 at the
    /// length; with [`Dynamic::repeat`] it starts over).
    Distance,
    /// The stroke's elapsed time in seconds over [`Dynamic::length`], the same way.
    Time,
    /// Pen pressure that only rises: easing off does not lower the value within a stroke.
    PressureIn,
    /// One random value per stroke (the same for every dab).
    FuzzyStroke,
    /// Depth on the perspective assistant: 1 at the grid's near edge, 0 at its horizon, so dabs
    /// farther away come out smaller. 1 without an active grid ([`BrushSettings::perspective_grid`]).
    Perspective,
}

/// How the sensors of one dynamic combine into its value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SensorCombine {
    #[default]
    Multiply,
    Add,
    Max,
    Min,
    Difference,
}

/// An additional sensor on a dynamic, combined with the primary control by [`Dynamic::combine`].
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Sensor {
    pub control: Control,
    pub fade_steps: u32,
    pub curve: ResponseCurve,
    /// Length of a [`Control::Distance`] (pixels) or [`Control::Time`] (seconds) sensor.
    #[serde(skip_serializing_if = "is_default_length")]
    pub length: f32,
    /// Distance and Time start over past `length` instead of holding 1.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub repeat: bool,
}

impl Sensor {
    pub const OFF: Self = Self { control: Control::Off, fade_steps: 25, curve: ResponseCurve::LINEAR, length: DEFAULT_SENSOR_LENGTH, repeat: false };
    pub const fn new(control: Control) -> Self {
        Self { control, fade_steps: 25, curve: ResponseCurve::LINEAR, length: DEFAULT_SENSOR_LENGTH, repeat: false }
    }
}

/// Default length of the Distance (pixels) and Time (seconds) sensors.
pub const DEFAULT_SENSOR_LENGTH: f32 = 30.0;

fn is_default_length(v: &f32) -> bool {
    *v == DEFAULT_SENSOR_LENGTH
}

impl Default for Sensor {
    fn default() -> Self {
        Self::OFF
    }
}

/// Settings of the drawing-angle sensor ([`Control::DrawingAngle`]).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DrawingAngle {
    /// Degrees added to the stroke direction before it is read.
    pub offset: f32,
    /// Read the direction of the first stroke segment for the whole stroke.
    pub lock: bool,
    /// At a corner sharper than `fan_angle`, put extra dabs in place so the tip turns through the
    /// corner instead of jumping.
    pub fan_corners: bool,
    /// Degrees of turn between two steps from which a corner is fanned.
    pub fan_angle: f32,
}

impl Default for DrawingAngle {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl DrawingAngle {
    pub const DEFAULT: Self = Self { offset: 0.0, lock: false, fan_corners: false, fan_angle: 30.0 };
    pub fn is_default(&self) -> bool {
        *self == Self::DEFAULT
    }
}

/// Extra sensors one dynamic may carry besides its primary control.
pub const MAX_EXTRA_SENSORS: usize = 3;

fn no_extra_sensors(s: &[Sensor; MAX_EXTRA_SENSORS]) -> bool {
    s.iter().all(|x| x.control == Control::Off)
}

fn is_default_combine(c: &SensorCombine) -> bool {
    *c == SensorCombine::Multiply
}

fn is_true(v: &bool) -> bool {
    *v
}

fn is_default_dynamic(d: &Dynamic) -> bool {
    *d == Dynamic::controlled(Control::Off)
}

fn default_gradient_length() -> f32 {
    100.0
}

fn is_default_gradient_length(v: &f32) -> bool {
    *v == default_gradient_length()
}

/// How an image hose (animated brush) picks the cell each dab paints with, per dimension.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HoseSelection {
    /// Cells in order, one per dab, wrapping around.
    #[default]
    Incremental,
    /// By the stroke direction: the full circle is split evenly over the cells.
    Angular,
    /// A random cell per dab (deterministic from the stroke seed).
    Random,
    /// By pointer speed (see [`Control::Velocity`]).
    Velocity,
    /// By pen pressure.
    Pressure,
    /// By the pen's x tilt (-90..90 over the cells).
    TiltX,
    /// By the pen's y tilt.
    TiltY,
}

/// One dimension of an image hose: how many cells it spans and how they are picked.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HoseDimension {
    /// Cells along this dimension (at least 1).
    pub rank: u32,
    pub selection: HoseSelection,
}

impl Default for HoseDimension {
    fn default() -> Self {
        Self { rank: 1, selection: HoseSelection::Incremental }
    }
}

/// An animated brush ("image hose"): a set of sampled cells, one of which paints each dab. The
/// cells are indexed row-major over `dims` (the first dimension varies slowest); a hose whose
/// `dims` multiply to more than `cells.len()` wraps. `tip` holds the first cell so previews,
/// outlines and the tip grid show the brush like any sampled tip.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ImageHose {
    pub cells: Vec<GrayTile>,
    pub dims: Vec<HoseDimension>,
}

impl Default for ImageHose {
    fn default() -> Self {
        Self { cells: Vec::new(), dims: vec![HoseDimension::default()] }
    }
}

impl ImageHose {
    /// Is there anything to animate (at least one valid cell)?
    pub fn is_active(&self) -> bool {
        !self.cells.is_empty() && self.cells.iter().all(GrayTile::is_valid)
    }
}

/// One dynamic parameter: random jitter plus a controller with a floor.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Dynamic {
    /// 0..1 random variation per dab (for scatter: the amount, may exceed 1).
    pub jitter: f32,
    pub control: Control,
    /// Steps for [`Control::Fade`].
    pub fade_steps: u32,
    /// 0..1 floor the control/jitter may reduce the value to (minimum diameter, roundness, depth…).
    pub minimum: f32,
    /// This dynamic's own response to its control (Fade, Pen Pressure, Pen Tilt, Stylus Wheel,
    /// Rotation), applied after the global pen curve. Linear by default. Always written to JSON, so
    /// a `tools.setBrush` patch can set it back to linear. Angle Jitter's controls ignore it.
    pub curve: ResponseCurve,
    /// Further sensors combined with `control` by `combine` (an Off sensor is skipped).
    #[serde(skip_serializing_if = "no_extra_sensors")]
    pub sensors: [Sensor; MAX_EXTRA_SENSORS],
    /// How `control` and `sensors` combine into one 0..1 value.
    #[serde(skip_serializing_if = "is_default_combine")]
    pub combine: SensorCombine,
    /// Settings of this dynamic's drawing-angle sensor(s).
    #[serde(skip_serializing_if = "DrawingAngle::is_default")]
    pub drawing_angle: DrawingAngle,
    /// Length of the primary [`Control::Distance`] (pixels) or [`Control::Time`] (seconds) control.
    #[serde(skip_serializing_if = "is_default_length")]
    pub length: f32,
    /// Distance and Time start over past `length` instead of holding 1.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub repeat: bool,
}

impl Default for Dynamic {
    fn default() -> Self {
        Self::controlled(Control::Off)
    }
}

impl Dynamic {
    pub const fn controlled(control: Control) -> Self {
        Self {
            jitter: 0.0,
            control,
            fade_steps: 25,
            minimum: 0.0,
            curve: ResponseCurve::LINEAR,
            sensors: [Sensor::OFF; MAX_EXTRA_SENSORS],
            combine: SensorCombine::Multiply,
            drawing_angle: DrawingAngle::DEFAULT,
            length: DEFAULT_SENSOR_LENGTH,
            repeat: false,
        }
    }
    pub const fn jitter(jitter: f32) -> Self {
        Self {
            jitter,
            control: Control::Off,
            fade_steps: 25,
            minimum: 0.0,
            curve: ResponseCurve::LINEAR,
            sensors: [Sensor::OFF; MAX_EXTRA_SENSORS],
            combine: SensorCombine::Multiply,
            drawing_angle: DrawingAngle::DEFAULT,
            length: DEFAULT_SENSOR_LENGTH,
            repeat: false,
        }
    }
    pub fn is_active(&self) -> bool {
        self.jitter > 0.0 || self.control != Control::Off || self.has_sensors()
    }
    /// Does any extra sensor read an input?
    pub fn has_sensors(&self) -> bool {
        !no_extra_sensors(&self.sensors)
    }
    /// Does this dynamic read the drawing angle (primary control or an extra sensor)?
    pub fn reads_drawing_angle(&self) -> bool {
        self.control == Control::DrawingAngle || self.sensors.iter().any(|s| s.control == Control::DrawingAngle)
    }
}

/// The brush tip bitmap source.
///
/// A loaded [`TipShape::Stored`] tip serialises as `sampled` (its full bitmap), so tool presets,
/// recorded actions and `brush.get` see the same JSON as for an embedded tip.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TipShape {
    /// Computed round/elliptical tip (hardness, angle, roundness).
    #[default]
    Round,
    /// Sampled tip: grayscale bitmap where 1 = full paint. Scaled so its larger side equals `size`.
    Sampled(GrayTile),
    /// A sampled tip its owner (the preset store) keeps outside the settings and loads on demand.
    Stored(StoredTile),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
enum TipShapeOut<'a> {
    Round,
    Sampled(&'a GrayTile),
    Stored(&'a StoredTile),
}

impl Serialize for TipShape {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            TipShape::Round => TipShapeOut::Round,
            TipShape::Sampled(g) => TipShapeOut::Sampled(g),
            TipShape::Stored(r) => match r.full.as_deref() {
                Some(g) => TipShapeOut::Sampled(g),
                None => TipShapeOut::Stored(r),
            },
        }
        .serialize(s)
    }
}

impl TipShape {
    /// The bitmap a sampled tip paints with (a stored tip's full bitmap, or its preview while it
    /// isn't loaded); `None` for a computed tip.
    pub fn bitmap(&self) -> Option<&GrayTile> {
        match self {
            TipShape::Round => None,
            TipShape::Sampled(g) => Some(g),
            TipShape::Stored(r) => Some(r.bitmap()),
        }
    }
    /// The tip's real pixel size (a stored tip's, not its preview's); `None` for a computed tip.
    pub fn bitmap_size(&self) -> Option<(u32, u32)> {
        match self {
            TipShape::Round => None,
            TipShape::Sampled(g) => Some((g.width, g.height)),
            TipShape::Stored(r) => Some((r.width, r.height)),
        }
    }
}

/// Outline of a computed tip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TipGeometry {
    #[default]
    Circle,
    Square,
    /// A square turned 45 degrees (the parametric brush editor's diamond).
    Diamond,
}

/// How a computed tip fades from its core to its edge.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MaskType {
    /// The measured round tip ([`crate::tip_falloff`]): hardness sets the core and shoulder.
    #[default]
    Default,
    /// A linear fade from the fade radius to the rim, shaped by [`GeneratedTip::soft_curve`].
    Soft,
    /// A Gaussian fall-off from the fade radius.
    Gaussian,
}

/// Extra parameters of a computed ([`TipShape::Round`]) tip beyond size, hardness, angle and
/// roundness: its outline, mask type, per-axis fade, spikes, density, randomness, anti-aliasing,
/// precision and auto spacing. All defaults leave the classic round tip exactly as it was, so
/// presets without this section render unchanged.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GeneratedTip {
    pub shape: TipGeometry,
    pub mask: MaskType,
    /// Soft mask: output at a 0..1 distance from the rim inward (1 at the fade radius, 0 at the rim).
    pub soft_curve: ResponseCurve,
    /// Where the fade begins along the tip's horizontal axis, as a fraction of the radius
    /// (1 = hard). Used by the Soft and Gaussian masks; the Default mask reads `hardness`.
    pub fade_x: f32,
    /// Where the fade begins along the vertical axis. With `lock_fade` it follows `fade_x`.
    pub fade_y: f32,
    pub lock_fade: bool,
    /// Star points (0 or 1 = a plain outline, 2..20 = a star with that many spikes).
    pub spikes: u32,
    /// 0..1 fraction of the tip's pixels that paint (1 = all).
    pub density: f32,
    /// 0..1 per-pixel randomness of the coverage.
    pub randomness: f32,
    /// Anti-alias the tip's edge (off: coverage is 0 or 1, like a pencil).
    pub antialias: bool,
    /// 1 (fast, dab sizes snapped to coarse steps) .. 5 (exact).
    pub precision: u8,
    /// Pick the precision from the dab size: small dabs exact, large ones coarse.
    pub auto_precision: bool,
    /// Spacing grows with the square root of the diameter (`auto_spacing_coeff × √diameter`
    /// pixels) instead of being a fixed fraction of it.
    pub auto_spacing: bool,
    pub auto_spacing_coeff: f32,
}

impl Default for GeneratedTip {
    fn default() -> Self {
        Self {
            shape: TipGeometry::Circle,
            mask: MaskType::Default,
            soft_curve: ResponseCurve::LINEAR,
            fade_x: 1.0,
            fade_y: 1.0,
            lock_fade: true,
            spikes: 0,
            density: 1.0,
            randomness: 0.0,
            antialias: true,
            precision: 5,
            auto_precision: false,
            auto_spacing: false,
            auto_spacing_coeff: 0.8,
        }
    }
}

impl GeneratedTip {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Does rasterising need the generated-tip path (anything but the classic round tip)?
    pub fn is_custom(&self) -> bool {
        self.shape != TipGeometry::Circle
            || self.mask != MaskType::Default
            || self.spikes >= 2
            || self.density < 1.0
            || self.randomness > 0.0
            || !self.antialias
            || self.fade_x < 1.0
            || self.fade_v() < 1.0
    }

    /// Does a dab of `diameter` pixels use a cached, coarser tip mask (precision below 5)?
    pub fn caches_tips(&self, diameter: f32) -> bool {
        self.precision_for(diameter) < 5
    }

    /// The relative step the precision level quantises tip parameters to (1 = 20 %, 2 = 10 %,
    /// 3 = 5 %, 4 = 2 %, 5 = exact: 0).
    pub fn precision_step(&self, diameter: f32) -> f32 {
        match self.precision_for(diameter) {
            1 => 0.2,
            2 => 0.1,
            3 => 0.05,
            4 => 0.02,
            _ => 0.0,
        }
    }

    /// The vertical fade (locked to the horizontal one when `lock_fade`).
    pub fn fade_v(&self) -> f32 {
        if self.lock_fade { self.fade_x } else { self.fade_y }
    }

    /// The precision level for a dab of `diameter` pixels (auto: exact below 20 px, coarser
    /// with every doubling of the size).
    pub fn precision_for(&self, diameter: f32) -> u8 {
        if self.auto_precision {
            let d = if diameter.is_finite() { diameter.max(1.0) } else { 1.0 };
            (5 - ((d / 20.0).log2().floor().clamp(0.0, 4.0) as u8)).clamp(1, 5)
        } else {
            self.precision.clamp(1, 5)
        }
    }

    /// A dab diameter snapped to the precision level's step: 1 = 20 %, 2 = 10 %, 3 = 5 %,
    /// 4 = 2 %, 5 = exact.
    pub fn snap_diameter(&self, diameter: f32) -> f32 {
        let step: f32 = match self.precision_for(diameter) {
            1 => 0.2,
            2 => 0.1,
            3 => 0.05,
            4 => 0.02,
            _ => return diameter,
        };
        if !diameter.is_finite() || diameter <= 0.0 {
            return diameter;
        }
        // Geometric steps: the size moves in `step`-sized relative jumps, never to 0.
        let k = 1.0 + step;
        let n = (diameter.ln() / k.ln()).round();
        k.powf(n).max(0.5)
    }
}

/// Shape Dynamics.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ShapeDynamics {
    pub enabled: bool,
    /// Size jitter / control; `minimum` is Photoshop's Minimum Diameter.
    pub size: Dynamic,
    /// Tilt Scale, 0..2 (0–200 %): with the size control on Pen Tilt, how much a tilted pen
    /// squashes the tip's height (before the tip angle is applied). At full tilt the height is
    /// scaled by `1 − tiltScale / 2`, so 0 leaves the tip alone and 200 % flattens it.
    pub tilt_scale: f32,
    /// Angle jitter (fraction of ±180°) / control.
    pub angle: Dynamic,
    /// Roundness jitter / control; `minimum` is Minimum Roundness.
    pub roundness: Dynamic,
    pub flip_x_jitter: bool,
    pub flip_y_jitter: bool,
    /// Brush Projection: the pen's tilt and barrel rotation are applied to the tip as a
    /// projection (the tip is foreshortened along the tilt direction and turns with the pen).
    pub brush_projection: bool,
}

/// Scattering.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Scattering {
    pub enabled: bool,
    /// `jitter` = scatter amount (1.0 = 100 %: up to one radius of displacement); control scales it.
    pub scatter: Dynamic,
    /// Scatter radially instead of only perpendicular to the stroke.
    pub both_axes: bool,
    /// Separate amounts along and across the stroke (`along`, `across`) instead of `scatter` +
    /// `both_axes`. The control still scales both.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub separate_axes: bool,
    /// Scatter amount along the direction of travel (1.0 = one radius), with `separate_axes`.
    #[serde(skip_serializing_if = "is_zero")]
    pub along: f32,
    /// Scatter amount across the direction of travel, with `separate_axes`.
    #[serde(skip_serializing_if = "is_zero")]
    pub across: f32,
    /// Dabs per spacing step (1..16).
    pub count: u32,
    /// Count jitter / control (reduces the count towards 1).
    pub count_jitter: Dynamic,
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

fn is_one(v: &f32) -> bool {
    *v == 1.0
}

/// Default of [`BrushSettings::speed_max`]: the classic [`crate::dynamics::VELOCITY_FULL_PX_PER_MS`].
pub const DEFAULT_SPEED_MAX: f32 = 2.0;

fn is_default_speed_max(v: &f32) -> bool {
    *v == DEFAULT_SPEED_MAX
}

fn is_pixel_engine(e: &crate::engines::EngineSettings) -> bool {
    matches!(e, crate::engines::EngineSettings::Pixel)
}

impl Default for Scattering {
    fn default() -> Self {
        Self {
            enabled: false,
            scatter: Dynamic::default(),
            both_axes: false,
            separate_axes: false,
            along: 0.0,
            across: 0.0,
            count: 1,
            count_jitter: Dynamic::default(),
        }
    }
}

/// How a grayscale mask (texture or dual tip) combines with the brush coverage.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MaskMode {
    #[default]
    Multiply,
    Subtract,
    Darken,
    Overlay,
    ColorDodge,
    ColorBurn,
    LinearBurn,
    HardMix,
    LinearHeight,
    Height,
}

/// Procedural pattern styles (generated deterministically; no bundled pattern files).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PatternStyle {
    /// Smooth value noise.
    #[default]
    Noise,
    /// Woven canvas.
    Canvas,
    /// Fibrous paper tooth.
    Paper,
    /// Halftone-like dots.
    Dots,
}

/// A texture pattern: a procedural generator or an explicit grayscale tile (tiled infinitely).
/// A loaded [`Pattern::Stored`] tile serialises as `tile`, like [`TipShape::Stored`].
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Pattern {
    Procedural {
        style: PatternStyle,
        size: u32,
        seed: u32,
    },
    Tile(GrayTile),
    /// A tile its owner (the preset store) keeps outside the settings and loads on demand.
    Stored(StoredTile),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
enum PatternOut<'a> {
    Procedural { style: PatternStyle, size: u32, seed: u32 },
    Tile(&'a GrayTile),
    Stored(&'a StoredTile),
}

impl Serialize for Pattern {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Pattern::Procedural { style, size, seed } => PatternOut::Procedural { style: *style, size: *size, seed: *seed },
            Pattern::Tile(g) => PatternOut::Tile(g),
            Pattern::Stored(r) => match r.full.as_deref() {
                Some(g) => PatternOut::Tile(g),
                None => PatternOut::Stored(r),
            },
        }
        .serialize(s)
    }
}

impl Pattern {
    /// The tile a bitmap pattern textures with (a stored tile's full bitmap, or its preview while
    /// it isn't loaded); `None` for a procedural pattern.
    pub fn bitmap(&self) -> Option<&GrayTile> {
        match self {
            Pattern::Procedural { .. } => None,
            Pattern::Tile(g) => Some(g),
            Pattern::Stored(r) => Some(r.bitmap()),
        }
    }
}

impl Default for Pattern {
    fn default() -> Self {
        Pattern::Procedural { style: PatternStyle::Paper, size: 128, seed: 1 }
    }
}

/// Texture.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Texture {
    pub enabled: bool,
    pub pattern: Pattern,
    pub invert: bool,
    /// Pattern scale (1 = 100 %).
    pub scale: f32,
    /// -1..1 (Photoshop's -150..150 mapped to ±1).
    pub brightness: f32,
    /// -1..1: Photoshop's -50..0 mapped to -1..0 and its 0..100 to 0..1.
    pub contrast: f32,
    /// Apply the texture to every dab (enables depth jitter) instead of once to the stroke.
    pub each_tip: bool,
    pub mode: MaskMode,
    /// 0..1.
    pub depth: f32,
    /// Depth jitter / control (per tip only); `minimum` is Minimum Depth.
    pub depth_jitter: Dynamic,
}

impl Texture {
    /// A pattern value (0..1) after Brightness, Contrast and Invert: the value the mode combines
    /// with the brush. Measured against Photoshop (#3340): Brightness shifts the pattern by
    /// `brightness / 255` (±150 moves it 150 levels of 255, not all the way to white or black);
    /// positive Contrast steepens it around mid-grey by `1 / (1 − c/100)` after the shift (100 is
    /// a threshold), negative Contrast flattens it by `1 + c/100` before the shift; Invert is last.
    pub fn level(&self, v: f32) -> f32 {
        let b = if self.brightness.is_finite() { self.brightness.clamp(-1.0, 1.0) * (150.0 / 255.0) } else { 0.0 };
        let c = if self.contrast.is_finite() { self.contrast.clamp(-1.0, 1.0) } else { 0.0 };
        let x = if c >= 0.0 { (v + b - 0.5) / (1.0 - c).max(1e-4) + 0.5 } else { (v - 0.5) * (1.0 + c / 2.0) + 0.5 + b };
        let x = if x.is_finite() { x.clamp(0.0, 1.0) } else { 0.5 };
        if self.invert { 1.0 - x } else { x }
    }
}

impl Default for Texture {
    fn default() -> Self {
        Self {
            enabled: false,
            pattern: Pattern::default(),
            invert: false,
            scale: 1.0,
            brightness: 0.0,
            contrast: 0.0,
            each_tip: false,
            mode: MaskMode::Multiply,
            depth: 1.0,
            depth_jitter: Dynamic::default(),
        }
    }
}

/// Dual Brush: a second tip stroked along the same path; the stroke is masked by it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DualBrush {
    pub enabled: bool,
    pub mode: MaskMode,
    pub tip: TipShape,
    pub size: f32,
    pub hardness: f32,
    pub roundness: f32,
    pub angle: f32,
    pub spacing: f32,
    pub scatter: f32,
    pub both_axes: bool,
    pub count: u32,
    pub flip: bool,
}

impl Default for DualBrush {
    fn default() -> Self {
        Self {
            enabled: false,
            mode: MaskMode::Multiply,
            tip: TipShape::Round,
            size: 20.0,
            hardness: 1.0,
            roundness: 1.0,
            angle: 0.0,
            spacing: 0.25,
            scatter: 0.0,
            both_axes: false,
            count: 1,
            flip: false,
        }
    }
}

/// Color Dynamics.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ColorDynamics {
    pub enabled: bool,
    /// Vary every dab (true) or once per stroke (false).
    pub per_tip: bool,
    /// Foreground/background mix jitter / control.
    pub fg_bg: Dynamic,
    /// 0..1 fraction of ±half the hue wheel.
    pub hue_jitter: f32,
    pub saturation_jitter: f32,
    pub brightness_jitter: f32,
    /// -1 (fully desaturated) .. 1 (fully saturated).
    pub purity: f32,
    /// Where the base colour comes from before the jitters: the foreground (mixed towards the
    /// background by `fg_bg`), the background, a gradient, or a random colour per dab.
    #[serde(skip_serializing_if = "ColorSource::is_default")]
    pub source: ColorSource,
    /// The gradient a [`ColorSource::Gradient`] reads (position 0..1, straight RGBA); empty = the
    /// foreground to the background.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub gradient: Vec<GradientStop>,
    /// What picks the gradient position: the control's value, or (Off) the dab's progress over
    /// `gradient_steps` dabs, repeating.
    #[serde(skip_serializing_if = "is_default_dynamic")]
    pub gradient_dynamic: Dynamic,
    pub gradient_steps: u32,
    /// Sensor-driven hue: the control's 0..1 value turns the hue by `(value − ½) × hue_amount`
    /// of the wheel.
    #[serde(skip_serializing_if = "is_default_dynamic")]
    pub hue: Dynamic,
    pub hue_amount: f32,
    /// Sensor-driven saturation shift of `(value − ½) × 2 × saturation_amount`.
    #[serde(skip_serializing_if = "is_default_dynamic")]
    pub saturation: Dynamic,
    pub saturation_amount: f32,
    /// Sensor-driven value (brightness) shift of `(value − ½) × 2 × value_amount`.
    #[serde(skip_serializing_if = "is_default_dynamic")]
    pub value: Dynamic,
    pub value_amount: f32,
    /// Sensor-driven darkening: the colour is scaled by `1 − darken_amount × value`.
    #[serde(skip_serializing_if = "is_default_dynamic")]
    pub darken: Dynamic,
    pub darken_amount: f32,
    /// Sensor-driven mix between the foreground and background: the control's value is the
    /// background's share (added to `fg_bg`).
    #[serde(skip_serializing_if = "is_default_dynamic")]
    pub mix: Dynamic,
}

impl Default for ColorDynamics {
    fn default() -> Self {
        Self {
            enabled: false,
            per_tip: true,
            fg_bg: Dynamic::default(),
            hue_jitter: 0.0,
            saturation_jitter: 0.0,
            brightness_jitter: 0.0,
            purity: 0.0,
            source: ColorSource::Foreground,
            gradient: Vec::new(),
            gradient_dynamic: Dynamic::default(),
            gradient_steps: 100,
            hue: Dynamic::default(),
            hue_amount: 1.0,
            saturation: Dynamic::default(),
            saturation_amount: 1.0,
            value: Dynamic::default(),
            value_amount: 1.0,
            darken: Dynamic::default(),
            darken_amount: 1.0,
            mix: Dynamic::default(),
        }
    }
}

/// Transfer: per-dab opacity ceiling and flow, plus the Mixer Brush's wetness and mix.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Transfer {
    pub enabled: bool,
    pub opacity: Dynamic,
    pub flow: Dynamic,
    /// Wetness Jitter / control (Mixer Brush only): scales the Wet setting per dab.
    pub wetness: Dynamic,
    /// Mix Jitter / control (Mixer Brush only): scales the Mix setting per dab.
    pub mix: Dynamic,
}

/// The sections a lock can hold (the lock icons in Photoshop's Brush Settings list): a locked
/// section keeps its settings when another brush preset is picked.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SectionLocks {
    pub shape_dynamics: bool,
    pub scattering: bool,
    pub texture: bool,
    pub dual_brush: bool,
    pub color_dynamics: bool,
    pub transfer: bool,
    pub pose: bool,
    pub noise: bool,
    pub wet_edges: bool,
    pub build_up: bool,
    pub smoothing: bool,
    pub protect_texture: bool,
    /// The brush tip itself (bitmap or computed shape, its angle, roundness, flips, generated
    /// parameters, image hose and tip mode) stays when another preset is picked.
    pub tip: bool,
    /// The diameter stays.
    pub size: bool,
    /// Spacing (and its checkbox, auto spacing) stays.
    pub spacing: bool,
    /// Opacity and flow stay.
    pub opacity_flow: bool,
    /// Hardness stays.
    pub hardness: bool,
    /// The blend mode stays.
    pub mode: bool,
}

impl SectionLocks {
    /// Every lock a brush can carry, by its JSON name, for `brush.locks.*` commands and the UI.
    pub const NAMES: [&'static str; 18] = [
        "shapeDynamics",
        "scattering",
        "texture",
        "dualBrush",
        "colorDynamics",
        "transfer",
        "pose",
        "noise",
        "wetEdges",
        "buildUp",
        "smoothing",
        "protectTexture",
        "tip",
        "size",
        "spacing",
        "opacityFlow",
        "hardness",
        "mode",
    ];

    /// The lock named `name` (camelCase, as in JSON), if any.
    pub fn get(&self, name: &str) -> Option<bool> {
        Some(match name {
            "shapeDynamics" => self.shape_dynamics,
            "scattering" => self.scattering,
            "texture" => self.texture,
            "dualBrush" => self.dual_brush,
            "colorDynamics" => self.color_dynamics,
            "transfer" => self.transfer,
            "pose" => self.pose,
            "noise" => self.noise,
            "wetEdges" => self.wet_edges,
            "buildUp" => self.build_up,
            "smoothing" => self.smoothing,
            "protectTexture" => self.protect_texture,
            "tip" => self.tip,
            "size" => self.size,
            "spacing" => self.spacing,
            "opacityFlow" => self.opacity_flow,
            "hardness" => self.hardness,
            "mode" => self.mode,
            _ => return None,
        })
    }

    /// Set the lock named `name`; false for an unknown name.
    pub fn set(&mut self, name: &str, on: bool) -> bool {
        let slot = match name {
            "shapeDynamics" => &mut self.shape_dynamics,
            "scattering" => &mut self.scattering,
            "texture" => &mut self.texture,
            "dualBrush" => &mut self.dual_brush,
            "colorDynamics" => &mut self.color_dynamics,
            "transfer" => &mut self.transfer,
            "pose" => &mut self.pose,
            "noise" => &mut self.noise,
            "wetEdges" => &mut self.wet_edges,
            "buildUp" => &mut self.build_up,
            "smoothing" => &mut self.smoothing,
            "protectTexture" => &mut self.protect_texture,
            "tip" => &mut self.tip,
            "size" => &mut self.size,
            "spacing" => &mut self.spacing,
            "opacityFlow" => &mut self.opacity_flow,
            "hardness" => &mut self.hardness,
            "mode" => &mut self.mode,
            _ => return false,
        };
        *slot = on;
        true
    }

    /// Is any lock on?
    pub fn any(&self) -> bool {
        Self::NAMES.iter().any(|n| self.get(n) == Some(true))
    }
}

/// How a sampled tip's bitmap paints ([`BrushSettings::tip_mode`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TipMode {
    /// The tip is a coverage mask painted with the brush colour (grayscale tips, the classic way).
    #[default]
    Mask,
    /// The tip bitmap is painted with its own colours (an image stamp); its alpha is the coverage.
    ColorStamp,
    /// The tip's alpha is kept, its colours replaced by the brush colour whose lightness follows
    /// the tip's lightness around [`TipAdjust::neutral_point`]: impasto highlights and shadows.
    Lightness,
    /// The tip's lightness indexes the brush's gradient ([`BrushSettings::color_gradient`], else
    /// the Color Dynamics gradient, else foreground to background), so one stamp paints many colours.
    GradientMap,
}

impl TipMode {
    pub fn is_default(&self) -> bool {
        *self == TipMode::Mask
    }
    /// Does this mode paint per-pixel colours from the tip bitmap?
    pub fn is_colored(&self) -> bool {
        !self.is_default()
    }
}

/// Adjustments of the Lightness and Gradient Map tip modes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TipAdjust {
    /// The tip lightness (0..1) that paints the brush colour unchanged; lighter parts brighten
    /// it, darker parts darken it.
    pub neutral_point: f32,
    /// -1..1 shift of the tip lightness before it is applied.
    pub brightness: f32,
    /// -1..1 contrast of the tip lightness around the neutral point.
    pub contrast: f32,
}

impl Default for TipAdjust {
    fn default() -> Self {
        Self { neutral_point: 0.5, brightness: 0.0, contrast: 0.0 }
    }
}

impl TipAdjust {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// A tip lightness after Brightness and Contrast, 0..1.
    pub fn lightness(&self, l: f32) -> f32 {
        let b = if self.brightness.is_finite() { self.brightness.clamp(-1.0, 1.0) } else { 0.0 };
        let c = if self.contrast.is_finite() { self.contrast.clamp(-1.0, 1.0) } else { 0.0 };
        let n = self.neutral_point.clamp(0.0, 1.0);
        let gain = if c >= 0.0 { 1.0 / (1.0 - c).max(1e-3) } else { 1.0 + c };
        let v = (l + b - n) * gain + n;
        if v.is_finite() { v.clamp(0.0, 1.0) } else { n }
    }
}

/// Brush Pose: stylus values used instead of (override) or for missing device data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Pose {
    pub enabled: bool,
    /// Degrees, -90..90 (W3C Pointer Events convention).
    pub tilt_x: f32,
    pub tilt_y: f32,
    /// Degrees 0..360.
    pub rotation: f32,
    /// 0..1.
    pub pressure: f32,
    pub override_tilt: bool,
    pub override_rotation: bool,
    pub override_pressure: bool,
}

impl Default for Pose {
    fn default() -> Self {
        Self {
            enabled: false,
            tilt_x: 0.0,
            tilt_y: 0.0,
            rotation: 0.0,
            pressure: 1.0,
            override_tilt: false,
            override_rotation: false,
            override_pressure: false,
        }
    }
}

/// Stroke smoothing (Photoshop's Smoothing % and its options).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Smoothing {
    /// Switch smoothing off without discarding its amount or mode. Old presets remain enabled.
    pub enabled: bool,
    /// 0..1.
    pub amount: f32,
    /// Lazy-mouse: paint only when the pointer pulls a string of `amount × 100 px` taut.
    pub pulled_string: bool,
    /// Keep catching up with the pointer while it pauses (uses point timestamps).
    pub catch_up: bool,
    /// Finish the stroke at the last pointer position.
    pub catch_up_on_end: bool,
    /// Divide the string length by the view zoom (the command's `zoom` param).
    pub adjust_for_zoom: bool,
    /// Smooth the pen pressure along with the position (the same amount), so a shaky pressure
    /// curve does not bead the stroke.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub smooth_pressure: bool,
    /// Stabilizer: average the last N input points (N between `samples_slow` at rest and
    /// `samples_fast` at full speed) instead of the exponential or pulled-string smoothing.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub stabilizer: bool,
    /// Samples averaged while the pointer moves slowly.
    pub samples_slow: u32,
    /// Samples averaged while the pointer moves fast ([`crate::dynamics::VELOCITY_FULL_PX_PER_MS`]).
    pub samples_fast: u32,
    /// Stabilizer dead zone: the brush does not move while the pointer stays within this many
    /// screen pixels of it.
    pub delay: f32,
    /// Stabilizer: finish the stroke at the last pointer position (draw the remaining line).
    pub finish_line: bool,
    /// Stabilizer: average pressure, tilt and wheel over the same window as the position.
    pub stabilize_sensors: bool,
}

impl Default for Smoothing {
    fn default() -> Self {
        Self {
            enabled: true,
            amount: 0.0,
            pulled_string: false,
            catch_up: true,
            catch_up_on_end: true,
            adjust_for_zoom: true,
            smooth_pressure: false,
            stabilizer: false,
            samples_slow: 20,
            samples_fast: 5,
            delay: 0.0,
            finish_line: true,
            stabilize_sensors: false,
        }
    }
}

/// A complete brush: tip shape, dynamics, and stroke-level options.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BrushSettings {
    /// Diameter in pixels.
    pub size: f32,
    /// 0 (soft) ..1 (hard). Computed tips only.
    pub hardness: f32,
    /// Spacing between dabs as a fraction of the diameter.
    pub spacing: f32,
    /// Photoshop's Spacing checkbox. Off: the pointer's speed sets the spacing (one dab every
    /// [`crate::dynamics::SPEED_SPACING_MS`] of stroke time; without timestamps, by distance).
    pub spacing_enabled: bool,
    /// Sensor-driven spacing: the control (and its curve) scales `spacing` between
    /// `spacing_dynamic.minimum` and 1; jitter varies it per step.
    #[serde(skip_serializing_if = "is_default_dynamic")]
    pub spacing_dynamic: Dynamic,
    /// Spacing from the diameter alone (true, the classic behaviour). Off: an elliptical tip's
    /// spacing shrinks with its roundness, so a flat tip keeps its dabs as close across its short
    /// axis as a round one does.
    #[serde(skip_serializing_if = "is_true")]
    pub isotropic_spacing: bool,
    /// Sensor-driven hardness (a dynamics preset's Hardness output): the control scales
    /// `hardness` between `hardness_dynamic.minimum` and 1 per dab.
    #[serde(skip_serializing_if = "is_default_dynamic")]
    pub hardness_dynamic: Dynamic,
    /// Sensor-driven force (a dynamics preset's Force output): how hard the tip presses, a
    /// per-dab flow factor on top of Transfer.
    #[serde(skip_serializing_if = "is_default_dynamic")]
    pub force_dynamic: Dynamic,
    /// Sensor-driven airbrush rate (a dynamics preset's Rate output): scales `build_up_rate`.
    #[serde(skip_serializing_if = "is_default_dynamic")]
    pub rate_dynamic: Dynamic,
    /// Extra parameters of the computed tip (outline, mask type, fade per axis, spikes, density,
    /// randomness, precision, auto spacing). The default is the classic round tip.
    #[serde(skip_serializing_if = "GeneratedTip::is_default")]
    pub generated: GeneratedTip,
    /// Maximum coverage for the whole stroke.
    pub opacity: f32,
    /// Per-dab coverage.
    pub flow: f32,
    /// Shortcut for Shape Dynamics › Size › Pen Pressure (the options-bar button).
    #[serde(alias = "pressure_size")]
    pub pressure_size: bool,
    /// Shortcut for Transfer › Opacity › Pen Pressure.
    #[serde(alias = "pressure_opacity")]
    pub pressure_opacity: bool,
    /// Straight RGBA colour (display RGB). Commands fill it from the foreground colour.
    pub color: [f32; 4],
    /// The inks `color` was picked as (its sRGB colour, the CMYK values), for a CMYK target: the
    /// stroke paints exactly these, since an sRGB round trip does not give them back (#4344).
    /// Only valid while `color` is that sRGB colour; ignored with Color Dynamics.
    #[serde(skip)]
    pub color_cmyk: Option<([f32; 3], [f32; 4])>,
    /// Background colour for Color Dynamics' foreground/background jitter.
    pub background: [f32; 4],
    /// Colour from gradient: when set, the dab colour follows these sorted stops (`(t, rgba)`)
    /// along the stroke, one full gradient per `color_gradient_length` pixels, repeating as
    /// `color_gradient_repeat` says. Empty: paint `color`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub color_gradient: Vec<(f32, [f32; 4])>,
    /// Stroke length in pixels over which `color_gradient` runs once (the fade length).
    #[serde(default = "default_gradient_length", skip_serializing_if = "is_default_gradient_length")]
    pub color_gradient_length: f32,
    /// How `color_gradient` continues past its length: `none` (holds the last colour),
    /// `sawtooth` (starts over) or `triangular` (runs back).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub color_gradient_repeat: String,
    /// Erase instead of paint.
    pub erase: bool,
    /// With `erase`: restore instead of remove. Erasing only lowers alpha and leaves the colour
    /// in place, so an anti-erase stroke brings those pixels back by raising alpha again.
    /// Nothing happens where nothing was ever painted.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub anti_erase: bool,
    /// Brush blend mode (Normal, Multiply, Dissolve…).
    pub mode: BlendMode,
    // --- Brush Tip Shape ---
    pub tip: TipShape,
    /// Degrees, counter-clockwise.
    pub angle: f32,
    /// 0..1 (1 = circle).
    pub roundness: f32,
    pub flip_x: bool,
    pub flip_y: bool,
    /// Pencil: no anti-aliasing (coverage is 0 or 1).
    pub aliased: bool,
    /// With `aliased`, a square tip: every pixel of the dab's whole-pixel square
    /// ([`crate::grid_square`]), whatever its angle or roundness. The Eraser's Block mode.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub square: bool,
    /// Animated brush (an imported `.gih` image hose): the cell each dab paints with is picked
    /// per dab from these instead of `tip`, which keeps the first cell for previews.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hose: Option<ImageHose>,
    /// How a sampled tip paints: as a mask (default), as a colour image stamp, as a lightness
    /// map over the brush colour, or as a gradient map.
    #[serde(skip_serializing_if = "TipMode::is_default")]
    pub tip_mode: TipMode,
    /// Neutral point, brightness and contrast of the Lightness and Gradient Map tip modes.
    #[serde(skip_serializing_if = "TipAdjust::is_default")]
    pub tip_adjust: TipAdjust,
    /// The colours of a colour tip (a `.gbr`/`.abr` colour bitmap or a defined image stamp), the
    /// same size as `tip`'s bitmap, whose alpha is `tip`. Painted by the Color Stamp mode and read
    /// for lightness by the Lightness and Gradient Map modes; `None` makes those read `tip`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tip_color: Option<ColorTile>,
    /// A predefined tip's own initial scale: the diameter the tip was imported at, as a
    /// multiple of its bitmap size (1 = native). Picking the tip in the chooser sets `size` to
    /// `bitmap × tip_scale` and `angle` to `tip_rotation`.
    #[serde(skip_serializing_if = "is_one")]
    pub tip_scale: f32,
    /// A predefined tip's own initial rotation in degrees (see `tip_scale`).
    #[serde(skip_serializing_if = "is_zero")]
    pub tip_rotation: f32,
    /// With `aliased` (Sharpness), the brush outline snaps to the pixel grid, so the cursor
    /// shows exactly the pixels a dab fills (pixel art).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub align_outline: bool,
    /// The perspective grid (four corners, clockwise from the top left) the Perspective sensor
    /// reads; the engine fills it from the document's active assistant for each stroke.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub perspective_grid: Option<[[f64; 2]; 4]>,
    /// Tablet settings: the pointer speed, in pixels per millisecond, that reads as 1 on the
    /// Velocity sensor.
    #[serde(skip_serializing_if = "is_default_speed_max")]
    pub speed_max: f32,
    /// Tablet settings: 0..1 smoothing of the speed reading between input points (0 = raw).
    #[serde(skip_serializing_if = "is_zero")]
    pub speed_smoothing: f32,
    /// Tablet settings: degrees added to the Tilt Direction reading (left-handed offset).
    #[serde(skip_serializing_if = "is_zero")]
    pub tilt_direction_offset: f32,
    /// The engine this brush paints with and its settings (`{"kind": "pixel"}`, the tip-stamping
    /// default, is left out of JSON). `brush.setEngine` switches it, resetting the engine's own
    /// options; `paint.engineStroke` uses it when the call names no engine.
    #[serde(skip_serializing_if = "is_pixel_engine")]
    pub engine: crate::engines::EngineSettings,
    // --- dynamics sections ---
    pub shape_dynamics: ShapeDynamics,
    pub scattering: Scattering,
    pub texture: Texture,
    pub dual_brush: DualBrush,
    pub color_dynamics: ColorDynamics,
    pub transfer: Transfer,
    pub pose: Pose,
    /// Add grain to the soft parts of the tip.
    pub noise: bool,
    /// Watercolour look: lighter interior, darker rims; overlapping dabs don't build up.
    pub wet_edges: bool,
    /// Airbrush: keep depositing dabs over time while the pointer lingers.
    pub build_up: bool,
    /// Build-up rate in dabs per second.
    pub build_up_rate: f32,
    /// Airbrush "motion only": the timed dabs are deposited only while the pointer moves, so a
    /// held pointer does not build up.
    pub build_up_motion_only: bool,
    pub smoothing: Smoothing,
    /// Keep the current texture (pattern + scale) when switching to another textured preset.
    pub protect_texture: bool,
    /// Sections whose settings stay when another preset is picked (tool state, like Photoshop's locks).
    pub locks: SectionLocks,
    /// Mixer Brush options (Wet, Load, Mix, Flow, Sample All Layers) used by `paint.mixerBrush`.
    pub mixer: MixerSettings,
    /// Random seed for jitters (commands derive it from the stroke when not given), so strokes replay.
    pub seed: u64,
}

impl Default for BrushSettings {
    fn default() -> Self {
        Self {
            size: 20.0,
            hardness: 1.0,
            spacing: 0.1,
            spacing_enabled: true,
            spacing_dynamic: Dynamic::controlled(Control::Off),
            isotropic_spacing: true,
            hardness_dynamic: Dynamic::controlled(Control::Off),
            force_dynamic: Dynamic::controlled(Control::Off),
            rate_dynamic: Dynamic::controlled(Control::Off),
            generated: GeneratedTip::default(),
            opacity: 1.0,
            flow: 1.0,
            pressure_size: true,
            pressure_opacity: false,
            color: [0.0, 0.0, 0.0, 1.0],
            color_cmyk: None,
            background: [1.0, 1.0, 1.0, 1.0],
            color_gradient: Vec::new(),
            color_gradient_length: default_gradient_length(),
            color_gradient_repeat: String::new(),
            erase: false,
            anti_erase: false,
            mode: BlendMode::Normal,
            tip: TipShape::Round,
            angle: 0.0,
            roundness: 1.0,
            flip_x: false,
            flip_y: false,
            aliased: false,
            square: false,
            hose: None,
            tip_mode: TipMode::Mask,
            tip_adjust: TipAdjust::default(),
            tip_color: None,
            tip_scale: 1.0,
            tip_rotation: 0.0,
            align_outline: false,
            perspective_grid: None,
            speed_max: DEFAULT_SPEED_MAX,
            speed_smoothing: 0.0,
            tilt_direction_offset: 0.0,
            engine: crate::engines::EngineSettings::Pixel,
            shape_dynamics: ShapeDynamics::default(),
            scattering: Scattering::default(),
            texture: Texture::default(),
            dual_brush: DualBrush::default(),
            color_dynamics: ColorDynamics::default(),
            transfer: Transfer::default(),
            pose: Pose::default(),
            noise: false,
            wet_edges: false,
            build_up: false,
            build_up_rate: 20.0,
            build_up_motion_only: false,
            smoothing: Smoothing::default(),
            protect_texture: false,
            locks: SectionLocks::default(),
            mixer: MixerSettings::default(),
            seed: 0,
        }
    }
}

impl BrushSettings {
    /// Resize the primary tip and keep an enabled Dual Brush tip at the same relative size.
    /// An explicit Dual Brush size edit should bypass this and retain the requested diameter.
    pub fn resize_with_dual_brush(&mut self, new_size: f32) {
        if self.dual_brush.enabled && self.size.is_finite() && self.size > 0.0 && new_size.is_finite() && new_size > 0.0 {
            self.dual_brush.size = (self.dual_brush.size * (new_size / self.size)).clamp(0.5, MAX_BRUSH_SIZE);
        }
        self.size = new_size;
    }

    /// Copy the settings with the primary and dual diameters and Scatter amounts constrained for
    /// rasterization.
    ///
    /// Engine commands reject out-of-range values; this is a final guard for direct users of the
    /// infallible paint API, which must not turn malformed brush dimensions into giant allocations,
    /// nor a huge Scatter amount into non-finite dab centres (#977).
    pub fn bounded_for_render(&self) -> Self {
        let safe_size = |size: f32| {
            if size.is_finite() { size.clamp(0.5, MAX_BRUSH_SIZE) } else { 0.5 }
        };
        // NaN scatters nothing (as before: it never passed the `> 0` check); infinity is the maximum.
        let safe_scatter = |amount: f32| {
            if amount.is_nan() { 0.0 } else { amount.clamp(0.0, MAX_SCATTER) }
        };
        let mut brush = self.clone();
        brush.size = safe_size(brush.size);
        brush.dual_brush.size = safe_size(brush.dual_brush.size);
        brush.scattering.scatter.jitter = safe_scatter(brush.scattering.scatter.jitter);
        brush.scattering.along = safe_scatter(brush.scattering.along);
        brush.scattering.across = safe_scatter(brush.scattering.across);
        brush.dual_brush.scatter = safe_scatter(brush.dual_brush.scatter);
        brush
    }

    /// This brush (a preset) picked while `current` is the tool's brush: Smoothing is a tool
    /// option, so it stays the tool's (Photoshop), a protected texture stays too, and so does every
    /// section `current` has locked (the locks themselves are tool state and carry over).
    pub fn picked_over(self, current: &BrushSettings) -> Self {
        let mut b = self.with_protected_texture(current);
        b.smoothing = current.smoothing.clone();
        let l = &current.locks;
        if l.shape_dynamics {
            b.shape_dynamics = current.shape_dynamics.clone();
        }
        if l.scattering {
            b.scattering = current.scattering.clone();
        }
        if l.texture {
            b.texture = current.texture.clone();
        }
        if l.dual_brush {
            b.dual_brush = current.dual_brush.clone();
        }
        if l.color_dynamics {
            b.color_dynamics = current.color_dynamics.clone();
        }
        if l.transfer {
            b.transfer = current.transfer.clone();
        }
        if l.pose {
            b.pose = current.pose.clone();
        }
        if l.noise {
            b.noise = current.noise;
        }
        if l.wet_edges {
            b.wet_edges = current.wet_edges;
        }
        if l.build_up {
            b.build_up = current.build_up;
            b.build_up_rate = current.build_up_rate;
            b.build_up_motion_only = current.build_up_motion_only;
        }
        if l.protect_texture {
            b.protect_texture = current.protect_texture;
        }
        if l.tip {
            b.keep_tip_of(current);
        }
        if l.size {
            b.resize_with_dual_brush(current.size);
        }
        if l.spacing {
            b.spacing = current.spacing;
            b.spacing_enabled = current.spacing_enabled;
            b.spacing_dynamic = current.spacing_dynamic;
            b.isotropic_spacing = current.isotropic_spacing;
            b.generated.auto_spacing = current.generated.auto_spacing;
            b.generated.auto_spacing_coeff = current.generated.auto_spacing_coeff;
        }
        if l.opacity_flow {
            b.opacity = current.opacity;
            b.flow = current.flow;
        }
        if l.hardness {
            b.hardness = current.hardness;
            b.hardness_dynamic = current.hardness_dynamic;
        }
        if l.mode {
            b.mode = current.mode;
        }
        b.locks = current.locks.clone();
        b
    }

    /// Take the tip of `other` (its bitmap or computed shape, angle, roundness, flips, generated
    /// parameters, image hose and tip mode), leaving every other option alone: the locked tip
    /// carried across presets.
    pub fn keep_tip_of(&mut self, other: &BrushSettings) {
        self.tip = other.tip.clone();
        self.angle = other.angle;
        self.roundness = other.roundness;
        self.flip_x = other.flip_x;
        self.flip_y = other.flip_y;
        self.aliased = other.aliased;
        self.square = other.square;
        self.generated = other.generated.clone();
        self.hose = other.hose.clone();
        self.tip_mode = other.tip_mode;
        self.tip_adjust = other.tip_adjust;
        self.tip_color = other.tip_color.clone();
        self.tip_scale = other.tip_scale;
        self.tip_rotation = other.tip_rotation;
        self.align_outline = other.align_outline;
    }

    /// Picking a predefined tip in the chooser: `size` becomes the bitmap's larger side times
    /// `tip_scale` and `angle` becomes `tip_rotation` (a computed tip is left alone).
    pub fn apply_tip_defaults(&mut self) {
        if let Some((w, h)) = self.tip.bitmap_size() {
            let side = w.max(h).max(1) as f32;
            let scale = if self.tip_scale.is_finite() && self.tip_scale > 0.0 { self.tip_scale } else { 1.0 };
            self.resize_with_dual_brush((side * scale).clamp(0.5, MAX_BRUSH_SIZE));
            if self.tip_rotation.is_finite() {
                self.angle = self.tip_rotation.rem_euclid(360.0);
            }
        }
    }

    /// Does this brush paint colours from its tip bitmap (a tip mode other than Mask on a
    /// sampled tip)?
    pub fn paints_tip_colors(&self) -> bool {
        self.tip_mode.is_colored() && self.tip.bitmap().is_some() && !self.erase
    }

    /// Apply Protect Texture: if `current` protects its texture, a newly chosen brush with a texture
    /// keeps `current`'s pattern and scale.
    pub fn with_protected_texture(mut self, current: &BrushSettings) -> Self {
        if current.protect_texture && current.texture.enabled && self.texture.enabled {
            self.texture.pattern = current.texture.pattern.clone();
            self.texture.scale = current.texture.scale;
            self.protect_texture = true;
        }
        self
    }
}

/// A named brush preset (session tool state; serde round-trips).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrushPreset {
    pub name: String,
    pub brush: BrushSettings,
    /// Shipped with Photocraft (built-ins can be deleted from a session but are regenerated on reset).
    #[serde(default)]
    pub builtin: bool,
    /// Preset group (folder) in the Brushes panel, e.g. "General" or an imported file's name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub group: String,
    /// Folders inside `group`, outermost first (empty = directly in the group), like the nested
    /// folders of Photoshop's Brushes panel; e.g. an imported `.abr` file's own folders.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub folder: Vec<String>,
    /// What picking the preset brings besides the tip and its sections: the New Brush Preset
    /// dialog's boxes.
    #[serde(default, skip_serializing_if = "PresetCaptures::is_default")]
    pub captures: PresetCaptures,
}

/// The New Brush Preset dialog's boxes, kept with the preset. The default is a preset made
/// without the dialog (a built-in, an import, an older save): picking it brings its size and tool
/// settings, as it always did, and leaves the foreground colour alone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PresetCaptures {
    /// Capture Brush Size in Preset. Off: picking the preset keeps the tool's size.
    pub size: bool,
    /// Include Tool Settings: the options bar's mode, opacity, flow and pressure buttons. Off:
    /// picking the preset keeps the tool's.
    pub tool_settings: bool,
    /// Include Color: picking the preset sets the foreground colour to the brush's `color`.
    pub color: bool,
}

impl Default for PresetCaptures {
    fn default() -> Self {
        Self { size: true, tool_settings: true, color: false }
    }
}

impl PresetCaptures {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

impl BrushPreset {
    /// The tool's brush once this preset is picked while `current` is the tool's brush: the
    /// preset's brush ([`BrushSettings::picked_over`]), with the size and tool settings it didn't
    /// capture left as they were.
    pub fn picked_over(&self, current: &BrushSettings) -> BrushSettings {
        let mut b = self.brush.clone().picked_over(current);
        if !self.captures.size {
            b.resize_with_dual_brush(current.size);
        }
        if !self.captures.tool_settings {
            b.mode = current.mode;
            b.opacity = current.opacity;
            b.flow = current.flow;
            b.pressure_size = current.pressure_size;
            b.pressure_opacity = current.pressure_opacity;
        }
        b
    }
}

/// Where Color Dynamics takes the base colour from ([`ColorDynamics::source`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColorSource {
    #[default]
    Foreground,
    Background,
    /// [`ColorDynamics::gradient`] at the position [`ColorDynamics::gradient_dynamic`] picks.
    Gradient,
    /// A random colour (full hue wheel, full saturation and value) per dab.
    Random,
}

impl ColorSource {
    pub fn is_default(&self) -> bool {
        *self == ColorSource::Foreground
    }
}

/// One stop of a Color Dynamics gradient.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GradientStop {
    /// 0..1 along the gradient.
    pub position: f32,
    /// Straight RGBA.
    pub color: [f32; 4],
}

impl Default for GradientStop {
    fn default() -> Self {
        Self { position: 0.0, color: [0.0, 0.0, 0.0, 1.0] }
    }
}

/// The colour of a gradient at `t` (0..1): linear between the neighbouring stops, flat beyond
/// the ends. An empty gradient runs from `fg` to `bg`.
pub fn gradient_at(stops: &[GradientStop], fg: [f32; 4], bg: [f32; 4], t: f32) -> [f32; 4] {
    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
    let lerp = |a: [f32; 4], b: [f32; 4], f: f32| [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f, a[3] + (b[3] - a[3]) * f];
    if stops.is_empty() {
        return lerp(fg, bg, t);
    }
    let mut sorted: Vec<GradientStop> = stops.iter().filter(|s| s.position.is_finite()).copied().collect();
    sorted.sort_by(|a, b| a.position.total_cmp(&b.position));
    let Some(first) = sorted.first() else { return lerp(fg, bg, t) };
    if t <= first.position {
        return first.color;
    }
    for w in sorted.windows(2) {
        let (a, b) = (w[0], w[1]);
        if t <= b.position {
            let f = if b.position > a.position { (t - a.position) / (b.position - a.position) } else { 1.0 };
            return lerp(a.color, b.color, f);
        }
    }
    sorted.last().map_or(fg, |s| s.color)
}
