// SPDX-License-Identifier: MIT OR Apache-2.0
//! Decor and web filters: Slide (film-slide frame with caption), Coffee Stain, Stencil Carve
//! and Semi-Flatten. The slide caption uses a built-in 5×7 pixel font.

use redrob_photo_geom::Rect;

use crate::Ctx;
use crate::fxutil::{rgba, set_rgba, xy};
use crate::gen_util::{Stream, mix, over, paint_over, value_noise};
use crate::image::{Edge, Image};

/// 5×7 glyphs: seven rows of five bits (MSB left) for `0-9`, `A-Z` and a few marks.
fn glyph(c: char) -> Option<[u8; 7]> {
    Some(match c.to_ascii_uppercase() {
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'D' => [0x1C, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1C],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x11, 0x0A, 0x04, 0x04, 0x04],
        'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        '-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C],
        '/' => [0x01, 0x01, 0x02, 0x04, 0x08, 0x10, 0x10],
        ':' => [0x00, 0x0C, 0x0C, 0x00, 0x0C, 0x0C, 0x00],
        ' ' => [0; 7],
        _ => return None,
    })
}

/// Coverage of the caption `text` drawn at `(ox, oy)` (top-left) with `size` px per font row, at
/// the pixel centred on `(x, y)`.
fn caption(text: &str, ox: f32, oy: f32, size: f32, x: f32, y: f32) -> f32 {
    let cell = (size / 7.0).max(0.5);
    let advance = cell * 6.0;
    let col = ((x - ox) / advance).floor();
    if col < 0.0 || y < oy || y >= oy + cell * 7.0 {
        return 0.0;
    }
    let Some(ch) = text.chars().nth(col as usize) else { return 0.0 };
    let Some(rows) = glyph(ch) else { return 0.0 };
    let gx = ((x - ox - col * advance) / cell).floor() as i32;
    let gy = ((y - oy) / cell).floor() as i32;
    if !(0..5).contains(&gx) || !(0..7).contains(&gy) {
        return 0.0;
    }
    if (rows[gy as usize] >> (4 - gx)) & 1 == 1 { 1.0 } else { 0.0 }
}

/// Slide: a film-slide frame (black bands with sprocket holes along the top and bottom, thin
/// borders at the sides), the caption top-left and the frame number bottom-right.
pub(crate) fn slide(src: &Image, out: Rect, ctx: &Ctx, text: &str, number: &str, font_size: f32, color: [f32; 4]) -> Vec<f32> {
    let b = ctx.bounds;
    let (w, h) = (b.width() as f32, b.height() as f32);
    let band = (h * 0.12).max(6.0);
    let side = (w * 0.03).max(3.0);
    let hole_w = band * 0.55;
    let hole_h = band * 0.42;
    let pitch = hole_w * 1.9;
    let size = font_size.clamp(4.0, band * 0.6);
    let text_y = b.y0 as f32 + (band - size) / 2.0;
    let number_x = b.x1 as f32 - side - (number.chars().count() as f32) * size / 7.0 * 6.0 - band * 0.3;
    let number_y = b.y1 as f32 - band + (band - size) / 2.0;
    paint_over(src, out, ctx, |x, y| {
        let (u, v) = (x - b.x0 as f32, y - b.y0 as f32);
        let in_band = v < band || v > h - band || u < side || u > w - side;
        if !in_band {
            return [0.0, 0.0, 0.0, 0.0];
        }
        // Sprocket holes punched out of the bands: lighter grey, rounded.
        let hole = {
            let row_v = if v < band { v } else { v - (h - band) };
            let cu = (u + pitch / 2.0).rem_euclid(pitch) - pitch / 2.0;
            let cv = row_v - band / 2.0;
            let inside = (cu.abs() - hole_w / 2.0 + hole_h / 4.0).max(0.0);
            let insv = (cv.abs() - hole_h / 2.0 + hole_h / 4.0).max(0.0);
            let d = (inside * inside + insv * insv).sqrt() - hole_h / 4.0;
            (v < band || v > h - band) && u > side * 2.0 && u < w - side * 2.0 && d < 0.0
        };
        let base = if hole { [0.85, 0.85, 0.85, 1.0] } else { [0.02, 0.02, 0.02, 1.0] };
        let t = caption(text, b.x0 as f32 + side + band * 0.3, text_y, size, x, y).max(caption(number, number_x, number_y, size, x, y));
        if t > 0.0 { [color[0], color[1], color[2], color[3]] } else { base }
    })
}

/// Coffee Stain: `stains` brown rings with ragged, noise-driven edges, dropped at random places
/// over the image. `darken_only` multiplies instead of painting over.
pub(crate) fn coffee_stain(src: &Image, out: Rect, ctx: &Ctx, stains: u32, darken_only: bool, seed: u32) -> Vec<f32> {
    let b = ctx.bounds;
    let (w, h) = (b.width() as f32, b.height() as f32);
    let mut rng = Stream::new(seed);
    let count = stains.clamp(1, 50) as usize;
    let rings: Vec<(f32, f32, f32, u32)> = (0..count)
        .map(|k| {
            (b.x0 as f32 + rng.range(0.1, 0.9) * w, b.y0 as f32 + rng.range(0.1, 0.9) * h, rng.range(0.12, 0.3) * w.min(h), seed.wrapping_add(k as u32 * 101))
        })
        .collect();
    let brown = [0.38, 0.22, 0.08];
    let n = src.ch;
    let mut res = src.crop(out);
    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        let (x, y) = xy(out, i);
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let mut a = 0.0f32;
        for &(cx, cy, r, s) in &rings {
            let (dx, dy) = (fx - cx, fy - cy);
            let d = (dx * dx + dy * dy).sqrt();
            if d > r * 1.4 {
                continue;
            }
            let ang = dy.atan2(dx);
            // Ragged rim: the radius wobbles with the angle.
            let wob = 1.0 + 0.18 * (value_noise(ang.cos() * 2.0 + 5.0, ang.sin() * 2.0 + 5.0, s) - 0.5) * 2.0;
            let rr = r * wob;
            let rim = (1.0 - ((d - rr).abs() / (rr * 0.08).max(1.0))).clamp(0.0, 1.0);
            let fill = if d < rr { 0.18 * (0.5 + 0.5 * value_noise(fx * 0.05, fy * 0.05, s ^ 0x99)) } else { 0.0 };
            a = a.max((rim * 0.8 + fill).min(1.0));
        }
        if a <= 0.0 {
            continue;
        }
        let under = rgba(ctx, px);
        let c = if darken_only {
            let m = |u: f32, s: f32| u * (1.0 - a) + u * s * a;
            [m(under[0], brown[0]), m(under[1], brown[1]), m(under[2], brown[2]), under[3].max(a)]
        } else {
            over([brown[0], brown[1], brown[2], a], under)
        };
        set_rgba(ctx, px, c);
    }
    res
}

/// Stencil Carve: the grey `map` (stretched over the bounds) is carved into the image: the
/// dark areas (or the white ones with `carve_white`) sink, with a shadow along their top-left
/// edge and a highlight along the bottom-right.
pub(crate) fn stencil_carve(src: &Image, out: Rect, ctx: &Ctx, carve_white: bool, map: Option<&Image>) -> Vec<f32> {
    let Some(map) = map else { return src.crop(out) };
    let b = ctx.bounds;
    let (w, h) = (b.width() as f32, b.height() as f32);
    let (mw, mh) = (map.rect.width() as f32, map.rect.height() as f32);
    let map_alpha = map.ch == 4 || map.ch == 2;
    let depth_at = |x: f32, y: f32| -> f32 {
        let u = ((x - b.x0 as f32) / w.max(1.0)).clamp(0.0, 0.9999);
        let v = ((y - b.y0 as f32) / h.max(1.0)).clamp(0.0, 0.9999);
        let mut tmp = [0.0f32; 8];
        map.sample(map.rect.x0 as f32 + u * mw, map.rect.y0 as f32 + v * mh, Edge::Repeat, map.rect, map_alpha, &mut tmp[..map.ch.min(8)]);
        let l = if map.ch >= 3 { 0.299 * tmp[0] + 0.587 * tmp[1] + 0.114 * tmp[2] } else { tmp[0] };
        if carve_white { l } else { 1.0 - l }
    };
    let n = src.ch;
    let mut res = src.crop(out);
    let step = (w.min(h) * 0.01).clamp(1.0, 6.0);
    for (i, px) in res.chunks_exact_mut(n).enumerate() {
        let (x, y) = xy(out, i);
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        let d = depth_at(fx, fy);
        if d <= 0.02 {
            // Outside the carving: a soft drop shadow cast by the carved rim below-right.
            let s = depth_at(fx - step, fy - step);
            if s > 0.5 {
                let under = rgba(ctx, px);
                set_rgba(ctx, px, over([0.0, 0.0, 0.0, 0.25 * s], under));
            }
            continue;
        }
        let shadow = depth_at(fx - step, fy - step);
        let light = depth_at(fx + step, fy + step);
        let under = rgba(ctx, px);
        // Sunken: darker overall, darkest where the top-left rim shades it, lit at the bottom-right rim.
        let mut c = [under[0] * 0.75, under[1] * 0.75, under[2] * 0.75, under[3]];
        let edge_shadow = (1.0 - shadow).clamp(0.0, 1.0) * d;
        let edge_light = (1.0 - light).clamp(0.0, 1.0) * d;
        c = over([0.0, 0.0, 0.0, 0.45 * edge_shadow], c);
        c = over([1.0, 1.0, 1.0, 0.35 * edge_light], c);
        set_rgba(ctx, px, mix(under, c, d.clamp(0.0, 1.0)));
    }
    res
}

/// Semi-Flatten: partially transparent pixels are blended with `background` and made opaque;
/// fully transparent pixels stay transparent, leaving a 1-bit alpha.
pub(crate) fn semi_flatten(src: &Image, out: Rect, ctx: &Ctx, background: [f32; 4]) -> Vec<f32> {
    let n = src.ch;
    let mut res = src.crop(out);
    if !ctx.alpha {
        return res;
    }
    for px in res.chunks_exact_mut(n) {
        let a = px[n - 1];
        if a <= 0.0 || a >= 1.0 {
            continue;
        }
        let c = rgba(ctx, px);
        let blended = [c[0] * a + background[0] * (1.0 - a), c[1] * a + background[1] * (1.0 - a), c[2] * a + background[2] * (1.0 - a), 1.0];
        set_rgba(ctx, px, blended);
    }
    res
}
