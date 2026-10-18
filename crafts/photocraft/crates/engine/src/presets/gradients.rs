//! Window › Gradients: gradient presets in groups, the Gradient tool's current gradient and
//! Gradient Fill layers made from presets.
//!
//! A preset has colour stops (a fixed colour, or the live foreground/background colour, as in
//! Photoshop's "Foreground to Background") and transparency stops, both at `0..=1`. Midpoints are
//! always 50 %. Built-in colours are our own.

use redrob_photo_doc::{Color, Fill, Layer, LayerContent};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Group, Named, always, bad, edit_groups, find, group_index, req_str, str_param, unique_name};
use crate::commands::CommandSpec;
use crate::{Result, Session};

/// A stop colour: fixed, or the current foreground/background colour.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StopColor {
    Foreground,
    Background,
    Rgb([f32; 3]),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradientPreset {
    pub name: String,
    /// Colour stops `(location 0..1, colour)`, sorted by location.
    pub stops: Vec<(f32, StopColor)>,
    /// Transparency stops `(location, opacity 0..1)`; empty = opaque.
    #[serde(default)]
    pub opacity: Vec<(f32, f32)>,
    /// How each interval between consecutive colour stops blends (`segments[i]` is the segment
    /// from stop `i` to stop `i + 1`): its midpoint, blending function and colouring type. Missing
    /// or short = plain linear RGB with centred midpoints (see [`Segment`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub segments: Vec<Segment>,
}

/// A segment's blending function: how the blend factor runs from one stop to the next.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Blend {
    #[default]
    Linear,
    /// A power curve through the midpoint.
    Curved,
    /// A sine ease-in-out.
    Sinusoidal,
    /// A quarter circle that rises steeply first.
    SphericalIncreasing,
    /// A quarter circle that rises steeply last.
    SphericalDecreasing,
    /// The left colour up to the midpoint, the right one after.
    Step,
}

impl Blend {
    pub fn name(self) -> &'static str {
        match self {
            Blend::Linear => "linear",
            Blend::Curved => "curved",
            Blend::Sinusoidal => "sinusoidal",
            Blend::SphericalIncreasing => "sphericalIncreasing",
            Blend::SphericalDecreasing => "sphericalDecreasing",
            Blend::Step => "step",
        }
    }
    pub fn parse(s: &str) -> Option<Blend> {
        Some(match s {
            "linear" => Blend::Linear,
            "curved" => Blend::Curved,
            "sinusoidal" | "sine" => Blend::Sinusoidal,
            "sphericalIncreasing" | "sphericalInc" => Blend::SphericalIncreasing,
            "sphericalDecreasing" | "sphericalDec" => Blend::SphericalDecreasing,
            "step" => Blend::Step,
            _ => return None,
        })
    }
    pub const ALL: [Blend; 6] = [Blend::Linear, Blend::Curved, Blend::Sinusoidal, Blend::SphericalIncreasing, Blend::SphericalDecreasing, Blend::Step];
}

/// A segment's colouring type: the space the two colours are mixed in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Coloring {
    #[default]
    Rgb,
    /// HSV with the hue turning counter-clockwise (increasing).
    HsvCcw,
    /// HSV with the hue turning clockwise (decreasing).
    HsvCw,
}

impl Coloring {
    pub fn name(self) -> &'static str {
        match self {
            Coloring::Rgb => "rgb",
            Coloring::HsvCcw => "hsvCcw",
            Coloring::HsvCw => "hsvCw",
        }
    }
    pub fn parse(s: &str) -> Option<Coloring> {
        Some(match s {
            "rgb" => Coloring::Rgb,
            "hsvCcw" | "hsvCounterClockwise" => Coloring::HsvCcw,
            "hsvCw" | "hsvClockwise" => Coloring::HsvCw,
            _ => return None,
        })
    }
    pub const ALL: [Coloring; 3] = [Coloring::Rgb, Coloring::HsvCcw, Coloring::HsvCw];
}

/// One segment's blending: midpoint (0..1 of the segment), blending function, colouring type.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Segment {
    pub midpoint: f32,
    pub blend: Blend,
    pub coloring: Coloring,
}

impl Default for Segment {
    fn default() -> Self {
        Segment { midpoint: 0.5, blend: Blend::Linear, coloring: Coloring::Rgb }
    }
}

impl Segment {
    /// A plain segment: linear RGB with its midpoint centred (what a stop list without segments is).
    pub fn is_plain(&self) -> bool {
        (self.midpoint - 0.5).abs() < 1e-4 && self.blend == Blend::Linear && self.coloring == Coloring::Rgb
    }

    /// The blend factor (0..1, how much of the right colour) at position `u` (0..1) along the
    /// segment: the midpoint maps to one half, then the blending function shapes the run.
    pub fn factor(&self, u: f32) -> f32 {
        let u = u.clamp(0.0, 1.0);
        let m = self.midpoint.clamp(0.001, 0.999);
        // Piecewise-linear warp putting the midpoint at one half.
        let w = if u <= m { 0.5 * u / m } else { 0.5 + 0.5 * (u - m) / (1.0 - m) };
        match self.blend {
            Blend::Linear => w,
            Blend::Curved => u.max(1e-6).powf((0.5f32).ln() / m.ln()),
            Blend::Sinusoidal => (((w - 0.5) * std::f32::consts::PI).sin() + 1.0) / 2.0,
            Blend::SphericalIncreasing => (1.0 - (1.0 - w) * (1.0 - w)).max(0.0).sqrt(),
            Blend::SphericalDecreasing => 1.0 - (1.0 - w * w).max(0.0).sqrt(),
            Blend::Step => {
                if w < 0.5 {
                    0.0
                } else {
                    1.0
                }
            }
        }
    }

    /// Mixes two RGBA colours by `f` in the segment's colouring space.
    pub fn mix(&self, a: [f32; 4], b: [f32; 4], f: f32) -> [f32; 4] {
        match self.coloring {
            Coloring::Rgb => lerp4(a, b, f),
            Coloring::HsvCcw | Coloring::HsvCw => {
                let (ha, sa, va) = rgb_to_hsv(a);
                let (hb, sb, vb) = rgb_to_hsv(b);
                // Grey ends take the other end's hue so the turn is defined.
                let ha = if sa < 1e-4 { hb } else { ha };
                let hb = if sb < 1e-4 { ha } else { hb };
                let mut d = hb - ha;
                if self.coloring == Coloring::HsvCcw && d < 0.0 {
                    d += 1.0;
                }
                if self.coloring == Coloring::HsvCw && d > 0.0 {
                    d -= 1.0;
                }
                let h = (ha + d * f).rem_euclid(1.0);
                let [r, g, bl] = hsv_to_rgb(h, sa + (sb - sa) * f, va + (vb - va) * f);
                [r, g, bl, a[3] + (b[3] - a[3]) * f]
            }
        }
    }

    /// The segment the other way round (Flip).
    pub fn flipped(&self) -> Segment {
        Segment {
            midpoint: 1.0 - self.midpoint,
            blend: match self.blend {
                Blend::SphericalIncreasing => Blend::SphericalDecreasing,
                Blend::SphericalDecreasing => Blend::SphericalIncreasing,
                b => b,
            },
            coloring: match self.coloring {
                Coloring::HsvCcw => Coloring::HsvCw,
                Coloring::HsvCw => Coloring::HsvCcw,
                c => c,
            },
        }
    }
}

fn rgb_to_hsv(c: [f32; 4]) -> (f32, f32, f32) {
    let (r, g, b) = (c[0], c[1], c[2]);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let v = max;
    let s = if max > 1e-6 { d / max } else { 0.0 };
    let h = if d < 1e-6 {
        0.0
    } else if max == r {
        ((g - b) / d).rem_euclid(6.0) / 6.0
    } else if max == g {
        ((b - r) / d + 2.0) / 6.0
    } else {
        ((r - g) / d + 4.0) / 6.0
    };
    (h, s, v)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h6 = h.rem_euclid(1.0) * 6.0;
    let i = h6.floor();
    let f = h6 - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    match i as i32 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

impl Named for GradientPreset {
    fn name(&self) -> &str {
        &self.name
    }
    fn set_name(&mut self, n: String) {
        self.name = n;
    }
}

fn hex(s: &str) -> [f32; 3] {
    let s = s.trim_start_matches('#');
    let b = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).map_or(0.0, |v| v as f32 / 255.0);
    [b(0), b(2), b(4)]
}

fn hex_of(c: [f32; 3]) -> String {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", q(c[0]), q(c[1]), q(c[2]))
}

fn lerp4(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

/// Piecewise-linear value of sorted `(t, v)` stops at `t` (clamped at the ends).
fn sample<V: Copy>(stops: &[(f32, V)], t: f32, mix: impl Fn(V, V, f32) -> V) -> Option<V> {
    let first = stops.first()?;
    if t <= first.0 {
        return Some(first.1);
    }
    for w in stops.windows(2) {
        let ((t0, a), (t1, b)) = (w[0], w[1]);
        if t <= t1 {
            let u = if t1 > t0 { (t - t0) / (t1 - t0) } else { 1.0 };
            return Some(mix(a, b, u));
        }
    }
    stops.last().map(|s| s.1)
}

impl GradientPreset {
    pub fn new(name: &str, colors: &[&str]) -> Self {
        let n = colors.len().max(2) - 1;
        GradientPreset {
            name: name.into(),
            stops: colors.iter().enumerate().map(|(i, c)| (i as f32 / n as f32, StopColor::Rgb(hex(c)))).collect(),
            opacity: Vec::new(),
            segments: Vec::new(),
        }
    }
    pub fn foreground_to_background() -> Self {
        GradientPreset {
            name: "Foreground to Background".into(),
            stops: vec![(0.0, StopColor::Foreground), (1.0, StopColor::Background)],
            opacity: Vec::new(),
            segments: Vec::new(),
        }
    }

    /// The segment settings of the interval from stop `i` to stop `i + 1` (plain when unset).
    pub fn segment(&self, i: usize) -> Segment {
        self.segments.get(i).copied().unwrap_or_default()
    }

    /// Whether any segment blends other than plain linear RGB.
    pub fn has_shaped_segments(&self) -> bool {
        self.segments.iter().any(|s| !s.is_plain())
    }

    /// The sorted colour stops with the live colours resolved, and the shaped segments expanded
    /// into dense sub-stops so a linear painter follows their curves (plain segments stay as
    /// they are; a Step segment becomes two stops at its midpoint).
    pub fn expanded_stops(&self, fg: [f32; 4], bg: [f32; 4]) -> Vec<(f32, [f32; 4])> {
        let rgb = |c: StopColor| match c {
            StopColor::Foreground => [fg[0], fg[1], fg[2], 1.0],
            StopColor::Background => [bg[0], bg[1], bg[2], 1.0],
            StopColor::Rgb(c) => [c[0], c[1], c[2], 1.0],
        };
        let mut cs: Vec<(f32, [f32; 4])> = self.stops.iter().map(|(t, c)| (t.clamp(0.0, 1.0), rgb(*c))).collect();
        cs.sort_by(|a, b| a.0.total_cmp(&b.0));
        if cs.is_empty() {
            cs = vec![(0.0, [fg[0], fg[1], fg[2], 1.0]), (1.0, [bg[0], bg[1], bg[2], 1.0])];
        }
        if !self.has_shaped_segments() {
            return cs;
        }
        const SUBDIV: usize = 16;
        let mut out = Vec::with_capacity(cs.len() * SUBDIV);
        for (i, w) in cs.windows(2).enumerate() {
            let ((t0, a), (t1, b)) = (w[0], w[1]);
            let seg = self.segment(i);
            out.push((t0, a));
            if seg.is_plain() || t1 - t0 < 1e-6 {
                continue;
            }
            if seg.blend == Blend::Step {
                let tm = t0 + (t1 - t0) * seg.midpoint.clamp(0.001, 0.999);
                out.push((tm - 1e-5, a));
                out.push((tm, seg.mix(a, b, 1.0)));
                continue;
            }
            for k in 1..SUBDIV {
                let u = k as f32 / SUBDIV as f32;
                out.push((t0 + (t1 - t0) * u, seg.mix(a, b, seg.factor(u))));
            }
        }
        if let Some(last) = cs.last() {
            out.push(*last);
        }
        out
    }

    /// RGBA stops for painting: colour and transparency stops merged (see [`apply_opacity`]).
    pub fn resolve(&self, fg: [f32; 4], bg: [f32; 4]) -> Vec<(f32, [f32; 4])> {
        let cs = self.expanded_stops(fg, bg);
        apply_opacity(cs, &self.opacity)
    }

    /// Colour stops (live colours resolved now, sorted, opaque) and opacity stops (sorted; empty
    /// when fully opaque) kept apart, as an editable Gradient Fill layer holds them.
    pub fn fill_stops(&self, fg: [f32; 4], bg: [f32; 4]) -> FillStops {
        let rgb = |c: StopColor| match c {
            StopColor::Foreground => Color::rgb(fg[0], fg[1], fg[2]),
            StopColor::Background => Color::rgb(bg[0], bg[1], bg[2]),
            StopColor::Rgb(c) => Color::rgb(c[0], c[1], c[2]),
        };
        let mut cs: Vec<(f32, Color)> = self.stops.iter().map(|(t, c)| (t.clamp(0.0, 1.0), rgb(*c))).collect();
        cs.sort_by(|a, b| a.0.total_cmp(&b.0));
        if cs.is_empty() {
            cs = vec![(0.0, rgb(StopColor::Foreground)), (1.0, rgb(StopColor::Background))];
        }
        let mut os: Vec<(f32, f32)> = self.opacity.iter().map(|(t, a)| (t.clamp(0.0, 1.0), a.clamp(0.0, 1.0))).collect();
        os.sort_by(|a, b| a.0.total_cmp(&b.0));
        if os.iter().all(|o| (o.1 - 1.0).abs() < 1e-6) {
            os.clear();
        }
        (cs, os)
    }

    /// Stops for a Gradient Fill layer / overlay (live colours resolved now, like Photoshop).
    pub fn doc_stops(&self, fg: [f32; 4], bg: [f32; 4]) -> Vec<(f32, Color)> {
        self.resolve(fg, bg).into_iter().map(|(t, c)| (t, Color::rgba(c[0], c[1], c[2], c[3]))).collect()
    }

    pub fn to_json(&self) -> Value {
        let stops: Vec<Value> = self
            .stops
            .iter()
            .map(|(t, c)| match c {
                StopColor::Foreground => json!([t, "foreground"]),
                StopColor::Background => json!([t, "background"]),
                StopColor::Rgb(c) => json!([t, hex_of(*c)]),
            })
            .collect();
        let opacity: Vec<Value> = self.opacity.iter().map(|(t, a)| json!([t, (a * 100.0).round()])).collect();
        let segments: Vec<Value> = (0..self.stops.len().saturating_sub(1))
            .map(|i| {
                let s = self.segment(i);
                json!({"midpoint": s.midpoint, "blend": s.blend.name(), "coloring": s.coloring.name()})
            })
            .collect();
        json!({"name": self.name, "stops": stops, "transparency": opacity, "segments": segments})
    }

    /// Parse `{"stops":[[t,"#hex"|"foreground"|"background"],…],"transparency":[[t,0..100],…]}`.
    pub fn from_params(name: &str, p: &Value, cmd: &str) -> Result<Self> {
        let arr = p.get("stops").and_then(Value::as_array).ok_or_else(|| bad(cmd, "missing `stops`"))?;
        let mut stops = Vec::new();
        for (i, s) in arr.iter().enumerate() {
            let (t, c) = match s {
                Value::Array(a) if a.len() == 2 => (a[0].as_f64().map(|v| v as f32), a[1].as_str()),
                Value::String(c) => (Some(if arr.len() > 1 { i as f32 / (arr.len() - 1) as f32 } else { 0.0 }), Some(c.as_str())),
                _ => (None, None),
            };
            let (Some(t), Some(c)) = (t, c) else { return Err(bad(cmd, "stops are [location, \"#rrggbb\"|\"foreground\"|\"background\"]")) };
            let c = match c {
                "foreground" => StopColor::Foreground,
                "background" => StopColor::Background,
                h if h.trim_start_matches('#').len() == 6 && h.trim_start_matches('#').chars().all(|c| c.is_ascii_hexdigit()) => StopColor::Rgb(hex(h)),
                o => return Err(bad(cmd, format!("bad stop colour `{o}`"))),
            };
            stops.push((t.clamp(0.0, 1.0), c));
        }
        if stops.len() < 2 {
            return Err(bad(cmd, "a gradient needs at least 2 colour stops"));
        }
        stops.sort_by(|a, b| a.0.total_cmp(&b.0));
        GradientPreset { name: name.into(), stops, opacity: Vec::new(), segments: Vec::new() }.with_transparency(p, cmd)?.with_segments(p, cmd)
    }

    /// This gradient with its segments replaced by explicit `"segments"` (a list of
    /// `{"midpoint","blend","coloring"}`), when `p` has them.
    pub fn with_segments(mut self, p: &Value, cmd: &str) -> Result<Self> {
        let Some(v) = p.get("segments") else { return Ok(self) };
        let arr = v.as_array().ok_or_else(|| bad(cmd, "`segments` is [{midpoint, blend, coloring}, …]"))?;
        let mut out = Vec::with_capacity(arr.len());
        for s in arr {
            out.push(segment_param(s, Segment::default(), cmd)?);
        }
        self.segments = out;
        Ok(self)
    }

    /// This gradient with its transparency stops replaced by explicit `"transparency"` stops,
    /// when `p` has them (see [`transparency_param`]).
    pub fn with_transparency(mut self, p: &Value, cmd: &str) -> Result<Self> {
        if let Some(opacity) = transparency_param(p, cmd)? {
            self.opacity = opacity;
        }
        Ok(self)
    }
}

/// Sorted RGBA colour stops `cs` with transparency stops `(location, opacity 0..1)` multiplied
/// into their alpha. Where both vary, extra samples keep the product close to the reference app's
/// per-pixel interpolation.
pub fn apply_opacity(cs: Stops, opacity: &[(f32, f32)]) -> Stops {
    let mut os: Vec<(f32, f32)> = opacity.iter().map(|(t, a)| (t.clamp(0.0, 1.0), a.clamp(0.0, 1.0))).collect();
    os.sort_by(|a, b| a.0.total_cmp(&b.0));
    if os.is_empty() || os.iter().all(|o| (o.1 - 1.0).abs() < 1e-6) {
        return cs;
    }
    let mut ts: Vec<f32> = cs.iter().map(|s| s.0).chain(os.iter().map(|s| s.0)).chain([0.0, 1.0]).collect();
    ts.sort_by(f32::total_cmp);
    ts.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    let mut all = Vec::new();
    for w in ts.windows(2) {
        for k in 0..4 {
            all.push(w[0] + (w[1] - w[0]) * k as f32 / 4.0);
        }
    }
    all.push(1.0);
    all.into_iter()
        .map(|t| {
            let mut c = sample(&cs, t, lerp4).unwrap_or([0.0; 4]);
            c[3] *= sample(&os, t, |a, b, u| a + (b - a) * u).unwrap_or(1.0);
            (t, c)
        })
        .collect()
}

/// Explicit `"transparency":[[location 0..1, opacity 0..100], …]` as sorted `(location,
/// opacity 0..1)` stops (`[]` = opaque); `None` when `p` has none.
pub fn transparency_param(p: &Value, cmd: &str) -> Result<Option<Vec<(f32, f32)>>> {
    let Some(v) = p.get("transparency") else { return Ok(None) };
    let arr = v.as_array().ok_or_else(|| bad(cmd, "`transparency` is [[location, opacity %], …]"))?;
    if arr.len() > 1024 {
        return Err(bad(cmd, "too many opacity stops (max 1024)"));
    }
    let mut stops = Vec::with_capacity(arr.len());
    for s in arr {
        let pair = s.as_array().filter(|a| a.len() == 2).ok_or_else(|| bad(cmd, "an opacity stop is [location 0..1, opacity 0..100]"))?;
        let f = |i: usize| pair.get(i).and_then(Value::as_f64).map(|v| v as f32).filter(|v| v.is_finite());
        let (Some(t), Some(o)) = (f(0), f(1)) else { return Err(bad(cmd, "opacity stops are numbers")) };
        stops.push((t.clamp(0.0, 1.0), (o / 100.0).clamp(0.0, 1.0)));
    }
    stops.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok(Some(stops))
}

/// Built-in groups, laid out like Photoshop's default Gradients panel (Basics, then folders by
/// hue). The colours are our own.
pub fn builtin() -> Vec<Group<GradientPreset>> {
    let fam = |prefix: &str, sets: &[&[&str]]| -> Vec<GradientPreset> {
        sets.iter().enumerate().map(|(i, c)| GradientPreset::new(&format!("{prefix} {:02}", i + 1), c)).collect()
    };
    let fg_transparent = GradientPreset {
        name: "Foreground to Transparent".into(),
        stops: vec![(0.0, StopColor::Foreground), (1.0, StopColor::Foreground)],
        opacity: vec![(0.0, 1.0), (1.0, 0.0)],
        segments: Vec::new(),
    };
    // A smooth spectrum: 13 hues evenly spaced from red (0°) to magenta (300°). The pastel and
    // neon presets reuse the same hues at a different saturation/lightness, and Rainbow Fade
    // reuses the classic ramp with its ends transparent. Colours are our own.
    let rainbow = GradientPreset::new(
        "Rainbow",
        &["#ec1313", "#ec6d13", "#ecc813", "#b6ec13", "#5bec13", "#13ec25", "#13ec80", "#13ecda", "#13a4ec", "#1349ec", "#3713ec", "#9213ec", "#ec13ec"],
    );
    vec![
        Group::new("Basics", vec![GradientPreset::foreground_to_background(), fg_transparent, GradientPreset::new("Black, White", &["#000000", "#ffffff"])]),
        Group::new(
            "Blues",
            fam(
                "Blue",
                &[
                    &["#0b3d91", "#4fa3e0"],
                    &["#1c2a5a", "#3f6fd8", "#a8d4ff"],
                    &["#00b4d8", "#03045e"],
                    &["#5ec8f2", "#e6f6ff"],
                    &["#253b80", "#20b2aa"],
                    &["#89c2ff", "#1d4ed8", "#0a1a40"],
                ],
            ),
        ),
        Group::new(
            "Purples",
            fam(
                "Purple",
                &[
                    &["#3c096c", "#c77dff"],
                    &["#5a189a", "#e0aaff"],
                    &["#240046", "#7b2cbf", "#f3d9ff"],
                    &["#9d4edd", "#4361ee"],
                    &["#6a0572", "#ab83a1"],
                    &["#2d0b59", "#ff6fd8"],
                ],
            ),
        ),
        Group::new(
            "Pinks",
            fam(
                "Pink",
                &[
                    &["#ff4d8d", "#ffd1e1"],
                    &["#c9184a", "#ff8fab"],
                    &["#ff70a6", "#ffd670"],
                    &["#8e2c5a", "#f7a1c4", "#fff0f5"],
                    &["#ff5fa2", "#a855f7"],
                    &["#ffc2d6", "#ff2e63"],
                ],
            ),
        ),
        Group::new(
            "Reds",
            fam(
                "Red",
                &[
                    &["#7f0000", "#ff3b30"],
                    &["#d00000", "#ffba08"],
                    &["#370617", "#9d0208", "#f48c06"],
                    &["#e5383b", "#f5f3f4"],
                    &["#a4161a", "#0b090a"],
                    &["#ff595e", "#ffca3a"],
                ],
            ),
        ),
        Group::new(
            "Oranges",
            fam(
                "Orange",
                &[
                    &["#ff6d00", "#ffd60a"],
                    &["#e85d04", "#ffba08"],
                    &["#9c3d0b", "#ff9e40", "#fff1d6"],
                    &["#ff7b00", "#ff0054"],
                    &["#fb8500", "#023047"],
                    &["#ffb703", "#fb5607"],
                ],
            ),
        ),
        Group::new(
            "Greens",
            fam(
                "Green",
                &[
                    &["#004b23", "#70e000"],
                    &["#2d6a4f", "#b7e4c7"],
                    &["#1b4332", "#40916c", "#d8f3dc"],
                    &["#38b000", "#ccff33"],
                    &["#0b6e4f", "#08a0a8"],
                    &["#606c38", "#dda15e"],
                ],
            ),
        ),
        Group::new(
            "Browns",
            fam(
                "Brown",
                &[
                    &["#3e2412", "#a47148"],
                    &["#6f4518", "#e6ccb2"],
                    &["#2b1708", "#7f5539", "#ddb892"],
                    &["#8b5e34", "#ffd8a8"],
                    &["#582f0e", "#b6ad90"],
                    &["#4a3728", "#c08552", "#f3e9dc"],
                ],
            ),
        ),
        Group::new(
            "Grays",
            fam(
                "Gray",
                &[
                    &["#212529", "#f8f9fa"],
                    &["#495057", "#dee2e6"],
                    &["#000000", "#6c757d", "#ffffff"],
                    &["#343a40", "#adb5bd", "#343a40"],
                    &["#8d99ae", "#2b2d42"],
                    &["#e9ecef", "#ced4da", "#868e96"],
                ],
            ),
        ),
        Group::new(
            "Rainbows",
            vec![
                rainbow.clone(),
                GradientPreset::new(
                    "Pastel Rainbow",
                    &[
                        "#f4afaf", "#f4cbaf", "#f4e8af", "#e2f4af", "#c6f4af", "#aff4b4", "#aff4d1", "#aff4ee", "#afddf4", "#afc0f4", "#baaff4", "#d7aff4",
                        "#f4aff4",
                    ],
                ),
                GradientPreset::new(
                    "Neon Rainbow",
                    &[
                        "#ff0000", "#ff6a00", "#ffd400", "#bfff00", "#55ff00", "#00ff15", "#00ff80", "#00ffea", "#00aaff", "#0040ff", "#2a00ff", "#9500ff",
                        "#ff00ff",
                    ],
                ),
                GradientPreset::new(
                    "Oil Slick",
                    &["#0b0f1a", "#1b2a4a", "#0f6f6f", "#1fae8a", "#5ac8a8", "#7a5cff", "#b13bd6", "#e04a8f", "#f2a65a", "#3a2a6b", "#0b0f1a"],
                ),
                GradientPreset { name: "Rainbow Fade".into(), opacity: vec![(0.0, 0.0), (0.125, 1.0), (0.875, 1.0), (1.0, 0.0)], ..rainbow },
            ],
        ),
    ]
}

// ------------------------------------------------------------------ commands

/// RGBA stops `(location, colour)` ready to paint.
pub type Stops = Vec<(f32, [f32; 4])>;

/// Colour stops and opacity stops of a Gradient Fill layer.
pub type FillStops = (Vec<(f32, Color)>, Vec<(f32, f32)>);

const CMD_SELECT: &str = "gradient.presets.select";

/// The Gradient tool's current gradient, as a one-stop edit target (`gradient.presets.stop`).
pub const STOP: &str = "gradient.presets.stop";

/// Gradient for the Gradient tool: `"gradient": preset name`, else explicit `"stops"`, else the
/// current gradient (Gradients panel selection); explicit `"transparency"` stops replace its
/// own. `None` when the caller gave `colors` (the legacy evenly spaced form `paint.gradient`
/// handles itself).
pub fn tool_stops(s: &Session, p: &Value) -> Result<Option<Stops>> {
    const CMD: &str = "paint.gradient";
    if p.get("colors").and_then(Value::as_array).is_some_and(|a| !a.is_empty()) {
        return Ok(None);
    }
    let g = if let Some(name) = str_param(p, "gradient") {
        let (gi, ii) = find(&s.presets.gradients, name, None).ok_or_else(|| bad(CMD, format!("no gradient preset \"{name}\"")))?;
        s.presets.gradients[gi].items[ii].clone().with_transparency(p, CMD)?
    } else if p.get("stops").is_some() {
        GradientPreset::from_params("Custom", p, CMD)?
    } else {
        s.presets.gradient.clone().with_transparency(p, CMD)?
    };
    Ok(Some(g.resolve(s.tools.foreground, s.tools.background)))
}

/// The preset named by `"preset"` (else the current gradient), or explicit `"stops"`.
fn preset_param(s: &Session, p: &Value, cmd: &str) -> Result<GradientPreset> {
    if let Some(name) = str_param(p, "preset") {
        let (gi, ii) = find(&s.presets.gradients, name, str_param(p, "group"))
            .ok_or_else(|| bad(cmd, format!("no gradient preset \"{name}\" (see gradient.presets.list)")))?;
        return Ok(s.presets.gradients[gi].items[ii].clone());
    }
    if p.get("stops").is_some() {
        return GradientPreset::from_params(str_param(p, "name").unwrap_or("Custom"), p, cmd);
    }
    Ok(s.presets.gradient.clone())
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let groups: Vec<Value> =
        s.presets.gradients.iter().map(|g| json!({"name": g.name, "presets": g.items.iter().map(GradientPreset::to_json).collect::<Vec<_>>()})).collect();
    Ok(json!({"groups": groups, "current": s.presets.gradient.to_json()}))
}

fn active_gradient_fill(s: &Session) -> Option<redrob_photo_doc::LayerId> {
    let d = s.active()?;
    let id = d.active_layer?;
    matches!(d.doc.layer(id)?.content, LayerContent::Fill(Fill::Gradient { .. })).then_some(id)
}

fn select(s: &mut Session, p: &Value) -> Result<Value> {
    let g = preset_param(s, p, CMD_SELECT)?;
    s.presets.gradient = g.clone();
    s.presets_changed();
    let mut out = json!({"current": g.to_json()});
    // Clicking a preset with a Gradient Fill layer selected changes that layer (Photoshop).
    if p.get("applyToLayer").and_then(Value::as_bool).unwrap_or(true)
        && let Some(id) = active_gradient_fill(s)
    {
        let (stops_new, opacity_new) = g.fill_stops(s.tools.foreground, s.tools.background);
        s.edit("Change Gradient Fill", |doc, _| {
            let mode = doc.mode;
            if let Some(l) = doc.layer_mut(id)
                && let LayerContent::Fill(Fill::Gradient { stops, opacity_stops, midpoints, .. }) = &mut l.content
            {
                *stops = stops_new.into_iter().map(|(t, c)| (t, c.in_mode(mode))).collect();
                *opacity_stops = opacity_new;
                midpoints.clear();
            }
            Ok(())
        })?;
        out["layer"] = json!(id.0);
    }
    Ok(out)
}

/// A preset stop colour: `"#rrggbb"`, `"foreground"` or `"background"`.
fn stop_color_param(v: &Value, cmd: &str) -> Result<StopColor> {
    let s = v.as_str().ok_or_else(|| bad(cmd, "a stop colour is \"#rrggbb\", \"foreground\" or \"background\""))?;
    match s {
        "foreground" => Ok(StopColor::Foreground),
        "background" => Ok(StopColor::Background),
        h if h.trim_start_matches('#').len() == 6 && h.trim_start_matches('#').chars().all(|c| c.is_ascii_hexdigit()) => Ok(StopColor::Rgb(hex(h))),
        o => Err(bad(cmd, format!("bad stop colour `{o}`"))),
    }
}

/// Applies a `gradient.presets.stop` edit to the Gradient tool's current gradient: the
/// `gradient.fill.stop` grammar (add|move|delete|color and the opacity kind) without a `layer`.
/// Midpoints are rejected — presets keep them at the centre; edit a Gradient Fill layer to move
/// them. Pure, so the editor previews a drag with exactly what the command commits.
pub fn apply_preset_stop(g: &GradientPreset, p: &Value, fg: [f32; 4], bg: [f32; 4], cmd: &str) -> Result<GradientPreset> {
    let mut out = g.clone();
    let action = p.get("action").and_then(Value::as_str).ok_or_else(|| bad(cmd, "missing `action` (add|move|delete|color|opacity)"))?;
    if action == "midpoint" {
        return Err(bad(cmd, "preset gradients keep midpoints at the centre (edit a Gradient Fill layer to move them)"));
    }
    let opacity_kind = match p.get("kind").and_then(Value::as_str).unwrap_or("color") {
        "color" => false,
        "opacity" => true,
        o => return Err(bad(cmd, format!("unknown kind `{o}` (color|opacity)"))),
    };
    let index = || -> Result<usize> {
        p.get("index").and_then(Value::as_u64).and_then(|v| usize::try_from(v).ok()).ok_or_else(|| bad(cmd, "missing `index` (a stop number from 0)"))
    };
    let location = || {
        p.get("location")
            .and_then(Value::as_f64)
            .map(|v| v as f32)
            .filter(|v| v.is_finite())
            .map(|v| v.clamp(0.0, 1.0))
            .ok_or_else(|| bad(cmd, "missing `location` (0..1)"))
    };
    let ramp = g.resolve(fg, bg);
    if opacity_kind {
        // Opacity stops: an empty list means "opaque" (two 100 % stops when first edited).
        let mut os = if out.opacity.is_empty() { vec![(0.0, 1.0), (1.0, 1.0)] } else { out.opacity.clone() };
        os.sort_by(|a, b| a.0.total_cmp(&b.0));
        let opacity =
            || p.get("opacity").and_then(Value::as_f64).map(|v| (v as f32 / 100.0).clamp(0.0, 1.0)).ok_or_else(|| bad(cmd, "missing `opacity` (0..100)"));
        match action {
            "add" => {
                let t = location()?;
                let a = match p.get("opacity") {
                    Some(_) => opacity()?,
                    None => sample(&ramp, t, lerp4).map_or(1.0, |c| c[3]),
                };
                os.push((t, a));
            }
            "move" => {
                let (i, t) = (index()?, location()?);
                os.get_mut(i).ok_or_else(|| bad(cmd, format!("no opacity stop {i}")))?.0 = t;
            }
            "delete" => {
                let i = index()?;
                if i >= os.len() {
                    return Err(bad(cmd, format!("no opacity stop {i}")));
                }
                if os.len() <= 2 {
                    return Err(bad(cmd, "a gradient keeps at least 2 opacity stops"));
                }
                os.remove(i);
            }
            "opacity" => {
                let (i, a) = (index()?, opacity()?);
                os.get_mut(i).ok_or_else(|| bad(cmd, format!("no opacity stop {i}")))?.1 = a;
            }
            o => return Err(bad(cmd, format!("unknown action `{o}` for opacity stops (add|move|delete|opacity)"))),
        }
        os.sort_by(|a, b| a.0.total_cmp(&b.0));
        if os.len() > 1024 {
            return Err(bad(cmd, "too many opacity stops (max 1024)"));
        }
        // Fully opaque again (the materialized ends left after edits, say): store as no opacity
        // stops, like `fill_stops` reads them, instead of pinning explicit 100 % ends.
        if os.iter().all(|o| (o.1 - 1.0).abs() < 1e-6) {
            os.clear();
        }
        out.opacity = os;
        return Ok(out);
    }
    let mut stops = out.stops.clone();
    stops.sort_by(|a, b| a.0.total_cmp(&b.0));
    match action {
        "add" => {
            let t = location()?;
            // A new stop takes the colour the gradient has there (Photoshop), unless given.
            let c = match p.get("color") {
                Some(v) => stop_color_param(v, cmd)?,
                None => sample(&ramp, t, lerp4).map_or(StopColor::Rgb([0.0, 0.0, 0.0]), |c| StopColor::Rgb([c[0], c[1], c[2]])),
            };
            stops.push((t, c));
            if stops.len() > 1024 {
                return Err(bad(cmd, "too many colour stops (max 1024)"));
            }
        }
        "move" => {
            let (i, t) = (index()?, location()?);
            stops.get_mut(i).ok_or_else(|| bad(cmd, format!("no colour stop {i}")))?.0 = t;
        }
        "delete" => {
            let i = index()?;
            if i >= stops.len() {
                return Err(bad(cmd, format!("no colour stop {i}")));
            }
            if stops.len() <= 2 {
                return Err(bad(cmd, "a gradient keeps at least 2 colour stops"));
            }
            stops.remove(i);
        }
        "color" => {
            let (i, c) = (index()?, p.get("color").ok_or_else(|| bad(cmd, "missing `color`")).and_then(|v| stop_color_param(v, cmd))?);
            stops.get_mut(i).ok_or_else(|| bad(cmd, format!("no colour stop {i}")))?.1 = c;
        }
        o => return Err(bad(cmd, format!("unknown action `{o}` (add|move|delete|color)"))),
    }
    stops.sort_by(|a, b| a.0.total_cmp(&b.0));
    out.stops = stops;
    Ok(out)
}

fn stop(s: &mut Session, p: &Value) -> Result<Value> {
    let g = apply_preset_stop(&s.presets.gradient, p, s.tools.foreground, s.tools.background, STOP)?;
    s.presets.gradient = g;
    s.presets_changed();
    Ok(json!({"current": s.presets.gradient.to_json()}))
}

fn apply(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "gradient.presets.apply";
    let g = preset_param(s, p, CMD)?;
    let stops = g.doc_stops(s.tools.foreground, s.tools.background);
    let angle = p.get("angle").and_then(Value::as_f64).unwrap_or(90.0) as f32;
    let scale = (p.get("scale").and_then(Value::as_f64).unwrap_or(100.0) as f32 / 100.0).clamp(0.1, 1.5);
    let style = crate::layer_style::gradient_style(p.get("style").and_then(Value::as_str).unwrap_or("linear"));
    let reverse = p.get("reverse").and_then(Value::as_bool).unwrap_or(false);
    let id = s.edit("New Gradient Fill Layer", |doc, active| {
        let fill = Fill::gradient(stops, angle, scale, style, reverse).in_mode(doc.mode);
        let id = doc.insert_above(*active, Layer::new(doc.next_layer_name("Gradient Fill"), LayerContent::Fill(fill)));
        *active = Some(id);
        Ok(id)
    })?;
    Ok(json!({"layer": id.0, "preset": g.name}))
}

fn new_preset(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "gradient.presets.new";
    let mut g = if p.get("stops").is_some() { GradientPreset::from_params("", p, CMD)? } else { s.presets.gradient.clone() };
    g.name = unique_name(&s.presets.gradients, str_param(p, "name").unwrap_or("Custom"));
    let gi = group_index(&mut s.presets.gradients, str_param(p, "group"));
    s.presets.gradients[gi].items.push(g.clone());
    let group = s.presets.gradients[gi].name.clone();
    s.presets_changed();
    Ok(json!({"name": g.name, "group": group}))
}

fn edit(s: &mut Session, p: &Value) -> Result<Value> {
    let action = req_str(p, "action", "gradient.presets.edit")?.to_string();
    let r = edit_groups(&mut s.presets.gradients, &action, p, "gradient.presets.edit")?;
    s.presets_changed();
    Ok(r)
}

fn reset(s: &mut Session, p: &Value) -> Result<Value> {
    let append = p.get("append").and_then(Value::as_bool).unwrap_or(false);
    let mut b = builtin();
    if append {
        for g in &mut b {
            g.name = unique_group(&s.presets.gradients, &g.name);
        }
        s.presets.gradients.extend(b);
    } else {
        s.presets.gradients = b;
    }
    s.presets_changed();
    Ok(json!({"groups": s.presets.gradients.len()}))
}

pub(crate) fn unique_group<T>(groups: &[Group<T>], base: &str) -> String {
    let taken = |n: &str| groups.iter().any(|g| g.name == n);
    if !taken(base) { base.to_string() } else { (1..).map(|i| format!("{base} {i}")).find(|n| !taken(n)).unwrap_or_default() }
}

pub fn specs() -> Vec<CommandSpec> {
    let stops = r##""stops":[[t 0..1,"#rrggbb"|"foreground"|"background"],…],"transparency":[[t,opacity 0..100],…]?"##;
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    vec![
        CommandSpec {
            id: "gradient.presets.list",
            label: "Gradient Presets",
            menu: &[],
            shortcut: None,
            params: "{} → {groups:[{name,presets:[{name,stops,transparency}]}],current}",
            enabled: always,
            run: list,
            journal: false,
        },
        CommandSpec {
            id: CMD_SELECT,
            label: "Select Gradient",
            menu: &[],
            shortcut: None,
            params: leak(format!(
                r##"{{"preset":name|{stops},"group":name?,"applyToLayer":bool=true (also recolours a selected Gradient Fill layer)}} → {{current,layer?}}. The Gradient tool paints with it."##
            )),
            enabled: always,
            run: select,
            journal: true,
        },
        CommandSpec {
            id: STOP,
            label: "Edit Gradient Stop",
            menu: &[],
            shortcut: None,
            params: r##"{"action":"add|move|delete|color|opacity","kind":"color|opacity"="color","index":stop number,"location":0..1,"color":"#rrggbb"|"foreground"|"background" (add: default the colour there),"opacity":0..100} → {"current"}. Edits the Gradient tool's current gradient (no layer, midpoints stay centred)."##,
            enabled: always,
            run: stop,
            journal: true,
        },
        CommandSpec {
            id: "gradient.presets.apply",
            label: "New Gradient Fill Layer from Preset",
            menu: &[],
            shortcut: None,
            params: leak(format!(
                r##"{{"preset":name?=current|{stops},"angle":deg=90,"style":"linear|radial|angle|reflected|diamond"="linear","scale":10..150=100,"reverse":bool}} → {{layer}}"##
            )),
            enabled: super::has_doc,
            run: apply,
            journal: true,
        },
        CommandSpec {
            id: "gradient.presets.new",
            label: "New Gradient Preset",
            menu: &[],
            shortcut: None,
            params: leak(format!(r##"{{"name":str="Custom","group":name?=first,{stops} (default: the current gradient)}}"##)),
            enabled: always,
            run: new_preset,
            journal: true,
        },
        CommandSpec {
            id: "gradient.presets.edit",
            label: "Edit Gradient Presets",
            menu: &[],
            shortcut: None,
            params: super::GROUP_EDIT_PARAMS,
            enabled: always,
            run: edit,
            journal: true,
        },
        CommandSpec {
            id: "gradient.presets.reset",
            label: "Restore Default Gradients",
            menu: &[],
            shortcut: None,
            params: r##"{"append":bool=false}"##,
            enabled: always,
            run: reset,
            journal: true,
        },
    ]
    .into_iter()
    .chain(segment_specs())
    .collect()
}

/// A `{"midpoint","blend","coloring"}` object over `base` (missing keys keep the base's values).
fn segment_param(v: &Value, base: Segment, cmd: &str) -> Result<Segment> {
    let mut seg = base;
    if let Some(m) = v.get("midpoint").and_then(Value::as_f64) {
        if !m.is_finite() {
            return Err(bad(cmd, "midpoint must be a number 0..1"));
        }
        seg.midpoint = (m as f32).clamp(0.001, 0.999);
    }
    if let Some(b) = v.get("blend").and_then(Value::as_str) {
        seg.blend = Blend::parse(b).ok_or_else(|| bad(cmd, format!("blend `{b}` (linear|curved|sinusoidal|sphericalIncreasing|sphericalDecreasing|step)")))?;
    }
    if let Some(c) = v.get("coloring").and_then(Value::as_str) {
        seg.coloring = Coloring::parse(c).ok_or_else(|| bad(cmd, format!("coloring `{c}` (rgb|hsvCcw|hsvCw)")))?;
    }
    Ok(seg)
}

/// Where a segment edit lands: the named preset in its group, or the current gradient.
fn edit_target(s: &mut Session, p: &Value, cmd: &str, f: impl FnOnce(&mut GradientPreset) -> Result<()>) -> Result<Value> {
    if let Some(name) = str_param(p, "preset") {
        let (gi, ii) = find(&s.presets.gradients, name, str_param(p, "group")).ok_or_else(|| bad(cmd, format!("no gradient preset \"{name}\"")))?;
        f(&mut s.presets.gradients[gi].items[ii])?;
        let g = s.presets.gradients[gi].items[ii].clone();
        // The current gradient follows when it is this preset.
        if s.presets.gradient.name == g.name {
            s.presets.gradient = g.clone();
        }
        s.presets_changed();
        return Ok(json!({"preset": g.to_json()}));
    }
    f(&mut s.presets.gradient)?;
    s.presets_changed();
    Ok(json!({"current": s.presets.gradient.to_json()}))
}

fn segment_index(g: &GradientPreset, p: &Value, cmd: &str) -> Result<usize> {
    let n = g.stops.len().saturating_sub(1);
    let i = p.get("index").and_then(Value::as_u64).ok_or_else(|| bad(cmd, "missing `index` (segment number, 0-based)"))? as usize;
    if i >= n {
        return Err(bad(cmd, format!("segment index {i} out of range (the gradient has {n} segments)")));
    }
    Ok(i)
}

fn sorted_stops(g: &mut GradientPreset) {
    g.stops.sort_by(|a, b| a.0.total_cmp(&b.0));
}

fn segment_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "gradient.presets.segment";
    edit_target(s, p, CMD, |g| {
        sorted_stops(g);
        let i = segment_index(g, p, CMD)?;
        let seg = segment_param(p, g.segment(i), CMD)?;
        while g.segments.len() <= i {
            g.segments.push(Segment::default());
        }
        g.segments[i] = seg;
        if g.segments.iter().all(Segment::is_plain) {
            g.segments.clear();
        }
        Ok(())
    })
}

/// Splits segment `index` at its midpoint: a new stop with the colour there, both halves keeping
/// the segment's blending.
fn split_segment(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "gradient.presets.splitSegment";
    let (fg, bg) = (s.tools.foreground, s.tools.background);
    edit_target(s, p, CMD, |g| {
        sorted_stops(g);
        let i = segment_index(g, p, CMD)?;
        let seg = g.segment(i);
        let (t0, t1) = (g.stops[i].0, g.stops[i + 1].0);
        let tm = t0 + (t1 - t0) * seg.midpoint;
        let cs = g.expanded_stops(fg, bg);
        let c = sample(&cs, tm, lerp4).unwrap_or([0.0, 0.0, 0.0, 1.0]);
        g.stops.insert(i + 1, (tm, StopColor::Rgb([c[0], c[1], c[2]])));
        let mut segs: Vec<Segment> = (0..g.stops.len() - 2).map(|k| g.segment(k)).collect();
        segs.insert(i + 1, Segment { midpoint: 0.5, ..seg });
        segs[i] = Segment { midpoint: 0.5, ..seg };
        g.segments = segs;
        if g.segments.iter().all(Segment::is_plain) {
            g.segments.clear();
        }
        Ok(())
    })
}

/// Replicates segment `index` into `count` copies of itself squeezed into its span.
fn replicate_segment(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "gradient.presets.replicate";
    let count = p.get("count").and_then(Value::as_u64).unwrap_or(2).clamp(2, 32) as usize;
    edit_target(s, p, CMD, |g| {
        sorted_stops(g);
        let i = segment_index(g, p, CMD)?;
        let seg = g.segment(i);
        let (t0, c0) = g.stops[i];
        let (t1, c1) = g.stops[i + 1];
        let mut segs: Vec<Segment> = (0..g.stops.len() - 1).map(|k| g.segment(k)).collect();
        let mut new_stops = Vec::new();
        let mut new_segs = Vec::new();
        for k in 0..count {
            let a = t0 + (t1 - t0) * k as f32 / count as f32;
            new_stops.push((a, c0));
            let b = t0 + (t1 - t0) * (k + 1) as f32 / count as f32;
            if k + 1 < count {
                // The copy ends on the right colour, and the next copy starts again on the left one.
                new_stops.push((b - 1e-5, c1));
                new_segs.push(seg);
                new_segs.push(Segment { blend: Blend::Step, midpoint: 0.5, coloring: Coloring::Rgb });
            } else {
                new_segs.push(seg);
            }
        }
        g.stops.splice(i..i + 1, new_stops);
        segs.splice(i..i + 1, new_segs);
        g.segments = segs;
        Ok(())
    })
}

/// Flips the gradient end to end: stops, transparency and segments reversed.
fn flip(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "gradient.presets.flip";
    edit_target(s, p, CMD, |g| {
        sorted_stops(g);
        let n = g.stops.len().saturating_sub(1);
        let segs: Vec<Segment> = (0..n).map(|k| g.segment(k)).collect();
        for st in &mut g.stops {
            st.0 = 1.0 - st.0;
        }
        g.stops.reverse();
        for o in &mut g.opacity {
            o.0 = 1.0 - o.0;
        }
        g.opacity.reverse();
        g.segments = segs.iter().rev().map(Segment::flipped).collect();
        if g.segments.iter().all(Segment::is_plain) {
            g.segments.clear();
        }
        Ok(())
    })
}

/// The gradient as a POV-Ray `color_map` (the stops with their shaped segments expanded), for
/// Save as POV-Ray.
pub fn pov_ray(g: &GradientPreset, fg: [f32; 4], bg: [f32; 4]) -> String {
    let stops = g.resolve(fg, bg);
    let mut out = format!("/* {} */\ncolor_map {{\n", g.name.replace("*/", "* /"));
    for (t, c) in stops {
        out.push_str(&format!("    [{:.6} color rgbt <{:.6}, {:.6}, {:.6}, {:.6}>]\n", t, c[0], c[1], c[2], 1.0 - c[3]));
    }
    out.push_str("}\n");
    out
}

fn export_pov(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "gradient.presets.exportPov";
    let g = preset_param(s, p, CMD)?;
    Ok(json!({"name": g.name, "pov": pov_ray(&g, s.tools.foreground, s.tools.background)}))
}

/// Segment editor commands (blending functions, colouring types, split, replicate, flip,
/// POV-Ray export), added to [`specs`] by [`segment_specs`].
pub fn segment_specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "gradient.presets.segment",
            label: "Edit Gradient Segment",
            menu: &[],
            shortcut: None,
            params: r##"{"preset":name?=current,"group":name?,"index":segment number (0-based, between stops index and index+1),"midpoint":0..1?,"blend":"linear|curved|sinusoidal|sphericalIncreasing|sphericalDecreasing|step"?,"coloring":"rgb|hsvCcw|hsvCw"?} → {current|preset}"##,
            enabled: always,
            run: segment_cmd,
            journal: true,
        },
        CommandSpec {
            id: "gradient.presets.splitSegment",
            label: "Split Gradient Segment",
            menu: &[],
            shortcut: None,
            params: r##"{"preset":name?=current,"index":segment number} (a new stop at the segment's midpoint) → {current|preset}"##,
            enabled: always,
            run: split_segment,
            journal: true,
        },
        CommandSpec {
            id: "gradient.presets.replicate",
            label: "Replicate Gradient Segment",
            menu: &[],
            shortcut: None,
            params: r##"{"preset":name?=current,"index":segment number,"count":2..32=2} → {current|preset}"##,
            enabled: always,
            run: replicate_segment,
            journal: true,
        },
        CommandSpec {
            id: "gradient.presets.flip",
            label: "Flip Gradient",
            menu: &[],
            shortcut: None,
            params: r##"{"preset":name?=current} → {current|preset}"##,
            enabled: always,
            run: flip,
            journal: true,
        },
        CommandSpec {
            id: "gradient.presets.exportPov",
            label: "Save Gradient as POV-Ray",
            menu: &[],
            shortcut: None,
            params: r##"{"preset":name?=current} → {name, pov} (a POV-Ray color_map)"##,
            enabled: always,
            run: export_pov,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod segment_tests {
    use super::*;

    #[test]
    fn shaped_segments_expand_and_commands_edit_them() {
        let mut s = Session::new();
        s.execute("gradient.presets.select", json!({"stops": [[0.0, "#000000"], [1.0, "#ffffff"]]})).unwrap();
        let plain = s.presets.gradient.resolve([0.0; 4], [1.0; 4]);
        assert_eq!(plain.len(), 2);
        let r = s.execute("gradient.presets.segment", json!({"index": 0, "blend": "sphericalIncreasing", "midpoint": 0.25})).unwrap();
        assert_eq!(r["current"]["segments"][0]["blend"], "sphericalIncreasing");
        let shaped = s.presets.gradient.resolve([0.0; 4], [1.0; 4]);
        assert!(shaped.len() > 10);
        // Spherical increasing rises fast: well past half at a quarter of the way.
        let at = |t: f32| sample(&shaped, t, lerp4).unwrap()[0];
        assert!(at(0.25) > 0.6, "{}", at(0.25));
        s.execute("gradient.presets.segment", json!({"index": 0, "blend": "step", "midpoint": 0.5})).unwrap();
        let step = s.presets.gradient.resolve([0.0; 4], [1.0; 4]);
        let at = |t: f32| sample(&step, t, lerp4).unwrap()[0];
        assert!(at(0.4) < 0.01 && at(0.6) > 0.99);
        // Split makes three stops; flip reverses; replicate doubles.
        s.execute("gradient.presets.segment", json!({"index": 0, "blend": "linear"})).unwrap();
        s.execute("gradient.presets.splitSegment", json!({"index": 0})).unwrap();
        assert_eq!(s.presets.gradient.stops.len(), 3);
        s.execute("gradient.presets.flip", json!({})).unwrap();
        assert_eq!(s.presets.gradient.stops[0].1, StopColor::Rgb([1.0, 1.0, 1.0]));
        s.execute("gradient.presets.replicate", json!({"index": 0, "count": 2})).unwrap();
        assert_eq!(s.presets.gradient.stops.len(), 5);
        assert!(s.execute("gradient.presets.segment", json!({"index": 9, "blend": "linear"})).is_err());
        let pov = s.execute("gradient.presets.exportPov", json!({})).unwrap();
        assert!(pov["pov"].as_str().unwrap().starts_with("/* "));
        assert!(pov["pov"].as_str().unwrap().contains("color_map {"));
    }

    #[test]
    fn hsv_coloring_turns_the_hue_the_asked_way() {
        let seg = Segment { coloring: Coloring::HsvCcw, ..Segment::default() };
        let red = [1.0, 0.0, 0.0, 1.0];
        let blue = [0.0, 0.0, 1.0, 1.0];
        let mid = seg.mix(red, blue, 0.5);
        // Counter-clockwise from red (0°) to blue (240°) passes green (120°).
        assert!(mid[1] > 0.9 && mid[0] < 0.1 && mid[2] < 0.1, "{mid:?}");
        let cw = Segment { coloring: Coloring::HsvCw, ..Segment::default() }.mix(red, blue, 0.5);
        // Clockwise passes magenta (300°).
        assert!(cw[0] > 0.9 && cw[2] > 0.9 && cw[1] < 0.1, "{cw:?}");
        assert!((Segment::default().factor(0.3) - 0.3).abs() < 1e-6);
        assert!((Segment { midpoint: 0.25, ..Segment::default() }.factor(0.25) - 0.5).abs() < 1e-6);
    }
}
