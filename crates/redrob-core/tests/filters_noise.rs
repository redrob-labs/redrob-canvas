//! K.9, noise and video.

use redrob_core::{Command, Editor, Filter, Pixel};

#[path = "common/canvas.rs"]
mod canvas;

/// Build a test canvas. Memoised on its content by `common/canvas.rs`.
fn image(width: u32, height: u32, colors: &[Pixel]) -> Editor {
    canvas::editor(width, height, colors)
}

fn pixels(editor: &Editor) -> Vec<u8> {
    editor.document().layers()[0].pixels().to_vec()
}

const SIZE: usize = 32;

fn flat(colour: Pixel) -> Vec<Pixel> {
    vec![colour; SIZE * SIZE]
}

fn grey(level: u8) -> Pixel {
    Pixel {
        r: level,
        g: level,
        b: level,
        a: 255,
    }
}

const WARM: Pixel = Pixel {
    r: 200,
    g: 60,
    b: 30,
    a: 255,
};

fn under(field: &[Pixel], filter: Filter) -> Vec<u8> {
    let mut editor = image(SIZE as u32, SIZE as u32, field);
    editor
        .execute(Command::ApplyFilter { filter })
        .expect("filter");
    pixels(&editor)
}

fn flatten(colors: &[Pixel]) -> Vec<u8> {
    colors.iter().flat_map(|p| [p.r, p.g, p.b, p.a]).collect()
}

fn cie_lch(lightness: f64, chroma: f64, hue: f64, seed: u32) -> Filter {
    Filter::NoiseCieLch {
        lightness,
        chroma,
        hue,
        seed,
    }
}

/// THE structural assertion, and it explains a difference that was READ from upstream rather than
/// assumed.
///
/// `filters-actions.c` gates `noise-hsv` on `writable && !gray` and gates `noise-cie-lch` not at
/// all. The reason is in the geometry of the two spaces: a grey pixel has chroma zero, so its hue is
/// the angle of a zero-length vector and rotating it cannot change anything -- while its **L** is
/// perfectly well defined. So hue noise is a no-op on grey and lightness noise is not, which is
/// exactly why the gate would be wrong here.
///
/// Measured: hue noise at 180 degrees leaves a grey field byte-identical, and changes a coloured one.
#[test]
fn cie_lch_hue_noise_cannot_touch_grey_but_does_touch_colour() {
    let dull = flat(grey(128));
    let before_grey = flatten(&dull);
    assert_eq!(
        under(&dull, cie_lch(0.0, 0.0, 180.0, 11)),
        before_grey,
        "rotating the hue of a zero-length chroma vector changes nothing"
    );

    let warm = flat(WARM);
    let before_warm = flatten(&warm);
    assert_ne!(
        under(&warm, cie_lch(0.0, 0.0, 180.0, 11)),
        before_warm,
        "but a coloured pixel has a hue to rotate"
    );
}

/// The other half of the gating argument: lightness is meaningful on grey, so this filter does work
/// there where its HSV sibling is refused outright.
///
/// Measured: 102 distinct values across a 32-square grey field.
#[test]
fn cie_lch_lightness_noise_works_on_grey() {
    let dull = flat(grey(128));
    let out = under(&dull, cie_lch(0.2, 0.0, 0.0, 11));

    let distinct: std::collections::HashSet<u8> = (0..SIZE * SIZE).map(|i| out[i * 4]).collect();
    assert!(
        distinct.len() > 20,
        "lightness is well defined on grey, so the noise must bite: {} distinct",
        distinct.len()
    );
}

/// Chroma completes the triple: lengthening a zero-length vector DOES change a grey pixel, even
/// though rotating it does not. So of the three channels, exactly one is inert on grey.
#[test]
fn cie_lch_chroma_noise_does_change_grey() {
    let dull = flat(grey(128));
    let before = flatten(&dull);
    assert_ne!(
        under(&dull, cie_lch(0.0, 0.2, 0.0, 11)),
        before,
        "giving a grey pixel chroma moves it off the neutral axis"
    );
}

/// With every amount zero the filter is the identity, and that is an exact claim rather than an
/// approximate one: the round trip sRGB to XYZ to Lab to LCh and back is lossless at 8 bits.
///
/// Measured exactly at grey 0, 1, 64, 128, 200 and 255, and on a saturated colour. Checked before
/// the assertion was written, because a lossy round trip would have made this test a tolerance
/// question instead of an equality.
#[test]
fn cie_lch_zero_amounts_are_exactly_the_identity() {
    for level in [0u8, 1, 64, 128, 200, 255] {
        let dull = flat(grey(level));
        assert_eq!(
            under(&dull, cie_lch(0.0, 0.0, 0.0, 7)),
            flatten(&dull),
            "the LCh round trip must be lossless at grey {level}"
        );
    }

    let warm = flat(WARM);
    assert_eq!(
        under(&warm, cie_lch(0.0, 0.0, 0.0, 7)),
        flatten(&warm),
        "and on a saturated colour too"
    );
}

/// The jitter comes from `noise_unit`, the generator the shipped RGB and HSV noise already use, with
/// a distinct stream per channel. Equal by construction with its siblings rather than a second
/// implementation that could drift -- so the same seed replays exactly and a different one does not.
#[test]
fn cie_lch_is_reproducible_from_its_seed() {
    let dull = flat(grey(128));
    assert_eq!(
        under(&dull, cie_lch(0.2, 0.1, 30.0, 5)),
        under(&dull, cie_lch(0.2, 0.1, 30.0, 5)),
        "the same seed must replay exactly"
    );
    assert_ne!(
        under(&dull, cie_lch(0.2, 0.1, 30.0, 5)),
        under(&dull, cie_lch(0.2, 0.1, 30.0, 6)),
        "and a different seed must not"
    );
}

/// Alpha is not a colour channel, so no colour-space noise may touch it.
#[test]
fn cie_lch_leaves_alpha_alone() {
    let translucent = flat(Pixel {
        r: 200,
        g: 60,
        b: 30,
        a: 90,
    });
    let out = under(&translucent, cie_lch(0.5, 0.5, 180.0, 3));
    assert!(
        (0..SIZE * SIZE).all(|i| out[i * 4 + 3] == 90),
        "alpha must be carried through untouched"
    );
}

/// Ranges ours -- nothing upstream declares any, because nothing upstream declares the parameters.
/// Hue is in degrees because that is what `lab_to_lch` returns.
#[test]
fn cie_lch_refuses_amounts_outside_our_ranges() {
    let dull = flat(grey(128));
    for bad in [
        cie_lch(-0.1, 0.0, 0.0, 1),
        cie_lch(1.1, 0.0, 0.0, 1),
        cie_lch(0.0, -0.1, 0.0, 1),
        cie_lch(0.0, 1.1, 0.0, 1),
        cie_lch(0.0, 0.0, -1.0, 1),
        cie_lch(0.0, 0.0, 361.0, 1),
        cie_lch(f64::NAN, 0.0, 0.0, 1),
    ] {
        let mut editor = image(SIZE as u32, SIZE as u32, &dull);
        assert!(
            editor
                .execute(Command::ApplyFilter { filter: bad })
                .is_err(),
            "an amount outside our declared range must be refused"
        );
    }
}

/// Three amounts, one per channel of the space the name points at, plus the seed both shipped
/// siblings already carry. `holdness` is deliberately absent -- see the variant.
#[test]
fn cie_lch_deserialises_with_four_fields() {
    let filter: Filter = serde_json::from_str(r#"{"kind":"noise_cie_lch"}"#).expect("deserialise");
    match filter {
        Filter::NoiseCieLch {
            lightness,
            chroma,
            hue,
            seed,
        } => {
            assert!(
                lightness.abs() < f64::EPSILON
                    && chroma.abs() < f64::EPSILON
                    && hue.abs() < f64::EPSILON,
                "no noise by default, so the default is the identity"
            );
            assert_eq!(seed, 0, "and a fixed seed");
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

/// `lch_to_lab` is the exact inverse of `lab_to_lch`, equal by construction rather than tested to
/// agree. This pins that, because the whole filter rests on the pair being inverses.
#[test]
fn lch_and_lab_conversions_are_inverses() {
    for &(l, a, b) in &[
        (0.0f64, 0.0, 0.0),
        (50.0, 20.0, -30.0),
        (100.0, -60.0, 45.0),
        (72.5, 0.0, 12.0),
    ] {
        let (ll, c, h) = redrob_core::color::lab_to_lch(l, a, b);
        let (l2, a2, b2) = redrob_core::color::lch_to_lab(ll, c, h);
        assert!(
            (l - l2).abs() < 1e-9 && (a - a2).abs() < 1e-9 && (b - b2).abs() < 1e-9,
            "round trip of ({l},{a},{b}) gave ({l2},{a2},{b2})"
        );
    }
}
