//! The single-path scans of Image Trace: Edge Detection and Quantization. Each turns the image into
//! a black-and-white one (black where the scan found something), which the Black and White trace
//! then outlines as one layer of ink.

use crate::quantize::{TRANSPARENT, quantize};
use crate::{Mode, Raster, TraceParams};

fn luma(p: &[u8]) -> f64 {
    (f64::from(p[0]) * 0.299 + f64::from(p[1]) * 0.587 + f64::from(p[2]) * 0.114) * f64::from(p[3]) / 255.0
}

/// A white image with black where `img`'s brightness changes sharply: the Sobel gradient's
/// magnitude is above `threshold` (0..255; the Threshold slider, read as the edge strength).
pub(crate) fn edges(img: &Raster, threshold: u8) -> Raster {
    let (w, h) = (img.width as usize, img.height as usize);
    let px = |x: isize, y: isize| -> f64 {
        let (x, y) = (x.clamp(0, w as isize - 1) as usize, y.clamp(0, h as isize - 1) as usize);
        img.rgba.get((y * w + x) * 4..(y * w + x) * 4 + 4).map_or(0.0, luma)
    };
    let limit = f64::from(threshold).max(1.0) * 4.0;
    let mut out = vec![255u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let (xi, yi) = (x as isize, y as isize);
            let gx =
                (px(xi + 1, yi - 1) + 2.0 * px(xi + 1, yi) + px(xi + 1, yi + 1)) - (px(xi - 1, yi - 1) + 2.0 * px(xi - 1, yi) + px(xi - 1, yi + 1));
            let gy =
                (px(xi - 1, yi + 1) + 2.0 * px(xi, yi + 1) + px(xi + 1, yi + 1)) - (px(xi - 1, yi - 1) + 2.0 * px(xi, yi - 1) + px(xi + 1, yi - 1));
            if gx.hypot(gy) >= limit
                && let Some(p) = out.get_mut((y * w + x) * 4..(y * w + x) * 4 + 3)
            {
                p.fill(0);
            }
        }
    }
    Raster::new(img.width, img.height, out)
}

/// A white image with black where `img`, quantised to `params.colors` colours as Color mode
/// quantises it, falls into the darker half of its palette (by luminance): the boundaries of the
/// quantised result as one path.
pub(crate) fn quantized(img: &Raster, params: &TraceParams) -> Raster {
    let (w, h) = (img.width as usize, img.height as usize);
    let p = TraceParams { mode: Mode::Color, colors: params.colors.max(2), ..params.clone() };
    let q = quantize(img, &p);
    let light = |c: &[u8; 3]| f64::from(c[0]) * 0.299 + f64::from(c[1]) * 0.587 + f64::from(c[2]) * 0.114;
    let mut tones: Vec<f64> = q.palette.iter().map(light).collect();
    tones.sort_by(f64::total_cmp);
    let cut = tones.get(tones.len() / 2).copied().unwrap_or(128.0);
    let mut out = vec![255u8; w * h * 4];
    for (i, &l) in q.labels.iter().enumerate() {
        if l == TRANSPARENT {
            continue;
        }
        if q.palette.get(l as usize).is_some_and(|c| light(c) < cut)
            && let Some(p) = out.get_mut(i * 4..i * 4 + 3)
        {
            p.fill(0);
        }
    }
    Raster::new(img.width, img.height, out)
}
