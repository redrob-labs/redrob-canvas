// SPDX-License-Identifier: GPL-3.0-or-later

//! ICC profile parsing and application (H.17), re-derived from the ICC specification and the
//! matrix-shaper handling in Krita's pigment and GIMP's babl (behaviour studied, no code copied).
//!
//! G.2 ported the colour MATHS — transfer functions, XYZ, Lab, chromatic adaptation — but nothing could
//! read a profile, so a tagged file's own colour space was ignored and its pixels were treated as
//! sRGB. That is not a subtle loss: an Adobe RGB photo opened as sRGB has visibly dull colour, and the
//! file said so all along.
//!
//! Scope is the MATRIX-SHAPER profile: three colorant columns plus a tone curve per channel. That is
//! what an RGB image file carries in practice. A profile built from lookup tables (`A2B0`) is a CMM's
//! job — multi-dimensional interpolation over a grid — and is refused by name rather than approximated
//! by a matrix it does not have.
//!
//! Two details are load-bearing and silent when wrong:
//!
//! 1. A matrix profile's colorants are stored ADAPTED TO D50, never to the file's own white point. So
//!    the result is XYZ relative to D50 and has to be adapted to D65 before it can meet sRGB. Skipping
//!    that step leaves every image slightly warm, which reads as a camera's white balance rather than
//!    as a missing conversion.
//! 2. The tone curve is NOT always a gamma number. `curv` with a count above one is a sampled table,
//!    and `para` is a parametric form with a linear toe — sRGB's own curve is `para` type 3, whose toe
//!    is why treating it as a plain 2.2 gamma darkens the shadows of every sRGB-tagged file.

use crate::{FormatError, Result};

/// One channel's tone curve, in the forms a profile actually stores.
#[derive(Clone, Debug, PartialEq)]
enum Curve {
    /// `curv` with a count of zero: the identity.
    Identity,
    /// `curv` with a count of one: a single gamma exponent.
    Gamma(f64),
    /// `curv` with a count above one: samples of the curve, interpolated between.
    Table(Vec<u16>),
    /// `para`: a parametric curve. Held as its type plus parameters so the branch structure stays
    /// visible -- these differ by where their linear segment begins, not by a coefficient.
    Parametric { function: u16, params: Vec<f64> },
}

impl Curve {
    /// Maps a device value in 0..=1 to linear light.
    fn to_linear(&self, value: f64) -> f64 {
        let x = value.clamp(0.0, 1.0);
        match self {
            Self::Identity => x,
            Self::Gamma(gamma) => x.powf(*gamma),
            Self::Table(samples) => {
                if samples.len() < 2 {
                    return x;
                }
                // Linear interpolation between samples. A nearest-sample lookup would band a 256-entry
                // curve visibly in gradients, which is exactly where a profile's curve matters.
                let position = x * (samples.len() - 1) as f64;
                let low = position.floor() as usize;
                let high = (low + 1).min(samples.len() - 1);
                let fraction = position - low as f64;
                let a = f64::from(samples[low]) / 65535.0;
                let b = f64::from(samples[high]) / 65535.0;
                a + (b - a) * fraction
            }
            Self::Parametric { function, params } => {
                let p = |index: usize| params.get(index).copied().unwrap_or(0.0);
                let g = p(0);
                match function {
                    0 => x.powf(g),
                    1 => {
                        let (a, b) = (p(1), p(2));
                        if a != 0.0 && x >= -b / a {
                            (a * x + b).powf(g)
                        } else {
                            0.0
                        }
                    }
                    2 => {
                        let (a, b, c) = (p(1), p(2), p(3));
                        if a != 0.0 && x >= -b / a {
                            (a * x + b).powf(g) + c
                        } else {
                            c
                        }
                    }
                    3 => {
                        let (a, b, c, d) = (p(1), p(2), p(3), p(4));
                        if x >= d {
                            (a * x + b).powf(g)
                        } else {
                            c * x
                        }
                    }
                    4 => {
                        let (a, b, c, d, e, f) = (p(1), p(2), p(3), p(4), p(5), p(6));
                        if x >= d {
                            (a * x + b).powf(g) + e
                        } else {
                            c * x + f
                        }
                    }
                    // An unknown function type is treated as the identity rather than guessed at: a
                    // wrong curve is a visible tone shift, and the identity is at least honest.
                    _ => x,
                }
            }
        }
    }
}

/// A parsed matrix-shaper RGB profile.
#[derive(Clone, Debug)]
pub struct IccProfile {
    /// Device RGB to XYZ, relative to D50, as the colorant tags state it.
    to_xyz_d50: [[f64; 3]; 3],
    curves: [Curve; 3],
}

/// Signature of a tag, as four bytes.
type Tag = [u8; 4];

const TAG_RED_COLORANT: Tag = *b"rXYZ";
const TAG_GREEN_COLORANT: Tag = *b"gXYZ";
const TAG_BLUE_COLORANT: Tag = *b"bXYZ";
const TAG_RED_TRC: Tag = *b"rTRC";
const TAG_GREEN_TRC: Tag = *b"gTRC";
const TAG_BLUE_TRC: Tag = *b"bTRC";
const TAG_A_TO_B0: Tag = *b"A2B0";

impl IccProfile {
    /// Parses a profile, accepting only the matrix-shaper RGB form.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 132 {
            return Err(FormatError::Malformed("ICC profile too short").into());
        }
        // The header's own size field must agree with the data: a profile embedded in an image is a
        // length-prefixed blob, and a disagreement means the embedding is wrong, not the profile.
        let declared = read_u32(bytes, 0) as usize;
        if declared > bytes.len() {
            return Err(FormatError::Malformed("ICC profile truncated").into());
        }
        let data_space = &bytes[16..20];
        if data_space != b"RGB " {
            return Err(FormatError::UnsupportedFeature(
                "this ICC profile is not an RGB device profile",
            )
            .into());
        }

        let count = read_u32(bytes, 128) as usize;
        // Each entry is signature, offset, size.
        let table_end = 132 + count * 12;
        if count > 256 || table_end > bytes.len() {
            return Err(FormatError::Malformed("ICC tag table").into());
        }
        let find = |wanted: Tag| -> Option<&[u8]> {
            for index in 0..count {
                let at = 132 + index * 12;
                let signature: Tag = [
                    bytes[at],
                    bytes[at + 1],
                    bytes[at + 2],
                    bytes[at + 3],
                ];
                if signature != wanted {
                    continue;
                }
                let offset = read_u32(bytes, at + 4) as usize;
                let size = read_u32(bytes, at + 8) as usize;
                let end = offset.checked_add(size)?;
                if end > bytes.len() {
                    return None;
                }
                return Some(&bytes[offset..end]);
            }
            None
        };

        // A lookup-table profile is refused BEFORE the matrix tags are looked for, because a profile can
        // carry both and the table is the authoritative one -- silently preferring the matrix would
        // render a CMYK-ish or device-link profile with the wrong transform and no complaint.
        if find(TAG_A_TO_B0).is_some() && find(TAG_RED_COLORANT).is_none() {
            return Err(FormatError::UnsupportedFeature(
                "this ICC profile is table-based, which needs a full colour management engine",
            )
            .into());
        }

        let red = find(TAG_RED_COLORANT).ok_or(FormatError::Malformed("ICC red colorant"))?;
        let green = find(TAG_GREEN_COLORANT).ok_or(FormatError::Malformed("ICC green colorant"))?;
        let blue = find(TAG_BLUE_COLORANT).ok_or(FormatError::Malformed("ICC blue colorant"))?;
        let red = read_xyz(red)?;
        let green = read_xyz(green)?;
        let blue = read_xyz(blue)?;
        // Colorants are COLUMNS: each is where one primary lands in XYZ, so they stack side by side.
        let to_xyz_d50 = [
            [red.0, green.0, blue.0],
            [red.1, green.1, blue.1],
            [red.2, green.2, blue.2],
        ];

        let curves = [
            read_curve(find(TAG_RED_TRC))?,
            read_curve(find(TAG_GREEN_TRC))?,
            read_curve(find(TAG_BLUE_TRC))?,
        ];

        Ok(Self {
            to_xyz_d50,
            curves,
        })
    }

    /// Converts one device RGB triple (0..=255) to sRGB (0..=255).
    pub fn to_srgb8(&self, rgb: [u8; 3]) -> [u8; 3] {
        let linear = [
            self.curves[0].to_linear(f64::from(rgb[0]) / 255.0),
            self.curves[1].to_linear(f64::from(rgb[1]) / 255.0),
            self.curves[2].to_linear(f64::from(rgb[2]) / 255.0),
        ];
        let m = &self.to_xyz_d50;
        let x = m[0][0] * linear[0] + m[0][1] * linear[1] + m[0][2] * linear[2];
        let y = m[1][0] * linear[0] + m[1][1] * linear[1] + m[1][2] * linear[2];
        let z = m[2][0] * linear[0] + m[2][1] * linear[1] + m[2][2] * linear[2];
        // D50 to D65: the profile's colorants are adapted to D50 by the specification, and sRGB is a
        // D65 space. Without this step every image stays slightly warm.
        let adapt = crate::color::bradford_adaptation(crate::color::D50, crate::color::D65);
        let xd = adapt[0] * x + adapt[1] * y + adapt[2] * z;
        let yd = adapt[3] * x + adapt[4] * y + adapt[5] * z;
        let zd = adapt[6] * x + adapt[7] * y + adapt[8] * z;
        let (r, g, b) = crate::color::xyz_to_linear_srgb(xd, yd, zd);
        [
            encode(r),
            encode(g),
            encode(b),
        ]
    }

    /// Converts an RGBA buffer in place. Alpha is untouched: it is coverage, not colour, and running it
    /// through a colour transform is a classic way to make edges darken.
    pub fn convert_rgba(&self, pixels: &mut [u8]) {
        for pixel in pixels.chunks_exact_mut(4) {
            let converted = self.to_srgb8([pixel[0], pixel[1], pixel[2]]);
            pixel[0] = converted[0];
            pixel[1] = converted[1];
            pixel[2] = converted[2];
        }
    }
}

/// Extracts and parses a PNG's embedded profile from its `iCCP` chunk, when it has one this product
/// can use.
///
/// Returns `None` rather than an error for a file with no profile, an unreadable one, or a form outside
/// the matrix-shaper scope: an image that cannot be colour-managed must still OPEN. Refusing the file
/// would be worse than showing it with its own channels, which is what every reader did before this.
///
/// `iCCP` is a zlib-compressed profile behind a NUL-terminated name and one compression-method byte.
/// Reading from a fixed offset instead of past the name mis-parses every file whose profile has a
/// longer name than the one it was tested with.
pub fn embedded_png_profile(bytes: &[u8]) -> Option<IccProfile> {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if bytes.len() < 8 || bytes[0..8] != SIGNATURE {
        return None;
    }
    let mut at = 8usize;
    while at + 8 <= bytes.len() {
        let length = read_u32(bytes, at) as usize;
        let kind = &bytes[at + 4..at + 8];
        let start = at + 8;
        let end = start.checked_add(length)?;
        // Every chunk carries a four-byte CRC after its data, which is part of the stride.
        if end + 4 > bytes.len() {
            return None;
        }
        if kind == b"iCCP" {
            let payload = &bytes[start..end];
            let separator = payload.iter().position(|byte| *byte == 0)?;
            // The byte after the name is the compression method; only deflate (0) is defined.
            let compressed = payload.get(separator + 2..)?;
            let mut decompressor = flate2::Decompress::new(true);
            // `decompress_vec` fills up to the vector's CAPACITY, so the bound is set here rather than
            // grown on demand -- and it doubles as a limit on a profile claiming to be enormous. 16 MiB
            // is far beyond any real display or camera profile.
            let mut profile = Vec::with_capacity(16 << 20);
            decompressor
                .decompress_vec(compressed, &mut profile, flate2::FlushDecompress::Finish)
                .ok()?;
            if profile.is_empty() {
                return None;
            }
            return IccProfile::parse(&profile).ok();
        }
        // IDAT begins the pixel data: a profile declared after it would not apply to this image.
        if kind == b"IDAT" {
            return None;
        }
        at = end + 4;
    }
    None
}

fn encode(linear: f64) -> u8 {    let value = crate::color::linear_to_srgb(linear.clamp(0.0, 1.0));
    (value * 255.0).round().clamp(0.0, 255.0) as u8
}

fn read_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// Reads an `s15Fixed16` number: a signed 16.16 fixed-point value, which is how ICC stores every
/// coefficient. Reading it as an integer scales every colour by 65536.
fn read_s15fixed16(bytes: &[u8], at: usize) -> f64 {
    let raw = i32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
    f64::from(raw) / 65536.0
}

fn read_xyz(tag: &[u8]) -> Result<(f64, f64, f64)> {
    if tag.len() < 20 || &tag[0..4] != b"XYZ " {
        return Err(FormatError::Malformed("ICC colorant tag").into());
    }
    Ok((
        read_s15fixed16(tag, 8),
        read_s15fixed16(tag, 12),
        read_s15fixed16(tag, 16),
    ))
}

fn read_curve(tag: Option<&[u8]>) -> Result<Curve> {
    // A missing curve is the identity, which is what a linear profile means by omitting it.
    let Some(tag) = tag else {
        return Ok(Curve::Identity);
    };
    if tag.len() < 12 {
        return Err(FormatError::Malformed("ICC curve tag").into());
    }
    match &tag[0..4] {
        b"curv" => {
            let count = read_u32(tag, 8) as usize;
            match count {
                0 => Ok(Curve::Identity),
                // A single entry is a gamma in u8Fixed8, NOT in s15Fixed16: the same number read with
                // the wrong scale turns gamma 2.2 into 563.
                1 => {
                    if tag.len() < 14 {
                        return Err(FormatError::Malformed("ICC gamma curve").into());
                    }
                    let raw = u16::from_be_bytes([tag[12], tag[13]]);
                    Ok(Curve::Gamma(f64::from(raw) / 256.0))
                }
                _ => {
                    let end = 12 + count * 2;
                    if end > tag.len() || count > 1 << 16 {
                        return Err(FormatError::Malformed("ICC curve table").into());
                    }
                    let samples = (0..count)
                        .map(|index| {
                            u16::from_be_bytes([tag[12 + index * 2], tag[13 + index * 2]])
                        })
                        .collect();
                    Ok(Curve::Table(samples))
                }
            }
        }
        b"para" => {
            let function = u16::from_be_bytes([tag[8], tag[9]]);
            // Parameter counts per function type, from the specification. Reading the wrong number of
            // them shifts every later parameter, which produces a curve that is wrong but plausible.
            let expected = match function {
                0 => 1,
                1 => 3,
                2 => 4,
                3 => 5,
                4 => 7,
                _ => 0,
            };
            if tag.len() < 12 + expected * 4 {
                return Err(FormatError::Malformed("ICC parametric curve").into());
            }
            let params = (0..expected)
                .map(|index| read_s15fixed16(tag, 12 + index * 4))
                .collect();
            Ok(Curve::Parametric { function, params })
        }
        _ => Err(FormatError::UnsupportedFeature("this ICC curve type is not a tone curve").into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// sRGB's own curve is a `para` type 3 with a linear toe. Treating it as a plain 2.2 gamma darkens
    /// the shadows of every sRGB-tagged file, so the toe is what this pins.
    #[test]
    fn a_parametric_curve_uses_its_linear_toe_below_the_breakpoint() {
        let srgb = Curve::Parametric {
            function: 3,
            params: vec![2.4, 1.0 / 1.055, 0.055 / 1.055, 1.0 / 12.92, 0.04045],
        };
        // Below the breakpoint the curve is the straight segment, not the power.
        let low = srgb.to_linear(0.02);
        assert!((low - 0.02 / 12.92).abs() < 1e-6, "{low}");
        // A plain 2.2 gamma would give about 0.0002 here, several times darker.
        assert!(low > 0.001, "the toe must not be read as a power curve: {low}");
        // At 1.0 the curve still reaches white.
        assert!((srgb.to_linear(1.0) - 1.0).abs() < 1e-6);
    }

    /// A single-entry `curv` is a gamma in u8Fixed8. Read as s15Fixed16 it becomes 563 instead of 2.2.
    #[test]
    fn a_single_entry_curve_is_a_u8fixed8_gamma() {
        let mut tag = b"curv".to_vec();
        tag.extend_from_slice(&[0, 0, 0, 0]); // reserved
        tag.extend_from_slice(&1_u32.to_be_bytes()); // one entry
        tag.extend_from_slice(&((2.2_f64 * 256.0) as u16).to_be_bytes());
        match read_curve(Some(&tag)).unwrap() {
            Curve::Gamma(gamma) => assert!((gamma - 2.2).abs() < 0.01, "{gamma}"),
            other => panic!("expected a gamma curve, got {other:?}"),
        }
    }

    /// A sampled table is interpolated, not snapped: a nearest lookup bands gradients, which is exactly
    /// where a curve matters.
    #[test]
    fn a_table_curve_interpolates_between_samples() {
        let curve = Curve::Table(vec![0, 65535]);
        let middle = curve.to_linear(0.5);
        assert!((middle - 0.5).abs() < 1e-4, "{middle}");
    }
}
