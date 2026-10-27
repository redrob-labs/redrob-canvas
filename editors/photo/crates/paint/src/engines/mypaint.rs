//! MyPaint-style engine: every setting is a base value plus curves over nine inputs (pressure, fine
//! and gross speed, random, stroke progress, direction, tilt declination and ascension, a custom
//! input), the radius is logarithmic, dabs are emitted per radius travelled and per second, and the
//! dab is a hardness-profiled ellipse that may smudge, erase, lock alpha or colorize.
//!
//! The model is written from the engine's public documentation and the `.myb` brush file format
//! (JSON: `settings.<name>.base_value` and `.inputs.<input>` point lists; the older line format
//! `name base | input (x,y), …` is read too).

use redrob_photo_geom::Rect;
use redrob_photo_raster::{Surface, from_rgba, to_rgba};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::buffer::{hsv_to_rgb, points_bounds, rgb_to_hsv};
use crate::retouch::{Region, alpha_index, mix_back, over_native};
use crate::rng::{rand_signed, rand01};
use crate::{Stroke, StrokePoint};

/// Most dabs one stroke emits.
pub const MAX_DABS: usize = 200_000;
/// Largest dab radius in pixels.
pub const MAX_RADIUS: f32 = 1000.0;

/// The inputs a setting's curves can read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MpInput {
    /// Pen pressure 0..1 (a mouse reads 1), times the pressure gain.
    Pressure,
    /// Fine speed: `ln(1 + speed)` smoothed a little (0..4 in practice).
    Speed1,
    /// Gross speed: the same, smoothed a lot.
    Speed2,
    /// A fresh 0..1 value per dab.
    Random,
    /// 0..1 while a stroke of `stroke_duration_logarithmic` pixels is drawn, then held and reset.
    Stroke,
    /// Direction of motion in degrees, 0..180.
    Direction,
    /// Tilt: how far the pen leans, 0 (upright)..90 (flat).
    TiltDeclination,
    /// Tilt: which way the pen leans, -180..180 degrees.
    TiltAscension,
    /// The `custom_input` setting's own value (-∞..∞; a feedback input).
    Custom,
    /// Vertical tilt component in degrees (-90..90).
    TiltDeclinationX,
    TiltDeclinationY,
    /// Attack angle between the pen and the drawing direction, -180..180.
    AttackAngle,
    /// Barrel rotation in degrees, 0..180.
    BarrelRotation,
    /// Stylus wheel / airbrush 0..1.
    Wheel,
}

impl MpInput {
    pub const ALL: [MpInput; 14] = [
        MpInput::Pressure,
        MpInput::Speed1,
        MpInput::Speed2,
        MpInput::Random,
        MpInput::Stroke,
        MpInput::Direction,
        MpInput::TiltDeclination,
        MpInput::TiltAscension,
        MpInput::Custom,
        MpInput::TiltDeclinationX,
        MpInput::TiltDeclinationY,
        MpInput::AttackAngle,
        MpInput::BarrelRotation,
        MpInput::Wheel,
    ];

    /// The name used in `.myb` files.
    pub fn name(self) -> &'static str {
        match self {
            MpInput::Pressure => "pressure",
            MpInput::Speed1 => "speed1",
            MpInput::Speed2 => "speed2",
            MpInput::Random => "random",
            MpInput::Stroke => "stroke",
            MpInput::Direction => "direction",
            MpInput::TiltDeclination => "tilt_declination",
            MpInput::TiltAscension => "tilt_ascension",
            MpInput::Custom => "custom",
            MpInput::TiltDeclinationX => "tilt_declinationx",
            MpInput::TiltDeclinationY => "tilt_declinationy",
            MpInput::AttackAngle => "attack_angle",
            MpInput::BarrelRotation => "barrel_rotation",
            MpInput::Wheel => "wheel",
        }
    }

    pub fn from_name(n: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|i| i.name() == n)
    }
}

/// One input's curve: `[input, output]` points, linearly interpolated, flat beyond the ends.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MpCurve {
    pub input: MpInput,
    pub points: Vec<[f32; 2]>,
}

impl MpCurve {
    pub fn eval(&self, x: f32) -> f32 {
        let pts: Vec<[f32; 2]> = self.points.iter().filter(|p| p[0].is_finite() && p[1].is_finite()).copied().collect();
        let Some(first) = pts.first() else { return 0.0 };
        if !x.is_finite() || x <= first[0] {
            return first[1];
        }
        for w in pts.windows(2) {
            let (a, b) = (w[0], w[1]);
            if x <= b[0] {
                let t = if b[0] > a[0] { (x - a[0]) / (b[0] - a[0]) } else { 1.0 };
                return a[1] + (b[1] - a[1]) * t;
            }
        }
        pts.last().map_or(0.0, |p| p[1])
    }
}

/// A setting: its base value plus the sum of its input curves.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MpSetting {
    pub base_value: f32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<MpCurve>,
}

impl MpSetting {
    pub const fn constant(v: f32) -> Self {
        Self { base_value: v, inputs: Vec::new() }
    }
    pub fn with(mut self, input: MpInput, points: &[[f32; 2]]) -> Self {
        self.inputs.push(MpCurve { input, points: points.to_vec() });
        self
    }
    pub fn is_constant(&self) -> bool {
        self.inputs.iter().all(|c| c.points.iter().all(|p| p[1] == 0.0))
    }
    /// The value for a set of inputs.
    pub fn eval(&self, inputs: &Inputs) -> f32 {
        let mut v = self.base_value;
        for c in &self.inputs {
            v += c.eval(inputs.get(c.input));
        }
        if v.is_finite() { v } else { self.base_value }
    }
    /// The largest value the setting can take (base plus every curve's maximum output).
    pub fn max(&self) -> f32 {
        self.base_value + self.inputs.iter().map(|c| c.points.iter().map(|p| p[1]).fold(0.0f32, f32::max)).sum::<f32>()
    }
}

/// The current value of every input.
#[derive(Clone, Copy, Debug, Default)]
pub struct Inputs {
    pub pressure: f32,
    pub speed1: f32,
    pub speed2: f32,
    pub random: f32,
    pub stroke: f32,
    pub direction: f32,
    pub tilt_declination: f32,
    pub tilt_ascension: f32,
    pub custom: f32,
    pub tilt_x: f32,
    pub tilt_y: f32,
    pub attack_angle: f32,
    pub barrel_rotation: f32,
    pub wheel: f32,
}

impl Inputs {
    pub fn get(&self, i: MpInput) -> f32 {
        match i {
            MpInput::Pressure => self.pressure,
            MpInput::Speed1 => self.speed1,
            MpInput::Speed2 => self.speed2,
            MpInput::Random => self.random,
            MpInput::Stroke => self.stroke,
            MpInput::Direction => self.direction,
            MpInput::TiltDeclination => self.tilt_declination,
            MpInput::TiltAscension => self.tilt_ascension,
            MpInput::Custom => self.custom,
            MpInput::TiltDeclinationX => self.tilt_x,
            MpInput::TiltDeclinationY => self.tilt_y,
            MpInput::AttackAngle => self.attack_angle,
            MpInput::BarrelRotation => self.barrel_rotation,
            MpInput::Wheel => self.wheel,
        }
    }
}

/// A MyPaint-style brush: every setting of the format (the ones the engine does not act on are
/// kept so a loaded brush round-trips).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MyPaintSettings {
    pub opaque: MpSetting,
    pub opaque_multiply: MpSetting,
    pub opaque_linearize: MpSetting,
    pub radius_logarithmic: MpSetting,
    pub hardness: MpSetting,
    pub anti_aliasing: MpSetting,
    pub dabs_per_basic_radius: MpSetting,
    pub dabs_per_actual_radius: MpSetting,
    pub dabs_per_second: MpSetting,
    pub radius_by_random: MpSetting,
    pub speed1_slowness: MpSetting,
    pub speed2_slowness: MpSetting,
    pub speed1_gamma: MpSetting,
    pub speed2_gamma: MpSetting,
    pub offset_by_random: MpSetting,
    pub offset_by_speed: MpSetting,
    pub offset_by_speed_slowness: MpSetting,
    pub slow_tracking: MpSetting,
    pub slow_tracking_per_dab: MpSetting,
    pub tracking_noise: MpSetting,
    /// Colour: when `use_brush_color` is off these set the paint colour (0..1 each).
    pub color_h: MpSetting,
    pub color_s: MpSetting,
    pub color_v: MpSetting,
    pub change_color_h: MpSetting,
    pub change_color_l: MpSetting,
    pub change_color_hsl_s: MpSetting,
    pub change_color_v: MpSetting,
    pub change_color_hsv_s: MpSetting,
    pub smudge: MpSetting,
    pub smudge_length: MpSetting,
    pub smudge_radius_log: MpSetting,
    pub eraser: MpSetting,
    pub stroke_threshold: MpSetting,
    pub stroke_duration_logarithmic: MpSetting,
    pub stroke_holdtime: MpSetting,
    pub custom_input: MpSetting,
    pub custom_input_slowness: MpSetting,
    pub elliptical_dab_ratio: MpSetting,
    pub elliptical_dab_angle: MpSetting,
    pub direction_filter: MpSetting,
    pub lock_alpha: MpSetting,
    pub colorize: MpSetting,
    pub snap_to_pixel: MpSetting,
    pub pressure_gain_log: MpSetting,
    /// Paint with the tool's foreground colour (true) instead of the brush file's `color_h/s/v`.
    pub use_brush_color: bool,
    /// Where the brush came from (a file name), for the options bar.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
}

impl Default for MyPaintSettings {
    fn default() -> Self {
        Self {
            opaque: MpSetting::constant(1.0),
            opaque_multiply: MpSetting::constant(0.0).with(MpInput::Pressure, &[[0.0, 0.0], [1.0, 1.0]]),
            opaque_linearize: MpSetting::constant(0.9),
            radius_logarithmic: MpSetting::constant(2.0),
            hardness: MpSetting::constant(0.8),
            anti_aliasing: MpSetting::constant(1.0),
            dabs_per_basic_radius: MpSetting::constant(0.0),
            dabs_per_actual_radius: MpSetting::constant(2.0),
            dabs_per_second: MpSetting::constant(0.0),
            radius_by_random: MpSetting::constant(0.0),
            speed1_slowness: MpSetting::constant(0.04),
            speed2_slowness: MpSetting::constant(0.8),
            speed1_gamma: MpSetting::constant(4.0),
            speed2_gamma: MpSetting::constant(4.0),
            offset_by_random: MpSetting::constant(0.0),
            offset_by_speed: MpSetting::constant(0.0),
            offset_by_speed_slowness: MpSetting::constant(1.0),
            slow_tracking: MpSetting::constant(0.0),
            slow_tracking_per_dab: MpSetting::constant(0.0),
            tracking_noise: MpSetting::constant(0.0),
            color_h: MpSetting::constant(0.0),
            color_s: MpSetting::constant(0.0),
            color_v: MpSetting::constant(0.0),
            change_color_h: MpSetting::constant(0.0),
            change_color_l: MpSetting::constant(0.0),
            change_color_hsl_s: MpSetting::constant(0.0),
            change_color_v: MpSetting::constant(0.0),
            change_color_hsv_s: MpSetting::constant(0.0),
            smudge: MpSetting::constant(0.0),
            smudge_length: MpSetting::constant(0.5),
            smudge_radius_log: MpSetting::constant(0.0),
            eraser: MpSetting::constant(0.0),
            stroke_threshold: MpSetting::constant(0.0),
            stroke_duration_logarithmic: MpSetting::constant(4.0),
            stroke_holdtime: MpSetting::constant(0.0),
            custom_input: MpSetting::constant(0.0),
            custom_input_slowness: MpSetting::constant(0.0),
            elliptical_dab_ratio: MpSetting::constant(1.0),
            elliptical_dab_angle: MpSetting::constant(90.0),
            direction_filter: MpSetting::constant(2.0),
            lock_alpha: MpSetting::constant(0.0),
            colorize: MpSetting::constant(0.0),
            snap_to_pixel: MpSetting::constant(0.0),
            pressure_gain_log: MpSetting::constant(0.0),
            use_brush_color: true,
            name: String::new(),
        }
    }
}

impl MyPaintSettings {
    /// Every setting by its `.myb` name.
    pub fn fields_mut(&mut self) -> Vec<(&'static str, &mut MpSetting)> {
        vec![
            ("opaque", &mut self.opaque),
            ("opaque_multiply", &mut self.opaque_multiply),
            ("opaque_linearize", &mut self.opaque_linearize),
            ("radius_logarithmic", &mut self.radius_logarithmic),
            ("hardness", &mut self.hardness),
            ("anti_aliasing", &mut self.anti_aliasing),
            ("dabs_per_basic_radius", &mut self.dabs_per_basic_radius),
            ("dabs_per_actual_radius", &mut self.dabs_per_actual_radius),
            ("dabs_per_second", &mut self.dabs_per_second),
            ("radius_by_random", &mut self.radius_by_random),
            ("speed1_slowness", &mut self.speed1_slowness),
            ("speed2_slowness", &mut self.speed2_slowness),
            ("speed1_gamma", &mut self.speed1_gamma),
            ("speed2_gamma", &mut self.speed2_gamma),
            ("offset_by_random", &mut self.offset_by_random),
            ("offset_by_speed", &mut self.offset_by_speed),
            ("offset_by_speed_slowness", &mut self.offset_by_speed_slowness),
            ("slow_tracking", &mut self.slow_tracking),
            ("slow_tracking_per_dab", &mut self.slow_tracking_per_dab),
            ("tracking_noise", &mut self.tracking_noise),
            ("color_h", &mut self.color_h),
            ("color_s", &mut self.color_s),
            ("color_v", &mut self.color_v),
            ("change_color_h", &mut self.change_color_h),
            ("change_color_l", &mut self.change_color_l),
            ("change_color_hsl_s", &mut self.change_color_hsl_s),
            ("change_color_v", &mut self.change_color_v),
            ("change_color_hsv_s", &mut self.change_color_hsv_s),
            ("smudge", &mut self.smudge),
            ("smudge_length", &mut self.smudge_length),
            ("smudge_radius_log", &mut self.smudge_radius_log),
            ("eraser", &mut self.eraser),
            ("stroke_threshold", &mut self.stroke_threshold),
            ("stroke_duration_logarithmic", &mut self.stroke_duration_logarithmic),
            ("stroke_holdtime", &mut self.stroke_holdtime),
            ("custom_input", &mut self.custom_input),
            ("custom_input_slowness", &mut self.custom_input_slowness),
            ("elliptical_dab_ratio", &mut self.elliptical_dab_ratio),
            ("elliptical_dab_angle", &mut self.elliptical_dab_angle),
            ("direction_filter", &mut self.direction_filter),
            ("lock_alpha", &mut self.lock_alpha),
            ("colorize", &mut self.colorize),
            ("snap_to_pixel", &mut self.snap_to_pixel),
            ("pressure_gain_log", &mut self.pressure_gain_log),
        ]
    }

    /// Read a `.myb` brush file: the JSON format (version 3) or the older line format.
    pub fn from_myb(text: &str) -> Result<Self, String> {
        let text = text.trim_start_matches('\u{feff}');
        let mut out = Self { use_brush_color: false, ..Self::default() };
        if let Ok(v) = serde_json::from_str::<Value>(text) {
            let settings = v.get("settings").and_then(Value::as_object).ok_or("the brush file has no `settings` object")?;
            for (name, field) in out.fields_mut() {
                let Some(sv) = settings.get(name) else { continue };
                *field = MpSetting::default();
                field.base_value = sv.get("base_value").and_then(Value::as_f64).unwrap_or(0.0) as f32;
                if let Some(inputs) = sv.get("inputs").and_then(Value::as_object) {
                    for (iname, pts) in inputs {
                        let Some(input) = MpInput::from_name(iname) else { continue };
                        let points: Vec<[f32; 2]> = pts
                            .as_array()
                            .map(|a| {
                                a.iter()
                                    .filter_map(|p| {
                                        let p = p.as_array()?;
                                        Some([p.first()?.as_f64()? as f32, p.get(1)?.as_f64()? as f32])
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();
                        if points.len() >= 2 {
                            field.inputs.push(MpCurve { input, points });
                        }
                    }
                }
            }
            if let Some(name) = v.get("name").and_then(Value::as_str) {
                out.name = name.to_string();
            }
            return Ok(out);
        }
        // Line format: `name base | input (x,y), (x,y) | input …`.
        let mut any = false;
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.split('|');
            let head = parts.next().unwrap_or("").trim();
            let mut hw = head.split_whitespace();
            let (Some(name), Some(base)) = (hw.next(), hw.next()) else { continue };
            let Ok(base) = base.parse::<f32>() else { continue };
            let mut setting = MpSetting::constant(base);
            for seg in parts {
                let seg = seg.trim();
                let Some((iname, rest)) = seg.split_once(' ') else { continue };
                let Some(input) = MpInput::from_name(iname.trim()) else { continue };
                let mut points = Vec::new();
                for pair in rest.split(')') {
                    let pair = pair.trim().trim_start_matches(',').trim().trim_start_matches('(');
                    if let Some((x, y)) = pair.split_once(',')
                        && let (Ok(x), Ok(y)) = (x.trim().parse::<f32>(), y.trim().parse::<f32>())
                    {
                        points.push([x, y]);
                    }
                }
                if points.len() >= 2 {
                    setting.inputs.push(MpCurve { input, points });
                }
            }
            for (fname, field) in out.fields_mut() {
                if fname == name {
                    *field = setting.clone();
                    any = true;
                }
            }
        }
        if any { Ok(out) } else { Err("not a brush file (neither JSON settings nor `name value | input (x,y)` lines)".into()) }
    }

    /// The largest dab radius this brush can produce, in pixels.
    pub fn max_radius(&self) -> f32 {
        let r = self.radius_logarithmic.max().exp() * (1.0 + self.radius_by_random.max().max(0.0));
        if r.is_finite() { r.clamp(0.5, MAX_RADIUS) } else { MAX_RADIUS }
    }
}

/// One dab, ready to paint.
#[derive(Clone, Copy, Debug)]
struct MpDab {
    x: f32,
    y: f32,
    radius: f32,
    ratio: f32,
    angle: f32,
    hardness: f32,
    /// Straight RGB, before smudging.
    color: [f32; 3],
    opacity: f32,
    smudge: f32,
    smudge_length: f32,
    eraser: f32,
    lock_alpha: f32,
    colorize: f32,
    antialias: bool,
}

/// Shift a straight RGB colour's HSL lightness by `dl` and HSL saturation by `ds`.
fn hsl_shift(c: [f32; 3], dl: f32, ds: f32) -> [f32; 3] {
    let mx = c[0].max(c[1]).max(c[2]);
    let mn = c[0].min(c[1]).min(c[2]);
    let l = (mx + mn) / 2.0;
    let d = mx - mn;
    let s = if d <= 1e-6 { 0.0 } else { d / (1.0 - (2.0 * l - 1.0).abs()).max(1e-6) };
    let [h, _, _] = rgb_to_hsv(c);
    let (l2, s2) = ((l + dl).clamp(0.0, 1.0), (s + ds).clamp(0.0, 1.0));
    // HSL → RGB.
    let c2 = (1.0 - (2.0 * l2 - 1.0).abs()) * s2;
    let h6 = h.rem_euclid(1.0) * 6.0;
    let x = c2 * (1.0 - (h6.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match h6.floor() as i32 {
        0 => (c2, x, 0.0),
        1 => (x, c2, 0.0),
        2 => (0.0, c2, x),
        3 => (0.0, x, c2),
        4 => (x, 0.0, c2),
        _ => (c2, 0.0, x),
    };
    let m = l2 - c2 / 2.0;
    [(r + m).clamp(0.0, 1.0), (g + m).clamp(0.0, 1.0), (b + m).clamp(0.0, 1.0)]
}

/// Generate the dabs of a stroke (pass one: geometry and colour; smudging happens while painting).
fn generate(stroke: &Stroke, s: &MyPaintSettings) -> Vec<MpDab> {
    let b = &stroke.brush;
    let seed = b.seed;
    let base_hsv = rgb_to_hsv([b.color[0], b.color[1], b.color[2]]);
    let mut dabs = Vec::new();
    let pts: Vec<StrokePoint> = stroke.points.iter().filter(|p| p.x.is_finite() && p.y.is_finite()).copied().collect();
    let Some(first) = pts.first() else { return dabs };
    // Tracked (slowed) position and the state the inputs are smoothed from.
    let (mut tx, mut ty) = (first.x as f32, first.y as f32);
    let mut inputs = Inputs { pressure: first.pressure, ..Default::default() };
    let (mut speed1, mut speed2) = (0.0f32, 0.0f32);
    let (mut dir_x, mut dir_y) = (0.0f32, 0.0f32);
    let mut custom = 0.0f32;
    let mut stroke_progress = 0.0f32;
    let mut stroke_started = false;
    let mut dab_acc = 0.0f32;
    let mut last_time = first.time;
    let mut dab_index: u64 = 0;
    let mut started_pos = false;
    let base_radius = s.radius_logarithmic.base_value.exp().clamp(0.1, MAX_RADIUS);
    let mut prev = *first;
    // Sub-step each segment so the inputs stay smooth.
    for p in pts.iter().chain(std::iter::once(first).filter(|_| pts.len() == 1)) {
        let seg_len = ((p.x - prev.x).hypot(p.y - prev.y)) as f32;
        let n_sub = (seg_len / (base_radius * 0.5).max(1.0)).ceil().clamp(1.0, 64.0) as usize;
        for k in 1..=n_sub {
            let f = k as f32 / n_sub as f32;
            let px = prev.x as f32 + (p.x - prev.x) as f32 * f;
            let py = prev.y as f32 + (p.y - prev.y) as f32 * f;
            let pressure_raw = prev.pressure + (p.pressure - prev.pressure) * f;
            let time = prev.time + (p.time - prev.time) * f64::from(f);
            let mut dt = ((time - last_time) / 1000.0) as f32;
            if dt <= 0.0 || !dt.is_finite() {
                dt = 1.0 / 60.0 / n_sub as f32;
            }
            last_time = time;
            // Pressure with gain.
            let gain = s.pressure_gain_log.eval(&inputs).exp();
            inputs.pressure = (pressure_raw * gain).clamp(0.0, 2.0);
            // Stroke threshold: nothing until the pressure reaches it.
            if !stroke_started {
                if inputs.pressure < s.stroke_threshold.eval(&inputs).clamp(0.0, 1.0) {
                    tx = px;
                    ty = py;
                    continue;
                }
                stroke_started = true;
            }
            // Slow tracking: the tracked position lags behind the pointer.
            let slow = s.slow_tracking.eval(&inputs).max(0.0);
            let follow = if slow > 0.0 { 1.0 - (-dt * 60.0 / (1.0 + slow * 10.0)).exp() } else { 1.0 };
            let (ox, oy) = (tx, ty);
            if !started_pos {
                tx = px;
                ty = py;
                started_pos = true;
            } else {
                tx += (px - tx) * follow;
                ty += (py - ty) * follow;
            }
            let (mdx, mdy) = (tx - ox, ty - oy);
            let dist = mdx.hypot(mdy);
            // Speeds: ln(1 + px/s / 30), smoothed by the slownesses (seconds).
            let raw_speed = (1.0 + dist / dt / 30.0).ln();
            let sl1 = s.speed1_slowness.eval(&inputs).max(0.001);
            let sl2 = s.speed2_slowness.eval(&inputs).max(0.001);
            speed1 += (raw_speed - speed1) * (1.0 - (-dt / sl1).exp());
            speed2 += (raw_speed - speed2) * (1.0 - (-dt / sl2).exp());
            inputs.speed1 = speed1.clamp(0.0, 8.0);
            inputs.speed2 = speed2.clamp(0.0, 8.0);
            // Direction, filtered.
            if dist > 1e-4 {
                let filt = s.direction_filter.eval(&inputs).max(0.0);
                let fa = 1.0 - (-dist / (1.0 + filt * base_radius)).exp();
                // Direction is modulo 180°: fold the vector so opposite moves agree.
                let (ux, uy) = if mdx < 0.0 || (mdx == 0.0 && mdy < 0.0) { (-mdx / dist, -mdy / dist) } else { (mdx / dist, mdy / dist) };
                dir_x += (ux - dir_x) * fa;
                dir_y += (uy - dir_y) * fa;
                inputs.direction = dir_y.atan2(dir_x).to_degrees().rem_euclid(180.0);
            }
            // Tilt.
            let (tlx, tly) = (prev.tilt_x + (p.tilt_x - prev.tilt_x) * f, prev.tilt_y + (p.tilt_y - prev.tilt_y) * f);
            inputs.tilt_x = tlx.clamp(-90.0, 90.0);
            inputs.tilt_y = tly.clamp(-90.0, 90.0);
            inputs.tilt_declination = tlx.hypot(tly).clamp(0.0, 90.0);
            inputs.tilt_ascension = if inputs.tilt_declination > 1e-3 { tly.atan2(tlx).to_degrees() } else { 0.0 };
            inputs.attack_angle = (inputs.tilt_ascension - inputs.direction).rem_euclid(360.0) - 180.0;
            inputs.barrel_rotation = (prev.rotation + (p.rotation - prev.rotation) * f).rem_euclid(180.0);
            inputs.wheel = (prev.wheel + (p.wheel - prev.wheel) * f).clamp(0.0, 1.0);
            // Stroke progress over `exp(duration)` pixels, held for `holdtime`, then reset.
            let duration = s.stroke_duration_logarithmic.eval(&inputs).exp().max(1.0);
            let hold = s.stroke_holdtime.eval(&inputs).max(0.0);
            stroke_progress += dist / duration;
            if stroke_progress > 1.0 + hold {
                stroke_progress = 0.0;
            }
            inputs.stroke = stroke_progress.min(1.0);
            // Custom input: a smoothed feedback value.
            let ci = s.custom_input.eval(&inputs);
            let cis = s.custom_input_slowness.eval(&inputs).max(0.0);
            custom += (ci - custom) * if cis > 0.0 { 1.0 - (-dt / cis).exp() } else { 1.0 };
            inputs.custom = custom;
            // Dabs to emit for this step.
            let radius = s.radius_logarithmic.eval(&inputs).exp().clamp(0.1, MAX_RADIUS);
            let per_basic = s.dabs_per_basic_radius.eval(&inputs).max(0.0);
            let per_actual = s.dabs_per_actual_radius.eval(&inputs).max(0.0);
            let per_second = s.dabs_per_second.eval(&inputs).max(0.0);
            dab_acc += dist / base_radius * per_basic + dist / radius * per_actual + dt * per_second;
            let mut emitted = 0;
            while dab_acc >= 1.0 && dabs.len() < MAX_DABS && emitted < 256 {
                dab_acc -= 1.0;
                emitted += 1;
                inputs.random = rand01(seed, dab_index, 601);
                // Dab settings at this point.
                let mut r = s.radius_logarithmic.eval(&inputs).exp().clamp(0.1, MAX_RADIUS);
                let rbr = s.radius_by_random.eval(&inputs);
                if rbr != 0.0 {
                    r *= (rand_signed(seed, dab_index, 602) * rbr).exp();
                    r = r.clamp(0.1, MAX_RADIUS);
                }
                let (mut x, mut y) = (tx, ty);
                // Position jitter and speed offset.
                let obr = s.offset_by_random.eval(&inputs);
                if obr != 0.0 {
                    x += rand_signed(seed, dab_index, 603) * obr * r;
                    y += rand_signed(seed, dab_index, 604) * obr * r;
                }
                let obs = s.offset_by_speed.eval(&inputs);
                if obs != 0.0 && dist > 1e-4 {
                    let k = obs * speed1 * base_radius * 0.2;
                    x += mdx / dist * k;
                    y += mdy / dist * k;
                }
                let noise = s.tracking_noise.eval(&inputs);
                if noise > 0.0 {
                    x += rand_signed(seed, dab_index, 605) * noise * base_radius;
                    y += rand_signed(seed, dab_index, 606) * noise * base_radius;
                }
                if s.snap_to_pixel.eval(&inputs) > 0.5 {
                    x = x.floor() + 0.5;
                    y = y.floor() + 0.5;
                }
                // Opacity: opaque × opaque_multiply, linearised for the dab density.
                let opa = (s.opaque.eval(&inputs) * s.opaque_multiply.eval(&inputs)).clamp(0.0, 1.0);
                let lin = s.opaque_linearize.eval(&inputs).clamp(0.0, 1.0);
                let dpp = (per_actual + per_basic).max(0.0) * 2.0;
                let dab_opa = if dpp > 1.0 && opa < 1.0 { 1.0 - (1.0 - opa).powf(1.0 / (1.0 + lin * (dpp - 1.0))) } else { opa };
                // Colour: the tool's colour or the brush's HSV, then the change_color shifts.
                let [mut h, mut sat, mut v] = if s.use_brush_color {
                    base_hsv
                } else {
                    [s.color_h.eval(&inputs).rem_euclid(1.0), s.color_s.eval(&inputs).clamp(0.0, 1.0), s.color_v.eval(&inputs).clamp(0.0, 1.0)]
                };
                h += s.change_color_h.eval(&inputs);
                sat = (sat + s.change_color_hsv_s.eval(&inputs)).clamp(0.0, 1.0);
                v = (v + s.change_color_v.eval(&inputs)).clamp(0.0, 1.0);
                let mut rgb = hsv_to_rgb(h, sat, v);
                let (dl, dsl) = (s.change_color_l.eval(&inputs), s.change_color_hsl_s.eval(&inputs));
                if dl != 0.0 || dsl != 0.0 {
                    rgb = hsl_shift(rgb, dl, dsl);
                }
                dabs.push(MpDab {
                    x,
                    y,
                    radius: r,
                    ratio: s.elliptical_dab_ratio.eval(&inputs).max(1.0),
                    angle: s.elliptical_dab_angle.eval(&inputs).to_radians(),
                    hardness: s.hardness.eval(&inputs).clamp(0.01, 1.0),
                    color: rgb,
                    opacity: dab_opa,
                    smudge: s.smudge.eval(&inputs).clamp(0.0, 1.0),
                    smudge_length: s.smudge_length.eval(&inputs).clamp(0.0, 1.0),
                    eraser: s.eraser.eval(&inputs).clamp(0.0, 1.0),
                    lock_alpha: s.lock_alpha.eval(&inputs).clamp(0.0, 1.0),
                    colorize: s.colorize.eval(&inputs).clamp(0.0, 1.0),
                    antialias: s.anti_aliasing.eval(&inputs) > 0.5,
                });
                dab_index += 1;
            }
        }
        prev = *p;
    }
    // A click: one dab where the pointer touched down.
    if dabs.is_empty() && stroke_started {
        inputs.random = rand01(seed, 0, 601);
        let r = s.radius_logarithmic.eval(&inputs).exp().clamp(0.1, MAX_RADIUS);
        let opa = (s.opaque.eval(&inputs) * s.opaque_multiply.eval(&inputs)).clamp(0.0, 1.0);
        let [h, sat, v] = if s.use_brush_color {
            base_hsv
        } else {
            [s.color_h.eval(&inputs), s.color_s.eval(&inputs).clamp(0.0, 1.0), s.color_v.eval(&inputs).clamp(0.0, 1.0)]
        };
        dabs.push(MpDab {
            x: tx,
            y: ty,
            radius: r,
            ratio: s.elliptical_dab_ratio.eval(&inputs).max(1.0),
            angle: s.elliptical_dab_angle.eval(&inputs).to_radians(),
            hardness: s.hardness.eval(&inputs).clamp(0.01, 1.0),
            color: hsv_to_rgb(h, sat, v),
            opacity: opa,
            smudge: s.smudge.eval(&inputs).clamp(0.0, 1.0),
            smudge_length: s.smudge_length.eval(&inputs).clamp(0.0, 1.0),
            eraser: s.eraser.eval(&inputs).clamp(0.0, 1.0),
            lock_alpha: s.lock_alpha.eval(&inputs).clamp(0.0, 1.0),
            colorize: s.colorize.eval(&inputs).clamp(0.0, 1.0),
            antialias: s.anti_aliasing.eval(&inputs) > 0.5,
        });
    }
    dabs
}

/// Coverage of the hardness-profiled dab at normalised squared distance `rr` (1 = the rim).
#[inline]
fn profile(rr: f32, hardness: f32) -> f32 {
    if rr >= 1.0 {
        return 0.0;
    }
    if hardness >= 1.0 {
        return 1.0;
    }
    if rr < hardness { rr + 1.0 - rr / hardness } else { hardness / (1.0 - hardness) * (1.0 - rr) }
}

/// Paint a MyPaint-style stroke. Returns the damaged rectangle.
pub fn paint(target: &mut Surface, stroke: &Stroke, s: &MyPaintSettings, selection: Option<&Surface>, lock: bool) -> Rect {
    let dabs = generate(stroke, s);
    if dabs.is_empty() {
        return Rect::EMPTY;
    }
    let fmt = target.format();
    let n = fmt.channels();
    let a = alpha_index(&fmt);
    let max_r = dabs.iter().map(|d| d.radius * d.ratio.max(1.0)).fold(0.0f32, f32::max) + 2.0;
    let mut bounds = Rect::EMPTY;
    for d in &dabs {
        let r = (d.radius * d.ratio.max(1.0) + 1.5).ceil();
        let dr = Rect::new((d.x - r).floor() as i32, (d.y - r).floor() as i32, (d.x + r).ceil() as i32, (d.y + r).ceil() as i32);
        bounds = bounds.union(&dr);
    }
    let budget = points_bounds(&stroke.points, max_r * 4.0 + 64.0);
    let bounds = bounds.intersect(&budget);
    if bounds.is_empty() {
        return Rect::EMPTY;
    }
    let orig = Region::read(target, bounds);
    let mut work = orig.clone();
    let native = |c: [f32; 3]| {
        let mut v = from_rgba(&fmt, [c[0], c[1], c[2], 1.0]);
        if let Some(ai) = a {
            v[ai] = 1.0;
        }
        v
    };
    let mut smudge_state: Option<[f32; 4]> = None;
    let mut acc = vec![0.0f32; n];
    for d in &dabs {
        let (rx, ry) = (d.radius.max(0.1), (d.radius / d.ratio.max(1.0)).max(0.1));
        // Sub-pixel dabs paint fainter instead of vanishing.
        let (rx, ry, tiny) = if rx < 1.0 { (1.0, (ry / rx).max(0.1), rx * rx) } else { (rx, ry, 1.0) };
        let aa_grow = if d.antialias { 0.5 } else { 0.0 };
        let reach = rx.max(ry) + aa_grow + 1.0;
        let r =
            Rect::new((d.x - reach).floor() as i32, (d.y - reach).floor() as i32, (d.x + reach).ceil() as i32, (d.y + reach).ceil() as i32).intersect(&bounds);
        if r.is_empty() {
            continue;
        }
        let (ca, sa) = (d.angle.cos(), d.angle.sin());
        let cov_at = |x: i32, y: i32| -> f32 {
            let (px, py) = (x as f32 + 0.5 - d.x, y as f32 + 0.5 - d.y);
            let (u, v) = ((px * ca + py * sa) / (rx + aa_grow), (-px * sa + py * ca) / (ry + aa_grow));
            profile(u * u + v * v, d.hardness)
        };
        // Smudge: sample the canvas under the dab, drift the smudge colour towards it.
        let mut color = d.color;
        let mut paint_alpha = 1.0;
        if d.smudge > 0.0 {
            acc.iter_mut().for_each(|v| *v = 0.0);
            let (mut wsum, mut asum) = (0.0f32, 0.0f32);
            for y in r.y0..r.y1 {
                for x in r.x0..r.x1 {
                    let w = cov_at(x, y);
                    if w <= 0.0 {
                        continue;
                    }
                    let px = work.px(x, y);
                    let al = a.map_or(1.0, |ai| px[ai]);
                    for c in 0..n {
                        acc[c] += px[c] * w * al;
                    }
                    wsum += w;
                    asum += w * al;
                }
            }
            let canvas = if asum > 1e-6 {
                let nat: Vec<f32> = (0..n).map(|c| acc[c] / asum).collect();
                let rgba = to_rgba(&fmt, &nat);
                [rgba[0], rgba[1], rgba[2], asum / wsum.max(1e-6)]
            } else {
                [0.0, 0.0, 0.0, 0.0]
            };
            let st = match smudge_state {
                None => canvas,
                Some(p) => {
                    let keep = d.smudge_length;
                    let mut o = [0.0; 4];
                    for c in 0..4 {
                        o[c] = p[c] * keep + canvas[c] * (1.0 - keep);
                    }
                    o
                }
            };
            smudge_state = Some(st);
            // Mix the smudged colour in by its weight; its transparency thins the paint.
            for c in 0..3 {
                color[c] += (st[c] - color[c]) * d.smudge;
            }
            paint_alpha = 1.0 - d.smudge * (1.0 - st[3]);
        }
        let src = native(color);
        let hsv_src = rgb_to_hsv(color);
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let k = cov_at(x, y) * d.opacity * tiny;
                if k <= 0.0 {
                    continue;
                }
                let dst = work.px_mut(x, y);
                if d.eraser > 0.0 {
                    if let Some(ai) = a {
                        dst[ai] *= 1.0 - k * d.eraser;
                    }
                    if d.eraser >= 1.0 {
                        continue;
                    }
                }
                let kk = k * (1.0 - d.eraser) * paint_alpha;
                if d.colorize > 0.0 {
                    // Keep the canvas value, take the dab's hue and saturation.
                    let cur = to_rgba(&fmt, dst);
                    let [_, _, v] = rgb_to_hsv([cur[0], cur[1], cur[2]]);
                    let o = hsv_to_rgb(hsv_src[0], hsv_src[1], v);
                    let enc = native(o);
                    over_native(&fmt, dst, &enc, kk * d.colorize, true);
                    if d.colorize >= 1.0 {
                        continue;
                    }
                }
                let lock_alpha = lock || d.lock_alpha >= 0.5;
                over_native(&fmt, dst, &src, kk * (1.0 - d.colorize), lock_alpha);
            }
        }
    }
    mix_back(target, &orig, &work, bounds, stroke.brush.opacity, selection, lock);
    bounds
}
