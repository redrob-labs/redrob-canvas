// SPDX-License-Identifier: GPL-3.0-or-later

//! Paint Shop Pro `.psp` / `.pspimage` — the CONTAINER (M.9a).
//!
//! Re-derived from `plug-ins/common/file-psp.c` in the pinned GIMP tree. That file is **3555 lines
//! with 20 block types**, so this module deliberately stops at the container: the signature, the
//! version, the block walk, and the General Image Attributes block that describes the image. The
//! composite image (M.9b), layers (M.9c) and palettes with LZ77 (M.9d) are separate items.
//!
//! Reading the container is not a stub. Every refusal below is a refusal upstream also makes, at
//! the same byte, which means a file this module accepts is one whose geometry and colour model are
//! known — and a file it rejects is rejected for a reason that can be named.
//!
//! **THE BLOCK HEADER CHANGES SIZE WITH THE VERSION, and that is the fact a reader gets wrong.**
//! Upstream reads `~BK\0` then a `u16` id then a `u32` length, and then a SECOND `u32` only when
//! `psp_ver_major < 4`:
//!
//! | version | header | the length field means |
//! |---|---|---|
//! | 3 | **14 bytes** | initial chunk length, then total length |
//! | 4 and later | **10 bytes** | total length; the initial-chunk field was DROPPED |
//!
//! Upstream's own comment: *"Version 4.0 seems to have dropped the initial data chunk length
//! field."* It then sets that variable to `0xDEADBEEF` — *"Intentionally bogus, should not be
//! used"* — which is upstream telling a porter, in code, that the field must not be consulted at
//! version 4. This module does not model it at all past version 3, so there is nothing to misuse.

use crate::document::BlendMode;
use crate::formats::FormatError;

/// The 32-byte signature at offset 0: the sentence, a newline, `0x1A`, then **five** NUL bytes.
/// Twenty-five characters of text and seven of padding, which is why the constant is written out
/// rather than computed — a reader counting NULs by eye gets five or six about equally often.
pub const PSP_SIGNATURE: &[u8; 32] = b"Paint Shop Pro Image File\n\x1a\0\0\0\0\0";

/// Every block carries this before its id and length.
const BLOCK_SIGNATURE: &[u8; 4] = b"~BK\0";

/// The General Image Attributes block, which must be the FIRST block in the file.
const PSP_IMAGE_BLOCK: u16 = 0;

/// Upstream refuses both lengths below this before reading any field.
const MIN_ATTRIBUTE_CHUNK: u32 = 38;

/// At version 4 and later a second length lives INSIDE the chunk, and upstream requires this of it.
/// Larger than the outer minimum because the chunk grew a `graphics_content` field.
const MIN_ATTRIBUTE_CHUNK_V4: u32 = 46;

/// How the pixel data is stored. `Jpeg` is in upstream's enumeration but is **not a legal value
/// here** — see [`PspCompression::from_u16`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PspCompression {
    None,
    Rle,
    Lz77,
}

impl PspCompression {
    /// Upstream rejects anything above `PSP_COMP_LZ77` with *"Unknown compression type"*.
    ///
    /// **So compression 3, `PSP_COMP_JPEG`, is refused for the image even though the format defines
    /// it.** Upstream's own enumeration comment says why: JPEG is *"only used by thumbnail and
    /// composite image"*. It is a legal value in those two blocks and an illegal one here, so the
    /// same number means different things in different blocks — which is exactly the kind of fact
    /// a single shared "parse compression" helper would erase.
    fn from_u16(value: u16) -> Result<Self, FormatError> {
        match value {
            0 => Ok(Self::None),
            1 => Ok(Self::Rle),
            2 => Ok(Self::Lz77),
            _ => Err(FormatError::UnsupportedFeature(
                "PSP image compression must be none, RLE or LZ77; JPEG is only legal in the thumbnail and composite blocks",
            )),
        }
    }
}

/// What the samples mean. Named for the file's model, not ours, because the mapping to our own
/// layer types is M.9b's decision and naming it here would pre-empt it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PspColourModel {
    Rgb,
    Gray,
    Indexed,
}

/// Where a block sits in the file. M.9b onwards needs the ranges; M.9a proves the walk reaches the
/// end of the file exactly, which is the only evidence that the lengths were read correctly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PspBlock {
    pub id: u16,
    /// Offset of the block's data, just past its header.
    pub start: usize,
    /// Length of the block's data, from the header's total-length field.
    pub len: usize,
}

/// Everything the container says about the image.
#[derive(Debug, Clone, PartialEq)]
pub struct PspContainer {
    pub version_major: u16,
    pub version_minor: u16,
    pub width: u32,
    pub height: u32,
    /// **Always per inch.** The file stores a `f64` plus a one-byte metric, and a metric of
    /// centimetres means upstream divides by 2.54. Converting here rather than exposing the raw
    /// pair means a caller cannot forget the division — the bug that produces an image 2.54 times
    /// the right physical size.
    pub resolution_per_inch: f64,
    pub compression: PspCompression,
    pub bit_depth: u16,
    pub grayscale: bool,
    pub colour_model: PspColourModel,
    pub bytes_per_sample: u8,
    pub active_layer: u32,
    pub layer_count: u16,
    pub blocks: Vec<PspBlock>,
}

/// A metric of centimetres, from upstream's `PSP_METRIC` enumeration.
const PSP_METRIC_CM: u8 = 2;

fn le_u16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn le_u32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// Whether `bytes` opens with the PSP signature. Length-safe on any input.
pub fn is_psp(bytes: &[u8]) -> bool {
    bytes.len() >= PSP_SIGNATURE.len() && &bytes[..PSP_SIGNATURE.len()] == PSP_SIGNATURE.as_slice()
}

/// Read the container: signature, version, every block header, and the image attributes.
///
/// Refuses, as upstream does: a wrong signature, a major version below 3, a block whose declared
/// length runs past the end of the file, a first block that is not the image block, an attribute
/// chunk shorter than its minimum, an unknown compression, and a bit depth the format allows but
/// upstream does not read.
pub fn read_container(bytes: &[u8]) -> Result<PspContainer, FormatError> {
    if !is_psp(bytes) {
        return Err(FormatError::Malformed("PSP signature missing"));
    }
    if bytes.len() < 36 {
        return Err(FormatError::Malformed("PSP file header is truncated"));
    }
    let version_major = le_u16(bytes, 32);
    let version_minor = le_u16(bytes, 34);

    // Upstream's reason, in its own words: "We don't have the documentation for file format
    // versions before 3.0, but newer versions should be mostly backwards compatible". So the floor
    // is an ADMISSION rather than a capability boundary, and the asymmetry is the point -- old
    // files are refused, unknown NEW ones are walked.
    if version_major < 3 {
        return Err(FormatError::UnsupportedFeature(
            "PSP major version below 3 is undocumented and refused upstream too",
        ));
    }

    // 14 bytes at version 3, 10 from version 4 -- see the module header.
    let header_len: usize = if version_major < 4 { 14 } else { 10 };

    let mut blocks: Vec<PspBlock> = Vec::new();
    let mut cursor = 36usize;
    let mut attributes: Option<Attributes> = None;

    while cursor < bytes.len() {
        if cursor + header_len > bytes.len() {
            return Err(FormatError::Malformed("PSP block header is truncated"));
        }
        if &bytes[cursor..cursor + 4] != BLOCK_SIGNATURE.as_slice() {
            return Err(FormatError::Malformed("PSP block header signature missing"));
        }
        let id = le_u16(bytes, cursor + 4);
        let first_len = le_u32(bytes, cursor + 6);
        // At version 3 the second field is the total; from version 4 the FIRST field is the total
        // and there is no second. Reading the same offset under both rules is the misparse this
        // branch exists to prevent.
        let (initial_len, total_len) = if version_major < 4 {
            (first_len, le_u32(bytes, cursor + 10))
        } else {
            (0, first_len)
        };

        let start = cursor + header_len;
        let total = total_len as usize;
        // Upstream's check, with its own message "invalid block size".
        if start.checked_add(total).is_none_or(|end| end > bytes.len()) {
            return Err(FormatError::Malformed(
                "PSP block declares a length past the end of the file",
            ));
        }

        if id == PSP_IMAGE_BLOCK {
            // Upstream refuses an image block that is not block zero, so position is part of the
            // format and not merely a convention.
            if !blocks.is_empty() {
                return Err(FormatError::Malformed(
                    "PSP general image attributes must be the first block",
                ));
            }
            attributes = Some(read_attributes(
                bytes,
                start,
                total_len,
                initial_len,
                version_major,
            )?);
        }

        blocks.push(PspBlock {
            id,
            start,
            len: total,
        });
        cursor = start + total;
    }

    let attributes = attributes.ok_or(FormatError::Malformed(
        "PSP carries no general image attributes block",
    ))?;

    Ok(PspContainer {
        version_major,
        version_minor,
        width: attributes.width,
        height: attributes.height,
        resolution_per_inch: attributes.resolution_per_inch,
        compression: attributes.compression,
        bit_depth: attributes.bit_depth,
        grayscale: attributes.grayscale,
        colour_model: attributes.colour_model,
        bytes_per_sample: attributes.bytes_per_sample,
        active_layer: attributes.active_layer,
        layer_count: attributes.layer_count,
        blocks,
    })
}

struct Attributes {
    width: u32,
    height: u32,
    resolution_per_inch: f64,
    compression: PspCompression,
    bit_depth: u16,
    grayscale: bool,
    colour_model: PspColourModel,
    bytes_per_sample: u8,
    active_layer: u32,
    layer_count: u16,
}

fn read_attributes(
    bytes: &[u8],
    start: usize,
    total_len: u32,
    initial_len: u32,
    version_major: u16,
) -> Result<Attributes, FormatError> {
    // Upstream tests BOTH lengths before reading a single field. At version 4 `initial_len` is the
    // bogus 0xDEADBEEF, which passes this test for free -- so the real guard at version 4 is the
    // inner length read below, not this one.
    if total_len < MIN_ATTRIBUTE_CHUNK || (version_major < 4 && initial_len < MIN_ATTRIBUTE_CHUNK) {
        return Err(FormatError::Malformed(
            "PSP general image attribute chunk is too short",
        ));
    }

    let mut at = start;
    if version_major >= 4 {
        // The chunk repeats its own length, and upstream requires 46 rather than 38 of it.
        if at + 4 > bytes.len() {
            return Err(FormatError::Malformed("PSP attribute chunk is truncated"));
        }
        let inner = le_u32(bytes, at);
        if inner < MIN_ATTRIBUTE_CHUNK_V4 {
            return Err(FormatError::Malformed(
                "PSP version 4 attribute chunk declares too small a length",
            ));
        }
        at += 4;
    }

    // Field order, straight from upstream's read sequence. The offsets are written out because
    // their SUM is a cross-check on the whole layout: they end at exactly 38, which is upstream's
    // own minimum chunk size. At version 4 the chunk also carries `graphics_content` (4) after the
    // layer count and repeats its length (4) at the front, giving 38 + 4 + 4 = 46 — again exactly
    // upstream's version-4 minimum. Two independent constants both land on the field list, so the
    // list is right.
    const ATTRIBUTE_FIELD_BYTES: usize = 38;
    if at + ATTRIBUTE_FIELD_BYTES > bytes.len() {
        return Err(FormatError::Malformed("PSP attribute chunk is truncated"));
    }

    let width = le_u32(bytes, at);
    let height = le_u32(bytes, at + 4);
    let resolution = f64::from_le_bytes([
        bytes[at + 8],
        bytes[at + 9],
        bytes[at + 10],
        bytes[at + 11],
        bytes[at + 12],
        bytes[at + 13],
        bytes[at + 14],
        bytes[at + 15],
    ]);
    let metric = bytes[at + 16];
    let compression = PspCompression::from_u16(le_u16(bytes, at + 17))?;
    let bit_depth = le_u16(bytes, at + 19);
    // Skipped: plane count (2) at 21 and colour count (4) at 23.
    let grayscale = bytes[at + 27] != 0;
    // Skipped: total image size (4) at 28.
    let active_layer = le_u32(bytes, at + 32);
    let layer_count = le_u16(bytes, at + 36);

    if width == 0 || height == 0 {
        return Err(FormatError::Malformed("PSP image has a zero dimension"));
    }

    // Centimetres become inches here, once. See the field's own comment.
    let resolution_per_inch = if metric == PSP_METRIC_CM {
        resolution / 2.54
    } else {
        resolution
    };

    let (colour_model, bytes_per_sample) = colour_model_for(bit_depth, grayscale)?;

    Ok(Attributes {
        width,
        height,
        resolution_per_inch,
        compression,
        bit_depth,
        grayscale,
        colour_model,
        bytes_per_sample,
        active_layer,
        layer_count,
    })
}

/// The depth table, and **it has two holes that are easy to read as bugs and are not ours to fix.**
///
/// | depth | `grayscale` | result |
/// |---|---|---|
/// | 24 | either | RGB, one byte per sample |
/// | 48 | either | RGB, two bytes per sample |
/// | 8, 16 | set | grey, one or two bytes per sample |
/// | 1, 4, 8 | clear | indexed, one byte per sample |
/// | **16** | **clear** | **REFUSED** |
/// | **1, 4** | **set** | **REFUSED** |
///
/// Upstream's two branches are `grayscale && depth >= 8` and `depth <= 8 && !grayscale`. Neither
/// covers 16-bit non-grey, and neither covers 1- or 4-bit grey — so **a one-bit image flagged grey
/// is refused while the same image not so flagged is read as indexed**. Both holes are reproduced
/// rather than closed: a file upstream cannot open is not a file this product should invent a
/// reading for, and the pair is asserted in the tests so a later "tidy-up" of this table has to
/// argue with a failing test instead of a comment.
fn colour_model_for(bit_depth: u16, grayscale: bool) -> Result<(PspColourModel, u8), FormatError> {
    match bit_depth {
        24 => Ok((PspColourModel::Rgb, 1)),
        48 => Ok((PspColourModel::Rgb, 2)),
        1 | 4 | 8 | 16 => {
            if grayscale && bit_depth >= 8 {
                Ok((PspColourModel::Gray, if bit_depth == 16 { 2 } else { 1 }))
            } else if bit_depth <= 8 && !grayscale {
                Ok((PspColourModel::Indexed, 1))
            } else {
                Err(FormatError::UnsupportedFeature(
                    "PSP bit depth and greyscale flag combination is refused upstream too",
                ))
            }
        }
        _ => Err(FormatError::UnsupportedFeature(
            "PSP bit depth is not one upstream reads",
        )),
    }
}

/// How many bytes one scanline of a channel occupies in the file.
///
/// **Below 8 bits a scanline is padded to a 4-byte boundary and at 8 or more it is not.** That
/// split is upstream's, with its own comment: *"Scanlines for 1 and 4 bit only end on a 4-byte
/// boundary."* And in the uncompressed branch, about the other case, upstream contradicts the
/// specification outright — *"Contrary to what the PSP specification seems to suggest scanlines are
/// not stored on a 4-byte boundary."* Both halves are reproduced, and the second is the reason the
/// obvious "round every scanline up to 4" reading is wrong.
pub fn channel_line_width(width: u32, depth: u16, bytes_per_sample: u8) -> usize {
    let width = width as usize;
    if depth < 8 {
        // Two separate roundings, kept separate: bits up to whole bytes, then bytes up to a
        // multiple of four. Collapsing them into one expression is what makes this rule look
        // arbitrary when it is two rules.
        (width * depth as usize).div_ceil(8).div_ceil(4) * 4
    } else {
        width * bytes_per_sample as usize
    }
}

/// How a channel's samples are laid into the destination.
///
/// A PSP channel holds ONE component, and a layer's components arrive in separate channels, so
/// decompressing writes every `stride`-th byte rather than a contiguous run. `stride` is upstream's
/// `bytespp`, and `offset` is which component inside the pixel this channel is.
#[derive(Debug, Clone, Copy)]
pub struct ChannelLayout {
    /// Bytes from one pixel to the next in the destination. Upstream's `bytespp`.
    pub stride: usize,
    /// Byte offset of this channel's component within a pixel.
    pub offset: usize,
    /// 1 or 2. Upstream asserts `bytes_per_sample <= 2`.
    pub bytes_per_sample: u8,
}

/// Decompress one RLE channel into `dest`, writing `layout.stride` bytes apart from `layout.offset`.
///
/// **The run flag is `> 128`, not `>= 128`, and that asymmetry is the whole encoding.** A count
/// byte of 128 is a LITERAL of 128 bytes; 129 is a RUN of one. So the literal reaches its maximum
/// at exactly the value a reader expects to be the first run, and getting the comparison wrong
/// shifts every subsequent byte of the channel rather than producing a visibly broken pixel.
///
/// | count byte | meaning |
/// |---|---|
/// | 1 to 128 | literal: copy that many bytes |
/// | 129 to 255 | run: `count - 128` copies of the next byte |
/// | **0** | **refused — see below** |
///
/// **A ZERO COUNT IS REFUSED, and the reverse-verification sharpened why.** Upstream's loop is
/// `while (q < endq)`: a zero count copies nothing, advances `q` by nothing, and its `fread`
/// return values are not checked in this branch, so a crafted file spins. Its own overflow guard
/// cannot catch it either — the guard tests `runcount > remaining`, and zero is never greater than
/// anything.
///
/// **This port could not spin even without the check**, because it reads from a slice and the next
/// count byte eventually falls off the end. Removing the check was tried: the test still failed,
/// but with *"ends mid-channel"* — so what the check buys is not termination, it is the TRUE cause
/// instead of a misleading one. A zero count is a malformed stream, not a truncated one, and a
/// reader chasing the wrong message looks for a missing tail that was never missing. Recorded as a
/// deliberate divergence either way, so a reader comparing the two does not think a case was lost.
pub fn decompress_rle(
    source: &[u8],
    dest: &mut [u8],
    layout: ChannelLayout,
    dest_len: usize,
) -> Result<(), FormatError> {
    if layout.stride == 0 || layout.bytes_per_sample == 0 || layout.bytes_per_sample > 2 {
        return Err(FormatError::InvalidOption(
            "PSP channel layout is not usable",
        ));
    }
    if layout.offset > dest.len() {
        return Err(FormatError::Malformed(
            "PSP channel offset is past its destination",
        ));
    }

    let mut read = 0usize;
    // **`end` is a CURSOR limit, not a buffer bound, and upstream has it the same way.** Its
    // `endq = q + npixels * bytespp` is measured from the channel's offset, so for any non-zero
    // offset it points past the destination -- and nothing is written there, because the cursor
    // steps by `stride` and the last step lands short. Treating `end` as a buffer bound instead
    // rejects a perfectly ordinary interleaved channel, which is what the first version of this
    // function did. Every write below is bounded by `dest.len()` separately.
    let mut q = layout.offset;
    let end = layout.offset + dest_len;

    while q < end {
        let count = *source
            .get(read)
            .ok_or(FormatError::Malformed("PSP RLE data ends mid-channel"))?;
        read += 1;

        if count == 0 {
            return Err(FormatError::Malformed(
                "PSP RLE count of zero would not advance; refused rather than looped",
            ));
        }

        // The literal buffer upstream allocates is 128 bytes, which is the maximum a literal can
        // be -- a cross-check that the `> 128` reading is the intended one.
        let mut run = [0u8; 128];
        let run_len: usize;
        if count > 128 {
            run_len = (count - 128) as usize;
            let byte = *source
                .get(read)
                .ok_or(FormatError::Malformed("PSP RLE run has no value byte"))?;
            read += 1;
            run[..run_len].fill(byte);
        } else {
            run_len = count as usize;
            let slice = source
                .get(read..read + run_len)
                .ok_or(FormatError::Malformed("PSP RLE literal is truncated"))?;
            run[..run_len].copy_from_slice(slice);
            read += run_len;
        }

        // Upstream's own overflow guard, kept in its own shape rather than tightened: it allows
        // `bytes_per_sample - 1` bytes of slack past the remaining space, which at two bytes per
        // sample lets a final odd byte through. Tightening it would reject files upstream accepts.
        let remaining = end - q;
        if run_len > remaining / layout.stride + layout.bytes_per_sample as usize - 1 {
            // Upstream prints a warning and BREAKS, keeping what it has, rather than failing the
            // load. A partly-decoded channel is what upstream hands back, so that is what this
            // does -- the alternative silently rejects images that currently open.
            break;
        }

        if layout.stride == 1 {
            // Contiguous: the fast path, and the only one where a run of N advances N bytes.
            let take = run_len.min(end - q).min(dest.len() - q);
            dest[q..q + take].copy_from_slice(&run[..take]);
            q += take;
            if take < run_len {
                break;
            }
        } else if layout.bytes_per_sample == 1 {
            // One byte per sample, scattered into an interleaved destination.
            for byte in &run[..run_len] {
                if q >= end || q >= dest.len() {
                    break;
                }
                dest[q] = *byte;
                q += layout.stride;
            }
        } else {
            // Two bytes per sample, read little-endian and scattered by whole samples.
            // **`run_len / 2` is upstream's count, so an ODD run loses its last byte.** Faithful:
            // the sample it would start is incomplete, and upstream does not invent its other half.
            for index in 0..run_len / 2 {
                if q + 2 > end || q + 2 > dest.len() {
                    break;
                }
                let sample = u16::from_le_bytes([run[index * 2], run[index * 2 + 1]]);
                dest[q..q + 2].copy_from_slice(&sample.to_le_bytes());
                q += layout.stride;
            }
        }
    }

    Ok(())
}

/// Read one uncompressed channel into `dest`.
///
/// Upstream has three paths here and they do not count the same thing: a contiguous destination is
/// one bulk read; one byte per sample loops over **`line_width`**; two bytes per sample reads
/// `width` samples and loops over **`width`**. At 8 bits or more the two counts coincide, so the
/// difference only shows below 8 bits — where `line_width` is the padded byte count and the samples
/// are bit-packed, which is why the byte loop is the correct one there.
pub fn read_uncompressed(
    source: &[u8],
    dest: &mut [u8],
    layout: ChannelLayout,
    width: u32,
    height: u32,
    depth: u16,
) -> Result<(), FormatError> {
    if layout.stride == 0 || layout.bytes_per_sample == 0 || layout.bytes_per_sample > 2 {
        return Err(FormatError::InvalidOption(
            "PSP channel layout is not usable",
        ));
    }
    let line_width = channel_line_width(width, depth, layout.bytes_per_sample);
    let height = height as usize;

    if layout.stride == 1 {
        let needed = height
            .checked_mul(line_width)
            .ok_or(FormatError::LimitExceeded("PSP channel size"))?;
        let slice = source.get(..needed).ok_or(FormatError::Malformed(
            "PSP uncompressed channel is truncated",
        ))?;
        dest.get_mut(layout.offset..layout.offset + needed)
            .ok_or(FormatError::Malformed(
                "PSP channel does not fit its destination",
            ))?
            .copy_from_slice(slice);
        return Ok(());
    }

    let mut read = 0usize;
    for row in 0..height {
        if layout.bytes_per_sample == 1 {
            let slice = source
                .get(read..read + line_width)
                .ok_or(FormatError::Malformed(
                    "PSP uncompressed channel is truncated",
                ))?;
            read += line_width;
            let mut q = layout.offset + row * width as usize * layout.stride;
            for byte in slice {
                if q >= dest.len() {
                    break;
                }
                dest[q] = *byte;
                q += layout.stride;
            }
        } else {
            let count = width as usize;
            let slice = source
                .get(read..read + count * 2)
                .ok_or(FormatError::Malformed(
                    "PSP uncompressed channel is truncated",
                ))?;
            read += count * 2;
            let mut q = layout.offset + row * width as usize * layout.stride;
            for index in 0..count {
                if q + 1 >= dest.len() {
                    break;
                }
                let sample = u16::from_le_bytes([slice[index * 2], slice[index * 2 + 1]]);
                dest[q..q + 2].copy_from_slice(&sample.to_le_bytes());
                q += layout.stride;
            }
        }
    }
    Ok(())
}

/// What kind of layer this is, from upstream's `keGLT*` enumeration.
///
/// **At version 3 this is read and then THROWN AWAY.** Upstream reads the type byte, prints a
/// message if it was a floating selection, and then assigns `type = keGLTRaster` unconditionally
/// with `can_handle_layer = TRUE`. So a version-3 file has no vector, adjustment, group or mask
/// layers whatever its bytes say, and the type is only load-bearing from version 4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PspLayerKind {
    Raster,
    /// A floating selection. **Upstream treats it as a raster layer on purpose** — at version 4 its
    /// `case` falls THROUGH to the raster case with only a message, and at version 3 it is
    /// overwritten. Kept as its own value because the message is a real difference in behaviour
    /// and collapsing it here would make that message unreachable.
    FloatingRasterSelection,
    Vector,
    Adjustment,
    /// Since PSP8.
    Group,
    /// Since PSP8.
    Mask,
    /// Since PSP9.
    ArtMedia,
}

impl PspLayerKind {
    fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Raster),
            1 => Some(Self::FloatingRasterSelection),
            2 => Some(Self::Vector),
            3 => Some(Self::Adjustment),
            4 => Some(Self::Group),
            5 => Some(Self::Mask),
            6 => Some(Self::ArtMedia),
            _ => None,
        }
    }
}

/// A rectangle as the file stores it: four little-endian `u32` in left, top, right, bottom order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PspRect {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

impl PspRect {
    fn read(bytes: &[u8], at: usize) -> Self {
        Self {
            left: le_u32(bytes, at),
            top: le_u32(bytes, at + 4),
            right: le_u32(bytes, at + 8),
            bottom: le_u32(bytes, at + 12),
        }
    }
}

/// One layer's attributes, before any pixel data is touched.
#[derive(Debug, Clone, PartialEq)]
pub struct PspLayer {
    /// Decoded from **ISO-8859-1**, not UTF-8 — upstream calls
    /// `g_convert (name, -1, "utf-8", "iso8859-1", ...)`. A latin-1 byte above 0x7F is therefore one
    /// character, not the start of a multi-byte sequence, and reading it as UTF-8 either mangles
    /// the name or fails outright. **A zero-length name is valid** (see the reader).
    pub name: String,
    pub kind: PspLayerKind,
    /// Where the layer sits in the image.
    pub image_rect: PspRect,
    /// **This is the one the dimensions come from**, not `image_rect`: upstream computes
    /// `width = saved_image_rect[2] - saved_image_rect[0]`. The saved rectangle is the part actually
    /// stored, which can be smaller than the layer's nominal place in the image.
    pub saved_image_rect: PspRect,
    pub mask_rect: PspRect,
    pub saved_mask_rect: PspRect,
    pub opacity: u8,
    /// The raw PSP blend value, kept beside the mapped one because the mapping is lossy — see
    /// [`blend_mode_for`].
    pub blend_mode_raw: u8,
    pub blend_mode: BlendMode,
    /// **False when the blend mode could not be mapped**, which is upstream's behaviour rather than
    /// ours: it keeps the layer, sets normal, and turns visibility OFF.
    pub visible: bool,
    pub transparency_protected: bool,
    pub link_group_id: u8,
    pub mask_linked: bool,
    pub mask_disabled: bool,
    pub width: u32,
    pub height: u32,
}

/// Map a PSP blend value onto ours.
///
/// **The mapping is LOSSY in a way worth stating, because upstream's is not.** GIMP has `_LEGACY`
/// and modern variants of the colour-composition modes, and PSP8 added a parallel "true" family —
/// so upstream sends `PSP_BLEND_HUE` to `HSV_HUE_LEGACY` and `PSP_BLEND_TRUE_HUE` to `HSV_HUE`,
/// keeping them apart. This product has one `HsvHue`, so the two PSP values arrive at the same
/// place. That is a real loss of fidelity against upstream and it is recorded rather than hidden:
/// the raw byte is kept on the layer so a future split has the information it needs.
///
/// **`PSP_BLEND_ADJUST` (255) has NO mapping and that is deliberate upstream**, marked with its own
/// `/* ??? */`. Upstream's `PSP_BLEND_LUMINOSITY` line carries the same marker — it sends luminosity
/// to `HSV_VALUE_LEGACY` and is visibly unsure. Both are reproduced as upstream has them.
pub fn blend_mode_for(value: u8) -> Option<BlendMode> {
    Some(match value {
        0 => BlendMode::Normal,
        1 => BlendMode::DarkenOnly,
        2 => BlendMode::LightenOnly,
        3 => BlendMode::HsvHue,
        4 => BlendMode::HsvSaturation,
        5 => BlendMode::HslColor,
        // Upstream's own "???": luminosity becomes HSV value.
        6 => BlendMode::HsvValue,
        7 => BlendMode::Multiply,
        8 => BlendMode::Screen,
        9 => BlendMode::Dissolve,
        10 => BlendMode::Overlay,
        11 => BlendMode::HardLight,
        12 => BlendMode::SoftLight,
        13 => BlendMode::Difference,
        14 => BlendMode::Dodge,
        15 => BlendMode::Burn,
        16 => BlendMode::Exclusion,
        // The PSP8 "true" family. Upstream separates these from 3..=6 by legacy-ness; we cannot.
        17 => BlendMode::HsvHue,
        18 => BlendMode::HsvSaturation,
        19 => BlendMode::HslColor,
        20 => BlendMode::HsvValue,
        // 255 is PSP_BLEND_ADJUST, which upstream explicitly declines to map.
        _ => return None,
    })
}

/// Upstream's own ceilings on a layer, reproduced including the odd one.
///
/// `width` and `height` must each be at most 2^18, **and** `(width / 256) * (height / 256)` must be
/// below 8192. That third test uses INTEGER division, which discards up to 255 from each edge and so
/// UNDER-estimates the area: the limit is **looser** than the true area check it resembles, and a
/// 23200-square layer passes it while its real area of 538,240,000 exceeds what a true check would
/// allow. Reproduced as written, because tightening it would refuse files upstream opens.
///
/// **A first attempt at documenting this said the quirk showed on a tall, narrow layer -- that a
/// layer under 256 wide multiplies by zero and passes however tall it is.** The arithmetic is true
/// and the conclusion was useless: with the edge capped at 2^18, such a layer reaches at most
/// 66,846,720 pixels, which a true area check accepts anyway. The reverse-verification that swapped
/// the formula for a true area check passed, which is what exposed it.
const PSP_MAX_LAYER_EDGE: u32 = 1 << 18;

fn layer_dimensions(saved: PspRect) -> Result<(u32, u32), FormatError> {
    // Upstream computes these as signed subtractions and then tests for negative, so a right edge
    // left of the left edge is a refusal rather than a wrap.
    let width = (saved.right as i64) - (saved.left as i64);
    let height = (saved.bottom as i64) - (saved.top as i64);
    if width < 0 || height < 0 {
        return Err(FormatError::Malformed("PSP layer rectangle is inverted"));
    }
    let width = width as u32;
    let height = height as u32;
    if width > PSP_MAX_LAYER_EDGE || height > PSP_MAX_LAYER_EDGE {
        return Err(FormatError::LimitExceeded("PSP layer edge"));
    }
    if (width / 256) * (height / 256) >= 8192 {
        return Err(FormatError::LimitExceeded("PSP layer area"));
    }
    Ok((width, height))
}

/// Read the layer bank: every sub-block of a `PSP_LAYER_START_BLOCK`.
///
/// **Each sub-block must be a layer block and upstream names the offender when it is not**, so a
/// bank carrying anything else is a refusal rather than something to skip past. `block_data` is the
/// bank block's data, which [`read_container`] already bounded.
pub fn read_layer_bank(
    block_data: &[u8],
    version_major: u16,
) -> Result<Vec<PspLayer>, FormatError> {
    let header_len: usize = if version_major < 4 { 14 } else { 10 };
    let mut layers = Vec::new();
    let mut cursor = 0usize;

    while cursor < block_data.len() {
        if cursor + header_len > block_data.len() {
            return Err(FormatError::Malformed(
                "PSP layer sub-block header is truncated",
            ));
        }
        if &block_data[cursor..cursor + 4] != BLOCK_SIGNATURE.as_slice() {
            return Err(FormatError::Malformed(
                "PSP layer sub-block header signature missing",
            ));
        }
        let id = le_u16(block_data, cursor + 4);
        if id != PSP_LAYER_BLOCK {
            return Err(FormatError::Malformed(
                "PSP layer bank holds a sub-block that is not a layer",
            ));
        }
        let first_len = le_u32(block_data, cursor + 6);
        let (initial_len, total_len) = if version_major < 4 {
            (first_len, le_u32(block_data, cursor + 10))
        } else {
            (0, first_len)
        };

        let start = cursor + header_len;
        let total = total_len as usize;
        if start
            .checked_add(total)
            .is_none_or(|end| end > block_data.len())
        {
            return Err(FormatError::Malformed(
                "PSP layer sub-block runs past its bank",
            ));
        }

        layers.push(read_layer_info(
            &block_data[start..start + total],
            initial_len,
            version_major,
        )?);
        cursor = start + total;
    }

    Ok(layers)
}

/// The layer id, from upstream's block enumeration.
const PSP_LAYER_BLOCK: u16 = 4;

fn read_layer_info(
    data: &[u8],
    initial_len: u32,
    version_major: u16,
) -> Result<PspLayer, FormatError> {
    // **THE NAME FIELD IS SHAPED DIFFERENTLY IN THE TWO VERSIONS, and this is the second place in
    // the format where that is true.** Version 3 stores a FIXED 256-byte field, NUL-terminated --
    // upstream allocates 257 and sets the last byte itself. Version 4 stores a `u16` length and
    // then exactly that many bytes. A reader that uses one shape on the other version either eats
    // 254 bytes of the following fields or reads a length out of the middle of a name.
    let mut at;
    let name_bytes: &[u8];

    if version_major >= 4 {
        // Version 4 opens with the chunk's own length, which is how it finds the extension that
        // follows. Version 3 has no such field and uses the block header's initial length instead
        // -- the very field version 4 dropped from the header. Same purpose, two mechanisms, and
        // neither version has the other's.
        if data.len() < 4 {
            return Err(FormatError::Malformed("PSP layer chunk is truncated"));
        }
        let _chunk_len = le_u32(data, 0) as usize;
        at = 4;

        if at + 2 > data.len() {
            return Err(FormatError::Malformed("PSP layer name length is missing"));
        }
        let name_len = le_u16(data, at) as usize;
        at += 2;

        // **A zero-length layer name is valid.** Upstream says so in a comment and then writes a
        // guard that CANNOT FIRE -- `(namelen = ...) && (FALSE || namelen == 0)` is
        // `namelen && !namelen`, always false -- purely to silence a compiler warning about an
        // unsigned value being negative. The guard is dead code and the comment is the real
        // documentation, so an empty name must be accepted. Asserted in the tests, because a
        // reader who skips the comment and ports the guard writes a condition that does nothing
        // while believing it does something.
        name_bytes = data
            .get(at..at + name_len)
            .ok_or(FormatError::Malformed("PSP layer name is truncated"))?;
        at += name_len;
    } else {
        // Version 3: 256 bytes, and the name is whatever precedes the first NUL.
        let field = data
            .get(..256)
            .ok_or(FormatError::Malformed("PSP layer name field is truncated"))?;
        let end = field.iter().position(|b| *b == 0).unwrap_or(field.len());
        name_bytes = &field[..end];
        at = 256;
        // `initial_len` is what upstream seeks by here; it is read for that purpose in M.9e.
        let _ = initial_len;
    }

    // Latin-1 to UTF-8: every byte is one code point. Upstream's g_convert from "iso8859-1".
    let name: String = name_bytes.iter().map(|b| *b as char).collect();

    // The fixed run of fields after the name is identical in both versions.
    const FIXED: usize = 1 + 16 + 16 + 1 + 1 + 1 + 1 + 1 + 16 + 16 + 1 + 1;
    let fixed = data.get(at..at + FIXED).ok_or(FormatError::Malformed(
        "PSP layer information chunk is truncated",
    ))?;

    let kind_raw = fixed[0];
    let image_rect = PspRect::read(fixed, 1);
    let saved_image_rect = PspRect::read(fixed, 17);
    let opacity = fixed[33];
    let blend_mode_raw = fixed[34];
    let visibility = fixed[35] != 0;
    let transparency_protected = fixed[36] != 0;
    let link_group_id = fixed[37];
    let mask_rect = PspRect::read(fixed, 38);
    let saved_mask_rect = PspRect::read(fixed, 54);
    let mask_linked = fixed[70] != 0;
    let mask_disabled = fixed[71] != 0;

    let kind = if version_major < 4 {
        // Version 3 reads the byte and overwrites it. See `PspLayerKind`.
        PspLayerKind::Raster
    } else {
        PspLayerKind::from_u8(kind_raw)
            .ok_or(FormatError::UnsupportedFeature("PSP layer type is unknown"))?
    };

    // Upstream keeps a layer whose blend mode it cannot map, sets normal, and turns visibility off
    // with a message. Failing instead would reject a whole image for one unmappable layer.
    let (blend_mode, visible) = match blend_mode_for(blend_mode_raw) {
        Some(mode) => (mode, visibility),
        None => (BlendMode::Normal, false),
    };

    let (width, height) = layer_dimensions(saved_image_rect)?;

    Ok(PspLayer {
        name,
        kind,
        image_rect,
        saved_image_rect,
        mask_rect,
        saved_mask_rect,
        opacity,
        blend_mode_raw,
        blend_mode,
        visible,
        transparency_protected,
        link_group_id,
        mask_linked,
        mask_disabled,
        width,
        height,
    })
}

/// Upstream's ceiling on a palette, and **its own comment says the limit is GIMP's, not the
/// format's**: *"GIMP currently only supports a maximum of 256 colors in an indexed image. If this
/// changes, we can change this check"*. Reproduced, because a product that reads a 300-entry
/// palette upstream refuses is diverging rather than improving — and the note records that the
/// number is a host limit a later cycle could revisit on purpose.
pub const PSP_MAX_PALETTE_ENTRIES: usize = 256;

/// Read the colour palette block into RGB triples.
///
/// **THE FILE STORES BGR, AND UPSTREAM'S COMMENT ABOUT IT IS BACKWARDS.** The comment reads
/// *"Convert to BGR palette"*, and the code does the opposite: it writes source byte 2 to
/// destination byte 0 and source byte 0 to destination byte 2, then hands the result to
/// `babl_format ("R'G'B' u8")`. That babl tag is what settles it — the OUTPUT is R'G'B', so the
/// input byte that becomes red is byte 2, which makes the stored order **B, G, R**. Following the
/// comment instead of the code swaps red and blue in every indexed image, which is the kind of bug
/// that looks like a colour-management problem for a week.
///
/// Entries are **four** bytes each and upstream notes *"the fourth byte is always zero"*, so the
/// stored alpha is not alpha and is dropped rather than carried.
///
/// Returns `None` when the image is not indexed. **That is not an error** — upstream says so in its
/// own words, *"Skipping, but not an error, can happen for grayscale PSP images"* — so a grey image
/// carrying a palette block is read successfully and the block ignored.
pub fn read_palette(
    block_data: &[u8],
    version_major: u16,
    colour_model: PspColourModel,
) -> Result<Option<Vec<[u8; 3]>>, FormatError> {
    if colour_model != PspColourModel::Indexed {
        return Ok(None);
    }

    // Version 4 again finds its data through a chunk length, and version 3 does not have one.
    // Note the ORDER at version 4: the length comes first and the entry count second, and then the
    // entries are read from `chunk_len` bytes into the block -- NOT from just after the count. So
    // at version 4 the count and the entries are not adjacent.
    let (entry_count, entries_at) = if version_major >= 4 {
        if block_data.len() < 8 {
            return Err(FormatError::Malformed("PSP colour block is truncated"));
        }
        let chunk_len = le_u32(block_data, 0) as usize;
        let count = le_u32(block_data, 4) as usize;
        (count, chunk_len)
    } else {
        if block_data.len() < 4 {
            return Err(FormatError::Malformed("PSP colour block is truncated"));
        }
        (le_u32(block_data, 0) as usize, 4)
    };

    if entry_count > PSP_MAX_PALETTE_ENTRIES {
        return Err(FormatError::UnsupportedFeature(
            "PSP palette has more than 256 entries, which upstream refuses too",
        ));
    }

    let bytes = block_data
        .get(entries_at..entries_at + entry_count * 4)
        .ok_or(FormatError::Malformed("PSP palette entries are truncated"))?;

    let mut palette = Vec::with_capacity(entry_count);
    for entry in bytes.chunks_exact(4) {
        // Stored B, G, R, then a byte upstream says is always zero.
        palette.push([entry[2], entry[1], entry[0]]);
    }
    Ok(Some(palette))
}

/// Decompress one `PSP_COMP_LZ77` channel.
///
/// **"LZ77" is PSP's name for a plain zlib stream, which is the single most useful fact here.**
/// Upstream calls `inflateInit` / `inflate` / `inflateEnd` with no custom window or dictionary, so
/// there is no bespoke scheme to re-derive — only a correct inflate and the same scatter the other
/// two paths use.
///
/// **It is STRICT where RLE is LENIENT, and the asymmetry is upstream's.** This path requires
/// `inflate` to reach `Z_STREAM_END` and fails the load otherwise; [`decompress_rle`] breaks out of
/// its loop on an overrun and hands back a partly-filled channel. So the same image's two
/// compression schemes have opposite failure policies, and a reader that unifies them has to break
/// one of them.
///
/// The output buffer is `pixel_count * bytes_per_sample`, **not** `line_width * height`: upstream's
/// `avail_out` is pixel-count based on this path even below 8 bits, where the other paths use the
/// padded line width. Reproduced rather than reconciled.
pub fn decompress_lz77(
    source: &[u8],
    dest: &mut [u8],
    layout: ChannelLayout,
    pixel_count: usize,
) -> Result<(), FormatError> {
    if layout.stride == 0 || layout.bytes_per_sample == 0 || layout.bytes_per_sample > 2 {
        return Err(FormatError::InvalidOption(
            "PSP channel layout is not usable",
        ));
    }
    let expected = pixel_count
        .checked_mul(layout.bytes_per_sample as usize)
        .ok_or(FormatError::LimitExceeded("PSP channel size"))?;

    // Upstream sets `avail_out` to exactly this and nothing larger, so the output capacity is part
    // of the contract rather than an implementation detail: a stream carrying MORE than the channel
    // cannot reach Z_STREAM_END because inflate runs out of room. Reserving exactly `expected`
    // reproduces that, and `flate2::Decompress::decompress_vec` writes only into spare capacity --
    // handing it a vector with none is why the first version of this inflated nothing at all.
    let mut plain = Vec::with_capacity(expected);
    let mut inflate = flate2::Decompress::new(true);
    let status = inflate
        .decompress_vec(source, &mut plain, flate2::FlushDecompress::Finish)
        .map_err(|_| FormatError::Malformed("PSP LZ77 channel is not a valid zlib stream"))?;

    // Upstream insists on Z_STREAM_END, so a stream that merely ran out of input is an error --
    // and so is one that filled the buffer without ending, which is the "too much data" case.
    if status != flate2::Status::StreamEnd {
        return Err(FormatError::Malformed(
            "PSP LZ77 channel did not reach the end of its zlib stream",
        ));
    }
    if plain.len() != expected {
        return Err(FormatError::Malformed(
            "PSP LZ77 channel did not inflate to its declared size",
        ));
    }

    if layout.stride == 1 {
        dest.get_mut(layout.offset..layout.offset + expected)
            .ok_or(FormatError::Malformed(
                "PSP channel does not fit its destination",
            ))?
            .copy_from_slice(&plain);
        return Ok(());
    }

    let mut q = layout.offset;
    if layout.bytes_per_sample == 1 {
        for byte in &plain {
            if q >= dest.len() {
                break;
            }
            dest[q] = *byte;
            q += layout.stride;
        }
    } else {
        for sample in plain.chunks_exact(2) {
            if q + 2 > dest.len() {
                break;
            }
            let value = u16::from_le_bytes([sample[0], sample[1]]);
            dest[q..q + 2].copy_from_slice(&value.to_le_bytes());
            q += layout.stride;
        }
    }
    Ok(())
}

/// Expand 1- or 4-bit indices to one byte each, for an indexed image below 8 bits.
///
/// **Bits are read MOST SIGNIFICANT FIRST within each byte** — upstream's mask is
/// `128 >> (current_bit % 8)` and it accumulates with `1 << (bpp - 1 - b)`, so the first bit of a
/// pixel is the high bit of its index. Reading them the other way round gives a plausible-looking
/// image with every index bit-reversed, which is why the order has its own test rather than a
/// comment.
///
/// Scanlines use the **padded** line width here, unlike the LZ77 output buffer above.
pub fn upscale_indexed_sub_8(
    packed: &[u8],
    width: u32,
    height: u32,
    depth: u16,
) -> Result<Vec<u8>, FormatError> {
    if depth != 1 && depth != 4 {
        return Err(FormatError::InvalidOption(
            "PSP index upscaling applies to 1- and 4-bit images only",
        ));
    }
    let width = width as usize;
    let height = height as usize;
    let line_width = channel_line_width(width as u32, depth, 1);
    let depth = depth as usize;

    let mut out = vec![0u8; width * height];
    for y in 0..height {
        let row = packed
            .get(y * line_width..(y + 1) * line_width)
            .ok_or(FormatError::Malformed("PSP indexed rows are truncated"))?;
        for x in 0..width {
            let mut index = 0u8;
            for bit in 0..depth {
                let at = depth * x + bit;
                if row[at / 8] & (128 >> (at % 8)) != 0 {
                    index += 1 << (depth - 1 - bit);
                }
            }
            out[y * width + x] = index;
        }
    }
    Ok(out)
}

/// Bitmap kinds, from upstream's `PSPDIBType`. Only the first three matter to a layer.
const PSP_DIB_IMAGE: u16 = 0;
const PSP_DIB_TRANS_MASK: u16 = 1;
const PSP_DIB_USER_MASK: u16 = 2;

/// Channel kinds, from upstream's `PSPChannelType`.
const PSP_CHANNEL_COMPOSITE: u16 = 0;
const PSP_CHANNEL_RED: u16 = 1;
const PSP_CHANNEL_BLUE: u16 = 3;

/// The channel block id, from upstream's block enumeration.
const PSP_CHANNEL_BLOCK: u16 = 5;

/// At version 4 the channel information chunk declares its own length and upstream requires at
/// least this much. **The number is exactly the field total**: the 4-byte length itself, then
/// `compressed_len` (4), `uncompressed_len` (4), `bitmap_type` (2) and `channel_type` (2). The same
/// kind of cross-check the image attribute chunk's 38 and 46 gave.
const MIN_CHANNEL_CHUNK_V4: u32 = 16;

/// One layer's pixels, assembled from its channels.
#[derive(Debug, Clone, PartialEq)]
pub struct PspLayerPixels {
    pub width: u32,
    pub height: u32,
    /// Components per pixel: 3 or 4 for RGB, 1 or 2 for grey and indexed. Upstream's `bytespp`
    /// before it is multiplied by the sample size.
    pub components: u8,
    pub bytes_per_sample: u8,
    /// `height * line_width` bytes, interleaved.
    pub data: Vec<u8>,
    /// Channels upstream reports on and skips rather than failing: an unsupported bitmap type, and
    /// the layer user mask it has a `FIXME` for. Carried so a caller can tell the user what was
    /// dropped instead of silently losing it.
    pub skipped: Vec<PspSkippedChannel>,
}

/// A channel upstream declines, with the reason it gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PspSkippedChannel {
    /// `bitmap_type > PSP_DIB_USER_MASK`. Upstream: *"Conversion of bitmap type %d is not
    /// supported."* A message, not an error.
    UnsupportedBitmapType(u16),
    /// `bitmap_type == PSP_DIB_USER_MASK`. Upstream: *"Conversion of layer mask is not supported"*,
    /// beside its own `FIXME: Add as layer mask`.
    LayerMask,
}

/// How many components a layer has, and whether it carries alpha.
///
/// **`bitmap_count == 1` means no alpha and anything else means alpha.** Upstream then multiplies
/// by `bytes_per_sample`, so `components` here is the count before that multiplication — keeping
/// them apart is what makes the channel offsets below readable.
fn layer_components(colour_model: PspColourModel, bitmap_count: u16) -> u8 {
    let base = match colour_model {
        PspColourModel::Rgb => 3,
        PspColourModel::Gray | PspColourModel::Indexed => 1,
    };
    if bitmap_count == 1 { base } else { base + 1 }
}

/// Which byte of a pixel this channel writes to.
///
/// Upstream's rule, verbatim in shape: a transparency bitmap or a composite channel goes to
/// `bytespp - bytes_per_sample` — the LAST component — and anything else to
/// `(channel_type - PSP_CHANNEL_RED) * bytes_per_sample`. For RGB that is exactly right: red, green
/// and blue land at 0, 1, 2 and the transparency mask at 3.
///
/// **ONE NARROW DIVERGENCE, because the rule collides with itself on grey and indexed layers that
/// have alpha.** A grey image's only legal channel type is `COMPOSITE` (see
/// [`validate_channel_type`]), so its colour data takes the first branch and is sent to the last
/// component. With no alpha that is component 0 and correct by coincidence. **With alpha,
/// `bytespp - bytes_per_sample` is the ALPHA slot, so upstream writes the grey into alpha and the
/// transparency mask then overwrites it** — the colour data is lost and the alpha is a copy of the
/// image. The formula is built for the RGB case, where `COMPOSITE` never arrives as colour data.
///
/// So when the bitmap is `PSP_DIB_IMAGE`, the channel is `COMPOSITE`, and the layer has alpha, this
/// sends the data to component 0. Every other case is upstream's. The divergence is recorded rather
/// than silent because faithfulness here produces visibly wrong pixels, which is the same ground
/// the RLE zero-count divergence stands on.
fn channel_offset(
    bitmap_type: u16,
    channel_type: u16,
    colour_model: PspColourModel,
    components: u8,
    bytes_per_sample: u8,
) -> usize {
    let bytespp = components as usize * bytes_per_sample as usize;
    let last = bytespp - bytes_per_sample as usize;

    // The divergence, kept as narrow as the problem: a grey or indexed layer -- where composite IS
    // the colour data -- that has alpha. An RGB layer keeps upstream's rule untouched, because a
    // composite channel there is not colour data and widening the divergence to cover it would be
    // changing behaviour nobody measured.
    if colour_model != PspColourModel::Rgb
        && bitmap_type == PSP_DIB_IMAGE
        && channel_type == PSP_CHANNEL_COMPOSITE
        && last > 0
    {
        return 0;
    }
    if bitmap_type == PSP_DIB_TRANS_MASK || channel_type == PSP_CHANNEL_COMPOSITE {
        return last;
    }
    (channel_type - PSP_CHANNEL_RED) as usize * bytes_per_sample as usize
}

/// **The legal channel types depend on the colour model, and asymmetrically.**
///
/// Upstream refuses `channel_type > PSP_CHANNEL_BLUE` for an RGB image and
/// `channel_type >= PSP_CHANNEL_RED` for anything else. So an RGB layer may carry composite, red,
/// green and blue, while a grey or indexed layer may carry **only composite** — a red channel in a
/// grey image is a refusal, not something to ignore.
fn validate_channel_type(
    colour_model: PspColourModel,
    channel_type: u16,
) -> Result<(), FormatError> {
    let legal = match colour_model {
        PspColourModel::Rgb => channel_type <= PSP_CHANNEL_BLUE,
        _ => channel_type < PSP_CHANNEL_RED,
    };
    if legal {
        Ok(())
    } else {
        Err(FormatError::Malformed(
            "PSP channel type is not legal for this colour model",
        ))
    }
}

/// The buffer's line width for a layer.
///
/// `width * bytespp`, **raised to the padded scanline width when that is larger**. Upstream's own
/// comment: *"For small widths, when depth is 1, or 4, the number of bytes used can be larger than
/// the width * bytespp. Adjust for that."* It is a maximum rather than a branch — the padded value
/// wins only sometimes, and a narrow 1-bit layer is where it does.
pub fn layer_line_width(width: u32, components: u8, bytes_per_sample: u8, depth: u16) -> usize {
    let plain = width as usize * components as usize * bytes_per_sample as usize;
    if depth < 8 {
        plain.max(channel_line_width(width, depth, bytes_per_sample))
    } else {
        plain
    }
}

/// Where a layer sits in the image.
///
/// **The origin is the SUM of the two rectangles' origins**, not either one alone: upstream calls
/// `gimp_layer_set_offsets (layer, image_rect[0] + saved_image_rect[0], ...)`. So the saved
/// rectangle's origin is RELATIVE to the image rectangle's, which is not what either name suggests
/// and is the sort of thing that puts every layer in the wrong place by a constant.
pub fn layer_origin(layer: &PspLayer) -> (u32, u32) {
    (
        layer
            .image_rect
            .left
            .saturating_add(layer.saved_image_rect.left),
        layer
            .image_rect
            .top
            .saturating_add(layer.saved_image_rect.top),
    )
}

/// Assemble one layer's channels into an interleaved buffer.
///
/// `channel_area` is the layer sub-block's data from `chunk_len` onwards — every channel sub-block,
/// one after another. `bitmap_count` comes from the layer or raster extension chunk.
///
/// **A zero-height layer becomes a one-row NULL layer rather than an error**, which is upstream's:
/// `if (height == 0) { height++; null_layer = TRUE; }`. A null layer allocates its buffer and reads
/// no channels at all, so it arrives as a one-row transparent strip instead of failing the load.
/// The image-level facts every layer's channels are read against.
///
/// All five come from the container — the version from the file header and the rest from the
/// General Image Attributes block — so grouping them says in the type that they are properties of
/// the IMAGE and not of a layer. Build one with [`PspImageContext::from_container`] rather than by
/// hand, so a caller cannot pair a depth with the wrong colour model.
#[derive(Debug, Clone, Copy)]
pub struct PspImageContext {
    pub version_major: u16,
    pub colour_model: PspColourModel,
    pub depth: u16,
    pub bytes_per_sample: u8,
    pub compression: PspCompression,
}

impl PspImageContext {
    pub fn from_container(container: &PspContainer) -> Self {
        Self {
            version_major: container.version_major,
            colour_model: container.colour_model,
            depth: container.bit_depth,
            bytes_per_sample: container.bytes_per_sample,
            compression: container.compression,
        }
    }
}

pub fn assemble_layer(
    channel_area: &[u8],
    layer: &PspLayer,
    image: PspImageContext,
    bitmap_count: u16,
) -> Result<PspLayerPixels, FormatError> {
    let PspImageContext {
        version_major,
        colour_model,
        depth,
        bytes_per_sample,
        compression,
    } = image;
    let components = layer_components(colour_model, bitmap_count);
    let (height, null_layer) = if layer.height == 0 {
        (1u32, true)
    } else {
        (layer.height, false)
    };
    let line_width = layer_line_width(layer.width, components, bytes_per_sample, depth);
    let total = line_width
        .checked_mul(height as usize)
        .ok_or(FormatError::LimitExceeded("PSP layer buffer"))?;
    let mut data = vec![0u8; total];
    let mut skipped = Vec::new();

    if null_layer {
        return Ok(PspLayerPixels {
            width: layer.width,
            height,
            components,
            bytes_per_sample,
            data,
            skipped,
        });
    }

    let header_len: usize = if version_major < 4 { 14 } else { 10 };
    let mut cursor = 0usize;

    while cursor < channel_area.len() {
        if cursor + header_len > channel_area.len() {
            return Err(FormatError::Malformed(
                "PSP channel block header is truncated",
            ));
        }
        if &channel_area[cursor..cursor + 4] != BLOCK_SIGNATURE.as_slice() {
            return Err(FormatError::Malformed(
                "PSP channel block header signature missing",
            ));
        }
        let id = le_u16(channel_area, cursor + 4);
        if id != PSP_CHANNEL_BLOCK {
            return Err(FormatError::Malformed(
                "PSP layer holds a sub-block that is not a channel",
            ));
        }
        let first_len = le_u32(channel_area, cursor + 6);
        let (initial_len, total_len) = if version_major < 4 {
            (first_len, le_u32(channel_area, cursor + 10))
        } else {
            (0, first_len)
        };

        let start = cursor + header_len;
        let block_end = start
            .checked_add(total_len as usize)
            .filter(|end| *end <= channel_area.len())
            .ok_or(FormatError::Malformed(
                "PSP channel block runs past its layer",
            ))?;

        // Version 3 uses the header's initial length as the chunk length; version 4 reads one from
        // the chunk and requires at least 16 of it. Third place in this format where the two
        // versions answer the same question with different fields.
        let (chunk_len, mut at) = if version_major >= 4 {
            if start + 4 > block_end {
                return Err(FormatError::Malformed("PSP channel chunk is truncated"));
            }
            let declared = le_u32(channel_area, start);
            if declared < MIN_CHANNEL_CHUNK_V4 {
                return Err(FormatError::Malformed(
                    "PSP channel chunk declares too small a length",
                ));
            }
            (declared as usize, start + 4)
        } else {
            (initial_len as usize, start)
        };

        if at + 12 > block_end {
            return Err(FormatError::Malformed(
                "PSP channel information chunk is truncated",
            ));
        }
        let compressed_len = le_u32(channel_area, at) as usize;
        let _uncompressed_len = le_u32(channel_area, at + 4) as usize;
        let bitmap_type = le_u16(channel_area, at + 8);
        let channel_type = le_u16(channel_area, at + 10);
        at += 12;
        let _ = at;

        // Two NON-FATAL skips, both of which upstream reports and moves past. Collected rather
        // than swallowed so a caller can say what was dropped.
        if bitmap_type > PSP_DIB_USER_MASK {
            skipped.push(PspSkippedChannel::UnsupportedBitmapType(bitmap_type));
        } else if bitmap_type == PSP_DIB_USER_MASK {
            skipped.push(PspSkippedChannel::LayerMask);
        } else {
            validate_channel_type(colour_model, channel_type)?;

            let offset = channel_offset(
                bitmap_type,
                channel_type,
                colour_model,
                components,
                bytes_per_sample,
            );
            // The data begins `chunk_len` bytes into the channel block, not after the fields --
            // upstream seeks to `channel_start + chunk_len` before reading.
            let data_at = start + chunk_len;
            let payload = channel_area
                .get(data_at..block_end)
                .ok_or(FormatError::Malformed("PSP channel data is truncated"))?;
            let payload = payload
                .get(..compressed_len.min(payload.len()))
                .unwrap_or(payload);

            let layout = ChannelLayout {
                stride: components as usize * bytes_per_sample as usize,
                offset,
                bytes_per_sample,
            };
            let pixel_count = layer.width as usize * height as usize;

            match compression {
                PspCompression::None => {
                    read_uncompressed(payload, &mut data, layout, layer.width, height, depth)?
                }
                PspCompression::Rle => {
                    let span = if depth < 8 {
                        line_width * height as usize
                    } else {
                        pixel_count * layout.stride
                    };
                    decompress_rle(payload, &mut data, layout, span)?
                }
                PspCompression::Lz77 => decompress_lz77(payload, &mut data, layout, pixel_count)?,
            }
        }

        cursor = block_end;
    }

    Ok(PspLayerPixels {
        width: layer.width,
        height,
        components,
        bytes_per_sample,
        data,
        skipped,
    })
}

/// Field chunks inside the creator block carry **`~FL\0`, not `~BK\0`.**
///
/// This is the format's second magic and it is easy to miss: a reader that reuses the block-header
/// walk rejects every creator block it meets. The field header is also **version-independent** — 4
/// bytes of signature, a `u16` keyword and a `u32` length, with none of the initial/total split the
/// outer blocks grew at version 4.
const FIELD_SIGNATURE: &[u8; 4] = b"~FL\0";

/// What the creator block says about the image. Upstream assembles these into one comment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PspCreator {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub copyright: Option<String>,
    pub description: Option<String>,
}

impl PspCreator {
    /// The comment upstream builds, in upstream's order, or `None` when nothing was found.
    ///
    /// **The order is fixed and the copyright line carries a literal prefix**: title, artist,
    /// `"Copyright "` then the holder, description — each followed by a newline. Upstream attaches
    /// the result as a `gimp-comment` parasite **only when it is non-empty**, so an all-empty
    /// creator block leaves no comment rather than an empty one.
    pub fn comment(&self) -> Option<String> {
        let mut out = String::new();
        if let Some(title) = &self.title {
            out.push_str(title);
            out.push('\n');
        }
        if let Some(artist) = &self.artist {
            out.push_str(artist);
            out.push('\n');
        }
        if let Some(copyright) = &self.copyright {
            out.push_str("Copyright ");
            out.push_str(copyright);
            out.push('\n');
        }
        if let Some(description) = &self.description {
            out.push_str(description);
            out.push('\n');
        }
        if out.is_empty() { None } else { Some(out) }
    }
}

/// Read the creator block's field chunks.
///
/// **The four NUMERIC fields upstream reads are never used.** Creation date, modification date,
/// application id and application version are all declared
/// `guint32 __attribute__((unused))` — read from the file, assigned, and then nothing. Dead reads,
/// the same shape as `mantiuk06`'s declared-but-unread `detail` property. They are skipped here
/// rather than parsed into a struct nobody reads, and recorded so the omission is visibly
/// deliberate.
///
/// **A string that is not valid UTF-8 after conversion is IGNORED, not an error** — upstream prints
/// *"Invalid creator keyword ignored."* and carries on. Since the conversion is from ISO-8859-1,
/// where every byte is a valid code point, that branch is unreachable in practice; it is reproduced
/// as a skip anyway because the alternative is inventing a failure upstream does not have.
///
/// **Upstream's own note: *"PSP does not zero terminate strings"*** — the length is the only
/// terminator, so a reader looking for a NUL runs into the next field.
pub fn read_creator(block_data: &[u8]) -> Result<PspCreator, FormatError> {
    const FLD_TITLE: u16 = 0;
    const FLD_ARTIST: u16 = 3;
    const FLD_COPYRIGHT: u16 = 4;
    const FLD_DESCRIPTION: u16 = 5;

    let mut creator = PspCreator::default();
    let mut cursor = 0usize;

    while cursor < block_data.len() {
        if cursor + 10 > block_data.len() {
            return Err(FormatError::Malformed(
                "PSP creator field header is truncated",
            ));
        }
        if &block_data[cursor..cursor + 4] != FIELD_SIGNATURE.as_slice() {
            return Err(FormatError::Malformed(
                "PSP creator field header signature missing",
            ));
        }
        let keyword = le_u16(block_data, cursor + 4);
        let length = le_u32(block_data, cursor + 6) as usize;
        let at = cursor + 10;

        // Upstream's bounds test, which is against the BLOCK's end and not the file's.
        let end = at
            .checked_add(length)
            .filter(|end| *end <= block_data.len())
            .ok_or(FormatError::Malformed(
                "PSP creator field runs past its block",
            ))?;

        match keyword {
            FLD_TITLE | FLD_ARTIST | FLD_COPYRIGHT | FLD_DESCRIPTION => {
                // ISO-8859-1, as upstream converts it, despite the PSP8 specification calling the
                // strings ASCII -- upstream trusts the broader encoding, so this does too.
                let text: String = block_data[at..end].iter().map(|b| *b as char).collect();
                let slot = match keyword {
                    FLD_TITLE => &mut creator.title,
                    FLD_ARTIST => &mut creator.artist,
                    FLD_COPYRIGHT => &mut creator.copyright,
                    _ => &mut creator.description,
                };
                // Upstream frees any previous value, so a repeated keyword WINS rather than being
                // ignored -- last one through the loop is the one that survives.
                *slot = Some(text);
            }
            // The dead numeric reads, and every keyword upstream does not know: both end up
            // advancing by `length`. **Upstream's known-numeric branch does NOT do that** -- it
            // reads exactly four bytes whatever the declared length says, so a numeric field
            // declaring any other length desynchronises its walk. Advancing by the declared length
            // is the only reading that cannot desynchronise, and it agrees with upstream wherever
            // upstream is self-consistent.
            _ => {}
        }

        cursor = end;
    }

    Ok(creator)
}

/// Read the colour profile block and return the raw ICC bytes.
///
/// **The profile's size lives in the LAST FOUR BYTES of a variable-length header, which is what
/// upstream's `psp_header_size - 8` means.** The block opens with that header's own size; the
/// header then holds a `u16` length and PSP's internal name for the profile, neither of which
/// upstream wants; and its final four bytes are the ICC length. Four bytes are already consumed by
/// reading the header size and four more are about to be read at the end, hence minus eight.
///
/// A header size below 8 would make upstream's `fseek (…, SEEK_CUR)` move BACKWARDS over data it
/// has already read. Refused here, because a rewind is never what the format meant and the
/// alternative is parsing the header size as part of the profile.
pub fn read_colour_profile(block_data: &[u8]) -> Result<Vec<u8>, FormatError> {
    if block_data.len() < 4 {
        return Err(FormatError::Malformed(
            "PSP colour profile block is truncated",
        ));
    }
    let header_size = le_u32(block_data, 0) as usize;
    if header_size < 8 {
        return Err(FormatError::Malformed(
            "PSP colour profile header is too small to hold its own length fields",
        ));
    }

    // Skip to the header's last four bytes: four already read, then header_size - 8 of name.
    let size_at = 4 + (header_size - 8);
    if size_at + 4 > block_data.len() {
        return Err(FormatError::Malformed(
            "PSP colour profile header runs past its block",
        ));
    }
    let profile_size = le_u32(block_data, size_at) as usize;
    let profile_at = size_at + 4;

    block_data
        .get(profile_at..profile_at + profile_size)
        .map(|bytes| bytes.to_vec())
        .ok_or(FormatError::Malformed("PSP ICC profile is truncated"))
}
