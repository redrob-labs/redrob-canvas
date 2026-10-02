// SPDX-License-Identifier: GPL-3.0-or-later

//! The ISO base media container shared by HEIF and AVIF (H.14): enough of the box structure to tell
//! the two apart and to read the primary image's declared size.
//!
//! Why this exists even though neither image is decoded yet. The two formats are the SAME container
//! with DIFFERENT codecs inside: HEIF carries HEVC, AVIF carries AV1. Folding them into one name, as
//! this product did, means an AVIF file is refused with a message about HEVC -- which sends the user
//! looking for the wrong thing entirely. Telling them apart needs the container read, and the
//! container is the half we can own without a codec.
//!
//! Reading `ispe` also turns a truncated or corrupt file into a MALFORMED error instead of an
//! "unsupported codec" one. That distinction is the whole value of the error: it says whether the file
//! or the product is the problem.
//!
//! On the codecs themselves, recorded here so the next pass does not re-derive it:
//!
//! * HEVC (HEIF) has no pure-Rust decoder. Wiring one means a C dependency, which this product does
//!   not take for a format it can already refuse cleanly.
//! * AV1 (AVIF) does have one -- `rav1d`, the Rust port of dav1d -- but it exposes dav1d's C-shaped
//!   API rather than an idiomatic one, so adopting it is a deliberate dependency decision that wants a
//!   build to verify against rather than a blind port.

use crate::{FormatError, Result};

/// Which codec a container declares through its brands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContainerBrand {
    /// AV1 inside the ISO base media container.
    Avif,
    /// HEVC inside the same container.
    Heif,
}

/// The primary image's declared geometry, when the container states one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ImageExtent {
    pub width: u32,
    pub height: u32,
}

/// Classifies a file by its `ftyp` brands.
///
/// Both the major brand and the COMPATIBLE brand list are considered, because a file whose major brand
/// is the generic `mif1` can still declare `avif` among its compatible brands -- and that is the brand
/// that says which codec is inside.
pub(crate) fn classify(bytes: &[u8]) -> Option<ContainerBrand> {
    if bytes.len() < 12 || &bytes[4..8] != b"ftyp" {
        return None;
    }
    let size = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    let end = size.clamp(12, bytes.len());
    let mut brands = Vec::new();
    brands.push(&bytes[8..12]);
    let mut at = 16usize;
    while at + 4 <= end {
        brands.push(&bytes[at..at + 4]);
        at += 4;
    }
    if brands
        .iter()
        .any(|brand| matches!(*brand, b"avif" | b"avis" | b"av01"))
    {
        return Some(ContainerBrand::Avif);
    }
    if brands
        .iter()
        .any(|brand| matches!(*brand, b"heic" | b"heif" | b"heix" | b"hevc" | b"mif1" | b"msf1"))
    {
        return Some(ContainerBrand::Heif);
    }
    None
}

/// Reads the primary item's `ispe` extent.
///
/// Walks the box tree rather than searching the bytes for `ispe`: a four-byte tag occurs inside
/// compressed image data often enough that a search finds one in the wrong place, and the size that
/// follows it would then be read as a dimension.
pub(crate) fn primary_extent(bytes: &[u8]) -> Result<ImageExtent> {
    let meta = find_box(bytes, b"meta").ok_or(FormatError::Malformed("HEIF/AVIF has no meta box"))?;
    // `meta` is a FULL box: a version byte and three flag bytes precede its children.
    let meta_children = meta
        .get(4..)
        .ok_or(FormatError::Malformed("HEIF/AVIF meta box truncated"))?;
    let iprp = find_box(meta_children, b"iprp")
        .ok_or(FormatError::Malformed("HEIF/AVIF has no item properties"))?;
    let ipco = find_box(iprp, b"ipco")
        .ok_or(FormatError::Malformed("HEIF/AVIF has no property container"))?;
    let ispe =
        find_box(ipco, b"ispe").ok_or(FormatError::Malformed("HEIF/AVIF has no image extent"))?;
    if ispe.len() < 12 {
        return Err(FormatError::Malformed("HEIF/AVIF image extent truncated").into());
    }
    // `ispe` is also a full box: four bytes of version and flags, then two 32-bit dimensions.
    let width = u32::from_be_bytes([ispe[4], ispe[5], ispe[6], ispe[7]]);
    let height = u32::from_be_bytes([ispe[8], ispe[9], ispe[10], ispe[11]]);
    if width == 0
        || height == 0
        || width > crate::document::MAX_DIMENSION
        || height > crate::document::MAX_DIMENSION
    {
        return Err(FormatError::Malformed("HEIF/AVIF dimensions out of range").into());
    }
    Ok(ImageExtent { width, height })
}

/// Finds a box's PAYLOAD among a sequence of boxes.
///
/// Handles the 64-bit size escape (a declared size of 1 means the real size follows as a u64) and the
/// "to end of file" form (size 0). Both appear in real files, and treating either as a literal length
/// walks straight off the end of the data.
fn find_box<'a>(bytes: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    let mut at = 0usize;
    while at + 8 <= bytes.len() {
        let declared =
            u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]) as u64;
        let tag = &bytes[at + 4..at + 8];
        let (header, size) = match declared {
            1 => {
                if at + 16 > bytes.len() {
                    return None;
                }
                let mut word = [0u8; 8];
                word.copy_from_slice(&bytes[at + 8..at + 16]);
                (16usize, u64::from_be_bytes(word))
            }
            0 => (8usize, (bytes.len() - at) as u64),
            _ => (8usize, declared),
        };
        let size = usize::try_from(size).ok()?;
        if size < header || at + size > bytes.len() {
            return None;
        }
        if tag == kind {
            return Some(&bytes[at + header..at + size]);
        }
        at += size;
    }
    None
}
