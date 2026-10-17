//! Decorations: Filmstrip, Fuzzy Border.

use redrob_photo_geom::Rect;

use super::{fbm, map_px, over, sstep};
use crate::Ctx;
use crate::image::{Edge, Image};

/// Filmstrip parameters (hole sizes as fractions of the film height).
pub(crate) struct FilmstripSpec {
    pub color: [f32; 4],
    pub number_color: [f32; 4],
    pub number_start: i32,
    pub numbers_top: bool,
    pub numbers_bottom: bool,
    pub hole_offset: f32,
    pub hole_width: f32,
    pub hole_height: f32,
    pub hole_spacing: f32,
}

/// 3×5 digit glyphs, one bit per cell, row-major from the top.
const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b001, 0b001, 0b001],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
];

/// Whether the glyph of `number`, drawn with its top-left at `origin` and `cell` pixels per
/// glyph cell, covers `(x, y)`.
fn digit_hit(number: i32, origin: (f32, f32), cell: f32, x: f32, y: f32) -> bool {
    let text = number.unsigned_abs().to_string();
    let (dx, dy) = (x - origin.0, y - origin.1);
    if dx < 0.0 || !(0.0..5.0 * cell).contains(&dy) {
        return false;
    }
    let col = (dx / cell).floor() as usize;
    let glyph = col / 4;
    let gc = col % 4;
    if gc == 3 || glyph >= text.len() {
        return false;
    }
    let d = text.as_bytes()[glyph].wrapping_sub(b'0') as usize;
    let row = (dy / cell).floor() as usize;
    DIGITS[d.min(9)][row.min(4)] & (0b100 >> gc) != 0
}

/// Filmstrip: the bounds become a strip of film of the film colour with two rows of sprocket
/// holes; the layer, scaled to the frame height, repeats as consecutive numbered frames across
/// the strip.
pub(crate) fn filmstrip(src: &Image, out: Rect, ctx: &Ctx, spec: &FilmstripSpec) -> Vec<f32> {
    let b = ctx.bounds;
    let (bw, bh) = (b.width() as f32, b.height() as f32);
    if bw < 1.0 || bh < 1.0 {
        return src.crop(out);
    }
    let hole_off = spec.hole_offset.clamp(0.0, 0.3) * bh;
    let hole_h = spec.hole_height.clamp(0.005, 0.3) * bh;
    let hole_w = spec.hole_width.clamp(0.005, 0.5) * bh;
    let hole_sp = spec.hole_spacing.clamp(0.01, 1.0) * bh;
    let margin = hole_off + hole_h + bh * 0.03;
    let frame_h = (bh - 2.0 * margin).max(1.0);
    let scale = frame_h / bh;
    let frame_w = bw * scale;
    let gap = bh * 0.04;
    let pitch = frame_w + gap;
    let film = [spec.color[0], spec.color[1], spec.color[2], 1.0];
    let glyph_cell = (hole_h * 0.18).max(1.0);
    let n = src.ch;
    let mut tmp = vec![0.0f32; n];
    map_px(src, out, ctx, |x, y, _| {
        let (fx, fy) = (x as f32 + 0.5 - b.x0 as f32, y as f32 + 0.5 - b.y0 as f32);
        // Sprocket holes.
        let in_hole_row = (hole_off..hole_off + hole_h).contains(&fy) || (bh - hole_off - hole_h..bh - hole_off).contains(&fy);
        if in_hole_row {
            let u = (fx + hole_w * 0.5).rem_euclid(hole_sp);
            if u < hole_w {
                return if ctx.alpha { [0.0, 0.0, 0.0, 0.0] } else { [1.0, 1.0, 1.0, 1.0] };
            }
        }
        // Frame numbers.
        let frame = (fx / pitch).floor() as i32;
        let number = spec.number_start + frame;
        let origin_x = frame as f32 * pitch + gap;
        if spec.numbers_top && digit_hit(number, (origin_x, hole_off + hole_h + bh * 0.005), glyph_cell, fx, fy) {
            return [spec.number_color[0], spec.number_color[1], spec.number_color[2], 1.0];
        }
        if spec.numbers_bottom && digit_hit(number, (origin_x, bh - margin + bh * 0.005), glyph_cell, fx, fy) {
            return [spec.number_color[0], spec.number_color[1], spec.number_color[2], 1.0];
        }
        // Frames.
        let u = fx.rem_euclid(pitch);
        if u >= gap && (margin..margin + frame_h).contains(&fy) {
            let sx = b.x0 as f32 + (u - gap) / scale;
            let sy = b.y0 as f32 + (fy - margin) / scale;
            src.sample(sx, sy, Edge::Repeat, b, ctx.alpha, &mut tmp);
            let c = crate::fxutil::rgba(ctx, &tmp);
            return over(c, film);
        }
        film
    })
}

/// Fuzzy Border parameters.
pub(crate) struct FuzzyBorderSpec {
    pub color: [f32; 4],
    pub size: f32,
    pub blur: bool,
    pub granularity: f32,
    pub shadow: bool,
    pub shadow_weight: f32,
    pub seed: u32,
}

/// Fuzzy Border: a ragged band of the border colour, about `size` pixels deep, around the edge
/// of the bounds. Granularity sets the size of the raggedness, blur softens its edge, and the
/// shadow darkens the image just inside the border.
pub(crate) fn fuzzy_border(src: &Image, out: Rect, ctx: &Ctx, spec: &FuzzyBorderSpec) -> Vec<f32> {
    let b = ctx.bounds;
    let size = spec.size.clamp(1.0, 300.0);
    let grain = spec.granularity.clamp(1.0, 16.0) * size * 0.15 + 1.0;
    let shadow = spec.shadow_weight.clamp(0.0, 100.0) / 100.0;
    map_px(src, out, ctx, |x, y, c| {
        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
        // Distance to the nearest edge of the bounds.
        let d = (fx - b.x0 as f32).min(b.x1 as f32 - fx).min(fy - b.y0 as f32).min(b.y1 as f32 - fy).max(0.0);
        let noise = fbm(fx / grain, fy / grain, spec.seed) - 0.5;
        let threshold = size * (0.65 + noise * 1.1);
        let soft = if spec.blur { size * 0.3 } else { 0.5 };
        let cover = sstep(threshold + soft * 0.5, threshold - soft * 0.5, d);
        let border = [spec.color[0], spec.color[1], spec.color[2], cover * spec.color[3]];
        let mut base = c;
        if spec.shadow {
            let k = 1.0 - shadow * 0.6 * sstep(threshold + size * 0.8, threshold, d) * (1.0 - cover);
            base = [c[0] * k, c[1] * k, c[2] * k, c[3]];
        }
        over(border, base)
    })
}
