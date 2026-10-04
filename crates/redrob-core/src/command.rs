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
        /// Hue shift for the ALL range, −180..180 degrees. Upstream stores −1..1 of a turn; ours
        /// is in degrees and that is kept, so commands saved before the six sectors existed still
        /// mean what they meant.
        hue_degrees: f32,
        /// Saturation adjustment for the ALL range, −100..100. Upstream's own range is −1..1.
        saturation: f32,
        /// Lightness adjustment for the ALL range, −100..100.
        lightness: f32,
        /// Per-sector hue shifts in degrees: red, yellow, green, cyan, blue, magenta.
        ///
        /// Seven ranges exist because upstream's config stores `hue[7]`, `saturation[7]` and
        /// `lightness[7]` -- one ALL entry plus the six hue sectors -- and the operation applies
        /// ALL **together with** whichever sector the pixel falls in. So `range` is the dialog
        /// naming which of the seven the sliders currently edit, exactly as it was for
        /// color-balance, and it is not a field here for the same reason.
        ///
        /// **The ALL and sector contributions combine differently per channel**, which is easy to
        /// get wrong and is taken straight from upstream's three map helpers: hue AVERAGES them
        /// (`(hue[ALL] + hue[range]) / 2`) while saturation and lightness SUM them.
        #[serde(default)]
        hue_sectors: [f32; 6],
        /// Per-sector saturation adjustments, −100..100, in the same sector order.
        #[serde(default)]
        saturation_sectors: [f32; 6],
        /// Per-sector lightness adjustments, −100..100, in the same sector order.
        #[serde(default)]
        lightness_sectors: [f32; 6],
        /// How far a pixel near a sector boundary is also adjusted by the NEIGHBOURING sector,
        /// 0..1. Zero means hard sector edges.
        ///
        /// Upstream halves it before use (`overlap = config->overlap / 2.0`) and then blends the
        /// two sectors' results by how far across the overlap band the pixel sits.
        #[serde(default)]
        overlap: f32,
    },
    BoxBlur {
        radius: u32,
    },
    Sharpen {
        amount: f32,
    },
    /// Rescales each channel so the darkest pixel becomes black and the brightest white (K.1).
    ///
    /// `keep_colors` decides whether the three channels share ONE range or each gets its own.
    /// Sharing preserves hue; stretching independently is a white balance, which is a different
    /// operation that happens to be reachable from the same code. Upstream's `gegl:stretch-contrast`
    /// exposes the same choice and defaults to sharing, and so does this.
    StretchContrast {
        #[serde(default = "crate::command::keep_colors_by_default")]
        keep_colors: bool,
    },
    /// Stretches saturation and value to their full ranges, leaving HUE untouched (K.1).
    ///
    /// Hue is an angle; stretching it would fan a narrow range of hues across the whole colour
    /// wheel. Leaving it alone is also what makes this different from the RGB stretch rather than a
    /// slower spelling of it.
    StretchContrastHsv,
    /// Stretches SATURATION to its full range, leaving hue and value alone (K.1).
    ///
    /// No parameters. REFUSED on a greyscale document, which is upstream's own rule — twelve
    /// chroma filters carry the `!gray` sensitivity guard and this is one of them.
    /// `gegl:alien-map` (K.2): a sinusoidal remap of each channel.
    ///
    /// GEGL itself is NOT vendored here -- only GIMP's own app tree -- so the contract was
    /// recovered from the one upstream artefact that does carry it: the translation catalogues in
    /// `po-plug-ins/`, which preserve the UI strings of the plug-in this operation replaced. They
    /// give the whole parameter set and, importantly, its UNITS:
    ///
    /// - "Number of cycles covering full value range" -> frequency counts FULL sine cycles across
    ///   the 0..1 input range, which is what pins the `2*pi` in the argument.
    /// - "Phase angle, range 0-360" -> phase is in DEGREES, not radians.
    /// - "RGB color model" / "HSL color model" -> two interpretations of the three channels.
    /// - "Modify red channel" and its five siblings -> a per-channel enable, so one channel can be
    ///   remapped while the others pass through.
    ///
    /// Those strings ARE upstream source, so this is derived rather than invented; what is not
    /// available is the exact expression, and the tooltip's own words are therefore the
    /// specification. A test asserts the frequency unit directly: at frequency 1 the output must
    /// complete exactly one cycle as the input sweeps 0..1.
    AlienMap {
        /// Which three channels the parameters address.
        #[serde(default)]
        model: AlienMapModel,
        /// Cycles across the full 0..1 range for channel 1 (red, or hue).
        #[serde(default = "crate::command::unit_frequency")]
        cpn1_frequency: f32,
        /// Phase for channel 1, in DEGREES (0..360) as upstream's blurb states.
        #[serde(default)]
        cpn1_phase: f32,
        /// Whether channel 1 is remapped at all. Default true: an alien-map that changed nothing
        /// unless three toggles were set would be a surprising no-op.
        #[serde(default = "crate::command::enabled")]
        cpn1_enabled: bool,
        /// Channel 2 (green, or saturation).
        #[serde(default = "crate::command::unit_frequency")]
        cpn2_frequency: f32,
        #[serde(default)]
        cpn2_phase: f32,
        #[serde(default = "crate::command::enabled")]
        cpn2_enabled: bool,
        /// Channel 3 (blue, or luminosity).
        #[serde(default = "crate::command::unit_frequency")]
        cpn3_frequency: f32,
        #[serde(default)]
        cpn3_phase: f32,
        #[serde(default = "crate::command::enabled")]
        cpn3_enabled: bool,
    },
    /// `gegl:color-exchange` (K.2): replace one colour with another, within a per-channel
    /// tolerance.
    ///
    /// GEGL is not vendored, so the contract comes from `po-plug-ins/`, which preserves the
    /// replaced plug-in's UI strings: "From Color", "To Color", and "Red threshold" / "Green
    /// threshold" / "Blue threshold".
    ///
    /// The thresholds are three INDEPENDENT per-channel tolerances, which makes the matched region
    /// an axis-aligned BOX in RGB space -- not a sphere. A Euclidean-distance implementation would
    /// be a different operation, and the dialog's "Lock thresholds" checkbox only exists because
    /// the three are separate; a single radius would have no use for it.
    ///
    /// That checkbox is a DIALOG affordance and deliberately absent here: it ties the three
    /// sliders together while the user drags them, which is UI state, not a property of the
    /// operation. A command carrying it would serialise a widget.
    ///
    /// What the strings do NOT settle is whether the swap is flat or proportional -- whether a
    /// near-match is shifted by the full `to - from` delta or by a scaled one that preserves
    /// shading. Flat is implemented, because "Swap one color with another" with three independent
    /// box thresholds describes a region being replaced, and a proportional scheme would need a
    /// direction that no string mentions. Recorded as a choice rather than hidden as a fact.
    ColorExchange {
        /// The colour to look for. Alpha is ignored when matching: the operation exchanges
        /// colours, and a pixel's coverage is not its colour.
        from: crate::Pixel,
        /// What matching pixels become. Its alpha is ignored for the same reason.
        to: crate::Pixel,
        /// Tolerance on red, 0..255. Zero matches the exact value only.
        #[serde(default)]
        red_threshold: u8,
        #[serde(default)]
        green_threshold: u8,
        #[serde(default)]
        blue_threshold: u8,
    },
    /// `gegl:color-rotate` (K.2): map one hue arc onto another.
    ///
    /// GEGL is not vendored; the contract comes from `po-plug-ins/`, where this operation's dialog
    /// strings survive under `plug-ins/color-rotate/`. They give two arcs -- "Original" and
    /// "Rotated", each with "From:" and "To:" -- plus a "Gray Options" block with a "Gray Mode"
    /// of "Treat as this" or "Change to this", a "Gray Threshold", and the grey's own "Hue:" and
    /// "Saturation:". The blurb is "Replace a range of colors with another", which is what makes
    /// this an arc-to-arc mapping rather than a flat hue offset.
    ///
    /// Three things in that dialog are deliberately NOT fields here. "Units"
    /// (Degrees / Radians / Radians-Pi) only changes how the angles are DISPLAYED, so angles are
    /// stored in degrees and the choice belongs to the dialog. "Continuous update" is a preview
    /// affordance. "Area" (Entire Layer / Selection / Context) is our selection, which every
    /// filter already honours.
    ColorRotate {
        /// Start of the source arc, in degrees.
        #[serde(default)]
        source_from: f32,
        /// End of the source arc, in degrees. The arc runs from `source_from` in the increasing
        /// direction and may wrap past 360 -- 300 to 60 is a 120-degree arc through red.
        #[serde(default)]
        source_to: f32,
        /// Start of the destination arc, in degrees.
        #[serde(default)]
        dest_from: f32,
        /// End of the destination arc, in degrees. A destination shorter than the source
        /// compresses the hues into it; a longer one spreads them out.
        #[serde(default)]
        dest_to: f32,
        /// How to treat pixels whose saturation is below `gray_threshold`.
        #[serde(default)]
        gray_mode: GrayMode,
        /// Saturation below which a pixel counts as grey, 0..1.
        #[serde(default)]
        gray_threshold: f32,
        /// The hue given to greys, in degrees.
        #[serde(default)]
        gray_hue: f32,
        /// The saturation given to greys, 0..1.
        #[serde(default)]
        gray_saturation: f32,
    },
    /// `gegl:color-to-alpha` (K.2): turn one colour into transparency, unmixing what is left.
    ///
    /// Derived from REAL vendored source this time, not translation strings: no po entry survives
    /// for this operation, but GIMP ships a custom property GUI for it at
    /// `app/propgui/gimppropgui-color-to-alpha.c`, which names all three properties -- `color`,
    /// `transparency-threshold` ("Pick farthest full-transparency color") and `opacity-threshold`
    /// ("Pick nearest full-opacity color").
    ///
    /// That file also settles the DISTANCE METRIC, which no label could. Its colour-pick callback
    /// computes the threshold as `MAX` over the three per-channel absolute differences -- the
    /// Chebyshev distance, not a Euclidean one and not three independent thresholds as
    /// `color-exchange` has. It must be the operation's own metric: picking a colour has to yield
    /// the threshold that makes exactly that colour fully transparent, which only holds if the
    /// widget and the operation measure the same way.
    ///
    /// It reads `R'G'B' double` -- the prime marks are Babl's notation for gamma-encoded sRGB --
    /// so the distance is on STORED values, not in linear light.
    ColorToAlpha {
        /// The colour to become transparent. Its own alpha is ignored.
        color: crate::Pixel,
        /// Chebyshev distance at or below which a pixel becomes fully transparent, 0..1.
        #[serde(default)]
        transparency_threshold: f32,
        /// Distance at or above which a pixel is left fully opaque, 0..1. Between the two the
        /// alpha ramps, which is what gives a soft edge rather than a cut-out.
        #[serde(default = "crate::command::unit_threshold")]
        opacity_threshold: f32,
    },
    /// `gegl:component-extract` (K.2): render one colour component as a greyscale image.
    ///
    /// The HARDEST derivation in this group so far, and the limits are recorded rather than
    /// papered over. All three sources were tried in order:
    ///
    /// - No `po-plug-ins` entry survives -- no plug-in was replaced.
    /// - No `app/propgui/gimppropgui-component-extract.c` -- it uses the generic widget builder.
    /// - The action entry gives only the operation name and the label "_Extract Component...",
    ///   whose ellipsis confirms it is INTERACTIVE and therefore has parameters, without naming
    ///   one of them.
    ///
    /// So the operation itself is unambiguous -- extract a component, get a mono image -- while
    /// the exhaustive component list is NOT available from vendored source. What IS available is
    /// the colour vocabulary GIMP's own code works in, read off the `babl_format` strings in
    /// `app/`: RGB, HSL, HSV, CIE Lab, CIE LCH(ab), CIE Yuv, CIE xyY, CMYK and Y.
    ///
    /// [`ColorComponent`] therefore covers the subset of that vocabulary THIS codebase can
    /// actually convert, and says so. CMYK, LCH, Yuv and xyY are absent because we have no
    /// conversion for them, not because upstream lacks them -- a bounded, honest subset rather
    /// than an invented enum that would claim coverage we do not have.
    ComponentExtract {
        /// Which component to render.
        #[serde(default)]
        component: ColorComponent,
    },
    /// `gegl:mono-mixer` (K.2): mix the three channels down to one grey, with weights.
    ///
    /// No po entry and no propgui of its own; the action entry's "_Mono Mixer..." confirms it is
    /// interactive. The parameter shape comes from its SIBLING, which IS vendored:
    /// `app/propgui/gimppropgui-channel-mixer.c` names nine gains (`rr-gain` through `bb-gain`)
    /// plus `preserve-luminosity`. channel-mixer is the 3x3 case and mono-mixer the 3x1 one -- the
    /// same family, so three gains and the same flag.
    ///
    /// Note channel-mixer is a DIFFERENT operation we already carry; reading its propgui is what
    /// established that, and it also exposed that our own `ChannelMixer` is missing
    /// `preserve_luminosity` -- filed separately, because a name-based gap count cannot see a
    /// missing parameter.
    MonoMixer {
        /// Weight on red.
        #[serde(default = "crate::command::third")]
        red_gain: f32,
        #[serde(default = "crate::command::third")]
        green_gain: f32,
        #[serde(default = "crate::command::third")]
        blue_gain: f32,
        /// When set, the three gains are normalised to sum to 1 before mixing, so changing the
        /// balance between channels does not also change overall brightness.
        ///
        /// That is the reading the name gives, and it is written down as a reading: no vendored
        /// source states the arithmetic, only that the flag exists and is shared with
        /// channel-mixer.
        #[serde(default)]
        preserve_luminosity: bool,
    },
    /// `gegl:sepia` (K.2): a sepia-toned monochrome.
    ///
    /// **The least derivable filter in this group, and the limits are stated rather than hidden.**
    /// All three vendored sources came up empty: no po entry, no propgui, and an action entry
    /// giving only `gegl:sepia` and the label "_Sepia...". The ellipsis establishes one thing and
    /// one thing only -- it is interactive, so it HAS at least one parameter.
    ///
    /// Two further searches were made and neither produced usable evidence:
    ///
    /// - Krita is also vendored, and a grep for the classical sepia matrix's decimals appeared to
    ///   find all six of them. They were coincidences in a colour-LUT data file and in SVG path
    ///   coordinates. Noise that looked exactly like proof.
    /// - Krita ships G'MIC definition files, including a GIMP-targeted one, which do contain
    ///   `gimp_sepia 0,1,0,0`. But those are invocation strings with no parameter names, the
    ///   trailing `,0,0` is G'MIC's own preview/output convention rather than part of the filter,
    ///   and G'MIC's sepia is a different implementation from GEGL's regardless.
    ///
    /// So `strength` is an INFERENCE from the interactive label plus the shape every comparable
    /// filter in this group has, and the tone itself is OUR choice, built from this crate's own
    /// tested Rec. 709 luminance rather than from a matrix no vendored source carries. Both are
    /// recorded as choices. If upstream's exact tone matters later, it needs GEGL vendored -- it
    /// is not recoverable from what is here.
    Sepia {
        /// How far to carry the image toward full sepia, 0..1. Zero is the original image and one
        /// is fully toned, so the parameter is a blend rather than a gain -- which is the only
        /// reading under which the filter has a sensible neutral.
        #[serde(default = "crate::command::unit_threshold")]
        strength: f32,
    },
    /// `gimp:colorize` (K.2): replace every hue with one, keeping the tonal structure.
    ///
    /// **The only filter in this group whose exact arithmetic is vendored.** It is a `gimp:`
    /// operation, not a `gegl:` one, so GIMP implements it itself and the file is right there:
    /// `app/operations/gimpoperationcolorize.c`. No reconstruction from labels, no inference, no
    /// choices of ours -- the properties, their ranges, their defaults and the per-pixel
    /// arithmetic are all read off the source.
    ///
    /// Two things that source says about ITSELF are reproduced deliberately, and both are quoted
    /// in the implementation so a later reader does not "fix" them:
    ///
    /// 1. GIMP's luminance weights are NOT Rec. 709. They are
    ///    `(0.22248840, 0.71690369, 0.06060791)` from `libgimpcolor/gimpcolor-private.h`, against
    ///    Rec. 709's `(0.2126, 0.7152, 0.0722)` used everywhere else in this crate.
    /// 2. Upstream computes luminance on LINEAR input and then writes a NON-LINEAR result into a
    ///    buffer it declares linear. Its own comment calls this out and keeps it anyway.
    Colorize {
        /// The hue every pixel takes, 0..1 as a fraction of a turn. Upstream's default is 0.5.
        #[serde(default = "crate::command::half")]
        hue: f32,
        /// Saturation, 0..1. Upstream's default is 0.5.
        #[serde(default = "crate::command::half")]
        saturation: f32,
        /// Lightness shift, **-1..1**, default 0 — a signed range, unlike the two above.
        ///
        /// Positive values lerp the luminance toward white and negative ones scale it toward
        /// black, which are two different operations rather than one signed one; see the
        /// implementation.
        #[serde(default)]
        lightness: f32,
    },
    /// `gegl:median-blur` (K.3): replace each pixel with the median of its neighbourhood.
    ///
    /// All three vendored sources are empty for this one -- no po entry, no propgui, no `gimp:`
    /// implementation -- so only the action entry remains, and its "_Median Blur..." establishes
    /// one fact: it is interactive, hence has at least one parameter.
    ///
    /// Two parameters are nonetheless DERIVABLE rather than invented:
    ///
    /// - `radius`, because a neighbourhood filter cannot exist without a size, and the ellipsis
    ///   proves at least one parameter exists to be it.
    /// - `edge_policy`, because K.0 counted upstream's three abyss policies out of source, and
    ///   `app/gegl/gimp-gegl-apply-operation.c` shows GIMP passing `"abyss-policy"` to its blur
    ///   wrappers -- so a blur taking an edge policy is upstream-attested even though this
    ///   operation's own property list is not readable.
    ///
    /// What is NOT recoverable, and is therefore absent rather than guessed: GEGL's generalisation
    /// of the median to an arbitrary `percentile`, and its choice of neighbourhood SHAPE (square,
    /// circle, diamond). Both change the output visibly, so inventing them would be inventing the
    /// filter. This implementation is the square-neighbourhood median, which is what the operation
    /// is called.
    MedianBlur {
        /// Half-width of the square neighbourhood.
        ///
        /// Zero is REFUSED with `InvalidFilterParameter`, like every other radius filter in this
        /// crate -- `validate_radius` is shared. That is a consistency decision rather than a
        /// claim about upstream: a single-pixel neighbourhood is arguably a no-op, but making this
        /// one filter accept what `BoxBlur` and the rest reject is a surprise a user would hit
        /// rather than a kindness. My first draft documented it as a no-op without checking the
        /// shared validator, and the test caught the contradiction.
        radius: u32,
        /// How samples outside the canvas are resolved. Defaults to the policy our own box blur
        /// already used by hand before K.0 gave it a name.
        #[serde(default)]
        edge_policy: crate::neighbourhood::EdgePolicy,
    },
    /// `gegl:mean-curvature-blur` (K.3): smooth by moving each level set along its own curvature.
    ///
    /// All four vendored sources are empty — no po entry, no propgui, no `gimp:` implementation,
    /// no config object — so the action entry is all there is, and its "Mean C_urvature Blur..."
    /// establishes only that it is interactive and therefore has at least one parameter.
    ///
    /// **Unlike sepia, though, the NAME here names the mathematics.** Mean curvature motion is a
    /// defined PDE, not a look someone chose: treating the image as a height field, the flow
    /// `I_t = kappa * |grad I|` expands to
    ///
    /// ```text
    ///         I_xx * I_y^2  -  2 * I_x * I_y * I_xy  +  I_yy * I_x^2
    /// I_t  =  -------------------------------------------------------
    ///                        I_x^2 + I_y^2
    /// ```
    ///
    /// So the arithmetic is derived from the operation's own name rather than invented. That is a
    /// stronger position than sepia's and worth distinguishing: there the name named an
    /// *appearance* and the tone had to be chosen; here it names an equation.
    ///
    /// What is NOT recoverable is the step size GEGL picks. `iterations` is the one parameter an
    /// iterative PDE smoother must expose, and the ellipsis proves a parameter exists to be it.
    MeanCurvatureBlur {
        /// How many times to apply the flow. Not a radius: curvature motion shortens level-set
        /// curves rather than averaging a neighbourhood, so repeated passes smooth progressively
        /// instead of widening a window.
        iterations: u32,
        /// How samples outside the canvas are resolved.
        #[serde(default)]
        edge_policy: crate::neighbourhood::EdgePolicy,
    },
    /// `gegl:focus-blur` (K.3): blur that increases with distance from a focus region.
    ///
    /// The first filter in K.3 with a readable property list. `app/propgui/gimppropgui-focus-blur.c`
    /// is a custom GUI for the on-canvas handles, and it names and NORMALISES the geometry:
    ///
    /// - `x`, `y` are fractions of the canvas (the GUI multiplies by `area->width`/`height`),
    ///   not pixels.
    /// - `radius` is a fraction of the WIDTH, and the region's half-extent is
    ///   `radius * area->width / 2.0` -- so the property is a diameter in width-fractions.
    /// - `rotation` is in DEGREES (the GUI converts with `/ 180.0 * G_PI`).
    /// - `aspect-ratio`, `focus` and `midpoint` are unitless.
    ///
    /// The `shape` values come from `GimpLimitType` in the display enums -- circle, square,
    /// diamond, horizontal, vertical -- so the five shapes are read from source rather than
    /// guessed.
    ///
    /// **What is NOT readable: the blur's own strength.** That custom GUI wires only the geometry
    /// block (it brackets the generic widgets between `shape` and `high-quality`), so the
    /// remaining properties go through the generic builder and are never named in any vendored
    /// file. A blur must have an amount, so `blur_radius` exists here under a name of our
    /// choosing; `high-quality` is omitted entirely, being a speed/quality toggle rather than a
    /// property of the result.
    FocusBlur {
        /// Which distance metric bounds the focus region.
        #[serde(default)]
        shape: FocusShape,
        /// Centre of the focus region, as a fraction of the canvas width.
        #[serde(default = "crate::command::half")]
        x: f32,
        /// Centre as a fraction of the canvas height.
        #[serde(default = "crate::command::half")]
        y: f32,
        /// Region diameter as a fraction of the canvas WIDTH, as upstream's GUI stores it.
        #[serde(default = "crate::command::half")]
        radius: f32,
        /// Height-to-width ratio of the region. 1.0 is round.
        #[serde(default = "crate::command::unit_threshold")]
        aspect_ratio: f32,
        /// Region rotation in DEGREES.
        #[serde(default)]
        rotation: f32,
        /// Fraction of the region that stays completely sharp, 0..1.
        #[serde(default)]
        focus: f32,
        /// Where the half-blur point sits within the falloff band, 0..1. Biases the curve toward
        /// the sharp end or the blurred end without moving either limit.
        #[serde(default = "crate::command::half")]
        midpoint: f32,
        /// Blur radius applied at full strength, in pixels.
        ///
        /// Named by us: see the type's note -- upstream's custom GUI brackets the geometry and
        /// leaves this to the generic builder, so no vendored file names it.
        blur_radius: u32,
        /// How samples outside the canvas are resolved.
        #[serde(default)]
        edge_policy: crate::neighbourhood::EdgePolicy,
    },
    /// `gegl:variable-blur` (K.3): blur each pixel by an amount read from a map.
    ///
    /// All four vendored sources are empty, so the action entry's "_Variable Blur..." gives only
    /// that it is interactive. What distinguishes it from `FocusBlur`, done last cycle, is where
    /// the variation comes FROM: focus-blur computes it geometrically from a region, this one
    /// takes it from an image. That contrast is the whole content of the name, and it is why both
    /// operations exist.
    ///
    /// The parameter shape follows this crate's OWN established pattern for map-driven filters
    /// rather than being invented: `WarpMap` and the bump maps already take
    /// `map: Option<NodeId>`, falling back to the layer's own luma when absent. Reusing it means a
    /// user who has learned one map filter has learned this one, and that the fallback behaves the
    /// way they already expect.
    ///
    /// What is NOT recoverable is upstream's own property names and whether it offers a blur-type
    /// choice. `radius` is the maximum blur, which a variable blur must have to vary between.
    VariableBlur {
        /// Blur radius where the map is white. Where the map is black nothing is blurred, and the
        /// map's own value scales between.
        radius: u32,
        /// Which layer supplies the blur amount. `None` reads the layer's own luma, matching
        /// `WarpMap` — so a bright subject blurs itself, which is rarely what is wanted but is
        /// the honest reading of "no map supplied" and keeps the family consistent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        map: Option<crate::NodeId>,
        /// How samples outside the canvas are resolved.
        #[serde(default)]
        edge_policy: crate::neighbourhood::EdgePolicy,
    },
    /// `gegl:gaussian-blur-selective` (K.3): a gaussian blur that skips high-contrast neighbours.
    ///
    /// The replaced plug-in's strings survive at `plug-ins/common/blur-gauss-selective.c` and give
    /// the whole contract: "Blur radius:", "Max. delta:", and the blurb "Blur neighboring pixels,
    /// but only in low-contrast areas".
    ///
    /// So a neighbour contributes only when it differs from the CENTRE by at most `max_delta`.
    /// That is a third mechanism for edge preservation in this group, and the three are worth
    /// telling apart because they fail differently:
    ///
    /// - `MeanCurvatureBlur` moves level sets, and cannot see a single-pixel speck at all.
    /// - `VariableBlur` takes its amount from a map, so it preserves whatever the map says to.
    /// - this one rejects neighbours by VALUE, so it preserves an edge of any shape without being
    ///   told where one is -- and leaves a lone speck alone, since the speck's own neighbours all
    ///   fail its delta test.
    ///
    /// What the strings do NOT settle is whether the delta is tested per channel or on luma.
    /// Per-channel is implemented: "Max. delta" is one value compared against channel values, and
    /// a per-channel test keeps a red edge against green -- which a luma test would blur through,
    /// since the two can share a luminance. Recorded as a choice.
    SelectiveGaussianBlur {
        /// Window half-width in pixels. Upstream's parameter is a RADIUS, not a sigma.
        radius: u32,
        /// How far a neighbour's channel value may differ from the centre's and still contribute,
        /// 0..255. Zero admits only exactly-equal neighbours, which is a no-op on any gradient.
        max_delta: u8,
        /// How samples outside the canvas are resolved.
        #[serde(default)]
        edge_policy: crate::neighbourhood::EdgePolicy,
    },
    /// `gegl:snn-mean` (K.3): symmetric nearest neighbour mean.
    ///
    /// All four vendored sources are empty, but the action entry's label spells the algorithm out
    /// in full — "_Symmetric Nearest Neighbor..." — so this is derivable from the name the way
    /// mean curvature motion was, and unlike sepia where the name named only an appearance.
    ///
    /// For each SYMMETRIC PAIR of neighbours — the sample at `+d` and the one at `−d` — take
    /// whichever is closer in value to the centre, and average those picks together with the
    /// centre.
    ///
    /// **That makes it the fourth edge-preserving mechanism in this group, and the only one with
    /// nothing to tune.** Across an edge, the pair member on the centre's own side is always the
    /// nearer in value, so the far side never contributes — no threshold, no map, no geometry.
    /// Compare `SelectiveGaussianBlur`, which needs a `max_delta` chosen to suit the image: get
    /// that number wrong and it either blurs through the edge or does nothing. SNN cannot be
    /// mistuned because it has no tuning.
    ///
    /// What is NOT recoverable is GEGL's `pairs` property, which selects how many of each pair's
    /// members to take. One per pair is the algorithm as named; taking both would make it an
    /// ordinary mean.
    SnnMean {
        /// Window half-width in pixels.
        radius: u32,
        /// How samples outside the canvas are resolved.
        #[serde(default)]
        edge_policy: crate::neighbourhood::EdgePolicy,
    },
    /// Noise reduction by outlier replacement (K.3).
    ///
    /// **The gap item is `gegl:noise-reduction`; the METHOD here is Krita's.** That needs saying
    /// plainly rather than being buried, because it is the first filter in this work whose purpose
    /// comes from one vendored upstream and whose algorithm comes from the other.
    ///
    /// GIMP's operation has no readable source: no po entry, no propgui, no `gimp:` implementation,
    /// no config object, and the action label "Noise R_eduction..." names a PURPOSE rather than an
    /// algorithm — unlike snn-mean's "Symmetric Nearest Neighbor", which named one. Inventing a
    /// method would have risked quietly duplicating one of the five mechanisms this group already
    /// has.
    ///
    /// Krita is also vendored and attributed, and it ships a readable one:
    /// `plugins/filters/imageenhancement/kis_simple_noise_reducer.cpp`, with `threshold` (0..255,
    /// default 15) and `windowsize` (0..10, default 1). Its algorithm:
    ///
    /// 1. blur the image with a CIRCULAR mask of that window size,
    /// 2. for each pixel take the difference between the ORIGINAL and the BLURRED value,
    /// 3. if that difference EXCEEDS the threshold, replace the pixel with the blurred value;
    ///    otherwise leave it exactly alone.
    ///
    /// That polarity is the opposite of [`Filter::SelectiveGaussianBlur`] and worth noticing: the
    /// selective blur includes neighbours that are SIMILAR, smoothing flat regions; this replaces
    /// pixels that are DISSIMILAR from their own surroundings, so it touches only outliers and
    /// leaves everything else byte-identical. A sixth genuinely distinct mechanism, read from
    /// source rather than guessed.
    NoiseReduction {
        /// How far a pixel may differ from its blurred self before being replaced, 0..255.
        /// Krita's default is 15.
        #[serde(default = "crate::command::krita_noise_threshold")]
        threshold: u8,
        /// Half-width of the circular blur window. Krita's declared range is 0..10 and its
        /// default is 1.
        ///
        /// **Zero is ALLOWED here**, unlike every `radius` field in this crate, which
        /// `validate_radius` refuses. The difference is deliberate: those ranges are ours, while
        /// this one is read from upstream, where 0 means a one-pixel window, a blur that is the
        /// identity, and therefore a genuine no-op. Refusing it would be overriding the source
        /// this filter is derived from.
        #[serde(default = "crate::command::krita_noise_window")]
        window_size: u32,
        /// How samples outside the canvas are resolved.
        #[serde(default)]
        edge_policy: crate::neighbourhood::EdgePolicy,
    },
    /// Difference of Gaussians edge detection (K.3).
    ///
    /// `gegl:difference-of-gaussians`, derived from source 1: the replaced plug-in is
    /// `plug-ins/common/edge-dog.c`, whose dialog the po file records as "DoG Edge Detect" with the
    /// frame "Smoothing Parameters" and, in line order, `_Radius 1:` (330), `R_adius 2:` (344),
    /// `_Normalize` (360) and `_Invert` (371). Its description is the useful part: "Edge detection
    /// with **control of edge thickness**" — the thickness is the GAP between the two radii, which
    /// is what makes this two blurs rather than one.
    ///
    /// Blur twice and subtract. The difference of two Gaussians of different widths keeps only the
    /// detail that lives between them, which is a band-pass: wide-radius structure cancels because
    /// both blurs contain it, and detail finer than the narrow radius cancels because neither
    /// does. What survives is the edges, and their thickness follows the gap.
    ///
    /// Reading that po file taught the loop a rule it did not have — see the backlog. A `grep -B`
    /// for the file name reads the PREVIOUS entry's string, because in a po file the `#:` reference
    /// line comes BEFORE the msgid it belongs to. Doing that here produced a confidently wrong
    /// parameter list: it attributed `contrast-normalize.c`'s "Stretch brightness values to cover
    /// the full range" to this filter AND missed `_Normalize`, which really is this filter's. Po
    /// files also group one msgid under every file that uses it, so a string can legitimately
    /// belong to several plug-ins at once.
    DifferenceOfGaussians {
        /// Standard deviation of the first blur, in pixels.
        radius1: f64,
        /// Standard deviation of the second blur, in pixels.
        ///
        /// The dialog presents the two symmetrically and does not require an ordering, so neither
        /// does this. Which one is larger only flips the sign of the difference, and `invert`
        /// already exists to flip it back — so refusing `radius2 > radius1` would reject a dialog
        /// state upstream allows.
        radius2: f64,
        /// Stretch the result to fill the full range.
        ///
        /// Without it the signed difference is clamped and the negative lobe is lost, which on a
        /// typical photograph is most of the output: the raw difference is small and centred on
        /// zero, so a clamped result reads nearly black. With it, the actual minimum and maximum
        /// are mapped to 0 and 255 and both lobes survive.
        #[serde(default)]
        normalize: bool,
        /// Invert the result, giving dark edges on white.
        #[serde(default)]
        invert: bool,
    },
    /// Antialias by Scale3X edge extrapolation (K.3).
    ///
    /// `gegl:antialias`. **Parameterless**, on source 4's rule: the action label is `_Antialias`
    /// with NO ellipsis, at line 65 of `filters-actions.c` — inside the array that applies with no
    /// dialog. Same basis as [`Filter::ValueInvert`]. So this variant carries no fields, and the
    /// edge policy is not exposed either: offering one would be inventing a parameter upstream
    /// does not have.
    ///
    /// The method is named outright by the replaced plug-in's own description, which is the whole
    /// derivation — `plug-ins/common/antialias.c`: "Antialias using the **Scale3X**
    /// edge-extrapolation algorithm".
    ///
    /// Scale3X is an upscaler: from a 3×3 neighbourhood it infers nine subpixels, extrapolating
    /// where a smooth edge *ought* to run from the pattern of equal and unequal neighbours. Used
    /// to antialias rather than enlarge, the nine subpixels are **averaged back down** to one.
    /// That is what turns an upscaler into an antialiaser, and it is why this smooths without
    /// blurring: wherever the extrapolation finds no diagonal structure all nine subpixels equal
    /// the centre, and the pixel comes back BYTE-IDENTICAL.
    ///
    /// Two consequences worth knowing before reaching for it:
    ///
    /// - **Straight edges are left completely alone.** A vertical or horizontal boundary fails
    ///   every Scale3X rule, so only diagonal steps and corners are softened. That is the
    ///   algorithm, not a shortfall — a staircase is what antialiasing is for.
    /// - **It compares colours for EXACT equality**, as Scale3X does, so it is built for pixel art
    ///   and hard-edged graphics. On a photograph, where neighbouring pixels are rarely bit-equal,
    ///   it will do almost nothing. Also correct, and also worth saying out loud.
    Antialias,
    /// Neon edge detection (K.4).
    ///
    /// `gegl:edge-neon`. Interactive (the label `_Neon...` carries an ellipsis), and the replaced
    /// plug-in is `plug-ins/common/edge-neon.c`, whose dialog is titled "Neon Detection" and
    /// described "Simulate the glowing boundary of a neon light".
    ///
    /// **What is READ from source, and what is INFERRED** — stated apart because this filter's
    /// arithmetic is not recoverable, and pretending otherwise is how a wrong implementation gets
    /// trusted:
    ///
    /// - READ: there are exactly two parameters, `_Radius:` (line 735) and `_Amount:` (line 750),
    ///   in that dialog order. Nothing else.
    /// - READ: the purpose is a glowing boundary, and the dialog calls it *detection*.
    /// - INFERRED: the method. A gradient magnitude of a Gaussian-blurred image — a Gaussian
    ///   derivative — is what produces glowing outlines from a *radius* plus a gain. A radius
    ///   rather than a kernel choice is what points at a Gaussian derivative instead of a fixed
    ///   3×3 Sobel, which has no radius to set.
    /// - INFERRED: that `amount` is an output gain, and its exact curve is unknown. Linear is the
    ///   assumption; a test pins it so the choice is visible rather than buried.
    ///
    /// Neither GIMP source nor Krita's could settle it. Krita's edge detection is a different
    /// shape — `horizontalRadius`/`verticalRadius` with a `type` (Sobel, Prewitt, Simple) and an
    /// `output` selector — so borrowing it would implement a different filter under this name and
    /// partly duplicate our existing `edge_detect`, which the no-duplicate condition on that
    /// source forbids.
    ///
    /// Contrast [`Filter::Antialias`], which leaves straight edges alone: this one responds to
    /// them, because detecting an edge is the whole point.
    EdgeNeon {
        /// Standard deviation of the Gaussian whose derivative is taken, in pixels.
        radius: f64,
        /// Gain applied to the gradient magnitude.
        amount: f64,
    },
    /// Antique engraving (K.4).
    ///
    /// `gegl:engrave`. Interactive (`En_grave...` carries an ellipsis), replaced plug-in
    /// `plug-ins/common/engrave.c`, dialog titled "Engraving" and described "Simulate an antique
    /// engraving".
    ///
    /// **READ from source**: exactly two parameters, `_Height:` (line 245) and the checkbox
    /// `_Limit line width` (line 256), in that order. Nothing else.
    ///
    /// **INFERRED**: the mechanics, though the parameter names constrain them tightly. An engraving
    /// renders tone as horizontal lines of varying thickness — ink or no ink, never grey — so a
    /// `height` is the band one line occupies, and the line's thickness within its band encodes how
    /// dark that part of the image is. `_Limit line width` then says what it says: it bounds the
    /// thickness so the line can neither vanish nor fill its band solid.
    ///
    /// The output is **binary** per channel, which is the property that makes it an engraving rather
    /// than a posterisation: a real engraver has one ink and varies coverage, not density.
    Engrave {
        /// Height in pixels of the band one engraved line occupies.
        height: u32,
        /// Bound the line thickness so it neither vanishes nor fills the band.
        ///
        /// Without it a white region engraves to nothing and a black region to a solid block;
        /// with it every band keeps at least one inked row and at least one bare one, so the line
        /// structure survives across the whole tonal range.
        #[serde(default)]
        limit: bool,
    },
    /// Superimposed rotated copies (K.4).
    ///
    /// `gegl:illusion`. Interactive (`_Illusion...`), replaced plug-in
    /// `plug-ins/common/illusion.c`, described "Superimpose many altered copies of the image".
    ///
    /// **READ**: `_Divisions:` (line 389) and a radio pair `Mode _1` (399) / `Mode _2` (414). So
    /// there are exactly two parameters and the second has exactly two values.
    ///
    /// **INFERRED**: the transform. "Superimpose many altered copies" plus a *divisions* count is a
    /// rotational superimposition about the image centre — `divisions` copies, each turned by
    /// `2πk/divisions`, averaged together.
    ///
    /// **The mode distinction had to be reasoned out, and the obvious guess is provably wrong.**
    /// The labels say nothing, so the first guess is that mode 2 reverses the rotation direction.
    /// That cannot be it: negating every angle produces the same SET of copies in a different
    /// order, and averaging is order-independent, so mode 2 would be byte-identical to mode 1 and
    /// upstream would not offer it. The distinction must therefore change the copies themselves,
    /// and the natural partner to a rotation is a reflection — so mode 2 mirrors before rotating,
    /// giving a kaleidoscope with reflection symmetry where mode 1 has only rotational symmetry.
    /// A test pins the dihedral reasoning by checking mode 2 at one division is the mirror, which
    /// a direction-flip reading would make the identity.
    Illusion {
        /// How many copies to superimpose.
        divisions: u32,
        /// Rotation only, or reflection then rotation.
        #[serde(default)]
        mode: crate::command::IllusionMode,
    },
    /// Irregular tiling (K.4).
    ///
    /// `gegl:mosaic`, described "Convert the image into irregular tiles". **The most complete READ
    /// contract in this work**: every parameter's name, its widget kind and its exact dialog
    /// position come from `plug-ins/common/mosaic.c`, so unusually little is inferred.
    ///
    /// | # | line | parameter |
    /// |---|---|---|
    /// | 1 | 642 | `_Tiling primitives:` — the four of [`TilingPrimitive`] |
    /// | 2 | 650 | `Tile _size:` |
    /// | 3 | 662 | `Tile _height:` |
    /// | 4 | 675 | `Til_e spacing:` |
    /// | 5 | 687 | `Tile _neatness:` |
    /// | 6 | 700 | `Light _direction:` |
    /// | 7 | 712 | `Color _variation:` |
    /// | 8 | 729 | `_Antialiasing` |
    /// | 9 | 741 | `Co_lor averaging` |
    /// | 10 | 754 | `Allo_w tile splitting` |
    /// | 11 | 767 | `_Pitted surfaces` |
    /// | 12 | 780 | `_FG/BG lighting` |
    ///
    /// **The progress strings are algorithmic evidence, not decoration.** The plug-in reports
    /// "Finding edges" and then "Rendering tiles", which proves a two-phase algorithm: it locates
    /// image contours BEFORE laying tiles. That is what `Allo_w tile splitting` acts on — a tile
    /// that would straddle a contour is split at it, so tiles follow the picture rather than
    /// ignoring it. Without that string the flag would read as something about the image border.
    ///
    /// INFERRED: the cell construction. Tiles come from seed points on the primitive's lattice with
    /// nearest-seed assignment, which is what makes a *size* and a *neatness* meaningful — neatness
    /// perturbs the seeds, so 1.0 is the exact lattice and 0.0 is fully irregular. "Octagons &
    /// squares" needs the one refinement: equal weights on two interleaved square lattices give
    /// diamonds, so the octagon seeds carry a weight that lets their cells grow past the
    /// perpendicular bisector into octagons.
    Mosaic {
        /// Which tiling to lay.
        #[serde(default)]
        primitive: crate::command::TilingPrimitive,
        /// Lattice step in pixels.
        tile_size: u32,
        /// Bevel depth. 0 is flat.
        #[serde(default)]
        tile_height: f64,
        /// Width of the grout between tiles, in pixels. 0 butts them together.
        #[serde(default)]
        tile_spacing: f64,
        /// 1.0 is the exact lattice; 0.0 is fully irregular.
        #[serde(default = "crate::command::unit_one")]
        tile_neatness: f64,
        /// Direction the bevel is lit from, in **degrees** — the unit every angle in this crate
        /// carries in its name or its docs rather than being guessed at.
        #[serde(default)]
        light_direction: f64,
        /// Per-tile colour jitter, 0.0 for none.
        #[serde(default)]
        color_variation: f64,
        /// Supersample the tile and grout decision so cell edges are not stair-stepped.
        #[serde(default)]
        antialiasing: bool,
        /// Take each tile's colour as the mean over the whole tile rather than the seed's own pixel.
        #[serde(default)]
        color_averaging: bool,
        /// Split a tile where it would straddle an image contour — the flag the "Finding edges"
        /// phase exists to serve.
        #[serde(default)]
        allow_tile_splitting: bool,
        /// Add surface noise to the bevel shading.
        #[serde(default)]
        pitted_surfaces: bool,
        /// Light the bevel with `foreground`/`background` instead of white and black.
        #[serde(default)]
        fg_bg_lighting: bool,
        /// Highlight colour when `fg_bg_lighting` is set.
        ///
        /// Upstream reads the application's current foreground and background. Our commands are
        /// self-contained — a saved command must replay identically whatever the palette now holds
        /// — so the two colours are carried here instead of read from app state.
        #[serde(default = "crate::command::white")]
        foreground: Pixel,
        /// Shadow colour when `fg_bg_lighting` is set.
        #[serde(default = "crate::command::black")]
        background: Pixel,
    },
    /// Glass-block distortion (K.4).
    ///
    /// `gegl:tile-glass`, described "Simulate distortion caused by square glass tiles", dialog
    /// "Glass Tile".
    ///
    /// **READ**: exactly two parameters, `Tile _width:` (line 290) and `Tile _height:` (line 304).
    /// Note they are SEPARATE axes — [`Filter::Mosaic`] has a single `Tile _size:`, so the
    /// difference is upstream's and not a liberty taken here. A tall narrow tile distorts
    /// differently from a wide flat one, and a test pins that the two are independent.
    ///
    /// **INFERRED**: the refraction. A thick glass block does not shift the view, it compresses it:
    /// the line of sight bends more the further from the block's axis you look. So the sample
    /// offset grows with the distance from the tile centre, which doubles the span each tile draws
    /// from — every tile shows twice its own area, and the seams between tiles are the
    /// discontinuities that make the effect read as glass rather than as a blur.
    ///
    /// The consequence worth knowing: a pixel exactly at a tile's centre samples **itself**, so
    /// tile centres come through untouched. That is the sharpest available check on the geometry.
    TileGlass {
        /// Tile width in pixels.
        tile_width: u32,
        /// Tile height in pixels.
        tile_height: u32,
    },
    /// Cut into paper tiles and slide them (K.4).
    ///
    /// `gegl:tile-paper`, described "Cut image into paper tiles, and slide them", dialog
    /// "Paper Tile". The dialog is read in frames:
    ///
    /// | frame | line | contents |
    /// |---|---|---|
    /// | Division | 270 | `_X:` 283, `_Y:` 292, `_Width:` 303, `_Height:` 314 |
    /// | Fractional Pixels | 320 | `_Background` 325, `_Ignore` 327, `_Force` 329 |
    /// | — | 336 | `C_entering` |
    /// | Movement | 351 | `_Max (%):` 364, `_Wrap around` 370 |
    /// | Background Type | 380 | `_Transparent` 385 … `S_elect here:` 395 |
    ///
    /// **`_Max (%)` states its unit in the label**, which is worth noting because most of this
    /// work's units had to be recovered from a description instead.
    ///
    /// **The "Division" frame holds FOUR controls and the operation cannot take four parameters.**
    /// Over a fixed image, a division count and a tile size determine each other — `width =
    /// image_width / x` — so an operation accepting both could be handed a contradiction. One pair
    /// is the parameter and the other is the dialog's convenience.
    ///
    /// Which one is **not recoverable**. The four labels are generic strings shared across seven
    /// plug-ins, so they carry no evidence, and tile-paper has no propgui and no config object to
    /// settle it. This is the third time a dialog control has turned out not to be a parameter —
    /// `range` on color-balance and on hue-saturation were the first two — and the first time no
    /// source can decide it, so the choice is recorded rather than presented as a reading:
    ///
    /// - a tile SIZE is meaningful on its own; a division count means nothing except relative to
    ///   the image it divides;
    /// - [`Filter::Mosaic`] takes `Tile _size:` and [`Filter::TileGlass`] takes width and height,
    ///   so sizes keep the family consistent.
    ///
    /// So `tile_width`/`tile_height` are the parameters and `_X:`/`_Y:` are treated as the dialog's
    /// derived view.
    TilePaper {
        /// Tile width in pixels.
        tile_width: u32,
        /// Tile height in pixels.
        tile_height: u32,
        /// How far a tile may slide, as a PERCENTAGE of its own size — upstream's own unit.
        #[serde(default)]
        move_max: f64,
        /// A tile sliding off one edge reappears at the opposite one.
        #[serde(default)]
        wrap_around: bool,
        /// Centre the tile grid on the image instead of starting it at the origin.
        #[serde(default)]
        centering: bool,
        /// What to do with the partial tiles at the far edges.
        #[serde(default)]
        fractional_pixels: crate::command::FractionalPixels,
        /// What shows through where a tile has slid away.
        #[serde(default)]
        background_type: crate::command::PaperBackground,
        /// Colour for `PaperBackground::ForegroundColor`.
        ///
        /// Upstream reads the application's palette for this and the next. Ours are carried in the
        /// command for the reason `Mosaic`'s are: a saved command must replay identically whatever
        /// the palette holds later.
        #[serde(default = "crate::command::white")]
        foreground: Pixel,
        /// Colour for `PaperBackground::BackgroundColor`.
        #[serde(default = "crate::command::black")]
        background: Pixel,
        /// Colour for `PaperBackground::Selected`, the dialog's own picker.
        #[serde(default = "crate::command::black")]
        selected: Pixel,
    },
    /// Windblown smear (K.4).
    ///
    /// `gegl:wind`, described "Smear image to give windblown effect". Closes K.4.
    ///
    /// **This is the only filter in the group whose scalar parameters came with TOOLTIPS**, which
    /// is the first explicit statement of semantics rather than a name to reason from:
    ///
    /// - `_Threshold:` (1006), tooltip at 1010 — *"Higher values restrict the effect to fewer areas
    ///   of the image"*. So it is a GATE on which edges are smeared at all, not a scale on the
    ///   result. Rising threshold must affect strictly fewer pixels, and a test asserts exactly
    ///   that monotonicity, because the tooltip is the evidence and testing it is testing the
    ///   derivation.
    /// - `_Strength:` (1025), tooltip at 1029 — *"Higher values increase the magnitude of the
    ///   effect"*. So it is the smear LENGTH, and rising strength must affect strictly more pixels.
    ///
    /// The radio groups are Style (919), Direction (943) and Edge Affected (967).
    ///
    /// INFERRED, and marked because the labels do not say: which sign of edge counts as *leading*.
    /// [`WindEdge::Leading`] is taken as the edge where brightness RISES along the blow direction —
    /// the lit front the wind strikes — and `Trailing` the falling one. The assignment is a choice;
    /// what is *not* a choice is that `Both` is their union, which is asserted.
    ///
    /// Neither scalar carries a unit upstream, so `threshold` is read against a channel difference
    /// in 0..255 (Chebyshev, this crate's established metric since `color-to-alpha`) and `strength`
    /// is read as a smear length in pixels. Both recorded as choices.
    Wind {
        /// Long fading streaks, or short uniform bursts.
        #[serde(default)]
        style: crate::command::WindStyle,
        /// Which way the wind blows.
        #[serde(default)]
        direction: crate::command::WindDirection,
        /// Which side of an edge is smeared.
        #[serde(default)]
        edge: crate::command::WindEdge,
        /// Minimum channel difference for an edge to be smeared at all, 0..255.
        #[serde(default)]
        threshold: u8,
        /// Smear length in pixels.
        strength: u32,
    },
    /// Rectangular ↔ polar remapping (K.5).
    ///
    /// `gegl:polar-coordinates`. **The weakest evidence position in this work, and the strongest
    /// name.** Those are worth separating:
    ///
    /// - Source 1 is EMPTY. The replaced plug-in is gone from the tree — `plug-ins/common/` holds
    ///   87 files and none of them is this one — so the po snapshot has no strings for it. Searching
    ///   the po files for "Polar" finds only `displace.c`, `flame.c` and `gfig`.
    /// - Sources 2 and 3 are empty: no propgui, no config object.
    /// - Krita, the fifth source, has no equivalent.
    /// - Source 4 gives one fact: `P_olar Coordinates...` carries an ellipsis, so it is interactive
    ///   and has at least one parameter.
    ///
    /// **But the name is the specification here, which is NOT the position sepia was in.** Sepia's
    /// label named an appearance, so its matrix constants were unrecoverable and any value would
    /// have looked plausible. "Polar coordinates" names an exact mapping: angle across one axis,
    /// radius along the other. There is nothing to guess about the arithmetic, and two consequences
    /// of the name alone are strong enough to test against — concentric rings must become
    /// horizontal stripes, and the round trip must return the image.
    ///
    /// The pole is the image centre, which is not a parameter because it is not a choice: no other
    /// origin is distinguished, and inventing `x`/`y` fields would be adding a parameter upstream
    /// may not have.
    ///
    /// The radius is normalised PER ANGLE to the image boundary in that direction, so the rectangle
    /// maps onto the whole (angle, radius) rectangle rather than onto an inscribed disc. That is
    /// what makes the transform a bijection and the round trip exact.
    PolarCoordinates {
        /// Rectangular to polar, or back again.
        ///
        /// INFERRED that this exists at all, and marked. The argument: a change of coordinate
        /// system that cannot be reversed is not a change of coordinate system — the inverse is the
        /// same operation read the other way, so it belongs to what the name denotes rather than
        /// being an extra feature bolted on.
        #[serde(default = "crate::command::enabled")]
        to_polar: bool,
    },
    /// Spherical bulge or pinch (K.5).
    ///
    /// `gegl:spherize`. Every source is empty but for the ellipsis, as with
    /// [`Filter::PolarCoordinates`] — and here there is corroboration for *why*: GIMP's own appdata
    /// release note says "2 new filters: \"Spherize\" and \"Recursive Transform\"", so spherize is
    /// GEGL-native and never had a plug-in predecessor. That is what an empty `po-plug-ins` means
    /// for it, rather than a lost file.
    ///
    /// (That note was reached by grepping `po/` for the name, which turned up only release prose.
    /// A hit is not evidence until the file it is in has been looked at — cycle 39's rule — and
    /// this one turned out to be worth something anyway, just not as a property list.)
    ///
    /// **ONE parameter, deliberately.** The ellipsis proves there is at least one; nothing proves
    /// what the rest are. A signed curvature is entailed by the name, because the pinch is the same
    /// mapping run the other way and 0 must be the identity. A mode selector — radial against
    /// per-axis — is NOT entailed: a sphere is radial, and adding an axis choice would be inventing
    /// a parameter rather than deriving one. Absent is more honest than guessed.
    ///
    /// The sphere is inscribed, its radius half the shorter side, so the image outside the ball is
    /// untouched. That is what makes it a ball resting on the picture rather than a warp of the
    /// whole frame, and it is exactly testable.
    Spherize {
        /// −1 pinches, 0 is the identity, +1 is a full hemisphere bulge.
        ///
        /// The geometry at the extremes is a sphere seen head-on: a bulge samples at
        /// `(2/π)·asin(ρ)`, which moves outward more slowly than the output radius and so magnifies
        /// the centre, and a pinch samples at `sin(ρ·π/2)`, which does the reverse. Intermediate
        /// values interpolate from the identity toward whichever extreme the sign selects.
        curvature: f64,
    },
    /// Stereographic projection, the "little planet" effect (K.5).
    ///
    /// `gegl:stereographic-projection`. **Upstream presents it as `_Little Planet...`**, not by its
    /// own name, which is worth noticing: by the rule the backlog now carries, "Little Planet" is a
    /// LABEL — it names an appearance and derives nothing — while "stereographic projection" is a
    /// SPECIFICATION, an exact and named mapping. The operation name is the one that carries
    /// evidence.
    ///
    /// Sources are otherwise empty: no propgui, no config object, nothing in `po-plug-ins`.
    /// (`gimppropgui-panorama-projection.c` DOES exist and names `pan`, `tilt`, `spin`, `zoom` and
    /// `inverse` — but that is a different operation, also in K.5, and attributing its properties
    /// here would be the false-attribution mistake cycle 51 made with a po file. Recorded against
    /// `panorama-projection` instead.)
    ///
    /// **Two parameters, each with an argument**, following spherize's discipline:
    ///
    /// - `inverse` is entailed the way polar's direction flag is: the projection is invertible and
    ///   the inverse is the same mapping read the other way, not a separate feature.
    /// - `zoom` is entailed by a different kind of necessity. The stereographic projection of a
    ///   sphere is UNBOUNDED — the pole opposite the projection point goes to infinity — so
    ///   rendering it into a finite raster requires a bound, and that bound is not something the
    ///   mapping can supply. A filter without it would be undefined as drawn.
    ///
    /// NOT entailed, and so absent: pan, tilt and spin. The projection has a standard form, from
    /// one pole onto the plane, and an orientation is a convenience rather than part of the notion.
    /// Upstream's sibling has them; that is not evidence that this one does.
    ///
    /// The input is read as equirectangular — x is longitude, y is latitude — which is what makes
    /// the bottom row the nadir and puts it at the centre of the little planet.
    ///
    /// **What separates this from [`Filter::PolarCoordinates`] is the radial profile, and only
    /// that.** Both map angle to one axis and radius to the other; a linear radius gives the plain
    /// polar remap, while `ψ = 2·atan(r)` gives the projection. A test names both numbers so the
    /// two filters cannot quietly become the same thing.
    StereographicProjection {
        /// How much of the sphere lands inside the frame. At 1.0 the equator falls on the inscribed
        /// circle.
        #[serde(default = "crate::command::unit_one")]
        zoom: f64,
        /// Project back from the plane to the equirectangular image.
        #[serde(default)]
        inverse: bool,
    },
    /// Rectilinear view of an equirectangular panorama (K.5).
    ///
    /// `gegl:panorama-projection`. **The best source-2 evidence in this work**: the propgui at
    /// `app/propgui/gimppropgui-panorama-projection.c` does not merely name the properties, it names
    /// their RELATIONS — and it does so twice, in both directions, so the two readings prove each
    /// other rather than one of them being a guess.
    ///
    /// `gyroscope_callback` sets the operation from the widget:
    ///
    /// ```text
    /// "pan",     -yaw,
    /// "tilt",    -pitch,
    /// "spin",    -roll,
    /// "zoom",    CLAMP (100.0 * zoom, 0.01, 1000.0),
    /// "inverse", invert,
    /// ```
    ///
    /// and `config_notify` reads it back as `-pan, -tilt, -spin, zoom / 100.0`. So:
    ///
    /// - `pan`, `tilt` and `spin` are the **negations** of yaw, pitch and roll. A sign convention
    ///   is normally the first thing lost when a source tree is unreadable; here it is written down.
    /// - `zoom` is a **percentage** — the controller's fraction times 100 — with an explicitly
    ///   declared range of **0.01 to 1000**. That is the first real upstream range in K.5; every
    ///   other bound in this group is ours.
    ///
    /// The widget is a gyroscope controller rather than sliders, which is why the operation has a
    /// custom propgui at all and therefore why any of this is readable.
    ///
    /// INFERRED and marked: the angles are taken as DEGREES, which the names yaw/pitch/roll and a
    /// drag controller imply but nothing states; and the rotation ORDER — yaw, then pitch, then
    /// roll — is a choice, since the propgui passes the three together and never composes them. The
    /// sign test below holds whatever the order, because it moves one angle at a time.
    PanoramaProjection {
        /// Horizontal look direction in degrees. Upstream's `pan`, which is **−yaw**.
        #[serde(default)]
        pan: f64,
        /// Vertical look direction in degrees. Upstream's `tilt`, which is **−pitch**.
        #[serde(default)]
        tilt: f64,
        /// Roll about the view axis in degrees. Upstream's `spin`, which is **−roll**.
        #[serde(default)]
        spin: f64,
        /// Field of view, as a PERCENTAGE. Upstream's declared range is 0.01 to 1000, and 100 is a
        /// 90-degree horizontal view.
        #[serde(default = "crate::command::percent_hundred")]
        zoom: f64,
        /// Project a rectilinear view back out to an equirectangular panorama.
        #[serde(default)]
        inverse: bool,
    },
    /// Recursively composited transforms — the Droste effect (K.5).
    ///
    /// `gegl:recursive-transform`. The other filter GIMP's appdata note named beside Spherize, so
    /// also GEGL-native with no plug-in predecessor.
    ///
    /// **Source 2 reads unusually precisely here, and also states its own limit.**
    /// `app/propgui/gimppropgui-recursive-transform.c`:
    ///
    /// - the property is `transform`, a STRING holding a `;`-separated list. `add_transform`
    ///   appends `";matrix (1, 0, 0, 0, 1, 0, 0, 0, 1)"`, which is **nine** numbers — a 3×3
    ///   **projective** matrix, not a six-number affine. That distinction is the whole reason the
    ///   filter can do perspective nesting, and a test pins it.
    /// - `duplicate_transform` copies the text after the last `;`; `remove_transform` truncates
    ///   there — but **guarded by `if (delim)`**, so a single-entry list cannot be emptied. At
    ///   least one transform always exists. That is an invariant read from source, not a courtesy.
    /// - the file then says, in its own comment, that it *"skip[s] the \"transform\" property, which
    ///   is controlled by a transform-grid controller"* and hands `param_specs + 1` to the generic
    ///   builder. So **there are further properties whose names this source does not reveal.**
    ///   Knowing that is better than assuming `transform` is the only one.
    ///
    /// `iterations` is entailed by necessity rather than by a label, the same argument that
    /// justified [`Filter::StereographicProjection`]'s `zoom`: the recursion does not terminate on
    /// its own and the raster is finite, so a bound is required for the operation to be computable
    /// at all. Nothing else is entailed, so nothing else is here.
    ///
    /// Upstream's `;`-separated string is the GUI's serialisation, not the operation's data model.
    /// Ours is the same information typed, which is what the rest of this command enum does.
    RecursiveTransform {
        /// The transforms to iterate, each a row-major 3×3 projective matrix.
        ///
        /// `[a, b, c, d, e, f, g, h, i]` maps `(x, y)` to `((ax+by+c)/(gx+hy+i),
        /// (dx+ey+f)/(gx+hy+i))`. Identity is `[1,0,0, 0,1,0, 0,0,1]`, exactly as upstream's
        /// `add_transform` writes it.
        transforms: Vec<[f64; 9]>,
        /// How deep to recurse. Must be at least 1.
        #[serde(default = "crate::command::one_iteration")]
        iterations: u32,
    },
    /// Kaleidoscope fold (K.5).
    ///
    /// `gegl:mirrors`, presented upstream as `_Kaleidoscope...`. Two names again, as with
    /// `stereographic-projection` / "Little Planet", and again only one of them derives anything:
    /// "Kaleidoscope" names an appearance, "mirrors" names the mechanism.
    ///
    /// Every source is empty but for the ellipsis — no propgui, no config object, nothing in
    /// `po-plug-ins`.
    ///
    /// **"mirrors" sits between a specification and a label**, which is a case the backlog's rule
    /// did not yet cover. "Polar coordinates" denotes exactly one mapping; "sepia" denotes only a
    /// look. This one names a determinate *mechanism* — reflection about lines through a centre —
    /// while leaving the *configuration* open. So the mechanism is read and the configuration is
    /// what the single parameter must be.
    ///
    /// And that is the argument for the count, which is tighter than it looks: the ellipsis proves
    /// there is at least one parameter, and the name is **plural but unquantified**. A fixed number
    /// of mirrors would leave the filter with nothing to set. So the number of mirrors is precisely
    /// the thing the name itself leaves open, and it is the one parameter here.
    ///
    /// NOT entailed, and absent: the orientation of the mirror set (a kaleidoscope with a fixed
    /// orientation is perfectly usable, so a rotation is a convenience), and the centre — the image
    /// centre is the only distinguished choice, as with [`Filter::Spherize`]'s pole.
    Mirrors {
        /// How many mirror lines pass through the centre.
        ///
        /// `n` lines divide the plane into `2n` wedges and give the result `n`-fold dihedral
        /// symmetry: it is unchanged by a rotation of `2π/n` and by reflection in each line.
        mirrors: u32,
    },
    /// Per-line displacement (K.5).
    ///
    /// `gegl:shift`. Every source is empty but for the ellipsis — no propgui, no config object, and
    /// nothing in `po-plug-ins`, the plug-in having gone from the tree as polar-coordinates' did.
    ///
    /// **What `shift` is NOT was settled from source, by its sibling rather than by its own name.**
    /// The name alone is ambiguous between a uniform translation and a per-line displacement, and
    /// the first reading is the simpler one. But `filters-actions.c` registers a SEPARATE operation
    /// `gimp:offset` as `_Offset...`, with its own keyboard shortcut — upstream would not ship two
    /// plain translations under different names. So the uniform reading is excluded by evidence,
    /// not by preference, and the displacement must be non-uniform.
    ///
    /// That is the same no-duplicate reasoning that kept Krita's edge detector out of `edge-neon`,
    /// pointed at upstream's own catalogue instead of ours — and it is stronger there, because a
    /// duplicate within one product is a contradiction rather than a judgement call.
    ///
    /// **Both parameters are degrees of freedom the name leaves open**, which is the argument shape
    /// cycle 67 preferred:
    ///
    /// - the amount, because "shift" does not say how far;
    /// - the axis, because a displacement needs a direction and **nothing privileges either**. That
    ///   is the difference from [`Filter::Spherize`], where "sphere" makes radial the only
    ///   non-arbitrary choice; here a default would be a coin toss dressed up as a reading.
    ///
    /// Lines wrap rather than leaving a gap. A CHOICE, recorded as one: wrapping loses nothing and
    /// makes a row's pixels a rotation of the original row, which is an invariant a test can check
    /// exactly. Clamping or filling would be equally defensible and is not readable either way.
    Shift {
        /// Greatest displacement in pixels. 0 is the identity.
        amount: u32,
        /// Rows along x, or columns along y.
        #[serde(default)]
        axis: crate::command::ShiftAxis,
    },
    /// Refraction through a lens (K.5).
    ///
    /// `gegl:apply-lens`, dialog "Lens Effect". **Source 1 took a second look to find**: the
    /// replaced plug-in is `plug-ins/common/lens-apply.c`, not `apply-lens.c` — the operation name
    /// reverses the file's words, so searching for the operation name finds nothing. Worth adding
    /// to the locate step: try the words the other way round before concluding source 1 is empty,
    /// because `lens-distortion.c` and `lens-flare.c` sit right beside it and neither is this one.
    ///
    /// READ: `_Lens refraction index:` (line 477) and the surroundings radio of
    /// [`LensSurroundings`]. Described "Simulate an **elliptical** lens over the image", and that
    /// adjective is load-bearing: the lens is the ellipse inscribed in the image bounds, not a
    /// circle, so on a non-square canvas it reaches into corners a circle would miss. A test pins
    /// it, because it is the one thing the description says outright that the name does not.
    ///
    /// The physics is then determinate. The lens is a half-ellipsoid over the image; the surface
    /// normal at each point is `(dx/a², dy/b², z/c²)`; the view ray refracts by Snell's law at the
    /// ratio `1/index`; and the refracted ray is followed to the image plane, which is the point
    /// sampled. Nothing there is a choice except the depth `c`, taken as the shorter semi-axis so
    /// the bulge is as deep as the lens is narrow — recorded as a choice, since no source gives it.
    ///
    /// An index of 1.0 is air and refracts nothing, so it is **exactly** the identity: at `η = 1`
    /// the refracted ray is the incident ray and the sampled point is the pixel itself.
    ApplyLens {
        /// Refractive index of the lens. 1.0 is air and the identity.
        refraction_index: f64,
        /// What to do with the image outside the lens.
        #[serde(default)]
        surroundings: crate::command::LensSurroundings,
        /// Colour for `LensSurroundings::Background` on a non-indexed document.
        ///
        /// Carried in the command rather than read from app state, as `Mosaic`'s and `TilePaper`'s
        /// are: a saved command must replay identically whatever the palette later holds.
        #[serde(default = "crate::command::black")]
        background: Pixel,
    },
    /// Spread chosen values into neighbouring pixels (K.5).
    ///
    /// **The strongest contract in group K, and it comes from source 4 — the route the backlog
    /// deliberately excluded.** `filters-actions.c` reaches this operation twice more through
    /// `filters_settings_actions`, and those entries carry the properties as a **literal list**:
    ///
    /// ```text
    /// "gegl:value-propagate\n"
    /// "(mode white)" "(lower-threshold 0.000000)" "(upper-threshold 1.000000)"
    /// "(rate 1.000000)" "(top yes)" "(left yes)" "(right yes)" "(bottom yes)"
    /// "(value yes)" "(alpha no)"
    /// ```
    ///
    /// That is eleven property NAMES with their types, which is what a propgui would have given.
    /// Every one is corroborated by the plug-in's own dialog strings: `Lower t_hreshold:` (1166),
    /// `_Upper threshold:` (1178), `_Propagating rate:` (1190), `To l_eft`/`To _right`/`To
    /// _top`/`To _bottom` (1201 to 1210), `Propagating _alpha channel` (1219) and `Propagating
    /// value channel` (1230). Two independent sources naming the same eleven things.
    ///
    /// **Dilate and Erode differ in the mode and in NOTHING else** — the two action strings are
    /// otherwise character-identical. That is a read fact, and the test asserts it.
    ///
    /// The action strings order the directions `top, left, right, bottom` while the dialog orders
    /// them `left, right, top, bottom`. Reference line numbers give the dialog order, so the fields
    /// follow the dialog.
    ///
    /// INFERRED, not read: the ranges. Both thresholds appear as `0.000000` and `1.000000` in the
    /// same six-decimal form, so they are the 0..1 value scale; `rate` appears as `1.000000` in a
    /// preset that must apply the effect fully, which reads as its maximum. The mode ARITHMETIC for
    /// the four peak and foreground modes is not readable from strings either — the mode list is
    /// read, the mechanism is reconstructed, and the two are kept apart here as in `EdgeNeon`.
    ValuePropagate {
        /// Which values propagate.
        #[serde(default)]
        mode: crate::command::PropagateMode,
        /// A pixel only propagates if its value is at or above this. Read as 0..1.
        #[serde(default)]
        lower_threshold: f64,
        /// ...and at or below this.
        #[serde(default = "crate::command::unit_one")]
        upper_threshold: f64,
        /// How far the result moves toward the propagated value. 0 is the identity.
        #[serde(default = "crate::command::unit_one")]
        rate: f64,
        /// Propagate leftward. Dialog position 1.
        #[serde(default = "crate::command::enabled")]
        left: bool,
        /// Dialog position 2.
        #[serde(default = "crate::command::enabled")]
        right: bool,
        /// Dialog position 3.
        #[serde(default = "crate::command::enabled")]
        top: bool,
        /// Dialog position 4.
        #[serde(default = "crate::command::enabled")]
        bottom: bool,
        /// Propagate the colour channels. `(value yes)` in both presets.
        #[serde(default = "crate::command::enabled")]
        value: bool,
        /// Propagate alpha. `(alpha no)` in both presets, so this defaults OFF — the one property
        /// whose default is read from source rather than chosen.
        #[serde(default)]
        alpha: bool,
        /// Foreground colour, for the two foreground modes and `ForegroundToPeaks`.
        #[serde(default = "crate::command::black")]
        foreground: Pixel,
        /// Background colour, for `OnlyBackground`.
        #[serde(default = "crate::command::white")]
        background: Pixel,
    },
    /// Distance from each pixel to the nearest pixel outside the set (K.5).
    ///
    /// `gegl:distance-transform`, presented as `Distance _Map...`. GEGL-native, as spherize is: no
    /// propgui, no config object, no replaced plug-in, and — checked per cycle 70's new step — no
    /// preset in `filters_settings_actions` either. The action entry and its ellipsis are the whole
    /// of the GIMP evidence.
    ///
    /// **The name is a SPECIFICATION, not a label** (cycles 61 and 62). A distance transform is an
    /// exact, standard operation: for each pixel, the distance to the nearest pixel that is not in
    /// the set. Nothing about the arithmetic is open, so its own consequences are strong enough to
    /// test against with no source at all — which is the polar-coordinates position, not the sepia
    /// one.
    ///
    /// Three parameters, each ENTAILED rather than imagined:
    ///
    /// - the **metric**, because "distance" does not say which;
    /// - the **threshold**, because the operation is defined on a SET and an image is not one, so
    ///   something must make it one. That is a requirement, not a guess;
    /// - **normalisation**, because a 1000-pixel image has distances past 500 while the output
    ///   holds 0..255, so the result must either clamp or be scaled, and which is a real choice.
    DistanceTransform {
        /// Which distance to measure.
        #[serde(default)]
        metric: crate::command::DistanceMetric,
        /// A pixel is in the set when its luminance is at or above this, on 0..1.
        #[serde(default = "crate::command::unit_half")]
        threshold: f64,
        /// Scale the result so the largest distance present becomes 255. With this off the raw
        /// distance is written and anything past 255 clamps.
        #[serde(default)]
        normalize: bool,
    },
    /// Simple Linear Iterative Clustering — superpixels by k-means in colour and space (K.5).
    ///
    /// `gegl:slic`, and upstream spells the name out in the menu rather than abbreviating it:
    /// `_Simple Linear Iterative Clustering...`. That is a **citation**, not a label — the name of
    /// a published algorithm with a fixed definition — which puts this at the strongest end of the
    /// specification case: the contract is the algorithm.
    ///
    /// GEGL-native. No propgui, no config object, no plug-in, no preset. The action entry and its
    /// ellipsis are the whole of the GIMP evidence, and Krita has nothing to corroborate from.
    ///
    /// Three parameters, each from the algorithm's own definition:
    ///
    /// - the **cluster size**, the grid spacing the centres start on, which sets how many
    ///   superpixels there are;
    /// - the **compactness**, the weight between colour distance and spatial distance. Without it
    ///   the two distances have no common scale, so it is required rather than chosen;
    /// - the **iteration count** — and here the name does the arguing itself. "Iterative" is a word
    ///   in the operation's own title, so the number of iterations is a degree of freedom the name
    ///   explicitly states, not merely leaves open. That is cycle 67's argument shape with the
    ///   source handing it over.
    Slic {
        /// Grid spacing of the initial cluster centres, in pixels.
        #[serde(default = "crate::command::default_cluster_size")]
        cluster_size: u32,
        /// Weight on the spatial term. Larger keeps superpixels squarer; smaller lets them follow
        /// colour.
        #[serde(default = "crate::command::default_compactness")]
        compactness: f64,
        /// How many assign-and-recentre passes to run.
        #[serde(default = "crate::command::default_slic_iterations")]
        iterations: u32,
    },
    /// Waterpixels — superpixels by a watershed on a regularised gradient (K.5).
    ///
    /// `gegl:waterpixels`, presented as `_Waterpixels...`. Also GEGL-native with nothing but the
    /// action entry behind it, and also a published algorithm's name.
    ///
    /// **Upstream shipping this AND `gegl:slic` is what requires the two to be different.** Cycle
    /// 68's route — that the operations around a filter constrain it — is used here in the opposite
    /// direction: there it excluded a reading because a duplicate would be a contradiction, and
    /// here the same argument forbids implementing one of these as an alias of the other. Two names
    /// in one catalogue mean two mechanisms, so this is a minimum-cost flood from grid seeds over
    /// the image gradient, not k-means.
    ///
    /// Which also gives them different blind spots, as K.3's six edge-preserving mechanisms did:
    /// SLIC's cells are pulled toward colour means and can cross a thin edge that no cluster centre
    /// sits across; a watershed cannot cross a ridge at all, but will happily let one cell swallow a
    /// flat region its neighbour should have had.
    Waterpixels {
        /// Grid spacing of the seeds, in pixels.
        #[serde(default = "crate::command::default_cluster_size")]
        cluster_size: u32,
        /// How much the distance from the seed is added to the gradient. 0 follows the image alone;
        /// large values drive the cells back toward the grid.
        #[serde(default = "crate::command::default_regularization")]
        regularization: f64,
    },
    /// Draw a labyrinth (K.6).
    ///
    /// `gegl:maze`, dialog "Maze". **Source 1 at its richest in this work** — three plug-in files,
    /// one of them a dedicated `maze-algorithms.c` — giving every name, every widget kind and the
    /// dialog order.
    ///
    /// **FOUR controls, TWO degrees of freedom, and here the po file PROVES it.** The `Maze Size`
    /// frame (line 184) holds:
    ///
    /// ```text
    /// 198  Width (pixels):     210  Pieces:
    /// 215  Height (pixels):    226  Pieces:
    /// ```
    ///
    /// `Pieces:` carries **two** reference lines, 210 and 226, interleaving with the pixel sizes at
    /// 198 and 215 — so the frame is two rows, each offering the same number twice: a cell size in
    /// pixels and the count of cells it implies (`pieces = extent / cell`). Only one per axis can be
    /// an input.
    ///
    /// This is the third time a dialog has shown more controls than the operation has parameters —
    /// after tile-paper's division frame and hue-saturation's `range` — and the **first where a
    /// source settles which**. In tile-paper nothing could decide and it was recorded as a choice.
    /// Here the reference line numbers give the dialog order, the pixel size leads each row, and the
    /// piece count follows as the derived readout. So the parameter is the cell size.
    ///
    /// It also nearly went the other way: read with one reference line per msgid, `Pieces:` looks
    /// like a single control and the frame looks like three parameters. The cycle-54 rule — read the
    /// WHOLE reference block, because po groups one msgid under every line that uses it — is what
    /// turned a wrong count into the argument above.
    Maze {
        /// `Width (pixels):`, line 198. The width of one maze unit.
        #[serde(default = "crate::command::default_maze_cell")]
        cell_width: u32,
        /// `Height (pixels):`, line 215.
        #[serde(default = "crate::command::default_maze_cell")]
        cell_height: u32,
        /// `Seed:`, line 251. Read from source, so reproducibility is in the contract rather than
        /// merely being our convention.
        #[serde(default)]
        seed: u32,
        /// The radio pair at lines 260 and 261.
        #[serde(default)]
        algorithm: crate::command::MazeAlgorithm,
        /// `Tileable`, line 268. A different CONSTRUCTION, not a post-process — see
        /// [`MazeAlgorithm`] for the evidence.
        #[serde(default)]
        tileable: bool,
        /// Wall colour. In the command, not app state, as every other FG/BG in this work.
        #[serde(default = "crate::command::black")]
        foreground: Pixel,
        /// Passage colour.
        #[serde(default = "crate::command::white")]
        background: Pixel,
    },
    /// Draw a grid on the image (K.6).
    ///
    /// **The strongest contract in this entire work, and it came from a source the backlog does not
    /// list: the plug-in's own C.** `plug-ins/common/grid.c` is VENDORED. The five derivation
    /// sources were written on the premise that a replaced plug-in's source is gone and its po
    /// strings are the only trace — which is true of every filter so far, because upstream DELETED
    /// each plug-in when its GEGL operation landed. `grid.c` survived, and 351 plug-in C files are
    /// there to be read.
    ///
    /// So instead of names and guessed ranges, this is read verbatim from
    /// `gimp_procedure_add_int_argument` declarations — name, type, minimum, maximum and default:
    ///
    /// | argument | min | max | default |
    /// |---|---|---|---|
    /// | `hwidth` / `vwidth` | 0 | 524288 | 1 |
    /// | `iwidth` | 0 | 524288 | **0** |
    /// | `hspace` / `vspace` | **1** | 524288 | 16 |
    /// | `ispace` | 1 | 524288 | 2 |
    /// | `hoffset` / `voffset` | 0 | 524288 | 8 |
    /// | `ioffset` | 0 | 524288 | 6 |
    ///
    /// Two things there would have been guessed wrong. **A width's minimum is 0 and a spacing's is
    /// 1** — asymmetric, and for a reason: an invisible line is meaningful while a spacing of zero
    /// is not. That overrides our `validate_radius` convention exactly as noise-reduction's
    /// `window_size` did, and for the same stated reason: a range read from source outranks our
    /// convention. And **`iwidth` defaults to 0**, so intersections are off until asked for.
    ///
    /// **A fourth dialog-state-not-a-parameter case, and the first the SOURCE labels.** `grid.c`
    /// also declares `width-unit`, `space-unit` and `offset-unit`, but through
    /// `gimp_procedure_add_unit_aux_argument` rather than `add_int_argument` — upstream marking
    /// them as dialog state in the function name itself. In tile-paper the same question had to be
    /// recorded as a choice because nothing could decide it.
    Grid {
        /// Thickness of the horizontal lines. 0 draws none.
        #[serde(default = "crate::command::default_grid_width")]
        horizontal_width: u32,
        /// Distance between horizontal lines. Minimum 1, read from source.
        #[serde(default = "crate::command::default_grid_space")]
        horizontal_space: u32,
        /// Phase of the horizontal lines.
        #[serde(default = "crate::command::default_grid_offset")]
        horizontal_offset: u32,
        /// `hcolor`, default black.
        #[serde(default = "crate::command::black")]
        horizontal_color: Pixel,
        /// Thickness of the vertical lines.
        #[serde(default = "crate::command::default_grid_width")]
        vertical_width: u32,
        #[serde(default = "crate::command::default_grid_space")]
        vertical_space: u32,
        #[serde(default = "crate::command::default_grid_offset")]
        vertical_offset: u32,
        /// `vcolor`, default black.
        #[serde(default = "crate::command::black")]
        vertical_color: Pixel,
        /// Thickness of the intersection strokes. **Defaults to 0**, so intersections are off.
        #[serde(default)]
        intersection_width: u32,
        /// How far from a crossing the intersection arm STARTS. Default 2.
        #[serde(default = "crate::command::default_intersection_space")]
        intersection_space: u32,
        /// How far from a crossing the arm ENDS. Default 6.
        ///
        /// Read from the drawing loop, not from the label: the arm is painted where the distance
        /// from the crossing is at least `space` and less than `offset`, so the two together make a
        /// **crosshair with a gap at the crossing itself** rather than a filled block. The strings
        /// alone said "Intersection / Width / Spacing / Offset" and would have produced a square.
        #[serde(default = "crate::command::default_intersection_offset")]
        intersection_offset: u32,
        /// `icolor`, default black.
        #[serde(default = "crate::command::black")]
        intersection_color: Pixel,
    },
    /// Render a spiral (K.6).
    ///
    /// `gegl:spiral`, presented as `S_piral...`. The plug-in route checked first per cycle 75:
    /// `plug-ins/gfig/gfig-spiral.c` exists but belongs to the **gfig** interactive figure editor,
    /// a different thing from this render generator, so it is not the source. Source 2 is, and it is
    /// strong — `app/propgui/gimppropgui-spiral.c` names seven properties on `config`.
    ///
    /// **It names them through RELATIONS, in both directions, so the readings prove each other** —
    /// the panorama-projection situation, and the strongest form of propgui evidence:
    ///
    /// ```text
    /// x        = x1 / area->width          x1 = x * area->width
    /// y        = y1 / area->height         y1 = y * area->height
    /// radius   = sqrt(SQR(x2-x1) + SQR(y2-y1))
    /// rotation = atan2(-(y2-y1), x2-x1) * 180 / G_PI   (+360 if negative)
    /// ```
    ///
    /// So **`x` and `y` are normalised to the area, 0..1, while `radius` is in PIXELS** — an
    /// asymmetry that would have been guessed wrong either way. `rotation` is in **degrees** over
    /// 0..360, with the **y axis negated** (screen down against maths up).
    ///
    /// The slider arithmetic pins the two scalars, forward and inverse, and the pairs invert exactly:
    ///
    /// | type | forward | inverse | endpoints |
    /// |---|---|---|---|
    /// | linear | `s = 0.5 + (1 - balance)/4` | `balance = 3 - 4s` | `s ∈ [0.5, 1]` ⇒ `balance ∈ [-1, 1]` |
    /// | log | `s = base^(-(balance+1)/4)` | `balance = -4·log(s)/log(base) - 1` | `s ∈ [base^-0.5, 1]` ⇒ `balance ∈ [-1, 1]` |
    /// | log | `s = 1/base` | `base = 1/s`, capped at 1e6 | `s ∈ [0, 1]` ⇒ `base ≥ 1` |
    ///
    /// `balance`'s range is doubly read: an explicit `CLAMP (balance, -1.0, 1.0)` in the source, and
    /// both slider endpoint calculations landing on exactly ±1.
    ///
    /// INFERRED, and kept apart as in edge-neon: what `balance` DOES. The range is read and
    /// symmetric about 0, so it is taken as the share of each turn given to the first colour, 0.5 at
    /// `balance = 0`. The colours themselves are not named by the propgui — a propgui names only
    /// what it builds custom widgets for, handing the rest to the generic builder — so they are
    /// entailed by this being a generator rather than read, and live in the command as every other
    /// FG/BG in this work does.
    Spiral {
        /// Which law the arms follow.
        #[serde(default)]
        spiral_type: crate::command::SpiralType,
        /// Centre, as a fraction of the area's width. Read as normalised.
        #[serde(default = "crate::command::unit_half")]
        x: f64,
        /// Centre, as a fraction of the area's height.
        #[serde(default = "crate::command::unit_half")]
        y: f64,
        /// Reference radius, in **pixels** — not normalised, unlike `x` and `y`.
        #[serde(default = "crate::command::default_spiral_radius")]
        radius: f64,
        /// Degrees, 0..360.
        #[serde(default)]
        rotation: f64,
        /// Growth per turn. **Logarithmic only** — the propgui gives linear one slider. `>= 1`,
        /// capped at 1e6 by upstream's own `MIN`.
        #[serde(default = "crate::command::default_spiral_base")]
        base: f64,
        /// −1..1, read from an explicit CLAMP and confirmed by both slider endpoints.
        #[serde(default)]
        balance: f64,
        /// First band colour.
        #[serde(default = "crate::command::black")]
        color1: Pixel,
        /// Second band colour.
        #[serde(default = "crate::command::white")]
        color2: Pixel,
    },
    /// Generate complex sinusoidal textures (K.6).
    ///
    /// `gegl:sinus`. Plug-in route checked first and the file is **gone** — `find` turns up nothing
    /// — while `po-plug-ins` still references `plug-ins/common/sinus.c`. That is the deletion
    /// pattern cycle 75 established: upstream removes a plug-in when its GEGL operation lands, and
    /// the strings are the remaining trace. So source 1, and it is unusually complete — three
    /// dialog tabs, every name and every widget kind, in dialog order.
    ///
    /// READ, by line: `_X scale:` 701, `_Y scale:` 710, `Co_mplexity:` 719 under `Drawing Settings`
    /// 691; `R_andom seed:` 742, `_Force tiling?` 751 and the `_Ideal`/`_Distorted` pair 764/765
    /// under `Calculation Settings` 729; the gradient radio 909–911 and `_Exponent:` 923 under
    /// `Blend Settings` 896.
    ///
    /// **A FIFTH dialog-state-not-a-parameter case, and the largest.** The `Colors` frame (799)
    /// holds a three-way radio — `Bl_ack & white` 803, `_Foreground & background` 805, `C_hoose
    /// here:` 807 — plus two colour buttons (820, 830) and two alpha sliders in an `Alpha Channels`
    /// frame (856, 871). Seven controls. But all three radio options write into the same **two**
    /// colour properties: one fills them with black and white, one takes them from context, one
    /// lets you pick. The alphas are those same colours' alpha channels, which our `Pixel` already
    /// carries. So seven controls, **two** degrees of freedom — and our standing invariant already
    /// decides it, since FG/BG live in the command rather than in app state precisely so a saved
    /// command replays identically whatever the palette later holds.
    ///
    /// INFERRED, and kept apart as in edge-neon: the arithmetic. The plug-in is deleted, so the
    /// sine construction, what `complexity` multiplies, the shape of the `Distorted` perturbation
    /// and the exponent's curve are reconstructed from the names and from what the names entail.
    /// What is NOT inferred is the behaviour the tests pin: that tiling closes over the canvas, that
    /// the two colours bound the output, and that each parameter changes the texture on its own.
    Sinus {
        /// `_X scale:`, line 701.
        #[serde(default = "crate::command::default_sinus_scale")]
        x_scale: f64,
        /// `_Y scale:`, line 710.
        #[serde(default = "crate::command::default_sinus_scale")]
        y_scale: f64,
        /// `Co_mplexity:`, line 719. How many sine terms contribute.
        #[serde(default = "crate::command::default_sinus_complexity")]
        complexity: f64,
        /// `R_andom seed:`, line 742.
        #[serde(default)]
        seed: u32,
        /// `_Force tiling?`, line 751. Snaps every frequency to a whole number of cycles across the
        /// canvas, which is what makes the result wrap.
        #[serde(default)]
        tiling: bool,
        /// The radio pair at 764/765.
        #[serde(default)]
        perturbation: crate::command::SinusPerturbation,
        /// First colour, with its alpha from the `Alpha Channels` frame.
        #[serde(default = "crate::command::black")]
        color1: Pixel,
        /// Second colour.
        #[serde(default = "crate::command::white")]
        color2: Pixel,
        /// The gradient radio at 909–911.
        #[serde(default)]
        blend: crate::command::SinusBlend,
        /// `_Exponent:`, line 923. 0 leaves the blend alone; the control is signed about that
        /// neutral middle.
        ///
        /// **Direction MEASURED, not assumed.** The first draft of this comment had it backwards —
        /// it said positive pushes toward the second colour. A positive exponent raises the blend
        /// factor to a higher power, and the factor is 0 at `color1`, so raising it pulls toward
        /// **`color1`**; negative does the reverse. Measured means against a black-to-white pair:
        /// 247.9 at −4, 228.2 at −2, 139.4 at 0, 45.3 at +2, 16.1 at +4.
        ///
        /// The direction itself is a CHOICE, since the plug-in that declared it is deleted — but it
        /// is the conventional one: a power applied to a 0..1 factor is a gamma, and a gamma above 1
        /// darkens.
        #[serde(default)]
        exponent: f64,
    },
    /// A sinusoid whose phase is linear in position (K.6).
    ///
    /// `gegl:linear-sinusoid`. Every source is empty but the action entry: no plug-in ever (the
    /// route cycle 75 added turns up nothing), no propgui, no config object, no po reference, no
    /// preset, and nothing in Krita. The polar-coordinates position.
    ///
    /// **An EIGHTH route settled what kind of thing it is: the menu XML declares the category.**
    /// `menus/image-menu.ui.in.in:836` places it in the `_Pattern` submenu beside `bayer-matrix`,
    /// `checkerboard`, `diffraction-patterns`, `grid`, `maze`, `sinus` and `spiral` — every one a
    /// render generator. That is a declaration, where the five sources above answer a different
    /// question (what the parameters are) and could not answer this one at all.
    ///
    /// **And it caught a red herring I had already half-believed.** Only TWO entries in the whole
    /// actions file carry `GIMP_ICON_TOOL_LEVELS`: `gimp:levels` and this one. Two out of ~126 is
    /// not a fallback, so it reads as deliberate — and a Levels icon says *tonal mapping*, which
    /// would have made this a transfer curve rather than a generator. The menu says otherwise.
    /// An icon is a UI asset choice and carries no semantic guarantee; a menu category is upstream
    /// stating the kind. Same shape as sepia's Krita grep returning data that resembled proof
    /// exactly (cycle 62): the more specific-looking signal was the wrong one.
    ///
    /// **Distinctness from `sinus`, which landed the cycle before, is forced by the catalogue**
    /// (cycle 68's route, as slic and waterpixels needed). `gegl:sinus` is a RANDOM sum of sines
    /// with a seed and a complexity; shipping both names means this cannot be that. So it is the
    /// deterministic single-frequency case: **no seed, no complexity.**
    ///
    /// The name is a SPECIFICATION in the cycle-61 sense. "Linear sinusoid" says the sinusoid's
    /// ARGUMENT is a linear function of position — `sin(ax + by + c)` — which fixes the form and
    /// leaves exactly its coefficients open. Two periods and one phase is therefore the entailed
    /// set, not a guess about it.
    ///
    /// A PRODUCT of two sinusoids was considered and rejected: that would give a lattice rather
    /// than a grating, and two multiplied sinusoids are not *a* sinusoid. The same reading also
    /// rules out a second phase — for one wave only the combined offset is observable, so a
    /// per-axis pair would be a redundant control invented rather than derived, which is the trap
    /// the four-controls-two-freedoms cases keep pointing at. Upstream may well have more; **absent
    /// is more honest than guessed**, as with spherize, and the suspicion is filed as a gap instead.
    LinearSinusoid {
        /// Pixels per cycle along x. Together with `y_period` this is the wave's direction and
        /// wavelength.
        #[serde(default = "crate::command::default_sinusoid_period")]
        x_period: f64,
        /// Pixels per cycle along y.
        #[serde(default = "crate::command::default_sinusoid_period")]
        y_period: f64,
        /// Phase, in degrees, so the unit matches `Spiral`'s rotation rather than introducing
        /// radians to the command surface.
        #[serde(default)]
        phase: f64,
        /// Trough colour.
        #[serde(default = "crate::command::black")]
        color1: Pixel,
        /// Crest colour.
        #[serde(default = "crate::command::white")]
        color2: Pixel,
    },
    /// Render a Bayer ordered-dither matrix as a visible pattern (K.6).
    ///
    /// `gegl:bayer-matrix`, in the `_Pattern` submenu (`menus/image-menu.ui.in.in:832`, the eighth
    /// route confirming the kind). Every other source is empty: no plug-in, no propgui, no config
    /// object, no po reference, no preset.
    ///
    /// **But a Bayer matrix is an exactly defined object, so the contract IS the definition** — the
    /// strongest specification case, as `slic` and `distance-transform` were. Built recursively
    /// from `[[0, 2], [3, 1]]`, each step scaling by four and tiling four offset copies in BLOCKS:
    ///
    /// ```text
    /// M(2n) = [ 4*M(n) + 0   4*M(n) + 2 ]
    ///         [ 4*M(n) + 3   4*M(n) + 1 ]
    /// ```
    ///
    /// **The block form is not the only plausible recursion, and the wrong one survives the obvious
    /// tests.** An interleaved variant — placing the four offsets at a stride rather than as blocks
    /// — still yields a permutation of `0..4^n - 1` and still tiles, so a test for either property
    /// passes on it. What tells them apart is this codebase's own `ORDERED_MATRIX` in
    /// `color_mode.rs`, a literal 4x4 written for the indexed-mode dither long before this filter:
    /// the block form reproduces it exactly and the interleaved form does not.
    ///
    /// That adjacency is worth being precise about rather than treating as a duplicate.
    /// `DitherMode::Ordered` CONSUMES a Bayer matrix as a per-pixel threshold while converting to
    /// indexed colour; this operation RENDERS one as a pattern. Different operations over the same
    /// mathematical object — the Krita-propagatecolors situation, where the shared thing was a
    /// distance metric. A test pins the two to the same matrix so they cannot drift apart, which
    /// also turns the dither's magic numbers into derived ones.
    ///
    /// Also checked and unrelated: the `bayer` hits in `raw.rs` are camera colour-filter-array
    /// mosaics, a different Bayer entirely.
    ///
    /// Two parameters, each the degree of freedom the name leaves open: the ORDER, because "Bayer
    /// matrix" does not say which, and the two colours, because a generator must have them.
    BayerMatrix {
        /// Recursion depth. The tile is `2^order` on a side and holds `4^order` distinct values, so
        /// order 1 is the 2x2 base case and order 2 the familiar 4x4.
        #[serde(default = "crate::command::default_bayer_order")]
        order: u32,
        /// Colour of value 0.
        #[serde(default = "crate::command::black")]
        color1: Pixel,
        /// Colour of the largest value.
        #[serde(default = "crate::command::white")]
        color2: Pixel,
    },
    /// Generate diffraction patterns (K.6).
    ///
    /// `gegl:diffraction-patterns`, `_Pattern` submenu. **Two sources that confirm each other, and
    /// neither alone would have been enough.**
    ///
    /// Source 2, `app/propgui/gimppropgui-diffraction-patterns.c`, is the POSITIONAL case AUDIT-7
    /// recorded for color-rotate, in a second shape: it passes SLICES to the generic builder —
    /// `param_specs + 0, 3`, `+ 3, 3`, `+ 6, 3`, `+ 9, 3` — so it names no property name at all,
    /// while naming the count (**12**) and the grouping (**four groups of three**) exactly, with the
    /// four tab labels `Frequencies`, `Contours`, `Sharp Edges`, `Other Options`.
    ///
    /// Source 1 supplies the names, and its reference lines settle the assignment:
    ///
    /// ```text
    /// 504 _Red:   513 _Green:  522 _Blue:    530 "Frequencies"
    /// 542 _Red:   551 _Green:  560 _Blue:    568 "Contours"
    /// 580 _Red:   589 _Green:  598 _Blue:    606 "Sharp Edges"
    /// 618 _Brightness:  627 Sc_attering:  636 Po_larization:  644 "Other Options"
    /// ```
    ///
    /// `_Red:` carries **three** references inside this file, and so do Green and Blue — the
    /// cycle-54 whole-block rule again, as maze's `Pieces:` needed.
    ///
    /// **The label comes AFTER its page's widgets here, not before.** Assuming the usual order would
    /// have mis-assigned every group by one: the first RGB triple would have gone to no label and
    /// the last would have been orphaned. The propgui's counts are what let the assignment be
    /// CHECKED rather than guessed — which is the whole value of two sources agreeing on the same
    /// structure from different directions.
    ///
    /// INFERRED, and kept apart as in edge-neon: the arithmetic. The plug-in is deleted, so how a
    /// frequency becomes a fringe, what a contour count shapes and how a sharp-edge term steepens
    /// are reconstructed. What is NOT inferred is the thing the tests pin — that these are three
    /// independent per-channel triples, which is read from the grouping and is exactly what a
    /// careless implementation would couple.
    DiffractionPatterns {
        /// `Frequencies` tab, `_Red:`.
        #[serde(default = "crate::command::default_diffraction_frequency")]
        frequency_red: f64,
        #[serde(default = "crate::command::default_diffraction_frequency")]
        frequency_green: f64,
        #[serde(default = "crate::command::default_diffraction_frequency")]
        frequency_blue: f64,
        /// `Contours` tab, `_Red:`.
        #[serde(default = "crate::command::default_diffraction_contours")]
        contour_red: f64,
        #[serde(default = "crate::command::default_diffraction_contours")]
        contour_green: f64,
        #[serde(default = "crate::command::default_diffraction_contours")]
        contour_blue: f64,
        /// `Sharp Edges` tab, `_Red:`.
        #[serde(default)]
        edges_red: f64,
        #[serde(default)]
        edges_green: f64,
        #[serde(default)]
        edges_blue: f64,
        /// `Other Options` tab, `_Brightness:`.
        #[serde(default = "crate::command::default_diffraction_brightness")]
        brightness: f64,
        /// `Sc_attering:`.
        #[serde(default)]
        scattering: f64,
        /// `Po_larization:`.
        #[serde(default)]
        polarization: f64,
    },
    /// Perlin gradient noise (K.6).
    ///
    /// `gegl:perlin-noise`, presented as `Perlin _Noise...` in the `N_oise` submenu
    /// (`menus/image-menu.ui.in.in:825`). Every source is empty but the action entry: no plug-in
    /// ever, no propgui, no config object, no po reference, no preset, nothing in Krita. The
    /// polar-coordinates position.
    ///
    /// **But Perlin noise is a published algorithm, so the contract is the definition** — the
    /// `slic` / `distance-transform` / `bayer-matrix` case. Gradients at the integer points of a
    /// square lattice, each dotted with the offset to the sample, the four corners interpolated
    /// with Perlin's own quintic ease `6t^5 - 15t^4 + 10t^3`.
    ///
    /// **Distinctness from `gegl:simplex-noise` is forced by the catalogue** (cycle 68's route, as
    /// slic and waterpixels needed), and it is sharper here than usual: simplex noise was invented
    /// by the same author as a REPLACEMENT for this one, so the two look alike and are structurally
    /// different — a square lattice with four corners against a simplex tiling with three, summed
    /// through a radial kernel rather than interpolated. Upstream shipping both names means neither
    /// may be the other.
    ///
    /// The definition also hands over the test: **gradient noise is exactly zero at every lattice
    /// point**, because the gradient there is dotted with a zero offset. Nothing else in this group
    /// has an invariant that exact, and value noise or a simplex would not have it on this lattice.
    ///
    /// ENTAILED rather than read: the `scale`, because "Perlin noise" does not say how big a cell
    /// is; the `seed`, because the gradients have to come from somewhere, and classical Perlin uses
    /// one fixed table, which would give a paint program exactly one noise field forever; and the
    /// two colours, because a generator must have them.
    ///
    /// One recorded CHOICE, and it departs from the rest of the group. The value is normalised by
    /// the theoretical bound of `sqrt(2)/2` rather than by the extremes actually present, so the
    /// mapping does not depend on the canvas and a lattice point lands exactly on the midpoint.
    /// The cost is that this generator does NOT reach both colours, where every other one in K.6
    /// does: the bound is rarely attained by any real sample. Stretching to the measured extremes
    /// would reach them and would make the same request give different pixels at different canvas
    /// sizes, which is the worse trade.
    PerlinNoise {
        /// Pixels per lattice cell. Larger is coarser.
        #[serde(default = "crate::command::default_noise_scale")]
        scale: f64,
        /// Picks the gradient table.
        #[serde(default)]
        seed: u32,
        /// Colour at the low end.
        #[serde(default = "crate::command::black")]
        color1: Pixel,
        /// Colour at the high end.
        #[serde(default = "crate::command::white")]
        color2: Pixel,
    },
    /// Simplex noise (K.6).
    ///
    /// `gegl:simplex-noise`, presented as `_Simplex Noise...` in the `N_oise` submenu
    /// (`menus/image-menu.ui.in.in:827`). Sources as empty as `perlin-noise`'s: the action entry
    /// and nothing else.
    ///
    /// **A published algorithm, so the contract is the definition** — and that definition is what
    /// keeps it distinct from `gegl:perlin-noise`, which upstream ships alongside it (cycle 68's
    /// route). The input is SKEWED into a triangular lattice by `F2 = (sqrt(3) - 1) / 2`, the
    /// containing simplex gives **three** corners rather than a square's four, and each contributes
    /// through the radial kernel `(0.5 - r^2)^4` rather than being interpolated.
    ///
    /// **The two definitions give opposite answers to the same question, which is the paired test.**
    /// Perlin noise is exactly zero at every integer lattice point, because the gradient there meets
    /// a zero offset and no other corner reaches. Simplex noise is NOT: a vertex kills its own
    /// corner's contribution, but the other two corners of the simplex are inside the kernel's
    /// support and still contribute. So the assertion that pins `PerlinNoise` must come back false
    /// here, and a simplex implemented as a renamed Perlin would fail it.
    ///
    /// The gradient hashing IS shared with `PerlinNoise`, deliberately: hashing a lattice point to a
    /// direction is the same mechanism in both, and the thing that differs — and that the catalogue
    /// requires to differ — is the sampling structure around it.
    ///
    /// ENTAILED rather than read, as with Perlin: the `scale`, the `seed`, and the two colours.
    /// No octave count.
    SimplexNoise {
        /// Pixels per lattice cell before skewing. Larger is coarser.
        #[serde(default = "crate::command::default_noise_scale")]
        scale: f64,
        /// Picks the gradient table.
        #[serde(default)]
        seed: u32,
        /// Colour at the low end.
        #[serde(default = "crate::command::black")]
        color1: Pixel,
        /// Colour at the high end.
        #[serde(default = "crate::command::white")]
        color2: Pixel,
    },
    /// The spatial gradient of the image (K.6, misfiled — see below).
    ///
    /// `gegl:image-gradient`. **The menu route (cycle 78's eighth source) overturned this
    /// backlog's own classification, and more sharply than it did for perlin and simplex.**
    /// `menus/image-menu.ui.in.in:773` places it in the **`Edge-De_tect`** submenu, beside
    /// `difference-of-gaussians`, `edge`, `edge-laplace`, `edge-neon` and `edge-sobel` — every one an
    /// edge operator, and four of them already handled here. So this is a spatial DERIVATIVE, not a
    /// render generator, where K.6's header files it as one.
    ///
    /// That matters because the name invites the other reading: "image gradient" reads perfectly
    /// well as a rendered colour ramp, and nothing but the menu says otherwise. Cycle 78's case was
    /// a wrong group NAME over a correctly-classified item; this one would have been the wrong
    /// filter entirely. The item is implemented here rather than moved, because audit3 reconciles on
    /// names and not on groups, and the cycle-54 lesson is that group headers are the fragile part.
    ///
    /// Checked and excluded, as `gfig-spiral.c` was for spiral: `app/operations/gimpoperationgradient.c`
    /// exists but registers `"gimp:gradient"`, the gradient TOOL's operation, not this one.
    ///
    /// **Distinctness from `gegl:edge-sobel`, which upstream ships alongside and this work has
    /// filed but not done, is forced by the catalogue** — and the split is derivable rather than
    /// arbitrary. `edge-sobel` names a KERNEL, so its open choices are about applying it: which
    /// axes, whether to keep the sign. `image-gradient` names the QUANTITY, so its open choice is
    /// which component of the vector to write. Accordingly this uses the plain central difference,
    /// which IS the discrete gradient, where Sobel's kernel is a gradient smoothed across three
    /// rows.
    ///
    /// One recorded CHOICE, and the parameter set is what forces it: the gradient is defined on a
    /// SCALAR field, so three colour channels must be reduced to one. Luminance, because
    /// [`GradientOutput::Direction`] has to be a single angle — a per-channel reading would give
    /// three directions and the output mode could not name one value.
    ImageGradient {
        /// Which component of the vector to write.
        #[serde(default)]
        output: crate::command::GradientOutput,
    },
    ColorEnhance,
    /// Inverts the HSV VALUE, keeping hue and saturation (K.1).
    ///
    /// A third distinct member of the invert family: `Filter::Invert` is `gegl:invert-gamma`
    /// (complement each stored channel), `invert-linear` complements in linear light, and this
    /// complements only brightness. Parameterless, as both vendored invert wrappers are.
    ValueInvert,
    /// The same complement as [`Self::Invert`], applied in LINEAR light (K.1).
    ///
    /// `Invert` is `gegl:invert-gamma` and complements the stored, sRGB-encoded value; this
    /// decodes to linear first. The vendored wrappers `gimp_gegl_apply_invert_gamma` and
    /// `gimp_gegl_apply_invert_linear` sit directly next to each other, which is what makes the
    /// two a deliberate pair rather than one filter with a flag. Parameterless, as both are.
    InvertLinear,
    /// The image minus a blurred copy of itself: what is left is the high spatial frequencies.
    ///
    /// `std_dev` is the blur's standard deviation — GEGL's own name for it, from the
    /// `gegl:gaussian-blur` call in `app/gegl/gimp-gegl-apply-operation.c`. `contrast` scales the
    /// extracted detail. Output is centred on mid-grey, because the difference is signed and an
    /// unsigned buffer cannot hold negative detail (K.1).
    HighPass {
        std_dev: f32,
        contrast: f32,
    },
    /// Clips samples into a range — the only K.1 filter whose POINT is the precision work (K.1).
    ///
    /// At 8- and 16-bit the encodings cannot hold a value outside 0..1, so this is inert there by
    /// construction. At F32 the storage is raw `f32` with no clamping, so out-of-range samples
    /// genuinely exist and clipping them is a real operation. That is why it is
    /// precision-native rather than narrowed to bytes first, which would clip as a side effect of
    /// the conversion and make the filter look like it worked.
    RgbClip {
        #[serde(default = "crate::command::clip_enabled_by_default")]
        clip_low: bool,
        #[serde(default = "crate::command::clip_enabled_by_default")]
        clip_high: bool,
        #[serde(default)]
        low_limit: f32,
        #[serde(default = "crate::command::unit_high_limit")]
        high_limit: f32,
    },
    /// Lifts shadows and recovers highlights using a BLURRED luminance mask (K.1).
    ///
    /// `radius` is upstream's "spatial extent": it is what makes this a local operator rather than
    /// a tone curve. Ranges are upstream's own, taken from the PDB wrapper in
    /// `app/pdb/drawable-color-cmds.c`: shadows and highlights -100..100, radius 0.1..1500.
    ///
    /// Upstream also has `whitepoint`, `compress`, `shadows-ccorrect` and `highlights-ccorrect`.
    /// Those are NOT exposed here, deliberately — accepting a parameter and ignoring it is worse
    /// than not offering it, because the caller cannot tell. Filed as its own backlog item.
    ShadowsHighlights {
        shadows: f32,
        highlights: f32,
        radius: f32,
        /// "Shift white point", -10..10. Upstream's blurb and range.
        #[serde(default)]
        whitepoint: f32,
        /// "Compress the effect on shadows/highlights and preserve midtones", 0..100.
        ///
        /// Zero is NEUTRAL and means no compression, so an existing command deserialises to
        /// exactly the behaviour it had before these four fields existed.
        #[serde(default)]
        compress: f32,
        /// "Adjust saturation of shadows", 0..100 — how much of the original saturation to
        /// restore after the tone change, which desaturates by compressing channel differences.
        ///
        /// Defaults to 100 (fully restore). Zero would leave the lifted region washed out, which
        /// is a legitimate look but not the one an unset parameter should produce.
        #[serde(default = "crate::command::full_colour_correction")]
        shadows_ccorrect: f32,
        /// "Adjust saturation of highlights", 0..100. Same meaning and default.
        #[serde(default = "crate::command::full_colour_correction")]
        highlights_ccorrect: f32,
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
        /// Cyan-to-red shift for MIDTONES, −100..100. Upstream's own range is −1..1; ours is
        /// scaled by 100 and that is kept, so commands saved before this filter gained its two
        /// other ranges still mean what they meant.
        red: f32,
        /// Magenta-to-green shift for midtones, −100..100.
        green: f32,
        blue: f32,
        /// Cyan-to-red shift for SHADOWS, −100..100.
        ///
        /// Three ranges exist because upstream's config stores an array per axis --
        /// `config->cyan_red[GIMP_TRANSFER_SHADOWS]` and its two siblings -- and the operation
        /// applies **all three at once**. That corrects what AUDIT-4 recorded: reading the
        /// property names alone suggested `range` SELECTED which range the filter touched, and
        /// reading the operation showed it is the dialog's own state, naming which of the three
        /// stored triples the sliders currently edit. So `range` is NOT a field here, for the same
        /// reason color-exchange's "Lock thresholds" is not: it is a widget, not an operation
        /// parameter.
        #[serde(default)]
        red_shadows: f32,
        /// Magenta-to-green shift for shadows, −100..100.
        #[serde(default)]
        green_shadows: f32,
        #[serde(default)]
        blue_shadows: f32,
        /// Cyan-to-red shift for HIGHLIGHTS, −100..100.
        #[serde(default)]
        red_highlights: f32,
        #[serde(default)]
        green_highlights: f32,
        #[serde(default)]
        blue_highlights: f32,
        /// Restore each pixel's original lightness after the shift.
        ///
        /// Upstream converts the RESULT to HSL, copies the ORIGINAL lightness back in, and
        /// converts back. That is a different mechanism from the flag of the same name on
        /// channel-mixer and mono-mixer, which normalises weights -- AUDIT-4's note said to reuse
        /// that rule and was wrong; reading the operation is what caught it.
        ///
        /// Upstream's default is TRUE. Ours is `false` via `#[serde(default)]`, deliberately:
        /// cycle 38 established that a field added to a shipped command variant must default to
        /// the behaviour the variant already had, or every saved command quietly changes meaning.
        /// The dialog should offer true as its initial value; the COMMAND cannot.
        #[serde(default)]
        preserve_luminosity: bool,
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
        /// Normalise each OUTPUT row's three weights to sum to 1 before mixing, so changing the
        /// balance between inputs does not also change that channel's brightness.
        ///
        /// Upstream has had this since before our filter existed and we did not: a gap the
        /// name-based measurement could never see, since `channel-mixer` has always counted as
        /// covered. Found in cycle 37 while deriving `MonoMixer` from this operation's own
        /// property GUI.
        ///
        /// Per-ROW is not a guess. `app/propgui/gimppropgui-channel-mixer.c` groups the nine
        /// gains into three frames labelled "Red Channel", "Green Channel" and "Blue Channel",
        /// each holding that output's three input weights -- so a frame IS a row -- and the single
        /// `preserve-luminosity` checkbox sits outside all three, applying to every row. A global
        /// normalisation over all nine would make the three outputs interfere, which the layout
        /// contradicts.
        #[serde(default)]
        preserve_luminosity: bool,
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
pub(crate) const PRECISION_NATIVE_FILTERS: &[&str] = &["invert", "invert_linear", "rgb_clip"];

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
    "stretch_contrast",
    "stretch_contrast_hsv",
    "shadows_highlights",
    "color_enhance",
    "value_invert",
    "invert_linear",
    "alien_map",
    "color_exchange",
    "color_rotate",
    "color_to_alpha",
    "component_extract",
    "mono_mixer",
    "sepia",
    "colorize",
    "median_blur",
    "mean_curvature_blur",
    "focus_blur",
    "variable_blur",
    "selective_gaussian_blur",
    "snn_mean",
    "noise_reduction",
    "difference_of_gaussians",
    "antialias",
    "edge_neon",
    "engrave",
    "illusion",
    "mosaic",
    "tile_glass",
    "tile_paper",
    "wind",
    "polar_coordinates",
    "spherize",
    "stereographic_projection",
    "panorama_projection",
    "recursive_transform",
    "mirrors",
    "shift",
    "apply_lens",
    "value_propagate",
    "distance_transform",
    "slic",
    "waterpixels",
    "maze",
    "grid",
    "spiral",
    "sinus",
    "linear_sinusoid",
    "bayer_matrix",
    "diffraction_patterns",
    "perlin_noise",
    "simplex_noise",
    "image_gradient",
    "high_pass",
    "rgb_clip",
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
        /// Fit cubic Béziers to the traced boundary instead of emitting straight segments (J.4-b).
        ///
        /// Defaults TRUE, which is upstream's own default: a straight trace of a round selection is
        /// one anchor per boundary step. A rectangle comes out the same either way, because a
        /// straight run fits a line with no measurable error.
        #[serde(default = "crate::command::fit_paths_by_default")]
        fit: bool,
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

pub(crate) fn clip_enabled_by_default() -> bool {
    true
}

pub(crate) fn unit_high_limit() -> f32 {
    1.0
}

pub(crate) fn full_colour_correction() -> f32 {
    100.0
}

pub(crate) fn keep_colors_by_default() -> bool {
    true
}

pub(crate) fn fit_paths_by_default() -> bool {
    true
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

/// Default alien-map frequency: one full cycle across the input range.
pub(crate) fn unit_frequency() -> f32 {
    1.0
}

/// Default alien-map per-channel enable.
pub(crate) fn enabled() -> bool {
    true
}

/// Which three channels alien-map's parameters address.
///
/// Upstream offers exactly these two, labelled "RGB color model" and "HSL color model" in the
/// vendored translation catalogues, which is also where the per-channel labels come from -- the
/// same three sliders read "red/green/blue" or "hue/saturation/luminosity" depending on this.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlienMapModel {
    /// Remap the stored red, green and blue channels independently.
    #[default]
    Rgb,
    /// Remap hue, saturation and lightness instead.
    Hsl,
}

/// What color-rotate does with a pixel too desaturated to have a meaningful hue.
///
/// Upstream's two radio labels are "Treat as this" and "Change to this", and the names carry their
/// own meaning: one lends the grey a colour and then processes it normally, the other simply
/// replaces it. That reading is the labels', not an invention -- but it IS a reading, so it is
/// written down here rather than left implicit.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrayMode {
    /// "Treat as this": give the grey the configured hue and saturation, then rotate it like any
    /// other pixel -- so it only changes further if that hue falls inside the source arc.
    #[default]
    TreatAsThis,
    /// "Change to this": replace the grey with the configured hue and saturation outright, with no
    /// rotation applied.
    ChangeToThis,
}

/// Default `opacity_threshold`: the far end of the range, so the ramp spans everything below it.
pub(crate) fn unit_threshold() -> f32 {
    1.0
}

/// A single colour component that [`Filter::ComponentExtract`] can render as a mono image.
///
/// Covers the whole colour vocabulary GIMP's own code works in, read off the `babl_format` strings
/// in `app/`: RGB, HSL, HSV, CIE Lab, CIE LCh(ab), CIE Yu'v', CIE xyY, CMYK and Y. The four spaces
/// beyond Lab were added in the cycle after this filter landed, once their conversions existed in
/// `color.rs` -- the enum was never the obstacle.
///
/// One limit remains and is deliberate: CMYK is the UNPROFILED separation, which is upstream's own
/// fallback when no ICC profile is set. A profiled separation is not derivable from RGB.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorComponent {
    /// The stored red channel.
    #[default]
    Red,
    Green,
    Blue,
    /// Opacity as a mono image, which is the only way to SEE a mask without applying it.
    Alpha,
    /// HSV hue, as a fraction of a turn so the result is displayable in 0..255. Note a grey has no
    /// hue; it renders as 0, which is red's position and not a meaningful value -- unavoidable
    /// when the component is undefined and the output is one byte.
    Hue,
    /// HSV saturation.
    Saturation,
    /// HSV value, i.e. the largest channel.
    Value,
    /// HSL lightness, `(max + min) / 2`. Deliberately distinct from [`Self::Value`]: they differ
    /// for every colour that is not a pure tint, and offering only one of them would quietly
    /// deny the other.
    Lightness,
    /// Rec. 709 relative luminance, the `Y` of GIMP's `"Y float"` formats. Not the mean of the
    /// channels -- pure green is 182, not 85.
    Luminance,
    /// CIE Lab lightness, which is PERCEPTUAL and therefore not [`Self::Luminance`]: L* applies a
    /// cube-root-like curve, so mid-grey sits near 50 of 100 where relative luminance puts it near
    /// 22 of 100.
    LabLightness,
    /// CIE Lab a*, the green-to-red axis, offset into 0..255 for display since it is signed.
    LabA,
    /// CIE Lab b*, the blue-to-yellow axis, likewise offset.
    LabB,
    /// CIE LCh(ab) chroma -- the distance from neutral, i.e. how colourful rather than how light.
    /// Built on Lab, not Luv; the two disagree and GIMP's format says `LCH(ab)`.
    LchChroma,
    /// CIE LCh(ab) hue angle, scaled from degrees into a byte. Perceptually spaced, unlike
    /// [`Self::Hue`], which is the HSV wheel.
    LchHue,
    /// CIE 1976 u', the horizontal axis of the uniform chromaticity scale.
    ///
    /// The 1976 form, settled from vendored source: GIMP's colour frame labels its readouts `u'`
    /// and `v'` under the context "Yu'v' color space", and the prime marks are 1976 notation. The
    /// 1960 form differs in one coefficient and would be wrong by a factor of 1.5 on v alone.
    YuvU,
    /// CIE 1976 v'.
    YuvV,
    /// CIE xyY chromaticity x.
    XyyX,
    /// CIE xyY chromaticity y.
    XyyY,
    /// Device CMYK cyan, from the UNPROFILED separation.
    ///
    /// Upstream resolves CMYK through an ICC profile and falls back to what it calls
    /// "No CMYK Profile (Default Values)"; the profiled path needs littleCMS, which is not
    /// vendored, so these four are upstream's own last resort rather than a press-ready plate.
    CmykCyan,
    /// Device CMYK magenta, unprofiled. See [`Self::CmykCyan`].
    CmykMagenta,
    /// Device CMYK yellow, unprofiled. See [`Self::CmykCyan`].
    CmykYellow,
    /// Device CMYK key (black), unprofiled. See [`Self::CmykCyan`].
    CmykKey,
}

/// Default mono-mixer gain: an equal share, so an unset filter is a plain average rather than a
/// black image (which three zero gains would give) or a triple-bright one (which three ones would).
pub(crate) fn third() -> f32 {
    1.0 / 3.0
}

/// 1, the shallowest meaningful recursion depth.
pub(crate) fn one_iteration() -> u32 {
    1
}

/// 100.0, the neutral value of a field upstream declares as a PERCENTAGE.
pub(crate) fn percent_hundred() -> f64 {
    100.0
}

/// 1.0, for a unit-range field whose neutral value is the top of the range.
pub(crate) fn unit_one() -> f64 {
    1.0
}

/// 0.5 on the f64 unit scale. Distinct from `half`, which is f32 -- the two scales are not
/// interchangeable and the compiler is what caught the mix.
pub(crate) fn unit_half() -> f64 {
    0.5
}

/// Grid spacing for both superpixel operations. Ours -- nothing upstream states one.
pub(crate) fn default_cluster_size() -> u32 {
    32
}

/// SLIC's colour-against-space weight. Ours; the published algorithm's own examples use 10.
pub(crate) fn default_compactness() -> f64 {
    10.0
}

/// SLIC converges in a handful of passes, so ten is past the useful range without being slow.
pub(crate) fn default_slic_iterations() -> u32 {
    10
}

/// Waterpixels' gradient-against-grid weight. Ours.
pub(crate) fn default_regularization() -> f64 {
    1.0
}

/// One maze unit, in pixels. Ours -- upstream's own default is not in the strings.
pub(crate) fn default_maze_cell() -> u32 {
    5
}

/// grid.c's own defaults, read from its argument declarations.
pub(crate) fn default_grid_width() -> u32 {
    1
}

pub(crate) fn default_grid_space() -> u32 {
    16
}

pub(crate) fn default_grid_offset() -> u32 {
    8
}

pub(crate) fn default_intersection_space() -> u32 {
    2
}

pub(crate) fn default_intersection_offset() -> u32 {
    6
}

/// Spiral's reference radius, in pixels. Ours -- the propgui derives it from a drag, so no
/// default is stated.
pub(crate) fn default_spiral_radius() -> f64 {
    64.0
}

/// Growth per turn. 2.0 doubles the arm spacing each turn, and upstream's range starts at 1.
pub(crate) fn default_spiral_base() -> f64 {
    2.0
}

/// Sinus scale and complexity. Ours; the plug-in that declared them is gone.
pub(crate) fn default_sinus_scale() -> f64 {
    0.05
}

pub(crate) fn default_sinus_complexity() -> f64 {
    2.0
}

/// Pixels per cycle for the linear sinusoid. Ours -- nothing upstream states one.
pub(crate) fn default_sinusoid_period() -> f64 {
    32.0
}

/// The familiar 4x4 Bayer matrix. Ours -- nothing upstream states an order.
pub(crate) fn default_bayer_order() -> u32 {
    2
}

/// Diffraction defaults. Ours -- the plug-in that declared them is deleted.
pub(crate) fn default_diffraction_frequency() -> f64 {
    0.815
}

pub(crate) fn default_diffraction_contours() -> f64 {
    0.819
}

pub(crate) fn default_diffraction_brightness() -> f64 {
    1.0
}

/// Pixels per noise lattice cell. Ours -- nothing upstream states a scale.
pub(crate) fn default_noise_scale() -> f64 {
    32.0
}

/// Opaque white, `Mosaic`'s default highlight.
pub(crate) fn white() -> Pixel {
    Pixel::rgba(255, 255, 255, 255)
}

/// Opaque black, `Mosaic`'s default shadow.
pub(crate) fn black() -> Pixel {
    Pixel::rgba(0, 0, 0, 255)
}

/// Krita's own default noise-reduction threshold, from `kis_simple_noise_reducer.cpp`.
pub(crate) fn krita_noise_threshold() -> u8 {
    15
}

/// Krita's own default noise-reduction window size.
pub(crate) fn krita_noise_window() -> u32 {
    1
}

/// The distance metric bounding [`Filter::FocusBlur`]'s sharp region.
///
/// These are `GimpLimitType`'s five values, read from the display enums rather than invented. The
/// metric each one implies follows from its name, and three of them already have precedent in this
/// crate: Euclidean from the gradient work, Chebyshev from `color-to-alpha`, and the axis-aligned
/// pair from the band shapes.
/// `gegl:sinus`' radio pair at `plug-ins/common/sinus.c` lines 764 and 765, under the frame
/// `Calculation Settings` (729).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SinusPerturbation {
    /// Line 764, `_Ideal`. The sine sum taken as it stands.
    #[default]
    Ideal,
    /// Line 765, `_Distorted`. The sum fed back as a phase shift into itself.
    Distorted,
}

/// `gegl:sinus`' gradient radio at lines 909 to 911, under `Blend Settings` (896) on the `_Blend`
/// tab (933).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SinusBlend {
    /// Line 909, `L_inear`.
    #[default]
    Linear,
    /// Line 910, `Bili_near`. Folded, so the two colours meet twice per cycle.
    Bilinear,
    /// Line 911, `Sin_usoidal`. An S-curve, so the ends flatten.
    Sinusoidal,
}

/// The two spiral laws, read **verbatim** from an enum vendored inside
/// `app/propgui/gimppropgui-spiral.c` lines 40 to 44:
///
/// ```c
/// typedef enum
/// {
///   GEGL_SPIRAL_TYPE_LINEAR,
///   GEGL_SPIRAL_TYPE_LOGARITHMIC
/// } GeglSpiralType;
/// ```
///
/// Unusually, GEGL's own enum is copied into GIMP's tree, so the spelling is read rather than
/// reconstructed from a label.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpiralType {
    /// Arms a constant distance apart. The propgui gives this type **one** slider
    /// (`n_sliders = 1`), so `base` is not used by it.
    #[default]
    Linear,
    /// Arms whose spacing multiplies by `base` each turn. Two sliders.
    Logarithmic,
}

/// The two maze constructions, read **verbatim** from `plug-ins/maze/maze-dialog.c` lines 260 and
/// 261 — a radio pair under the frame labelled `Algorithm` at line 234.
///
/// The progress strings in `maze-algorithms.c` are algorithmic evidence of the same kind mosaic's
/// were: line 278 is "Constructing maze using Prim's Algorithm" and line 488 "Constructing
/// **tileable** maze using Prim's Algorithm". Two separate strings in the algorithms file means two
/// separate construction paths, so tileability is not a post-process applied to a finished maze.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MazeAlgorithm {
    /// Line 260, `Depth first`. Randomised depth-first search with backtracking: long winding
    /// corridors and comparatively few dead ends.
    #[default]
    DepthFirst,
    /// Line 261, `Prim's algorithm`. Grow the tree from a random frontier wall each step: many
    /// short branches and many more dead ends.
    Prim,
}

/// Which component of the image gradient `gegl:image-gradient` writes out.
///
/// The gradient is a VECTOR field, and the name says which quantity without saying which part of it
/// you want — the cycle-67 shape, where a name gives the mechanism and withholds the configuration.
/// A vector in the plane has exactly two components to ask for, so the set is checkable rather than
/// asserted.
///
/// A third "both" value was considered and NOT shipped: packing two quantities into one raster needs
/// a convention for which channel carries which, and nothing in the name or the catalogue gives one.
/// **Absent is more honest than guessed**, as with spherize.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GradientOutput {
    /// How steeply the image changes here.
    #[default]
    Magnitude,
    /// Which way it changes, as an angle over the full turn.
    Direction,
}

/// Which distance `gegl:distance-transform` measures.
///
/// "Distance" does not say which distance, and that is the clearest degree of freedom the name
/// leaves open. GIMP's tree gives nothing here — the operation is GEGL-native and the action entry
/// is the whole of source 3 — so the **set** is corroborated from the fifth source, Krita, whose
/// `DistanceMetric` enum is `Chessboard`, `CityBlock`, `Euclidean`
/// (`plugins/filters/propagatecolors/KisPropagateColorsFilterConfiguration.h:20`).
///
/// Said plainly, as the fifth-source rule requires: the PURPOSE is GIMP's, the metric VOCABULARY is
/// Krita's. And Krita's file is not a distance transform — it is a colour-propagation filter that
/// uses one — so what is borrowed is the three-way metric choice and its geometry, nothing else.
/// Checked against our own `FILTER_NAMES` first: we ship no distance filter, so this duplicates
/// nothing.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DistanceMetric {
    /// Straight-line distance. Krita's `Euclidean`.
    #[default]
    Euclidean,
    /// Steps along the axes only. Krita's `CityBlock`, whose own tooltip is "Expand the colors in a
    /// diamond-like way" — which is the shape this metric's unit ball has.
    Manhattan,
    /// The larger of the two axis distances, so the unit ball is a square. Krita's `Chessboard`.
    Chebyshev,
}

/// The eight modes of `gegl:value-propagate`, read **verbatim** from
/// `plug-ins/common/value-propagate.c` lines 189 to 210, in dialog order.
///
/// Two of them are named in upstream's own action strings as well — `(mode white)` for Dilate and
/// `(mode black)` for Erode — so the enum's spelling is read rather than invented.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PropagateMode {
    /// Line 189, `More _white (larger value)`. Upstream's Dilate preset.
    #[default]
    White,
    /// Line 192, `More blac_k (smaller value)`. Upstream's Erode preset.
    Black,
    /// Line 195, `_Middle value to peaks`.
    MiddleToPeaks,
    /// Line 198, `_Foreground to peaks`.
    ForegroundToPeaks,
    /// Line 201, `O_nly foreground`.
    OnlyForeground,
    /// Line 204, `Only b_ackground`.
    OnlyBackground,
    /// Line 207, `Mor_e opaque`.
    MoreOpaque,
    /// Line 210, `More t_ransparent`.
    MoreTransparent,
}

/// What `gegl:apply-lens` leaves outside the lens — the radio group at lines 429 to 460 of
/// `plug-ins/common/lens-apply.c`.
///
/// **Three options, four strings.** Lines 444 and 445 — `_Set surroundings to index 0` and
/// `_Set surroundings to background color` — are one radio button whose label depends on whether
/// the image is indexed. Counting strings would have given four values and a parameter upstream
/// does not have.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LensSurroundings {
    /// Line 429 — leave the image outside the lens as it was.
    #[default]
    Keep,
    /// Lines 444/445 — palette index 0 on an indexed document, the given background colour
    /// otherwise, exactly as upstream's conditional label says.
    Background,
    /// Line 460.
    Transparent,
}

/// Which way `gegl:shift` displaces its lines.
///
/// A parameter rather than a default, because nothing privileges either axis — unlike
/// [`Filter::Spherize`]'s radial geometry, where "sphere" settles it. See [`Filter::Shift`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShiftAxis {
    /// Displace each row along x.
    #[default]
    Horizontal,
    /// Displace each column along y.
    Vertical,
}

/// Which of `gegl:illusion`'s two modes to use.
///
/// Upstream labels them only `Mode _1` and `Mode _2` — the radio pair at lines 399 and 414 of
/// `plug-ins/common/illusion.c` — so the labels carry no meaning at all and the distinction had to
/// be reasoned out rather than read. See [`Filter::Illusion`] for the reasoning and for why the
/// obvious guess is provably wrong.
/// `gegl:mosaic`'s tiling primitive — the four values of its `_Tiling primitives:` combo.
///
/// Read from `plug-ins/common/mosaic.c` lines 631–634, in that order, so unlike
/// [`IllusionMode`]'s anonymous pair these labels say exactly what they are.
/// `gegl:wind`'s "Style" radio group — lines 923 and 924.
///
/// Two genuinely separate renderers, which the plug-in's own progress strings confirm: it reports
/// "Rendering wind" at line 444 and "Rendering blast" at line 314, from different code.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindStyle {
    /// Line 923 — long streaks of varying length that fade out.
    #[default]
    Wind,
    /// Line 924 — short bursts of uniform length that do not fade.
    Blast,
}

/// `gegl:wind`'s "Direction" radio group — lines 947 and 948.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindDirection {
    /// Line 947.
    Left,
    /// Line 948.
    #[default]
    Right,
}

/// `gegl:wind`'s "Edge Affected" radio group — lines 971, 972 and 973.
///
/// Which side of a detected edge is smeared. `Both` is exactly the union of the other two, which is
/// what having three options where one is named "Both" means — and it is asserted as a union rather
/// than implemented separately, so the three can never drift apart.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindEdge {
    /// Line 971.
    Leading,
    /// Line 972.
    Trailing,
    /// Line 973.
    #[default]
    Both,
}

/// `gegl:tile-paper`'s "Fractional Pixels" radio group — lines 325, 327, 329.
///
/// What to do with the partial tiles left when the image is not an exact multiple of the tile size.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FractionalPixels {
    /// Line 325 — fill the remainder with the background.
    #[default]
    Background,
    /// Line 327 — leave the remainder as it was.
    Ignore,
    /// Line 329 — treat the remainder as a tile of its own and slide it too.
    Force,
}

/// `gegl:tile-paper`'s "Background Type" radio group — lines 385 to 395.
///
/// What shows through where a tile has slid away. Six options, and the first two of the colour ones
/// read the application's palette upstream; see [`Filter::TilePaper`] for why ours are explicit.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaperBackground {
    /// Line 385.
    Transparent,
    /// Line 387 — the original image, inverted.
    InvertedImage,
    /// Line 389 — the original image, unchanged, so the gaps do not read as holes.
    #[default]
    Image,
    /// Line 391.
    ForegroundColor,
    /// Line 393.
    BackgroundColor,
    /// Line 395, `S_elect here:`, whose colour picker is titled "Background Color" at line 402.
    Selected,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TilingPrimitive {
    /// Line 631.
    #[default]
    Squares,
    /// Line 632.
    Hexagons,
    /// Line 633, "Octagons & squares" — the only primitive with two cell shapes.
    OctagonsAndSquares,
    /// Line 634.
    Triangles,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IllusionMode {
    /// Pure rotation: each copy is the image turned about the centre.
    #[default]
    One,
    /// Reflection then rotation: each copy is the mirrored image turned about the centre.
    Two,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FocusShape {
    /// Euclidean distance — an ellipse once the aspect ratio is applied.
    #[default]
    Circle,
    /// Chebyshev distance, `max(|dx|, |dy|)` — a rectangle.
    Square,
    /// Manhattan distance, `|dx| + |dy|` — a rhombus.
    Diamond,
    /// Vertical distance only, so the sharp region is a horizontal BAND spanning the full width.
    Horizontal,
    /// Horizontal distance only — a vertical band.
    Vertical,
}

/// Upstream's default for colorize's hue and saturation.
pub(crate) fn half() -> f32 {
    0.5
}
