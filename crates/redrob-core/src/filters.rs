// SPDX-License-Identifier: GPL-3.0-or-later

use image::{ImageBuffer, Rgba};

use crate::{CoreError, Document, Filter, Result};

const MAX_FILTER_RADIUS: u32 = 4_096;

pub(crate) fn apply_filter(document: &mut Document, filter: &Filter) -> Result<()> {
    let width = document.width();
    let height = document.height();
    document.prepare_active_raster_edit()?;
    let original = document.active_raster_pixels()?.to_vec();
    let mut filtered = original.clone();

    match *filter {
        Filter::Invert => {
            for pixel in filtered.chunks_exact_mut(4) {
                pixel[0] = 255 - pixel[0];
                pixel[1] = 255 - pixel[1];
                pixel[2] = 255 - pixel[2];
            }
        }
        Filter::Grayscale => {
            for pixel in filtered.chunks_exact_mut(4) {
                let luminance = luminance(pixel);
                pixel[0..3].fill(luminance);
            }
        }
        Filter::BrightnessContrast {
            brightness,
            contrast,
        } => {
            if !(-255..=255).contains(&brightness)
                || !contrast.is_finite()
                || !(-100.0..=100.0).contains(&contrast)
            {
                return Err(CoreError::InvalidFilterParameter);
            }
            let contrast_255 = contrast * 2.55;
            let factor = (259.0 * (contrast_255 + 255.0)) / (255.0 * (259.0 - contrast_255));
            for pixel in filtered.chunks_exact_mut(4) {
                for channel in &mut pixel[0..3] {
                    *channel =
                        (factor * (f32::from(*channel) - 128.0) + 128.0 + f32::from(brightness))
                            .round()
                            .clamp(0.0, 255.0) as u8;
                }
            }
        }
        Filter::GaussianBlur { sigma } => {
            if !sigma.is_finite() || sigma <= 0.0 || sigma > 1_024.0 {
                return Err(CoreError::InvalidFilterParameter);
            }
            let premultiplied = premultiply(&original);
            let image: ImageBuffer<Rgba<u8>, Vec<u8>> =
                ImageBuffer::from_raw(width, height, premultiplied).ok_or_else(|| {
                    CoreError::MalformedProject("could not construct filter raster".into())
                })?;
            filtered = unpremultiply(image::imageops::blur(&image, sigma).into_raw());
        }
        Filter::Threshold { threshold } => {
            for pixel in filtered.chunks_exact_mut(4) {
                let value = if luminance(pixel) >= threshold {
                    255
                } else {
                    0
                };
                pixel[0..3].fill(value);
            }
        }
        Filter::Posterize { levels } => {
            if !(2..=256).contains(&levels) {
                return Err(CoreError::InvalidFilterParameter);
            }
            let last = u32::from(levels - 1);
            for pixel in filtered.chunks_exact_mut(4) {
                for channel in &mut pixel[0..3] {
                    let bucket = (u32::from(*channel) * last + 127) / 255;
                    *channel = ((bucket * 255 + last / 2) / last) as u8;
                }
            }
        }
        Filter::Curves { ref points } => {
            // The curve is rebuilt per application rather than cached. Measured: a 256-entry table from a
            // dozen control points is a tridiagonal solve of ten unknowns plus 256 evaluations, which is
            // nothing beside the per-pixel loop below, and a cache keyed on a point list would have to be
            // invalidated on every edit.
            let curve = crate::ToneCurve::new(points.clone())
                .map_err(|_| CoreError::InvalidFilterParameter)?;
            let table = curve.transfer_table_8bit();
            for pixel in filtered.chunks_exact_mut(4) {
                for channel in &mut pixel[0..3] {
                    *channel = table[usize::from(*channel)];
                }
            }
        }
        Filter::Levels {
            input_black,
            input_white,
            gamma,
            output_black,
            output_white,
        } => {
            if input_black >= input_white
                || output_black > output_white
                || !gamma.is_finite()
                || !(0.01..=100.0).contains(&gamma)
            {
                return Err(CoreError::InvalidFilterParameter);
            }
            let input_range = f32::from(input_white - input_black);
            let output_range = f32::from(output_white - output_black);
            for pixel in filtered.chunks_exact_mut(4) {
                for channel in &mut pixel[0..3] {
                    let normalized = ((f32::from(*channel) - f32::from(input_black)) / input_range)
                        .clamp(0.0, 1.0)
                        .powf(1.0 / gamma);
                    *channel = (f32::from(output_black) + normalized * output_range)
                        .round()
                        .clamp(0.0, 255.0) as u8;
                }
            }
        }
        Filter::HueSaturation {
            hue_degrees,
            saturation,
            lightness,
        } => {
            if !hue_degrees.is_finite()
                || !saturation.is_finite()
                || !lightness.is_finite()
                || !(-180.0..=180.0).contains(&hue_degrees)
                || !(-100.0..=100.0).contains(&saturation)
                || !(-100.0..=100.0).contains(&lightness)
            {
                return Err(CoreError::InvalidFilterParameter);
            }
            for pixel in filtered.chunks_exact_mut(4) {
                let (mut hue, mut sat, mut lit) = rgb_to_hsl(pixel[0], pixel[1], pixel[2]);
                hue = (hue + hue_degrees / 360.0).rem_euclid(1.0);
                let saturation_scale = saturation / 100.0;
                sat = if saturation_scale >= 0.0 {
                    sat + (1.0 - sat) * saturation_scale
                } else {
                    sat * (1.0 + saturation_scale)
                };
                let lightness_scale = lightness / 100.0;
                lit = if lightness_scale >= 0.0 {
                    lit + (1.0 - lit) * lightness_scale
                } else {
                    lit * (1.0 + lightness_scale)
                };
                let [red, green, blue] = hsl_to_rgb(hue, sat, lit);
                pixel[0] = red;
                pixel[1] = green;
                pixel[2] = blue;
            }
        }
        Filter::BoxBlur { radius } => {
            validate_radius(radius)?;
            filtered = box_blur_rgba(&original, width, height, radius);
        }
        Filter::Sharpen { amount } => {
            if !amount.is_finite() || !(0.0..=10.0).contains(&amount) {
                return Err(CoreError::InvalidFilterParameter);
            }
            if amount > 0.0 {
                let blurred = box_blur_rgba(&original, width, height, 1);
                for (output, blur) in filtered.chunks_exact_mut(4).zip(blurred.chunks_exact(4)) {
                    for channel in 0..3 {
                        output[channel] = (f32::from(output[channel])
                            + amount * (f32::from(output[channel]) - f32::from(blur[channel])))
                        .round()
                        .clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
        Filter::MotionBlur {
            angle_degrees,
            distance,
        } => {
            if !angle_degrees.is_finite() {
                return Err(CoreError::InvalidFilterParameter);
            }
            validate_radius(distance)?;
            let angle = f64::from(angle_degrees).to_radians();
            let (dx, dy) = (angle.cos(), angle.sin());
            let steps = distance as i64;
            for y in 0..height as i64 {
                for x in 0..width as i64 {
                    let (mut acc, mut n) = ([0.0f64; 4], 0.0f64);
                    // Sample the line centred on the pixel, from -distance/2 to +distance/2.
                    for s in -steps / 2..=steps / 2 {
                        let sx = (f64::from(x as i32) + dx * s as f64).round() as i64;
                        let sy = (f64::from(y as i32) + dy * s as f64).round() as i64;
                        if sx < 0 || sy < 0 || sx >= width as i64 || sy >= height as i64 {
                            continue;
                        }
                        let o = ((sy as usize) * width as usize + sx as usize) * 4;
                        for c in 0..4 {
                            acc[c] += f64::from(original[o + c]);
                        }
                        n += 1.0;
                    }
                    if n <= 0.0 {
                        continue;
                    }
                    let o = ((y as usize) * width as usize + x as usize) * 4;
                    for c in 0..4 {
                        filtered[o + c] = (acc[c] / n).round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
        Filter::LensBlur { radius } => {
            validate_radius(radius)?;
            let r = radius as i64;
            let r2 = (r * r) as f64;
            for y in 0..height as i64 {
                for x in 0..width as i64 {
                    let (mut acc, mut n) = ([0.0f64; 4], 0.0f64);
                    for oy in -r..=r {
                        for ox in -r..=r {
                            if (ox * ox + oy * oy) as f64 > r2 {
                                continue; // outside the disc
                            }
                            let sx = x + ox;
                            let sy = y + oy;
                            if sx < 0 || sy < 0 || sx >= width as i64 || sy >= height as i64 {
                                continue;
                            }
                            let o = ((sy as usize) * width as usize + sx as usize) * 4;
                            for c in 0..4 {
                                acc[c] += f64::from(original[o + c]);
                            }
                            n += 1.0;
                        }
                    }
                    if n <= 0.0 {
                        continue;
                    }
                    let o = ((y as usize) * width as usize + x as usize) * 4;
                    for c in 0..4 {
                        filtered[o + c] = (acc[c] / n).round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
        Filter::EdgeDetect { amount } => {
            if !amount.is_finite() || !(0.0..=10.0).contains(&amount) {
                return Err(CoreError::InvalidFilterParameter);
            }
            let w = width as i64;
            let h = height as i64;
            let lum = |x: i64, y: i64| -> f64 {
                let cx = x.clamp(0, w - 1) as usize;
                let cy = y.clamp(0, h - 1) as usize;
                let o = (cy * width as usize + cx) * 4;
                0.299 * f64::from(original[o])
                    + 0.587 * f64::from(original[o + 1])
                    + 0.114 * f64::from(original[o + 2])
            };
            for y in 0..h {
                for x in 0..w {
                    // Sobel gradients.
                    let gx = (lum(x + 1, y - 1) + 2.0 * lum(x + 1, y) + lum(x + 1, y + 1))
                        - (lum(x - 1, y - 1) + 2.0 * lum(x - 1, y) + lum(x - 1, y + 1));
                    let gy = (lum(x - 1, y + 1) + 2.0 * lum(x, y + 1) + lum(x + 1, y + 1))
                        - (lum(x - 1, y - 1) + 2.0 * lum(x, y - 1) + lum(x + 1, y - 1));
                    let mag = ((gx * gx + gy * gy).sqrt() * f64::from(amount))
                        .round()
                        .clamp(0.0, 255.0) as u8;
                    let o = (y as usize * width as usize + x as usize) * 4;
                    filtered[o] = mag;
                    filtered[o + 1] = mag;
                    filtered[o + 2] = mag;
                    // Alpha kept from the source.
                }
            }
        }
        Filter::Emboss { angle_degrees } => {
            if !angle_degrees.is_finite() {
                return Err(CoreError::InvalidFilterParameter);
            }
            let w = width as i64;
            let h = height as i64;
            let angle = f64::from(angle_degrees).to_radians();
            let (lx, ly) = (angle.cos(), angle.sin());
            let lum = |x: i64, y: i64| -> f64 {
                let cx = x.clamp(0, w - 1) as usize;
                let cy = y.clamp(0, h - 1) as usize;
                let o = (cy * width as usize + cx) * 4;
                0.299 * f64::from(original[o])
                    + 0.587 * f64::from(original[o + 1])
                    + 0.114 * f64::from(original[o + 2])
            };
            for y in 0..h {
                for x in 0..w {
                    // Surface gradient dotted with the light direction, biased to mid-grey.
                    let gx = lum(x + 1, y) - lum(x - 1, y);
                    let gy = lum(x, y + 1) - lum(x, y - 1);
                    let shade = (128.0 + (gx * lx + gy * ly)).round().clamp(0.0, 255.0) as u8;
                    let o = (y as usize * width as usize + x as usize) * 4;
                    filtered[o] = shade;
                    filtered[o + 1] = shade;
                    filtered[o + 2] = shade;
                }
            }
        }
        Filter::Laplace => {
            let w = width as i64;
            let h = height as i64;
            let at = |x: i64, y: i64, c: usize| -> f64 {
                let cx = x.clamp(0, w - 1) as usize;
                let cy = y.clamp(0, h - 1) as usize;
                f64::from(original[(cy * width as usize + cx) * 4 + c])
            };
            for y in 0..h {
                for x in 0..w {
                    let o = (y as usize * width as usize + x as usize) * 4;
                    for c in 0..3 {
                        // 3x3 Laplacian: 8*centre - 8 neighbours.
                        let lap = 8.0 * at(x, y, c)
                            - at(x - 1, y - 1, c)
                            - at(x, y - 1, c)
                            - at(x + 1, y - 1, c)
                            - at(x - 1, y, c)
                            - at(x + 1, y, c)
                            - at(x - 1, y + 1, c)
                            - at(x, y + 1, c)
                            - at(x + 1, y + 1, c);
                        filtered[o + c] = lap.abs().round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
        Filter::Pixelize { block } => {
            validate_radius(block)?;
            let b = block as usize;
            let w = width as usize;
            let h = height as usize;
            let mut by = 0;
            while by < h {
                let mut bx = 0;
                while bx < w {
                    let (mut acc, mut n) = ([0u64; 4], 0u64);
                    for y in by..(by + b).min(h) {
                        for x in bx..(bx + b).min(w) {
                            let o = (y * w + x) * 4;
                            for c in 0..4 {
                                acc[c] += u64::from(original[o + c]);
                            }
                            n += 1;
                        }
                    }
                    if n > 0 {
                        let avg = [
                            (acc[0] / n) as u8,
                            (acc[1] / n) as u8,
                            (acc[2] / n) as u8,
                            (acc[3] / n) as u8,
                        ];
                        for y in by..(by + b).min(h) {
                            for x in bx..(bx + b).min(w) {
                                let o = (y * w + x) * 4;
                                filtered[o..o + 4].copy_from_slice(&avg);
                            }
                        }
                    }
                    bx += b;
                }
                by += b;
            }
        }
        Filter::Waves {
            amplitude,
            wavelength,
        } => {
            if !amplitude.is_finite() || !wavelength.is_finite() || wavelength.abs() < 1e-3 {
                return Err(CoreError::InvalidFilterParameter);
            }
            let cx = f64::from(width) / 2.0;
            let cy = f64::from(height) / 2.0;
            let amp = f64::from(amplitude);
            let wl = f64::from(wavelength);
            for y in 0..height {
                for x in 0..width {
                    let dx = f64::from(x) + 0.5 - cx;
                    let dy = f64::from(y) + 0.5 - cy;
                    let dist = (dx * dx + dy * dy).sqrt();
                    // Displace radially by a sine of distance (concentric ripples from the centre).
                    let shift = amp * (dist / wl * std::f64::consts::TAU).sin();
                    let (nx, ny) = if dist > 1e-6 {
                        (
                            f64::from(x) + 0.5 + dx / dist * shift,
                            f64::from(y) + 0.5 + dy / dist * shift,
                        )
                    } else {
                        (f64::from(x) + 0.5, f64::from(y) + 0.5)
                    };
                    sample_bilinear(&original, width, height, nx - 0.5, ny - 0.5, x, y, &mut filtered);
                }
            }
        }
        Filter::Ripple {
            amplitude,
            wavelength,
            horizontal,
        } => {
            if !amplitude.is_finite() || !wavelength.is_finite() || wavelength.abs() < 1e-3 {
                return Err(CoreError::InvalidFilterParameter);
            }
            let amp = f64::from(amplitude);
            let wl = f64::from(wavelength);
            for y in 0..height {
                for x in 0..width {
                    let (nx, ny) = if horizontal {
                        // Shift x by a sine of y.
                        let s = amp * (f64::from(y) / wl * std::f64::consts::TAU).sin();
                        (f64::from(x) + 0.5 + s, f64::from(y) + 0.5)
                    } else {
                        let s = amp * (f64::from(x) / wl * std::f64::consts::TAU).sin();
                        (f64::from(x) + 0.5, f64::from(y) + 0.5 + s)
                    };
                    sample_bilinear(&original, width, height, nx - 0.5, ny - 0.5, x, y, &mut filtered);
                }
            }
        }
        Filter::WhirlPinch {
            whirl_degrees,
            pinch,
        } => {
            if !whirl_degrees.is_finite() || !pinch.is_finite() || !(-1.0..=1.0).contains(&pinch) {
                return Err(CoreError::InvalidFilterParameter);
            }
            let cx = f64::from(width) / 2.0;
            let cy = f64::from(height) / 2.0;
            let radius = cx.min(cy);
            let whirl = f64::from(whirl_degrees).to_radians();
            let pinch = f64::from(pinch);
            for y in 0..height {
                for x in 0..width {
                    let dx = f64::from(x) + 0.5 - cx;
                    let dy = f64::from(y) + 0.5 - cy;
                    let dist = (dx * dx + dy * dy).sqrt();
                    if dist >= radius || dist < 1e-6 {
                        let o = (y as usize * width as usize + x as usize) * 4;
                        filtered[o..o + 4].copy_from_slice(&original[o..o + 4]);
                        continue;
                    }
                    let factor = 1.0 - dist / radius; // 1 at centre, 0 at rim
                    let angle = whirl * factor * factor;
                    // Pinch: pull the source toward (positive) or away from the centre.
                    let scale = factor.powf(-pinch);
                    let (s, c) = angle.sin_cos();
                    let sx = cx + (dx * c - dy * s) * scale;
                    let sy = cy + (dx * s + dy * c) * scale;
                    sample_bilinear(&original, width, height, sx - 0.5, sy - 0.5, x, y, &mut filtered);
                }
            }
        }
        Filter::LensDistortion { main_amount } => {
            if !main_amount.is_finite() || !(-100.0..=100.0).contains(&main_amount) {
                return Err(CoreError::InvalidFilterParameter);
            }
            let cx = f64::from(width) / 2.0;
            let cy = f64::from(height) / 2.0;
            let norm = cx.hypot(cy);
            let k = f64::from(main_amount) / 100.0;
            for y in 0..height {
                for x in 0..width {
                    let dx = (f64::from(x) + 0.5 - cx) / norm;
                    let dy = (f64::from(y) + 0.5 - cy) / norm;
                    let r2 = dx * dx + dy * dy;
                    // Radial polynomial: barrel/pincushion by k*r^2.
                    let factor = 1.0 + k * r2;
                    let sx = cx + dx * norm * factor;
                    let sy = cy + dy * norm * factor;
                    sample_bilinear(&original, width, height, sx - 0.5, sy - 0.5, x, y, &mut filtered);
                }
            }
        }
    }

    blend_selection(document, &original, &mut filtered);
    document.replace_active_pixels(filtered)
}

/// Bilinear-sample `src` at `(fx, fy)` and write the result into `out` at destination pixel `(dx,
/// dy)`. Out-of-bounds reads clamp to the edge, so the warp filters do not tear at the borders.
#[allow(clippy::too_many_arguments)]
fn sample_bilinear(
    src: &[u8],
    width: u32,
    height: u32,
    fx: f64,
    fy: f64,
    dx: u32,
    dy: u32,
    out: &mut [u8],
) {
    let w = width as i64;
    let h = height as i64;
    let x0 = fx.floor() as i64;
    let y0 = fy.floor() as i64;
    let tx = fx - x0 as f64;
    let ty = fy - y0 as f64;
    let at = |x: i64, y: i64, c: usize| -> f64 {
        let cx = x.clamp(0, w - 1) as usize;
        let cy = y.clamp(0, h - 1) as usize;
        f64::from(src[(cy * width as usize + cx) * 4 + c])
    };
    let o = (dy as usize * width as usize + dx as usize) * 4;
    for c in 0..4 {
        let top = at(x0, y0, c) * (1.0 - tx) + at(x0 + 1, y0, c) * tx;
        let bottom = at(x0, y0 + 1, c) * (1.0 - tx) + at(x0 + 1, y0 + 1, c) * tx;
        out[o + c] = (top * (1.0 - ty) + bottom * ty).round().clamp(0.0, 255.0) as u8;
    }
}

fn validate_radius(radius: u32) -> Result<()> {
    if radius == 0 || radius > MAX_FILTER_RADIUS {
        Err(CoreError::InvalidFilterParameter)
    } else {
        Ok(())
    }
}

fn luminance(pixel: &[u8]) -> u8 {
    (0.2126 * f32::from(pixel[0]) + 0.7152 * f32::from(pixel[1]) + 0.0722 * f32::from(pixel[2]))
        .round()
        .clamp(0.0, 255.0) as u8
}

fn premultiply(input: &[u8]) -> Vec<u8> {
    let mut output = input.to_vec();
    for pixel in output.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        for channel in &mut pixel[0..3] {
            *channel = ((u16::from(*channel) * alpha + 127) / 255) as u8;
        }
    }
    output
}

fn unpremultiply(mut input: Vec<u8>) -> Vec<u8> {
    for pixel in input.chunks_exact_mut(4) {
        let alpha = u16::from(pixel[3]);
        if alpha == 0 {
            pixel.fill(0);
        } else {
            for channel in &mut pixel[0..3] {
                *channel = ((u16::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
    input
}

fn box_blur_rgba(input: &[u8], width: u32, height: u32, radius: u32) -> Vec<u8> {
    let premultiplied = premultiply(input);
    let horizontal = box_blur_rgba_pass(&premultiplied, width, height, radius, true);
    unpremultiply(box_blur_rgba_pass(
        &horizontal,
        width,
        height,
        radius,
        false,
    ))
}

fn box_blur_rgba_pass(
    input: &[u8],
    width: u32,
    height: u32,
    radius: u32,
    horizontal: bool,
) -> Vec<u8> {
    let mut output = vec![0; input.len()];
    let lines = if horizontal { height } else { width };
    let line_len = if horizontal { width } else { height };
    for line in 0..lines {
        let mut prefix = vec![[0_u64; 4]; line_len as usize + 1];
        for position in 0..line_len {
            let index = if horizontal {
                (line as usize * width as usize + position as usize) * 4
            } else {
                (position as usize * width as usize + line as usize) * 4
            };
            for channel in 0..4 {
                prefix[position as usize + 1][channel] =
                    prefix[position as usize][channel] + u64::from(input[index + channel]);
            }
        }
        for position in 0..line_len {
            let start = position.saturating_sub(radius);
            let end = position
                .saturating_add(radius)
                .saturating_add(1)
                .min(line_len);
            let count = u64::from(end - start);
            let index = if horizontal {
                (line as usize * width as usize + position as usize) * 4
            } else {
                (position as usize * width as usize + line as usize) * 4
            };
            for channel in 0..4 {
                let sum = prefix[end as usize][channel] - prefix[start as usize][channel];
                output[index + channel] = ((sum + count / 2) / count) as u8;
            }
        }
    }
    output
}

fn blend_selection(document: &Document, original: &[u8], filtered: &mut [u8]) {
    let width = document.width();
    for (index, (output, input)) in filtered
        .chunks_exact_mut(4)
        .zip(original.chunks_exact(4))
        .enumerate()
    {
        let x = index as u32 % width;
        let y = index as u32 / width;
        let amount = u16::from(document.selection().coverage(x, y));
        if amount != 255 {
            for channel in 0..4 {
                output[channel] = ((u16::from(output[channel]) * amount
                    + u16::from(input[channel]) * (255 - amount)
                    + 127)
                    / 255) as u8;
            }
        }
    }
}

fn rgb_to_hsl(red: u8, green: u8, blue: u8) -> (f32, f32, f32) {
    let red = f32::from(red) / 255.0;
    let green = f32::from(green) / 255.0;
    let blue = f32::from(blue) / 255.0;
    let max = red.max(green).max(blue);
    let min = red.min(green).min(blue);
    let lightness = (max + min) * 0.5;
    let delta = max - min;
    if delta <= f32::EPSILON {
        return (0.0, 0.0, lightness);
    }
    let saturation = delta / (1.0 - (2.0 * lightness - 1.0).abs());
    let hue_sector = if max == red {
        ((green - blue) / delta).rem_euclid(6.0)
    } else if max == green {
        (blue - red) / delta + 2.0
    } else {
        (red - green) / delta + 4.0
    };
    (hue_sector / 6.0, saturation, lightness)
}

fn hsl_to_rgb(hue: f32, saturation: f32, lightness: f32) -> [u8; 3] {
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let sector = hue * 6.0;
    let x = chroma * (1.0 - (sector.rem_euclid(2.0) - 1.0).abs());
    let (red, green, blue) = match sector.floor() as i32 {
        0 => (chroma, x, 0.0),
        1 => (x, chroma, 0.0),
        2 => (0.0, chroma, x),
        3 => (0.0, x, chroma),
        4 => (x, 0.0, chroma),
        _ => (chroma, 0.0, x),
    };
    let offset = lightness - chroma * 0.5;
    [red, green, blue].map(|channel| ((channel + offset) * 255.0).round().clamp(0.0, 255.0) as u8)
}
