//! A straight-RGBA paint buffer the line-based engines draw into, then composite onto a surface
//! through the retouch machinery ([`crate::retouch::apply_coverage`]), so every engine works on any
//! pixel format, honours the selection and lock-transparency, and can use a blend mode.

use redrob_photo_color::BlendMode;
use redrob_photo_geom::Rect;
use redrob_photo_raster::{Surface, from_rgba};

use crate::retouch::{Region, alpha_index, apply_coverage};

/// Largest buffer side an engine stroke may cover (a runaway stroke never allocates gigabytes).
pub const MAX_SIDE: i32 = 16_384;

/// Straight RGBA pixels over a rectangle; alpha is the coverage the pixel is painted with.
#[derive(Clone, Debug)]
pub struct PaintBuf {
    pub rect: Rect,
    pub px: Vec<[f32; 4]>,
    /// Marks build up on each other (source over) instead of only raising the coverage.
    pub accumulate: bool,
}

/// Blend a straight-RGBA colour `src` over `dst` at coverage `k` (source over).
#[inline]
pub fn over_rgba(dst: &mut [f32; 4], src: [f32; 4], k: f32) {
    let sa = (src[3] * k).clamp(0.0, 1.0);
    if sa <= 0.0 {
        return;
    }
    let da = dst[3];
    let oa = sa + da * (1.0 - sa);
    if oa > 0.0 {
        for c in 0..3 {
            dst[c] = (src[c] * sa + dst[c] * da * (1.0 - sa)) / oa;
        }
    }
    dst[3] = oa;
}

impl PaintBuf {
    /// An empty (transparent) buffer over `rect`, clamped to [`MAX_SIDE`].
    pub fn new(rect: Rect) -> Self {
        let rect = Rect::new(rect.x0, rect.y0, rect.x1.min(rect.x0.saturating_add(MAX_SIDE)), rect.y1.min(rect.y0.saturating_add(MAX_SIDE)));
        let n = if rect.is_empty() { 0 } else { rect.width() as usize * rect.height() as usize };
        Self { rect, px: vec![[0.0; 4]; n], accumulate: false }
    }

    /// Buffer index of a document pixel, `None` outside the rectangle.
    #[inline]
    pub fn index(&self, x: i32, y: i32) -> Option<usize> {
        if x < self.rect.x0 || y < self.rect.y0 || x >= self.rect.x1 || y >= self.rect.y1 {
            return None;
        }
        Some((y - self.rect.y0) as usize * self.rect.width() as usize + (x - self.rect.x0) as usize)
    }

    /// Read a pixel (transparent outside).
    #[inline]
    pub fn get(&self, x: i32, y: i32) -> [f32; 4] {
        self.index(x, y).map_or([0.0; 4], |i| self.px[i])
    }

    /// Source-over a colour at coverage `k` on one pixel.
    #[inline]
    pub fn blend(&mut self, x: i32, y: i32, color: [f32; 4], k: f32) {
        if let Some(i) = self.index(x, y) {
            over_rgba(&mut self.px[i], color, k);
        }
    }

    /// Paint a colour where the pixel's coverage is below `k`: overlapping marks of one dab do not
    /// build up (the way a tip's own pixels don't).
    #[inline]
    pub fn max(&mut self, x: i32, y: i32, color: [f32; 4], k: f32) {
        if let Some(i) = self.index(x, y) {
            if self.accumulate {
                over_rgba(&mut self.px[i], color, k);
                return;
            }
            let p = &mut self.px[i];
            let k = (color[3] * k).clamp(0.0, 1.0);
            if k > p[3] {
                let t = if p[3] > 0.0 { (k - p[3]) / k } else { 1.0 };
                for c in 0..3 {
                    p[c] += (color[c] - p[c]) * t;
                }
                p[3] = k;
            }
        }
    }

    /// An anti-aliased pixel at a fractional position: the coverage is split over the 2×2 pixels
    /// it overlaps.
    pub fn point_aa(&mut self, x: f32, y: f32, color: [f32; 4], k: f32) {
        if !x.is_finite() || !y.is_finite() {
            return;
        }
        let (fx, fy) = (x - 0.5, y - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = (fx - x0, fy - y0);
        let (x0, y0) = (x0 as i32, y0 as i32);
        self.blend(x0, y0, color, k * (1.0 - tx) * (1.0 - ty));
        self.blend(x0 + 1, y0, color, k * tx * (1.0 - ty));
        self.blend(x0, y0 + 1, color, k * (1.0 - tx) * ty);
        self.blend(x0 + 1, y0 + 1, color, k * tx * ty);
    }

    /// A whole pixel at a position (no anti-aliasing).
    pub fn point(&mut self, x: f32, y: f32, color: [f32; 4], k: f32) {
        if x.is_finite() && y.is_finite() {
            self.blend(x.floor() as i32, y.floor() as i32, color, k);
        }
    }

    /// A line of `width` pixels from `(x0, y0)` to `(x1, y1)` (pixel-centre coordinates) at
    /// coverage `k`. Anti-aliased unless `aa` is off; the line's own pixels never build up on each
    /// other (`max`), so a thick line is flat.
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: [f32; 4], k: f32, aa: bool) {
        if ![x0, y0, x1, y1].iter().all(|v| v.is_finite()) {
            return;
        }
        let hw = (width.max(0.0) / 2.0).min(MAX_SIDE as f32);
        let (dx, dy) = (x1 - x0, y1 - y0);
        let len = dx.hypot(dy);
        if len < 1e-4 {
            if aa {
                self.disc(x0, y0, hw.max(0.5), color, k, true);
            } else {
                self.point(x0, y0, color, k);
            }
            return;
        }
        if len > MAX_SIDE as f32 * 2.0 {
            return;
        }
        let (ux, uy) = (dx / len, dy / len);
        let reach = hw + 1.0;
        // Clip to the buffer with slack, so a line mostly off the buffer costs little.
        let r = self.rect;
        let bx0 = (x0.min(x1) - reach).floor().max(r.x0 as f32) as i32;
        let bx1 = (x0.max(x1) + reach).ceil().min(r.x1 as f32) as i32;
        let by0 = (y0.min(y1) - reach).floor().max(r.y0 as f32) as i32;
        let by1 = (y0.max(y1) + reach).ceil().min(r.y1 as f32) as i32;
        if bx0 >= bx1 || by0 >= by1 {
            return;
        }
        let cover = |d: f32| -> f32 {
            if aa {
                (hw + 0.5 - d).clamp(0.0, 1.0)
            } else if d <= hw.max(0.5) {
                1.0
            } else {
                0.0
            }
        };
        if hw <= 1.5 || (bx1 - bx0) as i64 * (by1 - by0) as i64 <= 4096 {
            // Small: the bounding box, exact distance to the segment.
            for y in by0..by1 {
                for x in bx0..bx1 {
                    let (px, py) = (x as f32 + 0.5 - x0, y as f32 + 0.5 - y0);
                    let t = (px * ux + py * uy).clamp(0.0, len);
                    let d = (px - t * ux).hypot(py - t * uy);
                    let c = cover(d);
                    if c > 0.0 {
                        self.max(x, y, color, k * c);
                    }
                }
            }
            return;
        }
        // Long and thick: walk the major axis, sweep the minor one over the line's width.
        let steps = len.ceil() as i32;
        let span = (reach / ux.abs().max(uy.abs()).max(0.2)).ceil() as i32 + 1;
        let horizontal = ux.abs() >= uy.abs();
        let mut done = std::collections::HashSet::new();
        for i in -1..=steps + 1 {
            let t = (i as f32).clamp(0.0, len);
            let (cx, cy) = (x0 + ux * t, y0 + uy * t);
            for j in -span..=span {
                let (sx, sy) = if horizontal { (cx, cy + j as f32) } else { (cx + j as f32, cy) };
                let (x, y) = (sx.floor() as i32, sy.floor() as i32);
                if x < bx0 || x >= bx1 || y < by0 || y >= by1 || !done.insert((x, y)) {
                    continue;
                }
                let (px, py) = (x as f32 + 0.5 - x0, y as f32 + 0.5 - y0);
                let tt = (px * ux + py * uy).clamp(0.0, len);
                let d = (px - tt * ux).hypot(py - tt * uy);
                let c = cover(d);
                if c > 0.0 {
                    self.max(x, y, color, k * c);
                }
            }
        }
    }

    /// A filled disc.
    pub fn disc(&mut self, cx: f32, cy: f32, radius: f32, color: [f32; 4], k: f32, aa: bool) {
        self.ellipse(cx, cy, radius, radius, 0.0, color, k, aa);
    }

    /// A filled ellipse with radii `rx`, `ry`, turned by `angle` radians (counter-clockwise).
    pub fn ellipse(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, angle: f32, color: [f32; 4], k: f32, aa: bool) {
        if ![cx, cy, rx, ry, angle].iter().all(|v| v.is_finite()) {
            return;
        }
        let (rx, ry) = (rx.max(0.0).min(MAX_SIDE as f32), ry.max(0.0).min(MAX_SIDE as f32));
        let reach = rx.max(ry) + 1.0;
        let r = self.rect;
        let bx0 = (cx - reach).floor().max(r.x0 as f32) as i32;
        let bx1 = (cx + reach).ceil().min(r.x1 as f32) as i32;
        let by0 = (cy - reach).floor().max(r.y0 as f32) as i32;
        let by1 = (cy + reach).ceil().min(r.y1 as f32) as i32;
        let (ca, sa) = (angle.cos(), -angle.sin());
        let (rx, ry) = (rx.max(0.25), ry.max(0.25));
        for y in by0..by1 {
            for x in bx0..bx1 {
                let (px, py) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                let (u, v) = ((px * ca - py * sa) / rx, (px * sa + py * ca) / ry);
                // Distance to the rim in pixels, approximated along the radial direction.
                let rr = (u * u + v * v).sqrt();
                let edge = (rx.min(ry)).max(0.5);
                let d = (rr - 1.0) * edge;
                let c = if aa {
                    (0.5 - d).clamp(0.0, 1.0)
                } else if rr <= 1.0 {
                    1.0
                } else {
                    0.0
                };
                if c > 0.0 {
                    self.max(x, y, color, k * c);
                }
            }
        }
    }

    /// A filled rectangle `w × h` centred at `(cx, cy)`, turned by `angle` radians.
    pub fn rect_shape(&mut self, cx: f32, cy: f32, w: f32, h: f32, angle: f32, color: [f32; 4], k: f32, aa: bool) {
        if ![cx, cy, w, h, angle].iter().all(|v| v.is_finite()) {
            return;
        }
        let (hw, hh) = ((w / 2.0).max(0.0).min(MAX_SIDE as f32), (h / 2.0).max(0.0).min(MAX_SIDE as f32));
        let reach = hw.hypot(hh) + 1.0;
        let r = self.rect;
        let bx0 = (cx - reach).floor().max(r.x0 as f32) as i32;
        let bx1 = (cx + reach).ceil().min(r.x1 as f32) as i32;
        let by0 = (cy - reach).floor().max(r.y0 as f32) as i32;
        let by1 = (cy + reach).ceil().min(r.y1 as f32) as i32;
        let (ca, sa) = (angle.cos(), -angle.sin());
        for y in by0..by1 {
            for x in bx0..bx1 {
                let (px, py) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                let (u, v) = (px * ca - py * sa, px * sa + py * ca);
                let d = (u.abs() - hw).max(v.abs() - hh);
                let c = if aa {
                    (0.5 - d).clamp(0.0, 1.0)
                } else if d <= 0.0 {
                    1.0
                } else {
                    0.0
                };
                if c > 0.0 {
                    self.max(x, y, color, k * c);
                }
            }
        }
    }

    /// Fill a closed polygon (pixel-centre coordinates). `even_odd` false = non-zero winding.
    /// Anti-aliased by 4×4 supersampling unless `hard`.
    pub fn polygon(&mut self, pts: &[[f32; 2]], color: [f32; 4], k: f32, even_odd: bool, hard: bool) {
        if pts.len() < 3 || pts.iter().any(|p| !p[0].is_finite() || !p[1].is_finite()) {
            return;
        }
        let r = self.rect;
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for p in pts {
            x0 = x0.min(p[0]);
            y0 = y0.min(p[1]);
            x1 = x1.max(p[0]);
            y1 = y1.max(p[1]);
        }
        let bx0 = (x0 - 1.0).floor().max(r.x0 as f32) as i32;
        let bx1 = (x1 + 1.0).ceil().min(r.x1 as f32) as i32;
        let by0 = (y0 - 1.0).floor().max(r.y0 as f32) as i32;
        let by1 = (y1 + 1.0).ceil().min(r.y1 as f32) as i32;
        let n = pts.len();
        let winding = |sx: f32, sy: f32| -> i32 {
            let mut w = 0;
            for i in 0..n {
                let (a, b) = (pts[i], pts[(i + 1) % n]);
                if (a[1] <= sy) != (b[1] <= sy) {
                    let t = (sy - a[1]) / (b[1] - a[1]);
                    let x = a[0] + (b[0] - a[0]) * t;
                    if x > sx {
                        w += if b[1] > a[1] { 1 } else { -1 };
                    }
                }
            }
            w
        };
        let inside = |w: i32| if even_odd { w.rem_euclid(2) == 1 } else { w != 0 };
        let sub: i32 = if hard { 1 } else { 4 };
        for y in by0..by1 {
            for x in bx0..bx1 {
                let mut hits = 0;
                for sy in 0..sub {
                    for sx in 0..sub {
                        let fx = x as f32 + (sx as f32 + 0.5) / sub as f32;
                        let fy = y as f32 + (sy as f32 + 0.5) / sub as f32;
                        if inside(winding(fx, fy)) {
                            hits += 1;
                        }
                    }
                }
                if hits > 0 {
                    self.max(x, y, color, k * hits as f32 / (sub * sub) as f32);
                }
            }
        }
    }

    /// The bounding rectangle of the painted pixels (empty when nothing was painted).
    pub fn painted_bounds(&self) -> Rect {
        let w = self.rect.width() as i32;
        let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        for (i, p) in self.px.iter().enumerate() {
            if p[3] > 0.0 {
                let (x, y) = (self.rect.x0 + (i as i32 % w), self.rect.y0 + (i as i32 / w));
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
        if x0 > x1 { Rect::EMPTY } else { Rect::new(x0, y0, x1, y1) }
    }

    /// Composite the buffer onto `target` at `opacity`, through the selection, in `blend` mode.
    /// Returns the damaged rectangle.
    pub fn composite(&self, target: &mut Surface, opacity: f32, selection: Option<&Surface>, lock_transparency: bool, blend: BlendMode) -> Rect {
        let rect = self.painted_bounds();
        if rect.is_empty() {
            return Rect::EMPTY;
        }
        let fmt = target.format();
        let a = alpha_index(&fmt);
        let mut paint = Region::new(rect, fmt.channels());
        let mut cov = Vec::with_capacity(rect.width() as usize * rect.height() as usize);
        for y in rect.y0..rect.y1 {
            for x in rect.x0..rect.x1 {
                let p = self.get(x, y);
                cov.push(p[3].clamp(0.0, 1.0));
                let mut v = from_rgba(&fmt, [p[0], p[1], p[2], 1.0]);
                if let Some(ai) = a {
                    v[ai] = 1.0;
                }
                paint.px_mut(x, y).copy_from_slice(&v[..fmt.channels()]);
            }
        }
        apply_coverage(target, rect, &cov, opacity.clamp(0.0, 1.0), selection, lock_transparency, &paint, blend)
    }
}

/// Document bounds a stroke's points span, grown by `reach` pixels (a budget for what an engine
/// draws around them), clipped to a sane size.
pub fn points_bounds(pts: &[crate::StrokePoint], reach: f32) -> Rect {
    let mut b: Option<(f64, f64, f64, f64)> = None;
    for p in pts {
        if !p.x.is_finite() || !p.y.is_finite() {
            continue;
        }
        b = Some(match b {
            None => (p.x, p.y, p.x, p.y),
            Some((x0, y0, x1, y1)) => (x0.min(p.x), y0.min(p.y), x1.max(p.x), y1.max(p.y)),
        });
    }
    let Some((x0, y0, x1, y1)) = b else { return Rect::EMPTY };
    let r = f64::from(if reach.is_finite() { reach.max(0.0) } else { 0.0 }) + 2.0;
    let clamp = |v: f64| v.clamp(-1e7, 1e7);
    Rect::new(clamp(x0 - r).floor() as i32, clamp(y0 - r).floor() as i32, clamp(x1 + r).ceil() as i32, clamp(y1 + r).ceil() as i32)
}

/// Straight RGB → HSV (all 0..1).
pub fn rgb_to_hsv(c: [f32; 3]) -> [f32; 3] {
    let mx = c[0].max(c[1]).max(c[2]);
    let mn = c[0].min(c[1]).min(c[2]);
    let d = mx - mn;
    let h = if d <= 0.0 {
        0.0
    } else if mx == c[0] {
        ((c[1] - c[2]) / d).rem_euclid(6.0)
    } else if mx == c[1] {
        (c[2] - c[0]) / d + 2.0
    } else {
        (c[0] - c[1]) / d + 4.0
    } / 6.0;
    [h, if mx > 0.0 { d / mx } else { 0.0 }, mx]
}

/// HSV → straight RGB (all 0..1; hue wraps).
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h6 = h.rem_euclid(1.0) * 6.0;
    let i = h6.floor();
    let f = h6 - i;
    let (s, v) = (s.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    match i as i32 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

/// Shift a straight RGBA colour by `dh` of the hue wheel, `ds` saturation and `dv` value.
pub fn shift_hsv(c: [f32; 4], dh: f32, ds: f32, dv: f32) -> [f32; 4] {
    let [h, s, v] = rgb_to_hsv([c[0], c[1], c[2]]);
    let o = hsv_to_rgb(h + dh, (s + ds).clamp(0.0, 1.0), (v + dv).clamp(0.0, 1.0));
    [o[0], o[1], o[2], c[3]]
}

/// A straight RGBA colour read from a surface (any format), `[0,0,0,0]` where it has no pixels.
#[inline]
pub fn surface_rgba(s: &Surface, x: f32, y: f32) -> [f32; 4] {
    if !x.is_finite() || !y.is_finite() {
        return [0.0; 4];
    }
    s.rgba(x.floor() as i32, y.floor() as i32)
}
