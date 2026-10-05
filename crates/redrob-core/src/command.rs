// SPDX-License-Identifier: GPL-3.0-or-later

use serde::{Deserialize, Serialize};

use crate::{
    BlendMode, DocumentMetadata, FrameId, LayerId, NodeId, Pixel, Rect, SelectionMode, Shape,
    TextContent, VectorContent, VectorPath,
};

/// How the convolution reads past the layer edge, READ from `convolution-matrix.c`'s po strings
/// `E_xtend`, `_Wrap` and `Cro_p` under the frame label `Border`.
///
/// Two of the three coincide exactly with `EdgePolicy` variants this crate already has, and are
/// implemented through it rather than reimplemented. `Crop` has no `EdgePolicy` equivalent -- it
/// declines to convolve the border at all -- so the sets genuinely differ and a separate enum with
/// the three READ names is honest. Cycle 79's equal-by-construction rule applies when the object is
/// the same; here it is not.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConvolutionBorder {
    /// `E_xtend` — the nearest edge sample repeats. `EdgePolicy::Clamp`.
    #[default]
    Extend,
    /// `_Wrap` — the layer tiles. `EdgePolicy::Wrap`.
    Wrap,
    /// `Cro_p` — border pixels are left as they were, not convolved.
    Crop,
}

/// Which scan field deinterlace treats as the real data.
///
/// The variant ORDER is GIMP's, read from `deinterlace.c`'s po strings: `Keep o_dd fields` is line
/// **356** and `Keep _even fields` is **357**. That order is load-bearing — it decides what an
/// integer in a saved document means — and it is unchanged.
///
/// # K.17f: the DEFAULT moved to `Even`, and the reason is the sharpest case in this item
///
/// The old comment here said *"odd comes first and is the default — the same declaration-order
/// reading that fixed `VideoPattern` and `FocusShape`"*. **The two projects order these two options
/// OPPOSITELY.** GEGL's own enum is
///
/// ```c
/// enum_value (GEGL_DEINTERLACE_KEEP_EVEN, "even", N_("Keep even fields"))
/// enum_value (GEGL_DEINTERLACE_KEEP_ODD,  "odd",  N_("Keep odd fields"))
/// ```
///
/// and it declares `GEGL_DEINTERLACE_KEEP_EVEN` as the default. So reading the order off GIMP's
/// dialog and reading it off GEGL's enum give **opposite answers**, and which one you get depends
/// only on which file you happened to open.
///
/// That is why the inference is unsafe rather than merely unlucky, and the precedent the comment
/// cited has now failed twice: cycle 33 found `VideoPattern`'s order-derived default wrong in the
/// same way. **Order is read; the default is declared. They are two facts.**
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeinterlaceField {
    /// `Keep o_dd fields` (line 356). Odd-numbered rows are the data; even rows are rebuilt.
    Odd,
    /// `Keep _even fields` (line 357).
    ///
    /// K.17f: the type's `Default` moved here from `Odd`, matching
    /// `GEGL_DEINTERLACE_KEEP_EVEN`. The variant order above is untouched.
    #[default]
    Even,
}

/// The nine video patterns, READ from `plug-ins/common/video.c`'s po strings at lines **42 to 50**.
///
/// Nine strings at nine consecutive line numbers are a single enum table in declaration order, which
/// is the same kind of reading that fixed `FocusShape`'s order from `display-enums.h`. The ORDER is
/// therefore read, not chosen -- it decides what an integer in a saved document means.
///
/// What is NOT readable is the exact cell layout behind each name: `video.c` itself is gone from the
/// tree, so only the names survive. Each variant's geometry is a recorded CHOICE guided by its own
/// name, and the names carry real information -- `Striped` and `3x3` are unambiguous, while the
/// three staggered forms differ in a way no surviving source states.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoPattern {
    /// `_Staggered` (line 42).
    Staggered,
    /// `_Large staggered` (43).
    LargeStaggered,
    /// `S_triped` (44) -- the channel depends on the column only, so every column is uniform.
    /// K.17f: the type's `Default` moved here from `Staggered`, matching upstream's
    /// `GEGL_VIDEO_DEGRADATION_TYPE_STRIPED`.
    #[default]
    Striped,
    /// `_Wide-striped` (45).
    WideStriped,
    /// `Lo_ng-staggered` (46).
    LongStaggered,
    /// `_3x3` (47).
    ThreeByThree,
    /// `Larg_e 3x3` (48).
    LargeThreeByThree,
    /// `_Hex` (49).
    Hex,
    /// `_Dots` (50).
    Dots,
}

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
    /// Turns colours into shades of grey.
    ///
    /// # K.16: five modes, and the default diverges from upstream's
    ///
    /// Upstream declares `mode` with default `GIMP_DESATURATE_LUMINANCE`. This variant has always
    /// applied the space's luminance weights to the **sRGB-encoded bytes**, which is upstream's
    /// [`crate::DesaturateMode::Luma`] — a real setting, not a defect — so the field defaults to
    /// `Luma` and a saved `Grayscale` keeps its meaning.
    ///
    /// Same judgement as `Levels`' clamp flags and `Curves`' `trc`, and the opposite of
    /// `Threshold`'s `channel`: the test is whether our old behaviour matched ANY upstream
    /// configuration. It did.
    ///
    /// This was a UNIT variant before K.16. `{"kind":"grayscale"}` still deserialises, because the
    /// one field carries a serde default — checked, not assumed.
    Grayscale {
        #[serde(default)]
        mode: crate::command::DesaturateMode,
    },
    BrightnessContrast {
        brightness: i16,
        contrast: f32,
    },
    GaussianBlur {
        sigma: f32,
    },
    Threshold {
        /// The band's lower bound, inclusive.
        ///
        /// # K.16: a BAND, not a cut
        ///
        /// `gimpoperationthreshold.c` declares two properties, read verbatim:
        /// `GIMP_CONFIG_PROP_DOUBLE(..., "low", _("Low threshold"), NULL, 0.0, 1.0, 0.5, ...)` and
        /// the same for `"high"` with default `1.0`. The test is one line:
        ///
        /// ```c
        /// value = (value >= threshold->low && value <= threshold->high) ? 1.0 : 0.0;
        /// ```
        ///
        /// **Both bounds are inclusive**, and a band can keep the midtones while blacking out
        /// shadows AND highlights together — which a single cut point cannot express at all.
        ///
        /// # Why this is the first `serde(alias)` in the crate
        ///
        /// This field was called `threshold` and was the whole filter. Upstream's name is `low`, and
        /// leaving ours as `threshold` would permanently mis-name the lower bound of a band. The
        /// alias keeps every saved document loading unchanged while the field takes upstream's name,
        /// which is why a new attribute is worth it here rather than renaming or not renaming.
        ///
        /// The old behaviour is preserved exactly, with no departure from the serde-default rule:
        /// our single cut was `measured >= threshold`, and upstream's band with `high` at maximum is
        /// `measured >= low && measured <= 255`, which is the same test. So `high` defaults to 255.
        #[serde(alias = "threshold")]
        low: u8,
        /// The band's upper bound, inclusive. Defaults to 255, which makes the band equivalent to
        /// the single cut this variant used to be.
        #[serde(default = "crate::command::full_byte")]
        high: u8,
        /// Which quantity the threshold is applied to.
        ///
        /// # K.16, and a deliberate departure from the serde-default rule
        ///
        /// Upstream declares this as `g_param_spec_enum("channel", ..., GIMP_HISTOGRAM_VALUE)`, so
        /// its default is `Value` -- the **MAXIMUM** of red, green and blue.
        ///
        /// Our variant previously tested **Rec. 709 luminance**, which is upstream's behaviour for
        /// NEITHER the default channel nor the `Luminance` one (that uses GIMP's own weights). So the
        /// filter was wrong in two ways, and both were found by reading rather than by a gate.
        ///
        /// The project rule is that a new field on an existing command variant defaults to the
        /// behaviour the variant already had, so that saved documents do not change meaning. **This
        /// field deliberately does not**: it defaults to upstream's `Value`, which DOES change what
        /// an existing saved `Threshold` does. Keeping the old behaviour would have required
        /// defaulting to `Luminance` and would have enshrined the measured defect in the very filter
        /// this item exists to correct. The rule guards against ACCIDENTAL change; here the change is
        /// the correction.
        #[serde(default)]
        channel: crate::command::HistogramChannel,
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
        /// The red slot, applied BEFORE the five fields above. `None` is the identity.
        ///
        /// K.16. The five fields above keep their meaning as upstream's **overall** slot, which is
        /// exactly what this variant already did — it applied one mapping to R, G and B and left
        /// alpha alone. So the four new slots default to the identity and an existing saved `Levels`
        /// is unchanged. See [`LevelsSlot`] for the composition order and the alpha exclusion.
        #[serde(default)]
        red: Option<crate::command::LevelsSlot>,
        /// The green slot, applied before the overall one. `None` is the identity.
        #[serde(default)]
        green: Option<crate::command::LevelsSlot>,
        /// The blue slot, applied before the overall one. `None` is the identity.
        #[serde(default)]
        blue: Option<crate::command::LevelsSlot>,
        /// The alpha slot. `None` is the identity, and the overall slot is NEVER applied to alpha.
        #[serde(default)]
        alpha: Option<crate::command::LevelsSlot>,
        /// Clamp the normalised input to 0..1 before the gamma and output stages.
        ///
        /// # K.16, and why this is NOT on [`LevelsSlot`]
        ///
        /// The five scalars are per-channel arrays, but these two flags are **not**: the process
        /// loop passes `config->clamp_input` and `config->clamp_output` for every channel, read
        /// from the config rather than from an array. So they are **operation-wide** and belong on
        /// the variant, not on the slot.
        ///
        /// # The default diverges from upstream, deliberately
        ///
        /// Upstream declares `clamp-input` and `clamp-output` with default **FALSE**
        /// (`GIMP_CONFIG_PROP_BOOLEAN(..., FALSE, 0)`), while this variant has always clamped. Both
        /// default to `true` here so an existing saved `Levels` keeps its meaning, which is the
        /// project's rule for a new field on an existing command variant.
        ///
        /// That is a different judgement from `Threshold`'s `channel`, where the default was moved
        /// to upstream's. The difference is that threshold's old behaviour matched **no** upstream
        /// configuration at all, so preserving it would have enshrined a defect; clamping is
        /// upstream's behaviour with these flags set, so nothing here is wrong — only the default
        /// differs, and the parity requirement is that both behaviours be **expressible**, which
        /// they now are.
        #[serde(default = "crate::command::yes")]
        clamp_input: bool,
        /// Clamp the final output to 0..1. Operation-wide, like `clamp_input`, and defaulting to
        /// `true` for the same reason.
        ///
        /// Only observable when `clamp_input` is false, because with the input clamped the output
        /// stage maps 0..1 into `output_black..output_white`, which is already inside 0..1.
        #[serde(default = "crate::command::yes")]
        clamp_output: bool,
        /// Which colour space the levels mapping is applied in.
        ///
        /// Same enum, same inherited `prepare` and the same default reasoning as
        /// [`Filter::Curves`]' `trc`: upstream's declared default is [`crate::TrcType::Linear`],
        /// this variant has always mapped the sRGB-encoded bytes, so the field defaults to
        /// `NonLinear` and a saved `Levels` keeps its meaning.
        ///
        /// # What the bounds mean once a space can be chosen
        ///
        /// Upstream's `low-input`, `high-input`, `low-output` and `high-output` are 0..1 **of the
        /// working space**, not of sRGB. So in `Linear` mode the pixel is converted into linear
        /// light but the bounds are NOT: a bound of 128 means `128/255` as a linear coordinate,
        /// which is a brighter point than sRGB mid-grey. That is upstream's own meaning — its 0.5 in
        /// linear mode is linear 0.5 — and it is why the mapping had to move from 0..255 byte units
        /// to 0..1 before this field could be honest.
        #[serde(default = "crate::command::non_linear_trc")]
        trc: crate::command::TrcType,
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
        /// K.17f: was a bare default, i.e. `0.0`. Upstream declares `90.0` on
        /// `value_range (0.0, 360.0)`.
        ///
        /// **With `from` and `to` both 0.0 the source range is EMPTY**, so the filter had nothing
        /// to rotate by default — degenerate rather than merely different.
        #[serde(default = "crate::command::default_color_rotate_to")]
        source_to: f32,
        /// Start of the destination arc, in degrees.
        #[serde(default)]
        dest_from: f32,
        /// End of the destination arc, in degrees. A destination shorter than the source
        /// compresses the hues into it; a longer one spreads them out.
        /// K.17f: was a bare default, i.e. `0.0`. Upstream declares `90.0`, and the same
        /// emptiness argument applies to the destination range.
        #[serde(default = "crate::command::default_color_rotate_to")]
        dest_to: f32,
        /// How to treat pixels whose saturation is below `gray_threshold`.
        /// K.17f: was `TreatAsThis`. Upstream declares `GEGL_COLOR_ROTATE_GRAY_CHANGE_TO` —
        /// **its enum lists `TREAT_AS` first and it defaults to the second anyway**, the same
        /// deliberate non-first choice `gegl:wind`'s `edge` makes. Ours was its first value.
        #[serde(default = "crate::command::default_color_rotate_gray_mode")]
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
        /// K.17f: was 0.5. Upstream declares `0.75` with `ui_meta ("unit",
        /// "relative-distance")`, which is our convention too — `radius * width / 2.0` — so this is
        /// a plain default correction and not a domain difference.
        #[serde(default = "crate::command::default_focus_blur_radius")]
        radius: f32,
        /// Height-to-width ratio of the region. 1.0 is round.
        /// **NOT a K.17f correction, and the reason is worth keeping.** audit4 reported
        /// `upstream 0.0 vs ours 1.0`, which looks like a divergent default and is the same state.
        /// Upstream's `aspect_ratio` is a SIGNED BIAS on `value_range (-1.0, +1.0)` that its own
        /// code turns into a scale — `scale = 1.0 - ratio` when non-negative, `1.0 / (1.0 + ratio)`
        /// below zero — so its `0.0` is the neutral circle. Ours IS that scale, used as a divisor,
        /// so our neutral is `1.0`. Setting this to upstream's number would be refused by our own
        /// validation, which rejects `<= 0.0` because zero would divide by zero.
        #[serde(default = "crate::command::unit_threshold")]
        aspect_ratio: f32,
        /// Region rotation in DEGREES.
        #[serde(default)]
        rotation: f32,
        /// Fraction of the region that stays completely sharp, 0..1.
        /// K.17f: was a bare default, i.e. `0.0` — no sharp core at all, so the focus region
        /// ramped from its very centre. Upstream declares `0.25` on `value_range (0.0, 1.0)`.
        #[serde(default = "crate::command::default_focus_blur_focus")]
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
        /// K.17f: was a bare default, i.e. `Squares`. Upstream declares
        /// `GEGL_MOSAIC_TILE_HEXAGONS`, which is also GIMP's own dialog default.
        #[serde(default = "crate::command::default_mosaic_primitive")]
        primitive: crate::command::TilingPrimitive,
        /// Lattice step in pixels.
        tile_size: u32,
        /// Bevel depth. 0 is flat.
        ///
        /// K.17f: was a bare default, i.e. `0.0`. Upstream declares `4.0` — and its
        /// `value_range (1.0, 1000.0)` puts **0.0 outside the legal range entirely**, so the old
        /// value was not merely a different choice.
        #[serde(default = "crate::command::default_mosaic_tile_height")]
        tile_height: f64,
        /// Width of the grout between tiles, in pixels. 0 butts them together.
        ///
        /// K.17f: was a bare default, i.e. `0.0`. Upstream declares `1.0`. Unlike `tile_height`,
        /// zero IS inside upstream's `value_range (0.0, 1000.0)` — it was legal, just not default.
        #[serde(default = "crate::command::unit_one")]
        tile_spacing: f64,
        /// 1.0 is the exact lattice; 0.0 is fully irregular.
        ///
        /// K.17f: was `1.0` — a chosen helper, not a bare default, so this replaces a decision.
        /// Upstream declares `0.65`, which is why its tiles look hand-laid rather than ruled.
        #[serde(default = "crate::command::default_mosaic_neatness")]
        tile_neatness: f64,
        /// Direction the bevel is lit from, in **degrees** — the unit every angle in this crate
        /// carries in its name or its docs rather than being guessed at.
        /// K.17f: was a bare default, i.e. `0.0`. Upstream declares `135.0` with
        /// `ui_meta ("direction", "ccw")` — light from the upper left, the convention every bevel
        /// in this product already assumes.
        #[serde(default = "crate::command::default_mosaic_light_direction")]
        light_direction: f64,
        /// Per-tile colour jitter, 0.0 for none.
        ///
        /// K.17f: was a bare default, i.e. `0.0`. Upstream declares `0.2`.
        #[serde(default = "crate::command::default_mosaic_color_variation")]
        color_variation: f64,
        /// Supersample the tile and grout decision so cell edges are not stair-stepped.
        ///
        /// K.17f: was a bare default, i.e. `false`. Upstream declares `TRUE`.
        #[serde(default = "crate::command::yes")]
        antialiasing: bool,
        /// Take each tile's colour as the mean over the whole tile rather than the seed's own pixel.
        ///
        /// K.17f: was a bare default, i.e. `false`. Upstream declares `TRUE`.
        #[serde(default = "crate::command::yes")]
        color_averaging: bool,
        /// Split a tile where it would straddle an image contour — the flag the "Finding edges"
        /// phase exists to serve.
        ///
        /// K.17f: was a bare default, i.e. `false`. Upstream declares `TRUE` — so the edge-finding
        /// phase was being computed and then never acted on by default.
        #[serde(default = "crate::command::yes")]
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
        /// K.17f: was a bare default, i.e. `0.0` — and 0% movement means **no tile moves at all**,
        /// so the filter's entire effect was off by default. Upstream declares `25.0` on
        /// `value_range (1.0, 100.0)` with `ui_meta ("unit", "percent")`, which is our unit too:
        /// the body computes `move_max / 100.0 * tile_width`.
        ///
        /// Note upstream's range FLOOR is 1.0, so its own dialog cannot even express the 0 we
        /// defaulted to. Ours accepts `0.0..=100.0`; see K.17i.
        #[serde(default = "crate::command::default_tile_paper_move")]
        move_max: f64,
        /// A tile sliding off one edge reappears at the opposite one.
        #[serde(default)]
        wrap_around: bool,
        /// Centre the tile grid on the image instead of starting it at the origin.
        ///
        /// K.17f: was a bare default, i.e. `false`. Upstream declares `TRUE`.
        #[serde(default = "crate::command::yes")]
        centering: bool,
        /// What to do with the partial tiles at the far edges.
        ///
        /// K.17f: was `Background`, our enum's first variant. Upstream declares
        /// `GEGL_FRACTIONAL_TYPE_FORCE` — the **third and last** of its three, so this is the
        /// strongest case yet of upstream declining the first variant.
        #[serde(default = "crate::command::default_fractional_pixels")]
        fractional_pixels: crate::command::FractionalPixels,
        /// What shows through where a tile has slid away.
        ///
        /// K.17f: was `Image`, our third variant. Upstream declares
        /// `GEGL_BACKGROUND_TYPE_INVERT`, the **second** of its four — so the gaps a tile leaves
        /// show the image INVERTED rather than unchanged, which is what makes the slide visible.
        #[serde(default = "crate::command::default_paper_background")]
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
        /// K.17f: was a bare default, i.e. `0` — the lowest possible, so the effect was applied
        /// to the WIDEST set of areas. Upstream declares `10` on `value_range (0, 50)`, described
        /// as "Higher values restrict the effect to fewer areas", so 0 was the least restrictive
        /// end rather than a neutral middle.
        #[serde(default = "crate::command::default_wind_threshold")]
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
        ///
        /// # Where this came from — resolved cycle 119
        ///
        /// **Name-derived, and necessary rather than merely plausible.** `gegl:recursive-transform`
        /// names a recursion, and a recursion without a depth has no stopping condition, so the
        /// operation is ill-defined without one.
        ///
        /// The propgui is positive evidence that other properties exist without naming any: it
        /// skips exactly one, with `/* skip the "transform" property, which is controlled by a
        /// transform-grid */`, and hands everything else to the generic builder.
        ///
        /// **What is NOT readable is this parameter's upstream name or range.** `iterations` and the
        /// `>= 1` bound are ours; upstream's could be spelled `depth` or `count` and bounded
        /// differently. The existence is derived, the spelling is chosen, and the two are recorded
        /// separately because only the first is evidence.
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
        /// How many mirror lines pass through the centre. **This is upstream's `n_segs`** — the
        /// same parameter under an unrelated name, which is why audit4's mechanical classifier
        /// files it as a candidate gap rather than a rename (K.17).
        ///
        /// `n` lines divide the plane into `2n` wedges and give the result `n`-fold dihedral
        /// symmetry: it is unchanged by a rotation of `2π/n` and by reflection in each line.
        ///
        /// **Upstream's range is `(2, 24)` and its default 6.** One mirror line is not a
        /// kaleidoscope, so the floor of 1 this product allowed is a divergence and is raised; the
        /// ceiling stays ours, because 24 is a dialog convenience and nothing in the algorithm
        /// breaks above it.
        #[serde(default = "crate::command::default_mirrors")]
        mirrors: u32,
        /// `m_angle`, degrees in `0..=180`: rotation applied to the MIRROR LINES.
        ///
        /// Added back after the fold (`ang = ang + angle1`), so it turns the wedges themselves.
        #[serde(default)]
        mirror_angle: f64,
        /// `r_angle`, degrees in `0..=360`: rotation applied to the RESULT.
        ///
        /// Subtracted before the fold and never added back, which is exactly what makes it rotate
        /// the output rather than the mirrors.
        #[serde(default)]
        result_angle: f64,
        /// Where the fold's centre sits, as a fraction of the canvas. Upstream's `c_x`/`c_y`.
        ///
        /// **Upstream's NAMES and LABELS are crossed here, and following either alone gets one of
        /// the pair backwards.** `c_x` is labelled *"Offset X"* while its description says
        /// *"position of symmetry center in output"*, and `o_x` is labelled *"Center X"* while its
        /// description says *"X axis ratio for the center of mirroring"*. The code settles it:
        /// `c_x` becomes `cen_x`, the centre the angle is measured from. These names follow the
        /// code.
        #[serde(default = "crate::command::unit_half")]
        center_x: f64,
        #[serde(default = "crate::command::unit_half")]
        center_y: f64,
        /// Added to the SAMPLED coordinate, as a ratio in `-1..=1`. Upstream's `o_x`/`o_y`, which
        /// it passes as `off_x * input_scale`.
        #[serde(default)]
        offset_x: f64,
        #[serde(default)]
        offset_y: f64,
        /// Zoom, `0.1..=100`. **Upstream divides this by 100 at the call site**, so its default of
        /// 100.0 means a factor of 1.0 — a reader taking the property value directly scales by a
        /// hundred.
        #[serde(default = "crate::command::default_mirror_input_scale")]
        input_scale: f64,
        /// Whether a sample outside the input REFLECTS back in (`true`) or clamps to the edge.
        ///
        /// Upstream calls it "Wrap input" and defaults it TRUE, but it is not a modulo wrap: the
        /// parity of the overrun decides whether the coordinate mirrors or wraps, so a sample two
        /// widths out comes back the same way round.
        #[serde(default = "crate::command::yes")]
        warp: bool,
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
        ///
        /// K.17f: was a bare default, i.e. `false`. Upstream declares `TRUE` — a generated pattern
        /// that does not tile is the odd case, not the default one.
        #[serde(default = "crate::command::yes")]
        tiling: bool,
        /// The radio pair at 764/765.
        ///
        /// K.17f: was `Ideal`. Upstream's `perturbation` is a BOOLEAN labelled "Distorted" and
        /// defaults to `TRUE`, so our `Ideal` is its `FALSE`. The two-variant enum is kept — it
        /// comes from GIMP's own `_Ideal`/`_Distorted` radio pair and names both states, which a
        /// boolean does not — but the default now agrees.
        #[serde(default = "crate::command::default_sinus_perturbation")]
        perturbation: crate::command::SinusPerturbation,
        /// First colour, with its alpha from the `Alpha Channels` frame.
        ///
        /// K.17f: was black. Upstream declares `"yellow"`.
        #[serde(default = "crate::command::sinus_yellow")]
        color1: Pixel,
        /// Second colour.
        ///
        /// K.17f: was white. Upstream declares `"blue"`.
        #[serde(default = "crate::command::sinus_blue")]
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
        #[serde(default = "crate::command::default_diffraction_frequency_red")]
        frequency_red: f64,
        #[serde(default = "crate::command::default_diffraction_frequency_green")]
        frequency_green: f64,
        #[serde(default = "crate::command::default_diffraction_frequency_blue")]
        frequency_blue: f64,
        /// `Contours` tab, `_Red:`.
        #[serde(default = "crate::command::default_diffraction_contours_red")]
        contour_red: f64,
        #[serde(default = "crate::command::default_diffraction_contours_green")]
        contour_green: f64,
        #[serde(default = "crate::command::default_diffraction_contours_blue")]
        contour_blue: f64,
        /// `Sharp Edges` tab, `_Red:`.
        #[serde(default = "crate::command::default_diffraction_edges_red")]
        edges_red: f64,
        #[serde(default = "crate::command::default_diffraction_edges_green")]
        edges_green: f64,
        #[serde(default = "crate::command::default_diffraction_edges_blue")]
        edges_blue: f64,
        /// `Other Options` tab, `_Brightness:`.
        #[serde(default = "crate::command::default_diffraction_brightness")]
        brightness: f64,
        /// `Sc_attering:`.
        #[serde(default = "crate::command::default_diffraction_scattering")]
        scattering: f64,
        /// `Po_larization:`.
        #[serde(default = "crate::command::default_diffraction_polarization")]
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
    /// Bright areas spill light into their surroundings (K.7).
    ///
    /// `gegl:bloom`. Every source is empty but the action entry; the menu (cycle 78's eighth route)
    /// places it in `_Light and Shadow`'s **`Light`** section beside `supernova` and `lens-flare`
    /// (`menus/image-menu.ui.in.in:745`). That section also **confirms K.7's own grouping** rather
    /// than correcting it, which is a first after cycles 78 and 83 both overturned one — and it
    /// settles the decision K.7's header reserved: `dropshadow` and `long-shadow` are registered
    /// FILTERS in `filters-actions.c` and placed in the Filters menu, not layer styles.
    ///
    /// **Distinctness from `softglow`, which we already ship, is forced by the catalogue and took
    /// two attempts to state correctly.** Upstream describes softglow as "Simulate glow by making
    /// highlights intense and fuzzy" with `_Glow radius:`, `_Brightness:` and `_Sharpness:`, and our
    /// `SoftGlow` SCREENS a blurred copy of the whole image over the original.
    ///
    /// The first discriminator considered was that bloom is purely ADDITIVE, so no pixel may darken.
    /// That is true of bloom and **does not separate them**: screen is monotone too, so our softglow
    /// never darkens either. The invariant is still worth pinning; it is simply not the difference.
    ///
    /// The difference is the THRESHOLD. Softglow glows from every pixel, however dark, because it
    /// screens the whole blurred image. Bloom glows only from what is above the threshold, so **a
    /// flat field below it comes back untouched where softglow brightens it** — which is the paired
    /// assertion cycle 82's rule asks for, and is exactly what the threshold parameter buys.
    ///
    /// Three parameters, each ENTAILED: "bloom" says bright areas spill, which requires saying
    /// WHICH areas, HOW FAR and HOW MUCH. A soft knee on the threshold was considered and not
    /// shipped — not entailed, and absent is more honest than guessed.
    /// A shadow cast by the layer's own opaque shape, offset, blurred and drawn UNDERNEATH it.
    ///
    /// # What declares this, and the one thing that does not
    ///
    /// Source 1 exists here in a form this backlog had not used before: not
    /// `plug-ins/common/<name>.c` but `plug-ins/script-fu/scripts/drop-shadow.scm`. A Script-Fu
    /// script IS the plug-in's own source, so it outranks the po file, and it hands over every
    /// range and default directly:
    ///
    /// | argument | default | range |
    /// |---|---|---|
    /// | `Offset X` | 4 | -4096..4096 |
    /// | `Offset Y` | 4 | -4096..4096 |
    /// | `Blur radius` | 15 | **0**..1024 |
    /// | `Color` | black | — |
    /// | `Opacity` | 60 | 0..100 |
    ///
    /// Two things are deliberately NOT taken from it.
    ///
    /// `Allow resizing` is the script's sixth argument and is not a pixel parameter: it grows the
    /// IMAGE so an offset shadow is not clipped. Our `ApplyFilter` works in place on a fixed canvas
    /// and cannot resize it, so the shadow is simply clipped at the edge. Same shape as the
    /// dialog-state exclusions -- a host concern that happens to arrive through the argument list.
    ///
    /// And the script is registered as `_"_Drop Shadow (legacy)..."` while the menu action points at
    /// `gegl:dropshadow`. By source 7, upstream shipping BOTH means they are two things, so the
    /// script's arguments are evidence for the CONCEPT and not a claim about the GEGL operation's
    /// contract. GEGL is not vendored, so whatever that operation adds is unreadable here -- and
    /// absent is more honest than guessed.
    ///
    /// # Why `radius` may be 0 when `validate_radius` refuses it
    ///
    /// The script's own range is `0 1024`, and it gates the blur with `(if (>= shadow-blur 1.0) ...)`
    /// -- so radius 0 is a legal request for a HARD-EDGED shadow, not an error. A read range
    /// outranks our convention, as it already does for noise-reduction's `window_size` and grid's
    /// line widths.
    ///
    /// # The one reading that is NOT transferable
    ///
    /// The script blurs with `gegl:gaussian-blur` at `std-dev = 0.32 * radius`. That factor is a
    /// GAUSSIAN STANDARD DEVIATION, and our blur is a box blur whose `radius` is a window extent --
    /// a different unit. Multiplying by 0.32 anyway would be the cycle-57 bug again, where a value
    /// was compared in squared-distance units because it looked like the right number. So the read
    /// fact used here is the one that survives the unit change: blur scales linearly with `radius`,
    /// and 0 means none.
    ///
    /// # Why it needs alpha
    ///
    /// `filters-actions.c` gates it on `writable && alpha`, which exactly four filter actions
    /// require -- this, `long-shadow`, `semi-flatten` and `threshold-alpha`. The shadow's SHAPE is
    /// the alpha channel, so on a fully opaque layer the shadow is completely hidden behind the
    /// layer that cast it and the filter is a no-op.
    /// A shadow EXTRUDED from the layer's opaque shape along one direction, for a given distance.
    ///
    /// # Every source is empty, which has happened once before
    ///
    /// There is no `plug-ins/common/<name>.c`, no Script-Fu script, no propgui, no config object and
    /// no po string -- the same position as `linear-sinusoid` in cycle 78. What exists:
    ///
    /// - `filters-actions.c` gives `_Long Shadow...`, so at least one parameter.
    /// - `menus/image-menu.ui.in.in` puts it in `_Light and Shadow`.
    /// - The action is gated `writable && alpha`, which exactly four filter actions require, so the
    ///   shadow's SHAPE is the alpha channel -- the same reading as drop shadow.
    /// - `desktop/org.gimp.GIMP.appdata.xml.in.in` announces `New "Long Shadow" filter` and nothing
    ///   more, so it corroborates the name and no parameter.
    ///
    /// **Krita has no long-shadow filter at all**, so source 6 is unavailable here and is reported
    /// as such rather than quietly skipped.
    ///
    /// # So the name is the specification, and it leaves exactly three freedoms
    ///
    /// "Long shadow" is a named effect, not a description, which puts it in the same class as
    /// `polar-coordinates` and `distance-transform`: the contract IS the definition. What it
    /// entails:
    ///
    /// - It is LONG, so a `length` -- the one thing the adjective explicitly leaves open, and the
    ///   ellipsis proves at least one parameter exists.
    /// - A shadow falls one way, so an `angle`. Unlike drop shadow this is an angle and not an x/y
    ///   pair, because the effect is a directional EXTRUSION rather than a displacement, and a
    ///   swept path is specified by its direction.
    /// - A shadow has a `color`.
    ///
    /// There is deliberately **no separate opacity**. Drop shadow needed one because its script
    /// declared one, but here nothing does, and `color` is a `Pixel` whose own alpha already carries
    /// it -- an opacity parameter would be a second control over one quantity.
    ///
    /// Anything further a real `gegl:long-shadow` may declare is unreadable: GEGL is not vendored.
    /// Absent is more honest than guessed, and the possibility that upstream declares more is filed
    /// beside K.16's other property gaps rather than invented here.
    ///
    /// # Why it cannot be drop shadow with different numbers
    ///
    /// Source 7: upstream ships BOTH `gegl:dropshadow` and `gegl:long-shadow`, so they must differ,
    /// and two names in one catalogue mean two mechanisms. Drop shadow DISPLACES one copy of the
    /// alpha shape; long shadow fills the ENTIRE SWEPT PATH. The consequence is testable: at a
    /// distance greater than the shape's own size, drop shadow leaves a gap between caster and
    /// shadow while long shadow cannot.
    ///
    /// `angle`'s default of 45 degrees is a recorded CHOICE, not a reading -- it is the direction
    /// that makes the effect recognisable, and no source states one. Same for `length`'s default.
    /// A bright core at a chosen point plus its ghost reflected through the image centre.
    ///
    /// # Source 2 gives the WHOLE parameter list, and it is two
    ///
    /// `plug-ins/common/lens-flare.c` is referenced by the po file but deleted from the tree, which
    /// is the normal case the source list assumes: upstream removes a plug-in once its GEGL
    /// operation lands. So the strings are the route, and read as a whole block they are complete:
    ///
    /// | line | string | what it is |
    /// |---|---|---|
    /// | 183 | `Add a lens flare effect` | blurb |
    /// | 190 | `Lens _Flare...` | menu label, so ≥1 parameter |
    /// | 265 | `Render lens flare` | progress string |
    /// | 301 | `Lens Flare` | dialog title |
    /// | 745 | `Center of Flare Effect` | frame label |
    /// | 764 | `_X:` | parameter |
    /// | 769 | `_Y:` | parameter |
    /// | 785 | `Show _position` | **dialog state, not a parameter** |
    ///
    /// The frame label says what the two numbers mean, so `x` and `y` are the flare's centre in
    /// pixels and there is nothing else to read.
    ///
    /// `Show _position` is shared with exactly one other file -- `nova.c`, which is `supernova`, the
    /// next item in this group -- and it toggles a crosshair in the PREVIEW. That makes it the sixth
    /// dialog-state exclusion, and the first that is a preview control rather than a question about
    /// frame layout.
    ///
    /// # Why two parameters is the right reading rather than a gap
    ///
    /// Source 7, in its strongest form yet: `plug-ins/gradient-flare/gradient-flare.c` is **still
    /// present and still a plug-in**, never converted to a GEGL operation. So upstream ships a
    /// configurable flare alongside this one, and the neighbour's continued existence EXPLAINS the
    /// short list -- this is the fixed, canonical flare, and its structure belongs in code rather
    /// than in arguments. A rich parameter set here would make `gradient-flare` redundant.
    ///
    /// Unlike the two shadow filters this action is NOT gated on `writable && alpha`: a flare adds
    /// light to whatever is there, so it needs no shape to read.
    ///
    /// # What the name fixes, and what is a recorded choice
    ///
    /// "Lens flare" names an optical fact, not an appearance: light bouncing between lens elements
    /// reappears MIRRORED THROUGH THE OPTICAL AXIS. That is what separates it from a glow, and it is
    /// the one part of the structure that is derivable rather than chosen -- the ghost sits at
    /// `(2*cx - x, 2*cy - y)`, exactly, with no constant to pick.
    ///
    /// The core and ghost radii, and the ghost's relative brightness, are recorded CHOICES scaled to
    /// the canvas, because nothing readable states them.
    /// A starburst: a bright core with `spokes` rays radiating from a point.
    ///
    /// # Two sources checking each other, and the units come from the better one
    ///
    /// `app/propgui/gimppropgui-supernova.c` exists and names three properties BY NAME, which is
    /// better than any string -- and it states their UNITS in its own arithmetic:
    ///
    /// ```text
    /// x      = x1 / area->width;                      // center-x is NORMALISED
    /// y      = y1 / area->height;                     // center-y is NORMALISED
    /// radius = sqrt (SQR (x2 - x1) + SQR (y2 - y1));  // radius is in PIXELS
    /// ```
    ///
    /// That is the `spiral` situation from cycle 76 -- position normalised to the area while the
    /// radius is a pixel distance -- except here it is READ rather than deduced from a wrong band
    /// count. The controller is a LINE whose first point is the centre and whose second is
    /// `(x1 + radius, y1)`, so the line's length IS the radius, confirming the same thing twice.
    ///
    /// The propgui delegates everything else to `_gimp_prop_gui_new_generic`, so the remaining
    /// properties come from `plug-ins/common/nova.c`'s strings, whose line numbers give dialog order:
    ///
    /// | line | string | parameter |
    /// |---|---|---|
    /// | 163 | `Add a starburst to the image` | names the MECHANISM |
    /// | 349 | `Co_lor:` | `color` |
    /// | 362 | `_Radius:` | `radius` |
    /// | 374 | `_Spokes:` | `spokes` |
    /// | 389 | `R_andom hue:` | `random_hue` |
    /// | 437 | `Center of Nova` | frame label |
    /// | 454 | `_X:` | `center_x` |
    /// | 459 | `_Y:` | `center_y` |
    /// | 475 | `Show _position` | **dialog state, NOT a parameter** |
    ///
    /// The frame label at 437 precedes its own widgets at 454 and 459, so the cycle-80 caveat about
    /// a label arriving AFTER its page does not bite here -- checked rather than assumed.
    ///
    /// `Show _position` is the same preview crosshair as lens flare's, and those two files are the
    /// only ones that share the string. Seventh dialog-state exclusion.
    ///
    /// # No seed, deliberately
    ///
    /// `R_andom hue:` implies randomness, and nothing readable declares a seed. This codebase's
    /// standing invariant is that jitter is a HASH OF AN INDEX rather than a PRNG -- so the hue
    /// offset is hashed from the spoke's own index, which needs no seed and replays identically.
    /// Maze is the one recorded departure from that invariant, and it had to be; this does not.
    ///
    /// Hue is in DEGREES, consistent with `rgb_to_hsv` throughout this crate. The range is ours; the
    /// string gives no bound.
    /// Darkens toward the edges of a shaped, rotatable, squeezable region.
    ///
    /// # The richest propgui read in this work, and every relation is stated TWICE
    ///
    /// `app/propgui/gimppropgui-vignette.c` names **nine** properties by name, and states each
    /// one's unit in its own arithmetic. `focus_callback` writes them and `config_notify` reads them
    /// back, and the two are exact inverses -- so the readings PROVE EACH OTHER rather than being
    /// one interpretation of one direction. That is the standard panorama-projection set in cycle
    /// 81, here at nine properties instead of two:
    ///
    /// | property | written as | read back as |
    /// |---|---|---|
    /// | `x` | `x / area->width` | `x * area->width` |
    /// | `y` | `y / area->height` | `y * area->height` |
    /// | `radius` | `2.0 * radius / area->width` | `radius * area->width / 2.0` |
    /// | `rotation` | `fmod(angle * 180/PI, 360)` normalised | `rotation / 180.0 * G_PI` |
    /// | `softness` | `1.0 - inner_limit` | `1.0 - softness` |
    /// | `gamma` | `log(0.5) / log(midpoint)` | `pow(0.5, 1.0 / gamma)` |
    /// | `squeeze` | `±2/PI * atan(...)` | `tan(±squeeze * PI/2)` |
    ///
    /// So: `x` and `y` are NORMALISED to the canvas, and `radius` is normalised to the WIDTH and
    /// DOUBLED -- it is a diameter fraction, not a pixel distance. That makes vignette the opposite
    /// of supernova, where the centre was normalised but the radius was pixels.
    ///
    /// # Three ranges that are derived or read rather than chosen
    ///
    /// - `squeeze` is `±2/PI * atan(x)` with `x > 0`, and `atan` maps that to `(0, PI/2)`, so the
    ///   range is exactly `(-1, 1)`. **Derived from the formula**, not picked.
    /// - `rotation` is in DEGREES over `0..360`, because the writer applies
    ///   `fmod(fmod(deg, 360) + 360, 360)` to force exactly that interval.
    /// - `gamma`'s ceiling is `#define MAX_GAMMA 1000.0`, **read from the file**.
    ///
    /// `softness` and `proportion` are `0..1`: the first is `1 - inner_limit` where the limit is a
    /// fraction, and the second interpolates, which is what a proportion means.
    ///
    /// # `shape` reuses `FocusShape` because upstream reuses `GimpLimitType`
    ///
    /// The shape comes from `GimpLimitType` -- `CIRCLE, SQUARE, DIAMOND, HORIZONTAL, VERTICAL` in
    /// `app/display/display-enums.h` -- which is the SAME enum `gimppropgui-focus-blur.c` reads. Our
    /// `FocusShape` was declared for focus-blur in K.3 with exactly those five variants in that
    /// order, so this variant reuses it rather than declaring a parallel copy.
    ///
    /// That is cycle 79's rule in its stronger form: where two copies of a defined object would
    /// exist, prefer EQUAL BY CONSTRUCTION over tested to agree. A second enum could drift; a shared
    /// one cannot.
    ///
    /// # How `proportion` and `squeeze` combine, read from the inverse
    ///
    /// `config_notify` reconstructs the aspect as
    /// `1 + (height/width - 1) * proportion`, then divides or multiplies by
    /// `tan(|squeeze| * PI/2) + 1`. So `proportion` interpolates the region's aspect from circular
    /// (0) to the image's own shape (1), and `squeeze` then distorts it further. At `proportion = 1`,
    /// `squeeze = 0` and `radius = 1` the region exactly inscribes the canvas, which is the case
    /// that pins the convention.
    /// Blur along circular arcs about a centre, so every sample keeps its distance from that centre.
    ///
    /// # Three properties, and the relations prove each other again
    ///
    /// `app/propgui/gimppropgui-motion-blur-circular.c` names `center-x`, `center-y` and `angle`,
    /// and `line_callback` and `config_notify` are exact inverses:
    ///
    /// | property | written as | read back as |
    /// |---|---|---|
    /// | `center-x` | `x1 / area->width` | `x * area->width` |
    /// | `center-y` | `y1 / area->height` | `y * area->height` |
    /// | `angle` | `atan2(-(y2-y1), x2-x1) * 180/PI`, `+= 360` if negative | `cos/sin(angle)`, with `y2 = y1 - sin(...)` |
    ///
    /// So the centre is NORMALISED -- the same convention as vignette, and the opposite of
    /// supernova's pixel radius -- and `angle` is in DEGREES over `0..360`, the interval forced by
    /// the `if (angle < 0) angle += 360`.
    ///
    /// # The negated y is UI plumbing, not filter geometry
    ///
    /// `atan2(-(y2 - y1), ...)` and its inverse `y2 = y1 - sin(angle) * 100` both measure the angle
    /// with y pointing UP, where the canvas has y pointing down. That matters for turning a dragged
    /// line into a number, and it is stated in both directions so it is certainly real.
    ///
    /// It has no effect on the rendered output, though, and saying why is the point: the blur spans
    /// an arc CENTRED on each pixel's own position, from `-angle/2` to `+angle/2`. A symmetric span
    /// is unchanged by flipping its sign, so no image can distinguish the two conventions. Recorded
    /// rather than tested, because a test asserting a direction here would be asserting something
    /// the filter cannot express.
    ///
    /// # Why this cannot be the linear motion blur with different numbers
    ///
    /// Source 7: upstream ships `motion-blur-linear`, `motion-blur-circular` and `motion-blur-zoom`
    /// as three separate operations with three separate propguis, so they are three mechanisms. The
    /// structural difference is exact and testable -- linear moves samples along a fixed direction,
    /// circular moves them along an arc and so **preserves each sample's radius from the centre**,
    /// and zoom moves them radially and so preserves each sample's ANGLE.
    ///
    /// Our shipped `MotionBlur` is the LINEAR one only. That was recorded wrongly once, at AUDIT-5,
    /// as "the three motion blurs we fold into one variant", and corrected at cycle 65; this variant
    /// is the first of the two that correction left outstanding.
    /// Blur along radial rays from a centre, so every sample keeps its bearing from that centre.
    ///
    /// # Three properties, relations proven in both directions
    ///
    /// `app/propgui/gimppropgui-motion-blur-zoom.c` names `center-x`, `center-y` and `factor`, and
    /// `line_callback` and `config_notify` are exact inverses:
    ///
    /// | property | written as | read back as |
    /// |---|---|---|
    /// | `center-x` | `x1 / area->width` | `x * area->width` |
    /// | `center-y` | `y1 / area->height` | `y * area->height` |
    /// | `factor` | `CLAMP ((x2 - x1) / 100.0, -0.5, 1.0)` | `x2 = x1 + radius * 100.0`, `y2 = y1` |
    ///
    /// `y2 = y1` in the inverse is informative on its own: the controller's line has no vertical
    /// component, so `factor` is a pure scalar with no angular part.
    ///
    /// # The ASYMMETRIC range explains the formula
    ///
    /// `CLAMP(..., -0.5, 1.0)` is read verbatim, and the asymmetry is the interesting part -- a
    /// guess would have been `0..1` or `±1`. It is explained by the factor being MULTIPLICATIVE:
    ///
    /// ```text
    /// 1 + 1.0  = 2        -- twice the radius
    /// 1 + -0.5 = 0.5      -- half the radius
    /// ```
    ///
    /// So the range is symmetric in the SCALE, `1 + factor`, while looking lopsided in the
    /// parameter. That is evidence for the multiplicative form rather than an additive one, from a
    /// bound that would otherwise just be a number to copy.
    ///
    /// # One-sided, and that part is a recorded CHOICE
    ///
    /// Whether the samples run from the pixel's own radius outward, or straddle it, is not readable:
    /// the drawing loop lives in GEGL, which is not vendored. One-sided is chosen, for two stated
    /// reasons -- the controller drags a line FROM the centre outward whose length is `factor * 100`,
    /// which reads as an extent rather than a half-extent; and a straddling span would be symmetric
    /// in the sign of `factor`, which would leave the read asymmetry of `-0.5..1.0` meaningless.
    ///
    /// The second is the stronger argument, and it is why this is recorded as a choice supported by
    /// evidence rather than a bare preference.
    ///
    /// # Why this is neither of its two siblings
    ///
    /// Source 7: three operations, three propguis, three mechanisms. Linear moves samples along a
    /// fixed direction; circular along an arc, preserving each sample's RADIUS from the centre; this
    /// one along a radial ray, preserving each sample's BEARING. Circular and zoom are exact
    /// complements, which is testable as one assertion about the pair.
    /// Random jitter applied in CIE LCh(ab) -- lightness, chroma and hue each perturbed separately.
    ///
    /// # Every source is empty except the action and the menu
    ///
    /// No plug-in C, no propgui, no config object, and no po string: the po file carries
    /// `noise-hsv.c`, `noise-rgb.c`, `noise-solid.c` and `noise-spread.c`, but nothing for this one.
    /// What exists is `_CIE lch Noise...` with its ellipsis, so at least one parameter, and the menu
    /// placing it under `N_oise`.
    ///
    /// # So the family decides the shape and the name decides the space
    ///
    /// Source 7 is unusually direct here, because the siblings are SHIPPED BY US. Upstream ships
    /// three: `gegl:noise-rgb`, `gegl:noise-hsv` and this. We already have the first two, and the
    /// readable pair shows the family's pattern -- `noise-hsv.c` declares `H_ue:`, `_Saturation:`,
    /// `_Value:`, one amount per channel of its space, and `noise-rgb.c` declares `_Red:`,
    /// `_Green:`, `_Blue:` the same way.
    ///
    /// CIE LCh has three channels, so three amounts: `lightness`, `chroma`, `hue`. Plus `seed`,
    /// which both shipped siblings already carry.
    ///
    /// # One difference that is READ, not assumed
    ///
    /// `filters-actions.c` gates `noise-hsv` on `writable && !gray` and gates this one **not at
    /// all**. That is a real semantic difference rather than an oversight: HSV's hue and saturation
    /// are meaningless on a grey image, while CIE LCh's **L** is perfectly meaningful there. So this
    /// filter works on a greyscale image where its HSV sibling is refused, and that is testable.
    ///
    /// # What is deliberately absent
    ///
    /// Upstream's `noise-hsv.c` also declares `_Holdness:`. Whether this operation has one is not
    /// readable, and our shipped `HsvNoise` does not carry it either, so adding it here alone would
    /// invent a parameter AND make the family inconsistent. Absent is more honest than guessed; the
    /// `holdness` gap on `HsvNoise` is filed separately rather than papered over here.
    /// Slur (GIMP noise-slur): with probability `amount`, replace a pixel with one from ABOVE it —
    /// a directional smear that reads as melting or dripping.
    ///
    /// # Nothing readable declares this, so the name and the siblings decide it
    ///
    /// All three of `gegl:noise-hurl`, `gegl:noise-pick` and `gegl:noise-slur` have only an action
    /// with an ellipsis and a menu entry under `N_oise`. There is no plug-in C, no propgui, no config
    /// object, and no po string for any of them — upstream once shipped the three from one plug-in
    /// and nothing of it survives in this tree.
    ///
    /// So the evidence is the name plus source 7, and here source 7 is unusually strong because the
    /// two siblings are SHIPPED BY US: `Hurl { amount, seed }` and `Pick { amount, seed }`. Three
    /// names in one catalogue mean three mechanisms, and the shape of the two we have fixes the shape
    /// of the third.
    ///
    /// # What separates the three, and what is entailed rather than chosen
    ///
    /// | filter | draws from | palette | direction |
    /// |---|---|---|---|
    /// | hurl | a random colour | destroyed | none |
    /// | pick | any of the EIGHT neighbours | kept | isotropic |
    /// | slur | the row ABOVE | kept | **downward** |
    ///
    /// Pick already occupies "replace with a random neighbour", so slur cannot be that without being
    /// a duplicate — and a duplicate within one catalogue is a contradiction rather than a judgement
    /// call. What the word adds is the direction: a slur RUNS, and it runs downward. That much is
    /// entailed.
    ///
    /// The exact weighting across the three cells above — straight up against the two diagonals — is
    /// not readable anywhere in this tree, so it is a recorded CHOICE in `filters.rs` rather than a
    /// reading. The direction is derived; the distribution is not.
    /// A sub-pixel display mask: each pixel keeps one colour channel, as a low-resolution monitor
    /// would show it.
    ///
    /// # Source 2 under a shorter name than the operation
    ///
    /// `gegl:video-degradation` has no plug-in C, propgui or config object, but the po file
    /// references `plug-ins/common/video.c` -- the file is gone and its strings remain, which is the
    /// case the source list assumes. Searching for `video` rather than `video-degradation` is what
    /// found it, the same lesson as `lens-apply.c` under reversed words.
    ///
    /// The block is a complete contract:
    ///
    /// | line | string | what it is |
    /// |---|---|---|
    /// | 42-50 | nine pattern names | **an enum, in declaration order** |
    /// | 1807 | `Simulate distortion produced by a fuzzy or low-res monitor` | names the MECHANISM |
    /// | 1814 | `Vi_deo...` | menu label |
    /// | 2040 | `Video Pattern` | the frame label for the enum |
    /// | 2084 | `_Additive` | toggle |
    /// | 2094 | `_Rotated` | toggle |
    ///
    /// So three parameters: a nine-valued `pattern`, and two booleans. The blurb is load-bearing --
    /// "fuzzy or low-res monitor" says the mask is a SUB-PIXEL layout rather than a blur or a noise,
    /// which is what makes each pattern a choice of channel per position.
    ///
    /// # What `additive` and `rotated` do, and why each is testable
    ///
    /// Replacing keeps the selected channel and drops the others, so it can only DARKEN. Adding puts
    /// the selected channel on top of what is there, so it can only BRIGHTEN. That gives a clean
    /// assertion about the pair rather than two vague ones.
    ///
    /// `rotated` transposes the mask. For `Striped`, whose channel depends on the column alone, the
    /// consequence is exact: unrotated every COLUMN is uniform, rotated every ROW is.
    /// Rebuilds every other row from its neighbours, for an image where one scan field is missing.
    ///
    /// # The blurb names the mechanism, so almost nothing is left to choose
    ///
    /// `plug-ins/common/deinterlace.c` is gone from the tree but its po strings survive, and they
    /// are a complete contract:
    ///
    /// | line | string | what it is |
    /// |---|---|---|
    /// | 91 | `Fix images where every other row is missing` | **names the MECHANISM** |
    /// | 100 | `_Deinterlace...` | menu label, so at least one parameter |
    /// | 157, 323 | `Deinterlace` | dialog title |
    /// | 356 | `Keep o_dd fields` | the parameter |
    /// | 357 | `Keep _even fields` | its other value |
    ///
    /// Two labelled options at consecutive lines are one two-way choice, and `Keep o_dd fields`
    /// coming first makes odd the default.
    ///
    /// The blurb does the rest of the work. "Every other row is missing" says exactly what the
    /// filter is for and therefore what it must do: the kept field is real data and passes through
    /// untouched, and the other rows are reconstructed from the rows either side of them -- which
    /// are both kept rows, because the fields alternate. There is no scope for a blend amount or a
    /// radius, and the ellipsis is accounted for by the one choice that exists.
    ///
    /// The menu places it under `En_hance` rather than `N_oise`, which agrees: this is a repair, not
    /// a degradation. Its sibling `video-degradation` is the filter that puts the artefact in.
    ///
    /// # The one case the blurb does not settle
    ///
    /// A discarded row at the very top or bottom edge has only ONE neighbour, not two. It takes that
    /// neighbour's value, which is the only reading that does not invent data.
    /// Pulls back a pixel's red channel to what its green and blue justify, leaving the rest alone.
    ///
    /// # What exists to read
    ///
    /// No plug-in C, no propgui, no config object, and no po reference to a plug-in file. The app's
    /// own po carries `_Red Eye Removal...` and `Red Eye Removal`, which are the action label and
    /// the dialog title rather than parameter names. So:
    ///
    /// - The ellipsis says at least one parameter.
    /// - The menu places it under `En_hance`, in the same submenu as `deinterlace` -- a repair.
    /// - `filters-actions.c` gates it on `writable && !gray`, and that is the useful reading:
    ///   **exactly eleven filter actions carry that gate**, and every one is a colour operation
    ///   (`color-balance`, `colorize`, `color-temperature`, `desaturate`, `hue-saturation`,
    ///   `mono-mixer`, `noise-hsv`, `saturation`, `sepia`, and this). Red eye is a relationship
    ///   BETWEEN channels, so on a grey image there is nothing to find -- which is why the gate is
    ///   there and why a grey pixel must come out untouched at any setting.
    ///
    /// # Why one parameter, and why it is a threshold
    ///
    /// The name specifies the operation completely: it says which artefact and that it is to be
    ///  removed. The only thing left open is **how much red counts as too much** -- a threshold. A
    /// radius would imply the filter searches for an eye, which the name does not say and which the
    /// `!gray` gate argues against, since a shape search would work on grey just as well. A strength
    /// would imply partial removal, which "removal" does not. So the ellipsis is satisfied by exactly
    /// one parameter and anything further would be invented.
    ///
    /// # The invariant that makes it this filter and not a desaturation
    ///
    /// Only the RED channel is ever written, and only ever downward. Green and blue are carried
    /// through byte for byte. A filter that touched them would be adjusting colour balance, not
    /// removing red eye, and that is the difference a test can state in one assertion.
    /// A user-supplied 5x5 kernel, with its own divisor, offset, border rule and channel mask.
    ///
    /// # Two sources, and together they are complete
    ///
    /// `app/propgui/gimppropgui-convolution-matrix.c` names the kernel cells **positionally and by
    /// name at once**, as a literal table:
    ///
    /// ```text
    /// {"a1", "b1", "c1", "d1", "e1"},
    /// {"a2", "b2", "c2", "d2", "e2"},
    /// ...
    /// {"a5", "b5", "c5", "d5", "e5"}};
    /// ```
    ///
    /// So the letter is the COLUMN and the number is the ROW, and `a1..e1` is the first row. That
    /// ordering decides what a saved matrix means, so it is pinned by a test rather than trusted.
    /// The propgui also names `divisor` and `offset`.
    ///
    /// `plug-ins/common/convolution-matrix.c`'s po strings supply everything the propgui leaves to
    /// the generic builder:
    ///
    /// | string | what it is |
    /// |---|---|
    /// | `Apply a generic 5x5 convolution matrix` | confirms the size **exactly** |
    /// | `Matrix` | frame label for the 25 cells |
    /// | `D_ivisor:`, `O_ffset:` | the two scalars |
    /// | `N_ormalise`, `A_lpha-weighting` | two toggles |
    /// | `Border` + `E_xtend`, `_Wrap`, `Cro_p` | a three-way edge rule |
    /// | `Channels` + `Gr_ey`, `Re_d`, `_Green`, `_Blue`, `_Alpha` | the channel mask |
    /// | `Convolution does not work on layers smaller than 3x3 pixels.` | **a READ constraint** |
    ///
    /// # Why the channel mask is four and not five
    ///
    /// The strings list five, but `Gr_ey` is not a fifth channel: it is what the same control shows
    /// for a GREYSCALE image, where `Re_d`/`_Green`/`_Blue` are meaningless. The set of toggles
    /// depends on the image's mode rather than there being five simultaneous ones.
    ///
    /// Our rasters are RGBA at every precision, so the four that exist here are red, green, blue and
    /// alpha. This is the dialog-state rule in an unfamiliar shape -- not a control that is pure UI,
    /// but a control whose SET is mode-dependent.
    ///
    /// # The size constraint is read, not chosen
    ///
    /// `Convolution does not work on layers smaller than 3x3 pixels.` is an error message, which
    /// means upstream refuses rather than coping. So a layer under 3x3 is rejected, and the test for
    /// it quotes the measurement rather than a convention of ours.
    ///
    /// Note it says 3x3 while the kernel is 5x5 -- upstream's own threshold, not a typo to tidy.
    /// Blends the image with its own half-offset copy so opposite edges meet.
    ///
    /// # The blurb is the whole specification
    ///
    /// `plug-ins/common/tile-seamless.c` is gone but three strings survive:
    ///
    /// | line | string |
    /// |---|---|
    /// | 66 | `Alters edges to make the image seamlessly tileable` |
    /// | 72 | `_Make Seamless` |
    /// | 335 | `Tiler` |
    ///
    /// The blurb specifies the operation completely, and the mechanism follows from it. Offsetting
    /// the image by half in both axes moves its edges to the centre: the offset copy's own edges are
    /// then ADJACENT columns of the original, which already match, while the original's old seam
    /// now runs as a cross through the offset copy's middle. Blending the two, weighted by distance
    /// from the original's edges, takes the smooth part of each.
    ///
    /// # Parameterless, and the one piece of evidence that disagrees is recorded rather than acted on
    ///
    /// `_Make Seamless` has **no ellipsis**, so the plug-in was parameterless. The blurb needs no
    /// parameter either.
    ///
    /// Against that: `filters-actions.c` puts this in `filters_interactive_actions[]` with
    /// `_Tile Seamless...`, and upstream keeps a separate `filters_actions[]` of exactly SIX
    /// parameterless operations that open no dialog -- `antialias`, `color-enhance`,
    /// `invert-linear`, `invert-gamma`, `value-invert`, `stretch-contrast-hsv`. Membership in the
    /// interactive array therefore suggests at least one property, and every interactive operation
    /// checked (`image-gradient`, `distance-transform`) does have one.
    ///
    /// I cannot settle it: the property list lives in GEGL, which is not vendored. So this ships
    /// parameterless, because **inventing a control to satisfy an ellipsis would put it in the
    /// command enum and in saved documents forever**, and the discrepancy is filed instead. That is
    /// the same call as the `softglow` third-parameter gap: record it, do not paper over it.
    /// Replaces PARTIAL transparency with a colour, leaving fully transparent and fully opaque
    /// pixels alone.
    ///
    /// # The implementation itself is vendored, so this is read rather than derived
    ///
    /// `gimp:semi-flatten` is one of GIMP's own operations, not a GEGL one, so
    /// `app/operations/gimpoperationsemiflatten.c` is in the tree. Only the second filter in this
    /// work whose arithmetic could be read directly -- `colorize` was the first.
    ///
    /// The body is exactly:
    ///
    /// ```c
    /// gfloat alpha = src[ALPHA];
    /// if (alpha <= 0.0 || alpha >= 1.0)
    ///   { dest = src; }                                  // untouched, alpha included
    /// else
    ///   {
    ///     dest[RED]   = src[RED]   * alpha + rgba[0] * (1.0 - alpha);
    ///     dest[GREEN] = src[GREEN] * alpha + rgba[1] * (1.0 - alpha);
    ///     dest[BLUE]  = src[BLUE]  * alpha + rgba[2] * (1.0 - alpha);
    ///     dest[ALPHA] = 1.0;
    ///   }
    /// ```
    ///
    /// The conditional is the load-bearing part. This is NOT "composite the layer onto a colour":
    /// a fully transparent pixel stays fully transparent and a fully opaque one is untouched. Only
    /// the partial pixels are affected, and they become **fully opaque**. The blurb says so in as
    /// many words -- `Replace partial transparency with a color`.
    ///
    /// # Two further readings from the same file
    ///
    /// `prepare` declares `babl_format_with_space ("RGBA float", space)` -- plain `RGBA float`, with
    /// **no `linear` suffix**, so the blend happens in NON-LINEAR space. Our 8-bit buffers already
    /// hold non-linear sRGB, so blending the bytes directly is exact here rather than an
    /// approximation, and linearising would be wrong. The opposite quirk to `colorize`, settled by a
    /// format declaration instead of by measurement.
    ///
    /// The colour's default is READ too:
    /// `gimp_param_spec_color_from_string ("color", _("Color"), _("The color"), FALSE, "white", ...)`.
    ///
    /// One parameter, which is all the operation declares. The action is gated `writable && alpha`,
    /// one of exactly four that are -- with `dropshadow`, `long-shadow` and `threshold-alpha` -- and
    /// here the reason is plain: with no alpha there is no partial transparency to replace.
    SemiFlatten {
        #[serde(default = "crate::command::white")]
        color: Pixel,
    },
    /// Makes transparency all-or-nothing by thresholding the alpha channel.
    ///
    /// # Vendored, so read rather than derived
    ///
    /// `gimp:threshold-alpha` is another of GIMP's own operations, so
    /// `app/operations/gimpoperationthresholdalpha.c` is in the tree. The body is exactly:
    ///
    /// ```c
    /// dest[RED]   = src[RED];
    /// dest[GREEN] = src[GREEN];
    /// dest[BLUE]  = src[BLUE];
    ///
    /// if (src[ALPHA] > self->value)
    ///   dest[ALPHA] = 1.0;
    /// else
    ///   dest[ALPHA] = 0.0;
    /// ```
    ///
    /// Two things are load-bearing. **RGB is copied unconditionally** -- including on pixels whose
    /// alpha is thrown away, so this thresholds the alpha channel rather than erasing the pixel.
    /// And the comparison is **strictly greater**, not `>=`.
    ///
    /// The blurb is the whole specification: `Make transparency all-or-nothing, by thresholding the
    /// alpha channel to a value`.
    ///
    /// # What strictness costs at the ends of the range
    ///
    /// `value` is `g_param_spec_double ("value", _("Value"), _("The alpha value"), 0.0, 1.0, 0.5,
    /// ...)` -- range and default both read.
    ///
    /// At `value` 1.0 nothing satisfies `alpha > 1.0`, so **every pixel becomes transparent,
    /// including the fully opaque ones**. A `>=` implementation would keep them at full alpha
    /// instead, which makes this the single sharpest check on the comparison.
    ///
    /// At `value` 0.0 it is not "keep everything" either: exactly-zero alpha fails `> 0.0` and is
    /// discarded, while one step above it survives.
    ///
    /// # The pair with semi-flatten
    ///
    /// Both operations exist to remove partial alpha, and they are the two opposite ways to do it:
    /// `semi-flatten` keeps every pixel and changes its colour, this keeps every colour and discards
    /// pixels. Both are gated `writable && alpha`.
    ThresholdAlpha {
        #[serde(default = "crate::command::default_alpha_threshold")]
        value: f64,
    },
    /// Direction-dependent edge detection with the Sobel kernels.
    ///
    /// # The parameter list is read, in dialog order
    ///
    /// `plug-ins/common/edge-sobel.c` is deleted but `po-plug-ins` still carries its strings, at
    /// consecutive line numbers which give the dialog order:
    ///
    /// - 108 `Specialized direction-dependent edge detection` -- the blurb
    /// - 121 `_Sobel...`
    /// - 229 `Sobel Edge Detection`
    /// - **259 `Sobel _horizontally`**
    /// - **271 `Sobel _vertically`**
    /// - **283 `_Keep sign of result (one direction only)`**
    /// - 370 `Sobel edge detecting`
    ///
    /// Three booleans, and the third's label carries its own semantics: keeping the sign is
    /// meaningful **only when one direction is active**, because with both on the result is a
    /// magnitude and has no sign to keep.
    ///
    /// # The kernels are the name
    ///
    /// `Sobel` is a published operator, so the contract IS the definition -- the strongest form of
    /// the naming rule, as with `slic` and `bayer-matrix`. `Gx` is `[-1 0 1; -2 0 2; -1 0 1]` and
    /// `Gy` is its transpose. Nothing here is chosen.
    ///
    /// The sign convention follows from what the kernels approximate: `Gx` is `dI/dx`, positive
    /// where intensity increases with x. `Gy` is `dI/dy`, positive where intensity increases with
    /// **y as the row index**, i.e. downward -- the raster's own direction. With `keep_sign` off the
    /// absolute value is taken and the convention is invisible; it is observable only with it on.
    ///
    /// # Why negatives clamp rather than biasing to 128
    ///
    /// Upstream computes in float where negatives simply exist. At 8 bits they must go somewhere,
    /// and this is **forced rather than chosen**: a flat field has no edges, so an edge detector must
    /// return black on it. Biasing by 128 would make flat input mid-grey, contradicting the blurb's
    /// own word "detection". So the signed response is written directly and negatives clamp to 0,
    /// which is what makes `keep_sign` genuinely direction-dependent -- only edges running one way
    /// light up.
    EdgeSobel {
        #[serde(default = "crate::command::yes")]
        horizontal: bool,
        #[serde(default = "crate::command::yes")]
        vertical: bool,
        #[serde(default)]
        keep_sign: bool,
    },
    /// Contrast to greyscale — a LOCAL-contrast mono conversion.
    ///
    /// # K.13's last operation, and the one the group was parked on
    ///
    /// Not [`Filter::Grayscale`] (which applies fixed luminance weights) and not a desaturation:
    /// this decides each pixel's grey from where it sits between the brightest and darkest colours
    /// in its own NEIGHBOURHOOD, so two pixels of the same colour can map to different greys. It is
    /// ported from `gegl/operations/common/c2g.c`, which GIMP only names.
    ///
    /// # It shares `stress`'s machinery and NOT its numbers
    ///
    /// `c2g.c` includes the same `envelopes.h` as `stress.c` — its file header is even titled
    /// *STRESS, Spatio Temporal Retinex Envelope with Stochastic Sampling* — so
    /// [`crate::filters::StressSpray`] is reused rather than rewritten. **Three of the four
    /// property declarations differ, which is exactly the trap in reusing the machinery:**
    ///
    /// | property | `c2g` | `stress` |
    /// |---|---|---|
    /// | `radius` | 300, `(2, 6000)` | 300, `(2, 6000)` |
    /// | `samples` | **4**, `(1, 1000)` | 5, `(2, 500)` |
    /// | `iterations` | **10**, `(1, 1000)` | 5, `(1, 1000)` |
    /// | `enhance_shadows` | FALSE | FALSE |
    ///
    /// So `samples` here accepts **1**, which `stress` rejects, and goes to 1000 where `stress`
    /// stops at 500. Carrying `stress`'s bounds across would refuse values upstream accepts.
    ///
    /// # The grey is a DISTANCE RATIO, and `enhance_shadows` changes which distance
    ///
    /// Upstream's own comment calls it an approximation of projecting the pixel onto the vector
    /// from the local minimum to the local maximum, computed by comparing distances:
    ///
    /// ```text
    /// ON : grey = |pixel - min| / (|pixel - min| + |pixel - max|)
    /// OFF: grey = |pixel|       / (|pixel|       + |pixel - max|)
    /// ```
    ///
    /// Both are Euclidean distances over RGB. With the flag OFF upstream passes `NULL` for the
    /// lower envelope and never computes it, measuring instead from the ORIGIN — so a pixel's grey
    /// is its distance from black rather than from the darkest nearby colour. That is the same
    /// shape as `stress`'s flag and a different formula.
    ///
    /// A zero denominator gives **0.5**, which upstream marks `/* shouldn't happen */`.
    ///
    /// # Output is greyscale plus alpha
    ///
    /// Upstream's output format is `YA float` — two components. Our raster is RGBA, so the one grey
    /// goes to all three colour channels and alpha is carried through from the sampled pixel.
    C2g {
        #[serde(default = "crate::command::stress_radius")]
        radius: u32,
        #[serde(default = "crate::command::c2g_samples")]
        samples: u32,
        #[serde(default = "crate::command::c2g_iterations")]
        iterations: u32,
        #[serde(default)]
        enhance_shadows: bool,
    },
    /// Mantiuk, Myszkowski and Seidel 2006 contrast-domain tone mapping.
    ///
    /// # K.10's last operator, and the largest filter in this backlog
    ///
    /// Ported from `gegl/operations/common/mantiuk06.c`, 1654 lines. GIMP only names it. The
    /// pipeline, the copied transducer table and the solver are documented in [`crate::mantiuk`];
    /// this is the parameter surface, and the parameter surface is where the finding is.
    ///
    /// Upstream declares THREE properties and reads TWO:
    ///
    /// - `contrast` — `property_double`, default **0.1**, `value_range (0.0, 1.0)`
    /// - `saturation` — default **0.8**, `value_range (0.0, 2.0)` — note the ceiling is 2, not 1,
    ///   unlike `fattal02`'s saturation
    /// - `detail` — default 1.0, `value_range (1.0, 99.0)`, described as *"Level of emphasis on
    ///   image gradient details"* … **and never read**
    ///
    /// # `detail` is DEAD upstream, so this product does not offer it
    ///
    /// `o->detail` appears nowhere in the file beyond its own declaration. `process` calls
    /// `contmap (width, height, pix, lum, o->contrast, o->saturation, FALSE, 200, 1e-3, NULL)` —
    /// there is no argument for it and no other reader. A user who drags that slider in GIMP
    /// changes nothing.
    ///
    /// Exposing it here would mean shipping a control that does nothing, which is worse than an
    /// absent one: the user who tries it learns that this product's controls lie. So it is omitted
    /// deliberately and `audit4`'s `DEAD_UPSTREAM` table names it, which turns a silent
    /// one-parameter gap into a recorded decision.
    ///
    /// # `contrast` 0 is a different ALGORITHM, not a weaker setting
    ///
    /// Upstream branches on `contrastFactor > 0`: above zero it multiplies every gradient by it,
    /// and at zero it runs **contrast equalisation** instead — a histogram equalisation of gradient
    /// magnitudes ranked across every pyramid level at once. Since the declared range starts at 0,
    /// the bottom of the slider is the only way to reach that branch.
    ///
    /// # Alpha is clipped and otherwise untouched
    ///
    /// A third answer on alpha, from a third reading. `fattal02`'s buffer is `RGB float` so it has
    /// no alpha; `reinhard05`'s is `RGBA float` and it rescales alpha with the colours. This one is
    /// `RGBA float` too, and its clip loop runs over all four components — so alpha is raised to
    /// the `1e-7 * max(Y)` floor — but the colour step writes only three, so nothing else touches
    /// it.
    Mantiuk06 {
        #[serde(default = "crate::command::mantiuk_contrast")]
        contrast: f64,
        #[serde(default = "crate::command::fattal_saturation")]
        saturation: f64,
    },
    /// Fattal, Lischinski and Werman 2002 gradient-domain tone mapping.
    ///
    /// # K.10, and the operator this group was parked over
    ///
    /// Three of K.10's four names are literal citations and this is the one the group's blocker was
    /// written about: reconstructing "fattal02" from memory would have attributed an invention of
    /// mine to named researchers. It is ported from `gegl/operations/common/fattal02.c`, which
    /// GIMP only names. The pipeline, the solver argument and the divergences are documented in
    /// [`crate::fattal`]; this is the parameter surface.
    ///
    /// Four properties, read from the property block:
    ///
    /// - `alpha` — `property_double`, default **1.0**, `value_range (0.0, 2.0)`, *"Gradient
    ///   threshold for detail enhancement"*
    /// - `beta` — default **0.9**, `value_range (0.1, 2.0)`, *"Strength of local detail
    ///   enhancement"*. Note the floor is 0.1, not 0.
    /// - `saturation` — default **0.8**, `value_range (0.0, 1.0)`
    /// - `noise` — default **0.0**, `value_range (0.0, 1.0)`
    ///
    /// # `noise` 0 does NOT mean no noise floor
    ///
    /// The one trap in the parameters, and it is in `process` rather than in the declaration:
    ///
    /// ```c
    /// if (o->noise == 0.0)
    ///   noise = o->alpha * 0.1;
    /// ```
    ///
    /// So the declared default of 0 is a sentinel for "derive it from alpha", and the effective
    /// default is **0.1**. A reader who takes the property block at face value gets an operator
    /// with no noise floor, which amplifies sensor noise in the shadows — exactly what the
    /// parameter exists to prevent.
    ///
    /// # Alpha is untouched, because upstream's buffer has none
    ///
    /// `OUTPUT_FORMAT` here is `"RGB float"` and `pix_stride` is **3**, where `reinhard05` next door
    /// uses `RGBA float` and 4. So this operator never sees an alpha channel, and the faithful
    /// reading is to pass ours through — the opposite conclusion from `reinhard05`, from the same
    /// kind of evidence, which is why both are recorded rather than assumed.
    Fattal02 {
        #[serde(default = "crate::command::unit_one")]
        alpha: f64,
        #[serde(default = "crate::command::fattal_beta")]
        beta: f64,
        #[serde(default = "crate::command::fattal_saturation")]
        saturation: f64,
        #[serde(default)]
        noise: f64,
    },
    /// Reinhard 2005 tone mapping — a global HDR-to-LDR operator.
    ///
    /// # K.10, and the first PRECISION-NATIVE filter that is not a complement
    ///
    /// GIMP names this operator and does not define it; the definition is
    /// `gegl/operations/common/reinhard05.c` (LGPL half of the tree), itself derived from pfstmo.
    /// Three properties, all read from the property block:
    ///
    /// - `brightness` — `property_double`, default **0.0**, `value_range (-100.0, 100.0)`
    /// - `chromatic` — default **0.0**, `value_range (0.0, 1.0)`
    /// - `light` — default **1.0**, `value_range (0.0, 1.0)`
    ///
    /// # Why this one is precision-native when `stress` was not
    ///
    /// Its whole purpose is to compress a range an 8-bit document no longer has. K.10's own note
    /// said these need the `PRECISION_NATIVE_FILTERS` path rather than the 8-bit match arm, and
    /// the machinery is already there: `Precision::F32` stores unclamped samples and EXR import
    /// preserves them, so a real HDR document can reach this filter.
    ///
    /// # The operator, and the two constants it DERIVES rather than takes
    ///
    /// ```text
    /// key       = (ln max_Y - mean ln(2.3e-5 + Y)) / (ln max_Y - ln(2.3e-5 + min_Y))
    /// contrast  = 0.3 + 0.7 * key^1.4
    /// intensity = exp(-brightness)
    /// ```
    ///
    /// Then per pixel and per RGB channel, in LINEAR light:
    ///
    /// ```text
    /// local  = chromatic * p   + (1 - chromatic) * Y
    /// global = chromatic * avg_channel + (1 - chromatic) * avg_Y
    /// adapt  = light * local + (1 - light) * global
    /// p      = p / (p + (intensity * adapt)^contrast)
    /// ```
    ///
    /// and finally every channel is rescaled by `(p - min) / range` of the values just written.
    ///
    /// # Three behaviours that are upstream's and look like our bugs
    ///
    /// **A pixel whose luminance is exactly 0 is SKIPPED** (`if (lum[i] == 0.0) continue;`), so it
    /// keeps its colour through the operator and does not contribute to the rescaling statistics.
    /// It is then rescaled anyway, to `(0 - min) / range`, which is negative.
    ///
    /// **The final rescale covers ALPHA.** Upstream loops `c < pix_stride` where `pix_stride` is 4,
    /// while the statistics were gathered over RGB only — so an opaque pixel's alpha becomes
    /// `(1 - min) / range`. That is upstream's own arithmetic, not an oversight here, and it is
    /// asserted in the tests rather than quietly corrected.
    ///
    /// **A fully black layer is REFUSED**, with [`crate::CoreError::FilterNoDynamicRange`]. The
    /// derivation divides two infinities, `contrast` comes out `NaN`, and upstream's own
    /// `g_return_val_if_fail (contrast >= 0.3 && contrast <= 1.0)` fails the operation. Refusing is
    /// equivalence.
    ///
    /// # One divergence, forced
    ///
    /// When every mapped sample is identical the rescaling range is zero and upstream computes
    /// `0 / 0`, writing `NaN` into the buffer. We leave the mapped values unrescaled instead. A
    /// document full of `NaN` is not a tone-mapped image, and no clamp recovers it.
    Reinhard05 {
        #[serde(default)]
        brightness: f64,
        #[serde(default)]
        chromatic: f64,
        #[serde(default = "crate::command::unit_one")]
        light: f64,
    },
    /// Spatio Temporal Retinex-like Envelope with Stochastic Sampling — a local-contrast stretch.
    ///
    /// # K.10, and the first item ported from GEGL's own tree rather than GIMP's
    ///
    /// GIMP names this operator and does not define it: it has an action label, the
    /// `_Tone Mapping` menu category and an ellipsis, and nothing else. The definition is
    /// `gegl/operations/common/stress.c` plus the `envelopes.h` it includes, registered as
    /// `[gegl_operations]` in `docs/upstream-sources.toml`. Four properties, all read from the
    /// property block:
    ///
    /// - `radius` — `property_int`, default **300**, `value_range (2, 6000)`
    /// - `samples` — default **5**, `value_range (2, 500)`
    /// - `iterations` — default **5**, `value_range (1, 1000)`
    /// - `enhance_shadows` — `property_boolean`, default **FALSE**
    ///
    /// A fifth, `rgamma`, is declared and then commented out in favour of a fixed `RGAMMA 2.0`, so
    /// it is withdrawn upstream rather than omitted here. See [`crate::filters::STRESS_RGAMMA`].
    ///
    /// # `enhance_shadows` changes which envelope is used, not how strongly
    ///
    /// This is the reading that matters, and the name does not give it away. With the flag OFF —
    /// the default — upstream passes `NULL` for the minimum envelope and never computes it, then
    /// writes `pixel / max`. With it ON it computes both and writes `(pixel - min) / (max - min)`.
    /// So it is not an intensity knob: off divides by the upper envelope alone, which leaves dark
    /// regions dark, and on rescales the full envelope, which lifts them. Upstream's own
    /// description agrees — *"when disabled a more natural result is yielded"*.
    ///
    /// A channel whose divisor is zero becomes **0.5**, in both modes.
    ///
    /// # Two divergences, both forced, both recorded
    ///
    /// **We are deterministic and upstream is not.** See [`crate::filters::StressSpray`]: upstream's
    /// spray comes from an unseeded PRNG and its own reference hash is marked `"unstable"`.
    ///
    /// **Our samples are the stored sRGB-encoded bytes**, where upstream samples `RGBA float` in
    /// linear light and writes premultiplied `RaGaBaA float`. That is this dispatcher's convention
    /// for every filter, not a choice made here. The operator's shape survives it — the output is a
    /// position within the local envelope, and the envelope is measured in the same units as the
    /// pixel — but the numbers are not GEGL's.
    ///
    /// # Radius is free here and costly upstream
    ///
    /// Upstream's blurb says increasing the radius *"increases the runtime"*, which is true of
    /// upstream and not of us: it is a `GEGL_OP_AREA_FILTER` whose `prepare` grows the required
    /// input rectangle by the radius, so a bigger radius fetches more tiles. The work per pixel is
    /// `samples * iterations` either way. We read the whole layer from memory, so the full
    /// `2..=6000` range is accepted rather than being capped at
    /// [`crate::filters::MAX_FILTER_RADIUS`] — that cap exists for the convolution-shaped filters,
    /// where the radius really is the work.
    Stress {
        #[serde(default = "crate::command::stress_radius")]
        radius: u32,
        #[serde(default = "crate::command::stress_samples")]
        samples: u32,
        #[serde(default = "crate::command::stress_iterations")]
        iterations: u32,
        #[serde(default)]
        enhance_shadows: bool,
    },
    /// Shifts the pixels by a whole number of pixels, optionally wrapping at the borders.
    ///
    /// # Vendored, so read rather than derived
    ///
    /// `gimp:offset` is one of GIMP's own operations, so
    /// `app/operations/gimpoperationoffset.c` is in the tree. Blurb: `Shift the pixels, optionally
    /// wrapping them at the borders`. Four properties, all read:
    ///
    /// - `x`, `y` — `g_param_spec_int` over `G_MININT, G_MAXINT, 0`, so signed and unbounded
    /// - `type` — `GimpOffsetType`, three members
    /// - `color` — `gimp_param_spec_color_from_string`, the fill for [`OffsetType::Color`]
    ///
    /// # The type chooses the NORMALISATION, not just the fill
    ///
    /// This is the reading worth having. `gimp_operation_offset_get_offset` ends:
    ///
    /// ```c
    /// if (offset->type == GIMP_OFFSET_WRAP_AROUND)
    ///   {
    ///     *x %= bounds.width;   if (*x < 0) *x += bounds.width;
    ///     *y %= bounds.height;  if (*y < 0) *y += bounds.height;
    ///   }
    /// else
    ///   {
    ///     *x = CLAMP (*x, -bounds.width,  +bounds.width);
    ///     *y = CLAMP (*y, -bounds.height, +bounds.height);
    ///   }
    /// ```
    ///
    /// So wrapping takes a **positive modulo** into `0..extent`, which makes a negative offset
    /// exactly equal to its positive complement — offsetting by −1 IS offsetting by `width − 1`.
    /// The other two **clamp to one full extent**, so an offset of `width` pushes the whole image
    /// out and leaves the canvas entirely background.
    ///
    /// # The zero check happens AFTER normalisation
    ///
    /// Upstream calls `get_offset` first and only then tests `if (x == 0 && y == 0)`, returning the
    /// input untouched. The order matters: under wrapping an offset of exactly `width` normalises
    /// to 0 and so passes through, while under a fill type it clamps to `width` and vacates
    /// everything. The same number, two opposite results, decided by the type.
    ///
    /// Distinct from [`Filter::Shift`], which displaces each line by a random amount. Upstream
    /// shipping this operation separately is what excluded the uniform reading of `shift` back in
    /// cycle 68.
    Offset {
        #[serde(default)]
        x: i32,
        #[serde(default)]
        y: i32,
        #[serde(default)]
        offset_type: crate::command::OffsetType,
        #[serde(default = "crate::command::white")]
        color: Pixel,
    },
    TileSeamless,
    ConvolutionMatrix {
        /// `a1..e5` in ROW-MAJOR order: `[a1, b1, c1, d1, e1, a2, ...]`.
        #[serde(default = "crate::command::identity_kernel")]
        matrix: [f64; 25],
        #[serde(default = "crate::command::unit_one")]
        divisor: f64,
        #[serde(default)]
        offset: f64,
        #[serde(default)]
        normalise: bool,
        #[serde(default)]
        alpha_weighting: bool,
        #[serde(default)]
        border: ConvolutionBorder,
        /// Red, green, blue, alpha — see the variant on why `Gr_ey` is not a fifth.
        #[serde(default = "crate::command::colour_channels")]
        channels: [bool; 4],
    },
    RedEyeRemoval {
        #[serde(default = "crate::command::unit_half")]
        threshold: f64,
    },
    Deinterlace {
        #[serde(default)]
        keep: DeinterlaceField,
    },
    VideoDegradation {
        /// K.17f: was `Staggered`, our enum's first variant. Upstream declares
        /// `GEGL_VIDEO_DEGRADATION_TYPE_STRIPED` — the **third** of its nine.
        #[serde(default = "crate::command::default_video_pattern")]
        pattern: VideoPattern,
        /// K.17f: was a bare default, i.e. `false`. Upstream declares `TRUE`, blurbed "Whether the
        /// function adds the result to the original image" — so by default the degradation was
        /// REPLACING the image rather than being added to it.
        #[serde(default = "crate::command::yes")]
        additive: bool,
        #[serde(default)]
        rotated: bool,
    },
    Slur {
        #[serde(default)]
        amount: f32,
        #[serde(default)]
        seed: u32,
    },
    NoiseCieLch {
        #[serde(default)]
        lightness: f64,
        #[serde(default)]
        chroma: f64,
        #[serde(default)]
        hue: f64,
        #[serde(default)]
        seed: u32,
    },
    MotionBlurZoom {
        #[serde(default = "crate::command::unit_half")]
        center_x: f64,
        #[serde(default = "crate::command::unit_half")]
        center_y: f64,
        #[serde(default = "crate::command::default_zoom_factor")]
        factor: f64,
    },
    MotionBlurCircular {
        #[serde(default = "crate::command::unit_half")]
        center_x: f64,
        #[serde(default = "crate::command::unit_half")]
        center_y: f64,
        #[serde(default = "crate::command::default_circular_angle")]
        angle: f64,
    },
    Vignette {
        #[serde(default)]
        shape: FocusShape,
        #[serde(default = "crate::command::unit_half")]
        x: f64,
        #[serde(default = "crate::command::unit_half")]
        y: f64,
        /// K.17f: was 1.0. Upstream declares `1.2`, which reaches a fifth beyond the EDGE
        /// MIDPOINT — not beyond the corner. A square canvas puts its corner at `sqrt(2)` times
        /// the half-width, about 1.414, so 1.2 leaves the corner crushed and spares the midpoints.
        /// Measured, after a first version of this comment claimed the corner.
        ///
        /// **And the unit is the half-WIDTH, which upstream's own description contradicts.** It
        /// reads "portion of half image diagonal", and that describes a DEAD line: `process()`
        /// initialises `length` to `hypot (width, height) / 2` at its declaration and then
        /// unconditionally overwrites it with `bounds->width / 2.0` nine lines later, with no read
        /// in between. Our half-width convention matches the code; the description is stale.
        #[serde(default = "crate::command::default_vignette_radius")]
        radius: f64,
        #[serde(default = "crate::command::unit_one")]
        proportion: f64,
        #[serde(default)]
        squeeze: f64,
        #[serde(default)]
        rotation: f64,
        /// K.17f: was 0.5. Upstream declares `0.8`, and the pair matters: the falloff spans
        /// `radius * (1 - softness)` to `radius`, so upstream's defaults ramp from 0.24 to 1.2 of
        /// the half-width — a far wider, softer gradient than our 0.5-to-1.0.
        #[serde(default = "crate::command::default_vignette_softness")]
        softness: f64,
        /// K.17f: was 1.0, which is the LINEAR special case — upstream calls this property
        /// "Falloff linearity" and defaults it to `2.0`, so ours defaulted to the one value that
        /// takes the curve out of the picture.
        #[serde(default = "crate::command::default_vignette_gamma")]
        gamma: f64,
        /// What the vignette darkens TOWARD (K.17).
        ///
        /// `gegl:vignette` declares `property_color (color, _("Color"), "black")`, and this product
        /// had no colour at all — the blend was hard-coded to zero. **Black is the default, so a
        /// document that omits this field renders exactly as it did before**, which is the property
        /// the tests pin: `base * keep + colour * darkening` reduces to `base * keep` when the
        /// colour is zero.
        ///
        /// Alpha is carried from the source pixel, not from this colour: a vignette tints, it does
        /// not punch holes, and upstream's own buffer keeps the alpha channel untouched.
        #[serde(default = "crate::command::default_vignette_color")]
        color: Pixel,
    },
    Supernova {
        #[serde(default = "crate::command::unit_half")]
        center_x: f64,
        #[serde(default = "crate::command::unit_half")]
        center_y: f64,
        #[serde(default = "crate::command::default_nova_radius")]
        radius: u32,
        #[serde(default = "crate::command::white")]
        color: Pixel,
        #[serde(default = "crate::command::default_nova_spokes")]
        spokes: u32,
        #[serde(default)]
        random_hue: f64,
    },
    LensFlare {
        #[serde(default = "crate::command::default_flare_center")]
        x: f64,
        #[serde(default = "crate::command::default_flare_center")]
        y: f64,
    },
    LongShadow {
        #[serde(default = "crate::command::default_long_shadow_angle")]
        angle: f64,
        #[serde(default = "crate::command::default_long_shadow_length")]
        length: u32,
        #[serde(default = "crate::command::black")]
        color: Pixel,
    },
    DropShadow {
        #[serde(default = "crate::command::default_shadow_offset")]
        offset_x: i32,
        #[serde(default = "crate::command::default_shadow_offset")]
        offset_y: i32,
        #[serde(default = "crate::command::default_shadow_blur")]
        radius: u32,
        #[serde(default = "crate::command::black")]
        color: Pixel,
        #[serde(default = "crate::command::default_shadow_opacity")]
        opacity: f64,
    },
    Bloom {
        /// Luminance above which a pixel contributes, on 0..1.
        #[serde(default = "crate::command::unit_half")]
        threshold: f64,
        /// How far the spill reaches, in pixels.
        #[serde(default = "crate::command::default_bloom_radius")]
        radius: u32,
        /// How much of the spill is added back. 0 is the identity.
        ///
        /// K.17f: was 1.0, the top of our own 0..1 unit. Upstream declares `50.0` on
        /// `ui_range (0.0, 100.0)`, which is **0.5** here — so ours defaulted to double its glow.
        ///
        /// **The scaled reading is the one to trust.** Unconverted, audit4 reported this as
        /// "upstream 50.0 vs ours 1.0", a factor of fifty; in our unit it is a factor of two. That
        /// gap is what made cycle 21 add `DEFAULT_UNITS` — a reader triaging by apparent size would
        /// have started here instead of on the real outliers.
        #[serde(default = "crate::command::unit_half")]
        strength: f64,
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
        /// K.17f: was a bare default, i.e. `0.0` — **no compression at all**, so the midtones this
        /// parameter exists to preserve were not preserved. Upstream declares `50.0` on
        /// `value_range (0.0, 100.0)`.
        #[serde(default = "crate::command::default_sh_compress")]
        compress: f32,
        /// "Adjust saturation of shadows", 0..100 — how much of the original saturation to
        /// restore after the tone change, which desaturates by compressing channel differences.
        ///
        /// Defaults to 100 (fully restore). Zero would leave the lifted region washed out, which
        /// is a legitimate look but not the one an unset parameter should produce.
        ///
        /// Upstream declares `100.0` here, which ours already matched — but through a helper SHARED
        /// with `highlights_ccorrect`, where upstream declares 50.0. Split at K.17f so the two
        /// cannot drift back together.
        #[serde(default = "crate::command::default_sh_shadows_ccorrect")]
        shadows_ccorrect: f32,
        /// "Adjust saturation of highlights", 0..100. Same meaning and default.
        /// K.17f: was 100.0, from a helper shared with `shadows_ccorrect`. Upstream declares
        /// `50.0`.
        ///
        /// **The asymmetry is upstream's own and it is the point.** Lifting shadows desaturates
        /// them more than pulling highlights down does, so shadows get the full correction and
        /// highlights half. One shared value made them symmetric — the same shape as
        /// `gegl:diffraction-patterns`' three shared helpers.
        #[serde(default = "crate::command::default_sh_highlights_ccorrect")]
        highlights_ccorrect: f32,
    },
    /// An arbitrary transfer curve through user-placed control points.
    ///
    /// `Levels` above expresses a black point, a white point and a gamma, which cannot describe a curve
    /// that rises and falls. This can. The points are the curve's definition rather than a sampled
    /// table, so a document stays editable and re-samples at whatever precision it renders at.
    Curves {
        points: Vec<crate::CurvePoint>,
        /// The red curve, applied BEFORE `points`. `None` is the identity.
        ///
        /// # K.16: five curve slots, not a channel selector
        ///
        /// Upstream holds five curves and applies them all in one pass --
        /// `gimp_curve_map_pixels(curve_colors, curve_red, curve_green, curve_blue, curve_alpha, …)`
        /// -- so the `channel` property on `GimpCurvesConfig` is dialog state choosing which slot the
        /// UI edits, not an operation parameter. Measured at cycle 108: `"channel"` is declared zero
        /// times in `gimpoperationcurves.c`.
        ///
        /// The composition order is read from `gimpcurve-map.c`'s default case, and the file states
        /// it twice:
        ///
        /// ```c
        /// dest[0] = map (curve_colors, map (curve_red,   src[0]));
        /// dest[1] = map (curve_colors, map (curve_green, src[1]));
        /// dest[2] = map (curve_colors, map (curve_blue,  src[2]));
        /// /* don't apply the colors curve to the alpha channel */
        /// dest[3] = map (curve_alpha, src[3]);
        /// ```
        ///
        /// So the per-channel curve is INNER and `points` is OUTER, and the colours curve never
        /// touches alpha.
        ///
        /// `points` keeps its existing meaning as the colours curve, which is exactly what this
        /// variant already did -- it applied one table to R, G and B and left alpha alone. So the
        /// four new slots default to the identity and an existing saved `Curves` is unchanged.
        #[serde(default)]
        red: Option<Vec<crate::CurvePoint>>,
        /// The green curve, applied before `points`. `None` is the identity.
        #[serde(default)]
        green: Option<Vec<crate::CurvePoint>>,
        /// The blue curve, applied before `points`. `None` is the identity.
        #[serde(default)]
        blue: Option<Vec<crate::CurvePoint>>,
        /// The alpha curve. `None` is the identity, and `points` is NEVER applied to alpha.
        #[serde(default)]
        alpha: Option<Vec<crate::CurvePoint>>,
        /// Which colour space the curves are applied in.
        ///
        /// # The default preserves our behaviour and diverges from upstream's
        ///
        /// Upstream's declared default is [`crate::TrcType::Linear`]; this variant has always
        /// applied its table directly to the sRGB-encoded bytes, which is exactly
        /// [`crate::TrcType::NonLinear`]. So the field defaults to `NonLinear` and a saved `Curves`
        /// keeps its meaning.
        ///
        /// That is the same judgement as `Levels`' clamp flags and the opposite of `Threshold`'s
        /// `channel`: the test is whether our old behaviour matches ANY upstream configuration. It
        /// does here — `NonLinear` is a real setting, not a defect — so there is nothing to correct
        /// and the parity requirement is only that upstream's default be **expressible**, which it
        /// now is.
        #[serde(default = "crate::command::non_linear_trc")]
        trc: crate::command::TrcType,
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
        /// Which colour model to screen in, and so how many screens are used.
        ///
        /// K.16. Defaults to [`crate::HalftoneColorModel::BlackOnWhite`], the single luminance
        /// screen this filter has always been, so a saved `Halftone` keeps its meaning.
        #[serde(default)]
        color_model: crate::command::HalftoneColorModel,
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
pub(crate) const PRECISION_NATIVE_FILTERS: &[&str] = &[
    "fattal02",
    "invert",
    "invert_linear",
    "mantiuk06",
    "reinhard05",
    "rgb_clip",
];

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
    "bloom",
    "semi_flatten",
    "edge_sobel",
    "c2g",
    "fattal02",
    "mantiuk06",
    "reinhard05",
    "stress",
    "offset",
    "threshold_alpha",
    "tile_seamless",
    "convolution_matrix",
    "red_eye_removal",
    "deinterlace",
    "video_degradation",
    "slur",
    "noise_cie_lch",
    "motion_blur_zoom",
    "motion_blur_circular",
    "vignette",
    "supernova",
    "lens_flare",
    "long_shadow",
    "drop_shadow",
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
    /// Places an alignment guide (L.1).
    ///
    /// `style` defaults to `Normal`, the only style a user places by hand and the only one tools
    /// snap to. A mode that draws its own construction lines passes one of the others, and those
    /// are drawn but never snapped to.
    AddGuide {
        id: crate::GuideId,
        orientation: crate::GuideOrientation,
        position: i32,
        #[serde(default)]
        style: crate::GuideStyle,
    },
    MoveGuide {
        id: crate::GuideId,
        position: i32,
    },
    RemoveGuide {
        id: crate::GuideId,
    },
    /// Places a position whose composited colour the user watches (L.1).
    AddSamplePoint {
        id: crate::SamplePointId,
        x: i32,
        y: i32,
    },
    MoveSamplePoint {
        id: crate::SamplePointId,
        x: i32,
        y: i32,
    },
    RemoveSamplePoint {
        id: crate::SamplePointId,
    },
    /// Replaces the whole guide settings record (L.1).
    ///
    /// One command rather than five toggles because the five are read together by the snap and by
    /// the renderer, and a per-flag command set would let a caller write a half-updated record
    /// across two history entries.
    SetGuideSettings {
        settings: crate::GuideSettings,
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
    /// Places the caret in a text node, optionally with a selection (L.7).
    ///
    /// The caret is EDITOR state, not document content, so this changes nothing a save would
    /// write — see `crate::text_caret` for why upstream keeps it on the tool too.
    SetTextCaret {
        id: LayerId,
        insert: usize,
        /// Equal to `insert` when nothing is selected.
        #[serde(default)]
        anchor: Option<usize>,
    },
    /// Moves the caret in a text node (L.7).
    MoveTextCaret {
        id: LayerId,
        movement: crate::CaretMovement,
        count: i32,
        #[serde(default)]
        extend: bool,
    },
    /// Replaces the caret's selection with `text`, or inserts at the caret (L.7).
    InsertAtTextCaret {
        id: LayerId,
        text: String,
    },
    /// Deletes the selection, or one position in `direction` when nothing is selected (L.7).
    ///
    /// One command with a sign rather than two: a selection is deleted whichever way the sign
    /// points, so the direction only decides which side of a BARE caret goes.
    DeleteAtTextCaret {
        id: LayerId,
        direction: i32,
    },
    /// Seamless clone: copy `src` to `(dst_x, dst_y)` on the active layer with its boundary made
    /// to disappear (L.6).
    ///
    /// `max_refine_scale` is upstream's only option, `0, 50, 5` — the refinement of the
    /// interpolation mesh, here the number of boundary samples taken along each edge.
    ///
    /// **Not a Poisson blend**, despite how this gap was filed: the single readable parameter names
    /// an interpolation MESH, which a Poisson solve does not have. See `crate::seamless_clone`.
    SeamlessClone {
        src: Rect,
        dst_x: i32,
        dst_y: i32,
        #[serde(default = "crate::command::default_seamless_clone_refine_scale")]
        max_refine_scale: u32,
    },
    /// Paint select: rough strokes refine the EXISTING selection (L.5).
    ///
    /// The neighbouring tool to `SelectForeground` upstream, and deliberately a different shape.
    /// That one takes both labels at once and ignores what is already selected; this one carries
    /// **one label per stroke** and works against the selection as it stands, because upstream
    /// resets its trimap to grey on every button press and latches the operation at that moment.
    /// `mode` is that label: `Add` scribbles the object, anything else scribbles the background.
    ///
    /// `stroke_width` is the diameter of the round dab each scribble point paints, 1..=6000 with
    /// upstream's default of 50.
    PaintSelect {
        scribbles: Vec<(u32, u32)>,
        #[serde(default = "crate::command::default_paint_select_stroke_width")]
        stroke_width: u32,
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
    /// Handle transform of the active layer: 1 to 4 pinned handles carry their source positions to
    /// their destinations (L.3).
    ///
    /// **The number of handles is the transform class** — one translates, two give rotation plus
    /// uniform scale, three add shear and non-uniform scale, four give full perspective. That is
    /// read from `gimptoolhandlegrid.c`'s `switch (n_handles)`, which moves the other corners
    /// before the same four-point solver runs. There is deliberately no `n_handles` field: the
    /// count is `src.len()`, and a second copy of it could disagree with the list.
    ///
    /// Distinct from `Perspective`, whose source quad is always the layer's own corners, and from
    /// `NPointTransform`, which warps smoothly through any number of points rather than applying
    /// one matrix.
    HandleTransform {
        src: Vec<(f32, f32)>,
        dst: Vec<(f32, f32)>,
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

/// `gegl:shadows-highlights`' `compress`. K.17f: was a bare 0.0.
pub(crate) fn default_sh_compress() -> f32 {
    50.0
}

/// `gegl:shadows-highlights`' `shadows_ccorrect` — upstream's full correction.
pub(crate) fn default_sh_shadows_ccorrect() -> f32 {
    100.0
}

/// `gegl:shadows-highlights`' `highlights_ccorrect` — HALF, not the full correction shadows get.
///
/// This and the one above replaced a single `full_colour_correction` helper returning 100.0, which
/// both `ccorrect` fields shared. Splitting it left that helper with no callers and the compiler
/// said so, which is the cheapest possible confirmation that the sharing was the whole bug: a
/// helper used by exactly the two parameters upstream gives DIFFERENT values to.
pub(crate) fn default_sh_highlights_ccorrect() -> f32 {
    50.0
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
    TreatAsThis,
    /// "Change to this": replace the grey with the configured hue and saturation outright, with no
    /// rotation applied.
    ///
    /// K.17f: the type's `Default` moved here from `TreatAsThis`, so the bare-default path and
    /// `default_color_rotate_gray_mode` agree with upstream instead of disagreeing with each other.
    #[default]
    ChangeToThis,
}

/// Default `opacity_threshold`: the far end of the range, so the ramp spans everything below it.
/// `gegl:focus-blur`'s `radius`, a fraction of the width. K.17f: was 0.5.
/// `gegl:color-rotate`'s `src_to` and `dest_to` — the same 90.0 for both. K.17f: was a bare 0.0,
/// which made each range empty.
pub(crate) fn default_color_rotate_to() -> f32 {
    90.0
}

/// `gegl:color-rotate`'s `gray_mode`. K.17f: was `TreatAsThis`, its enum's first value.
pub(crate) fn default_color_rotate_gray_mode() -> GrayMode {
    GrayMode::ChangeToThis
}

pub(crate) fn default_focus_blur_radius() -> f32 {
    0.75
}

/// `gegl:focus-blur`'s `focus`, the focus region's inner limit. K.17f: was a bare 0.0.
pub(crate) fn default_focus_blur_focus() -> f32 {
    0.25
}

/// `gegl:vignette`'s `radius`, as a portion of the half-WIDTH. K.17f: was 1.0.
pub(crate) fn default_vignette_radius() -> f64 {
    1.2
}

/// `gegl:vignette`'s `softness`. K.17f: was 0.5.
pub(crate) fn default_vignette_softness() -> f64 {
    0.8
}

/// `gegl:vignette`'s `gamma`, its falloff linearity. K.17f: was 1.0, the linear case.
pub(crate) fn default_vignette_gamma() -> f64 {
    2.0
}

/// `gegl:wind`'s `threshold`. K.17f: was a bare 0.
pub(crate) fn default_wind_threshold() -> u8 {
    10
}

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

/// Vignette's default colour: opaque black, which is `gegl:vignette`'s own `"black"` (K.17).
///
/// Opaque rather than transparent because the alpha of this colour is never consulted — the filter
/// carries the SOURCE pixel's alpha through, as upstream does. A transparent default would read as
/// meaningful and be ignored, which is worse than a value that is simply never used.
pub(crate) fn default_vignette_color() -> Pixel {
    Pixel::rgba(0, 0, 0, 255)
}

/// `gegl:mirrors` declares `property_int (n_segs, _("Mirrors"), 6)` (K.17c).
pub(crate) fn default_mirrors() -> u32 {
    6
}

/// `gegl:mirrors` declares `property_double (input_scale, _("Zoom"), 100.0)` and then divides by
/// 100 at the call site, so the stored value is a percentage and 100 means "unchanged" (K.17c).
pub(crate) fn default_mirror_input_scale() -> f64 {
    100.0
}

/// 0.5 on the f64 unit scale. Distinct from `half`, which is f32 -- the two scales are not
/// interchangeable and the compiler is what caught the mix.
pub(crate) fn unit_half() -> f64 {
    0.5
}

/// Grid spacing for both superpixel operations. Ours -- nothing upstream states one.
/// `gegl:video-degradation`'s `pattern` — upstream's STRIPED, the third of its nine variants.
pub(crate) fn default_video_pattern() -> VideoPattern {
    VideoPattern::Striped
}

/// `gegl:tile-paper`'s `move_rate`, a percentage of the tile's own size. K.17f: was a bare 0.0,
/// i.e. no movement at all, which turned the filter's whole effect off by default.
pub(crate) fn default_tile_paper_move() -> f64 {
    25.0
}

/// `gegl:tile-paper`'s `fractional_type` — upstream's FORCE, the last of its three variants.
pub(crate) fn default_fractional_pixels() -> FractionalPixels {
    FractionalPixels::Force
}

/// `gegl:tile-paper`'s `background_type` — upstream's INVERT, the second of its four.
pub(crate) fn default_paper_background() -> PaperBackground {
    PaperBackground::InvertedImage
}

pub(crate) fn default_cluster_size() -> u32 {
    32
}

/// SLIC's colour-against-space weight. Ours; the published algorithm's own examples use 10.
/// `gegl:slic`'s `compactness`. K.17f: was 10.0, half upstream's.
///
/// Upstream declares it as a `property_int` on `value_range (1, 40)`, so 20 is the MIDDLE of its
/// range and 10 was a quarter of the way up. We carry it as `f64` because the clustering weights
/// it continuously; the default is upstream's integer either way.
pub(crate) fn default_compactness() -> f64 {
    20.0
}

/// SLIC converges in a handful of passes, so ten is past the useful range without being slow.
/// `gegl:slic`'s `iterations`. K.17f: was 10 — **ten times upstream's**.
///
/// Upstream declares `1` on `value_range (1, 30)` with `ui_range (1, 15)`. This is the one
/// correction in K.17f that makes the filter do LESS work rather than different work: ten SLIC
/// refinement passes where upstream does one, on every call that omitted the field.
pub(crate) fn default_slic_iterations() -> u32 {
    1
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

/// `gegl:sinus`' `color1`, read as `property_color (color1, _("Color 1"), "yellow")`.
pub(crate) fn sinus_yellow() -> Pixel {
    Pixel::rgba(255, 255, 0, 255)
}

/// `gegl:sinus`' `color2`, read as `property_color (color2, _("Color 2"), "blue")`.
pub(crate) fn sinus_blue() -> Pixel {
    Pixel::rgba(0, 0, 255, 255)
}

/// `gegl:sinus`' own `perturbation` default: its boolean is TRUE, i.e. distorted.
pub(crate) fn default_sinus_perturbation() -> SinusPerturbation {
    SinusPerturbation::Distorted
}

/// K.17f: was 2.0. Upstream declares `1.0` on `value_range (0.0, 15.0)`.
pub(crate) fn default_sinus_complexity() -> f64 {
    1.0
}

/// Pixels per cycle for the linear sinusoid. Ours -- nothing upstream states one.
/// `gegl:linear-sinusoid`'s `x_period` and `y_period` — the same 128.0 for both.
///
/// K.17f: was 32.0. **And the unit needed checking rather than assuming**, because `gegl:sinus`
/// two cycles earlier was the trap: its `x_scale` is a frequency on a NORMALISED coordinate, so no
/// number could reconcile it and it had to be filed as K.17g. This one is different — upstream
/// declares `ui_meta ("unit", "pixel-distance")` and we compute `TAU / x_period`, so both sides
/// carry a period in PIXELS and the correction is just the number.
pub(crate) fn default_sinusoid_period() -> f64 {
    128.0
}

/// The familiar 4x4 Bayer matrix. Ours -- nothing upstream states an order.
pub(crate) fn default_bayer_order() -> u32 {
    2
}

// `gegl:diffraction-patterns`' twelve defaults, read from
// `operations/common-gpl3+/diffraction-patterns.c`.
//
// This block used to be headed "Diffraction defaults. Ours -- the plug-in that declared them is
// deleted." That was true when GIMP's plug-in was the only source, and it stopped being true at
// cycle 0 when GEGL was fetched. **Nothing here is ours any more**, which is why the line is gone
// rather than edited: clippy flagged it as a doc comment with an empty line after it, and the claim
// it carried is the one this change disproves.
//
// **These are ONE tuned preset, not twelve independent choices.** Every value is an odd
// non-round number -- 0.815, 1.221, 37.126, -0.473 -- because together they make one particular
// diffraction figure that upstream ships as its opening picture. K.17f corrected eleven of the
// twelve; see the doc comments on the fields for what each replaced.
//
// **And the per-channel split is the point of the filter.** Diffraction fringes are coloured
// because red, green and blue diffract at different frequencies. Our old defaults used ONE shared
// helper per group, seeded from the red channel, so all three channels got the same frequency and
// the same contour count -- which produces a GREY pattern and defeats the operator.

/// Red light frequency. The one value our old shared helper happened to get right.
pub(crate) fn default_diffraction_frequency_red() -> f64 {
    0.815
}

/// Green light frequency. K.17f: was 0.815, the red value.
pub(crate) fn default_diffraction_frequency_green() -> f64 {
    1.221
}

/// Blue light frequency. K.17f: was 0.815, the red value.
pub(crate) fn default_diffraction_frequency_blue() -> f64 {
    1.123
}

/// Red contour count. K.17f: was 0.819 — upstream declares 0.821, so the old value was also a
/// transcription slip, not only a shared helper.
pub(crate) fn default_diffraction_contours_red() -> f64 {
    0.821
}

/// Green contour count. Upstream declares the same value as red here; blue is the one that differs.
pub(crate) fn default_diffraction_contours_green() -> f64 {
    0.821
}

/// Blue contour count. K.17f: was 0.819.
pub(crate) fn default_diffraction_contours_blue() -> f64 {
    0.974
}

/// Red sharp-edge count. K.17f: was a bare default, i.e. 0.0.
pub(crate) fn default_diffraction_edges_red() -> f64 {
    0.610
}

/// Green sharp-edge count. K.17f: was a bare default, i.e. 0.0.
pub(crate) fn default_diffraction_edges_green() -> f64 {
    0.677
}

/// Blue sharp-edge count. K.17f: was a bare default, i.e. 0.0.
pub(crate) fn default_diffraction_edges_blue() -> f64 {
    0.636
}

/// K.17f: was 1.0, i.e. full. Upstream declares 0.066 on `value_range (0.0, 1.0)` — the figure is
/// a faint one, and 1.0 washes it out.
pub(crate) fn default_diffraction_brightness() -> f64 {
    0.066
}

/// K.17f: was a bare default, i.e. 0.0 — no scattering at all. Upstream declares 37.126 on
/// `value_range (0.0, 100.0)`, described as "speed vs. quality".
pub(crate) fn default_diffraction_scattering() -> f64 {
    37.126
}

/// K.17f: was a bare default, i.e. 0.0, the neutral middle. Upstream declares -0.473 on
/// `value_range (-1.0, 1.0)`.
pub(crate) fn default_diffraction_polarization() -> f64 {
    -0.473
}

/// Pixels per noise lattice cell. Ours -- nothing upstream states a scale.
pub(crate) fn default_noise_scale() -> f64 {
    32.0
}

/// How far bloom spills, in pixels. Ours -- nothing upstream states a radius.
pub(crate) fn default_bloom_radius() -> u32 {
    10
}

/// Opaque white, `Mosaic`'s default highlight.
/// Both Sobel directions default ON: the dialog offers two checkboxes and an operation that
/// computed nothing by default would have no edges to show.
/// Upstream's `high` default is 1.0, the top of its 0..1 range.
/// Our own prior behaviour, which is upstream's `NonLinear`. Not upstream's DEFAULT -- see the
/// field's documentation for why the two differ.
pub(crate) fn non_linear_trc() -> TrcType {
    TrcType::NonLinear
}

pub(crate) fn full_byte() -> u8 {
    u8::MAX
}

/// `gegl:mosaic`'s own `tile_type` default, read from `operations/common-gpl3+/mosaic.c`.
pub(crate) fn default_mosaic_primitive() -> TilingPrimitive {
    TilingPrimitive::Hexagons
}

/// `gegl:mosaic`'s `tile_height`. Its `value_range` floor is 1.0, so the old `0.0` was illegal.
pub(crate) fn default_mosaic_tile_height() -> f64 {
    4.0
}

/// `gegl:mosaic`'s `tile_neatness`: a deliberate deviation from the exact lattice.
pub(crate) fn default_mosaic_neatness() -> f64 {
    0.65
}

/// `gegl:mosaic`'s `light_dir`, in degrees counter-clockwise.
pub(crate) fn default_mosaic_light_direction() -> f64 {
    135.0
}

/// `gegl:mosaic`'s `color_variation`.
pub(crate) fn default_mosaic_color_variation() -> f64 {
    0.2
}

pub(crate) fn yes() -> bool {
    true
}

/// `property_int (samples, _("Samples"), 4)` in `gegl/operations/common/c2g.c`.
///
/// Four, not `stress`'s five, and its range starts at 1 where `stress`'s starts at 2. Two
/// operators that share `envelopes.h` and declare different numbers for the same-named property.
pub(crate) fn c2g_samples() -> u32 {
    4
}

/// `property_int (iterations, _("Iterations"), 10)`, same property block — twice `stress`'s 5.
pub(crate) fn c2g_iterations() -> u32 {
    10
}

/// `property_double (contrast, _("Contrast"), 0.1)` in `gegl/operations/common/mantiuk06.c`.
///
/// Its own, not shared: this variant reuses [`fattal_saturation`] because both upstream blocks
/// declare 0.8 for saturation, but the contrast default belongs to this operator alone.
pub(crate) fn mantiuk_contrast() -> f64 {
    0.1
}

/// `property_double (beta, _("Beta"), 0.9)` in `gegl/operations/common/fattal02.c`.
pub(crate) fn fattal_beta() -> f64 {
    0.9
}

/// `property_double (saturation, _("Saturation"), 0.8)`, same property block.
pub(crate) fn fattal_saturation() -> f64 {
    0.8
}

/// `property_int (radius, _("Radius"), 300)` in `gegl/operations/common/stress.c`.
pub(crate) fn stress_radius() -> u32 {
    300
}

/// `property_int (samples, _("Samples"), 5)`, same property block.
pub(crate) fn stress_samples() -> u32 {
    5
}

/// `property_int (iterations, _("Iterations"), 5)`, same property block.
///
/// Separate from [`stress_samples`] despite the equal value: they are two upstream properties that
/// happen to share a default, and one function for both would silently move the other if upstream's
/// ever changed.
pub(crate) fn stress_iterations() -> u32 {
    5
}

/// Read verbatim from `gimppaintselectoptions.c`'s `stroke-width` declaration: `1, 6000, 50`.
///
/// A serde default rather than a bare field so a caller that omits it gets the tool's own default
/// instead of `0`, which the range check would then refuse.
pub(crate) fn default_paint_select_stroke_width() -> u32 {
    crate::PAINT_SELECT_DEFAULT_STROKE_WIDTH
}

/// Read verbatim from `gimpseamlesscloneoptions.c`'s `max-refine-scale`: `0, 50, 5`.
///
/// A serde default rather than a bare field because `0` is a LEGAL value here — the coarsest mesh —
/// so an omitted field cannot be told from a deliberate zero without one.
pub(crate) fn default_seamless_clone_refine_scale() -> u32 {
    crate::SEAMLESS_CLONE_DEFAULT_REFINE_SCALE
}

/// Read verbatim from `gimpoperationthresholdalpha.c`: `0.0, 1.0, 0.5`.
pub(crate) fn default_alpha_threshold() -> f64 {
    0.5
}

pub(crate) fn white() -> Pixel {
    Pixel::rgba(255, 255, 255, 255)
}

/// Opaque black, `Mosaic`'s default shadow.
/// The identity kernel: 1 at the centre, 0 elsewhere. A default that changes nothing is the only
/// honest one for a user-supplied matrix.
pub(crate) fn identity_kernel() -> [f64; 25] {
    let mut kernel = [0.0; 25];
    kernel[12] = 1.0;
    kernel
}

/// Red, green and blue on, alpha off — convolving alpha by default would alter a layer's shape as
/// well as its colour, which is not what reaching for a kernel usually means.
pub(crate) fn colour_channels() -> [bool; 4] {
    [true, true, true, false]
}

/// A recorded CHOICE. The propgui reads the range `-0.5..1.0` but states no default.
pub(crate) fn default_zoom_factor() -> f64 {
    0.1
}

/// A recorded CHOICE. The propgui forces `0..360` but states no default.
pub(crate) fn default_circular_angle() -> f64 {
    5.0
}

/// `_Radius:` has no readable default; 20 is a recorded choice.
pub(crate) fn default_nova_radius() -> u32 {
    20
}

/// `_Spokes:` has no readable default; 8 is a recorded choice.
pub(crate) fn default_nova_spokes() -> u32 {
    8
}

/// A recorded CHOICE. The po strings give `_X:` and `_Y:` but no default, so 0 puts the flare at the
/// top-left corner and leaves the caller to place it.
pub(crate) fn default_flare_center() -> f64 {
    0.0
}

/// A recorded CHOICE, not a reading: 45 degrees is the direction that makes a long shadow
/// recognisable, and no readable source states one.
pub(crate) fn default_long_shadow_angle() -> f64 {
    45.0
}

/// A recorded CHOICE for the same reason.
pub(crate) fn default_long_shadow_length() -> u32 {
    20
}

/// `Offset X` and `Offset Y`, both default 4 in `drop-shadow.scm`.
pub(crate) fn default_shadow_offset() -> i32 {
    4
}

/// `Blur radius`, default 15 in `drop-shadow.scm`.
pub(crate) fn default_shadow_blur() -> u32 {
    15
}

/// `Opacity`, default 60 in `drop-shadow.scm`.
pub(crate) fn default_shadow_opacity() -> f64 {
    60.0
}

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
    Ideal,
    /// Line 765, `_Distorted`. The sum fed back as a phase shift into itself.
    ///
    /// K.17f: the type's own `Default` moved here from `Ideal`, so the bare-default path and
    /// `default_sinus_perturbation` agree with upstream instead of disagreeing with each other.
    #[default]
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

/// One levels slot: the five scalars upstream keeps per channel.
///
/// # K.16: five slots applied in one pass, not a channel selector
///
/// `gimplevelsconfig.c` holds `low_input[5]`, `high_input[5]`, `gamma[5]`, `low_output[5]` and
/// `high_output[5]`, and `gimpoperationlevels.c` applies them all in a single pass. Index **0 is the
/// overall slot** and 1..4 are red, green, blue and alpha. Measured at cycle 108: `"channel"` is
/// declared **zero** times in the operation, so it is dialog state choosing which slot the UI edits.
///
/// The process loop is exactly parallel to curves:
///
/// ```c
/// value = gimp_operation_levels_map (src[channel], ... [channel + 1] ...);
/// /* don't apply the overall curve to the alpha channel */
/// if (channel != ALPHA)
///   value = gimp_operation_levels_map (value, ... [0] ...);
/// ```
///
/// So the per-channel slot is applied FIRST and the overall slot on top of its result, and the
/// overall slot never touches alpha. The same composition rule and the same exclusion as
/// [`Filter::Curves`], stated in the same words.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LevelsSlot {
    pub input_black: u8,
    pub input_white: u8,
    pub gamma: f32,
    pub output_black: u8,
    pub output_white: u8,
}

/// Which shade of grey `gimp:desaturate` reduces a colour to.
///
/// Read from `GimpDesaturateMode` in `libgimpbase/gimpbaseenums.h`, five members in declaration
/// order, each with upstream's own gloss. Blurb: `Turn colors into shades of gray`.
///
/// # Luma and Luminance run the SAME arithmetic in DIFFERENT spaces
///
/// This is the reading worth having. The two share one `case` in
/// `gimpoperationdesaturate.c` — one weighted sum, with the weights taken from the space itself via
/// `babl_space_get_rgb_luminance` — and they are told apart entirely by `prepare`:
///
/// ```c
/// if (desaturate->mode == GIMP_DESATURATE_LUMINANCE)
///   format = babl_format_with_space ("RGBA float", format);     /* linear   */
/// else
///   format = babl_format_with_space ("R'G'B'A float", format);  /* non-linear */
/// ```
///
/// So luminance is the weighted sum of LINEAR light and luma the same sum of the sRGB-encoded
/// values. A reimplementation that gave them different weights, or the same space, would get both
/// wrong.
///
/// # Two different luminance definitions coexist upstream
///
/// These weights come from the space (`babl_space_get_rgb_luminance`, Rec. 709 for sRGB), while
/// `gimp:threshold`'s `LUMINANCE` channel uses the fixed `GIMP_RGB_LUMINANCE` macro
/// (`0.22248840 / 0.71690369 / 0.06060791`), which is **not** Rec. 709 — established at cycle 108.
/// Which definition applies depends on the operation, so neither can be "unified" without breaking
/// one of them.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesaturateMode {
    /// `(max + min) / 2` — HSL's bi-hexcone lightness, in upstream's own words.
    Lightness,
    /// The space's luminance weights applied to the NON-LINEAR values. What this filter has always
    /// done.
    #[default]
    Luma,
    /// `(r + g + b) / 3` — HSI intensity.
    Average,
    /// The same weights applied to LINEAR light. Upstream's default.
    Luminance,
    /// `max(r, g, b)` — HSV's value.
    Value,
}

/// Which colour model `gegl:newsprint` screens in.
///
/// Read from `ColorModel` in `app/propgui/gimppropgui-newsprint.c`, four members in declaration
/// order. **How many screens each model uses is read from the same file's `label_strings` table**,
/// whose non-NULL entries are the channels that get one:
///
/// | model | channel labels | screens |
/// |---|---|---|
/// | `WhiteOnBlack` | `White` | 1 |
/// | `BlackOnWhite` | `Black` | 1 |
/// | `Rgb` | `Red`, `Green`, `Blue` | 3 |
/// | `Cmyk` | `Cyan`, `Magenta`, `Yellow`, `Black` | 4 |
///
/// ```c
/// static const gchar *label_strings[N_COLOR_MODELS][4] =
/// {
///   { NULL,       NULL,          NULL,         N_("White") },
///   { NULL,       NULL,          NULL,         N_("Black") },
///   { N_("Red"),  N_("Green"),   N_("Blue"),   NULL        },
///   { N_("Cyan"), N_("Magenta"), N_("Yellow"), N_("Black") }
/// };
/// ```
///
/// The table also fixes which slot the single-screen models use: **channel 3**, which is why
/// upstream's per-screen property arrays run `pattern2`, `pattern3`, `pattern4`, `pattern` — the
/// UNNUMBERED name is channel 3, the one the one-screen models drive.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HalftoneColorModel {
    /// One screen; light dots on a dark ground.
    WhiteOnBlack,
    /// One screen; dark dots on a light ground. What this filter has always done.
    #[default]
    BlackOnWhite,
    /// Three screens, one per additive channel.
    Rgb,
    /// Four screens, one per subtractive channel.
    Cmyk,
}

/// Which colour space a histogram operation works in.
///
/// Read from `GimpTRCType` in `app/core/core-enums.h`, three members in declaration order. What it
/// selects is the **babl format the operation's pixels arrive in**, from
/// `gimp_operation_point_filter_prepare`'s own switch:
///
/// | value | format | meaning |
/// |---|---|---|
/// | `Linear` | `"RGBA float"` | linear light |
/// | `NonLinear` | `"R'G'B'A float"` | sRGB-encoded, which is what our bytes already hold |
/// | `Perceptual` | `"R~G~B~A float"` | babl's perceptual TRC |
///
/// Neither `gimpoperationcurves.c` nor `gimpoperationlevels.c` has its own `prepare`; both inherit
/// it from `GimpOperationPointFilter`, which carries the property and binds it to the config.
///
/// # Upstream's default is wrong by its own account, and must be reproduced as shipped
///
/// `GIMP_CONFIG_PROP_ENUM(..., "trc", ..., GIMP_TRC_LINEAR, 0)`, above which upstream writes:
/// *"'trc' should default to GIMP_TRC_PERCEPTUAL (cf. #15962). We cannot change it until we
/// implement GEGL op versioning. In GIMP 3.0, calling this op from the public API was always run in
/// linear (#15681)."* So the shipped default is linear, upstream considers that wrong, and it is
/// reproduced as shipped rather than as intended — the same call as `colorize`'s documented
/// luminance-weight quirk.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrcType {
    /// Linear light. Upstream's declared default.
    #[default]
    Linear,
    /// sRGB-encoded, the space our 8-bit buffers already hold.
    NonLinear,
    /// babl's perceptual TRC. **Refused** — see [`crate::CoreError::FilterTrcUnsupported`].
    Perceptual,
}

/// Which quantity a histogram-driven operation reads.
///
/// Read from `GimpHistogramChannel` in `app/core/core-enums.h`, with its explicit `= 0`..`= 6`
/// values, so the order is upstream's own and not inferred.
///
/// # Two of these do not mean what their names suggest
///
/// From `gimpoperationthreshold.c`'s own switch:
///
/// - `Value` is the **MAXIMUM** of red, green and blue — not luminance, not an average
/// - `Rgb` is the **MINIMUM** of the three
///
/// So `Value` and `Rgb` are opposite ends of the same triple, and on a saturated colour they give
/// opposite verdicts. This is exactly the trap K.16's own preamble warns about: AUDIT-4 twice named
/// a gap correctly and described it wrongly from the property name alone.
///
/// `Luminance` uses GIMP's own weights (`0.22248840 / 0.71690369 / 0.06060791`), not Rec. 709.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistogramChannel {
    /// The maximum of red, green and blue.
    #[default]
    Value,
    Red,
    Green,
    Blue,
    Alpha,
    /// GIMP's own luminance weights, not Rec. 709.
    Luminance,
    /// The minimum of red, green and blue.
    Rgb,
}

/// Background fill for [`Filter::Offset`].
///
/// Read from `GimpOffsetType` in `libgimpbase/gimpbaseenums.h` — exactly three members, in
/// declaration order, whose own doc comment calls them "Background fill types for the offset
/// operation".
///
/// The type does more than choose a fill: upstream normalises the offset DIFFERENTLY for
/// [`Self::WrapAround`] than for the other two. See [`Filter::Offset`].
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OffsetType {
    /// Fill the vacated area with the operation's colour.
    #[default]
    Color,
    /// Fill the vacated area with transparency.
    Transparent,
    /// Wrap the image around, so nothing is vacated.
    WrapAround,
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
    ///
    /// K.17f: the type's `Default` moved here from `Right`. Upstream declares
    /// `GEGL_WIND_DIRECTION_LEFT`, which is also the first value of its enum.
    #[default]
    Left,
    /// Line 948.
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
    ///
    /// K.17f: the type's `Default` moved here from `Both`. **Upstream's enum lists `BOTH` first
    /// and then defaults to `LEADING` anyway**, so this is a deliberate choice on its part rather
    /// than the usual first-variant default — and ours happened to be `Both`, its first value.
    #[default]
    Leading,
    /// Line 972.
    Trailing,
    /// Line 973.
    Both,
}

/// `gegl:tile-paper`'s "Fractional Pixels" radio group — lines 325, 327, 329.
///
/// What to do with the partial tiles left when the image is not an exact multiple of the tile size.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FractionalPixels {
    /// Line 325 — fill the remainder with the background.
    Background,
    /// Line 327 — leave the remainder as it was.
    Ignore,
    /// Line 329 — treat the remainder as a tile of its own and slide it too.
    ///
    /// K.17f: the type's `Default` moved here from `Background`, matching upstream's
    /// `GEGL_FRACTIONAL_TYPE_FORCE`.
    #[default]
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
    ///
    /// K.17f: the type's `Default` moved here from `Image`, matching upstream's
    /// `GEGL_BACKGROUND_TYPE_INVERT`.
    #[default]
    InvertedImage,
    /// Line 389 — the original image, unchanged, so the gaps do not read as holes.
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
    Squares,
    /// Line 632.
    /// K.17f: the type's own `Default` moved here from `Squares`, so the bare-default path and
    /// `default_mosaic_primitive` agree with upstream instead of disagreeing with each other.
    #[default]
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
