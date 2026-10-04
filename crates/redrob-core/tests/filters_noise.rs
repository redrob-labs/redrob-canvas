//! K.9, noise and video.

use redrob_core::{Command, Editor, Filter, Pixel, VideoPattern};

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

const VIDEO_PATTERNS: [VideoPattern; 9] = [
    VideoPattern::Staggered,
    VideoPattern::LargeStaggered,
    VideoPattern::Striped,
    VideoPattern::WideStriped,
    VideoPattern::LongStaggered,
    VideoPattern::ThreeByThree,
    VideoPattern::LargeThreeByThree,
    VideoPattern::Hex,
    VideoPattern::Dots,
];

fn video(pattern: VideoPattern, additive: bool, rotated: bool) -> Filter {
    Filter::VideoDegradation {
        pattern,
        additive,
        rotated,
    }
}

/// Nine names at nine consecutive line numbers are nine patterns, so no two may render alike. This
/// is the test that catches a copy-pasted layout, which is the likeliest way to get a nine-variant
/// enum wrong.
///
/// Measured: 0 clashes across all 36 pairs.
#[test]
fn video_all_nine_patterns_are_distinct() {
    let field = flat(grey(200));
    let rendered: Vec<Vec<u8>> = VIDEO_PATTERNS
        .iter()
        .map(|&p| under(&field, video(p, false, false)))
        .collect();

    let mut clashes = Vec::new();
    for i in 0..VIDEO_PATTERNS.len() {
        for j in i + 1..VIDEO_PATTERNS.len() {
            if rendered[i] == rendered[j] {
                clashes.push((VIDEO_PATTERNS[i], VIDEO_PATTERNS[j]));
            }
        }
    }
    assert!(
        clashes.is_empty(),
        "every pattern must render differently, but these matched: {clashes:?}"
    );
}

/// `rotated` transposes the mask, and `Striped` is the pattern that makes that exact: its channel
/// depends on the column alone, so unrotated every COLUMN is uniform and rotated every ROW is.
///
/// Both measured, and both directions asserted -- a filter that ignored `rotated` would leave the
/// columns uniform in the second case and fail.
#[test]
fn video_rotation_swaps_the_uniform_axis() {
    let field = flat(grey(200));

    let upright = under(&field, video(VideoPattern::Striped, false, false));
    let columns_uniform =
        |p: &[u8]| (0..SIZE).all(|x| (0..SIZE).all(|y| p[(y * SIZE + x) * 4] == p[x * 4]));
    let rows_uniform =
        |p: &[u8]| (0..SIZE).all(|y| (0..SIZE).all(|x| p[(y * SIZE + x) * 4] == p[y * SIZE * 4]));

    assert!(
        columns_uniform(&upright),
        "striped varies with the column only, so columns are uniform"
    );
    assert!(
        !rows_uniform(&upright),
        "and rows are therefore not uniform"
    );

    let turned = under(&field, video(VideoPattern::Striped, false, true));
    assert!(
        rows_uniform(&turned),
        "rotated, it varies with the row only"
    );
    assert!(
        !columns_uniform(&turned),
        "and the columns stop being uniform"
    );
}

/// One assertion about the pair: replacing DROPS the unselected channels and so can only darken,
/// while adding lays the selected channel on top and so can only brighten.
///
/// Measured on a flat 200 field: replace produces 0 channels above 200, additive produces 0 below.
#[test]
fn video_additive_brightens_where_replacing_darkens() {
    let field = flat(grey(200));

    let replaced = under(&field, video(VideoPattern::Staggered, false, false));
    let added = under(&field, video(VideoPattern::Staggered, true, false));

    let brighter = (0..SIZE * SIZE * 4)
        .filter(|&i| i % 4 != 3 && replaced[i] > 200)
        .count();
    let darker = (0..SIZE * SIZE * 4)
        .filter(|&i| i % 4 != 3 && added[i] < 200)
        .count();

    assert_eq!(brighter, 0, "dropping a channel cannot brighten anything");
    assert_eq!(darker, 0, "laying one on top cannot darken anything");
    assert_ne!(replaced, added, "and the two modes must differ");
}

/// The blurb is load-bearing: `Simulate distortion produced by a fuzzy or low-res monitor` says the
/// mask is a SUB-PIXEL layout rather than a blur or a noise. So a flat grey field must come out as a
/// small set of pure channel colours, which is what a shadow mask does to white.
///
/// Measured: exactly 3 distinct colours from a flat 200.
#[test]
fn video_turns_a_flat_field_into_a_channel_mosaic() {
    let field = flat(grey(200));
    let out = under(&field, video(VideoPattern::Staggered, false, false));

    let colours: std::collections::HashSet<(u8, u8, u8)> = (0..SIZE * SIZE)
        .map(|i| (out[i * 4], out[i * 4 + 1], out[i * 4 + 2]))
        .collect();
    assert_eq!(
        colours.len(),
        3,
        "a three-channel mask on one grey gives three colours: {colours:?}"
    );
}

/// `Dots` is the one pattern whose name promises GAPS, and its layout leaves 12 of its 16 cells
/// neutral. On a flat field that is exactly three quarters of the pixels untouched.
///
/// Measured: 432 of 576 on a 24-square canvas, which is 12/16 exactly. The arithmetic is the test --
/// a layout with a different number of neutral cells would miss it.
#[test]
fn video_dots_leaves_three_quarters_of_the_field_alone() {
    let field = flat(grey(200));
    let out = under(&field, video(VideoPattern::Dots, false, false));

    let untouched = (0..SIZE * SIZE)
        .filter(|&i| out[i * 4] == 200 && out[i * 4 + 1] == 200 && out[i * 4 + 2] == 200)
        .count();
    assert_eq!(
        untouched * 16,
        SIZE * SIZE * 12,
        "12 of 16 cells are neutral, so three quarters survive: {untouched} of {}",
        SIZE * SIZE
    );
}

/// Three parameters, matching the po block's enum plus two toggles. The enum's first variant is
/// `Staggered` because that is the first of the nine strings, at line 42 -- the ORDER is read, and it
/// decides what an integer in a saved document means.
#[test]
fn video_deserialises_with_the_first_read_variant() {
    let filter: Filter =
        serde_json::from_str(r#"{"kind":"video_degradation"}"#).expect("deserialise");
    match filter {
        Filter::VideoDegradation {
            pattern,
            additive,
            rotated,
        } => {
            assert_eq!(
                pattern,
                VideoPattern::Staggered,
                "line 42 is `_Staggered`, so it is the first variant"
            );
            assert!(!additive && !rotated, "both toggles default off");
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

/// Top half white, bottom half black -- a sharp horizontal edge, which is the only input that can
/// show whether a smear has a DIRECTION.
fn split() -> Vec<Pixel> {
    let white = Pixel {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    };
    let black = Pixel {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };
    (0..SIZE * SIZE)
        .map(|i| if i / SIZE < SIZE / 2 { white } else { black })
        .collect()
}

/// How far white has travelled into the black half, and black into the white half.
///
/// Row 0 is excluded from the upward count because it clamps to itself and so can never receive
/// from elsewhere -- counting it would measure the edge policy rather than the filter.
fn bleed(out: &[u8]) -> (usize, usize) {
    let down = (SIZE / 2..SIZE)
        .flat_map(|y| (0..SIZE).map(move |x| (x, y)))
        .filter(|&(x, y)| out[(y * SIZE + x) * 4] > 128)
        .count();
    let up = (1..SIZE / 2)
        .flat_map(|y| (0..SIZE).map(move |x| (x, y)))
        .filter(|&(x, y)| out[(y * SIZE + x) * 4] <= 128)
        .count();
    (down, up)
}

/// THE assertion about the difference, and it is total rather than statistical: slur's upward bleed
/// is exactly ZERO at every amount, because every source is the row above. Pick draws from all eight
/// neighbours, so its bleed is symmetric.
///
/// Measured on a 32-square canvas split at row 16:
///
/// | amount | slur down / up | pick down / up |
/// |---|---|---|
/// | 1.0 | 32 / **0** | 14 / 12 |
/// | 0.5 | 15 / **0** | 5 / 5 |
///
/// Drawing from the row BELOW instead would invert the slur column exactly; drawing isotropically
/// would make it look like the pick column. Both are caught by the one zero.
#[test]
fn slur_runs_downward_where_pick_scatters_both_ways() {
    let field = split();

    for amount in [1.0f32, 0.5] {
        let (slur_down, slur_up) = bleed(&under(&field, Filter::Slur { amount, seed: 9 }));
        let (pick_down, pick_up) = bleed(&under(&field, Filter::Pick { amount, seed: 9 }));

        assert_eq!(
            slur_up, 0,
            "a slur never carries anything upward, at amount {amount}"
        );
        assert!(
            slur_down > 0,
            "but it does carry downward: {slur_down} at amount {amount}"
        );
        assert!(
            pick_up > 0 && pick_down > 0,
            "pick is isotropic, so it bleeds both ways: {pick_down} down, {pick_up} up"
        );
    }
}

/// At amount 1 every pixel is selected, and every source is one row up, so the whole image moves
/// down exactly one row.
///
/// Measured white counts in rows 14..18: `[32, 32, 32, 0, 0]` against the original
/// `[32, 32, 0, 0, 0]` -- the edge has moved from between rows 15 and 16 to between 16 and 17.
#[test]
fn slur_at_full_amount_moves_the_image_down_one_row() {
    let field = split();
    let out = under(
        &field,
        Filter::Slur {
            amount: 1.0,
            seed: 9,
        },
    );

    let white_in = |pixels: &[u8], row: usize| {
        (0..SIZE)
            .filter(|&x| pixels[(row * SIZE + x) * 4] > 128)
            .count()
    };

    assert_eq!(white_in(&out, 15), SIZE, "row 15 was already white");
    assert_eq!(
        white_in(&out, 16),
        SIZE,
        "row 16 took the white row above it"
    );
    assert_eq!(
        white_in(&out, 17),
        0,
        "and row 17 took row 16, which was black"
    );
}

/// Slur draws from its neighbours, so like pick and unlike hurl it can only move colours that are
/// already there. On a two-colour field it must never produce a third value.
#[test]
fn slur_keeps_the_palette_where_hurl_does_not() {
    let field = split();

    let slurred = under(
        &field,
        Filter::Slur {
            amount: 0.7,
            seed: 2,
        },
    );
    let invented = (0..SIZE * SIZE)
        .filter(|&i| slurred[i * 4] != 0 && slurred[i * 4] != 255)
        .count();
    assert_eq!(invented, 0, "a slur moves pixels, it does not mix them");

    let hurled = under(
        &field,
        Filter::Hurl {
            amount: 0.7,
            seed: 2,
        },
    );
    let hurl_invented = (0..SIZE * SIZE)
        .filter(|&i| hurled[i * 4] != 0 && hurled[i * 4] != 255)
        .count();
    assert!(
        hurl_invented > 0,
        "where hurl throws random colours in: {hurl_invented}"
    );
}

/// Amount 0 selects nothing, so nothing moves.
#[test]
fn slur_zero_amount_is_the_identity() {
    let field = split();
    assert_eq!(
        under(
            &field,
            Filter::Slur {
                amount: 0.0,
                seed: 9
            }
        ),
        flatten(&field),
        "no probability means no smear"
    );
}

/// The same `noise_unit` generator the shipped hurl, pick and spread already use, so the seed
/// replays exactly and a different one does not.
#[test]
fn slur_is_reproducible_from_its_seed() {
    let field = split();
    assert_eq!(
        under(
            &field,
            Filter::Slur {
                amount: 0.5,
                seed: 3
            }
        ),
        under(
            &field,
            Filter::Slur {
                amount: 0.5,
                seed: 3
            }
        ),
        "the same seed must replay exactly"
    );
    assert_ne!(
        under(
            &field,
            Filter::Slur {
                amount: 0.5,
                seed: 3
            }
        ),
        under(
            &field,
            Filter::Slur {
                amount: 0.5,
                seed: 4
            }
        ),
        "and a different seed must not"
    );
}

/// `amount` is a probability, so the range matches its two shipped siblings exactly.
#[test]
fn slur_refuses_an_amount_outside_the_family_range() {
    let field = split();
    for bad in [
        Filter::Slur {
            amount: -0.1,
            seed: 1,
        },
        Filter::Slur {
            amount: 1.1,
            seed: 1,
        },
        Filter::Slur {
            amount: f32::NAN,
            seed: 1,
        },
    ] {
        let mut editor = image(SIZE as u32, SIZE as u32, &field);
        assert!(
            editor
                .execute(Command::ApplyFilter { filter: bad })
                .is_err(),
            "an amount outside 0..1 must be refused, as for hurl and pick"
        );
    }
}

/// Two fields, matching its two shipped siblings.
#[test]
fn slur_deserialises_with_two_fields() {
    let filter: Filter = serde_json::from_str(r#"{"kind":"slur"}"#).expect("deserialise");
    match filter {
        Filter::Slur { amount, seed } => {
            assert!(amount.abs() < f32::EPSILON, "no smear by default");
            assert_eq!(seed, 0, "and a fixed seed");
        }
        other => panic!("wrong variant: {other:?}"),
    }
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
/// # Why one canvas of bands rather than seven flat ones
///
/// This test first built a separate flat canvas per level and was, at 12.79s, the slowest single
/// test in the suite -- found by AUDIT-9's suite re-measurement. Seven DISTINCT canvases defeat the
/// memoised builder by construction, since each is a genuine first build of 1024 command pairs.
///
/// One canvas carrying all seven levels as horizontal bands is one build, and it checks strictly
/// more: that the round trip is lossless at each level AND that neighbouring levels do not
/// interfere, since a filter reading across pixels would show up here and could not in a flat field.
#[test]
fn cie_lch_zero_amounts_are_exactly_the_identity() {
    let levels = [0u8, 1, 64, 128, 200, 255];
    let banded: Vec<Pixel> = (0..SIZE * SIZE)
        .map(|i| {
            let band = (i / SIZE) * levels.len() / SIZE;
            grey(levels[band.min(levels.len() - 1)])
        })
        .collect();

    assert_eq!(
        under(&banded, cie_lch(0.0, 0.0, 0.0, 7)),
        flatten(&banded),
        "the LCh round trip must be lossless at every grey level, and not mix the bands"
    );

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
