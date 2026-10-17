//! Segmented gradients and their `.ggr` text files.
//!
//! A segmented gradient is a run of segments over 0..1, each with a left, middle and right
//! position, a colour (with alpha) at each end, a blend function (linear, curved, sinusoidal,
//! spherical increasing or decreasing, step) that shapes how far along the segment the midpoint
//! colour sits, and a colouring model (RGB, or HSV going counter-clockwise or clockwise around the
//! hue circle). Either end may also stand for the live foreground or background colour, opaque or
//! transparent.
//!
//! The `.ggr` file is read from the public format description only: a `GIMP Gradient` header
//! line, an optional `Name:` line, the segment count, then one whitespace-separated line per
//! segment: `left middle right r0 g0 b0 a0 r1 g1 b1 a1 blend colouring [leftType rightType]`.

/// Most segments read from one file.
pub const MAX_SEGMENTS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    Linear,
    Curved,
    Sine,
    SphereIncreasing,
    SphereDecreasing,
    Step,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Coloring {
    Rgb,
    HsvCcw,
    HsvCw,
}

/// Where a segment end takes its colour from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndColor {
    Fixed,
    Foreground,
    ForegroundTransparent,
    Background,
    BackgroundTransparent,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Segment {
    pub left: f32,
    pub middle: f32,
    pub right: f32,
    /// RGBA at the left and right ends (used when the end type is `Fixed`).
    pub left_color: [f32; 4],
    pub right_color: [f32; 4],
    pub blend: Blend,
    pub coloring: Coloring,
    pub left_type: EndColor,
    pub right_type: EndColor,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SegmentedGradient {
    pub name: String,
    pub segments: Vec<Segment>,
}

/// The live colours a segment end may refer to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LiveColors {
    pub foreground: [f32; 4],
    pub background: [f32; 4],
}

impl Default for LiveColors {
    fn default() -> Self {
        Self { foreground: [0.0, 0.0, 0.0, 1.0], background: [1.0, 1.0, 1.0, 1.0] }
    }
}

fn rgb_to_hsv(c: [f32; 3]) -> [f32; 3] {
    let (mx, mn) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
    let d = mx - mn;
    let h = if d <= 0.0 {
        0.0
    } else if mx == c[0] {
        60.0 * ((c[1] - c[2]) / d).rem_euclid(6.0)
    } else if mx == c[1] {
        60.0 * ((c[2] - c[0]) / d + 2.0)
    } else {
        60.0 * ((c[0] - c[1]) / d + 4.0)
    };
    [h, if mx <= 0.0 { 0.0 } else { d / mx }, mx]
}

fn hsv_to_rgb([h, s, v]: [f32; 3]) -> [f32; 3] {
    let h = h.rem_euclid(360.0) / 60.0;
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r + m, g + m, b + m]
}

impl Segment {
    /// The colour at the left or right end, resolving the live colour types.
    pub fn end_color(&self, right: bool, live: LiveColors) -> [f32; 4] {
        let (fixed, kind) = if right { (self.right_color, self.right_type) } else { (self.left_color, self.left_type) };
        match kind {
            EndColor::Fixed => fixed,
            EndColor::Foreground => live.foreground,
            EndColor::ForegroundTransparent => [live.foreground[0], live.foreground[1], live.foreground[2], 0.0],
            EndColor::Background => live.background,
            EndColor::BackgroundTransparent => [live.background[0], live.background[1], live.background[2], 0.0],
        }
    }

    /// Does either end follow the foreground or background colour?
    pub fn is_live(&self) -> bool {
        self.left_type != EndColor::Fixed || self.right_type != EndColor::Fixed
    }

    /// Where between the ends (0..1) the colour sits at `t` (0..1 across the segment), after the
    /// midpoint and the blend function.
    pub fn factor(&self, t: f32) -> f32 {
        let span = (self.right - self.left).max(1e-9);
        let middle = ((self.middle - self.left) / span).clamp(1e-3, 1.0 - 1e-3);
        let t = t.clamp(0.0, 1.0);
        // The midpoint maps to 0.5: a power curve through it, as the curved blend; the others
        // reshape that same position.
        let linear = if t <= middle { 0.5 * t / middle } else { 0.5 + 0.5 * (t - middle) / (1.0 - middle) };
        match self.blend {
            Blend::Linear => linear,
            Blend::Curved => t.powf((0.5f32).ln() / middle.ln()),
            Blend::Sine => (1.0 - (std::f32::consts::PI * linear).cos()) * 0.5,
            Blend::SphereIncreasing => {
                let u = linear - 1.0;
                (1.0 - u * u).max(0.0).sqrt()
            }
            Blend::SphereDecreasing => 1.0 - (1.0 - linear * linear).max(0.0).sqrt(),
            Blend::Step => {
                if linear < 0.5 {
                    0.0
                } else {
                    1.0
                }
            }
        }
    }

    /// The colour at `t` (0..1 across the segment).
    pub fn color_at(&self, t: f32, live: LiveColors) -> [f32; 4] {
        let (a, b) = (self.end_color(false, live), self.end_color(true, live));
        let f = self.factor(t);
        let alpha = a[3] + (b[3] - a[3]) * f;
        let rgb = match self.coloring {
            Coloring::Rgb => [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f],
            Coloring::HsvCcw | Coloring::HsvCw => {
                let (ha, hb) = (rgb_to_hsv([a[0], a[1], a[2]]), rgb_to_hsv([b[0], b[1], b[2]]));
                let mut h0 = ha[0];
                let mut h1 = hb[0];
                // Counter-clockwise goes towards increasing hue, clockwise towards decreasing.
                if self.coloring == Coloring::HsvCcw {
                    if h1 < h0 {
                        h1 += 360.0;
                    }
                } else if h1 > h0 {
                    h0 += 360.0;
                }
                hsv_to_rgb([h0 + (h1 - h0) * f, ha[1] + (hb[1] - ha[1]) * f, ha[2] + (hb[2] - ha[2]) * f])
            }
        };
        [rgb[0], rgb[1], rgb[2], alpha]
    }
}

impl SegmentedGradient {
    /// The colour at position `t` along the gradient (0..1).
    pub fn color_at(&self, t: f32, live: LiveColors) -> [f32; 4] {
        let t = t.clamp(0.0, 1.0);
        let Some(seg) = self.segments.iter().find(|s| t <= s.right).or(self.segments.last()) else {
            return [0.0, 0.0, 0.0, 0.0];
        };
        let span = (seg.right - seg.left).max(1e-9);
        seg.color_at((t - seg.left) / span, live)
    }

    /// Segments sorted, non-overlapping, covering 0..1 without gaps larger than rounding.
    pub fn is_valid(&self) -> bool {
        if self.segments.is_empty() || self.segments.len() > MAX_SEGMENTS {
            return false;
        }
        let mut prev = 0.0f32;
        for s in &self.segments {
            let finite = [s.left, s.middle, s.right].iter().all(|v| v.is_finite()) && s.left_color.iter().chain(&s.right_color).all(|v| v.is_finite());
            // Each segment starts where the previous one ended (the first at 0).
            if !finite || (s.left - prev).abs() > 1e-3 || s.left > s.middle + 1e-6 || s.middle > s.right + 1e-6 || s.right < s.left {
                return false;
            }
            prev = s.right;
        }
        prev >= 1.0 - 1e-3
    }

    /// Sample the gradient into sorted `(position, rgba)` stops a stop-based gradient can draw:
    /// the ends of every segment, plus `subdivisions` points inside each segment whose blend or
    /// colouring is not a plain RGB line (a linear RGB segment needs only its ends). Live
    /// (foreground/background) ends are reported in `live_ends` as `(stop index, end type)` so a
    /// caller can keep them live instead of fixing the colour.
    pub fn sample(&self, subdivisions: usize, live: LiveColors) -> (Vec<(f32, [f32; 4])>, Vec<(usize, EndColor)>) {
        let mut stops: Vec<(f32, [f32; 4])> = Vec::new();
        let mut live_ends = Vec::new();
        let eps = 1e-4;
        for seg in &self.segments {
            let span = seg.right - seg.left;
            // A hard edge against the previous segment: keep both colours a hair apart.
            let left_pos = match stops.last() {
                Some((p, _)) if (*p - seg.left).abs() < eps => (seg.left + eps).min(1.0),
                _ => seg.left,
            };
            if seg.left_type != EndColor::Fixed {
                live_ends.push((stops.len(), seg.left_type));
            }
            stops.push((left_pos, seg.end_color(false, live)));
            let simple = seg.blend == Blend::Linear && seg.coloring == Coloring::Rgb && (seg.middle - (seg.left + seg.right) / 2.0).abs() < 1e-4;
            if !simple && span > eps {
                let n = if seg.blend == Blend::Step { 2 } else { subdivisions.max(1) };
                for i in 1..n {
                    let t = i as f32 / n as f32;
                    if seg.blend == Blend::Step {
                        // One stop just before and one just after the step.
                        let m = (seg.middle - seg.left) / span;
                        stops.push((seg.left + span * (m - 1e-3).max(0.0), seg.color_at((m - 1e-3).max(0.0), live)));
                        stops.push((seg.left + span * (m + 1e-3).min(1.0), seg.color_at((m + 1e-3).min(1.0), live)));
                        break;
                    }
                    stops.push((seg.left + span * t, seg.color_at(t, live)));
                }
            }
            if seg.right_type != EndColor::Fixed {
                live_ends.push((stops.len(), seg.right_type));
            }
            stops.push((seg.right, seg.end_color(true, live)));
        }
        (stops, live_ends)
    }
}

fn blend_from(n: i64) -> Option<Blend> {
    Some(match n {
        0 => Blend::Linear,
        1 => Blend::Curved,
        2 => Blend::Sine,
        3 => Blend::SphereIncreasing,
        4 => Blend::SphereDecreasing,
        5 => Blend::Step,
        _ => return None,
    })
}

fn coloring_from(n: i64) -> Option<Coloring> {
    Some(match n {
        0 => Coloring::Rgb,
        1 => Coloring::HsvCcw,
        2 => Coloring::HsvCw,
        _ => return None,
    })
}

fn end_from(n: i64) -> Option<EndColor> {
    Some(match n {
        0 => EndColor::Fixed,
        1 => EndColor::Foreground,
        2 => EndColor::ForegroundTransparent,
        3 => EndColor::Background,
        4 => EndColor::BackgroundTransparent,
        _ => return None,
    })
}

/// Parse a `.ggr` file.
pub fn parse(bytes: &[u8]) -> Result<SegmentedGradient, String> {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let header = lines.next().ok_or("empty file")?;
    if !header.starts_with("GIMP Gradient") {
        return Err("not a segmented gradient file (missing the header line)".into());
    }
    let mut name = String::new();
    let mut count_line = lines.next().ok_or("missing the segment count")?;
    if let Some(n) = count_line.strip_prefix("Name:") {
        name = n.trim().to_string();
        count_line = lines.next().ok_or("missing the segment count")?;
    }
    let count: usize = count_line.parse().map_err(|_| format!("bad segment count `{count_line}`"))?;
    if count == 0 || count > MAX_SEGMENTS {
        return Err(format!("segment count {count} out of range (1..={MAX_SEGMENTS})"));
    }
    let mut segments = Vec::with_capacity(count);
    for (i, line) in lines.take(count).enumerate() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 13 {
            return Err(format!("segment {} has {} fields, 13 or 15 expected", i + 1, f.len()));
        }
        let num = |j: usize| -> Result<f32, String> { f[j].parse::<f32>().ok().filter(|v| v.is_finite()).ok_or_else(|| format!("segment {}: bad number `{}`", i + 1, f[j])) };
        let int = |j: usize| -> Result<i64, String> { f.get(j).map_or(Ok(0), |s| s.parse::<i64>().map_err(|_| format!("segment {}: bad integer `{s}`", i + 1))) };
        let seg = Segment {
            left: num(0)?,
            middle: num(1)?,
            right: num(2)?,
            left_color: [num(3)?, num(4)?, num(5)?, num(6)?].map(|v| v.clamp(0.0, 1.0)),
            right_color: [num(7)?, num(8)?, num(9)?, num(10)?].map(|v| v.clamp(0.0, 1.0)),
            blend: blend_from(int(11)?).ok_or_else(|| format!("segment {}: unknown blend function", i + 1))?,
            coloring: coloring_from(int(12)?).ok_or_else(|| format!("segment {}: unknown colouring model", i + 1))?,
            left_type: end_from(int(13)?).ok_or_else(|| format!("segment {}: unknown left colour type", i + 1))?,
            right_type: end_from(int(14)?).ok_or_else(|| format!("segment {}: unknown right colour type", i + 1))?,
        };
        segments.push(seg);
    }
    if segments.len() != count {
        return Err(format!("{} segments declared, {} found", count, segments.len()));
    }
    let g = SegmentedGradient { name, segments };
    if !g.is_valid() {
        return Err("segments must be sorted, left <= middle <= right, and cover 0..1".into());
    }
    Ok(g)
}

/// Write a gradient as a `.ggr` file.
pub fn write(g: &SegmentedGradient) -> String {
    let mut out = String::from("GIMP Gradient\n");
    if !g.name.is_empty() {
        out.push_str(&format!("Name: {}\n", g.name));
    }
    out.push_str(&format!("{}\n", g.segments.len()));
    let idx_blend = |b: Blend| match b {
        Blend::Linear => 0,
        Blend::Curved => 1,
        Blend::Sine => 2,
        Blend::SphereIncreasing => 3,
        Blend::SphereDecreasing => 4,
        Blend::Step => 5,
    };
    let idx_col = |c: Coloring| match c {
        Coloring::Rgb => 0,
        Coloring::HsvCcw => 1,
        Coloring::HsvCw => 2,
    };
    let idx_end = |e: EndColor| match e {
        EndColor::Fixed => 0,
        EndColor::Foreground => 1,
        EndColor::ForegroundTransparent => 2,
        EndColor::Background => 3,
        EndColor::BackgroundTransparent => 4,
    };
    for s in &g.segments {
        let [r0, g0, b0, a0] = s.left_color;
        let [r1, g1, b1, a1] = s.right_color;
        out.push_str(&format!(
            "{:.6} {:.6} {:.6} {:.6} {:.6} {:.6} {:.6} {:.6} {:.6} {:.6} {:.6} {} {} {} {}\n",
            s.left,
            s.middle,
            s.right,
            r0,
            g0,
            b0,
            a0,
            r1,
            g1,
            b1,
            a1,
            idx_blend(s.blend),
            idx_col(s.coloring),
            idx_end(s.left_type),
            idx_end(s.right_type)
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO: &str = "GIMP Gradient\nName: Test\n2\n0.000000 0.250000 0.500000 1 0 0 1 0 1 0 1 0 0 0 0\n0.500000 0.750000 1.000000 0 1 0 1 0 0 1 1 1 0 1 3\n";

    #[test]
    fn parses_and_samples() {
        let g = parse(TWO.as_bytes()).unwrap();
        assert_eq!(g.name, "Test");
        assert_eq!(g.segments.len(), 2);
        assert_eq!(g.segments[1].blend, Blend::Curved);
        assert_eq!(g.segments[1].left_type, EndColor::Foreground);
        assert_eq!(g.segments[1].right_type, EndColor::Background);
        let live = LiveColors::default();
        let c = g.color_at(0.0, live);
        assert_eq!(c, [1.0, 0.0, 0.0, 1.0]);
        // The midpoint of the first segment is at 0.25: the colour halfway between the ends there.
        let m = g.color_at(0.25, live);
        assert!((m[0] - 0.5).abs() < 1e-5 && (m[1] - 0.5).abs() < 1e-5, "{m:?}");
        // The second segment runs from the foreground (black) to the background (white).
        let e = g.color_at(1.0, live);
        assert_eq!(e, [1.0, 1.0, 1.0, 1.0]);
        let (stops, live_ends) = g.sample(4, live);
        assert!(stops.windows(2).all(|w| w[0].0 <= w[1].0), "{stops:?}");
        assert_eq!(live_ends.len(), 2);
        assert_eq!(live_ends[1].1, EndColor::Background);
        let back = parse(write(&g).as_bytes()).unwrap();
        assert_eq!(back, g);
    }

    #[test]
    fn blends_and_hsv_directions() {
        let base = Segment {
            left: 0.0,
            middle: 0.5,
            right: 1.0,
            left_color: [1.0, 0.0, 0.0, 1.0],
            right_color: [0.0, 0.0, 1.0, 1.0],
            blend: Blend::Linear,
            coloring: Coloring::Rgb,
            left_type: EndColor::Fixed,
            right_type: EndColor::Fixed,
        };
        assert!((base.factor(0.5) - 0.5).abs() < 1e-6);
        let step = Segment { blend: Blend::Step, ..base };
        assert_eq!(step.factor(0.49), 0.0);
        assert_eq!(step.factor(0.51), 1.0);
        let sine = Segment { blend: Blend::Sine, ..base };
        assert!((sine.factor(0.5) - 0.5).abs() < 1e-6 && sine.factor(0.25) < 0.25);
        let inc = Segment { blend: Blend::SphereIncreasing, ..base };
        assert!(inc.factor(0.5) > 0.8);
        let dec = Segment { blend: Blend::SphereDecreasing, ..base };
        assert!(dec.factor(0.5) < 0.2);
        // Red to blue: counter-clockwise passes through green-ish hues, clockwise through magenta.
        let ccw = Segment { coloring: Coloring::HsvCcw, ..base }.color_at(0.5, LiveColors::default());
        let cw = Segment { coloring: Coloring::HsvCw, ..base }.color_at(0.5, LiveColors::default());
        assert!(ccw[1] > 0.9 && ccw[0] < 0.1, "ccw via cyan/green: {ccw:?}");
        assert!(cw[0] > 0.9 && cw[2] > 0.9, "cw via magenta: {cw:?}");
        assert!(parse(b"not a gradient").is_err());
        assert!(parse(b"GIMP Gradient\n1\n0 0.5 1 0 0 0 1 1 1 1 1 9 0\n").is_err(), "unknown blend");
        assert!(parse(b"GIMP Gradient\n1\n0.5 0.7 1 0 0 0 1 1 1 1 1 0 0\n").is_err(), "does not start at 0");
    }
}
