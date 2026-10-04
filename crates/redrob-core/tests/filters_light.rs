//! K.7, light and shadow.

use redrob_core::{Command, Document, Editor, Filter, Pixel, Rect, SelectionMode};

fn image(width: u32, height: u32, colors: &[Pixel]) -> Editor {
    let mut editor = Editor::new(Document::new(width, height).expect("document")).expect("editor");
    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let color = colors[(y as usize) * width as usize + x as usize];
            editor
                .execute(Command::SelectRectangle {
                    rect: Rect::new(x, y, 1, 1),
                    mode: SelectionMode::Replace,
                })
                .expect("select");
            editor.execute(Command::Fill { color }).expect("fill");
        }
    }
    editor.execute(Command::ClearSelection).expect("clear");
    editor
}

fn pixels(editor: &Editor) -> Vec<u8> {
    editor.document().layers()[0].pixels().to_vec()
}

fn flatten(colors: &[Pixel]) -> Vec<u8> {
    colors.iter().flat_map(|p| [p.r, p.g, p.b, p.a]).collect()
}

fn apply(size: usize, colors: &[Pixel], filter: Filter) -> Vec<u8> {
    let mut editor = image(size as u32, size as u32, colors);
    editor
        .execute(Command::ApplyFilter { filter })
        .expect("filter");
    pixels(&editor)
}

fn bloom(threshold: f64, radius: u32, strength: f64) -> Filter {
    Filter::Bloom {
        threshold,
        radius,
        strength,
    }
}

/// A bright patch on dark ground, for the spill tests.
fn bright_spot(size: usize) -> Vec<Pixel> {
    let mut colors = vec![Pixel::rgba(10, 10, 10, 255); size * size];
    for y in 14..18 {
        for x in 14..18 {
            colors[y * size + x] = Pixel::rgba(255, 255, 255, 255);
        }
    }
    colors
}

/// The THRESHOLD is what separates bloom from the softglow we already ship.
///
/// Upstream ships both, so the catalogue requires them to differ (cycle 68's route), and this is the
/// paired assertion cycle 82's rule asks for: the same input, the two filters, opposite answers.
/// Softglow screens a blur of the WHOLE image, so every pixel however dark contributes and a flat
/// field brightens. Bloom spills only what is above the threshold, so a flat field below it comes
/// back untouched.
///
/// Measured on a flat field of 100 (0.39 of full) under a threshold of 0.7: bloom returns it
/// unchanged, softglow takes it to **130**.
///
/// **An earlier version of this claim was that bloom is purely ADDITIVE so no pixel may darken.**
/// That is true of bloom and does NOT separate them — screen is monotone too, so our softglow never
/// darkens either. The invariant is pinned by its own test below, labelled as what it is.
#[test]
fn bloom_threshold_is_what_separates_it_from_softglow() {
    let size = 32usize;
    let flat = vec![Pixel::rgba(100, 100, 100, 255); size * size];
    let before = flatten(&flat);

    assert_eq!(
        apply(size, &flat, bloom(0.7, 8, 1.0)),
        before,
        "nothing is above the threshold, so bloom has nothing to spill"
    );

    let softened = apply(
        size,
        &flat,
        Filter::SoftGlow {
            radius: 8,
            amount: 0.5,
        },
    );
    assert_ne!(
        softened, before,
        "softglow glows from every pixel, so the same field must change"
    );
    assert_eq!(
        softened[0], 130,
        "and specifically it brightens, which is the contrast"
    );
}

/// Bloom only ADDS light: no pixel may come back darker than it went in.
///
/// True, exact, and **not** the thing that separates it from softglow — see the test above. It is
/// still worth pinning, because it is what distinguishes bloom from a blur: a blur would darken the
/// bright patch's own pixels while brightening its surroundings.
///
/// Measured: 0 pixels of 1024 darkened.
#[test]
fn bloom_never_darkens_a_pixel() {
    let size = 32usize;
    let colors = bright_spot(size);
    let before = flatten(&colors);
    let after = apply(size, &colors, bloom(0.5, 8, 1.0));

    let darkened = (0..size * size)
        .filter(|&i| (0..3).any(|c| after[i * 4 + c] < before[i * 4 + c]))
        .count();
    assert_eq!(
        darkened, 0,
        "bloom puts light on top of the image; a blur would have darkened the patch itself"
    );
    assert_ne!(after, before, "and it must actually do something");
}

/// The spill is LOCAL: near the bright patch brightens, far from it does not.
///
/// Measured on a 4x4 white patch at the centre of a dark field, radius 8: the pixel at (10, 16) goes
/// from 10 to **24**, while (2, 2) stays at **10**.
#[test]
fn bloom_spill_is_local_to_the_bright_area() {
    let size = 32usize;
    let colors = bright_spot(size);
    let before = flatten(&colors);
    let after = apply(size, &colors, bloom(0.5, 8, 1.0));
    let at = |out: &[u8], x: usize, y: usize| i32::from(out[(y * size + x) * 4]);

    assert_eq!(at(&before, 10, 16), 10);
    assert_eq!(
        at(&after, 10, 16),
        24,
        "six pixels from the patch, inside a radius of 8, must receive spill"
    );
    assert_eq!(
        at(&after, 2, 2),
        10,
        "a corner far outside the radius must be untouched"
    );
}

/// The radius is how far the spill reaches.
///
/// Measured as the columns of row 16 lifted above 15: **12** at radius 4, **24** at radius 12.
#[test]
fn bloom_radius_sets_the_reach() {
    let size = 32usize;
    let colors = bright_spot(size);
    let reach_for = |radius: u32| {
        let out = apply(size, &colors, bloom(0.5, radius, 1.0));
        (0..size).filter(|&x| out[(16 * size + x) * 4] > 15).count()
    };
    assert_eq!(reach_for(4), 12, "a radius of 4 reaches twelve columns");
    assert_eq!(reach_for(12), 24, "and a radius of 12 reaches twenty-four");
}

/// Strength 0 is the identity, and so is a threshold of 1.
///
/// Both are neutral settings rather than errors, as `Spiral`'s zero rotation and `Shift`'s zero
/// amount are. A threshold of exactly 1 also has to divide by a zero headroom, so it is the case
/// that would otherwise produce NaN.
#[test]
fn bloom_neutral_settings_are_the_identity() {
    let size = 32usize;
    let colors = bright_spot(size);
    let before = flatten(&colors);

    assert_eq!(
        apply(size, &colors, bloom(0.5, 8, 0.0)),
        before,
        "adding none of the spill changes nothing"
    );
    assert_eq!(
        apply(size, &colors, bloom(1.0, 8, 1.0)),
        before,
        "a threshold of 1 admits nothing, and must not divide by a zero headroom"
    );
}

/// Strength is monotone: more of it is more light.
#[test]
fn bloom_strength_is_monotone() {
    let size = 32usize;
    let colors = bright_spot(size);
    let at_point = |strength: f64| {
        let out = apply(size, &colors, bloom(0.5, 8, strength));
        i32::from(out[(16 * size + 10) * 4])
    };
    let values = [at_point(0.0), at_point(0.5), at_point(1.0), at_point(2.0)];
    assert!(
        values.windows(2).all(|w| w[1] >= w[0]),
        "raising the strength must not reduce the light: {values:?}"
    );
    assert!(
        values[3] > values[0],
        "and the span must be real: {values:?}"
    );
}

/// Out-of-range parameters are refused.
#[test]
fn bloom_refuses_bad_parameters() {
    let size = 8usize;
    let colors = vec![Pixel::rgba(100, 100, 100, 255); size * size];
    let refused = |threshold: f64, radius: u32, strength: f64| {
        let mut editor = image(size as u32, size as u32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: bloom(threshold, radius, strength),
            })
            .is_err()
    };
    assert!(refused(0.5, 0, 1.0), "a zero radius has no spill");
    assert!(refused(-0.1, 8, 1.0), "a threshold below zero");
    assert!(refused(1.5, 8, 1.0), "a threshold above one");
    assert!(refused(0.5, 8, -1.0), "a negative strength");
    assert!(refused(0.5, 8, 99.0), "a strength past the cap");
    assert!(refused(f64::NAN, 8, 1.0), "a non-finite threshold");
}

/// A saved command with nothing but the kind loads.
#[test]
fn bloom_deserialises_with_defaults() {
    let filter: Filter =
        serde_json::from_str(r#"{"kind":"bloom"}"#).expect("older saved commands must load");
    match filter {
        Filter::Bloom {
            threshold,
            radius,
            strength,
        } => {
            assert_eq!(threshold, 0.5);
            assert_eq!(radius, 10);
            assert_eq!(strength, 1.0);
        }
        other => panic!("wrong variant: {other:?}"),
    }
}
