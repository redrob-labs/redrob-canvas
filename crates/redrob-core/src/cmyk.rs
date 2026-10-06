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

/// A loaded CMYK profile and the transforms built from it.
pub struct CmykProfile {
    proof: Transform<[u8; 4], [u8; 4]>,
    proof_gamut: Transform<[u8; 4], [u8; 4]>,
    separate: Transform<[u8; 4], [u8; 4]>,
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
        let proof_gamut = make_proof(Flags::SOFT_PROOFING | Flags::GAMUT_CHECK | Flags::COPY_ALPHA)?;
        let separate = Transform::new(&srgb, PixelFormat::RGBA_8, &cmyk, PixelFormat::CMYK_8, intent.lcms())
            .map_err(|_| CoreError::InvalidSemanticStyle)?;
        Ok(Self { proof, proof_gamut, separate })
    }

    /// Soft-proofs straight 8-bit RGBA in place: each colour becomes what the press would print,
    /// shown back in sRGB. With `gamut_check`, colours the press cannot reach show in Little CMS's
    /// alarm colour (grey by default) instead, as Photoshop's Gamut Warning. Alpha is kept.
    pub fn soft_proof_rgba8(&self, pixels: &mut [u8], gamut_check: bool) {
        let transform = if gamut_check { &self.proof_gamut } else { &self.proof };
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
