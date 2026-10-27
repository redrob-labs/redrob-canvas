//! Gradient tool extensions: repeat modes (none, forward, alternating) and the shaped gradient,
//! whose position follows the selection's outline (0 at the edge, 1 at the point farthest
//! inside).

use redrob_photo_color::blend::BlendMode;
use redrob_photo_geom::Rect;
use redrob_photo_raster::Surface;
use serde::{Deserialize, Serialize};

use crate::paint::{GradientShape, sample_stops, tool_gradient_t};

/// How a gradient continues past its two ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GradientRepeat {
    /// The end colours extend (the default).
    #[default]
    None,
    /// The gradient starts over: a sawtooth.
    Forward,
    /// The gradient runs back and forth: a triangle wave.
    Alternating,
}

impl GradientRepeat {
    pub const NAMES: [&'static str; 3] = ["none", "forward", "alternating"];

    pub fn from_name(s: &str) -> Option<Self> {
        Some(match s {
            "none" => Self::None,
            "forward" => Self::Forward,
            "alternating" => Self::Alternating,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Forward => "forward",
            Self::Alternating => "alternating",
        }
    }

    /// An unbounded gradient position folded into 0..=1.
    pub fn fold(self, t: f32) -> f32 {
        if !t.is_finite() {
            return 0.0;
        }
        match self {
            Self::None => t.clamp(0.0, 1.0),
            Self::Forward => t.rem_euclid(1.0),
            Self::Alternating => {
                let u = t.rem_euclid(2.0);
                if u > 1.0 { 2.0 - u } else { u }
            }
        }
    }
}

/// [`tool_gradient_t`] without the final clamp, so a repeat mode can fold the overshoot. The
/// angular shapes are periodic already and come back as they are.
pub fn unbounded_t(shape: GradientShape, from: (f32, f32), to: (f32, f32), x: f32, y: f32) -> f32 {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let len = (dx * dx + dy * dy).sqrt().max(1e-6);
    let (ux, uy) = (dx / len, dy / len);
    let (px, py) = (x - from.0, y - from.1);
    let along = px * ux + py * uy;
    let across = -px * uy + py * ux;
    match shape {
        GradientShape::Linear => along / len,
        GradientShape::Radial => (px * px + py * py).sqrt() / len,
        GradientShape::Reflected => along.abs() / len,
        GradientShape::Diamond => (along.abs() + across.abs()) / len,
        GradientShape::Square => along.abs().max(across.abs()) / len,
        _ => tool_gradient_t(shape, from, to, x, y),
    }
}

/// The shaped gradient's field over a selection: 0 on the selection's edge, 1 at its deepest
/// point, by a chamfer (3-4) distance transform of the selected pixels within `bbox`.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapedField {
    pub bbox: Rect,
    values: Vec<f32>,
}

impl ShapedField {
    /// The field of `inside` (row-major over `bbox`, true where selected). Pixels outside the
    /// selection read 0.
    pub fn from_mask(bbox: Rect, inside: &[bool]) -> Self {
        let (w, h) = (bbox.width() as usize, bbox.height() as usize);
        if w == 0 || h == 0 || inside.len() < w * h {
            return Self { bbox, values: Vec::new() };
        }
        const INF: u32 = u32::MAX / 4;
        // Distance to the nearest non-selected pixel (the bbox edge counts as outside).
        let mut d = vec![0u32; w * h];
        for i in 0..w * h {
            d[i] = if inside[i] { INF } else { 0 };
        }
        let at = |d: &[u32], x: i32, y: i32| -> u32 {
            if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
                return 0;
            }
            d[y as usize * w + x as usize]
        };
        // Forward pass.
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let i = y as usize * w + x as usize;
                if d[i] == 0 {
                    continue;
                }
                let v = d[i]
                    .min(at(&d, x - 1, y).saturating_add(3))
                    .min(at(&d, x, y - 1).saturating_add(3))
                    .min(at(&d, x - 1, y - 1).saturating_add(4))
                    .min(at(&d, x + 1, y - 1).saturating_add(4));
                d[i] = v;
            }
        }
        // Backward pass.
        for y in (0..h as i32).rev() {
            for x in (0..w as i32).rev() {
                let i = y as usize * w + x as usize;
                if d[i] == 0 {
                    continue;
                }
                let v = d[i]
                    .min(at(&d, x + 1, y).saturating_add(3))
                    .min(at(&d, x, y + 1).saturating_add(3))
                    .min(at(&d, x + 1, y + 1).saturating_add(4))
                    .min(at(&d, x - 1, y + 1).saturating_add(4));
                d[i] = v;
            }
        }
        // A selected pixel next to the outside reads one step (3); that is the edge, 0.
        let max = d.iter().copied().filter(|v| *v < INF).max().unwrap_or(0) as f32;
        let span = (max - 3.0).max(1.0);
        let values = d
            .iter()
            .map(|v| {
                if *v >= INF {
                    1.0
                } else if *v == 0 {
                    0.0
                } else {
                    ((*v as f32 - 3.0) / span).clamp(0.0, 1.0)
                }
            })
            .collect();
        Self { bbox, values }
    }

    /// The field of a selection surface (channel 0 above zero is selected) over its content
    /// bounds clipped to `area`; the whole `area` when there is no selection.
    pub fn from_selection(sel: Option<&Surface>, area: Rect) -> Self {
        let bbox = sel.map_or(area, |m| m.content_bounds().intersect(&area));
        let (w, h) = (bbox.width() as usize, bbox.height() as usize);
        let mut inside = vec![true; w * h];
        if let Some(m) = sel {
            for y in 0..h {
                for x in 0..w {
                    inside[y * w + x] = m.sample_channel(bbox.x0 + x as i32, bbox.y0 + y as i32, 0) > 0.5;
                }
            }
        }
        Self::from_mask(bbox, &inside)
    }

    /// The field at document pixel `(x, y)`; 0 outside.
    pub fn at(&self, x: i32, y: i32) -> f32 {
        if !self.bbox.contains(x, y) {
            return 0.0;
        }
        let i = (y - self.bbox.y0) as usize * self.bbox.width() as usize + (x - self.bbox.x0) as usize;
        self.values.get(i).copied().unwrap_or(0.0)
    }
}

/// The colour of a gradient at an unbounded position `t`: folded by `repeat`, reversed when asked,
/// then sampled from `stops`, with optional dither noise.
pub fn gradient_color(stops: &[(f32, [f32; 4])], t: f32, repeat: GradientRepeat, reverse: bool, dither: Option<(i32, i32)>) -> [f32; 4] {
    let t = repeat.fold(t);
    let mut c = sample_stops(stops, if reverse { 1.0 - t } else { t });
    if let Some((x, y)) = dither {
        let n = (redrob_photo_color::dither_noise(x, y) - 0.5) / 255.0;
        for ch in c.iter_mut().take(3) {
            *ch = (*ch + n).clamp(0.0, 1.0);
        }
    }
    c
}

/// The gradient tool's colour at `(x, y)` for a drag `from → to` of `shape`, with the repeat
/// mode; `shaped` (the selection's field) is what the Shaped shape reads.
#[allow(clippy::too_many_arguments)]
pub fn tool_color(
    shape: Option<GradientShape>,
    shaped: Option<&ShapedField>,
    from: (f32, f32),
    to: (f32, f32),
    x: i32,
    y: i32,
    stops: &[(f32, [f32; 4])],
    repeat: GradientRepeat,
    reverse: bool,
    dither: bool,
) -> [f32; 4] {
    let t = match (shape, shaped) {
        (Some(s), _) => unbounded_t(s, from, to, x as f32 + 0.5, y as f32 + 0.5),
        (None, Some(f)) => f.at(x, y),
        (None, None) => 0.0,
    };
    gradient_color(stops, t, repeat, reverse, dither.then_some((x, y)))
}

/// Paint a gradient over `area` of `s` from a per-pixel colour function (the Shaped shape, or
/// any shape with a repeat mode), weighted by the selection and `opacity`.
#[allow(clippy::too_many_arguments)]
pub fn paint_gradient_colors(
    s: &mut Surface,
    area: Rect,
    color: impl Fn(i32, i32) -> [f32; 4] + Sync,
    opacity: f32,
    blend_mode: BlendMode,
    selection: Option<&Surface>,
    lock_transparency: bool,
) {
    crate::paint::composite_area(s, area, blend_mode, |x, y| opacity * selection.map_or(1.0, |m| m.sample_channel(x, y, 0)), color, lock_transparency, None);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeat_modes_fold_positions() {
        assert_eq!(GradientRepeat::None.fold(1.7), 1.0);
        assert_eq!(GradientRepeat::None.fold(-0.2), 0.0);
        assert!((GradientRepeat::Forward.fold(1.25) - 0.25).abs() < 1e-6);
        assert!((GradientRepeat::Forward.fold(-0.25) - 0.75).abs() < 1e-6);
        assert!((GradientRepeat::Alternating.fold(1.25) - 0.75).abs() < 1e-6);
        assert!((GradientRepeat::Alternating.fold(2.25) - 0.25).abs() < 1e-6);
        assert_eq!(GradientRepeat::from_name("alternating"), Some(GradientRepeat::Alternating));
        assert_eq!(GradientRepeat::from_name("x"), None);
        for n in GradientRepeat::NAMES {
            assert_eq!(GradientRepeat::from_name(n).unwrap().name(), n);
        }
        // Linear overshoot is unbounded; the clamped tool value is not.
        let t = unbounded_t(GradientShape::Linear, (0.0, 0.0), (10.0, 0.0), 15.0, 0.0);
        assert!((t - 1.5).abs() < 1e-6);
        assert_eq!(tool_gradient_t(GradientShape::Linear, (0.0, 0.0), (10.0, 0.0), 15.0, 0.0), 1.0);
        let stops = [(0.0, [0.0, 0.0, 0.0, 1.0]), (1.0, [1.0, 1.0, 1.0, 1.0])];
        let c = gradient_color(&stops, 1.5, GradientRepeat::Forward, false, None);
        assert!((c[0] - 0.5).abs() < 1e-5);
        let c = gradient_color(&stops, 0.25, GradientRepeat::None, true, None);
        assert!((c[0] - 0.75).abs() < 1e-5);
    }

    #[test]
    fn shaped_field_is_zero_at_the_edge_and_one_at_the_centre() {
        let bbox = Rect::new(0, 0, 11, 11);
        let f = ShapedField::from_mask(bbox, &vec![true; 121]);
        assert_eq!(f.at(0, 0), 0.0);
        assert_eq!(f.at(5, 5), 1.0);
        assert!(f.at(2, 5) > 0.0 && f.at(2, 5) < f.at(4, 5), "{} {}", f.at(2, 5), f.at(4, 5));
        assert_eq!(f.at(20, 20), 0.0);
        // A ring-shaped mask: the hole reads zero, the ring peaks halfway.
        let mut ring = vec![true; 121];
        for y in 4..7 {
            for x in 4..7 {
                ring[y * 11 + x] = false;
            }
        }
        let f = ShapedField::from_mask(bbox, &ring);
        assert_eq!(f.at(5, 5), 0.0);
        assert!(f.at(2, 5) > 0.0 && f.at(2, 5) < 1.0, "{}", f.at(2, 5));
        assert_eq!(f.at(2, 2), 1.0);
        assert_eq!(ShapedField::from_selection(None, Rect::new(0, 0, 5, 5)).at(2, 2), 1.0);
    }
}
