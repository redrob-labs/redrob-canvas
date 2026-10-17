//! Straight painting symmetry axes. Geometry is independent of pixels and the UI.

use crate::StrokePoint;
use serde::{Deserialize, Serialize};

pub const MAX_COORDINATE: f64 = 1_000_000.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SymmetryMode {
    Vertical,
    Horizontal,
    Dual,
    Diagonal,
}

impl SymmetryMode {
    pub const ALL: [Self; 4] = [Self::Vertical, Self::Horizontal, Self::Dual, Self::Diagonal];
    pub fn id(self) -> &'static str {
        match self {
            Self::Vertical => "vertical",
            Self::Horizontal => "horizontal",
            Self::Dual => "dual",
            Self::Diagonal => "diagonal",
        }
    }
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id() == id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct PresetSymmetry {
    pub mode: SymmetryMode,
    pub center: [f64; 2],
    /// Clockwise degrees relative to the mode's initial direction (document Y points down).
    pub rotation: f64,
}

impl PresetSymmetry {
    pub fn new(mode: SymmetryMode, center: [f64; 2], rotation: f64) -> Option<Self> {
        if !rotation.is_finite() || !center.iter().all(|x| x.is_finite() && x.abs() <= MAX_COORDINATE) {
            return None;
        }
        Some(Self { mode, center, rotation: rotation.rem_euclid(360.0) })
    }
    pub fn directions(self) -> Vec<[f64; 2]> {
        let initial: f64 = match self.mode {
            SymmetryMode::Horizontal => 0.0,
            SymmetryMode::Diagonal => 45.0,
            _ => 90.0,
        };
        let a = (initial + self.rotation).to_radians();
        let first = [a.cos(), a.sin()];
        if self.mode == SymmetryMode::Dual { vec![first, [-first[1], first[0]]] } else { vec![first] }
    }
    pub fn mirror_count(self) -> usize {
        if self.mode == SymmetryMode::Dual { 3 } else { 1 }
    }
    pub fn reflected_passes(self, points: &[StrokePoint]) -> Vec<Vec<StrokePoint>> {
        let reflect = |p: &StrokePoint, direction: [f64; 2]| {
            let [dx, dy] = [p.x - self.center[0], p.y - self.center[1]];
            let along = dx * direction[0] + dy * direction[1];
            StrokePoint { x: self.center[0] + 2.0 * along * direction[0] - dx, y: self.center[1] + 2.0 * along * direction[1] - dy, ..*p }
        };
        let mut copies: Vec<_> = self.directions().into_iter().map(|d| points.iter().map(|p| reflect(p, d)).collect()).collect();
        if self.mode == SymmetryMode::Dual {
            copies.push(points.iter().map(|p| StrokePoint { x: 2.0 * self.center[0] - p.x, y: 2.0 * self.center[1] - p.y, ..*p }).collect());
        }
        copies
    }
    /// Clip infinite axes to document bounds without assuming that their centre is inside.
    pub fn segments(self, size: [f64; 2]) -> Vec<[[f64; 2]; 2]> {
        self.directions()
            .into_iter()
            .filter_map(|d| {
                let (mut lo, mut hi) = (f64::NEG_INFINITY, f64::INFINITY);
                for ((origin, direction), extent) in self.center.into_iter().zip(d).zip(size) {
                    if direction.abs() < 1e-12 {
                        if origin < 0.0 || origin > extent {
                            return None;
                        }
                    } else {
                        let (a, b) = (-origin / direction, (extent - origin) / direction);
                        lo = lo.max(a.min(b));
                        hi = hi.min(a.max(b));
                    }
                }
                if !lo.is_finite() || !hi.is_finite() || hi < lo {
                    return None;
                }
                Some([[self.center[0] + lo * d[0], self.center[1] + lo * d[1]], [self.center[0] + hi * d[0], self.center[1] + hi * d[1]]])
            })
            .collect()
    }
}

/// Rotational multibrush: the stroke repeated `count` times around `center` at even angles,
/// optionally with each copy mirrored across its own spoke as well.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct RotationalSymmetry {
    pub center: [f64; 2],
    /// Copies around the centre, the original included (2..=64).
    pub count: u32,
    /// Also paint every copy mirrored across the vertical axis through the centre.
    pub mirror: bool,
}

impl RotationalSymmetry {
    pub const MIN_COUNT: u32 = 2;
    pub const MAX_COUNT: u32 = 64;
    pub fn new(center: [f64; 2], count: u32, mirror: bool) -> Option<Self> {
        if !(Self::MIN_COUNT..=Self::MAX_COUNT).contains(&count) || !center.iter().all(|x| x.is_finite() && x.abs() <= MAX_COORDINATE) {
            return None;
        }
        Some(Self { center, count, mirror })
    }
    /// Extra render passes besides the stroke itself.
    pub fn mirror_count(self) -> usize {
        let n = self.count as usize;
        if self.mirror { 2 * n - 1 } else { n - 1 }
    }
    /// The angle of copy `k` (0 is the stroke itself), clockwise in document space.
    pub fn angle(self, k: u32) -> f64 {
        std::f64::consts::TAU * f64::from(k) / f64::from(self.count)
    }
    fn rotate(self, p: &StrokePoint, angle: f64) -> StrokePoint {
        let (sin, cos) = angle.sin_cos();
        let [dx, dy] = [p.x - self.center[0], p.y - self.center[1]];
        StrokePoint { x: self.center[0] + dx * cos - dy * sin, y: self.center[1] + dx * sin + dy * cos, ..*p }
    }
    /// The copies of `points`, in the order [`Self::mirror_count`] counts: the rotations first,
    /// then (with `mirror`) the reflection and its rotations.
    pub fn copies(self, points: &[StrokePoint]) -> Vec<Vec<StrokePoint>> {
        let mut out = Vec::with_capacity(self.mirror_count());
        for k in 1..self.count {
            let a = self.angle(k);
            out.push(points.iter().map(|p| self.rotate(p, a)).collect());
        }
        if self.mirror {
            let mirrored: Vec<StrokePoint> = points.iter().map(|p| StrokePoint { x: 2.0 * self.center[0] - p.x, ..*p }).collect();
            for k in 0..self.count {
                let a = self.angle(k);
                out.push(mirrored.iter().map(|p| self.rotate(p, a)).collect());
            }
        }
        out
    }
    /// Unit directions of the spokes between copies (one per copy), for guides.
    pub fn spokes(self) -> Vec<[f64; 2]> {
        (0..self.count)
            .map(|k| {
                let a = -std::f64::consts::FRAC_PI_2 + self.angle(k);
                [a.cos(), a.sin()]
            })
            .collect()
    }
}

/// Wrap-around painting: a stroke on a `size` canvas also lands shifted by every combination of
/// ±width and ±height, so paint leaving one edge comes back at the opposite one and the canvas
/// tiles seamlessly. Eight extra passes, always in the same order.
pub fn wrapped_passes(points: &[StrokePoint], size: [f64; 2]) -> Vec<Vec<StrokePoint>> {
    let mut out = Vec::with_capacity(8);
    for j in -1..=1 {
        for i in -1..=1 {
            if (i, j) == (0, 0) {
                continue;
            }
            let (dx, dy) = (f64::from(i) * size[0], f64::from(j) * size[1]);
            out.push(points.iter().map(|p| StrokePoint { x: p.x + dx, y: p.y + dy, ..*p }).collect());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn modes_reflect_about_their_centre_and_keep_stylus_data() {
        let p = StrokePoint { x: 12.0, y: 25.0, pressure: 0.4, tilt_x: 18.0, ..Default::default() };
        for (mode, expected) in [(SymmetryMode::Vertical, [28.0, 25.0]), (SymmetryMode::Horizontal, [12.0, 35.0]), (SymmetryMode::Diagonal, [15.0, 22.0])] {
            let s = PresetSymmetry::new(mode, [20.0, 30.0], 0.0).unwrap();
            let r = s.reflected_passes(&[p])[0][0];
            assert!((r.x - expected[0]).abs() < 1e-9 && (r.y - expected[1]).abs() < 1e-9);
            assert_eq!((r.pressure, r.tilt_x), (p.pressure, p.tilt_x));
        }
        let s = PresetSymmetry::new(SymmetryMode::Dual, [20.0, 30.0], 0.0).unwrap();
        let copies = s.reflected_passes(&[p]);
        assert_eq!(copies.len(), 3);
        for (p, e) in copies.iter().zip([[28.0, 25.0], [12.0, 35.0], [28.0, 35.0]]) {
            assert!((p[0].x - e[0]).abs() < 1e-9 && (p[0].y - e[1]).abs() < 1e-9);
        }
    }
    #[test]
    fn rotated_axes_clip_on_rectangles_and_accept_half_pixel_centres() {
        for size in [[100.0, 100.0], [101.0, 53.0]] {
            for mode in SymmetryMode::ALL {
                let s = PresetSymmetry::new(mode, [size[0] / 2.0, size[1] / 2.0], 0.0).unwrap();
                let segments = s.segments(size);
                assert_eq!(segments.len(), if mode == SymmetryMode::Dual { 2 } else { 1 });
                for [a, b] in segments {
                    for p in [a, b] {
                        assert!(p[0] >= -1e-9 && p[0] <= size[0] + 1e-9 && p[1] >= -1e-9 && p[1] <= size[1] + 1e-9);
                    }
                }
                if mode == SymmetryMode::Diagonal {
                    let d = s.directions()[0];
                    assert!((d[0] - d[1]).abs() < 1e-9);
                }
            }
        }
        let s = PresetSymmetry::new(SymmetryMode::Vertical, [20.0, 30.0], 90.0).unwrap();
        let p = s.reflected_passes(&[StrokePoint::new(12.0, 25.0, 1.0)])[0][0];
        assert!((p.x - 12.0).abs() < 1e-9 && (p.y - 35.0).abs() < 1e-9);
        assert!(PresetSymmetry::new(SymmetryMode::Dual, [f64::NAN, 0.0], 0.0).is_none());
        assert!(PresetSymmetry::new(SymmetryMode::Dual, [0.0, 0.0], f64::INFINITY).is_none());
        assert!(PresetSymmetry::new(SymmetryMode::Dual, [MAX_COORDINATE + 1.0, 0.0], 0.0).is_none());
        assert!(PresetSymmetry::new(SymmetryMode::Horizontal, [20.0, -5.0], 0.0).unwrap().segments([100.0, 60.0]).is_empty());
    }
    #[test]
    fn rotational_copies_keep_distance_and_wrap_shifts_by_the_canvas() {
        let r = RotationalSymmetry::new([50.0, 50.0], 4, false).unwrap();
        let p = StrokePoint { x: 60.0, y: 50.0, pressure: 0.7, ..Default::default() };
        let copies = r.copies(&[p]);
        assert_eq!(copies.len(), r.mirror_count());
        assert_eq!(copies.len(), 3);
        for (c, e) in copies.iter().zip([[50.0, 60.0], [40.0, 50.0], [50.0, 40.0]]) {
            assert!((c[0].x - e[0]).abs() < 1e-9 && (c[0].y - e[1]).abs() < 1e-9, "{:?} vs {e:?}", c[0]);
            assert_eq!(c[0].pressure, 0.7);
        }
        let m = RotationalSymmetry::new([50.0, 50.0], 3, true).unwrap();
        assert_eq!(m.copies(&[p]).len(), 5);
        assert_eq!(m.mirror_count(), 5);
        assert_eq!(m.spokes().len(), 3);
        assert!(RotationalSymmetry::new([50.0, 50.0], 1, false).is_none());
        assert!(RotationalSymmetry::new([f64::NAN, 50.0], 4, false).is_none());
        let w = wrapped_passes(&[p], [100.0, 80.0]);
        assert_eq!(w.len(), 8);
        assert!(w.iter().any(|c| c[0].x == -40.0 && c[0].y == 50.0));
        assert!(w.iter().any(|c| c[0].x == 160.0 && c[0].y == 130.0));
    }
}
