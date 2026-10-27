//! Distortions of the third batch: Engrave, Kaleidoscope, Shift, Value Propagate, Video
//! Degradation, Blinds, Page Curl.

use redrob_photo_geom::Rect;

use super::{alpha_at, lerp4, luma_rgb, map_px, over, rgba_at};
use crate::fxutil::{MAXC, set_rgba};
use crate::image::{Edge, Image};
use crate::noise::hash01;
use crate::{Axis, Ctx, CurlColors, CurlCorner, PropagateMode, VideoPattern};

/// Engrave: strips `height` pixels tall (anchored at the bounds) become black lines whose
/// thickness grows with the strip column's darkness, as in a copper engraving.
pub(crate) fn engrave(src: &Image, out: Rect, ctx: &Ctx, height: f32, limit_line_width: bool) -> Vec<f32> {
    let h = height.clamp(2.0, 16.0).round() as i32;
    let b = ctx.bounds;
    let n = src.ch;
    let mut res = src.crop(out);
    let w = out.width() as usize;
    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        let (x, y) = (out.x0 + (i % w) as i32, out.y0 + (i / w) as i32);
        let strip = (y - b.y0).div_euclid(h);
        let y0 = b.y0 + strip * h;
        let mut sum = 0.0;
        for yy in y0..y0 + h {
            let c = rgba_at(src, ctx, x, yy);
            sum += (1.0 - luma_rgb(c)) * c[3];
        }
        let dark = sum / h as f32;
        let mut thick = dark * h as f32;
        if limit_line_width {
            thick = thick.min((h - 1) as f32);
        }
        let k = (y - b.y0).rem_euclid(h) as f32;
        let a = alpha_at(src, ctx, x, y);
        let v = if k + 0.5 < thick { 0.0 } else { 1.0 };
        set_rgba(ctx, px, [v, v, v, a]);
    }
    res
}

/// Kaleidoscope parameters.
pub(crate) struct KaleidoscopeSpec {
    pub mirrors: u32,
    pub mirror_rotation: f32,
    pub result_rotation: f32,
    pub offset: (f32, f32),
    pub zoom: f32,
    pub trim: (f32, f32),
    pub clip: bool,
}

/// Kaleidoscope: the plane is folded into `mirrors` wedges around a centre (`offset` as a fraction
/// of the bounds); every output pixel samples the input through the fold. `zoom` scales the
/// input (100 = 1:1); `trim` shrinks the usable input; outside it the input wraps, or the
/// result is clipped to transparency.
pub(crate) fn kaleidoscope(src: &Image, out: Rect, ctx: &Ctx, spec: &KaleidoscopeSpec) -> Vec<f32> {
    let b = ctx.bounds;
    let (bw, bh) = (b.width() as f32, b.height() as f32);
    let (cx, cy) = (b.x0 as f32 + spec.offset.0 * bw, b.y0 as f32 + spec.offset.1 * bh);
    let m = spec.mirrors.clamp(1, 24) as f32;
    let wedge = std::f32::consts::PI / m;
    let zoom = (spec.zoom / 100.0).clamp(0.01, 10.0);
    let mr = spec.mirror_rotation.to_radians();
    let rr = spec.result_rotation.to_radians();
    let inner = Rect::new(
        b.x0 + (bw * spec.trim.0 * 0.5) as i32,
        b.y0 + (bh * spec.trim.1 * 0.5) as i32,
        b.x1 - (bw * spec.trim.0 * 0.5) as i32,
        b.y1 - (bh * spec.trim.1 * 0.5) as i32,
    );
    let inner = if inner.is_empty() { b } else { inner };
    let edge = if spec.clip { Edge::Transparent } else { Edge::Wrap };
    let n = src.ch;
    let mut res = Vec::with_capacity(out.width() as usize * out.height() as usize * n);
    let mut tmp = vec![0.0f32; n];
    for y in out.y0..out.y1 {
        for x in out.x0..out.x1 {
            let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
            let r = (dx * dx + dy * dy).sqrt();
            let th = dy.atan2(dx) - rr - mr;
            let mut t = th.rem_euclid(2.0 * wedge);
            if t > wedge {
                t = 2.0 * wedge - t;
            }
            let t = t + mr;
            let (s, c) = t.sin_cos();
            let (sx, sy) = (cx + r * c / zoom, cy + r * s / zoom);
            src.sample(sx, sy, edge, inner, ctx.alpha, &mut tmp);
            if spec.clip && !ctx.alpha && !inner.contains(sx.floor() as i32, sy.floor() as i32) {
                let mut px = [0.0f32; MAXC];
                set_rgba(ctx, &mut px[..n], [0.0, 0.0, 0.0, 1.0]);
                tmp.copy_from_slice(&px[..n]);
            }
            res.extend_from_slice(&tmp);
        }
    }
    res
}

/// Shift: every row (or column) slides sideways by a random whole number of pixels up to
/// `amount`, the same for every tile.
pub(crate) fn shift(src: &Image, out: Rect, ctx: &Ctx, amount: f32, axis: Axis, seed: u32) -> Vec<f32> {
    let a = amount.clamp(0.0, 200.0);
    let b = ctx.bounds;
    let n = src.ch;
    let mut res = Vec::with_capacity(out.width() as usize * out.height() as usize * n);
    for y in out.y0..out.y1 {
        for x in out.x0..out.x1 {
            let (sx, sy) = match axis {
                Axis::Horizontal => {
                    let d = ((hash01(y - b.y0, 0, 5, seed) * 2.0 - 1.0) * a).round() as i32;
                    (x - d, y)
                }
                Axis::Vertical => {
                    let d = ((hash01(0, x - b.x0, 6, seed) * 2.0 - 1.0) * a).round() as i32;
                    (x, y - d)
                }
            };
            for c in 0..n {
                res.push(src.get_edge(sx, sy, c, Edge::Repeat, src.rect));
            }
        }
    }
    res
}

/// Value Propagate parameters.
pub(crate) struct PropagateSpec {
    pub mode: PropagateMode,
    pub lower: f32,
    pub upper: f32,
    pub rate: f32,
    pub color: [f32; 4],
    pub directions: [bool; 4],
}

/// Value Propagate: each pixel moves toward the neighbour the mode prefers (lighter, darker,
/// nearer the middle, the chosen colour, more or less opaque) by `rate`, when the difference lies
/// between the thresholds. `directions` = [top, left, right, bottom] limits where values come
/// from (a value propagating toward the top arrives from below).
pub(crate) fn value_propagate(src: &Image, out: Rect, ctx: &Ctx, spec: &PropagateSpec) -> Vec<f32> {
    let [top, left, right, bottom] = spec.directions;
    let allowed = |dx: i32, dy: i32| -> bool {
        let v_ok = match dy {
            1 => top,
            -1 => bottom,
            _ => true,
        };
        let h_ok = match dx {
            1 => left,
            -1 => right,
            _ => true,
        };
        v_ok && h_ok && !(dx == 0 && dy == 0)
    };
    let within = |d: f32| (spec.lower..=spec.upper).contains(&d.abs());
    let rate = spec.rate.clamp(0.0, 1.0);
    let close = |a: [f32; 4], b: [f32; 4]| (a[0] - b[0]).abs() + (a[1] - b[1]).abs() + (a[2] - b[2]).abs() < 0.05;
    map_px(src, out, ctx, |x, y, me| {
        let mut nbs: [([f32; 4], bool); 9] = [([0.0; 4], false); 9];
        for dy in -1..=1 {
            for dx in -1..=1 {
                if allowed(dx, dy) {
                    nbs[((dy + 1) * 3 + dx + 1) as usize] = (rgba_at(src, ctx, x + dx, y + dy), true);
                }
            }
        }
        let active = nbs.iter().filter(|n| n.1).map(|n| n.0);
        let my_l = luma_rgb(me);
        match spec.mode {
            PropagateMode::MoreWhite | PropagateMode::MoreBlack => {
                let pick = active.fold(None::<[f32; 4]>, |acc, c| match acc {
                    None => Some(c),
                    Some(a) => {
                        let (la, lc) = (luma_rgb(a), luma_rgb(c));
                        let better = if spec.mode == PropagateMode::MoreWhite { lc > la } else { lc < la };
                        Some(if better { c } else { a })
                    }
                });
                match pick {
                    Some(c) => {
                        let d = luma_rgb(c) - my_l;
                        let wanted = if spec.mode == PropagateMode::MoreWhite { d > 0.0 } else { d < 0.0 };
                        if wanted && within(d) {
                            let mut o = lerp4(me, c, rate);
                            o[3] = me[3];
                            o
                        } else {
                            me
                        }
                    }
                    None => me,
                }
            }
            PropagateMode::MiddleValue => {
                let (mut lo, mut hi) = (my_l, my_l);
                let list: Vec<[f32; 4]> = active.collect();
                for c in &list {
                    let l = luma_rgb(*c);
                    lo = lo.min(l);
                    hi = hi.max(l);
                }
                let mid = 0.5 * (lo + hi);
                let best = list.iter().copied().min_by(|p, q| (luma_rgb(*p) - mid).abs().total_cmp(&(luma_rgb(*q) - mid).abs()));
                match best {
                    Some(c) if within(luma_rgb(c) - my_l) && (luma_rgb(c) - mid).abs() < (my_l - mid).abs() => {
                        let mut o = lerp4(me, c, rate);
                        o[3] = me[3];
                        o
                    }
                    _ => me,
                }
            }
            PropagateMode::ForegroundToPeaks | PropagateMode::OnlyForeground => {
                let touches = active.clone().any(|c| close(c, spec.color));
                let peak = spec.mode == PropagateMode::OnlyForeground || active.clone().all(|c| luma_rgb(c) <= my_l + 1e-3);
                if touches && !close(me, spec.color) && peak && within(luma_rgb(spec.color) - my_l) {
                    let mut o = lerp4(me, spec.color, rate);
                    o[3] = me[3];
                    o
                } else {
                    me
                }
            }
            PropagateMode::OnlyBackground => {
                if !close(me, spec.color) {
                    return me;
                }
                match active.clone().find(|c| !close(*c, spec.color)) {
                    Some(c) if within(luma_rgb(c) - my_l) => {
                        let mut o = lerp4(me, c, rate);
                        o[3] = me[3];
                        o
                    }
                    _ => me,
                }
            }
            PropagateMode::MoreOpaque | PropagateMode::MoreTransparent => {
                let more = spec.mode == PropagateMode::MoreOpaque;
                let pick = active.fold(None::<[f32; 4]>, |acc, c| match acc {
                    None => Some(c),
                    Some(a) => Some(if (c[3] > a[3]) == more { c } else { a }),
                });
                match pick {
                    Some(c) if ((c[3] > me[3]) == more) && c[3] != me[3] && within(c[3] - me[3]) => {
                        let mut o = me;
                        if more {
                            o = lerp4(me, c, rate);
                        }
                        o[3] = me[3] + (c[3] - me[3]) * rate;
                        o
                    }
                    _ => me,
                }
            }
        }
    })
}

/// Which colour channel a Video Degradation pattern shows at `(x, y)` (bounds-relative).
fn video_channel(pattern: VideoPattern, x: i32, y: i32) -> usize {
    const STAGGER: [[usize; 3]; 3] = [[0, 1, 2], [2, 0, 1], [1, 2, 0]];
    let at = |g: &[[usize; 3]; 3], x: i32, y: i32| g[y.rem_euclid(3) as usize][x.rem_euclid(3) as usize];
    match pattern {
        VideoPattern::Staggered => at(&STAGGER, x, y),
        VideoPattern::LargeStaggered => at(&STAGGER, x.div_euclid(2), y.div_euclid(2)),
        VideoPattern::Striped => x.rem_euclid(3) as usize,
        VideoPattern::WideStriped => x.div_euclid(2).rem_euclid(3) as usize,
        VideoPattern::LongStaggered => at(&STAGGER, x, y.div_euclid(2)),
        VideoPattern::ThreeByThree => (x.div_euclid(3) + y.div_euclid(3)).rem_euclid(3) as usize,
        VideoPattern::LargeThreeByThree => (x.div_euclid(6) + y.div_euclid(6)).rem_euclid(3) as usize,
        VideoPattern::Hex => (x + y.div_euclid(2) * 2 + (y.rem_euclid(2))).rem_euclid(3) as usize,
        VideoPattern::Dots => {
            let (cx, cy) = (x.rem_euclid(2), y.rem_euclid(2));
            if cx == 1 && cy == 1 { 3 } else { at(&STAGGER, x.div_euclid(2), y.div_euclid(2)) }
        }
    }
}

/// Video Degradation: only one colour channel survives per pixel, laid out in a repeating
/// sub-pixel pattern (4 = a dark gap between dots). Additive keeps the other channels at half
/// strength instead of black; rotated swaps the pattern's axes.
pub(crate) fn video_degradation(src: &Image, out: Rect, ctx: &Ctx, pattern: VideoPattern, additive: bool, rotated: bool) -> Vec<f32> {
    let b = ctx.bounds;
    map_px(src, out, ctx, |x, y, c| {
        let (px, py) = if rotated { (y - b.y0, x - b.x0) } else { (x - b.x0, y - b.y0) };
        let ch = video_channel(pattern, px, py);
        let mut o = c;
        for k in 0..3 {
            o[k] = if k == ch {
                if additive { (c[k] * 1.5).min(1.0) } else { c[k] }
            } else if additive {
                c[k] * 0.5
            } else {
                0.0
            };
        }
        o
    })
}

/// Blinds: the bounds are cut into `segments` strips which pivot about their centre line by
/// `displacement` degrees, so each shows a compressed copy of its content with the background
/// (or transparency) in the gaps.
#[allow(clippy::too_many_arguments)]
pub(crate) fn blinds(src: &Image, out: Rect, ctx: &Ctx, displacement: f32, segments: u32, axis: Axis, transparent: bool, background: [f32; 4]) -> Vec<f32> {
    let b = ctx.bounds;
    let k = segments.clamp(1, 10) as f32;
    let visible = displacement.clamp(1.0, 90.0).to_radians().cos().max(0.02);
    let n = src.ch;
    let mut res = Vec::with_capacity(out.width() as usize * out.height() as usize * n);
    let mut tmp = vec![0.0f32; n];
    let mut bgpx = [0.0f32; MAXC];
    set_rgba(ctx, &mut bgpx[..n], if transparent && ctx.alpha { [0.0; 4] } else { [background[0], background[1], background[2], 1.0] });
    for y in out.y0..out.y1 {
        for x in out.x0..out.x1 {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let (coord, start, len) = match axis {
                Axis::Horizontal => (fy, b.y0 as f32, b.height() as f32),
                Axis::Vertical => (fx, b.x0 as f32, b.width() as f32),
            };
            let strip = len / k;
            let t = ((coord - start) / strip).clamp(0.0, k - 1e-4);
            let local = t - t.floor();
            let d = (local - 0.5) / visible;
            if d.abs() <= 0.5 {
                let mapped = start + (t.floor() + 0.5 + d) * strip;
                let (sx, sy) = match axis {
                    Axis::Horizontal => (fx, mapped),
                    Axis::Vertical => (mapped, fy),
                };
                src.sample(sx, sy, Edge::Repeat, b, ctx.alpha, &mut tmp);
                // Face the light: the compressed strip darkens toward its hinge.
                let shade = 1.0 - 0.35 * (1.0 - visible) * (d + 0.5);
                let c = crate::fxutil::rgba(ctx, &tmp);
                let mut px = [0.0f32; MAXC];
                set_rgba(ctx, &mut px[..n], [c[0] * shade, c[1] * shade, c[2] * shade, c[3]]);
                res.extend_from_slice(&px[..n]);
            } else {
                res.extend_from_slice(&bgpx[..n]);
            }
        }
    }
    res
}

/// Page Curl parameters.
pub(crate) struct CurlSpec {
    pub corner: CurlCorner,
    pub axis: Axis,
    pub shade: bool,
    pub colors: CurlColors,
    pub opacity: f32,
    pub foreground: [f32; 4],
    pub background: [f32; 4],
}

/// Page Curl: one corner rolls back over a cylinder whose axis runs diagonally across the
/// corner. The lifted part of the page becomes transparent, the back of the page shows the
/// foreground/background colours (or a gradient between them) with cylindrical shading, and the
/// front just before the fold is shaded when `shade` is on.
pub(crate) fn page_curl(src: &Image, out: Rect, ctx: &Ctx, spec: &CurlSpec) -> Vec<f32> {
    let b = ctx.bounds;
    let (bw, bh) = (b.width() as f32, b.height() as f32);
    // Corner point and the inward unit normal of the fold line.
    let (corner, sx, sy) = match spec.corner {
        CurlCorner::UpperLeft => ((b.x0 as f32, b.y0 as f32), 1.0, 1.0),
        CurlCorner::UpperRight => ((b.x1 as f32, b.y0 as f32), -1.0, 1.0),
        CurlCorner::LowerLeft => ((b.x0 as f32, b.y1 as f32), 1.0, -1.0),
        CurlCorner::LowerRight => ((b.x1 as f32, b.y1 as f32), -1.0, -1.0),
    };
    // The fold line crosses the two edges at these distances from the corner.
    let (ex, ey) = match spec.axis {
        Axis::Horizontal => (bw * 0.75, bh * 0.45),
        Axis::Vertical => (bw * 0.45, bh * 0.75),
    };
    // Normal of the line through (ex, 0) and (0, ey) in corner-relative coordinates, pointing
    // toward the corner.
    let len = (ex * ex + ey * ey).sqrt().max(1.0);
    let (nx, ny) = (ey / len, ex / len);
    let line_d = ex * ey / len;
    let radius = (bw.min(bh) * 0.16).max(2.0);
    let roll = std::f32::consts::FRAC_PI_2 * radius;
    let opacity = (spec.opacity / 100.0).clamp(0.0, 1.0);
    let (c0, c1) = match spec.colors {
        CurlColors::ForegroundBackground => (spec.foreground, spec.background),
        CurlColors::CurrentGradient => (spec.foreground, spec.background),
        CurlColors::CurrentGradientReversed => (spec.background, spec.foreground),
    };
    map_px(src, out, ctx, |x, y, c| {
        // Corner-relative coordinates with the corner at the origin and the page toward +x, +y.
        let px = (x as f32 + 0.5 - corner.0) * sx;
        let py = (y as f32 + 0.5 - corner.1) * sy;
        // Signed distance past the fold line, toward the corner.
        let d = line_d - (px * nx + py * ny);
        if d <= 0.0 {
            if spec.shade && d > -radius {
                let k = 1.0 - 0.45 * (1.0 + d / radius);
                return [c[0] * k, c[1] * k, c[2] * k, c[3]];
            }
            return c;
        }
        if d >= roll {
            return if ctx.alpha { [0.0, 0.0, 0.0, 0.0] } else { [1.0, 1.0, 1.0, 1.0] };
        }
        let t = d / roll;
        let mut back = match spec.colors {
            CurlColors::ForegroundBackground => {
                if t < 0.5 {
                    c0
                } else {
                    c1
                }
            }
            _ => lerp4(c0, c1, t),
        };
        if spec.shade {
            let shade = 0.55 + 0.45 * (t * std::f32::consts::PI).sin();
            back = [back[0] * shade, back[1] * shade, back[2] * shade, back[3]];
        }
        back[3] = opacity;
        let under = if ctx.alpha { [0.0, 0.0, 0.0, 0.0] } else { [1.0, 1.0, 1.0, 1.0] };
        over(back, under)
    })
}
