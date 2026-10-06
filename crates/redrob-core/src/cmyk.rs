// SPDX-License-Identifier: GPL-3.0-or-later

//! L5 step 1: profiled CMYK through Little CMS (the `lcms2` crate).
//!
//! What this gives: a CMYK ICC profile (a press or printer description the user loads) drives
//!
//! - **soft proofing** -- what the image would look like printed, shown without changing a pixel
//!   (View > Proof Colors, Ctrl+Y), with an optional gamut warning colour;
//! - **separations** -- each pixel's C, M, Y and K through the profile, for export.
//!
//! What it does not give yet: a document whose working space IS CMYK. That changes every pixel
//! path and is its own item (L5 step 2).

use lcms2::{Flags, Intent, PixelFormat, Profile, Transform};

use crate::error::{CoreError, Result};

/// L5c: the parsed profile for these bytes, built once and reused (building Little CMS
/// transforms is far slower than one edit's pixels). Relative colorimetric, as Photoshop's
/// mode conversion defaults to. `None` for bytes that are not a CMYK profile.
pub(crate) fn cached_profile(bytes: &[u8]) -> Option<std::rc::Rc<CmykProfile>> {
    use std::hash::{Hash, Hasher};
    // Per thread: Little CMS transforms with a cache are Send but not Sync.
    thread_local! {
        static CACHE: std::cell::RefCell<Option<(u64, std::rc::Rc<CmykProfile>)>> = const { std::cell::RefCell::new(None) };
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    let key = hasher.finish();
    CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some((cached, profile)) = cache.as_ref() {
            if *cached == key {
                return Some(std::rc::Rc::clone(profile));
            }
        }
        let profile =
            std::rc::Rc::new(CmykProfile::parse(bytes, ProofIntent::RelativeColorimetric).ok()?);
        *cache = Some((key, std::rc::Rc::clone(&profile)));
        Some(profile)
    })
}

/// A loaded CMYK profile and the transforms built from it.
pub struct CmykProfile {
    proof: Transform<[u8; 4], [u8; 4]>,
    proof_gamut: Transform<[u8; 4], [u8; 4]>,
    separate: Transform<[u8; 4], [u8; 4]>,
    icc: Vec<u8>,
}

/// Baseline little-endian TIFF: CMYK 8-bit (Photometric 5, InkSet 1), one uncompressed strip,
/// the ICC profile in tag 34675.
fn write_cmyk_tiff(width: u32, height: u32, inks: &[[u8; 4]], icc: &[u8]) -> Vec<u8> {
    const SHORT: u16 = 3;
    const LONG: u16 = 4;
    const UNDEFINED: u16 = 7;
    let pixel_len = inks.len() * 4;
    // Layout: header (8) | pixels | bits-per-sample (8) | icc (padded even) | IFD.
    let pixels_at = 8_u32;
    let bps_at = pixels_at + pixel_len as u32;
    let icc_at = bps_at + 8;
    let mut ifd_at = icc_at + icc.len() as u32;
    ifd_at += ifd_at & 1;
    let mut out = Vec::with_capacity(ifd_at as usize + 200);
    out.extend_from_slice(b"II");
    out.extend_from_slice(&42_u16.to_le_bytes());
    out.extend_from_slice(&ifd_at.to_le_bytes());
    for ink in inks {
        out.extend_from_slice(ink);
    }
    for _ in 0..4 {
        out.extend_from_slice(&8_u16.to_le_bytes());
    }
    out.extend_from_slice(icc);
    if out.len() % 2 == 1 {
        out.push(0);
    }
    // (tag, type, count, value-or-offset), sorted by tag as TIFF requires.
    let mut entries: Vec<(u16, u16, u32, u32)> = vec![
        (256, LONG, 1, width),
        (257, LONG, 1, height),
        (258, SHORT, 4, bps_at),
        (259, SHORT, 1, 1),
        (262, SHORT, 1, 5),
        (273, LONG, 1, pixels_at),
        (277, SHORT, 1, 4),
        (278, LONG, 1, height),
        (279, LONG, 1, pixel_len as u32),
        (284, SHORT, 1, 1),
        (332, SHORT, 1, 1),
    ];
    if !icc.is_empty() {
        entries.push((34675, UNDEFINED, icc.len() as u32, icc_at));
    }
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    for (tag, kind, n, value) in entries {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&n.to_le_bytes());
        if kind == SHORT && n == 1 {
            // A single SHORT sits left-justified in the value field.
            out.extend_from_slice(&(value as u16).to_le_bytes());
            out.extend_from_slice(&0_u16.to_le_bytes());
        } else {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    out.extend_from_slice(&0_u32.to_le_bytes());
    out
}

/// Rendering intent, as the ICC defines them (Photoshop's Proof Setup offers the same four).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ProofIntent {
    Perceptual,
    #[default]
    RelativeColorimetric,
    Saturation,
    AbsoluteColorimetric,
}

impl ProofIntent {
    pub fn from_index(index: u32) -> Self {
        match index {
            0 => Self::Perceptual,
            2 => Self::Saturation,
            3 => Self::AbsoluteColorimetric,
            _ => Self::RelativeColorimetric,
        }
    }
    fn lcms(self) -> Intent {
        match self {
            Self::Perceptual => Intent::Perceptual,
            Self::RelativeColorimetric => Intent::RelativeColorimetric,
            Self::Saturation => Intent::Saturation,
            Self::AbsoluteColorimetric => Intent::AbsoluteColorimetric,
        }
    }
}

impl CmykProfile {
    /// Parses an ICC profile and refuses anything that is not CMYK.
    pub fn parse(bytes: &[u8], intent: ProofIntent) -> Result<Self> {
        let cmyk = Profile::new_icc(bytes).map_err(|_| CoreError::InvalidSemanticStyle)?;
        if cmyk.color_space() != lcms2::ColorSpaceSignature::CmykData {
            return Err(CoreError::InvalidSemanticStyle);
        }
        let srgb = Profile::new_srgb();
        let make_proof = |flags: Flags| {
            Transform::new_proofing(
                &srgb,
                PixelFormat::RGBA_8,
                &srgb,
                PixelFormat::RGBA_8,
                &cmyk,
                intent.lcms(),
                Intent::RelativeColorimetric,
                flags,
            )
            .map_err(|_| CoreError::InvalidSemanticStyle)
        };
        let proof = make_proof(Flags::SOFT_PROOFING | Flags::COPY_ALPHA)?;
        let proof_gamut =
            make_proof(Flags::SOFT_PROOFING | Flags::GAMUT_CHECK | Flags::COPY_ALPHA)?;
        let separate = Transform::new(
            &srgb,
            PixelFormat::RGBA_8,
            &cmyk,
            PixelFormat::CMYK_8,
            intent.lcms(),
        )
        .map_err(|_| CoreError::InvalidSemanticStyle)?;
        Ok(Self {
            proof,
            proof_gamut,
            separate,
            icc: bytes.to_vec(),
        })
    }

    /// L5b: a CMYK TIFF of straight 8-bit RGBA, separated through this profile, which is embedded
    /// (ICC tag) so the print shop reads the same inks. CMYK has no transparency in print, so the
    /// image is flattened on white first, as a press sheet is. Uncompressed baseline TIFF,
    /// one strip, little-endian.
    pub fn encode_tiff(&self, width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>> {
        let count = width as usize * height as usize;
        if width == 0 || height == 0 || rgba.len() != count * 4 {
            return Err(CoreError::InvalidSemanticStyle);
        }
        let mut flat = rgba.to_vec();
        for px in flat.chunks_exact_mut(4) {
            let a = u32::from(px[3]);
            for c in &mut px[..3] {
                *c = ((u32::from(*c) * a + 255 * (255 - a) + 127) / 255) as u8;
            }
            px[3] = 255;
        }
        let inks = self.separate_rgba8(&flat);
        Ok(write_cmyk_tiff(width, height, &inks, &self.icc))
    }

    /// Soft-proofs straight 8-bit RGBA in place: each colour becomes what the press would print,
    /// shown back in sRGB. With `gamut_check`, colours the press cannot reach show in Little CMS's
    /// alarm colour (grey by default) instead, as Photoshop's Gamut Warning. Alpha is kept.
    pub fn soft_proof_rgba8(&self, pixels: &mut [u8], gamut_check: bool) {
        let transform = if gamut_check {
            &self.proof_gamut
        } else {
            &self.proof
        };
        let (chunks, _) = pixels.as_chunks_mut::<4>();
        transform.transform_in_place(chunks);
    }

    /// C, M, Y, K (0..=255, 255 = full ink) for each straight 8-bit RGBA pixel.
    pub fn separate_rgba8(&self, pixels: &[u8]) -> Vec<[u8; 4]> {
        let (chunks, _) = pixels.as_chunks::<4>();
        let mut out = vec![[0_u8; 4]; chunks.len()];
        self.separate.transform_pixels(chunks, &mut out);
        out
    }
}
