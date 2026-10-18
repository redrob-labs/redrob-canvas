//! Light and shadow: Supernova, Gradient Flare, Sparkle, Long Shadow, Perspective Shadow,
//! Xach-Effect.

use redrob_photo_geom::Rect;

use super::{add_light, alpha_at, hsv_to_rgb, luma_rgb, map_px, over, rgb_to_hsv, rgba_at, sstep};
use crate::fxutil::gauss_blur_n;
use crate::image::Image;
use crate::noise::hash01;
use crate::{Ctx, FlareDesign, ShadowComposition, ShadowInterpolation, ShadowStyle, SparkleColor};

/// Supernova parameters.
pub(crate) struct SupernovaSpec {
    pub center: (f32, f32),
    pub radius: f32,
    pub spokes: u32,
    pub random_hue: f32,
    pub color: [f32; 4],
    pub seed: u32,
}

/// Supernova: a white-hot core of `radius` with a coloured glow and `spokes` rays of random
/// length, each ray's hue shifted by up to `random_hue` degrees.
pub(crate) fn supernova(src: &Image, out: Rect, ctx: &Ctx, spec: &SupernovaSpec) -> Vec<f32> {
    let b = ctx.bounds;
    let (cx, cy) = (b.x0 as f32 + spec.center.0 * b.width() as f32, b.y0 as f32 + spec.center.1 * b.height() as f32);
    let r = spec.radius.clamp(1.0, 3000.0);
    let spokes = spec.spokes.clamp(1, 1024) as i32;
    let base_hsv = rgb_to_hsv([spec.color[0], spec.color[1], spec.color[2]]);
    map_px(src, out, ctx, |x, y, c| {
        let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
        let d = (dx * dx + dy * dy).sqrt().max(0.5);
        let th = dy.atan2(dx).rem_euclid(std::f32::consts::TAU);
        let wedge = std::f32::consts::TAU / spokes as f32;
        let k = (th / wedge).floor() as i32;
        let frac = th / wedge - k as f32;
        // Each spoke has its own length and hue; blend with the neighbour across the wedge edge.
        let spoke = |k: i32| -> (f32, f32) {
            let k = k.rem_euclid(spokes);
            (0.2 + 0.8 * hash01(k, 0, 21, spec.seed), (hash01(k, 1, 22, spec.seed) - 0.5) * spec.random_hue / 360.0)
        };
        let (l0, h0) = spoke(k);
        let (l1, h1) = spoke(k + 1);
        let t = sstep(0.5, 1.0, frac) * 0.5 + (1.0 - sstep(0.0, 0.5, frac)) * 0.5;
        let (len, hue) = (l0 * (1.0 - t) + l1 * t, h0 * (1.0 - t) + h1 * t);
        let ray_profile = (1.0 - (frac - 0.5).abs() * 2.0).powi(2);
        let rays = ray_profile * len * (-d / (r * 6.0 * len)).exp() * 0.9;
        let glow = (r / d).powf(1.3).min(1.0) * 0.8;
        let core = sstep(r * 1.1, r * 0.5, d);
        let rgb = hsv_to_rgb([base_hsv[0] + hue, base_hsv[1], base_hsv[2].max(0.6)]);
        let light = [rgb[0] * (glow + rays) + core, rgb[1] * (glow + rays) + core, rgb[2] * (glow + rays) + core];
        add_light(c, light, 1.0)
    })
}

/// Gradient Flare parameters.
pub(crate) struct FlareSpec {
    pub center: (f32, f32),
    pub radius: f32,
    pub rotation: f32,
    pub hue_rotation: f32,
    pub vector_angle: f32,
    pub vector_length: f32,
    pub glow: bool,
    pub rays: bool,
    pub second_flares: bool,
    /// Built-in design (palette, ray count, ghost layout).
    pub design: FlareDesign,
    /// Rays around the centre (0 = the design's own count).
    pub ray_count: u32,
    /// Glow size as a percentage of `radius` (100 = the design's own).
    pub glow_size: f32,
    /// Second flares along the vector (0 = the design's own count).
    pub ghost_count: u32,
}

/// A second flare: position along the vector, radius factor, hue, saturation, ring?
type Ghost = (f32, f32, f32, f32, bool);

/// The fixed parts of a flare design: glow palette (core, mid, fringe as HSV hue/sat/value),
/// where the mid colour sits (0..1 of the glow), ray count, ray reach factor, ghosts.
struct FlareDesignSpec {
    core: [f32; 3],
    mid: [f32; 3],
    fringe: [f32; 3],
    mid_at: f32,
    rays: u32,
    ray_reach: f32,
    ray_hsv: [f32; 3],
    glow_scale: f32,
    ghosts: &'static [Ghost],
}

const CLASSIC_GHOSTS: [Ghost; 6] =
    [(0.25, 0.12, 0.6, 0.8, false), (0.45, 0.3, 0.33, 0.6, true), (0.6, 0.08, 0.1, 0.9, false), (0.8, 0.22, 0.75, 0.7, true), (1.0, 0.16, 0.5, 0.8, false), (1.2, 0.4, 0.15, 0.5, true)];
const STAR_GHOSTS: [Ghost; 2] = [(0.5, 0.1, 0.55, 0.5, false), (1.1, 0.14, 0.08, 0.6, true)];
const SUN_GHOSTS: [Ghost; 4] = [(0.35, 0.45, 0.08, 0.5, false), (0.7, 0.6, 0.05, 0.4, false), (1.0, 0.3, 0.12, 0.6, true), (1.4, 0.8, 0.1, 0.3, false)];
const RING_GHOSTS: [Ghost; 8] = [
    (0.2, 0.1, 0.6, 0.7, true),
    (0.35, 0.14, 0.65, 0.7, true),
    (0.5, 0.09, 0.7, 0.8, true),
    (0.65, 0.18, 0.55, 0.6, true),
    (0.8, 0.12, 0.75, 0.7, true),
    (0.95, 0.2, 0.6, 0.6, true),
    (1.1, 0.1, 0.8, 0.8, true),
    (1.3, 0.26, 0.62, 0.5, true),
];
const PLANET_GHOSTS: [Ghost; 5] = [(0.3, 0.2, 0.4, 0.9, false), (0.55, 0.35, 0.45, 0.7, true), (0.75, 0.5, 0.38, 0.8, false), (1.0, 0.25, 0.5, 0.9, true), (1.25, 0.6, 0.42, 0.6, false)];

fn design_spec(d: FlareDesign) -> FlareDesignSpec {
    match d {
        FlareDesign::Classic => FlareDesignSpec {
            core: [0.0, 0.0, 1.0],
            mid: [0.12, 0.9, 1.0],
            fringe: [0.0, 1.0, 0.8],
            mid_at: 0.3,
            rays: 9,
            ray_reach: 2.5,
            ray_hsv: [0.1, 0.5, 1.0],
            glow_scale: 1.0,
            ghosts: &CLASSIC_GHOSTS,
        },
        FlareDesign::BrightStar => FlareDesignSpec {
            core: [0.0, 0.0, 1.0],
            mid: [0.6, 0.15, 1.0],
            fringe: [0.6, 0.6, 0.6],
            mid_at: 0.5,
            rays: 6,
            ray_reach: 6.0,
            ray_hsv: [0.6, 0.1, 1.0],
            glow_scale: 0.6,
            ghosts: &STAR_GHOSTS,
        },
        FlareDesign::DistantSun => FlareDesignSpec {
            core: [0.1, 0.3, 1.0],
            mid: [0.08, 0.9, 1.0],
            fringe: [0.03, 1.0, 0.7],
            mid_at: 0.2,
            rays: 16,
            ray_reach: 1.6,
            ray_hsv: [0.08, 0.7, 0.9],
            glow_scale: 1.6,
            ghosts: &SUN_GHOSTS,
        },
        FlareDesign::ColdRings => FlareDesignSpec {
            core: [0.0, 0.0, 1.0],
            mid: [0.62, 0.7, 1.0],
            fringe: [0.75, 0.9, 0.7],
            mid_at: 0.35,
            rays: 12,
            ray_reach: 2.0,
            ray_hsv: [0.6, 0.4, 0.9],
            glow_scale: 0.9,
            ghosts: &RING_GHOSTS,
        },
        FlareDesign::HiddenPlanet => FlareDesignSpec {
            core: [0.45, 0.2, 1.0],
            mid: [0.42, 0.8, 0.9],
            fringe: [0.5, 1.0, 0.5],
            mid_at: 0.4,
            rays: 5,
            ray_reach: 1.2,
            ray_hsv: [0.45, 0.6, 0.8],
            glow_scale: 0.5,
            ghosts: &PLANET_GHOSTS,
        },
        FlareDesign::SoftGlow => FlareDesignSpec {
            core: [0.0, 0.0, 1.0],
            mid: [0.1, 0.2, 1.0],
            fringe: [0.1, 0.3, 0.8],
            mid_at: 0.5,
            rays: 0,
            ray_reach: 1.0,
            ray_hsv: [0.1, 0.2, 1.0],
            glow_scale: 1.4,
            ghosts: &[],
        },
    }
}

/// Gradient Flare: a glow with the design's radial gradient, rotating rays, and second flares
/// (coloured rings and discs) laid along a vector of `vector_length` pixels at `vector_angle`.
/// `hue_rotation` turns every colour; the ray count, glow size and ghost count can override the
/// design's own.
pub(crate) fn gradient_flare(src: &Image, out: Rect, ctx: &Ctx, spec: &FlareSpec) -> Vec<f32> {
    let b = ctx.bounds;
    let d = design_spec(spec.design);
    let (cx, cy) = (b.x0 as f32 + spec.center.0 * b.width() as f32, b.y0 as f32 + spec.center.1 * b.height() as f32);
    let r = (spec.radius.clamp(1.0, 3000.0) * d.glow_scale * (spec.glow_size / 100.0).clamp(0.05, 10.0)).max(1.0);
    let ray_r = spec.radius.clamp(1.0, 3000.0);
    let rot = spec.rotation.to_radians();
    let hue = spec.hue_rotation / 360.0;
    let (vs, vc) = spec.vector_angle.to_radians().sin_cos();
    let vlen = spec.vector_length.max(0.0);
    let tint = |h: f32, s: f32, v: f32| hsv_to_rgb([h + hue, s, v]);
    let core = tint(d.core[0], d.core[1], d.core[2]);
    let mid = tint(d.mid[0], d.mid[1], d.mid[2]);
    let fringe = tint(d.fringe[0], d.fringe[1], d.fringe[2]);
    let rays_n = if spec.ray_count > 0 { spec.ray_count.min(256) } else { d.rays } as f32;
    let ghosts: Vec<Ghost> = if spec.ghost_count > 0 {
        // More ghosts than the design has: continue the layout further along the vector.
        (0..spec.ghost_count.min(32) as usize)
            .map(|i| {
                let base = if d.ghosts.is_empty() { CLASSIC_GHOSTS[i % CLASSIC_GHOSTS.len()] } else { d.ghosts[i % d.ghosts.len()] };
                let cycle = (i / d.ghosts.len().max(1)) as f32;
                (base.0 + cycle * 0.25, base.1 * (1.0 + cycle * 0.3), base.2, base.3, base.4)
            })
            .collect()
    } else {
        d.ghosts.to_vec()
    };
    map_px(src, out, ctx, |x, y, c| {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let (dx, dy) = (fx - cx, fy - cy);
        let dist = (dx * dx + dy * dy).sqrt();
        let mut light = [0.0f32; 3];
        if spec.glow {
            let t = (dist / r).min(1.0);
            let col = if t < d.mid_at { lerp3(core, mid, t / d.mid_at.max(1e-3)) } else { lerp3(mid, fringe, (t - d.mid_at) / (1.0 - d.mid_at).max(1e-3)) };
            let k = (1.0 - t).powi(2) * 0.9 + 0.1 * (-dist / r).exp();
            for i in 0..3 {
                light[i] += col[i] * k;
            }
        }
        if spec.rays && rays_n > 0.0 {
            let th = dy.atan2(dx) + rot;
            let m = ((th * rays_n).sin() * 0.5 + 0.5).powi(6) * 0.7 + ((th * (rays_n * 2.5 + 0.5) + 0.7).sin() * 0.5 + 0.5).powi(14) * 0.5;
            let k = m * (-dist / (ray_r * d.ray_reach)).exp() * 0.8;
            let col = tint(d.ray_hsv[0], d.ray_hsv[1], d.ray_hsv[2]);
            for i in 0..3 {
                light[i] += col[i] * k;
            }
        }
        if spec.second_flares && vlen > 0.0 {
            for &(t, rf, h, s, ring) in &ghosts {
                let (gx, gy) = (cx + vc * vlen * t, cy + vs * vlen * t);
                let gr = (ray_r * rf).max(1.0);
                let gd = ((fx - gx).powi(2) + (fy - gy).powi(2)).sqrt() / gr;
                let k = if ring { sstep(0.6, 0.9, gd) * sstep(1.1, 0.95, gd) * 0.35 } else { sstep(1.0, 0.6, gd) * 0.3 };
                let col = tint(h, s, 1.0);
                for i in 0..3 {
                    light[i] += col[i] * k;
                }
            }
        }
        add_light(c, light, 1.0)
    })
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Sparkle parameters.
pub(crate) struct SparkleSpec {
    pub threshold: f32,
    pub flare_intensity: f32,
    pub spike_length: f32,
    pub spike_points: u32,
    pub spike_angle: f32,
    pub density: f32,
    pub transparency: f32,
    pub random_hue: f32,
    pub random_saturation: f32,
    pub preserve_luminosity: bool,
    pub inverse: bool,
    pub border: bool,
    pub color: SparkleColor,
    pub foreground: [f32; 4],
    pub background: [f32; 4],
    pub seed: u32,
}

/// How far a Sparkle reaches from its seed pixel.
pub(crate) fn sparkle_reach(spike_length: f32) -> f32 {
    spike_length.clamp(1.0, 100.0) + 2.0
}

/// Sparkle: pixels above the luminosity threshold (or below it, inverted) become stars with
/// `spike_points` spikes of up to `spike_length` pixels and a central flare. The star takes the
/// pixel's own colour (or the foreground/background), optionally with random hue and saturation.
pub(crate) fn sparkle(src: &Image, out: Rect, ctx: &Ctx, spec: &SparkleSpec) -> Vec<f32> {
    let b = ctx.bounds;
    let reach = sparkle_reach(spec.spike_length) as i32;
    let win = out.inflate(reach).intersect(&src.rect);
    let cut = (spec.threshold.clamp(0.0, 0.1) * 5.0).max(0.002);
    // Seeds: bright (or dark) pixels, thinned by density, plus the border when asked.
    let mut seeds: Vec<(i32, i32, f32, [f32; 4])> = Vec::new();
    for y in win.y0..win.y1 {
        for x in win.x0..win.x1 {
            let c = rgba_at(src, ctx, x, y);
            let l = luma_rgb(c) * c[3];
            let on_border = spec.border && (x == b.x0 || y == b.y0 || x == b.x1 - 1 || y == b.y1 - 1) && (x + y).rem_euclid(reach.max(3)) == 0;
            let bright = if spec.inverse { l <= cut } else { l >= 1.0 - cut };
            if (bright || on_border) && hash01(x, y, 31, spec.seed) <= spec.density.clamp(0.0, 1.0) {
                seeds.push((x, y, if on_border && !bright { 1.0 } else { l.max(0.05) }, c));
            }
        }
    }
    if seeds.len() > 6000 {
        let step = seeds.len() / 6000 + 1;
        seeds = seeds.into_iter().step_by(step).collect();
    }
    let points = spec.spike_points.min(16);
    let mut res = src.crop(out);
    let n = src.ch;
    let w = out.width() as usize;
    // Accumulate light (or darkness) per output pixel.
    let mut acc = vec![[0.0f32; 3]; w * out.height() as usize];
    for &(sx, sy, l, c) in &seeds {
        let len = spec.spike_length.clamp(1.0, 100.0) * if spec.preserve_luminosity { l } else { 0.5 + 0.5 * l };
        let base = match spec.color {
            SparkleColor::NaturalColor => c,
            SparkleColor::ForegroundColor => spec.foreground,
            SparkleColor::BackgroundColor => spec.background,
        };
        let mut hsv = rgb_to_hsv([base[0], base[1], base[2]]);
        hsv[0] += (hash01(sx, sy, 32, spec.seed) - 0.5) * spec.random_hue;
        hsv[1] = (hsv[1] + (hash01(sx, sy, 33, spec.seed) - 0.5) * spec.random_saturation).clamp(0.0, 1.0);
        hsv[2] = if spec.inverse { 0.0 } else { hsv[2].max(0.7) };
        let col = hsv_to_rgb(hsv);
        let a0 = (if spec.spike_angle < 0.0 { hash01(sx, sy, 34, spec.seed) * 360.0 } else { spec.spike_angle }).to_radians();
        let r = len.ceil() as i32 + 1;
        let region = Rect::new(sx - r, sy - r, sx + r + 1, sy + r + 1).intersect(&out);
        for y in region.y0..region.y1 {
            for x in region.x0..region.x1 {
                let (dx, dy) = ((x - sx) as f32, (y - sy) as f32);
                let d2 = dx * dx + dy * dy;
                let flare = spec.flare_intensity * (-d2 / (2.0 * (len * 0.15).max(0.5).powi(2))).exp();
                let mut spikes = 0.0;
                for k in 0..points {
                    let a = a0 + k as f32 * std::f32::consts::TAU / points.max(1) as f32;
                    let (s, co) = a.sin_cos();
                    let t = dx * co + dy * s;
                    if t < 0.0 {
                        continue;
                    }
                    let q = (dx * s - dy * co).abs();
                    let width = 0.45 + 0.03 * t;
                    spikes += (1.0 - t / len).max(0.0).powi(2) * (-(q * q) / (2.0 * width * width)).exp();
                }
                let i = (flare + spikes).min(1.0) * (1.0 - spec.transparency.clamp(0.0, 1.0));
                if i <= 0.0 {
                    continue;
                }
                let a = &mut acc[(y - out.y0) as usize * w + (x - out.x0) as usize];
                for ch in 0..3 {
                    a[ch] = if spec.inverse { a[ch].max(i) } else { 1.0 - (1.0 - a[ch]) * (1.0 - col[ch] * i) };
                }
            }
        }
    }
    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        let a = acc[i];
        if a == [0.0; 3] {
            continue;
        }
        let c = crate::fxutil::rgba(ctx, px);
        let o = if spec.inverse { [c[0] * (1.0 - a[0]), c[1] * (1.0 - a[1]), c[2] * (1.0 - a[2]), c[3]] } else { add_light(c, a, 1.0) };
        crate::fxutil::set_rgba(ctx, px, o);
    }
    res
}

/// Long Shadow parameters.
pub(crate) struct LongShadowSpec {
    pub style: ShadowStyle,
    pub angle: f32,
    pub length: f32,
    pub midpoint: f32,
    pub color: [f32; 4],
    pub composition: ShadowComposition,
}

/// Pixels a Long Shadow reads along its direction (`None` = the whole bounds).
pub(crate) fn long_shadow_reach(spec_style: ShadowStyle, length: f32, midpoint: f32) -> Option<f32> {
    match spec_style {
        ShadowStyle::Finite => Some(length.max(0.0) + 2.0),
        ShadowStyle::FadingFixedLength => Some(length.max(0.0) + 2.0),
        ShadowStyle::Fading => Some(midpoint.max(1.0) * 7.0 + 2.0),
        ShadowStyle::Infinite => None,
    }
}

/// Long Shadow: the layer's alpha is extruded along `angle` (degrees, clockwise from three
/// o'clock on screen). Finite stops after `length`; fading halves every `midpoint` pixels;
/// fading with fixed length holds full strength to `midpoint` then fades to zero at `length`;
/// infinite runs to the bounds. The shadow goes under the image, replaces it, or is cut by it.
pub(crate) fn long_shadow(src: &Image, out: Rect, ctx: &Ctx, spec: &LongShadowSpec) -> Vec<f32> {
    let b = ctx.bounds;
    let (s, c) = spec.angle.to_radians().sin_cos();
    let max_t = match spec.style {
        ShadowStyle::Infinite => (b.width() as f32).hypot(b.height() as f32) + 1.0,
        _ => long_shadow_reach(spec.style, spec.length, spec.midpoint).unwrap_or(0.0),
    };
    let steps = max_t.ceil().max(1.0) as i32;
    map_px(src, out, ctx, |x, y, me| {
        let mut shadow = 0.0f32;
        for i in 1..=steps {
            let t = i as f32;
            let wgt = match spec.style {
                ShadowStyle::Finite | ShadowStyle::Infinite => {
                    if spec.style == ShadowStyle::Finite && t > spec.length {
                        break;
                    }
                    1.0
                }
                ShadowStyle::Fading => 0.5f32.powf(t / spec.midpoint.max(1.0)),
                ShadowStyle::FadingFixedLength => {
                    if t > spec.length {
                        break;
                    }
                    if t <= spec.midpoint { 1.0 } else { 1.0 - (t - spec.midpoint) / (spec.length - spec.midpoint).max(1e-3) }
                }
            };
            if wgt < 0.003 {
                break;
            }
            let (sx, sy) = ((x as f32 + 0.5 - c * t).floor() as i32, (y as f32 + 0.5 - s * t).floor() as i32);
            if !b.contains(sx, sy) {
                if spec.style == ShadowStyle::Infinite {
                    break;
                }
                continue;
            }
            shadow = shadow.max(alpha_at(src, ctx, sx, sy) * wgt);
            if shadow >= 0.999 {
                break;
            }
        }
        let sh = [spec.color[0], spec.color[1], spec.color[2], shadow * spec.color[3]];
        match spec.composition {
            ShadowComposition::ShadowPlusImage => over(me, sh),
            ShadowComposition::ShadowOnly => sh,
            ShadowComposition::ShadowMinusImage => [sh[0], sh[1], sh[2], sh[3] * (1.0 - me[3])],
        }
    })
}

/// Perspective Shadow parameters.
pub(crate) struct PerspectiveShadowSpec {
    pub angle: f32,
    pub distance: f32,
    pub length: f32,
    pub blur: f32,
    pub color: [f32; 4],
    pub opacity: f32,
    pub interpolation: ShadowInterpolation,
}

/// Perspective Shadow: the layer's alpha is cast onto a ground plane through the bottom of its
/// content. `angle` (0..180) is the direction on the ground, `length` the shadow length relative
/// to the object height, and `distance` the viewer's distance (in content heights) that
/// foreshortens the far end. The shadow is blurred by `blur` and composited under the image.
pub(crate) fn perspective_shadow(src: &Image, out: Rect, ctx: &Ctx, spec: &PerspectiveShadowSpec) -> Vec<f32> {
    let b = ctx.bounds;
    // Content bottom: the lowest row with any alpha.
    let mut bottom = None;
    'rows: for y in (b.y0..b.y1).rev() {
        for x in b.x0..b.x1 {
            if alpha_at(src, ctx, x, y) > 0.01 {
                bottom = Some(y + 1);
                break 'rows;
            }
        }
    }
    let Some(base) = bottom else { return src.crop(out) };
    if !ctx.alpha {
        return src.crop(out);
    }
    let height = (base - b.y0).max(1) as f32;
    let a = spec.angle.clamp(0.0, 180.0).to_radians();
    let (sa, ca) = (a.sin().max(0.05), a.cos());
    let len = spec.length.clamp(0.1, 24.0);
    let dist = spec.distance.clamp(0.1, 24.0) * height;
    let sigma = spec.blur.max(0.0) / 2.0;
    let margin = (sigma * 3.0).ceil() as i32 + 1;
    let win = out.inflate(margin);
    let (ww, wh) = (win.width() as usize, win.height() as usize);
    let sample = |x: f32, y: f32| -> f32 {
        match spec.interpolation {
            ShadowInterpolation::None => alpha_at(src, ctx, x.floor() as i32, y.floor() as i32),
            ShadowInterpolation::Linear => {
                let (fx, fy) = (x - 0.5, y - 0.5);
                let (x0, y0) = (fx.floor(), fy.floor());
                let (tx, ty) = (fx - x0, fy - y0);
                let (x0, y0) = (x0 as i32, y0 as i32);
                let p = |dx: i32, dy: i32| alpha_at(src, ctx, x0 + dx, y0 + dy);
                (p(0, 0) * (1.0 - tx) + p(1, 0) * tx) * (1.0 - ty) + (p(0, 1) * (1.0 - tx) + p(1, 1) * tx) * ty
            }
            ShadowInterpolation::Cubic => {
                let (fx, fy) = (x - 0.5, y - 0.5);
                let (x0, y0) = (fx.floor(), fy.floor());
                let wx = crate::photo_util::catmull_rom(fx - x0);
                let wy = crate::photo_util::catmull_rom(fy - y0);
                let (x0, y0) = (x0 as i32, y0 as i32);
                let mut acc = 0.0;
                for (j, wyj) in wy.iter().enumerate() {
                    for (i, wxi) in wx.iter().enumerate() {
                        acc += alpha_at(src, ctx, x0 + i as i32 - 1, y0 + j as i32 - 1) * wxi * wyj;
                    }
                }
                acc.clamp(0.0, 1.0)
            }
        }
    };
    let mut shadow = vec![0.0f32; ww * wh];
    for (i, v) in shadow.iter_mut().enumerate() {
        let (x, y) = (win.x0 + (i % ww) as i32, win.y0 + (i / ww) as i32);
        let dy = y as f32 + 0.5 - base as f32;
        if dy <= 0.0 || dy >= dist {
            continue;
        }
        // Ground depth z projects to dy = z · dist / (dist + z).
        let z = dy * dist / (dist - dy);
        let hgt = z / (len * sa);
        if hgt > height {
            continue;
        }
        let sx = x as f32 + 0.5 - hgt * len * ca * dist / (dist + z);
        let sy = base as f32 - hgt;
        *v = sample(sx, sy);
    }
    if sigma >= 0.3 {
        gauss_blur_n(&mut shadow, ww, wh, 1, sigma);
    }
    let opacity = (spec.opacity / 100.0).clamp(0.0, 1.0);
    map_px(src, out, ctx, |x, y, me| {
        let sv = shadow[(y - win.y0) as usize * ww + (x - win.x0) as usize];
        over(me, [spec.color[0], spec.color[1], spec.color[2], sv * opacity])
    })
}

/// Xach-Effect parameters.
pub(crate) struct XachSpec {
    pub highlight_offset: (f32, f32),
    pub highlight_color: [f32; 4],
    pub highlight_opacity: f32,
    pub shadow_color: [f32; 4],
    pub shadow_opacity: f32,
    pub blur: f32,
    pub shadow_offset: (f32, f32),
}

/// Pixels the Xach-Effect reads around an output pixel.
pub(crate) fn xach_reach(spec_blur: f32, highlight: (f32, f32), shadow: (f32, f32)) -> f32 {
    let h = highlight.0.abs().max(highlight.1.abs());
    let s = shadow.0.abs().max(shadow.1.abs()) + spec_blur.max(0.0) * 1.5 + 1.0;
    h.max(s) + 2.0
}

/// Xach-Effect: a translucent cut-out look. A highlight (the layer's shape, shifted by the
/// highlight offset, where it leaves the shape uncovered) rims one side; a blurred drop shadow
/// falls behind the other side; the shape itself becomes slightly translucent.
pub(crate) fn xach(src: &Image, out: Rect, ctx: &Ctx, spec: &XachSpec) -> Vec<f32> {
    let sigma = spec.blur.max(0.0) / 2.0;
    let margin = (sigma * 3.0).ceil() as i32 + 1;
    let win = out.inflate(margin);
    let (ww, wh) = (win.width() as usize, win.height() as usize);
    let (sdx, sdy) = (spec.shadow_offset.0.round() as i32, spec.shadow_offset.1.round() as i32);
    let mut shadow: Vec<f32> = (0..ww * wh).map(|i| alpha_at(src, ctx, win.x0 + (i % ww) as i32 - sdx, win.y0 + (i / ww) as i32 - sdy)).collect();
    if sigma >= 0.3 {
        gauss_blur_n(&mut shadow, ww, wh, 1, sigma);
    }
    let (hdx, hdy) = (spec.highlight_offset.0.round() as i32, spec.highlight_offset.1.round() as i32);
    let ho = (spec.highlight_opacity / 255.0).clamp(0.0, 1.0);
    let so = (spec.shadow_opacity / 255.0).clamp(0.0, 1.0);
    map_px(src, out, ctx, |x, y, me| {
        let a = alpha_at(src, ctx, x, y);
        let shifted = alpha_at(src, ctx, x - hdx, y - hdy);
        let rim = (a - shifted).max(0.0);
        let sv = shadow[(y - win.y0) as usize * ww + (x - win.x0) as usize];
        let under = if ctx.alpha { [0.0, 0.0, 0.0, 0.0] } else { [1.0, 1.0, 1.0, 1.0] };
        let with_shadow = over([spec.shadow_color[0], spec.shadow_color[1], spec.shadow_color[2], sv * so], under);
        let body = if ctx.alpha { [me[0], me[1], me[2], me[3] * 0.85] } else { me };
        let composed = over(body, with_shadow);
        let hl = [spec.highlight_color[0], spec.highlight_color[1], spec.highlight_color[2], rim * ho];
        over(hl, composed)
    })
}
