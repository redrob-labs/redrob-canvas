//! CMYK soft proof through Little CMS, batch 4 item L5 step 1.
//!
//! Needs a CMYK ICC profile; uses the Ghostscript one most Linux systems ship and passes with a
//! note where there is none.

use redrob_core::cmyk::{CmykProfile, ProofIntent};

fn profile_bytes() -> Option<Vec<u8>> {
    [
        "/usr/share/color/icc/ghostscript/default_cmyk.icc",
        "/usr/share/color/icc/ghostscript/ps_cmyk.icc",
    ]
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
    assert_ne!(
        &pixels[0..3],
        &[0, 0, 255],
        "the proof changed the out-of-gamut blue"
    );
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
    assert!(
        inks[0].iter().all(|v| *v < 16),
        "white is no ink: {:?}",
        inks[0]
    );
    assert!(inks[1][3] > 128, "black uses K: {:?}", inks[1]);
}

#[test]
fn an_rgb_or_broken_profile_is_refused() {
    assert!(CmykProfile::parse(b"not a profile", ProofIntent::Perceptual).is_err());
}

#[test]
fn cmyk_tiff_is_a_readable_cmyk_file_with_the_profile_embedded() {
    let Some(bytes) = profile_bytes() else {
        return;
    };
    let profile = CmykProfile::parse(&bytes, ProofIntent::RelativeColorimetric).unwrap();
    // 2x1: transparent (prints as white paper) and opaque black.
    let tiff = profile
        .encode_tiff(2, 1, &[0, 0, 0, 0, 0, 0, 0, 255])
        .unwrap();
    assert_eq!(&tiff[0..4], b"II*\0");
    let mut decoder = tiff::decoder::Decoder::new(std::io::Cursor::new(&tiff)).unwrap();
    assert_eq!(decoder.colortype().unwrap(), tiff::ColorType::CMYK(8));
    assert_eq!(decoder.dimensions().unwrap(), (2, 1));
    assert_eq!(
        decoder.get_tag_u8_vec(tiff::tags::Tag::IccProfile).unwrap(),
        bytes
    );
    let tiff::decoder::DecodingResult::U8(inks) = decoder.read_image().unwrap() else {
        panic!("8-bit");
    };
    assert!(inks[0..4].iter().all(|v| *v < 16), "paper white: {inks:?}");
    assert!(inks[7] > 128, "black uses K: {inks:?}");
    assert!(profile.encode_tiff(3, 1, &[0; 8]).is_err());
}

#[test]
fn a_cmyk_document_keeps_its_pixels_printable_after_each_edit() {
    use redrob_core::{ColorMode, Command, Document, Editor, Pixel};
    let Some(bytes) = profile_bytes() else {
        return;
    };
    let profile = CmykProfile::parse(&bytes, ProofIntent::RelativeColorimetric).unwrap();
    let mut editor = Editor::new(Document::new(4, 4).unwrap()).unwrap();
    editor
        .execute(Command::ConvertColorMode {
            mode: ColorMode::Cmyk,
            palette: None,
            dither: Default::default(),
            cmyk_profile: Some(bytes.clone()),
        })
        .unwrap();
    assert_eq!(editor.document().color_mode(), ColorMode::Cmyk);
    assert_eq!(editor.document().cmyk_profile(), Some(bytes.as_slice()));
    // Pure sRGB blue cannot be printed; after the fill it is the press's nearest blue.
    editor.execute(Command::SelectAll).unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(0, 0, 255, 255),
        })
        .unwrap();
    let doc = editor.document();
    let pixels = doc.layer(doc.active_layer_id()).unwrap().pixels().to_vec();
    let mut expected = vec![0, 0, 255, 255];
    profile.soft_proof_rgba8(&mut expected, false);
    for c in 0..3 {
        assert!(
            pixels[c].abs_diff(expected[c]) <= 1,
            "{pixels:?} vs {expected:?}"
        );
    }
    // Back to RGB drops the profile.
    editor
        .execute(Command::ConvertColorMode {
            mode: ColorMode::Rgb,
            palette: None,
            dither: Default::default(),
            cmyk_profile: None,
        })
        .unwrap();
    assert!(editor.document().cmyk_profile().is_none());
}

#[test]
fn cmyk_mode_needs_a_profile() {
    use redrob_core::{ColorMode, Command, Document, Editor};
    let mut editor = Editor::new(Document::new(2, 2).unwrap()).unwrap();
    assert!(
        editor
            .execute(Command::ConvertColorMode {
                mode: ColorMode::Cmyk,
                palette: None,
                dither: Default::default(),
                cmyk_profile: None
            })
            .is_err()
    );
}

// ---- U7: CMYK PSD ----

fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}
fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

#[test]
fn a_cmyk_psd_is_cmyk_with_the_profile_and_layers_inside() {
    let Some(bytes) = profile_bytes() else {
        eprintln!("no CMYK profile on this machine; skipped");
        return;
    };
    let profile = CmykProfile::parse(&bytes, ProofIntent::RelativeColorimetric).unwrap();
    // One layer: left half black, right half transparent.
    let mut editor =
        redrob_core::Editor::new(redrob_core::Document::new(8, 4).unwrap()).unwrap();
    editor
        .execute(redrob_core::Command::SelectRectangle {
            rect: redrob_core::Rect::new(0, 0, 4, 4),
            mode: Default::default(),
        })
        .unwrap();
    editor
        .execute(redrob_core::Command::Fill {
            color: redrob_core::Pixel::rgba(0, 0, 0, 255),
        })
        .unwrap();
    let doc = editor.document();
    let psd = redrob_core::export_cmyk_psd(doc, doc.current_frame_id(), &profile).unwrap();

    assert_eq!(&psd[0..4], b"8BPS");
    assert_eq!(be16(&psd, 12), 4, "four composite channels (C, M, Y, K)");
    assert_eq!(be16(&psd, 24), 4, "colour mode 4 = CMYK");
    // Colour mode data (empty), then image resources holding the ICC profile as 1039.
    assert_eq!(be32(&psd, 26), 0);
    let resources = be32(&psd, 30) as usize;
    let block = &psd[34..34 + resources];
    assert_eq!(&block[0..4], b"8BIM");
    assert_eq!(be16(block, 4), 1039);
    let icc_len = be32(block, 8) as usize;
    assert_eq!(&block[12..12 + icc_len], profile.icc(), "the profile is embedded");

    // Layer info: one layer with five channels (C, M, Y, K, alpha).
    let layers = 34 + resources;
    let layer_info = layers + 4 + 4; // section length, layer info length
    assert_eq!(be16(&psd, layer_info), 1, "one layer");
    let channels = be16(&psd, layer_info + 2 + 16);
    assert_eq!(channels, 5);

    // The merged image is the last 2 + 4 * w * h bytes: raw, inverted inks, flattened on white.
    let n = 8 * 4;
    let merged = &psd[psd.len() - 4 * n..];
    let k = &merged[3 * n..4 * n];
    assert!(k[0] < 128, "black prints K ink (inverted: low value): {}", k[0]);
    for plane in 0..4 {
        assert_eq!(
            merged[plane * n + 7],
            255,
            "the transparent half is white paper, no ink"
        );
    }
}
