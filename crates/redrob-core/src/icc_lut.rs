// SPDX-License-Identifier: GPL-3.0-or-later

//! ICC lookup-table transforms (`mft2`), for the perceptual and saturation intents (J.5-b).
//!
//! # How this was derived, which is not from source
//!
//! The reference implementation delegates colour transforms to littleCMS, which this repository
//! does not vendor, so there is no upstream C file to re-derive this from — the usual loop
//! procedure had nothing to read.
//!
//! The layout below was instead derived EMPIRICALLY from real profiles shipped in the Krita tree
//! (`krita/data/profiles/`), and then confirmed by arithmetic rather than by recollection of the
//! specification. For `bt709-6_ycbcr_v2.icc`'s `A2B0`: 3 input channels, 3 output channels, a
//! 24-point grid, 2 input and 2 output table entries gives
//! `52 + 3·2·2 + 24³·3·2 + 3·2·2 = 83020` bytes — exactly the tag size in its table. A layout that
//! were wrong by a single field would not reproduce that number.
//!
//! # What is deliberately NOT here
//!
//! `mft1` (the 8-bit variant) and a Lab PCS. Not an oversight and not laziness: a search of all 112
//! `.icc` files in the local corpus found **no `mft1` profile at all**, so there is nothing to
//! validate a parser against, and writing one from memory of the specification is exactly the guess
//! that reading real bytes avoids. Filed rather than bluffed.

use crate::FormatError;
use crate::{Result, color};

/// A parsed `mft2` pipeline: input curves, matrix, colour lookup table, output curves.
#[derive(Clone, Debug)]
pub(crate) struct Lut {
    input_channels: usize,
    output_channels: usize,
    grid_points: usize,
    /// Row-major 3x3, applied only for an XYZ PCS — see [`Self::apply`].
    matrix: [f64; 9],
    /// One table per input channel, each `input_entries` long, normalised to 0..=1.
    input_tables: Vec<Vec<f64>>,
    /// `grid_points^input_channels` entries of `output_channels` values each.
    clut: Vec<f64>,
    output_tables: Vec<Vec<f64>>,
}

impl Lut {
    /// Parses an `mft2` tag body.
    pub(crate) fn parse_mft2(body: &[u8]) -> Result<Self> {
        let malformed = || FormatError::Malformed("ICC lut tag");
        if body.len() < 52 || &body[0..4] != b"mft2" {
            return Err(malformed().into());
        }
        let input_channels = body[8] as usize;
        let output_channels = body[9] as usize;
        let grid_points = body[10] as usize;
        // Three in, three out. A CMYK or n-channel pipeline is a different feature with its own
        // colour model, and accepting one here would silently read four channels as three.
        if input_channels != 3 || output_channels != 3 || grid_points < 2 {
            return Err(FormatError::UnsupportedFeature(
                "this ICC lookup table is not a 3-to-3 transform",
            )
            .into());
        }
        let mut matrix = [0.0f64; 9];
        for (index, slot) in matrix.iter_mut().enumerate() {
            let at = 12 + index * 4;
            *slot = s15_fixed16(&body[at..at + 4]);
        }
        let input_entries = u16::from_be_bytes([body[48], body[49]]) as usize;
        let output_entries = u16::from_be_bytes([body[50], body[51]]) as usize;
        if input_entries < 2 || output_entries < 2 {
            return Err(malformed().into());
        }

        // Guard the total BEFORE allocating: the counts are attacker-controlled 8- and 16-bit
        // fields, and 255 grid points cubed times three channels is 50 MB from four bytes of input.
        let clut_entries = grid_points
            .checked_pow(input_channels as u32)
            .and_then(|cells| cells.checked_mul(output_channels))
            .ok_or_else(malformed)?;
        let expected = 52
            + input_channels * input_entries * 2
            + clut_entries * 2
            + output_channels * output_entries * 2;
        if expected > body.len() || clut_entries > 8 * 1024 * 1024 {
            return Err(malformed().into());
        }

        let mut at = 52;
        let read_tables = |channels: usize, entries: usize, at: &mut usize| {
            (0..channels)
                .map(|_| {
                    (0..entries)
                        .map(|_| {
                            let value =
                                u16::from_be_bytes([body[*at], body[*at + 1]]) as f64 / 65535.0;
                            *at += 2;
                            value
                        })
                        .collect()
                })
                .collect::<Vec<Vec<f64>>>()
        };
        let input_tables = read_tables(input_channels, input_entries, &mut at);
        let clut: Vec<f64> = (0..clut_entries)
            .map(|_| {
                let value = u16::from_be_bytes([body[at], body[at + 1]]) as f64 / 65535.0;
                at += 2;
                value
            })
            .collect();
        let output_tables = read_tables(output_channels, output_entries, &mut at);

        Ok(Self {
            input_channels,
            output_channels,
            grid_points,
            matrix,
            input_tables,
            clut,
            output_tables,
        })
    }

    /// Runs the pipeline: matrix, input curves, CLUT, output curves.
    ///
    /// `apply_matrix` is false unless the PCS is XYZ. The specification restricts the matrix to that
    /// case, and applying it to a Lab PCS would transform three values that are not a colour vector
    /// — the result looks like a plausible colour and is wrong everywhere.
    pub(crate) fn apply(&self, input: [f64; 3], apply_matrix: bool) -> [f64; 3] {
        let mut values = if apply_matrix {
            [
                self.matrix[0] * input[0] + self.matrix[1] * input[1] + self.matrix[2] * input[2],
                self.matrix[3] * input[0] + self.matrix[4] * input[1] + self.matrix[5] * input[2],
                self.matrix[6] * input[0] + self.matrix[7] * input[1] + self.matrix[8] * input[2],
            ]
        } else {
            input
        };
        // Indexed rather than zipped: the index selects a table AND a value in two different
        // arrays, and the channel count comes from the tag rather than from either length.
        #[allow(clippy::needless_range_loop)]
        for channel in 0..self.input_channels {
            values[channel] = sample(&self.input_tables[channel], values[channel]);
        }
        let interpolated = self.interpolate(values);
        let mut output = [0.0f64; 3];
        #[allow(clippy::needless_range_loop)]
        for channel in 0..self.output_channels {
            output[channel] = sample(&self.output_tables[channel], interpolated[channel]);
        }
        output
    }

    /// Trilinear interpolation in the colour lookup table.
    ///
    /// Trilinear rather than nearest-cell: a 24-point grid sampled by nearest neighbour quantises
    /// every colour to one of 24 levels per axis, which is visible as flat bands in any gradient —
    /// and a gradient is exactly where a profile's table matters. Tetrahedral interpolation would be
    /// marginally better on the diagonals and needs a cell decomposition this does not have; the
    /// difference is below one 8-bit step, where nearest-cell is tens of steps.
    fn interpolate(&self, values: [f64; 3]) -> [f64; 3] {
        let last = self.grid_points - 1;
        let mut base = [0usize; 3];
        let mut fraction = [0.0f64; 3];
        for channel in 0..3 {
            let position = values[channel].clamp(0.0, 1.0) * last as f64;
            let low = position.floor() as usize;
            base[channel] = low.min(last);
            fraction[channel] = position - low as f64;
        }
        let cell = |x: usize, y: usize, z: usize, channel: usize| -> f64 {
            let index = ((x.min(last) * self.grid_points + y.min(last)) * self.grid_points
                + z.min(last))
                * self.output_channels
                + channel;
            self.clut.get(index).copied().unwrap_or(0.0)
        };
        let mut output = [0.0f64; 3];
        #[allow(clippy::needless_range_loop)]
        for channel in 0..self.output_channels {
            let mut accumulated = 0.0;
            for corner in 0..8 {
                let (dx, dy, dz) = (corner & 1, (corner >> 1) & 1, (corner >> 2) & 1);
                let weight = (if dx == 1 {
                    fraction[0]
                } else {
                    1.0 - fraction[0]
                }) * (if dy == 1 {
                    fraction[1]
                } else {
                    1.0 - fraction[1]
                }) * (if dz == 1 {
                    fraction[2]
                } else {
                    1.0 - fraction[2]
                });
                if weight == 0.0 {
                    continue;
                }
                accumulated += weight * cell(base[0] + dx, base[1] + dy, base[2] + dz, channel);
            }
            output[channel] = accumulated;
        }
        output
    }
}

/// Interpolates a normalised table at `value`.
fn sample(table: &[f64], value: f64) -> f64 {
    if table.len() < 2 {
        return value;
    }
    let position = value.clamp(0.0, 1.0) * (table.len() - 1) as f64;
    let low = position.floor() as usize;
    let high = (low + 1).min(table.len() - 1);
    let fraction = position - low as f64;
    table[low] + (table[high] - table[low]) * fraction
}

fn s15_fixed16(bytes: &[u8]) -> f64 {
    f64::from(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])) / 65536.0
}

/// The 16-bit PCS XYZ encoding: 0..=0xFFFF maps to 0..=(65535/32768).
///
/// The odd scale is the specification's, not a convenience: the encoding reserves headroom above
/// 1.0 so a value brighter than the reference white survives. Treating it as plain 0..1 darkens
/// every colour a lut profile produces by a factor of two.
pub(crate) fn pcs_xyz_decode(value: f64) -> f64 {
    value * 65535.0 / 32768.0
}

/// The inverse of [`pcs_xyz_decode`].
pub(crate) fn pcs_xyz_encode(value: f64) -> f64 {
    value * 32768.0 / 65535.0
}

/// D50 XYZ to sRGB unit, shared by the matrix and lut paths.
pub(crate) fn xyz_d50_to_srgb_unit(x: f64, y: f64, z: f64) -> [f64; 3] {
    let adapt = color::bradford_adaptation(color::D50, color::D65);
    let xd = adapt[0] * x + adapt[1] * y + adapt[2] * z;
    let yd = adapt[3] * x + adapt[4] * y + adapt[5] * z;
    let zd = adapt[6] * x + adapt[7] * y + adapt[8] * z;
    let (r, g, b) = color::xyz_to_linear_srgb(xd, yd, zd);
    [
        color::linear_to_srgb(r),
        color::linear_to_srgb(g),
        color::linear_to_srgb(b),
    ]
}

/// sRGB unit to D50 XYZ.
pub(crate) fn srgb_unit_to_xyz_d50(srgb: [f64; 3]) -> (f64, f64, f64) {
    let (x, y, z) = color::linear_srgb_to_xyz(
        color::srgb_to_linear(srgb[0]),
        color::srgb_to_linear(srgb[1]),
        color::srgb_to_linear(srgb[2]),
    );
    let adapt = color::bradford_adaptation(color::D65, color::D50);
    (
        adapt[0] * x + adapt[1] * y + adapt[2] * z,
        adapt[3] * x + adapt[4] * y + adapt[5] * z,
        adapt[6] * x + adapt[7] * y + adapt[8] * z,
    )
}
