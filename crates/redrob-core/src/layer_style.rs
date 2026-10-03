// SPDX-License-Identifier: GPL-3.0-or-later

//! Layer styles: drop shadow, outer glow and bevel, re-derived from Krita's layer-style feature and
//! the PSD layer-effects model (behaviour studied, no code copied). Each effect is computed from the
//! layer's own alpha silhouette and composited with the layer. We bake the result into the layer
//! (a "rasterize layer style"); keeping styles live as non-destructive metadata is a later pass.

use crate::{Pixel, Result};
use serde::{Deserialize, Serialize};

/// Parameters for the three bundled effects. Any subset can be enabled.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerStyle {
    #[serde(default)]
    pub drop_shadow: Option<DropShadow>,
    #[serde(default)]
    pub outer_glow: Option<OuterGlow>,
    #[serde(default)]
    pub bevel: Option<Bevel>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DropShadow {
    pub color: Pixel,
    pub offset_x: i32,
    pub offset_y: i32,
    pub blur: u32,
    pub opacity: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct OuterGlow {
    pub color: Pixel,
    pub blur: u32,
    pub opacity: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bevel {
    pub azimuth_degrees: f32,
    pub depth: f32,
    pub blur: u32,
}

/// Box-blur a single-channel (alpha) buffer, separable, `radius` each side.
fn blur_alpha(alpha: &[f32], width: usize, height: usize, radius: usize) -> Vec<f32> {
    if radius == 0 {
        return alpha.to_vec();
    }
    let mut horizontal = vec![0.0f32; width * height];
    let window = (radius * 2 + 1) as f32;
    for y in 0..height {
        for x in 0..width {
            let mut sum = 0.0;
            for k in 0..=(radius * 2) {
                let sx = (x as isize + k as isize - radius as isize).clamp(0, width as isize - 1)
                    as usize;
                sum += alpha[y * width + sx];
            }
            horizontal[y * width + x] = sum / window;
        }
    }
    let mut out = vec![0.0f32; width * height];
    for y in 0..height {
        for x in 0..width {
            let mut sum = 0.0;
            for k in 0..=(radius * 2) {
                let sy = (y as isize + k as isize - radius as isize).clamp(0, height as isize - 1)
                    as usize;
                sum += horizontal[sy * width + x];
            }
            out[y * width + x] = sum / window;
        }
    }
    out
}

/// Apply the enabled layer styles to `pixels` (RGBA8, w*h*4) in place, baking them into the layer.
/// Shadow and glow are composited UNDER the layer; bevel shading is composited OVER it.
pub fn apply_layer_style(
    pixels: &mut [u8],
    width: u32,
    height: u32,
    style: &LayerStyle,
) -> Result<()> {
    let w = width as usize;
    let h = height as usize;
    let n = w * h;
    // The layer's alpha silhouette, 0..1.
    let alpha: Vec<f32> = (0..n)
        .map(|i| f32::from(pixels[i * 4 + 3]) / 255.0)
        .collect();

    // Build the "under" layers (shadow, glow) then composite the original layer over them.
    let mut under = vec![0.0f32; n * 4]; // premultiplied straight-alpha accumulation as f32 RGBA

    let composite_src_over = |dst: &mut [f32], i: usize, sr: f32, sg: f32, sb: f32, sa: f32| {
        let da = dst[i * 4 + 3];
        let out_a = sa + da * (1.0 - sa);
        if out_a <= 0.0 {
            return;
        }
        for c in 0..3 {
            let s = [sr, sg, sb][c];
            let d = dst[i * 4 + c];
            dst[i * 4 + c] = (s * sa + d * da * (1.0 - sa)) / out_a;
        }
        dst[i * 4 + 3] = out_a;
    };

    if let Some(glow) = style.outer_glow {
        let blurred = blur_alpha(&alpha, w, h, glow.blur as usize);
        for (i, &blur) in blurred.iter().enumerate().take(n) {
            let a = blur * glow.opacity.clamp(0.0, 1.0);
            composite_src_over(
                &mut under,
                i,
                f32::from(glow.color.r) / 255.0,
                f32::from(glow.color.g) / 255.0,
                f32::from(glow.color.b) / 255.0,
                a,
            );
        }
    }
    if let Some(sh) = style.drop_shadow {
        // Offset the alpha, then blur.
        let mut shifted = vec![0.0f32; n];
        for y in 0..h {
            for x in 0..w {
                let sx = x as isize - sh.offset_x as isize;
                let sy = y as isize - sh.offset_y as isize;
                if sx >= 0 && sy >= 0 && (sx as usize) < w && (sy as usize) < h {
                    shifted[y * w + x] = alpha[sy as usize * w + sx as usize];
                }
            }
        }
        let blurred = blur_alpha(&shifted, w, h, sh.blur as usize);
        for (i, &blur) in blurred.iter().enumerate().take(n) {
            let a = blur * sh.opacity.clamp(0.0, 1.0);
            composite_src_over(
                &mut under,
                i,
                f32::from(sh.color.r) / 255.0,
                f32::from(sh.color.g) / 255.0,
                f32::from(sh.color.b) / 255.0,
                a,
            );
        }
    }

    // Composite the original layer over the shadow/glow.
    for i in 0..n {
        composite_src_over(
            &mut under,
            i,
            f32::from(pixels[i * 4]) / 255.0,
            f32::from(pixels[i * 4 + 1]) / 255.0,
            f32::from(pixels[i * 4 + 2]) / 255.0,
            alpha[i],
        );
    }

    // Bevel: shade the layer by the gradient of its (blurred) alpha, lit from the azimuth. Applied
    // only where the layer is opaque, over the composite.
    if let Some(bev) = style.bevel {
        let soft = blur_alpha(&alpha, w, h, bev.blur as usize);
        let az = f64::from(bev.azimuth_degrees).to_radians();
        let (lx, ly) = (az.cos(), az.sin());
        let depth = f64::from(bev.depth);
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if alpha[i] <= 0.0 {
                    continue;
                }
                let xm = x.saturating_sub(1);
                let xp = (x + 1).min(w - 1);
                let ym = y.saturating_sub(1);
                let yp = (y + 1).min(h - 1);
                let gx = f64::from(soft[y * w + xp] - soft[y * w + xm]) * depth;
                let gy = f64::from(soft[yp * w + x] - soft[ym * w + x]) * depth;
                // Lambert-ish term in [-1, 1]: positive = lit, negative = shadowed.
                let lit = (gx * lx + gy * ly).clamp(-1.0, 1.0);
                let factor = 1.0 + lit * 0.6;
                for c in 0..3 {
                    under[i * 4 + c] = (under[i * 4 + c] * factor as f32).clamp(0.0, 1.0);
                }
            }
        }
    }

    // Write back to 8-bit RGBA.
    for i in 0..n {
        for c in 0..4 {
            pixels[i * 4 + c] = (under[i * 4 + c] * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    Ok(())
}
