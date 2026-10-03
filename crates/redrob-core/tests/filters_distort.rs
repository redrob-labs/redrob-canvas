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

/// Curvature 0 is EXACTLY the identity, not nearly so.
///
/// That exactness is why the implementation interpolates from the identity toward an extreme rather
/// than evaluating a formula that happens to reduce to it: a formula could leave rounding behind at
/// the neutral setting, and a filter whose neutral setting changes the image is a bug a user cannot
/// work around.
#[test]
fn spherize_zero_curvature_is_the_identity() {
    let colors: Vec<Pixel> = (0..48 * 48)
        .map(|index| {
            let x = (index % 48) as u8;
            let y = (index / 48) as u8;
            Pixel::rgba(x * 5, y * 5, 90, 255)
        })
        .collect();
    let mut editor = image(48, 48, &colors);
    let before = pixels(&editor);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Spherize { curvature: 0.0 },
        })
        .unwrap();
    assert_eq!(
        pixels(&editor),
        before,
        "the neutral setting must leave every pixel alone"
    );
}

/// Outside the inscribed ball nothing moves — it is a ball on the picture, not a warp of the frame.
///
/// Exact, and the property that distinguishes a sphere from a whole-image distortion. The corners
/// of a square image lie outside a ball of radius half the side, so they must come back untouched
/// at any curvature.
#[test]
fn spherize_leaves_the_image_outside_the_ball_untouched() {
    let size = 48usize;
    let colors: Vec<Pixel> = (0..size * size)
        .map(|index| {
            let x = (index % size) as u8;
            let y = (index / size) as u8;
            Pixel::rgba(x * 5, y * 5, 90, 255)
        })
        .collect();

    for curvature in [1.0f64, -1.0] {
        let mut editor = image(size as u32, size as u32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Spherize { curvature },
            })
            .unwrap();
        let out = pixels(&editor);

        let centre = size as f64 / 2.0;
        let sphere = centre;
        let mut checked = 0usize;
        for y in 0..size {
            for x in 0..size {
                let dx = x as f64 + 0.5 - centre;
                let dy = y as f64 + 0.5 - centre;
                if dx.hypot(dy) < sphere {
                    continue;
                }
                checked += 1;
                let index = y * size + x;
                assert_eq!(
                    out[index * 4],
                    colors[index].r,
                    "({x}, {y}) is outside the ball and must be untouched at curvature {curvature}"
                );
            }
        }
        assert!(
            checked > 100,
            "the corners must actually be outside the ball, only {checked} checked"
        );
    }
}

/// Positive curvature MAGNIFIES the centre and negative one shrinks it.
///
/// The sign test, and it names both wrong behaviours. A small bright disc at the centre must come
/// out larger under a bulge and smaller under a pinch; swapping `asin` for `sin` would reverse both
/// at once, and a filter ignoring the sign would make the two counts equal.
#[test]
fn spherize_sign_decides_bulge_against_pinch() {
    let size = 64usize;
    let centre = size as f64 / 2.0;
    let colors: Vec<Pixel> = (0..size * size)
        .map(|index| {
            let x = (index % size) as f64 + 0.5 - centre;
            let y = (index / size) as f64 + 0.5 - centre;
            // A disc of radius 8 at the centre.
            let v = if x.hypot(y) < 8.0 { 240u8 } else { 20 };
            Pixel::rgba(v, v, v, 255)
        })
        .collect();

    let bright_area = |curvature: f64| {
        let mut editor = image(size as u32, size as u32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Spherize { curvature },
            })
            .unwrap();
        let out = pixels(&editor);
        (0..size * size)
            .filter(|index| out[index * 4] > 128)
            .count()
    };

    let plain = bright_area(0.0);
    let bulged = bright_area(0.8);
    let pinched = bright_area(-0.8);

    assert!(
        bulged > plain,
        "a bulge must magnify the centre: {bulged} against {plain}"
    );
    assert!(
        pinched < plain,
        "and a pinch must shrink it: {pinched} against {plain}"
    );
}

/// The mapping depends only on radius, so points at equal radius are treated alike.
///
/// A radial transform cannot favour an axis. Four compass points at the same radius must land on
/// the same value — which a mapping applied per axis, the most plausible wrong implementation given
/// that upstream may well have a mode selector, could not satisfy.
#[test]
fn spherize_is_radial_not_per_axis() {
    let size = 64usize;
    let centre = size as f64 / 2.0;
    // Value depends only on radius, so the output must too.
    let colors: Vec<Pixel> = (0..size * size)
        .map(|index| {
            let x = (index % size) as f64 + 0.5 - centre;
            let y = (index / size) as f64 + 0.5 - centre;
            let v = ((x.hypot(y) * 5.0) as u32 % 256) as u8;
            Pixel::rgba(v, v, v, 255)
        })
        .collect();

    let mut editor = image(size as u32, size as u32, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Spherize { curvature: 0.7 },
        })
        .unwrap();
    let out = pixels(&editor);

    for radius in [6.0f64, 12.0, 20.0] {
        let sample = |angle: f64| {
            let x = (centre + radius * angle.cos()).floor() as usize;
            let y = (centre + radius * angle.sin()).floor() as usize;
            i32::from(out[(y.min(size - 1) * size + x.min(size - 1)) * 4])
        };
        let values = [
            sample(0.0),
            sample(std::f64::consts::FRAC_PI_2),
            sample(std::f64::consts::PI),
            sample(3.0 * std::f64::consts::FRAC_PI_2),
        ];
        let low = *values.iter().min().expect("non-empty");
        let high = *values.iter().max().expect("non-empty");
        assert!(
            high - low <= 12,
            "at radius {radius} the four compass points must agree: {values:?}"
        );
    }
}

/// The exact centre never moves, because it has no direction to move along.
#[test]
fn spherize_leaves_the_centre_pixel_alone() {
    let size = 48usize;
    let colors: Vec<Pixel> = (0..size * size)
        .map(|index| {
            let x = (index % size) as u8;
            let y = (index / size) as u8;
            Pixel::rgba(x * 5, y * 5, 90, 255)
        })
        .collect();
    let mut editor = image(size as u32, size as u32, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Spherize { curvature: 1.0 },
        })
        .unwrap();
    let out = pixels(&editor);

    // The pixel whose centre is nearest the image centre.
    let index = (size / 2) * size + size / 2;
    assert_eq!(
        out[index * 4],
        colors[index].r,
        "the pole of the ball has no direction, so it cannot be displaced"
    );
}

/// A flat field survives any curvature exactly.
#[test]
fn spherize_on_a_flat_field_changes_nothing() {
    let colors = vec![Pixel::rgba(70, 130, 180, 255); 48 * 48];
    for curvature in [1.0f64, 0.5, -0.5, -1.0] {
        let mut editor = image(48, 48, &colors);
        let before = pixels(&editor);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Spherize { curvature },
            })
            .unwrap();
        assert_eq!(
            pixels(&editor),
            before,
            "resampling a flat field at curvature {curvature} must return it exactly"
        );
    }
}

/// Curvature outside −1..=1 is refused, since nothing outside it is a sphere.
#[test]
fn spherize_refuses_a_curvature_outside_the_range() {
    let colors = vec![Pixel::rgba(100, 100, 100, 255); 64];
    for curvature in [1.5f64, -1.5, f64::NAN, f64::INFINITY] {
        let mut editor = image(8, 8, &colors);
        assert!(
            editor
                .execute(Command::ApplyFilter {
                    filter: Filter::Spherize { curvature }
                })
                .is_err(),
            "a curvature of {curvature} must be refused"
        );
    }
}
