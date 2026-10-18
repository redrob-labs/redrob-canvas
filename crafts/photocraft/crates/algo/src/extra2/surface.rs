//! Relief of the fourth batch: Bump Map (an auxiliary height map lights the image) and Stencil
//! Chrome (the layer's own greyscale shape becomes polished chrome).

use redrob_photo_geom::Rect;

use super::super::extra::{hsv_to_rgb, lerp4, luma_rgb, map_px, rgb_to_hsv, rgba_at, sstep};
use super::{light_dir, mask_at};
use crate::fxutil::gauss_blur_n;
use crate::image::Image;
use crate::{BumpType, Ctx, MaskSource};

/// Bump Map parameters.
pub(crate) struct BumpMapSpec<'a> {
    pub kind: BumpType,
    pub compensate: bool,
    pub invert: bool,
    pub tiled: bool,
    pub azimuth: f32,
    pub elevation: f32,
    pub depth: f32,
    pub offset_x: i32,
    pub offset_y: i32,
    pub waterlevel: f32,
    pub ambient: f32,
    pub source: MaskSource,
    pub map: Option<&'a Image>,
}

/// Height (0..1) of the bump map at document pixel `(x, y)`.
fn bump_height(src: &Image, ctx: &Ctx, spec: &BumpMapSpec, b: Rect, x: i32, y: i32) -> f32 {
    let (mut x, mut y) = (x - spec.offset_x, y - spec.offset_y);
    if spec.tiled {
        x = b.x0 + (x - b.x0).rem_euclid(b.width().max(1) as i32);
        y = b.y0 + (y - b.y0).rem_euclid(b.height().max(1) as i32);
    }
    let raw = match spec.source {
        MaskSource::SelfImage => {
            let c = rgba_at(src, ctx, x, y);
            luma_rgb(c) * c[3]
        }
        source => {
            // A tiled external map wraps as well; otherwise the edge pixels repeat.
            let xx = if spec.tiled || b.contains(x, y) { x } else { x.clamp(b.x0, b.x1 - 1) };
            let yy = if spec.tiled || b.contains(x, y) { y } else { y.clamp(b.y0, b.y1 - 1) };
            mask_at(spec.map, source, b, xx, yy, 0.5)
        }
    };
    let h = if spec.invert { 1.0 - raw } else { raw }.clamp(0.0, 1.0);
    let h = match spec.kind {
        BumpType::Linear => h,
        BumpType::Spherical => (1.0 - (1.0 - h) * (1.0 - h)).max(0.0).sqrt(),
        BumpType::Sinusoidal => (1.0 - (h * std::f32::consts::PI).cos()) * 0.5,
    };
    // Below the water level the surface is flat.
    h.max(spec.waterlevel.clamp(0.0, 1.0))
}

/// Bump Map: the image lit as a relief whose height is the map (an external image, the layer
/// mask, or the layer's own luminance) through a linear, spherical or sinusoidal curve, from
/// `azimuth` / `elevation` with relief `depth`. `compensate` keeps the average brightness,
/// `waterlevel` flattens the lows, and `ambient` lifts the shadows.
pub(crate) fn bump_map(src: &Image, out: Rect, ctx: &Ctx, spec: &BumpMapSpec) -> Vec<f32> {
    let b = ctx.bounds;
    let light = light_dir(spec.azimuth, spec.elevation);
    let depth = spec.depth.clamp(0.0, 65.0);
    let ambient = spec.ambient.clamp(0.0, 1.0);
    // A flat surface's shade, so `compensate` can hold the image's average brightness.
    let flat = light[2].max(0.02);
    map_px(src, out, ctx, |x, y, me| {
        let h = |dx: i32, dy: i32| bump_height(src, ctx, spec, b, x + dx, y + dy);
        let nx = (h(-1, 0) - h(1, 0)) * depth;
        let ny = (h(0, -1) - h(0, 1)) * depth;
        let len = (nx * nx + ny * ny + 1.0).sqrt();
        let mut shade = ((nx * light[0] + ny * light[1] + light[2]) / len).max(0.0);
        if spec.compensate {
            shade /= flat;
        }
        let k = ambient + (1.0 - ambient) * shade;
        [(me[0] * k).clamp(0.0, 1.0), (me[1] * k).clamp(0.0, 1.0), (me[2] * k).clamp(0.0, 1.0), me[3]]
    })
}

/// Stencil Chrome parameters.
pub(crate) struct ChromeSpec<'a> {
    pub saturation: f32,
    pub lightness: f32,
    pub factor: f32,
    pub highlight_balance: f32,
    pub chrome_balance: f32,
    pub environment: Option<&'a Image>,
}

/// Pixels Stencil Chrome reads around an output pixel (the blur that rounds the stencil).
pub(crate) fn chrome_reach() -> f32 {
    12.0
}

/// Procedural environment: a bright sky fading to a dark horizon band over a lit ground.
fn sky(ny: f32) -> [f32; 3] {
    // ny: -1 looks straight up, +1 straight down (screen y grows downward).
    if ny < 0.0 {
        let t = (-ny).min(1.0);
        let horizon = [0.95, 0.9, 0.8];
        let zenith = [0.45, 0.6, 0.95];
        [horizon[0] + (zenith[0] - horizon[0]) * t, horizon[1] + (zenith[1] - horizon[1]) * t, horizon[2] + (zenith[2] - horizon[2]) * t]
    } else {
        let t = ny.min(1.0);
        let horizon = [0.2, 0.17, 0.14];
        let ground = [0.55, 0.5, 0.42];
        let band = 1.0 - sstep(0.0, 0.12, t);
        let base = [horizon[0] + (ground[0] - horizon[0]) * t, horizon[1] + (ground[1] - horizon[1]) * t, horizon[2] + (ground[2] - horizon[2]) * t];
        [base[0] * (1.0 - band * 0.6), base[1] * (1.0 - band * 0.6), base[2] * (1.0 - band * 0.6)]
    }
}

/// Stencil Chrome: the layer's greyscale shape (its luminance times alpha) is blurred into a
/// rounded relief whose normals reflect an environment (a supplied image, or a built-in sky and
/// ground), tinted by the chrome `saturation` / `lightness`, mixed in by `factor`. The highlight
/// balance sets the specular strength, the chrome balance the reflection contrast.
pub(crate) fn stencil_chrome(src: &Image, out: Rect, ctx: &Ctx, spec: &ChromeSpec) -> Vec<f32> {
    let reach = chrome_reach().ceil() as i32;
    let win = out.inflate(reach);
    let (ww, wh) = (win.width() as usize, win.height() as usize);
    let mut height: Vec<f32> = Vec::with_capacity(ww * wh);
    for y in win.y0..win.y1 {
        for x in win.x0..win.x1 {
            let c = rgba_at(src, ctx, x, y);
            height.push(luma_rgb(c) * if ctx.alpha { c[3] } else { 1.0 });
        }
    }
    gauss_blur_n(&mut height, ww, wh, 1, 3.0);
    let depth = 6.0;
    let light = light_dir(135.0, 45.0);
    let spec_k = (spec.highlight_balance.clamp(-100.0, 100.0) / 100.0 * 0.5 + 0.5) * 1.5;
    let contrast = 1.0 + spec.chrome_balance.clamp(-100.0, 100.0) / 100.0 * 0.8;
    let sat = spec.saturation.clamp(-100.0, 100.0) / 100.0;
    let lig = spec.lightness.clamp(-100.0, 100.0) / 100.0;
    let factor = spec.factor.clamp(0.0, 1.0);
    let env = |nx: f32, ny: f32| -> [f32; 3] {
        match spec.environment.filter(|m| !m.rect.is_empty()) {
            Some(m) => {
                // Sphere mapping: the reflected normal's x/y pick a point of the environment image.
                let r = m.rect;
                let u = r.x0 + (((nx * 0.5 + 0.5) * r.width() as f32).floor() as i32).clamp(0, r.width() as i32 - 1);
                let v = r.y0 + (((ny * 0.5 + 0.5) * r.height() as f32).floor() as i32).clamp(0, r.height() as i32 - 1);
                if m.ch >= 3 { [m.get(u, v, 0), m.get(u, v, 1), m.get(u, v, 2)] } else { [m.get(u, v, 0); 3] }
            }
            None => sky(ny),
        }
    };
    map_px(src, out, ctx, |x, y, me| {
        let (ix, iy) = ((x - win.x0) as i32, (y - win.y0) as i32);
        let h = |dx: i32, dy: i32| height[(iy + dy).clamp(0, wh as i32 - 1) as usize * ww + (ix + dx).clamp(0, ww as i32 - 1) as usize];
        let nx = (h(-1, 0) - h(1, 0)) * depth;
        let ny = (h(0, -1) - h(0, 1)) * depth;
        let len = (nx * nx + ny * ny + 1.0).sqrt();
        let n = [nx / len, ny / len, 1.0 / len];
        // Reflection of the viewing direction (0, 0, -1) about the normal.
        let rx = 2.0 * n[2] * n[0];
        let ry = 2.0 * n[2] * n[1];
        let mut refl = env(rx.clamp(-1.0, 1.0), ry.clamp(-1.0, 1.0));
        // Diffuse shading and a specular highlight.
        let diff = (n[0] * light[0] + n[1] * light[1] + n[2] * light[2]).max(0.0);
        let half = [light[0], light[1], light[2] + 1.0];
        let hl = (half[0] * half[0] + half[1] * half[1] + half[2] * half[2]).sqrt();
        let specular = ((n[0] * half[0] + n[1] * half[1] + n[2] * half[2]) / hl).max(0.0).powf(24.0) * spec_k;
        for c in &mut refl {
            *c = ((*c - 0.5) * contrast + 0.5) * (0.6 + 0.4 * diff) + specular;
        }
        let mut hsv = rgb_to_hsv([refl[0].clamp(0.0, 1.0), refl[1].clamp(0.0, 1.0), refl[2].clamp(0.0, 1.0)]);
        hsv[1] = (hsv[1] * (1.0 + sat)).clamp(0.0, 1.0);
        hsv[2] = (hsv[2] + lig * 0.5).clamp(0.0, 1.0);
        let rgb = hsv_to_rgb(hsv);
        let chrome = [rgb[0], rgb[1], rgb[2], me[3]];
        lerp4(me, chrome, factor)
    })
}
