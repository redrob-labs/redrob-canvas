// SPDX-License-Identifier: MIT OR Apache-2.0
//! Map filters: Fractal Trace, Illusion, Little Planet, Recursive Transform, Tile Seamless, Map
//! Object and Small Tiles. All read the whole reference bounds ([`crate::Halo::Bounds`]) and
//! are centred on it.

use redrob_photo_geom::Rect;

use crate::fxutil::{native, xy};
use crate::gen_util::frame;
use crate::image::{Edge, Image};
use crate::{Ctx, FractalAbyss, FractalKind, IllusionType, MapShape, TileFlip, TileSelection};

/// Mixes native pixel `b` into `a` by `t`, in place.
#[inline]
fn lerp_px(a: &mut [f32], b: &[f32], t: f32) {
    for (x, y) in a.iter_mut().zip(b) {
        *x += (*y - *x) * t;
    }
}

/// Composites the straight native pixel `top` (alpha last when `alpha`) over `under`, in place.
fn over_px(under: &mut [f32], top: &[f32], alpha: bool) {
    let n = under.len();
    if !alpha {
        under.copy_from_slice(&top[..n]);
        return;
    }
    let a = top[n - 1].clamp(0.0, 1.0);
    let ua = under[n - 1];
    let out_a = a + ua * (1.0 - a);
    if out_a <= 0.0 {
        under.iter_mut().for_each(|v| *v = 0.0);
        return;
    }
    for c in 0..n - 1 {
        under[c] = (top[c] * a + under[c] * ua * (1.0 - a)) / out_a;
    }
    under[n - 1] = out_a;
}

/// Fractal Trace settings (the complex window the bounds map to, the Julia seed, iterations).
pub(crate) struct FractalTraceSpec {
    pub kind: FractalKind,
    pub x1: f32,
    pub x2: f32,
    pub y1: f32,
    pub y2: f32,
    pub jx: f32,
    pub jy: f32,
    pub depth: u32,
    pub bailout: f32,
    pub abyss: FractalAbyss,
}

/// Fractal Trace: each pixel's position is read as a complex number in the window, iterated
/// `depth` times through z² + c, and the result is read back as a source position.
pub(crate) fn fractal_trace(src: &Image, out: Rect, ctx: &Ctx, spec: &FractalTraceSpec) -> Vec<f32> {
    let n = src.ch;
    let b = ctx.bounds;
    let (w, h) = (b.width() as f32, b.height() as f32);
    let (sx, sy) = ((spec.x2 - spec.x1) / w.max(1.0), (spec.y2 - spec.y1) / h.max(1.0));
    let bail = spec.bailout.max(1.0) * spec.bailout.max(1.0);
    let mut res = Vec::with_capacity(out.width() as usize * out.height() as usize * n);
    let mut tmp = vec![0.0f32; n];
    let black = native(ctx, [0.0, 0.0, 0.0, 1.0]);
    let white = native(ctx, [1.0, 1.0, 1.0, 1.0]);
    for y in out.y0..out.y1 {
        for x in out.x0..out.x1 {
            let cx = spec.x1 + (x as f32 + 0.5 - b.x0 as f32) * sx;
            let cy = spec.y1 + (y as f32 + 0.5 - b.y0 as f32) * sy;
            let (mut zr, mut zi) = (cx, cy);
            let (cr, ci) = match spec.kind {
                FractalKind::Mandelbrot => (cx, cy),
                FractalKind::Julia => (spec.jx, spec.jy),
            };
            for _ in 0..spec.depth.clamp(1, 64) {
                let nr = zr * zr - zi * zi + cr;
                zi = 2.0 * zr * zi + ci;
                zr = nr;
                if zr * zr + zi * zi > bail {
                    break;
                }
            }
            if !zr.is_finite() || !zi.is_finite() {
                zr = cx;
                zi = cy;
            }
            let px = b.x0 as f32 + (zr - spec.x1) / sx.abs().max(1e-9) * sx.signum();
            let py = b.y0 as f32 + (zi - spec.y1) / sy.abs().max(1e-9) * sy.signum();
            let inside = px >= b.x0 as f32 && px < b.x1 as f32 && py >= b.y0 as f32 && py < b.y1 as f32;
            match spec.abyss {
                FractalAbyss::Loop => src.sample(px, py, Edge::Wrap, b, ctx.alpha, &mut tmp),
                FractalAbyss::Clamp => src.sample(px, py, Edge::Repeat, b, ctx.alpha, &mut tmp),
                _ if inside => src.sample(px, py, Edge::Repeat, b, ctx.alpha, &mut tmp),
                FractalAbyss::Black => tmp.copy_from_slice(&black[..n]),
                FractalAbyss::White => tmp.copy_from_slice(&white[..n]),
                FractalAbyss::Transparent => {
                    tmp.iter_mut().for_each(|v| *v = 0.0);
                    if !ctx.alpha {
                        tmp.copy_from_slice(&white[..n]);
                    }
                }
            }
            res.extend_from_slice(&tmp);
        }
    }
    res
}

/// Illusion: the image is overlaid with `division` copies of itself arranged around the centre,
/// each pulled toward its sector and shrunk with the radius, so the picture reads as a
/// kaleidoscopic fan. Type 2 turns the copies the other way round.
pub(crate) fn illusion(src: &Image, out: Rect, ctx: &Ctx, division: u32, kind: IllusionType) -> Vec<f32> {
    let n = src.ch;
    let (cx, cy, w, h) = frame(ctx);
    let rr = (w.min(h) / 2.0).max(1.0);
    let div = division.clamp(1, 64) as f32;
    let sector = std::f32::consts::TAU / div;
    let mut res = Vec::with_capacity(out.width() as usize * out.height() as usize * n);
    let mut a = vec![0.0f32; n];
    let mut c = vec![0.0f32; n];
    for y in out.y0..out.y1 {
        for x in out.x0..out.x1 {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let (dx, dy) = (fx - cx, fy - cy);
            let r = (dx * dx + dy * dy).sqrt();
            let ang = dy.atan2(dx);
            // Angle of this pixel's sector centre, and the offset within the sector.
            let k = (ang / sector).round();
            let centre_ang = k * sector;
            let off = ang - centre_ang;
            let rn = (r / rr).min(1.0);
            // The copy: pulled toward the sector centre and scaled by the radius.
            let (turn, scale) = match kind {
                IllusionType::Type1 => (-off * (1.0 - rn), 1.0 - 0.5 * rn),
                IllusionType::Type2 => (off * (1.0 - rn), 0.5 + 0.5 * rn),
            };
            let a2 = ang + turn;
            let r2 = r * scale;
            let (sx, sy) = (cx + r2 * a2.cos(), cy + r2 * a2.sin());
            src.sample(fx, fy, Edge::Repeat, ctx.bounds, ctx.alpha, &mut a);
            src.sample(sx, sy, Edge::Repeat, ctx.bounds, ctx.alpha, &mut c);
            // Strongest near the centre, fading out at the frame.
            let t = 0.5 * (1.0 - rn * rn);
            lerp_px(&mut a, &c, t);
            res.extend_from_slice(&a);
        }
    }
    res
}

/// Little Planet settings: pan / tilt / spin in degrees, zoom in % (100 = the planet's horizon
/// touches the shorter side), `inverse` looks up from inside instead (a tunnel).
pub(crate) struct PlanetSpec {
    pub pan: f32,
    pub tilt: f32,
    pub spin: f32,
    pub zoom: f32,
    pub inverse: bool,
}

/// Little Planet: a stereographic projection of an equirectangular panorama (the bounds), the
/// ground at the centre and the sky around the edge.
pub(crate) fn little_planet(src: &Image, out: Rect, ctx: &Ctx, spec: &PlanetSpec) -> Vec<f32> {
    let n = src.ch;
    let b = ctx.bounds;
    let (cx, cy, w, h) = frame(ctx);
    let k = 2.0 / (w.min(h).max(1.0)) * 100.0 / spec.zoom.max(1.0);
    let (pan, tilt, spin) = (spec.pan.to_radians(), spec.tilt.to_radians(), spec.spin.to_radians());
    let (st, ct) = tilt.sin_cos();
    let mut res = Vec::with_capacity(out.width() as usize * out.height() as usize * n);
    let mut tmp = vec![0.0f32; n];
    for y in out.y0..out.y1 {
        for x in out.x0..out.x1 {
            let u = (x as f32 + 0.5 - cx) * k;
            let v = (y as f32 + 0.5 - cy) * k;
            let r = (u * u + v * v).sqrt();
            let theta = v.atan2(u) + spin;
            // Inverse stereographic: the centre is the nadir (or the zenith when inverted).
            let mut lat = 2.0 * r.atan() - std::f32::consts::FRAC_PI_2;
            if spec.inverse {
                lat = -lat;
            }
            // Direction vector, tilted about the x axis.
            let (cl, sl) = (lat.cos(), lat.sin());
            let (dx, dy, dz) = (cl * theta.cos(), sl, cl * theta.sin());
            let (dy2, dz2) = (dy * ct - dz * st, dy * st + dz * ct);
            let lat2 = dy2.clamp(-1.0, 1.0).asin();
            let lon2 = dz2.atan2(dx) + pan;
            let sx = b.x0 as f32 + (lon2 / std::f32::consts::TAU + 0.5).rem_euclid(1.0) * w;
            let sy = b.y0 as f32 + (0.5 - lat2 / std::f32::consts::PI).clamp(0.0, 1.0) * h;
            src.sample(sx, sy, Edge::Wrap, b, ctx.alpha, &mut tmp);
            res.extend_from_slice(&tmp);
        }
    }
    res
}

/// Recursive Transform settings: one step is a scale (%), a rotation (degrees) and an offset
/// (fractions of the bounds) about the centre; iterations `first..=last` of it are pasted.
pub(crate) struct RecursiveSpec {
    pub scale: f32,
    pub angle: f32,
    pub offset_x: f32,
    pub offset_y: f32,
    pub first: u32,
    pub last: u32,
    pub fade: [f32; 4],
    pub paste_below: bool,
}

/// Recursive Transform: the image transformed by the step 1, 2, … `last` times and pasted on top
/// of itself (or below, so the original stays in front), each copy fading toward `fade`.
pub(crate) fn recursive_transform(src: &Image, out: Rect, ctx: &Ctx, spec: &RecursiveSpec) -> Vec<f32> {
    let n = src.ch;
    let (cx, cy, w, h) = frame(ctx);
    let s = (spec.scale / 100.0).clamp(0.01, 10.0);
    let a = spec.angle.to_radians();
    let (ox, oy) = (spec.offset_x * w, spec.offset_y * h);
    let last = spec.last.clamp(1, 32);
    let first = spec.first.min(last);
    let fade = native(ctx, spec.fade);
    let fade_a = spec.fade[3].clamp(0.0, 1.0);
    let mut res = Vec::with_capacity(out.width() as usize * out.height() as usize * n);
    let mut acc = vec![0.0f32; n];
    let mut tmp = vec![0.0f32; n];
    let (sa, ca) = a.sin_cos();
    for y in out.y0..out.y1 {
        for x in out.x0..out.x1 {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            src.sample(fx, fy, Edge::Transparent, ctx.bounds, ctx.alpha, &mut acc);
            let mut layers: Vec<Vec<f32>> = Vec::with_capacity(last as usize);
            // Inverse of the step applied i times gives where copy i reads from.
            let (mut px, mut py) = (fx, fy);
            for i in 1..=last {
                let (dx, dy) = (px - cx - ox, py - cy - oy);
                let (rx, ry) = ((dx * ca + dy * sa) / s, (-dx * sa + dy * ca) / s);
                px = cx + rx;
                py = cy + ry;
                if i < first {
                    continue;
                }
                src.sample(px, py, Edge::Transparent, ctx.bounds, ctx.alpha, &mut tmp);
                if fade_a > 0.0 {
                    let t = fade_a * i as f32 / last as f32;
                    let keep_a = if ctx.alpha { tmp[n - 1] } else { 1.0 };
                    lerp_px(&mut tmp, &fade[..n], t);
                    if ctx.alpha {
                        tmp[n - 1] = keep_a;
                    }
                }
                layers.push(tmp.clone());
            }
            if spec.paste_below {
                // Deepest copy at the back, the original in front.
                let mut base = layers.pop().unwrap_or_else(|| vec![0.0; n]);
                for l in layers.iter().rev() {
                    over_px(&mut base, l, ctx.alpha);
                }
                over_px(&mut base, &acc, ctx.alpha);
                res.extend_from_slice(&base);
            } else {
                for l in &layers {
                    over_px(&mut acc, l, ctx.alpha);
                }
                res.extend_from_slice(&acc);
            }
        }
    }
    res
}

/// Tile Seamless: blends the image with a copy shifted by half the bounds in both directions,
/// weighting each by its distance from its own centre, so opposite edges meet without a seam.
pub(crate) fn tile_seamless(src: &Image, out: Rect, ctx: &Ctx) -> Vec<f32> {
    let n = src.ch;
    let b = ctx.bounds;
    let (cx, cy, w, h) = frame(ctx);
    let mut res = Vec::with_capacity(out.width() as usize * out.height() as usize * n);
    let mut a = vec![0.0f32; n];
    let mut c = vec![0.0f32; n];
    for y in out.y0..out.y1 {
        for x in out.x0..out.x1 {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let u = ((fx - cx) / (w / 2.0).max(1.0)).abs().min(1.0);
            let v = ((fy - cy) / (h / 2.0).max(1.0)).abs().min(1.0);
            let w1 = (1.0 - u) * (1.0 - v);
            let w2 = u * v;
            src.sample(fx, fy, Edge::Wrap, b, ctx.alpha, &mut a);
            src.sample(fx + w / 2.0, fy + h / 2.0, Edge::Wrap, b, ctx.alpha, &mut c);
            let t = if w1 + w2 > 0.0 { w2 / (w1 + w2) } else { 0.5 };
            lerp_px(&mut a, &c, t);
            res.extend_from_slice(&a);
        }
    }
    res
}

/// Map Object settings. Rotations in degrees, light position in bounds units (the image spans
/// −0.5…0.5, the viewer sits at z = 2), material terms 0…1 (highlight is the shininess exponent).
pub(crate) struct MapObjectSpec {
    pub shape: MapShape,
    pub rot_x: f32,
    pub rot_y: f32,
    pub rot_z: f32,
    pub light_x: f32,
    pub light_y: f32,
    pub light_z: f32,
    pub ambient: f32,
    pub diffuse: f32,
    pub specular: f32,
    pub highlight: f32,
    pub radius: f32,
    pub transparent_background: bool,
}

type V3 = [f32; 3];

#[inline]
fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
#[inline]
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
#[inline]
fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
#[inline]
fn scale(a: V3, s: f32) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}
#[inline]
fn norm(a: V3) -> V3 {
    let l = dot(a, a).sqrt();
    if l > 0.0 { scale(a, 1.0 / l) } else { a }
}

/// Rotation about x, then y, then z (degrees) as a row-major matrix.
fn rotation(rx: f32, ry: f32, rz: f32) -> [V3; 3] {
    let (sx, cx) = rx.to_radians().sin_cos();
    let (sy, cy) = ry.to_radians().sin_cos();
    let (sz, cz) = rz.to_radians().sin_cos();
    // Rz * Ry * Rx
    [[cz * cy, cz * sy * sx - sz * cx, cz * sy * cx + sz * sx], [sz * cy, sz * sy * sx + cz * cx, sz * sy * cx - cz * sx], [-sy, cy * sx, cy * cx]]
}
#[inline]
fn apply(m: &[V3; 3], v: V3) -> V3 {
    [dot(m[0], v), dot(m[1], v), dot(m[2], v)]
}
#[inline]
fn transpose(m: &[V3; 3]) -> [V3; 3] {
    [[m[0][0], m[1][0], m[2][0]], [m[0][1], m[1][1], m[2][1]], [m[0][2], m[1][2], m[2][2]]]
}

/// Ray / object intersection in object space: distance, normal, texture coordinates (0…1).
fn hit(shape: MapShape, radius: f32, o: V3, d: V3) -> Option<(f32, V3, (f32, f32))> {
    let r = radius.clamp(0.05, 2.0);
    match shape {
        MapShape::Plane => {
            if d[2].abs() < 1e-6 {
                return None;
            }
            let t = -o[2] / d[2];
            if t <= 0.0 {
                return None;
            }
            let p = add(o, scale(d, t));
            if p[0].abs() > 0.5 || p[1].abs() > 0.5 {
                return None;
            }
            Some((t, [0.0, 0.0, if d[2] < 0.0 { 1.0 } else { -1.0 }], (p[0] + 0.5, 0.5 - p[1])))
        }
        MapShape::Sphere => {
            let b = dot(o, d);
            let c = dot(o, o) - r * r;
            let disc = b * b - c;
            if disc < 0.0 {
                return None;
            }
            let t = -b - disc.sqrt();
            if t <= 0.0 {
                return None;
            }
            let p = add(o, scale(d, t));
            let nrm = norm(p);
            let u = 0.5 + nrm[0].atan2(-nrm[2]) / std::f32::consts::TAU;
            let v = 0.5 - nrm[1].clamp(-1.0, 1.0).asin() / std::f32::consts::PI;
            Some((t, nrm, (u.rem_euclid(1.0), v)))
        }
        MapShape::Box => {
            // Slab test on the cube of half size r.
            let (mut tmin, mut tmax) = (f32::NEG_INFINITY, f32::INFINITY);
            let mut axis = 0;
            for i in 0..3 {
                let inv = if d[i].abs() < 1e-9 { 1e9 * d[i].signum().max(1.0) } else { 1.0 / d[i] };
                let (mut t0, mut t1) = ((-r - o[i]) * inv, (r - o[i]) * inv);
                if t0 > t1 {
                    std::mem::swap(&mut t0, &mut t1);
                }
                if t0 > tmin {
                    tmin = t0;
                    axis = i;
                }
                tmax = tmax.min(t1);
                if tmin > tmax {
                    return None;
                }
            }
            if tmin <= 0.0 {
                return None;
            }
            let p = add(o, scale(d, tmin));
            let mut nrm = [0.0; 3];
            nrm[axis] = p[axis].signum();
            let (u, v) = match axis {
                0 => (0.5 - p[2] / (2.0 * r) * nrm[0], 0.5 - p[1] / (2.0 * r)),
                1 => (0.5 + p[0] / (2.0 * r), 0.5 + p[2] / (2.0 * r) * nrm[1]),
                _ => (0.5 + p[0] / (2.0 * r) * nrm[2], 0.5 - p[1] / (2.0 * r)),
            };
            Some((tmin, nrm, (u.clamp(0.0, 1.0), v.clamp(0.0, 1.0))))
        }
        MapShape::Cylinder => {
            // Infinite cylinder about y, then clipped to the height 2r; caps are discs.
            let a = d[0] * d[0] + d[2] * d[2];
            let mut best: Option<(f32, V3, (f32, f32))> = None;
            if a > 1e-9 {
                let b = o[0] * d[0] + o[2] * d[2];
                let c = o[0] * o[0] + o[2] * o[2] - r * r;
                let disc = b * b - a * c;
                if disc >= 0.0 {
                    let t = (-b - disc.sqrt()) / a;
                    if t > 0.0 {
                        let p = add(o, scale(d, t));
                        if p[1].abs() <= r {
                            let nrm = norm([p[0], 0.0, p[2]]);
                            let u = 0.5 + p[0].atan2(-p[2]) / std::f32::consts::TAU;
                            let v = 0.5 - p[1] / (2.0 * r);
                            best = Some((t, nrm, (u.rem_euclid(1.0), v)));
                        }
                    }
                }
            }
            if d[1].abs() > 1e-6 {
                for sign in [1.0f32, -1.0] {
                    let t = (sign * r - o[1]) / d[1];
                    if t > 0.0 && best.is_none_or(|(bt, _, _)| t < bt) {
                        let p = add(o, scale(d, t));
                        if p[0] * p[0] + p[2] * p[2] <= r * r {
                            best = Some((t, [0.0, sign, 0.0], (0.5 + p[0] / (2.0 * r), 0.5 + p[2] / (2.0 * r))));
                        }
                    }
                }
            }
            best
        }
    }
}

/// Map Object: the bounds are a texture wrapped onto a plane, sphere, box or cylinder, lit by
/// one point light with ambient, diffuse and specular terms, seen from z = 2.
pub(crate) fn map_object(src: &Image, out: Rect, ctx: &Ctx, spec: &MapObjectSpec) -> Vec<f32> {
    let n = src.ch;
    let b = ctx.bounds;
    let (cx, cy, w, h) = frame(ctx);
    let m = rotation(spec.rot_x, spec.rot_y, spec.rot_z);
    let inv = transpose(&m);
    let eye: V3 = [0.0, 0.0, 2.0];
    let light: V3 = [spec.light_x, spec.light_y, spec.light_z];
    let mut res = Vec::with_capacity(out.width() as usize * out.height() as usize * n);
    let mut tmp = vec![0.0f32; n];
    let mut tex = vec![0.0f32; n];
    let subtractive = crate::fxutil::subtractive(ctx);
    for y in out.y0..out.y1 {
        for x in out.x0..out.x1 {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            // Image plane point in world units (the shorter side spans 1).
            let px = (fx - cx) / h.max(1.0);
            let py = -(fy - cy) / h.max(1.0);
            let target: V3 = [px, py, 0.0];
            let dir = norm(sub(target, eye));
            // Into object space.
            let o = apply(&inv, eye);
            let d = apply(&inv, dir);
            match hit(spec.shape, spec.radius, o, d) {
                Some((t, nrm_obj, (u, v))) => {
                    let p = add(eye, scale(dir, t));
                    let nrm = norm(apply(&m, nrm_obj));
                    let l = norm(sub(light, p));
                    let view = norm(sub(eye, p));
                    let diff = dot(nrm, l).max(0.0);
                    let refl = sub(scale(nrm, 2.0 * dot(nrm, l)), l);
                    let spec_term = dot(refl, view).max(0.0).powf(spec.highlight.max(1.0)) * spec.specular.clamp(0.0, 1.0);
                    let shade = (spec.ambient.clamp(0.0, 1.0) + spec.diffuse.clamp(0.0, 1.0) * diff).min(1.5);
                    let sx = b.x0 as f32 + u.clamp(0.0, 0.9999) * w;
                    let sy = b.y0 as f32 + v.clamp(0.0, 0.9999) * h;
                    src.sample(sx, sy, Edge::Repeat, b, ctx.alpha, &mut tex);
                    let cc = if ctx.alpha { n - 1 } else { n };
                    for c in 0..cc {
                        let lit = if subtractive { 1.0 - (1.0 - tex[c]) * shade } else { tex[c] * shade };
                        let v = if subtractive { lit - spec_term } else { lit + spec_term };
                        tex[c] = v.clamp(0.0, 1.0);
                    }
                    if ctx.alpha {
                        tex[n - 1] = 1.0;
                    }
                    res.extend_from_slice(&tex);
                }
                None => {
                    if spec.transparent_background && ctx.alpha {
                        tmp.iter_mut().for_each(|v| *v = 0.0);
                    } else {
                        src.sample(fx, fy, Edge::Repeat, b, ctx.alpha, &mut tmp);
                    }
                    res.extend_from_slice(&tmp);
                }
            }
        }
    }
    res
}

/// Small Tiles: the bounds shrunk into `segments` × `segments` copies; the selected copies are
/// flipped; `opacity` (%) mixes the tiled result over the original.
#[allow(clippy::too_many_arguments)]
pub(crate) fn small_tiles(src: &Image, out: Rect, ctx: &Ctx, segments: u32, flip: TileFlip, which: TileSelection, opacity: f32) -> Vec<f32> {
    let n = src.ch;
    let b = ctx.bounds;
    let (w, h) = (b.width() as f32, b.height() as f32);
    let seg = segments.clamp(1, 32) as f32;
    let t = (opacity / 100.0).clamp(0.0, 1.0);
    let mut res = Vec::with_capacity(out.width() as usize * out.height() as usize * n);
    let mut a = vec![0.0f32; n];
    let mut c = vec![0.0f32; n];
    for i in 0..out.width() as usize * out.height() as usize {
        let (x, y) = xy(out, i);
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let u = ((fx - b.x0 as f32) / w.max(1.0)).clamp(0.0, 0.9999) * seg;
        let v = ((fy - b.y0 as f32) / h.max(1.0)).clamp(0.0, 0.9999) * seg;
        let (ti, tj) = (u.floor() as i32, v.floor() as i32);
        let (mut lu, mut lv) = (u - u.floor(), v - v.floor());
        let flipped = match which {
            TileSelection::All => true,
            TileSelection::Alternate => (ti + tj) % 2 == 0,
            TileSelection::Explicit => ti == 0 && tj == 0,
        };
        if flipped {
            if matches!(flip, TileFlip::Horizontal | TileFlip::Both) {
                lu = 1.0 - lu;
            }
            if matches!(flip, TileFlip::Vertical | TileFlip::Both) {
                lv = 1.0 - lv;
            }
        }
        src.sample(b.x0 as f32 + lu * w, b.y0 as f32 + lv * h, Edge::Repeat, b, ctx.alpha, &mut c);
        src.sample(fx, fy, Edge::Repeat, b, ctx.alpha, &mut a);
        lerp_px(&mut a, &c, t);
        res.extend_from_slice(&a);
    }
    res
}
