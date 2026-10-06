//! CMYK soft proof through Little CMS, batch 4 item L5 step 1.
//!
//! Needs a CMYK ICC profile; uses the Ghostscript one most Linux systems ship and passes with a
//! note where there is none.

use redrob_core::cmyk::{CmykProfile, ProofIntent};

fn profile_bytes() -> Option<Vec<u8>> {
    ["/usr/share/color/icc/ghostscript/default_cmyk.icc", "/usr/share/color/icc/ghostscript/ps_cmyk.icc"]
        .iter()
        .find_map(|p| std::fs::read(p).ok())
}

#[test]
fn a_saturated_rgb_colour_dulls_in_the_proof_and_alpha_is_kept() {
    let Some(bytes) = profile_bytes() else {
        eprintln!("no CMYK profile on this machine; skipped");
        return;
    };
    let profile = CmykProfile::parse(&bytes, ProofIntent::RelativeColorimetric).unwrap();
    // Pure sRGB blue is far outside any press gamut.
    let mut pixels = vec![0, 0, 255, 128, 128, 128, 128, 255];
    profile.soft_proof_rgba8(&mut pixels, false);
    assert_ne!(&pixels[0..3], &[0, 0, 255], "the proof changed the out-of-gamut blue");
    assert_eq!(pixels[3], 128, "alpha untouched");
    // A mid grey is printable and stays close.
    for c in &pixels[4..7] {
        assert!((i32::from(*c) - 128).abs() < 24, "{pixels:?}");
    }
}

#[test]
fn separations_put_ink_where_the_colour_is() {
    let Some(bytes) = profile_bytes() else {
        return;
    };
    let profile = CmykProfile::parse(&bytes, ProofIntent::Perceptual).unwrap();
    let inks = profile.separate_rgba8(&[255, 255, 255, 255, 0, 0, 0, 255]);
    assert!(inks[0].iter().all(|v| *v < 16), "white is no ink: {:?}", inks[0]);
    assert!(inks[1][3] > 128, "black uses K: {:?}", inks[1]);
}

#[test]
fn an_rgb_or_broken_profile_is_refused() {
    assert!(CmykProfile::parse(b"not a profile", ProofIntent::Perceptual).is_err());
}
