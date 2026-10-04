// SPDX-License-Identifier: GPL-3.0-or-later

//! GIMP patterns: decoding the `.pat` format (M.10).
//!
//! Re-derived from `app/core/gimppattern-load.c` and `app/core/gimppattern-header.h`
//! (GPL-3.0-or-later), pinned in `docs/upstream-sources.toml`.
//!
//! **A pattern is a RESOURCE, not a document**, which is why this is not a `FileFormat` variant and
//! why `audit6-format-gap.py` excludes `pat` from the format gap by name. It mirrors
//! `crate::brush_tip`, which does the same job for GIMP's `.gbr` brushes — and unlike that one, this
//! has better provenance: GBR's specification is thin enough that Krita's reader had to stand in for
//! it, whereas `.pat` is defined by the vendored upstream's own header and loader.
//!
//! # The header, every field MSB — upstream's own comment says so
//!
//! | offset | field          | notes                                                        |
//! |--------|----------------|--------------------------------------------------------------|
//! | 0      | `header_size`  | 24 + the name's bytes; **must be > 24**                      |
//! | 4      | `version`      | must be exactly **1**                                        |
//! | 8      | `width`        | 1 ..= 10000                                                  |
//! | 12     | `height`       | 1 ..= 10000                                                  |
//! | 16     | `bytes`        | depth in **BYTES**, 1 ..= 4                                  |
//! | 20     | `magic_number` | **`GPAT`** — and note the offset                             |
//! | 24     | name           | NUL-terminated, at most 256 bytes                            |
//! | …      | data           | `width * height * bytes`                                     |
//!
//! **THE MAGIC IS AT OFFSET 20, NOT 0**, because `header_size` is the first field. That makes this
//! the second format in group M whose signature is not at the start — M.2's TGA keeps its at the
//! very END — and it is the fact a sniff gets wrong by assuming.
//!
//! **A NAME IS MANDATORY.** Upstream rejects `header_size <= sizeof (header)`, so a bare 24-byte
//! header with no name is refused even though every other field may be valid. The smallest legal
//! `header_size` is 25: one NUL.
//!
//! **`bytes` counts BYTES**, which is the fourth spelling of depth this group has met — SGI's `bpp`
//! is bytes, SUN raster's `depth` is bits, XBM takes its word size from a C type, and this is bytes
//! again. Upstream's own error message says what the range means: *"GIMP Patterns must be GRAY or
//! RGB"*, so 1 is grey, 2 grey-with-alpha, 3 RGB, 4 RGBA.

use serde::{Deserialize, Serialize};

/// `GIMP_PATTERN_MAGIC`, which upstream spells as a shifted character sum:
/// `('G' << 24) + ('P' << 16) + ('A' << 8) + ('T' << 0)`.
const PAT_MAGIC: u32 = u32::from_be_bytes(*b"GPAT");

/// Where that magic actually sits. Named rather than inlined because the whole point is that it is
/// not zero.
const MAGIC_OFFSET: usize = 20;

/// The fixed part of the header, `sizeof (GimpPatternHeader)`.
const HEADER_SIZE: usize = 24;

/// `GIMP_PATTERN_MAX_SIZE` — upstream's own ceiling in either dimension.
pub const MAX_PATTERN_EDGE: u32 = 10_000;

/// `GIMP_PATTERN_MAX_NAME`.
pub const MAX_PATTERN_NAME: usize = 256;

/// This product's own allocation ceiling, which upstream does not have.
///
/// Upstream's dimension check permits 10000 x 10000 x 4, which is 400 MB from a 24-byte header. The
/// dimensions are attacker-controlled, so a bound is needed on top of the ones upstream performs —
/// the same reasoning `MAX_BRUSH_TIP_PIXELS` carries in `crate::brush_tip`.
pub const MAX_PATTERN_PIXELS: usize = 4096 * 4096;

/// A decoded pattern: RGBA, plus the name the file carried.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pattern {
    width: u32,
    height: u32,
    name: String,
    rgba: Vec<u8>,
}

/// Why a `.pat` was refused.
///
/// One variant per check upstream performs, so a refusal says which rule it broke rather than
/// collapsing into "malformed".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatError {
    /// Fewer than 24 bytes, so the header itself is incomplete.
    TooShort,
    /// `magic_number` at offset 20 was not `GPAT`.
    NotAPattern,
    /// `version` was not 1.
    UnsupportedVersion(u32),
    /// `header_size <= 24`: no room for the mandatory name.
    MissingName,
    /// The name was longer than `GIMP_PATTERN_MAX_NAME`.
    NameTooLong(usize),
    /// `bytes` outside 1..=4 — upstream: "GIMP Patterns must be GRAY or RGB".
    UnsupportedDepth(u32),
    /// A dimension was zero or above `GIMP_PATTERN_MAX_SIZE`.
    InvalidDimensions { width: u32, height: u32 },
    /// Within upstream's limits but beyond this product's allocation ceiling.
    TooLarge { pixels: usize },
    /// The declared pixel data was not all there.
    Truncated,
}

impl std::fmt::Display for PatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooShort => write!(f, "pattern file is shorter than its 24-byte header"),
            Self::NotAPattern => write!(f, "no GPAT magic at offset 20"),
            Self::UnsupportedVersion(version) => {
                write!(f, "unknown pattern format version {version}")
            }
            Self::MissingName => write!(f, "pattern header leaves no room for a name"),
            Self::NameTooLong(len) => write!(f, "pattern name is too long: {len}"),
            Self::UnsupportedDepth(bytes) => {
                write!(f, "unsupported pattern depth {bytes}: must be GRAY or RGB")
            }
            Self::InvalidDimensions { width, height } => {
                write!(f, "invalid pattern dimensions {width}x{height}")
            }
            Self::TooLarge { pixels } => write!(f, "pattern of {pixels} pixels is too large"),
            Self::Truncated => write!(f, "pattern pixel data is truncated"),
        }
    }
}

impl std::error::Error for PatError {}

/// Is this a `.pat`?
///
/// Reads the magic at **offset 20**, so it needs the whole fixed header present. A sniff looking at
/// offset 0 would find `header_size` and claim nothing.
pub fn looks_like_pat(data: &[u8]) -> bool {
    data.len() >= HEADER_SIZE
        && u32::from_be_bytes([
            data[MAGIC_OFFSET],
            data[MAGIC_OFFSET + 1],
            data[MAGIC_OFFSET + 2],
            data[MAGIC_OFFSET + 3],
        ]) == PAT_MAGIC
}

fn be32(data: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]])
}

impl Pattern {
    /// Decode a `.pat`, performing every check upstream performs and one it does not.
    pub fn from_pat(data: &[u8]) -> Result<Self, PatError> {
        if data.len() < HEADER_SIZE {
            return Err(PatError::TooShort);
        }
        // Offset 20, not 0.
        if !looks_like_pat(data) {
            return Err(PatError::NotAPattern);
        }
        let header_size = be32(data, 0) as usize;
        let version = be32(data, 4);
        let width = be32(data, 8);
        let height = be32(data, 12);
        let depth = be32(data, 16);

        if version != 1 {
            return Err(PatError::UnsupportedVersion(version));
        }
        // Upstream: `header.header_size <= sizeof (header)` is an error, so a name is mandatory.
        if header_size <= HEADER_SIZE {
            return Err(PatError::MissingName);
        }
        if !(1..=4).contains(&depth) {
            return Err(PatError::UnsupportedDepth(depth));
        }
        if width == 0 || width > MAX_PATTERN_EDGE || height == 0 || height > MAX_PATTERN_EDGE {
            return Err(PatError::InvalidDimensions { width, height });
        }

        let name_len = header_size - HEADER_SIZE;
        if name_len > MAX_PATTERN_NAME {
            return Err(PatError::NameTooLong(name_len));
        }

        let pixels = (width as usize) * (height as usize);
        if pixels > MAX_PATTERN_PIXELS {
            return Err(PatError::TooLarge { pixels });
        }

        let name_bytes = data
            .get(HEADER_SIZE..HEADER_SIZE + name_len)
            .ok_or(PatError::Truncated)?;
        // Upstream converts `bn_size - 1` bytes, dropping the trailing NUL rather than carrying it
        // into the string.
        let name = String::from_utf8_lossy(&name_bytes[..name_len.saturating_sub(1)]).into_owned();

        let body_at = HEADER_SIZE + name_len;
        let needed = pixels * depth as usize;
        let body = data
            .get(body_at..body_at + needed)
            .ok_or(PatError::Truncated)?;

        let mut rgba = Vec::with_capacity(pixels * 4);
        for sample in body.chunks_exact(depth as usize) {
            let [r, g, b, a] = match depth {
                1 => [sample[0], sample[0], sample[0], 255],
                2 => [sample[0], sample[0], sample[0], sample[1]],
                3 => [sample[0], sample[1], sample[2], 255],
                _ => [sample[0], sample[1], sample[2], sample[3]],
            };
            rgba.extend_from_slice(&[r, g, b, a]);
        }

        Ok(Self {
            width,
            height,
            name,
            rgba,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// The name the file carried, with its terminating NUL dropped.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The pattern as RGBA, row-major from the top.
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// The pixel at `(x, y)`, which is what a tiling fill needs.
    ///
    /// Coordinates WRAP, because that is what a pattern is for: filling an area larger than itself.
    /// Returning an `Option` or clamping would both make every caller handle a case the format
    /// already answers.
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let x = (x % self.width) as usize;
        let y = (y % self.height) as usize;
        let at = (y * self.width as usize + x) * 4;
        [
            self.rgba[at],
            self.rgba[at + 1],
            self.rgba[at + 2],
            self.rgba[at + 3],
        ]
    }
}
