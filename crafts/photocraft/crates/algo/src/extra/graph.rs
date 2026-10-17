//! Operation Graph: a text box of operations, one per line, applied in order as a single filter
//! (`blur radius=4`, `invert`, `brightness value=0.1`, …). Unknown lines are skipped.

use redrob_photo_geom::Rect;

use super::{hsv_to_rgb, luma_rgb, map_px, rgb_to_hsv, rgba_window, window_px};
use crate::Ctx;
use crate::fxutil::gauss_blur_n;
use crate::image::Image;
use crate::noise::hash01;

/// One parsed operation.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Op {
    Blur(f32),
    Sharpen(f32),
    Invert,
    Brightness(f32),
    Contrast(f32),
    Saturation(f32),
    Hue(f32),
    Gamma(f32),
    Threshold(f32),
    Grayscale,
    Sepia,
    Posterize(u32),
    Levels(f32, f32),
    Opacity(f32),
    Noise(f32, u32),
}

/// The operations named in a usual listing, for the dialog's help text.
pub const OPERATIONS: &str = "blur radius | sharpen amount | invert | brightness value | contrast value | saturation value | hue degrees | gamma value | threshold value | grayscale | sepia | posterize levels | levels low high | opacity value | noise amount seed";

fn num(args: &[(&str, &str)], key: &str, pos: usize, default: f32) -> f32 {
    args.iter()
        .find(|(k, _)| *k == key)
        .or_else(|| args.get(pos).filter(|(k, _)| k.is_empty()))
        .and_then(|(_, v)| v.parse::<f32>().ok())
        .filter(|v| v.is_finite())
        .unwrap_or(default)
}

/// Parses the graph source: one operation per line (or separated by `;`), `name key=value …`
/// (positional values are accepted too). Unknown names and malformed values are ignored.
pub(crate) fn parse(source: &str) -> Vec<Op> {
    let mut ops = Vec::new();
    for line in source.split(['\n', ';']) {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut words = line.split_whitespace();
        let Some(name) = words.next() else { continue };
        let name = name.trim_end_matches(':').to_ascii_lowercase();
        let name = name.rsplit(':').next().unwrap_or(name.as_str());
        let args: Vec<(&str, &str)> = words.map(|w| w.split_once('=').unwrap_or(("", w))).collect();
        let op = match name {
            "blur" | "gaussian-blur" => Op::Blur(num(&args, "radius", 0, 3.0).max(num(&args, "std-dev-x", 0, 0.0)).clamp(0.0, 300.0)),
            "sharpen" | "unsharp-mask" => Op::Sharpen(num(&args, "amount", 0, 0.5).clamp(0.0, 10.0)),
            "invert" | "invert-linear" | "invert-gamma" => Op::Invert,
            "brightness" => Op::Brightness(num(&args, "value", 0, 0.0).clamp(-1.0, 1.0)),
            "contrast" => Op::Contrast(num(&args, "value", 0, 1.0).clamp(0.0, 10.0)),
            "brightness-contrast" => {
                ops.push(Op::Brightness(num(&args, "brightness", 0, 0.0).clamp(-1.0, 1.0)));
                Op::Contrast((num(&args, "contrast", 1, 0.0) + 1.0).clamp(0.0, 10.0))
            }
            "saturation" => Op::Saturation(num(&args, "value", 0, 1.0).max(num(&args, "scale", 0, 0.0)).clamp(0.0, 10.0)),
            "hue" | "hue-rotate" => Op::Hue(num(&args, "degrees", 0, 0.0)),
            "gamma" => Op::Gamma(num(&args, "value", 0, 1.0).clamp(0.05, 20.0)),
            "threshold" => Op::Threshold(num(&args, "value", 0, 0.5).clamp(0.0, 1.0)),
            "grayscale" | "gray" | "desaturate" => Op::Grayscale,
            "sepia" => Op::Sepia,
            "posterize" => Op::Posterize(num(&args, "levels", 0, 4.0).clamp(2.0, 256.0) as u32),
            "levels" => Op::Levels(num(&args, "low", 0, 0.0).clamp(0.0, 1.0), num(&args, "high", 1, 1.0).clamp(0.0, 1.0)),
            "opacity" => Op::Opacity(num(&args, "value", 0, 1.0).clamp(0.0, 1.0)),
            "noise" | "noise-rgb" => Op::Noise(num(&args, "amount", 0, 0.1).clamp(0.0, 1.0), num(&args, "seed", 1, 0.0).max(0.0) as u32),
            _ => continue,
        };
        ops.push(op);
    }
    ops
}

/// Pixels the graph reads around an output pixel (the blurs and sharpens, in sequence).
pub(crate) fn reach(source: &str) -> f32 {
    parse(source)
        .iter()
        .map(|op| match op {
            Op::Blur(r) => super::gauss_reach(*r),
            Op::Sharpen(_) => 2.0,
            _ => 0.0,
        })
        .sum::<f32>()
}

fn point(op: &Op, c: [f32; 4], x: i32, y: i32) -> [f32; 4] {
    let mut o = c;
    match *op {
        Op::Invert => {
            for v in o.iter_mut().take(3) {
                *v = 1.0 - *v;
            }
        }
        Op::Brightness(b) => {
            for v in o.iter_mut().take(3) {
                *v = (*v + b).clamp(0.0, 1.0);
            }
        }
        Op::Contrast(k) => {
            for v in o.iter_mut().take(3) {
                *v = ((*v - 0.5) * k + 0.5).clamp(0.0, 1.0);
            }
        }
        Op::Saturation(s) => {
            let l = luma_rgb(c);
            for v in o.iter_mut().take(3) {
                *v = (l + (*v - l) * s).clamp(0.0, 1.0);
            }
        }
        Op::Hue(deg) => {
            let mut hsv = rgb_to_hsv([c[0], c[1], c[2]]);
            hsv[0] += deg / 360.0;
            let rgb = hsv_to_rgb(hsv);
            o[..3].copy_from_slice(&rgb);
        }
        Op::Gamma(g) => {
            for v in o.iter_mut().take(3) {
                *v = v.clamp(0.0, 1.0).powf(1.0 / g);
            }
        }
        Op::Threshold(t) => {
            let v = if luma_rgb(c) >= t { 1.0 } else { 0.0 };
            o[..3].copy_from_slice(&[v, v, v]);
        }
        Op::Grayscale => {
            let l = luma_rgb(c);
            o[..3].copy_from_slice(&[l, l, l]);
        }
        Op::Sepia => {
            let l = luma_rgb(c);
            o[..3].copy_from_slice(&[(l * 1.07 + 0.08).min(1.0), (l * 0.98 + 0.02).min(1.0), (l * 0.8).min(1.0)]);
        }
        Op::Posterize(n) => {
            let steps = (n.max(2) - 1) as f32;
            for v in o.iter_mut().take(3) {
                *v = (*v * steps).round() / steps;
            }
        }
        Op::Levels(lo, hi) => {
            let span = (hi - lo).abs().max(1e-4);
            for v in o.iter_mut().take(3) {
                *v = ((*v - lo.min(hi)) / span).clamp(0.0, 1.0);
            }
        }
        Op::Opacity(a) => o[3] = c[3] * a,
        Op::Noise(amount, seed) => {
            for (k, v) in o.iter_mut().enumerate().take(3) {
                *v = (*v + (hash01(x, y, k as u32, seed) - 0.5) * amount).clamp(0.0, 1.0);
            }
        }
        Op::Blur(_) | Op::Sharpen(_) => {}
    }
    o
}

/// Runs the graph over a window large enough for its blurs and returns the output pixels.
pub(crate) fn run(src: &Image, out: Rect, ctx: &Ctx, source: &str) -> Vec<f32> {
    let ops = parse(source);
    if ops.is_empty() {
        return src.crop(out);
    }
    let r = reach(source).ceil() as i32;
    let win = out.inflate(r);
    let (ww, wh) = (win.width() as usize, win.height() as usize);
    // Straight RGBA working buffer.
    let mut buf = rgba_window(src, ctx, win);
    for px in buf.chunks_exact_mut(4) {
        let a = px[3];
        if a > 1e-7 {
            for v in px.iter_mut().take(3) {
                *v /= a;
            }
        }
    }
    for op in &ops {
        match op {
            Op::Blur(radius) => {
                // Blur premultiplied, then back to straight colour.
                for px in buf.chunks_exact_mut(4) {
                    let a = px[3];
                    for v in px.iter_mut().take(3) {
                        *v *= a;
                    }
                }
                gauss_blur_n(&mut buf, ww, wh, 4, *radius);
                for px in buf.chunks_exact_mut(4) {
                    let a = px[3];
                    for v in px.iter_mut().take(3) {
                        *v = if a > 1e-7 { *v / a } else { 0.0 };
                    }
                }
            }
            Op::Sharpen(amount) => {
                let mut blurred = buf.clone();
                gauss_blur_n(&mut blurred, ww, wh, 4, 1.0);
                for (px, bl) in buf.chunks_exact_mut(4).zip(blurred.chunks_exact(4)) {
                    for k in 0..3 {
                        px[k] = (px[k] + (px[k] - bl[k]) * amount).clamp(0.0, 1.0);
                    }
                }
            }
            other => {
                for (i, px) in buf.chunks_exact_mut(4).enumerate() {
                    let (x, y) = (win.x0 + (i % ww) as i32, win.y0 + (i / ww) as i32);
                    let o = point(other, [px[0], px[1], px[2], px[3]], x, y);
                    px.copy_from_slice(&o);
                }
            }
        }
    }
    // `buf` is straight now; re-premultiply for the shared reader.
    for px in buf.chunks_exact_mut(4) {
        let a = px[3];
        for v in px.iter_mut().take(3) {
            *v *= a;
        }
    }
    map_px(src, out, ctx, |x, y, _| window_px(&buf, win, x, y))
}
