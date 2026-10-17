//! Paint dynamics presets: a named table mapping input sensors (pressure, velocity, direction,
//! tilt, wheel, random, fade) onto brush outputs (opacity, size, angle, colour, hardness, force,
//! aspect ratio, spacing, rate, flow, jitter), saved as `.rdyn` JSON and applied to a brush.
//!
//! Applying a preset rewrites the brush's dynamics sections ([`crate::brush::ShapeDynamics`],
//! [`crate::brush::Transfer`], [`crate::brush::ColorDynamics`], [`crate::brush::Scattering`]):
//! each output row becomes the `Dynamic` that drives it, its first non-random input the control,
//! `Random` the jitter, its curve the dynamic's response curve. Outputs the brush model has no
//! per-dab parameter for (hardness, force, spacing, rate) are reported by [`DynamicsPreset::apply`]
//! so a caller can tell the user instead of dropping them silently.

use serde::{Deserialize, Serialize};

use crate::brush::{BrushSettings, Control, Dynamic};
use crate::curve::ResponseCurve;

/// File extension of a saved dynamics preset.
pub const FILE_EXT: &str = "rdyn";
/// Format tag written into every file.
pub const FORMAT: &str = "redrob-paint-dynamics";

/// An input sensor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DynInput {
    Pressure,
    Velocity,
    Direction,
    Tilt,
    Wheel,
    Random,
    Fade,
}

impl DynInput {
    pub const ALL: [DynInput; 7] =
        [DynInput::Pressure, DynInput::Velocity, DynInput::Direction, DynInput::Tilt, DynInput::Wheel, DynInput::Random, DynInput::Fade];

    pub fn label(self) -> &'static str {
        match self {
            DynInput::Pressure => "Pressure",
            DynInput::Velocity => "Velocity",
            DynInput::Direction => "Direction",
            DynInput::Tilt => "Tilt",
            DynInput::Wheel => "Wheel",
            DynInput::Random => "Random",
            DynInput::Fade => "Fade",
        }
    }

    /// The brush control this input drives; `None` for Random (jitter, not a control).
    fn control(self) -> Option<Control> {
        Some(match self {
            DynInput::Pressure => Control::PenPressure,
            DynInput::Velocity => Control::Velocity,
            DynInput::Direction => Control::Direction,
            DynInput::Tilt => Control::PenTilt,
            DynInput::Wheel => Control::StylusWheel,
            DynInput::Fade => Control::Fade,
            DynInput::Random => return None,
        })
    }
}

/// A brush property an input can drive.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DynOutput {
    Opacity,
    Size,
    Angle,
    Color,
    Hardness,
    Force,
    AspectRatio,
    Spacing,
    Rate,
    Flow,
    Jitter,
}

impl DynOutput {
    pub const ALL: [DynOutput; 11] = [
        DynOutput::Opacity,
        DynOutput::Size,
        DynOutput::Angle,
        DynOutput::Color,
        DynOutput::Hardness,
        DynOutput::Force,
        DynOutput::AspectRatio,
        DynOutput::Spacing,
        DynOutput::Rate,
        DynOutput::Flow,
        DynOutput::Jitter,
    ];

    pub fn label(self) -> &'static str {
        match self {
            DynOutput::Opacity => "Opacity",
            DynOutput::Size => "Size",
            DynOutput::Angle => "Angle",
            DynOutput::Color => "Color",
            DynOutput::Hardness => "Hardness",
            DynOutput::Force => "Force",
            DynOutput::AspectRatio => "Aspect Ratio",
            DynOutput::Spacing => "Spacing",
            DynOutput::Rate => "Rate",
            DynOutput::Flow => "Flow",
            DynOutput::Jitter => "Jitter",
        }
    }

    /// Does the brush model have a per-dab dynamic for this output?
    pub fn is_mappable(self) -> bool {
        !matches!(self, DynOutput::Hardness | DynOutput::Force | DynOutput::Spacing | DynOutput::Rate)
    }
}

/// One row of the table: the inputs that drive an output, with the output's own response curve
/// and the floor the inputs may take it down to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DynamicsRow {
    pub output: DynOutput,
    pub inputs: Vec<DynInput>,
    /// Response curve applied to the control value.
    pub curve: ResponseCurve,
    /// 0..1 floor (minimum diameter, minimum roundness…).
    pub minimum: f32,
}

impl Default for DynamicsRow {
    fn default() -> Self {
        Self { output: DynOutput::Opacity, inputs: Vec::new(), curve: ResponseCurve::LINEAR, minimum: 0.0 }
    }
}

impl DynamicsRow {
    pub fn new(output: DynOutput, inputs: &[DynInput]) -> Self {
        Self { output, inputs: inputs.to_vec(), ..Default::default() }
    }
    pub fn with_minimum(mut self, minimum: f32) -> Self {
        self.minimum = minimum;
        self
    }
    pub fn drives(&self, input: DynInput) -> bool {
        self.inputs.contains(&input)
    }
}

/// A named dynamics preset (the `.rdyn` file content).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DynamicsPreset {
    /// Format tag ([`FORMAT`]); checked when a file is read.
    pub format: String,
    pub version: u32,
    pub name: String,
    /// Shipped with the app; never written to the user presets directory.
    pub builtin: bool,
    pub rows: Vec<DynamicsRow>,
    /// Spacing steps a Fade input ramps over.
    pub fade_steps: u32,
    /// How far Random jitter reaches: 1 = the whole range.
    pub random_amount: f32,
}

impl Default for DynamicsPreset {
    fn default() -> Self {
        Self { format: FORMAT.into(), version: 1, name: String::new(), builtin: false, rows: Vec::new(), fade_steps: 100, random_amount: 0.5 }
    }
}

/// What applying a preset could not express on the brush.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Unmapped {
    /// Outputs the brush has no dynamic for.
    pub outputs: Vec<DynOutput>,
    /// `(output, input)` pairs dropped because the output already had a control (one per dynamic).
    pub extra_inputs: Vec<(DynOutput, DynInput)>,
}

impl Unmapped {
    pub fn is_empty(&self) -> bool {
        self.outputs.is_empty() && self.extra_inputs.is_empty()
    }
    /// A one-line note for the status bar; empty when everything mapped.
    pub fn note(&self) -> String {
        let mut parts = Vec::new();
        if !self.outputs.is_empty() {
            let names: Vec<&str> = self.outputs.iter().map(|o| o.label()).collect();
            parts.push(format!("no brush dynamic for {}", names.join(", ")));
        }
        for (o, i) in &self.extra_inputs {
            parts.push(format!("{} keeps only its first input ({} dropped)", o.label(), i.label()));
        }
        parts.join("; ")
    }
}

impl DynamicsPreset {
    pub fn new(name: &str, rows: Vec<DynamicsRow>) -> Self {
        Self { name: name.to_string(), rows, ..Default::default() }
    }

    fn builtin(name: &str, rows: Vec<DynamicsRow>) -> Self {
        Self { builtin: true, ..Self::new(name, rows) }
    }

    /// The row for `output`, if any.
    pub fn row(&self, output: DynOutput) -> Option<&DynamicsRow> {
        self.rows.iter().find(|r| r.output == output)
    }

    /// The row for `output`, created empty when missing (the editor's matrix cells).
    pub fn row_mut(&mut self, output: DynOutput) -> &mut DynamicsRow {
        if let Some(i) = self.rows.iter().position(|r| r.output == output) {
            &mut self.rows[i]
        } else {
            self.rows.push(DynamicsRow::new(output, &[]));
            let last = self.rows.len() - 1;
            &mut self.rows[last]
        }
    }

    /// Is `input` checked for `output`?
    pub fn maps(&self, output: DynOutput, input: DynInput) -> bool {
        self.row(output).is_some_and(|r| r.drives(input))
    }

    /// Check or uncheck one cell of the matrix.
    pub fn set(&mut self, output: DynOutput, input: DynInput, on: bool) {
        let row = self.row_mut(output);
        row.inputs.retain(|i| *i != input);
        if on {
            row.inputs.push(input);
        }
        self.rows.retain(|r| !r.inputs.is_empty());
    }

    /// The `Dynamic` a row becomes: its first non-random input is the control, Random the
    /// jitter. Also returns the inputs that had to be dropped (one control per dynamic).
    fn dynamic(&self, row: &DynamicsRow, angle: bool) -> (Dynamic, Vec<DynInput>) {
        let mut d = Dynamic { fade_steps: self.fade_steps.max(1), minimum: row.minimum.clamp(0.0, 1.0), curve: row.curve, ..Dynamic::default() };
        let mut dropped = Vec::new();
        for i in &row.inputs {
            match i.control() {
                None => d.jitter = self.random_amount.clamp(0.0, 1.0),
                // Direction is an angle: for a scalar output it has no 0..1 value, so it is
                // dropped like a second control.
                Some(Control::Direction) if !angle => dropped.push(*i),
                Some(c) if d.control == Control::Off => d.control = c,
                Some(_) => dropped.push(*i),
            }
        }
        (d, dropped)
    }

    /// Rewrite `b`'s dynamics sections from this preset. Sections the preset doesn't touch are
    /// switched off, so "Dynamics Off" really is off; the tip, colour and tool options stay.
    pub fn apply(&self, b: &mut BrushSettings) -> Unmapped {
        let mut un = Unmapped::default();
        let sd = &mut b.shape_dynamics;
        sd.enabled = false;
        sd.size = Dynamic::default();
        sd.angle = Dynamic::default();
        sd.roundness = Dynamic::default();
        b.transfer.enabled = false;
        b.transfer.opacity = Dynamic::default();
        b.transfer.flow = Dynamic::default();
        b.color_dynamics.enabled = false;
        b.color_dynamics.fg_bg = Dynamic::default();
        b.scattering.enabled = false;
        b.scattering.scatter = Dynamic::default();
        for row in &self.rows {
            if row.inputs.is_empty() {
                continue;
            }
            if !row.output.is_mappable() {
                un.outputs.push(row.output);
                continue;
            }
            let (d, dropped) = self.dynamic(row, row.output == DynOutput::Angle);
            un.extra_inputs.extend(dropped.into_iter().map(|i| (row.output, i)));
            if !d.is_active() {
                continue;
            }
            match row.output {
                DynOutput::Opacity => {
                    b.transfer.enabled = true;
                    b.transfer.opacity = d;
                }
                DynOutput::Flow => {
                    b.transfer.enabled = true;
                    b.transfer.flow = d;
                }
                DynOutput::Size => {
                    b.shape_dynamics.enabled = true;
                    b.shape_dynamics.size = d;
                }
                DynOutput::Angle => {
                    b.shape_dynamics.enabled = true;
                    b.shape_dynamics.angle = d;
                }
                DynOutput::AspectRatio => {
                    b.shape_dynamics.enabled = true;
                    b.shape_dynamics.roundness = d;
                }
                DynOutput::Color => {
                    b.color_dynamics.enabled = true;
                    b.color_dynamics.fg_bg = d;
                }
                DynOutput::Jitter => {
                    // Scatter amount: one radius of displacement, scaled by the control.
                    b.scattering.enabled = true;
                    b.scattering.scatter = Dynamic { jitter: if d.jitter > 0.0 { d.jitter } else { 1.0 }, ..d };
                }
                DynOutput::Hardness | DynOutput::Force | DynOutput::Spacing | DynOutput::Rate => {}
            }
        }
        if self.rows.iter().any(|r| r.drives(DynInput::Random) && r.output == DynOutput::Color) {
            b.color_dynamics.per_tip = true;
        }
        un
    }

    /// The table a brush's dynamics sections amount to (the inverse of [`Self::apply`], for
    /// saving the current brush's dynamics as a preset). Disabled sections contribute nothing.
    pub fn from_brush(name: &str, b: &BrushSettings) -> Self {
        fn input(c: Control) -> Option<DynInput> {
            Some(match c {
                Control::Off => return None,
                Control::Fade => DynInput::Fade,
                Control::PenPressure => DynInput::Pressure,
                Control::PenTilt => DynInput::Tilt,
                Control::StylusWheel => DynInput::Wheel,
                Control::Rotation => DynInput::Wheel,
                Control::Velocity => DynInput::Velocity,
                Control::InitialDirection | Control::Direction | Control::DrawingAngle => DynInput::Direction,
                Control::Random => DynInput::Random,
            })
        }
        let mut p = Self::new(name, Vec::new());
        let mut fade = None;
        let mut random = None;
        let mut row = |output: DynOutput, d: &Dynamic, on: bool| {
            if !on || !d.is_active() {
                return;
            }
            let mut inputs = Vec::new();
            if let Some(i) = input(d.control) {
                inputs.push(i);
                if d.control == Control::Fade {
                    fade = Some(d.fade_steps);
                }
            }
            if d.jitter > 0.0 {
                inputs.push(DynInput::Random);
                random = Some(d.jitter.clamp(0.0, 1.0));
            }
            p.rows.push(DynamicsRow { output, inputs, curve: d.curve, minimum: d.minimum.clamp(0.0, 1.0) });
        };
        let sd = &b.shape_dynamics;
        row(DynOutput::Size, &sd.size, sd.enabled);
        row(DynOutput::Angle, &sd.angle, sd.enabled);
        row(DynOutput::AspectRatio, &sd.roundness, sd.enabled);
        row(DynOutput::Opacity, &b.transfer.opacity, b.transfer.enabled);
        row(DynOutput::Flow, &b.transfer.flow, b.transfer.enabled);
        row(DynOutput::Color, &b.color_dynamics.fg_bg, b.color_dynamics.enabled);
        row(DynOutput::Jitter, &b.scattering.scatter, b.scattering.enabled);
        if let Some(f) = fade {
            p.fade_steps = f.max(1);
        }
        if let Some(r) = random {
            p.random_amount = r;
        }
        p
    }

    /// Read a saved preset; a file of another format or version is an error.
    pub fn from_json(bytes: &[u8]) -> Result<Self, String> {
        let p: DynamicsPreset = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if p.format != FORMAT {
            return Err(format!("not a dynamics preset (format `{}`)", p.format));
        }
        if p.version != 1 {
            return Err(format!("unsupported dynamics preset version {}", p.version));
        }
        if p.name.trim().is_empty() {
            return Err("the dynamics preset has no name".into());
        }
        Ok(p)
    }

    pub fn to_json(&self) -> Result<Vec<u8>, String> {
        let mut out = self.clone();
        out.format = FORMAT.into();
        out.version = 1;
        out.builtin = false;
        serde_json::to_vec_pretty(&out).map_err(|e| e.to_string())
    }
}

/// The shipped presets, in panel order.
pub fn builtin() -> Vec<DynamicsPreset> {
    use DynInput::*;
    use DynOutput::*;
    vec![
        DynamicsPreset::builtin("Dynamics Off", Vec::new()),
        DynamicsPreset::builtin(
            "Basic Dynamics",
            vec![DynamicsRow::new(Opacity, &[Pressure]), DynamicsRow::new(Size, &[Pressure]).with_minimum(0.1), DynamicsRow::new(Angle, &[Direction])],
        ),
        DynamicsPreset::builtin("Pressure Opacity", vec![DynamicsRow::new(Opacity, &[Pressure])]),
        DynamicsPreset::builtin("Pressure Size", vec![DynamicsRow::new(Size, &[Pressure]).with_minimum(0.05)]),
        DynamicsPreset::builtin("Velocity Tapering", vec![DynamicsRow::new(Size, &[Velocity]).with_minimum(0.1), DynamicsRow::new(Opacity, &[Velocity])]),
        DynamicsPreset {
            fade_steps: 200,
            ..DynamicsPreset::builtin("Fade Tapering", vec![DynamicsRow::new(Size, &[Fade]).with_minimum(0.05), DynamicsRow::new(Opacity, &[Fade])])
        },
        DynamicsPreset { random_amount: 1.0, ..DynamicsPreset::builtin("Random Color", vec![DynamicsRow::new(Color, &[Random])]) },
        DynamicsPreset::builtin("Tilt Angle", vec![DynamicsRow::new(Angle, &[Tilt]), DynamicsRow::new(Size, &[Pressure]).with_minimum(0.2)]),
        DynamicsPreset::builtin("Track Direction", vec![DynamicsRow::new(Angle, &[Direction])]),
    ]
}

/// Find a preset by name (case-insensitive).
pub fn find<'a>(presets: &'a [DynamicsPreset], name: &str) -> Option<&'a DynamicsPreset> {
    presets.iter().find(|p| p.name.eq_ignore_ascii_case(name))
}
