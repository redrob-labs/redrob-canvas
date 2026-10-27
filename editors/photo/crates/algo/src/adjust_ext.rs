//! Colour Temperature and Hue-Chroma: per-pixel colour adjustments offered as filters under
//! Image > Adjustments (like Desaturate), evaluated on the sRGB conversion of the native pixel.

use redrob_photo_color::convert::{lab_to_srgb, linear_to_srgb, srgb_to_lab, srgb_to_linear};
use redrob_photo_geom::Rect;

use crate::Ctx;
use crate::fxutil::{rgba, set_rgba};
use crate::image::Image;

/// CIE 1931 xy chromaticity of a Planckian (black-body) radiator at `kelvin`, by the cubic
/// spline approximation of Kang et al. (2002), valid 1667–25000 K (clamped).
fn planckian_xy(kelvin: f32) -> (f32, f32) {
    let t = f64::from(kelvin.clamp(1667.0, 25000.0));
    let (t2, t3) = (t * t, t * t * t);
    let x = if t <= 4000.0 {
        -0.2661239e9 / t3 - 0.2343589e6 / t2 + 0.8776956e3 / t + 0.179_910
    } else {
        -3.0258469e9 / t3 + 2.1070379e6 / t2 + 0.2226347e3 / t + 0.240_390
    };
    let (x2, x3) = (x * x, x * x * x);
    let y = if t <= 2222.0 {
        -1.1063814 * x3 - 1.34811020 * x2 + 2.18555832 * x - 0.20219683
    } else if t <= 4000.0 {
        -0.9549476 * x3 - 1.37418593 * x2 + 2.09137015 * x - 0.16748867
    } else {
        3.0817580 * x3 - 5.87338670 * x2 + 3.75112997 * x - 0.37001483
    };
    (x as f32, y as f32)
}

/// Linear sRGB of the black-body white at `kelvin`, with luminance Y = 1.
pub fn kelvin_to_linear_rgb(kelvin: f32) -> [f32; 3] {
    let (x, y) = planckian_xy(kelvin);
    let y = y.max(1e-4);
    let xyz = [x / y, 1.0, (1.0 - x - y) / y];
    // XYZ (D65) → linear sRGB.
    [
        3.2406 * xyz[0] - 1.5372 * xyz[1] - 0.4986 * xyz[2],
        -0.9689 * xyz[0] + 1.8758 * xyz[1] + 0.0415 * xyz[2],
        0.0557 * xyz[0] - 0.2040 * xyz[1] + 1.0570 * xyz[2],
    ]
    .map(|v| v.max(1e-4))
}

/// Colour Temperature: re-renders the picture as if its light source of `original` K were one
/// of `intended` K, by scaling each linear-light channel by the ratio of the two black-body
/// whites (a von Kries adaptation in the sRGB primaries). A lower intended temperature warms the
/// picture, a higher one cools it. Clipped at 1.
pub(crate) fn color_temperature(src: &Image, out: Rect, ctx: &Ctx, original: f32, intended: f32) -> Vec<f32> {
    let n = src.ch;
    let from = kelvin_to_linear_rgb(original);
    let to = kelvin_to_linear_rgb(intended);
    let gain = [to[0] / from[0], to[1] / from[1], to[2] / from[2]];
    let mut res = src.crop(out);
    for px in res.chunks_exact_mut(n) {
        let c = rgba(ctx, px);
        let mut o = c;
        for i in 0..3 {
            o[i] = linear_to_srgb((srgb_to_linear(c[i].max(0.0)) * gain[i]).min(1.0));
        }
        set_rgba(ctx, px, o);
    }
    res
}

/// Hue-Chroma: rotates the hue by `hue` degrees and offsets the chroma and lightness (−100..100,
/// in CIE LCh(ab) units) of every pixel, keeping the other two components.
pub(crate) fn hue_chroma(src: &Image, out: Rect, ctx: &Ctx, hue: f32, chroma: f32, lightness: f32) -> Vec<f32> {
    let n = src.ch;
    let mut res = src.crop(out);
    if hue == 0.0 && chroma == 0.0 && lightness == 0.0 {
        return res;
    }
    let rot = hue.to_radians();
    for px in res.chunks_exact_mut(n) {
        let c = rgba(ctx, px);
        let lab = srgb_to_lab([c[0], c[1], c[2]]);
        let (l, a, b) = (lab[0], lab[1], lab[2]);
        let cr = (a * a + b * b).sqrt();
        let h = b.atan2(a) + rot;
        let cr2 = (cr + chroma).max(0.0);
        let l2 = (l + lightness).clamp(0.0, 100.0);
        let o = lab_to_srgb([l2, cr2 * h.cos(), cr2 * h.sin()]);
        set_rgba(ctx, px, [o[0], o[1], o[2], c[3]]);
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_body_whites_follow_the_locus() {
        // 6504 K is sRGB's D65: close to equal channels. Lower temperatures are redder.
        let d65 = kelvin_to_linear_rgb(6504.0);
        assert!((d65[0] - d65[2]).abs() < 0.08 && (d65[1] - 1.0).abs() < 0.05, "{d65:?}");
        let warm = kelvin_to_linear_rgb(3000.0);
        assert!(warm[0] > warm[1] && warm[1] > warm[2], "{warm:?}");
        let cool = kelvin_to_linear_rgb(12000.0);
        assert!(cool[2] > cool[0], "{cool:?}");
    }
}
