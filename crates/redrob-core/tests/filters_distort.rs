//! K.5, distorts and projections.

use redrob_core::{Command, Document, Editor, Filter, Pixel, Rect, SelectionMode};

/// Build an editor holding one layer painted from `colors`, row-major.
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

/// Concentric rings become horizontal stripes. This is what "polar coordinates" MEANS.
///
/// The derivation for this filter rests on the name rather than on any upstream source — every one
/// of the five is empty but for the ellipsis — so the test has to come from the name too. A ring is
/// the locus of constant radius; the output's y axis is radius; therefore each output row must be
/// one uniform value.
///
/// **This test is what rejected the first implementation.** Normalising the radius per angle, so
/// the mapping would be an exact bijection of the rectangle, makes an output row a scaled copy of
/// the rectangle's BOUNDARY rather than a curve of constant radius — rings came out as stripes only
/// approximately, spread 26 on a row that should be flat. The name won over the bijection, because
/// the name is the only source this filter has.
///
/// Rows past the inscribed radius are excluded, and the cut-off is arithmetic rather than fitted:
/// the radius scale is half the diagonal, so a row first exceeds the rectangle's half-width at
/// `32 / (32·√2) · 64` = **row 45**. Measured spread is ≤ 8 for every row up to 45 and starts
/// climbing at exactly 46 — so the exclusion lands where the arithmetic says it must, and the
/// second assertion checks the clamping beyond it is real rather than imagined.
#[test]
fn polar_turns_rings_into_stripes() {
    let size = 64usize;
    let centre = size as f64 / 2.0;
    let colors: Vec<Pixel> = (0..size * size)
        .map(|index| {
            let x = (index % size) as f64 + 0.5 - centre;
            let y = (index / size) as f64 + 0.5 - centre;
            // Value depends ONLY on distance from the centre.
            let v = ((x * x + y * y).sqrt() * 6.0) as u32 % 256;
            Pixel::rgba(v as u8, v as u8, v as u8, 255)
        })
        .collect();

    let mut editor = image(size as u32, size as u32, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::PolarCoordinates { to_polar: true },
        })
        .unwrap();
    let out = pixels(&editor);

    // The inscribed limit, computed rather than hard-coded.
    let limit = (centre / centre.hypot(centre) * size as f64) as usize;
    assert_eq!(
        limit, 45,
        "the inscribed limit must be where the arithmetic puts it"
    );

    for y in 1..=limit {
        let row: Vec<i32> = (0..size)
            .map(|x| i32::from(out[(y * size + x) * 4]))
            .collect();
        let low = *row.iter().min().expect("non-empty");
        let high = *row.iter().max().expect("non-empty");
        assert!(
            high - low <= 8,
            "row {y} must be a stripe of near-constant value, spread was {}",
            high - low
        );
    }

    // And beyond it the clamping is real, not imagined -- so the exclusion above describes
    // behaviour rather than hiding a failure.
    let outer: Vec<i32> = (0..size)
        .map(|x| i32::from(out[(60 * size + x) * 4]))
        .collect();
    let outer_spread =
        outer.iter().max().expect("non-empty") - outer.iter().min().expect("non-empty");
    assert!(
        outer_spread > 8,
        "row 60 lies outside the inscribed radius and must show the clamping, got {outer_spread}"
    );
}

/// Horizontal stripes become concentric rings — the converse, and the inverse direction.
#[test]
fn polar_inverse_turns_stripes_into_rings() {
    let size = 64usize;
    let colors: Vec<Pixel> = (0..size * size)
        .map(|index| {
            // Value depends ONLY on the row.
            let v = ((index / size) * 4) as u8;
            Pixel::rgba(v, v, v, 255)
        })
        .collect();

    let mut editor = image(size as u32, size as u32, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::PolarCoordinates { to_polar: false },
        })
        .unwrap();
    let out = pixels(&editor);

    // Pixels at equal radius must now hold equal values. Checked on four points of a circle, which
    // a transform about the wrong centre — or about one axis only — could not satisfy.
    let centre = size as f64 / 2.0;
    for radius in [8.0f64, 16.0, 24.0] {
        let sample = |angle: f64| {
            let x = (centre + radius * angle.cos()).floor() as usize;
            let y = (centre + radius * angle.sin()).floor() as usize;
            i32::from(out[(y.min(size - 1) * size + x.min(size - 1)) * 4])
        };
        let east = sample(0.0);
        let north = sample(std::f64::consts::FRAC_PI_2);
        let west = sample(std::f64::consts::PI);
        let south = sample(3.0 * std::f64::consts::FRAC_PI_2);
        let values = [east, north, west, south];
        let low = *values.iter().min().expect("non-empty");
        let high = *values.iter().max().expect("non-empty");
        assert!(
            high - low <= 24,
            "at radius {radius} the four compass points must agree: {values:?}"
        );
    }
}

/// The round trip returns the image, because the mapping is a bijection of the rectangle.
///
/// This is what the per-angle radius normalisation buys, and the reason it is worth the arithmetic:
/// normalising by a fixed radius would inscribe a disc, throw the corners away, and make this test
/// impossible to pass. Sampling is nearest so the recovery is approximate, but it must be close
/// over most of the image.
#[test]
fn polar_round_trip_recovers_the_image() {
    let size = 64usize;
    let colors: Vec<Pixel> = (0..size * size)
        .map(|index| {
            let x = (index % size) as u8;
            let y = (index / size) as u8;
            Pixel::rgba(x * 4, y * 4, 128, 255)
        })
        .collect();

    let mut editor = image(size as u32, size as u32, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::PolarCoordinates { to_polar: true },
        })
        .unwrap();
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::PolarCoordinates { to_polar: false },
        })
        .unwrap();
    let out = pixels(&editor);

    // Count how many pixels came back close to where they started. The centre is the one place
    // where this must be loose: every angle maps to it, so a whole output row collapses onto a few
    // pixels and the inverse cannot pick them apart.
    let mut close = 0usize;
    let mut total = 0usize;
    let centre = size as f64 / 2.0;
    for y in 0..size {
        for x in 0..size {
            let dx = x as f64 + 0.5 - centre;
            let dy = y as f64 + 0.5 - centre;
            if dx.hypot(dy) < 6.0 {
                continue;
            }
            total += 1;
            let index = y * size + x;
            let want = i32::from(colors[index].r);
            let got = i32::from(out[index * 4]);
            if (want - got).abs() <= 16 {
                close += 1;
            }
        }
    }
    assert!(
        close * 10 >= total * 8,
        "the round trip must recover most of the image: {close} of {total} within tolerance"
    );
}

/// A flat field survives either direction exactly — resampling must not invent values.
#[test]
fn polar_on_a_flat_field_changes_nothing() {
    let colors = vec![Pixel::rgba(70, 130, 180, 255); 32 * 32];
    for to_polar in [true, false] {
        let mut editor = image(32, 32, &colors);
        let before = pixels(&editor);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::PolarCoordinates { to_polar },
            })
            .unwrap();
        assert_eq!(
            pixels(&editor),
            before,
            "a flat field must survive to_polar={to_polar} exactly"
        );
    }
}

/// The two directions are genuinely different transforms.
#[test]
fn polar_directions_differ() {
    let colors: Vec<Pixel> = (0..32 * 32)
        .map(|index| {
            let x = (index % 32) as u8;
            let y = (index / 32) as u8;
            Pixel::rgba(x * 8, y * 8, 90, 255)
        })
        .collect();

    let under = |to_polar: bool| {
        let mut editor = image(32, 32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::PolarCoordinates { to_polar },
            })
            .unwrap();
        pixels(&editor)
    };

    assert_ne!(
        under(true),
        under(false),
        "forward and inverse must not coincide"
    );
}

/// The top row of a to-polar result is the pole, so it samples near the image centre.
///
/// Names the wrong behaviour: a transform taking the pole at a corner — the easy mistake, since
/// pixel coordinates start there — would make this row sample the corner instead, and on this input
/// the corner and the centre are far apart by construction.
#[test]
fn polar_pole_is_the_image_centre_not_a_corner() {
    let size = 64usize;
    let centre = size / 2;
    let colors: Vec<Pixel> = (0..size * size)
        .map(|index| {
            let x = index % size;
            let y = index / size;
            // Bright only near the centre, dark at the corners.
            let near = x.abs_diff(centre) < 4 && y.abs_diff(centre) < 4;
            let v = if near { 240u8 } else { 20 };
            Pixel::rgba(v, v, v, 255)
        })
        .collect();

    let mut editor = image(size as u32, size as u32, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::PolarCoordinates { to_polar: true },
        })
        .unwrap();
    let out = pixels(&editor);

    for x in 0..size {
        assert_eq!(
            out[x * 4],
            240,
            "the pole row must sample the bright centre at x={x}; a corner pole would read 20"
        );
    }
}

/// A saved command without the flag still loads, defaulting to the forward transform.
#[test]
fn polar_deserialises_without_the_flag() {
    let filter: Filter = serde_json::from_str(r#"{"kind":"polar_coordinates"}"#)
        .expect("older saved commands must still load");
    match filter {
        Filter::PolarCoordinates { to_polar } => {
            assert!(to_polar, "the default must be the forward transform");
        }
        other => panic!("wrong variant: {other:?}"),
    }
}
