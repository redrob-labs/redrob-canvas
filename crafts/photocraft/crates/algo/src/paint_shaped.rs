//! Gradient tool extensions: repeat modes (sawtooth, triangular, truncate), a start offset and
//! the shaped gradients that follow the selection outline (angular, spherical, dimpled).
//!
//! [`paint::paint_gradient`](crate::paint::paint_gradient) paints the plain drag shapes; the
//! tool goes through [`paint_gradient_ext`] here when any of these options is in play.

use redrob_photo_color::blend::{self, BlendMode};
use redrob_photo_geom::Rect;
use redrob_photo_raster::{Surface, from_rgba_into};
use serde::{Deserialize, Serialize};

use crate::paint::{GradientShape, sample_stops, tool_gradient_t};

/// How the gradient continues past its end points.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum GradientRepeat {
    /// The end colours extend to infinity.
    #[default]
    None,
    /// The ramp starts over at each drag length.
    Sawtooth,
    /// The ramp runs back and forth.
    Triangular,
    /// Nothing is painted outside the drag.
    Truncate,
}

impl GradientRepeat {
    /// Parse the tool option names (`none|sawtooth|triangular|truncate`).
    pub fn from_name(s: &str) -> Option<Self> {
        Some(match s {
            "none" => GradientRepeat::None,
            "sawtooth" | "sawtoothWave" => GradientRepeat::Sawtooth,
            "triangular" | "triangularWave" => GradientRepeat::Triangular,
            "truncate" => GradientRepeat::Truncate,
            _ => return None,
        })
    }
}

/// The shaped gradients: the parameter is the distance to the selection edge, not to the drag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShapedKind {
    /// Linear in the distance to the outline.
    Angular,
    /// Bulges early: a quarter circle in the distance.
    Spherical,
    /// Starts slowly: the complement of the quarter circle.
    Dimpled,
}

impl ShapedKind {
    /// Parse the tool style names (`shapedAngular|shapedSpherical|shapedDimpled`).
    pub fn from_style(s: &str) -> Option<Self> {
        Some(match s {
            "shapedAngular" | "shaped" => ShapedKind::Angular,
            "shapedSpherical" => ShapedKind::Spherical,
            "shapedDimpled" => ShapedKind::Dimpled,
            _ => return None,
        })
    }

    fn map(self, d: f32) -> f32 {
        let d = d.clamp(0.0, 1.0);
        match self {
            ShapedKind::Angular => d,
            ShapedKind::Spherical => (1.0 - (1.0 - d) * (1.0 - d)).max(0.0).sqrt(),
            ShapedKind::Dimpled => 1.0 - (1.0 - d * d).max(0.0).sqrt(),
        }
    }
}

/// The raw drag parameter with the repeat mode and offset applied. `None` means the pixel is left
/// alone (truncated).
pub fn apply_repeat(t: f32, repeat: GradientRepeat, offset: f32) -> Option<f32> {
    // Offset (0..1 of the drag) holds the start colour that far along before the ramp begins.
    let ramp = |t: f32| {
        if offset >= 1.0 { 1.0 } else { ((t - offset) / (1.0 - offset)).clamp(0.0, 1.0) }
    };
    match repeat {
        GradientRepeat::None => Some(ramp(t.clamp(0.0, 1.0))),
        GradientRepeat::Truncate => (0.0..=1.0).contains(&t).then(|| ramp(t)),
        GradientRepeat::Sawtooth => Some(ramp(t.rem_euclid(1.0))),
        GradientRepeat::Triangular => {
            let m = t.rem_euclid(2.0);
            Some(ramp(if m > 1.0 { 2.0 - m } else { m }))
        }
    }
}

/// The shaped gradient field over `area`: for every pixel, its distance to the nearest pixel
/// outside the selection (or outside `area` when there is none), normalised to `0..=1` by the
/// largest distance. Row-major, `area.width() * area.height()` values.
pub fn shaped_field(area: Rect, selection: Option<&Surface>) -> Vec<f32> {
    let (w, h) = (area.width() as usize, area.height() as usize);
    if w == 0 || h == 0 {
        return Vec::new();
    }
    // One pixel of "outside" all round, so the canvas edge counts as an edge too.
    let (pw, ph) = (w + 2, h + 2);
    let mut outside = vec![true; pw * ph];
    for y in 0..h {
        for x in 0..w {
            let inside = selection.is_none_or(|m| m.sample_channel(area.x0 + x as i32, area.y0 + y as i32, 0) > 0.5);
            if inside {
                outside[(y + 1) * pw + x + 1] = false;
            }
        }
    }
    let dist = crate::selection::edt(&outside, pw, ph);
    if dist.len() != pw * ph {
        return vec![0.0; w * h];
    }
    let mut out = vec![0.0f32; w * h];
    let mut max = 0.0f32;
    for y in 0..h {
        for x in 0..w {
            let d = dist[(y + 1) * pw + x + 1];
            out[y * w + x] = d;
            max = max.max(d);
        }
    }
    if max > 0.0 {
        for d in &mut out {
            *d /= max;
        }
    }
    out
}

/// Gradient tool over `area` with repeat, offset and the shaped styles.
///
/// `shaped` picks a shaped gradient (the drag then only sets the direction of nothing: the field
/// comes from the selection); otherwise `shape` is the drag shape of
/// [`paint::paint_gradient`](crate::paint::paint_gradient).
#[allow(clippy::too_many_arguments)]
pub fn paint_gradient_ext(
    s: &mut Surface,
    area: Rect,
    from: (f32, f32),
    to: (f32, f32),
    shape: GradientShape,
    shaped: Option<ShapedKind>,
    repeat: GradientRepeat,
    offset: f32,
    stops: &[(f32, [f32; 4])],
    reverse: bool,
    opacity: f32,
    blend_mode: BlendMode,
    dither: bool,
    selection: Option<&Surface>,
    lock_transparency: bool,
) {
    let field = shaped.map(|_| shaped_field(area, selection));
    let w = area.width().max(1) as usize;
    let fmt = s.format();
    let n = fmt.channels();
    let tiles: Vec<Rect> = area.tiles().map(|tc| tc.rect().intersect(&area)).filter(|r| !r.is_empty()).collect();
    let param = |x: i32, y: i32| -> Option<f32> {
        let raw = match (shaped, &field) {
            (Some(k), Some(f)) => {
                let i = (y - area.y0) as usize * w + (x - area.x0) as usize;
                k.map(f.get(i).copied().unwrap_or(0.0))
            }
            _ => {
                // Spirals repeat on their own; the drag parameter is unbounded for the others.
                let (dx, dy) = (to.0 - from.0, to.1 - from.1);
                let len = (dx * dx + dy * dy).sqrt().max(1e-6);
                let (ux, uy) = (dx / len, dy / len);
                let (px, py) = (x as f32 + 0.5 - from.0, y as f32 + 0.5 - from.1);
                let along = px * ux + py * uy;
                let across = -px * uy + py * ux;
                match shape {
                    GradientShape::Linear => along / len,
                    GradientShape::Radial => (px * px + py * py).sqrt() / len,
                    GradientShape::Reflected => along.abs() / len,
                    GradientShape::Diamond => (along.abs() + across.abs()) / len,
                    GradientShape::Square => along.abs().max(across.abs()) / len,
                    _ => tool_gradient_t(shape, from, to, x as f32 + 0.5, y as f32 + 0.5),
                }
            }
        };
        apply_repeat(raw, repeat, offset)
    };
    let src: &Surface = s;
    let work = |r: &Rect| -> Option<(Rect, Vec<f32>)> {
        let rw = r.width() as usize;
        let mut any = false;
        let mut region: Vec<f32> = Vec::new();
        let mut enc = [0.0f32; 8];
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let k = opacity * selection.map_or(1.0, |m| m.sample_channel(x, y, 0));
                if k <= 0.0 {
                    continue;
                }
                let Some(t) = param(x, y) else { continue };
                if !any {
                    region = src.read_region(*r);
                    any = true;
                }
                let mut c = sample_stops(stops, if reverse { 1.0 - t } else { t });
                if dither {
                    let nz = (redrob_photo_color::dither_noise(x, y) - 0.5) / 255.0;
                    for ch in c.iter_mut().take(3) {
                        *ch = (*ch + nz).clamp(0.0, 1.0);
                    }
                }
                let i = ((y - r.y0) as usize * rw + (x - r.x0) as usize) * n;
                let px = &mut region[i..i + n];
                let d = redrob_photo_raster::to_rgba(&fmt, px);
                let mut o = blend::composite(blend_mode, d, c, k);
                if lock_transparency {
                    o[3] = d[3];
                }
                from_rgba_into(&fmt, o, &mut enc);
                px.copy_from_slice(&enc[..n]);
            }
        }
        any.then_some((*r, region))
    };
    #[cfg(not(target_arch = "wasm32"))]
    let done: Vec<(Rect, Vec<f32>)> = {
        use rayon::prelude::*;
        tiles.par_iter().filter_map(work).collect()
    };
    #[cfg(target_arch = "wasm32")]
    let done: Vec<(Rect, Vec<f32>)> = tiles.iter().filter_map(work).collect();
    for (r, region) in done {
        s.write_region(r, &region);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeat_modes() {
        assert_eq!(apply_repeat(1.5, GradientRepeat::None, 0.0), Some(1.0));
        assert_eq!(apply_repeat(1.5, GradientRepeat::Truncate, 0.0), None);
        assert!((apply_repeat(1.25, GradientRepeat::Sawtooth, 0.0).unwrap_or(0.0) - 0.25).abs() < 1e-6);
        assert!((apply_repeat(1.25, GradientRepeat::Triangular, 0.0).unwrap_or(0.0) - 0.75).abs() < 1e-6);
        assert_eq!(apply_repeat(0.25, GradientRepeat::None, 0.5), Some(0.0));
        assert!((apply_repeat(0.75, GradientRepeat::None, 0.5).unwrap_or(0.0) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn shaped_field_is_zero_at_the_edge_and_one_in_the_middle() {
        let f = shaped_field(Rect::from_xywh(0, 0, 9, 9), None);
        assert_eq!(f.len(), 81);
        assert!(f[0] < 0.3);
        assert!((f[4 * 9 + 4] - 1.0).abs() < 1e-6);
        assert_eq!(ShapedKind::Angular.map(0.5), 0.5);
        assert!(ShapedKind::Spherical.map(0.5) > 0.5);
        assert!(ShapedKind::Dimpled.map(0.5) < 0.5);
    }
}
