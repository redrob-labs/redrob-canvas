// SPDX-License-Identifier: GPL-3.0-or-later

//! Camera raw import (H.16): sensor samples developed into an sRGB raster.
//!
//! The split here is deliberate. `rawloader` UNPACKS a vendor container — hundreds of camera-specific
//! layouts, lossless-JPEG variants and bit packings — into sensor samples plus the metadata needed to
//! develop them. That part is a catalogue of hardware quirks, not an algorithm, and re-writing it would
//! be transcription with no understanding to gain.
//!
//! The DEVELOPMENT is ours, and it is the actual work: black and white levels, white balance, demosaic,
//! and the camera's own colour matrix into sRGB. A raw file is not an image yet, and each of those
//! steps is a decision a raw converter makes.
//!
//! This is a faithful baseline, not a raw converter's full pipeline. There is no highlight recovery, no
//! noise reduction, no lens correction and no tone curve beyond the sRGB transfer. A camera JPEG will
//! look different — flatter, usually — and that is the honest result of not inventing a look the file
//! does not carry.

use crate::{FormatError, Result};

/// Develops a camera raw file into `(width, height, rgba)`.
pub(crate) fn decode_raw(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    let mut cursor = std::io::Cursor::new(bytes);
    let raw = rawloader::decode(&mut cursor)
        .map_err(|_| FormatError::Malformed("camera raw container"))?;

    // `cpp` above one means the file does not hold a colour-filter mosaic at all (a Foveon stack, or
    // an already-interpolated preview). Demosaicing it would be meaningless, so it is refused by name.
    if raw.cpp != 1 {
        return Err(FormatError::UnsupportedFeature(
            "this raw file is not a Bayer mosaic; its sensor needs its own development",
        )
        .into());
    }
    let samples = match &raw.data {
        rawloader::RawImageData::Integer(data) => data,
        // Float raws exist (some DNGs) and are a different normalisation; refused rather than read as
        // integers, which would come out as noise.
        rawloader::RawImageData::Float(_) => {
            return Err(FormatError::UnsupportedFeature(
                "this raw file stores floating-point samples",
            )
            .into());
        }
    };

    // `crops` is [top, right, bottom, left]: the masked border the sensor exposes but the image does
    // not include. Keeping it would put a black frame around every photo.
    let [crop_top, crop_right, crop_bottom, crop_left] = raw.crops;
    let full_w = raw.width;
    let full_h = raw.height;
    if full_w == 0 || full_h == 0 || samples.len() < full_w * full_h {
        return Err(FormatError::Malformed("camera raw sample count").into());
    }
    let out_w = full_w.saturating_sub(crop_left + crop_right);
    let out_h = full_h.saturating_sub(crop_top + crop_bottom);
    if out_w == 0
        || out_h == 0
        || out_w as u64 > u64::from(crate::document::MAX_DIMENSION)
        || out_h as u64 > u64::from(crate::document::MAX_DIMENSION)
    {
        return Err(FormatError::Malformed("camera raw dimensions out of range").into());
    }

    // White balance as the CAMERA recorded it, normalised against green. Green is the reference because
    // a Bayer sensor has twice as many green sites, so leaving the coefficients unnormalised changes
    // overall exposure as well as colour. A file with no coefficients is left unbalanced rather than
    // given invented ones -- a wrong white balance is worse than a neutral one.
    let green = if raw.wb_coeffs[1].is_finite() && raw.wb_coeffs[1] > 0.0 {
        raw.wb_coeffs[1]
    } else {
        1.0
    };
    let balance = |channel: usize| -> f32 {
        let coefficient = raw.wb_coeffs[channel.min(3)];
        if coefficient.is_finite() && coefficient > 0.0 {
            coefficient / green
        } else {
            1.0
        }
    };

    // Normalise one sample to 0..=1 within its own channel's levels. Black and white levels are PER
    // CHANNEL: using one pair for all of them tints the shadows, which is the most visible way to get
    // this wrong.
    let normalise = |value: u16, channel: usize| -> f32 {
        let black = f32::from(raw.blacklevels[channel.min(3)]);
        let white = f32::from(raw.whitelevels[channel.min(3)]);
        if white <= black {
            return 0.0;
        }
        ((f32::from(value) - black) / (white - black)).clamp(0.0, 1.0)
    };

    // Demosaic: bilinear over the mosaic, which is the baseline every raw converter starts from. For a
    // missing colour at a site, average the neighbours that DO carry it, choosing them by the CFA
    // pattern rather than by a fixed offset -- the pattern's phase differs per camera, and a hard-coded
    // RGGB swaps red and blue on half of them.
    let mut linear = vec![[0f32; 3]; out_w * out_h];
    for y in 0..out_h {
        for x in 0..out_w {
            let sy = y + crop_top;
            let sx = x + crop_left;
            let mut sums = [0f32; 3];
            let mut counts = [0u32; 3];
            // The 3x3 neighbourhood, including the centre: every channel present in it contributes.
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let ny = sy as i32 + dy;
                    let nx = sx as i32 + dx;
                    if ny < 0 || nx < 0 || ny as usize >= full_h || nx as usize >= full_w {
                        continue;
                    }
                    let channel = raw.cfa.color_at(ny as usize, nx as usize).min(2);
                    let value = samples[ny as usize * full_w + nx as usize];
                    sums[channel] += normalise(value, channel);
                    counts[channel] += 1;
                }
            }
            let mut pixel = [0f32; 3];
            for channel in 0..3 {
                if counts[channel] > 0 {
                    pixel[channel] = sums[channel] / counts[channel] as f32 * balance(channel);
                }
            }
            linear[y * out_w + x] = pixel;
        }
    }

    // Camera RGB to sRGB. The file gives XYZ-to-camera, so it is INVERTED to get camera-to-XYZ and then
    // composed with XYZ-to-sRGB from the colour module. Skipping this and treating camera RGB as sRGB
    // is the single biggest error available here: every camera's primaries differ, and the result is a
    // visible colour cast rather than a subtle one.
    let cam_to_xyz = invert3(&[raw.xyz_to_cam[0], raw.xyz_to_cam[1], raw.xyz_to_cam[2]]);

    let mut rgba = Vec::with_capacity(out_w * out_h * 4);
    for pixel in &linear {
        let (r, g, b) = match &cam_to_xyz {
            Some(matrix) => {
                let (x, y, z) = apply3(matrix, pixel);
                crate::color::xyz_to_linear_srgb(f64::from(x), f64::from(y), f64::from(z))
            }
            // A file with a singular or absent matrix keeps its own channels. Reported by being
            // visibly uncorrected rather than by a guess at primaries the file never stated.
            None => (
                f64::from(pixel[0]),
                f64::from(pixel[1]),
                f64::from(pixel[2]),
            ),
        };
        for channel in [r, g, b] {
            let encoded = crate::color::linear_to_srgb(channel.clamp(0.0, 1.0));
            rgba.push((encoded * 255.0).round().clamp(0.0, 255.0) as u8);
        }
        rgba.push(255);
    }

    Ok((out_w as u32, out_h as u32, rgba))
}

/// Inverts a 3x3 matrix, or `None` when it is singular.
///
/// Written out rather than pulled from a linear-algebra crate because it is three rows: the dependency
/// would be larger than the function, and a singular matrix has to be a case we HANDLE rather than a
/// panic, since it comes from a file we do not control.
fn invert3(matrix: &[[f32; 3]; 3]) -> Option<[[f32; 3]; 3]> {
    let m = matrix;
    let determinant = f64::from(m[0][0])
        * (f64::from(m[1][1]) * f64::from(m[2][2]) - f64::from(m[1][2]) * f64::from(m[2][1]))
        - f64::from(m[0][1])
            * (f64::from(m[1][0]) * f64::from(m[2][2]) - f64::from(m[1][2]) * f64::from(m[2][0]))
        + f64::from(m[0][2])
            * (f64::from(m[1][0]) * f64::from(m[2][1]) - f64::from(m[1][1]) * f64::from(m[2][0]));
    if !determinant.is_finite() || determinant.abs() < 1e-9 {
        return None;
    }
    let at = |r: usize, c: usize| f64::from(m[r][c]);
    let cofactor = |r0: usize, r1: usize, c0: usize, c1: usize| {
        at(r0, c0) * at(r1, c1) - at(r0, c1) * at(r1, c0)
    };
    let mut out = [[0f32; 3]; 3];
    for (row, out_row) in out.iter_mut().enumerate() {
        for (col, out_cell) in out_row.iter_mut().enumerate() {
            // Transposed cofactor, divided by the determinant: the adjugate form.
            let (r0, r1) = ((col + 1) % 3, (col + 2) % 3);
            let (c0, c1) = ((row + 1) % 3, (row + 2) % 3);
            *out_cell = (cofactor(r0, r1, c0, c1) / determinant) as f32;
        }
    }
    Some(out)
}

fn apply3(matrix: &[[f32; 3]; 3], pixel: &[f32; 3]) -> (f32, f32, f32) {
    let row =
        |r: usize| matrix[r][0] * pixel[0] + matrix[r][1] * pixel[1] + matrix[r][2] * pixel[2];
    (row(0), row(1), row(2))
}

#[cfg(test)]
mod tests {
    use super::{apply3, invert3};

    /// The colour matrix is the step with no visible intermediate: a wrong inverse produces a plausible
    /// photo with a cast. So the inverse is checked against the identity it must reconstruct, with a
    /// matrix shaped like a real camera's (asymmetric, with negative off-diagonal terms).
    #[test]
    fn inverting_a_camera_matrix_reconstructs_the_identity() {
        let xyz_to_cam = [
            [1.901, -0.734, -0.141],
            [-0.217, 1.112, 0.121],
            [0.008, 0.139, 0.628],
        ];
        let inverse = invert3(&xyz_to_cam).expect("a camera matrix is invertible");
        // Applying one then the other must return the input, for a vector that exercises all nine terms.
        let probe = [0.31f32, 0.52, 0.17];
        let (x, y, z) = apply3(&inverse, &probe);
        let (r, g, b) = apply3(&xyz_to_cam, &[x, y, z]);
        for (actual, expected) in [(r, probe[0]), (g, probe[1]), (b, probe[2])] {
            assert!((actual - expected).abs() < 1e-3, "{actual} vs {expected}");
        }
    }

    /// A singular matrix must be a HANDLED case, not a panic: it arrives from a file we do not control.
    #[test]
    fn a_singular_matrix_is_rejected_rather_than_dividing_by_zero() {
        let singular = [[1.0, 2.0, 3.0], [2.0, 4.0, 6.0], [1.0, 1.0, 1.0]];
        assert!(invert3(&singular).is_none());
    }
}
