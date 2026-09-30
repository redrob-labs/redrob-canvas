//! Brush tip images: decoding GIMP's GBR format and sampling a tip as dab coverage.
//!
//! TRANSLATED from Krita, pinned at `fdbf33b2146735465bb8aa59928fbc1890ceb160`, GPL-2.0-or-later, taken
//! into this GPL-3.0-or-later product under the or-later grant:
//!
//! - `libs/brush/kis_gbr_brush.cpp` — `KisGbrBrush::init`, the header layout and the two depths
//!
//! Licence change notice: the original is GPL-2.0-or-later; this file is GPL-3.0-or-later, which the
//! or-later grant permits. Attribution is in `UPSTREAM_NOTICES.md`.
//!
//! # Why the format is worth translating and the documentation is not
//!
//! GBR is GIMP's brush format and its specification is thin. Krita's reader *is* the specification
//! available to us, which is what item 1c.2 means by these formats being unobtainable elsewhere. The
//! previous cycle translated the generated dab shape so a loaded tip would have somewhere to go.
//!
//! # Two inversions that compose, and getting them wrong gives a negative
//!
//! GIMP stores a grayscale GBR with **255 meaning paint**. Krita's masks run the other way — 0 is opaque —
//! so its reader stores `255 - v`. This product emits coverage, where 1.0 is opaque, so it inverts Krita's
//! stored value back again and **the GBR byte maps straight to coverage**. Verified by compiling Krita's
//! own load path: byte 0 becomes coverage 0.00 and byte 255 becomes 1.00.

use serde::{Deserialize, Serialize};

/// The largest tip this will decode, in pixels.
///
/// The header declares the dimensions, so without a bound a five-byte file could ask for a terabyte. 512
/// square is far larger than any hand-made brush and is 256 KB of coverage.
pub const MAX_BRUSH_TIP_PIXELS: usize = 512 * 512;

/// The largest tip dimension, so a 1 × 262144 strip is refused as well as a square one.
pub const MAX_BRUSH_TIP_EDGE: u32 = 4096;

/// GIMP's default spacing, as a fraction, used for version 1 files that carry none.
const DEFAULT_SPACING: f32 = 0.10;

const V1_HEADER: usize = 20;
const V2_HEADER: usize = 28;
/// `GIMP` as a big-endian u32, which a version 2 file carries and a version 1 file does not.
const GIMP_V2_MAGIC: u32 = u32::from_be_bytes(*b"GIMP");

/// A brush tip: a grayscale coverage image with its spacing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrushTip {
    width: u32,
    height: u32,
    /// Row-major coverage, 0 for uncovered and 255 for fully covered.
    coverage: Vec<u8>,
    /// Dab spacing as a fraction of the tip's size, as GIMP stores it divided by 100.
    spacing: f32,
    /// The tip's name from the file. Kept because it is the only human-readable identifier a GBR has.
    name: String,
}

/// Why a GBR could not be read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GbrError {
    #[error("the data is shorter than a GBR header")]
    TooShort,
    #[error("the declared header size is zero or runs past the end of the data")]
    BadHeaderSize,
    #[error("GBR version {version} is not supported")]
    UnsupportedVersion { version: u32 },
    #[error("a GBR depth of {bytes} bytes is not supported; only 1 and 4 are")]
    UnsupportedDepth { bytes: u32 },
    #[error("a GBR with a zero width or height has no tip")]
    EmptyTip,
    #[error("the tip is {pixels} pixels, past the {max} this will decode")]
    TooLarge { pixels: usize, max: usize },
    #[error("the tip is {edge} pixels on one edge, past the {max} this will decode")]
    EdgeTooLong { edge: u32, max: u32 },
    #[error("the pixel data is shorter than the declared dimensions require")]
    PayloadTooShort,
    #[error("the declared spacing {spacing} is above GIMP's maximum of 1000")]
    SpacingTooLarge { spacing: u32 },
    #[error("the tip's name is not valid UTF-8")]
    BadName,
}

impl BrushTip {
    /// Decodes a GBR file.
    ///
    /// Krita reads the fields in a fixed order and checks the **version 2** header length before it has
    /// read the version. Measured consequence: a valid 23-byte version 1 file — a 20-byte header, a
    /// one-character name and a single pixel — is refused by Krita outright, because 28 > 23. This reads
    /// the version first and then requires the header length that version actually needs.
    pub fn from_gbr(data: &[u8]) -> Result<Self, GbrError> {
        // Enough for the five fields every version shares. The version is among them.
        if data.len() < V1_HEADER {
            return Err(GbrError::TooShort);
        }
        let header_size = be32(data, 0) as usize;
        let version = be32(data, 4);
        let width = be32(data, 8);
        let height = be32(data, 12);
        let bytes = be32(data, 16);

        let name_base = match version {
            1 => V1_HEADER,
            // Krita's comment notes version 3 is CinePaint's and may hold float16 data, which it does not
            // handle either. Refused by name rather than read as if it were version 2.
            2 => V2_HEADER,
            version => return Err(GbrError::UnsupportedVersion { version }),
        };
        if data.len() < name_base {
            return Err(GbrError::TooShort);
        }

        let spacing_percent = if version == 1 {
            // No spacing field exists in version 1, so GIMP's default stands in.
            (DEFAULT_SPACING * 100.0) as u32
        } else {
            let spacing = be32(data, 24);
            if spacing > 1000 {
                return Err(GbrError::SpacingTooLarge { spacing });
            }
            spacing
        };

        if header_size == 0 || header_size > data.len() {
            return Err(GbrError::BadHeaderSize);
        }
        // The name occupies the header after the fixed fields, and ends with a NUL that is counted in
        // `header_size`. A header that cannot hold even the NUL is malformed rather than unnamed.
        if header_size < name_base + 1 {
            return Err(GbrError::BadHeaderSize);
        }
        let name_bytes = &data[name_base..header_size - 1];
        // Version 1's encoding is undefined and Krita reads it as Latin-1; version 2 is UTF-8. A name is
        // a label, so an undecodable one is not worth refusing a whole brush over -- except that silently
        // replacing bytes would hide a misparsed header, which is why the length checks above come first.
        let name = if version == 1 {
            name_bytes.iter().map(|byte| *byte as char).collect()
        } else {
            String::from_utf8(name_bytes.to_vec()).map_err(|_| GbrError::BadName)?
        };

        if width == 0 || height == 0 {
            return Err(GbrError::EmptyTip);
        }
        if width > MAX_BRUSH_TIP_EDGE {
            return Err(GbrError::EdgeTooLong {
                edge: width,
                max: MAX_BRUSH_TIP_EDGE,
            });
        }
        if height > MAX_BRUSH_TIP_EDGE {
            return Err(GbrError::EdgeTooLong {
                edge: height,
                max: MAX_BRUSH_TIP_EDGE,
            });
        }
        // Checked as usize AFTER the edge bounds, so the multiplication cannot overflow on the way to the
        // check that would have caught it.
        let pixels = width as usize * height as usize;
        if pixels > MAX_BRUSH_TIP_PIXELS {
            return Err(GbrError::TooLarge {
                pixels,
                max: MAX_BRUSH_TIP_PIXELS,
            });
        }

        let payload = &data[header_size..];
        let coverage = match bytes {
            // Grayscale. The GBR byte IS the coverage -- see the module comment on the two inversions.
            1 => {
                if payload.len() < pixels {
                    return Err(GbrError::PayloadTooShort);
                }
                payload[..pixels].to_vec()
            }
            // RGBA. Only the alpha channel describes coverage; the colour is a tip's own colour, which
            // this product does not carry, so it is dropped rather than averaged into the mask.
            4 => {
                let needed = pixels.checked_mul(4).ok_or(GbrError::PayloadTooShort)?;
                if payload.len() < needed {
                    return Err(GbrError::PayloadTooShort);
                }
                payload[..needed]
                    .chunks_exact(4)
                    .map(|pixel| pixel[3])
                    .collect()
            }
            bytes => return Err(GbrError::UnsupportedDepth { bytes }),
        };

        Ok(Self {
            width,
            height,
            coverage,
            spacing: spacing_percent as f32 / 100.0,
            name,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn spacing(&self) -> f32 {
        self.spacing
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Coverage at a pixel of the tip's own grid.
    pub fn pixel(&self, x: u32, y: u32) -> u8 {
        if x >= self.width || y >= self.height {
            return 0;
        }
        self.coverage[(y as usize * self.width as usize) + x as usize]
    }

    /// Whether the tip has a sane shape, for the command boundary.
    ///
    /// A tip arrives from a serialised command as well as from a file, so the deserialised form must be
    /// checked too: `coverage` could carry any length regardless of the declared dimensions.
    pub fn is_valid(&self) -> bool {
        self.width > 0
            && self.height > 0
            && self.width <= MAX_BRUSH_TIP_EDGE
            && self.height <= MAX_BRUSH_TIP_EDGE
            && self.coverage.len() == self.width as usize * self.height as usize
            && self.spacing.is_finite()
            && (0.0..=10.0).contains(&self.spacing)
    }

    /// Coverage at an offset from the dab's centre, for a dab of the given diameter.
    ///
    /// The tip is stretched to fit a box `diameter` wide, keeping its own aspect ratio so a wide tip stays
    /// wide. Sampling is bilinear.
    ///
    /// Krita samples from a pyramid of pre-scaled masks instead, which is faster and is its own block of
    /// the port. Bilinear from the source is correct and slower; nothing here depends on it being fast
    /// yet, and a pyramid built now would be optimising before measuring.
    pub fn coverage_at(&self, dx: f32, dy: f32, diameter: f32) -> f32 {
        if !diameter.is_finite() || diameter <= 0.0 || !self.is_valid() {
            return 0.0;
        }
        let scale_x = self.width as f32 / diameter;
        let aspect = self.height as f32 / self.width as f32;
        let scale_y = self.height as f32 / (diameter * aspect);

        // Centre of the tip's grid, offset by where we are in the dab.
        let tip_x = self.width as f32 * 0.5 + dx * scale_x;
        let tip_y = self.height as f32 * 0.5 + dy * scale_y;

        // Sampling at pixel centres, so the texel covering tip_x is at tip_x - 0.5.
        let sample_x = tip_x - 0.5;
        let sample_y = tip_y - 0.5;
        let x0 = sample_x.floor();
        let y0 = sample_y.floor();
        let fx = sample_x - x0;
        let fy = sample_y - y0;

        let at = |x: f32, y: f32| -> f32 {
            if x < 0.0 || y < 0.0 || x >= self.width as f32 || y >= self.height as f32 {
                return 0.0;
            }
            f32::from(self.pixel(x as u32, y as u32)) / 255.0
        };

        let top = at(x0, y0) * (1.0 - fx) + at(x0 + 1.0, y0) * fx;
        let bottom = at(x0, y0 + 1.0) * (1.0 - fx) + at(x0 + 1.0, y0 + 1.0) * fx;
        (top * (1.0 - fy) + bottom * fy).clamp(0.0, 1.0)
    }
}

fn be32(data: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

/// Whether a byte slice starts like a version 2 GBR, for telling formats apart.
pub fn looks_like_gbr_v2(data: &[u8]) -> bool {
    data.len() >= V2_HEADER && be32(data, 20) == GIMP_V2_MAGIC
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Byte arrays printed by the reference decoder built from Krita's own load path.
    ///
    /// A 3x2 grayscale ramp, version 2, spacing 25, named "ramp". The payload bytes are
    /// `0, 64, 128, 192, 255, 32`, and GIMP's convention makes 255 the fully painted one.
    const V2_GRAY: &[u8] = &[
        0, 0, 0, 33, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 2, 0, 0, 0, 1, 71, 73, 77, 80, 0, 0, 0, 25,
        114, 97, 109, 112, 0, 0, 64, 128, 192, 255, 32,
    ];

    /// The same tip as version 1: no magic number, no spacing field, name at offset 20.
    const V1_GRAY: &[u8] = &[
        0, 0, 0, 24, 0, 0, 0, 1, 0, 0, 0, 3, 0, 0, 0, 2, 0, 0, 0, 1, 111, 108, 100, 0, 0, 64, 128,
        192, 255, 32,
    ];

    /// Version 2, RGBA, 3x2. Alphas are 255, 128, 0, 255, 64, 200.
    const V2_RGBA: &[u8] = &[
        0, 0, 0, 35, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0, 2, 0, 0, 0, 4, 71, 73, 77, 80, 0, 0, 0, 10,
        99, 111, 108, 111, 117, 114, 0, 255, 0, 0, 255, 0, 255, 0, 128, 0, 0, 255, 0, 255, 255,
        255, 255, 0, 0, 0, 64, 9, 9, 9, 200,
    ];

    /// Builds a GBR the way the reference did, so the negative cases can be constructed precisely.
    fn build(
        version: u32,
        width: u32,
        height: u32,
        bytes: u32,
        spacing: u32,
        name: &str,
        payload: &[u8],
    ) -> Vec<u8> {
        let base = if version == 1 { V1_HEADER } else { V2_HEADER };
        let header_size = (base + name.len() + 1) as u32;
        let mut out = Vec::new();
        for field in [header_size, version, width, height, bytes] {
            out.extend_from_slice(&field.to_be_bytes());
        }
        if version != 1 {
            out.extend_from_slice(&GIMP_V2_MAGIC.to_be_bytes());
            out.extend_from_slice(&spacing.to_be_bytes());
        }
        out.extend_from_slice(name.as_bytes());
        out.push(0);
        out.extend_from_slice(payload);
        out
    }

    /// The GBR byte IS the coverage. This is the assertion that fails if either inversion is dropped or
    /// doubled, which would give a photographic negative of every brush.
    #[test]
    fn a_grayscale_tip_decodes_with_the_byte_as_coverage() {
        let tip = BrushTip::from_gbr(V2_GRAY).unwrap();
        assert_eq!(tip.width(), 3);
        assert_eq!(tip.height(), 2);
        assert_eq!(tip.name(), "ramp");
        assert!(
            (tip.spacing() - 0.25).abs() < 1e-6,
            "spacing {}",
            tip.spacing()
        );

        // Reference coverage: 0.00, 0.25, 0.50, 0.75, 1.00, 0.13.
        assert_eq!(tip.pixel(0, 0), 0, "byte 0 is uncovered, not opaque");
        assert_eq!(tip.pixel(1, 0), 64);
        assert_eq!(tip.pixel(2, 0), 128);
        assert_eq!(tip.pixel(0, 1), 192);
        assert_eq!(tip.pixel(1, 1), 255, "byte 255 is fully covered");
        assert_eq!(tip.pixel(2, 1), 32);
    }

    /// Version 1 has no magic number and no spacing, and its name sits eight bytes earlier.
    #[test]
    fn a_version_one_tip_decodes_with_the_default_spacing() {
        let tip = BrushTip::from_gbr(V1_GRAY).unwrap();
        assert_eq!((tip.width(), tip.height()), (3, 2));
        assert_eq!(
            tip.name(),
            "old",
            "the name is read from the shorter header"
        );
        assert!(
            (tip.spacing() - 0.10).abs() < 1e-6,
            "version 1 uses GIMP's default spacing, got {}",
            tip.spacing()
        );
        // Same payload as the version 2 file, so the same coverage.
        assert_eq!(tip.pixel(1, 1), 255);
        assert_eq!(tip.pixel(0, 0), 0);
    }

    /// Krita cannot read a small version 1 file. This can.
    ///
    /// Measured: Krita checks `sizeof(GimpBrushHeader)` -- the 28-byte VERSION 2 header -- against the data
    /// length before it has read the version field. A valid 23-byte version 1 file (20-byte header,
    /// one-character name, one pixel) is refused outright because 28 > 23.
    #[test]
    fn a_minimal_version_one_tip_is_readable_although_krita_refuses_it() {
        let data = build(1, 1, 1, 1, 0, "a", &[200]);
        assert_eq!(data.len(), 23, "the file Krita cannot read");
        assert!(
            data.len() < V2_HEADER,
            "and it is shorter than the v2 header Krita checks first"
        );

        let tip = BrushTip::from_gbr(&data).unwrap();
        assert_eq!((tip.width(), tip.height()), (1, 1));
        assert_eq!(tip.name(), "a");
        assert_eq!(tip.pixel(0, 0), 200);
    }

    /// RGBA tips take their coverage from alpha alone.
    #[test]
    fn an_rgba_tip_takes_coverage_from_alpha() {
        let tip = BrushTip::from_gbr(V2_RGBA).unwrap();
        assert_eq!((tip.width(), tip.height()), (3, 2));
        assert_eq!(tip.name(), "colour");
        // Reference: 1.00, 0.50, 0.00, 1.00, 0.25, 0.78 -> alphas 255, 128, 0, 255, 64, 200.
        assert_eq!(tip.pixel(0, 0), 255, "opaque red is fully covered");
        assert_eq!(tip.pixel(1, 0), 128);
        assert_eq!(
            tip.pixel(2, 0),
            0,
            "transparent blue covers nothing, whatever its colour"
        );
        assert_eq!(tip.pixel(0, 1), 255);
        assert_eq!(tip.pixel(1, 1), 64);
        assert_eq!(tip.pixel(2, 1), 200);
    }

    /// Every refusal the reference decoder produced, plus the ones only a bound can catch.
    #[test]
    fn malformed_files_are_refused_with_a_reason() {
        use GbrError::*;
        let cases: Vec<(&str, Vec<u8>, GbrError)> = vec![
            (
                "spacing above 1000",
                build(2, 2, 2, 1, 1001, "x", &[1, 2, 3, 4]),
                SpacingTooLarge { spacing: 1001 },
            ),
            ("zero width", build(2, 0, 2, 1, 10, "x", &[1, 2]), EmptyTip),
            ("zero height", build(2, 2, 0, 1, 10, "x", &[1, 2]), EmptyTip),
            (
                "depth 2",
                build(2, 2, 2, 2, 10, "x", &[1, 2, 3, 4]),
                UnsupportedDepth { bytes: 2 },
            ),
            (
                "payload one byte short",
                build(2, 2, 2, 1, 10, "x", &[1, 2, 3]),
                PayloadTooShort,
            ),
            (
                "rgba payload short",
                build(2, 2, 2, 4, 10, "x", &[0; 15]),
                PayloadTooShort,
            ),
            (
                "version 3, CinePaint's, which may be float16",
                build(3, 2, 2, 1, 10, "x", &[1, 2, 3, 4]),
                UnsupportedVersion { version: 3 },
            ),
            (
                "version 0",
                build(0, 2, 2, 1, 10, "x", &[1, 2, 3, 4]),
                UnsupportedVersion { version: 0 },
            ),
            (
                "edge past the bound",
                build(2, 9999, 1, 1, 10, "x", &[0; 4]),
                EdgeTooLong {
                    edge: 9999,
                    max: MAX_BRUSH_TIP_EDGE,
                },
            ),
        ];
        for (label, data, want) in cases {
            assert_eq!(
                BrushTip::from_gbr(&data).unwrap_err(),
                want,
                "{label} should be refused precisely"
            );
        }

        // Truncated below even the shared header.
        assert_eq!(
            BrushTip::from_gbr(&V2_GRAY[..12]).unwrap_err(),
            TooShort,
            "a 12-byte file has no header at all"
        );
        // A version 2 file truncated between the two header sizes: the version reads, the rest does not.
        assert_eq!(
            BrushTip::from_gbr(&V2_GRAY[..24]).unwrap_err(),
            TooShort,
            "24 bytes holds a v1 header but not a v2 one"
        );

        // header_size of zero, and one running past the end.
        let mut zero_header = build(2, 2, 2, 1, 10, "x", &[1, 2, 3, 4]);
        zero_header[0..4].copy_from_slice(&0u32.to_be_bytes());
        assert_eq!(BrushTip::from_gbr(&zero_header).unwrap_err(), BadHeaderSize);

        let mut huge_header = build(2, 2, 2, 1, 10, "x", &[1, 2, 3, 4]);
        huge_header[0..4].copy_from_slice(&999_999u32.to_be_bytes());
        assert_eq!(BrushTip::from_gbr(&huge_header).unwrap_err(), BadHeaderSize);

        // A header too small to hold even the name's terminator.
        let mut tiny_header = build(2, 2, 2, 1, 10, "x", &[1, 2, 3, 4]);
        tiny_header[0..4].copy_from_slice(&(V2_HEADER as u32).to_be_bytes());
        assert_eq!(BrushTip::from_gbr(&tiny_header).unwrap_err(), BadHeaderSize);
    }

    /// A pixel count past the bound is refused before anything is allocated.
    ///
    /// The dimensions come from the file, so a header declaring 4096 x 4096 asks for 16 megapixels from a
    /// file of forty bytes. The edge bound is checked first so the multiplication cannot overflow on the
    /// way to the check meant to catch it.
    #[test]
    fn an_enormous_declared_tip_is_refused_before_allocating() {
        let data = build(2, 4096, 4096, 1, 10, "big", &[0; 8]);
        assert_eq!(
            BrushTip::from_gbr(&data).unwrap_err(),
            GbrError::TooLarge {
                pixels: 4096 * 4096,
                max: MAX_BRUSH_TIP_PIXELS,
            }
        );

        // And the largest accepted shape is genuinely accepted, given a payload.
        let side = 512u32;
        let ok = build(
            2,
            side,
            side,
            1,
            10,
            "max",
            &vec![128u8; (side * side) as usize],
        );
        let tip = BrushTip::from_gbr(&ok).unwrap();
        assert_eq!(tip.pixel(300, 300), 128);
    }

    /// A version 2 name that is not UTF-8 is refused; a version 1 one is read as Latin-1.
    #[test]
    fn name_encoding_follows_the_version() {
        let mut v2 = build(2, 1, 1, 1, 10, "ab", &[200]);
        // Replace the name bytes with an invalid UTF-8 sequence.
        v2[V2_HEADER] = 0xff;
        v2[V2_HEADER + 1] = 0xfe;
        assert_eq!(BrushTip::from_gbr(&v2).unwrap_err(), GbrError::BadName);

        let mut v1 = build(1, 1, 1, 1, 0, "ab", &[200]);
        v1[V1_HEADER] = 0xe9; // Latin-1 'é'
        let tip = BrushTip::from_gbr(&v1).unwrap();
        assert_eq!(
            tip.name(),
            "éb",
            "version 1's encoding is undefined and Krita reads it as Latin-1"
        );
    }

    /// Spacing at both ends of GIMP's range.
    #[test]
    fn spacing_bounds_are_accepted_as_measured() {
        let zero = BrushTip::from_gbr(&build(2, 1, 1, 1, 0, "s", &[200])).unwrap();
        assert_eq!(zero.spacing(), 0.0);
        let full = BrushTip::from_gbr(&build(2, 1, 1, 1, 1000, "s", &[200])).unwrap();
        assert!(
            (full.spacing() - 10.0).abs() < 1e-6,
            "1000 percent is a spacing of 10, got {}",
            full.spacing()
        );
    }

    /// Sampling a tip scaled to a dab.
    #[test]
    fn a_tip_samples_across_the_dab() {
        // A 4x4 tip: solid in the middle two rows and columns, empty around them.
        let mut payload = vec![0u8; 16];
        for y in 1..3 {
            for x in 1..3 {
                payload[y * 4 + x] = 255;
            }
        }
        let tip = BrushTip::from_gbr(&build(2, 4, 4, 1, 10, "box", &payload)).unwrap();

        // Centre of a 40-pixel dab lands in the solid middle.
        assert!(
            tip.coverage_at(0.0, 0.0, 40.0) > 0.9,
            "the centre should be covered, got {}",
            tip.coverage_at(0.0, 0.0, 40.0)
        );
        // Well outside the dab's box, nothing.
        assert_eq!(tip.coverage_at(40.0, 0.0, 40.0), 0.0);
        assert_eq!(tip.coverage_at(0.0, -40.0, 40.0), 0.0);
        // The corner of the tip is empty, so the dab's corner is too.
        assert!(
            tip.coverage_at(18.0, 18.0, 40.0) < 0.1,
            "the tip's corner is empty, got {}",
            tip.coverage_at(18.0, 18.0, 40.0)
        );
    }

    /// Sampling never produces a value outside the unit range or a NaN, for any tip and any dab.
    #[test]
    fn sampling_is_always_finite_and_bounded() {
        let payload: Vec<u8> = (0..24).map(|i| (i * 11) as u8).collect();
        let tip = BrushTip::from_gbr(&build(2, 6, 4, 1, 10, "ramp", &payload)).unwrap();
        for diameter in [0.5, 1.0, 13.0, 40.0, 1000.0] {
            for step in -60..=60 {
                let offset = step as f32 * 0.9;
                let coverage = tip.coverage_at(offset, offset * 0.5, diameter);
                assert!(
                    coverage.is_finite() && (0.0..=1.0).contains(&coverage),
                    "diameter {diameter} offset {offset} gave {coverage}"
                );
            }
        }
        // A degenerate diameter covers nothing rather than dividing by zero.
        for diameter in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(tip.coverage_at(0.0, 0.0, diameter), 0.0);
        }
    }

    /// A tip arriving from a deserialised command can lie about its own dimensions.
    #[test]
    fn a_tip_whose_coverage_length_disagrees_is_invalid() {
        let good = BrushTip::from_gbr(V2_GRAY).unwrap();
        assert!(good.is_valid());

        let json = serde_json::to_string(&good).unwrap();
        // Shrink the coverage array without touching the declared dimensions.
        let tampered = json.replace("[0,64,128,192,255,32]", "[0,64]");
        assert_ne!(
            tampered, json,
            "the test's own substitution must have applied"
        );
        let bad: BrushTip = serde_json::from_str(&tampered).unwrap();
        assert!(
            !bad.is_valid(),
            "a 3x2 tip carrying two coverage bytes must be refused"
        );
        // And sampling it is still safe.
        assert_eq!(bad.coverage_at(0.0, 0.0, 10.0), 0.0);
    }

    #[test]
    fn the_magic_number_identifies_a_version_two_file() {
        assert!(looks_like_gbr_v2(V2_GRAY));
        assert!(looks_like_gbr_v2(V2_RGBA));
        assert!(
            !looks_like_gbr_v2(V1_GRAY),
            "version 1 carries no magic number"
        );
        assert!(!looks_like_gbr_v2(&[0; 8]));
        assert!(!looks_like_gbr_v2(&[]));
    }
}
