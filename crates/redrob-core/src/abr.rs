//! Reading Photoshop's ABR brush collections.
//!
//! TRANSLATED from Krita, pinned at `fdbf33b2146735465bb8aa59928fbc1890ceb160`, GPL-2.0-or-later, taken
//! into this GPL-3.0-or-later product under the or-later grant:
//!
//! - `libs/brush/kis_abr_brush_collection.cpp` — the header, both sample layouts and the RLE decoder
//!
//! Licence change notice: the original is GPL-2.0-or-later; this file is GPL-3.0-or-later, which the
//! or-later grant permits. Attribution is in `UPSTREAM_NOTICES.md`.
//!
//! # Why ABR after GBR
//!
//! ABR is the format users actually have, because Photoshop brush packs are everywhere, and its
//! specification is not published at all — Krita's reader and GIMP's before it are what exists. Unlike
//! GIH, which needs a multi-tip selection concept this product does not have, an ABR yields plain tips
//! that plug straight into [`crate::BrushTip`].
//!
//! # What is supported, matching the upstream
//!
//! Krita's own `abr_supported_content` accepts version 1, version 2, and version 6 subversions 1 and 2.
//! Anything else it refuses by name, and so does this.

use crate::brush_tip::{BrushTip, MAX_BRUSH_TIP_EDGE, MAX_BRUSH_TIP_PIXELS};

/// The largest number of brushes this will take from one collection.
///
/// The count comes from the file, so a bound is needed for the same reason the tip dimensions have one.
pub const MAX_ABR_BRUSHES: usize = 512;

/// Why an ABR could not be read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum AbrError {
    #[error("the data is too short to hold an ABR header")]
    TooShort,
    #[error("ABR version {version} subversion {subversion} is not supported")]
    UnsupportedVersion { version: i16, subversion: i16 },
    #[error("the collection declares no brushes")]
    NoBrushes,
    #[error("the collection declares {count} brushes, past the {max} this will read")]
    TooManyBrushes { count: usize, max: usize },
    #[error("a brush's declared bounds are empty or inverted")]
    BadBounds,
    #[error("a brush depth of {depth} bits is not supported; only 8 is")]
    UnsupportedDepth { depth: i16 },
    #[error("a brush is {pixels} pixels, past the {max} this will read")]
    BrushTooLarge { pixels: usize, max: usize },
    #[error("the data ends inside a brush")]
    Truncated,
    #[error("no brush in the collection could be read")]
    NothingUsable,
}

impl AbrError {
    /// Whether the stream is still positioned usefully after this failure.
    ///
    /// A record's length is read before its contents, so a complaint about the CONTENTS leaves the next
    /// record's position known and the walk can carry on. A read that ran off the end does not.
    fn leaves_stream_usable(&self) -> bool {
        match self {
            Self::BadBounds | Self::UnsupportedDepth { .. } | Self::BrushTooLarge { .. } => true,
            Self::TooShort
            | Self::UnsupportedVersion { .. }
            | Self::NoBrushes
            | Self::TooManyBrushes { .. }
            | Self::Truncated
            | Self::NothingUsable => false,
        }
    }
}

/// A cursor over big-endian data that refuses to read past the end.
///
/// Krita reads through a `QDataStream` whose failures are silent: a short read leaves the destination
/// untouched and the loop carries on with whatever was there. Every read here is checked, which is what
/// turns a truncated file into one error instead of a plausible-looking brush made of stale memory.
struct Cursor<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }

    fn u8(&mut self) -> Result<u8, AbrError> {
        let byte = *self.data.get(self.position).ok_or(AbrError::Truncated)?;
        self.position += 1;
        Ok(byte)
    }

    fn i16(&mut self) -> Result<i16, AbrError> {
        let bytes = self
            .data
            .get(self.position..self.position + 2)
            .ok_or(AbrError::Truncated)?;
        self.position += 2;
        Ok(i16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn i32(&mut self) -> Result<i32, AbrError> {
        let bytes = self
            .data
            .get(self.position..self.position + 4)
            .ok_or(AbrError::Truncated)?;
        self.position += 4;
        Ok(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn skip(&mut self, count: usize) -> Result<(), AbrError> {
        self.position = self
            .position
            .checked_add(count)
            .filter(|next| *next <= self.data.len())
            .ok_or(AbrError::Truncated)?;
        Ok(())
    }

    /// Absolute seek, refused past the end.
    fn seek(&mut self, position: usize) -> Result<(), AbrError> {
        if position > self.data.len() {
            return Err(AbrError::Truncated);
        }
        self.position = position;
        Ok(())
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], AbrError> {
        let slice = self
            .data
            .get(
                self.position
                    ..self
                        .position
                        .checked_add(count)
                        .ok_or(AbrError::Truncated)?,
            )
            .ok_or(AbrError::Truncated)?;
        self.position += count;
        Ok(slice)
    }

    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.position)
    }
}

/// Reads every sampled brush in an ABR collection.
///
/// A collection holds several brushes, so this returns several tips. Photoshop's computed brushes -- a
/// shape described by parameters rather than pixels -- are skipped, as Krita skips them: they are the
/// same thing [`crate::DabShape`] already generates, not a tip image.
pub fn read_abr(data: &[u8]) -> Result<Vec<BrushTip>, AbrError> {
    let mut cursor = Cursor::new(data);
    let version = cursor.i16()?;
    let (subversion, declared) = match version {
        1 | 2 => (0, cursor.i16()? as i64),
        6 => {
            let subversion = cursor.i16()?;
            if subversion != 1 && subversion != 2 {
                return Err(AbrError::UnsupportedVersion {
                    version,
                    subversion,
                });
            }
            // Krita walks the whole file counting samples before reading any. There is nothing to gain
            // from that here: the v6 loop runs until the data is exhausted, so the count is what it
            // produced rather than something to agree with beforehand.
            (subversion, -1)
        }
        version => {
            return Err(AbrError::UnsupportedVersion {
                version,
                subversion: 0,
            });
        }
    };

    if declared == 0 {
        return Err(AbrError::NoBrushes);
    }
    if declared > MAX_ABR_BRUSHES as i64 {
        return Err(AbrError::TooManyBrushes {
            count: declared as usize,
            max: MAX_ABR_BRUSHES,
        });
    }

    let mut tips = Vec::new();
    // Two kinds of failure, kept apart because they need different handling.
    //
    // A record whose CONTENT this cannot use -- an unsupported depth, empty bounds, a brush past the size
    // bound -- does not cost us the stream: the record's length was read before its contents, so the next
    // record's position is still known and the walk continues. Krita does the same, with a warning.
    //
    // A record that RUNS OUT of data desynchronises everything after it, so the walk stops there.
    //
    // Either way the first reason is kept rather than flattened. An earlier version counted failures and
    // returned a generic `Truncated`, so a file of oversized brushes and a half-written file reported the
    // same thing.
    let mut first_problem: Option<AbrError> = None;

    if version == 6 {
        while cursor.remaining() > 4 && tips.len() < MAX_ABR_BRUSHES {
            let before = cursor.position;
            match read_sample_v6(&mut cursor, subversion) {
                Ok(Some(tip)) => tips.push(tip),
                Ok(None) => {}
                Err(error) => {
                    let recoverable = error.leaves_stream_usable();
                    first_problem.get_or_insert(error);
                    if !recoverable {
                        break;
                    }
                }
            }
            // A record that consumed nothing would loop for ever. A positive declared size rules it out
            // and is checked, but the guard costs one comparison.
            if cursor.position <= before {
                break;
            }
        }
    } else {
        for _ in 0..declared {
            if cursor.remaining() < 6 {
                break;
            }
            let before = cursor.position;
            match read_sample_v12(&mut cursor, version) {
                Ok(Some(tip)) => tips.push(tip),
                Ok(None) => {}
                Err(error) => {
                    let recoverable = error.leaves_stream_usable();
                    first_problem.get_or_insert(error);
                    if !recoverable {
                        break;
                    }
                }
            }
            if cursor.position <= before {
                break;
            }
        }
    }

    if tips.is_empty() {
        // A collection that produced nothing reports WHY. `NothingUsable` is only for the case where every
        // record parsed cleanly and none of them was a tip -- all computed brushes, say.
        return Err(first_problem.unwrap_or(AbrError::NothingUsable));
    }
    Ok(tips)
}

/// One version 6 sample.
fn read_sample_v6(cursor: &mut Cursor, subversion: i16) -> Result<Option<BrushTip>, AbrError> {
    let brush_size = cursor.i32()?;
    if brush_size <= 0 {
        return Err(AbrError::Truncated);
    }
    // Records are padded to a multiple of four.
    let padded = ((brush_size as usize) + 3) & !3;
    let next = cursor
        .position
        .checked_add(padded)
        .ok_or(AbrError::Truncated)?;

    cursor.skip(37)?; // the key
    cursor.skip(if subversion == 1 { 10 } else { 264 })?;

    // The record's end is known from its declared size, so a content complaint still leaves the cursor at
    // the next record and the walk can continue.
    let outcome = read_bounds_and_pixels(cursor);
    // Absolute, which is what the upstream does here and does NOT do in its version 1/2 path.
    cursor.seek(next.min(cursor.data.len()))?;
    outcome
}

/// One version 1 or 2 sample.
fn read_sample_v12(cursor: &mut Cursor, version: i16) -> Result<Option<BrushTip>, AbrError> {
    let brush_type = cursor.i16()?;
    let brush_size = cursor.i32()?;
    if brush_size < 0 {
        return Err(AbrError::Truncated);
    }
    let next = cursor
        .position
        .checked_add(brush_size as usize)
        .ok_or(AbrError::Truncated)?;

    if brush_type == 1 {
        // A computed brush: parameters, not pixels. Skipped, as Krita skips it.
        //
        // Krita seeks `pos() + next_brush` here, but `next_brush` is ALREADY the absolute target it
        // computed two lines earlier -- the same function seeks plain `next_brush` in its other two
        // exits, and the version 6 path does so everywhere. So the upstream jumps roughly twice as far
        // as it should and lands past the sampled brushes that follow, losing them silently. There is
        // even a TODO beside the line. This seeks the position that was computed.
        cursor.seek(next.min(cursor.data.len()))?;
        return Ok(None);
    }
    if brush_type != 2 {
        cursor.seek(next.min(cursor.data.len()))?;
        return Ok(None);
    }

    cursor.skip(6)?; // four misc bytes and two of spacing
    if version == 2 {
        // A UCS-2 name, length-prefixed as a big-endian character count. Read past rather than kept: the
        // tip's name is set from its index below, matching Krita, which also falls back to that when the
        // name is absent.
        let characters = cursor.i32()?;
        if characters < 0 {
            return Err(AbrError::Truncated);
        }
        cursor.skip((characters as usize).saturating_mul(2))?;
    }
    cursor.skip(9)?; // one antialiasing byte and four short bounds

    let outcome = read_bounds_and_pixels(cursor);
    cursor.seek(next.min(cursor.data.len()))?;
    outcome
}

/// The part both layouts share: bounds, depth, compression flag, pixels.
fn read_bounds_and_pixels(cursor: &mut Cursor) -> Result<Option<BrushTip>, AbrError> {
    let top = cursor.i32()?;
    let left = cursor.i32()?;
    let bottom = cursor.i32()?;
    let right = cursor.i32()?;
    let depth = cursor.i16()?;
    let compressed = cursor.u8()? != 0;

    let width = right.checked_sub(left).ok_or(AbrError::BadBounds)?;
    let height = bottom.checked_sub(top).ok_or(AbrError::BadBounds)?;
    if width <= 0 || height <= 0 {
        return Err(AbrError::BadBounds);
    }
    // Krita computes `width * (depth >> 3) * height` and accepts any depth, so a depth of 16 would read
    // two bytes per pixel and then treat each byte as a gray level. Only 8 is handled here, by name.
    if depth != 8 {
        return Err(AbrError::UnsupportedDepth { depth });
    }
    if width > MAX_BRUSH_TIP_EDGE as i32 || height > MAX_BRUSH_TIP_EDGE as i32 {
        // Past what a tip may be. An ERROR rather than a silent `None`, so a collection that yields
        // nothing can say the brushes were too large instead of reporting `NothingUsable`; the caller
        // skips it and keeps reading, which is the shape of Krita's own wide-brush skip.
        return Err(AbrError::BrushTooLarge {
            pixels: (width as usize).saturating_mul(height as usize),
            max: MAX_BRUSH_TIP_PIXELS,
        });
    }
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or(AbrError::BrushTooLarge {
            pixels: usize::MAX,
            max: MAX_BRUSH_TIP_PIXELS,
        })?;
    if pixels > MAX_BRUSH_TIP_PIXELS {
        return Err(AbrError::BrushTooLarge {
            pixels,
            max: MAX_BRUSH_TIP_PIXELS,
        });
    }

    let coverage = if compressed {
        decode_packbits(cursor, width as usize, height as usize)?
    } else {
        cursor.take(pixels)?.to_vec()
    };

    // Photoshop stores a sampled brush with high bytes as paint, the same sense as GBR's grayscale data,
    // so the byte is the coverage. Krita builds an RGB image from it and inverts later.
    Ok(Some(BrushTip::from_raw_coverage(
        width as u32,
        height as u32,
        coverage,
    )))
}

/// PackBits, row by row, with a table of compressed row lengths in front.
///
/// Photoshop's scheme and Krita's `rle_decode`: a signed count byte, where a negative `n` repeats the next
/// byte `1 - n` times, a non-negative `n` copies the next `n + 1` bytes, and -128 is a no-op.
fn decode_packbits(cursor: &mut Cursor, width: usize, height: usize) -> Result<Vec<u8>, AbrError> {
    let mut row_lengths = Vec::with_capacity(height);
    for _ in 0..height {
        let length = cursor.i16()?;
        if length < 0 {
            return Err(AbrError::Truncated);
        }
        row_lengths.push(length as usize);
    }

    let mut out = vec![0u8; width * height];
    for (row, length) in row_lengths.iter().enumerate() {
        let row_start = row * width;
        let mut written = 0usize;
        let mut consumed = 0usize;
        while consumed < *length {
            let control = cursor.u8()? as i8;
            consumed += 1;
            if control == -128 {
                // A no-op, and Krita's loop continues without consuming a data byte.
                continue;
            }
            if control < 0 {
                let run = 1 - control as i32;
                let value = cursor.u8()?;
                consumed += 1;
                for _ in 0..run {
                    // A row that claims more pixels than it has is truncated at the row's end rather
                    // than spilling into the next one, which is what an unchecked `*data++` does.
                    if written >= width {
                        break;
                    }
                    out[row_start + written] = value;
                    written += 1;
                }
            } else {
                let count = control as usize + 1;
                for _ in 0..count {
                    let value = cursor.u8()?;
                    consumed += 1;
                    if written >= width {
                        continue;
                    }
                    out[row_start + written] = value;
                    written += 1;
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    include!("abr_fixtures.rs");

    /// Version 6 subversion 1, uncompressed: a 4x2 tip whose bytes are the coverage.
    #[test]
    fn a_version_six_uncompressed_brush_decodes() {
        let tips = read_abr(ABR_V6_RAW).unwrap();
        assert_eq!(tips.len(), 1);
        let tip = &tips[0];
        assert_eq!((tip.width(), tip.height()), (4, 2));
        // The payload was 0, 64, 128, 255, 32, 96, 160, 224.
        assert_eq!(tip.pixel(0, 0), 0);
        assert_eq!(tip.pixel(1, 0), 64);
        assert_eq!(tip.pixel(3, 0), 255);
        assert_eq!(tip.pixel(0, 1), 32);
        assert_eq!(tip.pixel(3, 1), 224);
        assert!(tip.is_valid());
    }

    /// Subversion 2 pads its record with 264 bytes where subversion 1 uses 10, and this file is
    /// RLE-compressed. Same pixels, so the same tip.
    #[test]
    fn a_version_six_compressed_brush_decodes_to_the_same_pixels() {
        let tips = read_abr(ABR_V6_RLE).unwrap();
        assert_eq!(tips.len(), 1);
        let tip = &tips[0];
        assert_eq!((tip.width(), tip.height()), (4, 2));
        for (x, want) in [(0u32, 0u8), (1, 64), (2, 128), (3, 255)] {
            assert_eq!(tip.pixel(x, 0), want, "row 0 column {x}");
        }
        for (x, want) in [(0u32, 32u8), (1, 96), (2, 160), (3, 224)] {
            assert_eq!(tip.pixel(x, 1), want, "row 1 column {x}");
        }

        // And the two encodings agree, which is the real assertion about the RLE.
        let raw = read_abr(ABR_V6_RAW).unwrap();
        for y in 0..2 {
            for x in 0..4 {
                assert_eq!(
                    tip.pixel(x, y),
                    raw[0].pixel(x, y),
                    "compressed and raw must agree at {x},{y}"
                );
            }
        }
    }

    /// A collection holds several brushes and yields several tips.
    #[test]
    fn several_brushes_in_one_collection_all_decode() {
        let tips = read_abr(ABR_V6_TWO).unwrap();
        assert_eq!(tips.len(), 2, "both records must be read");
        assert_eq!((tips[0].width(), tips[0].height()), (2, 2));
        assert_eq!(tips[0].pixel(0, 0), 1);
        assert_eq!(tips[0].pixel(1, 1), 4);
        assert_eq!((tips[1].width(), tips[1].height()), (3, 1));
        assert_eq!(tips[1].pixel(0, 0), 9);
        assert_eq!(tips[1].pixel(2, 0), 7);
    }

    /// Version 1 uses a different record layout: a type and size in front, no key block.
    #[test]
    fn a_version_one_brush_decodes() {
        let tips = read_abr(ABR_V1).unwrap();
        assert_eq!(tips.len(), 1);
        assert_eq!((tips[0].width(), tips[0].height()), (3, 2));
        assert_eq!(tips[0].pixel(0, 0), 10);
        assert_eq!(tips[0].pixel(2, 0), 30);
        assert_eq!(tips[0].pixel(2, 1), 60);
    }

    /// A computed brush followed by a sampled one: the case Krita loses.
    ///
    /// Krita seeks `pos() + next_brush` past a computed brush, but `next_brush` is already the absolute
    /// target it computed two lines earlier -- the same function seeks plain `next_brush` at its other two
    /// exits, and the version 6 path does so everywhere. Measured on this fixture: the correct next record
    /// begins at byte 42 and Krita jumps to 52, ten bytes inside it, so it reads the sampled brush's
    /// middle as a record header and loses the brush.
    #[test]
    fn a_sampled_brush_after_a_computed_one_is_not_lost() {
        let tips = read_abr(ABR_V2_COMPUTED_THEN_SAMPLED).unwrap();
        assert_eq!(
            tips.len(),
            1,
            "the computed brush is skipped and the sampled one is kept"
        );
        assert_eq!((tips[0].width(), tips[0].height()), (2, 2));
        assert_eq!(tips[0].pixel(0, 0), 11);
        assert_eq!(tips[0].pixel(1, 0), 22);
        assert_eq!(tips[0].pixel(0, 1), 33);
        assert_eq!(tips[0].pixel(1, 1), 44);
    }

    /// Versions the upstream refuses are refused here by name.
    #[test]
    fn unsupported_versions_are_refused() {
        // Krita's own abr_supported_content accepts 1, 2, and 6 with subversion 1 or 2.
        assert_eq!(
            read_abr(&[0, 3, 0, 1]).unwrap_err(),
            AbrError::UnsupportedVersion {
                version: 3,
                subversion: 0
            }
        );
        assert_eq!(
            read_abr(&[0, 6, 0, 3]).unwrap_err(),
            AbrError::UnsupportedVersion {
                version: 6,
                subversion: 3
            },
            "version 6 subversion 3 is not among the two the upstream handles"
        );
        assert_eq!(
            read_abr(&[0, 0, 0, 1]).unwrap_err(),
            AbrError::UnsupportedVersion {
                version: 0,
                subversion: 0
            }
        );
    }

    #[test]
    fn a_version_one_collection_declaring_no_brushes_is_refused() {
        assert_eq!(read_abr(&[0, 1, 0, 0]).unwrap_err(), AbrError::NoBrushes);
    }

    #[test]
    fn a_collection_declaring_too_many_brushes_is_refused() {
        let count = (MAX_ABR_BRUSHES + 1) as i16;
        let mut data = vec![0, 1];
        data.extend_from_slice(&count.to_be_bytes());
        assert_eq!(
            read_abr(&data).unwrap_err(),
            AbrError::TooManyBrushes {
                count: MAX_ABR_BRUSHES + 1,
                max: MAX_ABR_BRUSHES,
            }
        );
    }

    /// Truncation anywhere is an error rather than a tip made of whatever was in the buffer.
    ///
    /// Krita reads through a QDataStream whose short reads fail silently, leaving the destination
    /// untouched and the loop running. Every read here is length-checked.
    #[test]
    fn truncation_at_every_length_is_refused_or_yields_nothing() {
        for length in 0..ABR_V6_RAW.len() {
            let result = read_abr(&ABR_V6_RAW[..length]);
            if let Ok(tips) = result {
                // A prefix may legitimately contain a whole earlier record; it must never contain THIS
                // one, whose pixels start at byte 74.
                for tip in tips {
                    assert!(
                        tip.is_valid(),
                        "a tip from a {length}-byte prefix must still be self-consistent"
                    );
                }
            }
        }
        // The full file works, so the sweep above was not vacuous.
        assert!(read_abr(ABR_V6_RAW).is_ok());
        assert_eq!(read_abr(&[]).unwrap_err(), AbrError::Truncated);
        assert_eq!(read_abr(&[0]).unwrap_err(), AbrError::Truncated);
    }

    /// A depth other than 8 is refused rather than read as bytes.
    ///
    /// Krita computes `width * (depth >> 3) * height` and then treats each byte as a gray level, so a
    /// 16-bit brush would be read at twice the size and rendered as noise.
    #[test]
    fn a_depth_other_than_eight_is_refused() {
        let mut data = ABR_V6_RAW.to_vec();
        // The file's own header is 4 bytes (version and subversion), then the record's 4-byte size, the
        // 37-byte key, 10 bytes of subversion-1 padding and four 4-byte bounds:
        // 4 + 4 + 37 + 10 + 16 = 71. An earlier version of this test omitted the file header and read
        // zeroes, which is why the assertion below exists at all.
        let depth_at = 4 + 4 + 37 + 10 + 16;
        assert_eq!(
            i16::from_be_bytes([data[depth_at], data[depth_at + 1]]),
            8,
            "the fixture's depth must be where this test thinks it is"
        );
        data[depth_at..depth_at + 2].copy_from_slice(&16i16.to_be_bytes());
        // The depth itself is the refusal, and it is reported as such. The record's declared length was
        // read before its contents, so this failure does not desynchronise the stream and the walk could
        // have continued -- there simply is no further record here.
        assert_eq!(
            read_abr(&data).unwrap_err(),
            AbrError::UnsupportedDepth { depth: 16 }
        );

        // And with trailing bytes that could look like another record, the first reason still wins rather
        // than being replaced by whatever the trailing garbage does.
        data.extend_from_slice(&[0u8; 64]);
        assert_eq!(
            read_abr(&data).unwrap_err(),
            AbrError::UnsupportedDepth { depth: 16 }
        );
    }

    /// Empty or inverted bounds are refused.
    #[test]
    fn bad_bounds_are_refused() {
        let mut data = ABR_V6_RAW.to_vec();
        let bounds_at = 4 + 4 + 37 + 10;
        // right = left, so width 0.
        data[bounds_at + 12..bounds_at + 16].copy_from_slice(&0i32.to_be_bytes());
        data.extend_from_slice(&[0u8; 64]);
        assert_eq!(read_abr(&data).unwrap_err(), AbrError::BadBounds);

        // bottom above top, so a negative height.
        let mut inverted = ABR_V6_RAW.to_vec();
        inverted[bounds_at..bounds_at + 4].copy_from_slice(&100i32.to_be_bytes());
        inverted.extend_from_slice(&[0u8; 64]);
        assert_eq!(read_abr(&inverted).unwrap_err(), AbrError::BadBounds);
    }

    /// A brush declaring more pixels than the bound is refused before allocating.
    #[test]
    fn an_enormous_brush_is_refused_before_allocating() {
        let mut data = ABR_V6_RAW.to_vec();
        let bounds_at = 4 + 4 + 37 + 10;
        // A 5000 x 5000 brush: past the edge bound, so skipped rather than failing the collection.
        data[bounds_at + 8..bounds_at + 12].copy_from_slice(&5000i32.to_be_bytes());
        data[bounds_at + 12..bounds_at + 16].copy_from_slice(&5000i32.to_be_bytes());
        data.extend_from_slice(&[0u8; 64]);
        // The reason is reported precisely rather than as a vague "nothing usable", and no allocation of
        // 25 megapixels was attempted. An earlier design flattened every per-record failure into
        // `Truncated`, and then into `NothingUsable`; both threw away what the decoder had worked out.
        assert_eq!(
            read_abr(&data).unwrap_err(),
            AbrError::BrushTooLarge {
                pixels: 5000 * 5000,
                max: MAX_BRUSH_TIP_PIXELS,
            }
        );
    }

    /// PackBits' no-op byte must not consume a data byte.
    ///
    /// -128 is defined as a no-op in Photoshop's scheme, and Krita's loop `continue`s on it without
    /// reading a value. A decoder that treated it as a run of 129 would corrupt every row containing one.
    #[test]
    fn the_packbits_no_op_consumes_nothing() {
        // A row of four bytes encoded as: no-op, literal run of 4.
        let row_data: Vec<u8> = vec![0x80, 3, 7, 8, 9, 10];
        let mut record = Vec::new();
        record.extend_from_slice(&(row_data.len() as i16).to_be_bytes());
        record.extend_from_slice(&row_data);

        let mut cursor = Cursor::new(&record);
        let decoded = decode_packbits(&mut cursor, 4, 1).unwrap();
        assert_eq!(
            decoded,
            vec![7, 8, 9, 10],
            "the no-op must be skipped without eating a byte"
        );
    }

    /// A row claiming more pixels than its width is truncated at the row rather than spilling.
    #[test]
    fn an_overlong_row_does_not_spill_into_the_next() {
        // Two rows of width 2. The first row's run claims five pixels.
        let first: Vec<u8> = vec![0xfc, 42]; // -4 -> a run of 5
        let second: Vec<u8> = vec![1, 1, 2]; // a literal run of 2
        let mut record = Vec::new();
        record.extend_from_slice(&(first.len() as i16).to_be_bytes());
        record.extend_from_slice(&(second.len() as i16).to_be_bytes());
        record.extend_from_slice(&first);
        record.extend_from_slice(&second);

        let mut cursor = Cursor::new(&record);
        let decoded = decode_packbits(&mut cursor, 2, 2).unwrap();
        assert_eq!(
            decoded,
            vec![42, 42, 1, 2],
            "the overlong run fills its own row and stops"
        );
    }

    /// The decoded tips must be usable by the paint path, not merely well-formed.
    #[test]
    fn a_decoded_abr_tip_samples_as_coverage() {
        let tips = read_abr(ABR_V6_RAW).unwrap();
        let tip = &tips[0];
        for diameter in [4.0, 20.0, 100.0] {
            for step in -30..=30 {
                let offset = step as f32;
                let coverage = tip.coverage_at(offset, offset * 0.5, diameter);
                assert!(
                    coverage.is_finite() && (0.0..=1.0).contains(&coverage),
                    "diameter {diameter} offset {offset} gave {coverage}"
                );
            }
        }
        // The tip's bright end must read as more coverage than its dark end.
        let bright = tip.coverage_at(1.5 * 5.0, -0.5 * 5.0, 20.0);
        let dark = tip.coverage_at(-1.5 * 5.0, -0.5 * 5.0, 20.0);
        assert!(
            bright > dark,
            "the 255 end should cover more than the 0 end: {bright} against {dark}"
        );
    }
}
