// SPDX-License-Identifier: GPL-3.0-or-later

//! Document sample precision: how many bits one colour component occupies in stored pixels.
//!
//! J.1a. Until now every buffer in this crate was four `u8` per pixel and nothing said so; the
//! assumption was spread across the renderer, the filters, undo and the FFI. This module makes it a
//! declared property of the document so the rest can be moved onto it one step at a time.
//!
//! # What this is NOT
//!
//! Precision here is the COMPONENT TYPE only — the width of one sample. The other axis, how the
//! stored number maps to light (sRGB-encoded versus linear), is deliberately left out. Reference
//! implementations spell both into one enumeration (`U8_LINEAR`, `U8_NON_LINEAR`, …), and that is
//! right for them, but folding the two together HERE would make one value mean two unrelated
//! things while our pipeline is still sRGB-encoded everywhere and a separate linear working buffer
//! already exists ([`crate::scene`]). Widening or linearising are different migrations; a reader
//! who finds `U16` must not have to guess which one happened.
//!
//! # Storage
//!
//! Samples stay in one byte buffer ([`crate::RasterBytes`]) and precision says how to read it.
//! That is what keeps this change small: undo snapshots, the project format and the FFI all stay
//! byte-oriented, so a document at 8-bit is byte-identical to one written before this existed.
//!
//! Multi-byte samples are LITTLE-ENDIAN. Not because it is the only defensible choice, but because
//! it must be written down somewhere a reader will find: this is our own buffer, never a wire
//! format, every target is little-endian, and a future reader guessing wrong would see a document
//! whose pixels are plausible but wrong rather than one that fails to load.

use serde::{Deserialize, Serialize};

/// The width of one colour component in stored pixels.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Precision {
    /// Eight-bit unsigned. The default, and what every document written before J.1a contains.
    #[default]
    U8,
    /// Sixteen-bit unsigned, little-endian.
    U16,
    /// Thirty-two-bit float, little-endian. Values outside 0..=1 survive storage: that headroom is
    /// the reason to choose it, so reading does not clamp.
    F32,
}

impl Precision {
    /// Bytes one sample occupies.
    pub const fn bytes_per_sample(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::U16 => 2,
            Self::F32 => 4,
        }
    }

    /// Bytes one RGBA pixel occupies.
    pub const fn bytes_per_pixel(self) -> usize {
        self.bytes_per_sample() * 4
    }

    /// Bytes needed for `pixels` RGBA pixels.
    pub const fn buffer_len(self, pixels: usize) -> usize {
        pixels * self.bytes_per_pixel()
    }

    /// How many whole RGBA pixels a buffer of `bytes` holds. A trailing partial pixel is not
    /// counted: a caller that would read it is already reading a buffer that does not match its
    /// declared precision, and returning the rounded-up count would hand it an out-of-range index.
    pub const fn pixel_count(self, bytes: usize) -> usize {
        bytes / self.bytes_per_pixel()
    }

    /// Reads sample `index` as a unit value.
    ///
    /// Integer precisions divide by their own maximum, so full-scale is exactly 1.0 and the two
    /// integer widths agree at the ends — 255 and 65535 both read as 1.0. Dividing `u16` by 65536
    /// instead (a tempting shift) would make white read as 0.99998 and a round-trip lose a step.
    ///
    /// `F32` is returned as stored, including values above 1.0 and below 0.0.
    pub fn read_sample(self, bytes: &[u8], index: usize) -> f32 {
        let at = index * self.bytes_per_sample();
        match self {
            Self::U8 => f32::from(bytes[at]) / 255.0,
            Self::U16 => {
                let raw = u16::from_le_bytes([bytes[at], bytes[at + 1]]);
                f32::from(raw) / 65535.0
            }
            Self::F32 => {
                f32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
            }
        }
    }

    /// Writes sample `index` from a unit value.
    ///
    /// Integer precisions round to nearest and clamp; rounding matters because truncation loses
    /// half a step on every write and a repeated filter would visibly drift darker.
    pub fn write_sample(self, bytes: &mut [u8], index: usize, value: f32) {
        let at = index * self.bytes_per_sample();
        match self {
            Self::U8 => bytes[at] = (value * 255.0).round().clamp(0.0, 255.0) as u8,
            Self::U16 => {
                let raw = (value * 65535.0).round().clamp(0.0, 65535.0) as u16;
                bytes[at..at + 2].copy_from_slice(&raw.to_le_bytes());
            }
            Self::F32 => bytes[at..at + 4].copy_from_slice(&value.to_le_bytes()),
        }
    }

    /// Re-encodes a whole buffer into `target` precision.
    ///
    /// Converting to a narrower precision LOSES data and says so by returning the loss: that is
    /// information the caller must be able to put in front of a user, and a conversion function
    /// that quietly discards it is how a 16-bit import came to be narrowed with nothing reported.
    pub fn convert(self, bytes: &[u8], target: Precision) -> Converted {
        if self == target {
            return Converted {
                bytes: bytes.to_vec(),
                narrowed: false,
            };
        }
        let samples = bytes.len() / self.bytes_per_sample();
        let mut out = vec![0u8; samples * target.bytes_per_sample()];
        for index in 0..samples {
            target.write_sample(&mut out, index, self.read_sample(bytes, index));
        }
        Converted {
            bytes: out,
            // Float holds every integer value either width can carry, so widening and going to
            // float are lossless. Everything else drops bits.
            narrowed: !matches!(
                (self, target),
                (Precision::U8, Precision::U16)
                    | (Precision::U8, Precision::F32)
                    | (Precision::U16, Precision::F32)
            ),
        }
    }
}

/// The result of a precision change: the re-encoded buffer, and whether bits were dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Converted {
    pub bytes: Vec<u8>,
    pub narrowed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_scale_agrees_across_integer_widths() {
        // The one number most likely to be wrong: if u16 divided by 65536, white would read 0.99998
        // and a widen-then-narrow round trip would lose a step at the top end.
        assert_eq!(Precision::U8.read_sample(&[255], 0), 1.0);
        assert_eq!(Precision::U16.read_sample(&[0xff, 0xff], 0), 1.0);
        assert_eq!(Precision::U8.read_sample(&[0], 0), 0.0);
        assert_eq!(Precision::U16.read_sample(&[0, 0], 0), 0.0);
    }

    #[test]
    fn widening_is_lossless_and_narrowing_reports_itself() {
        let eight: Vec<u8> = (0..=255u8).collect();
        let wide = Precision::U8.convert(&eight, Precision::U16);
        assert!(!wide.narrowed, "8 -> 16 keeps every value");
        let back = Precision::U16.convert(&wide.bytes, Precision::U8);
        assert!(back.narrowed, "16 -> 8 drops bits and must say so");
        assert_eq!(
            back.bytes, eight,
            "every 8-bit value survives the round trip"
        );
    }

    #[test]
    fn float_keeps_values_outside_the_unit_range() {
        // Headroom is the whole reason to pick F32, so storage must not clamp it.
        let mut bytes = vec![0u8; 4];
        Precision::F32.write_sample(&mut bytes, 0, 2.5);
        assert_eq!(Precision::F32.read_sample(&bytes, 0), 2.5);
        Precision::F32.write_sample(&mut bytes, 0, -0.25);
        assert_eq!(Precision::F32.read_sample(&bytes, 0), -0.25);
    }

    #[test]
    fn integer_writes_round_rather_than_truncate() {
        // Truncation loses half a step per write, which a repeated filter turns into visible drift.
        let mut bytes = vec![0u8; 1];
        Precision::U8.write_sample(&mut bytes, 0, 100.4 / 255.0);
        assert_eq!(bytes[0], 100);
        Precision::U8.write_sample(&mut bytes, 0, 100.6 / 255.0);
        assert_eq!(bytes[0], 101);
    }

    #[test]
    fn buffer_geometry_matches_the_declared_precision() {
        assert_eq!(Precision::U8.buffer_len(4), 16);
        assert_eq!(Precision::U16.buffer_len(4), 32);
        assert_eq!(Precision::F32.buffer_len(4), 64);
        assert_eq!(Precision::U16.pixel_count(32), 4);
        // A trailing partial pixel is not counted; counting it would hand out an index that reads
        // past the end.
        assert_eq!(Precision::U16.pixel_count(33), 4);
    }
}
