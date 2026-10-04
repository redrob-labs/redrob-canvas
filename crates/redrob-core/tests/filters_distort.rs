//! K.5, distorts and projections.

use redrob_core::{
    Command, DistanceMetric, Editor, Filter, LensSurroundings, Pixel, PropagateMode, ShiftAxis,
};

/// Build an editor holding one layer painted from `colors`, row-major.
#[path = "common/canvas.rs"]
mod canvas;

/// Build a test canvas. Memoised on its content by `common/canvas.rs` -- see that module for why.
fn image(width: u32, height: u32, colors: &[Pixel]) -> Editor {
    canvas::editor(width, height, colors)
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

/// The radial profile is `atan`, which is the ONLY thing separating this from a polar remap.
///
/// Both filters map angle to one axis and radius to the other, so without this they could quietly
/// become the same operation. Names both numbers, computed rather than fitted: with the reference
/// radius at 32 and zoom 1, output radius 24 gives `ψ = 2·atan(24/32)` = 1.287, which is input row
/// `1.287/π·64` ≈ **26** and so value **104**. A linear remap would read row `24/(32√2)·64` ≈ 34,
/// value **136** — that is what `PolarCoordinates` does with the same input, and it is asserted
/// alongside so the two are pinned apart rather than merely described as different.
#[test]
fn stereographic_radial_profile_is_atan_not_linear() {
    let size = 64usize;
    // Value depends only on the row, so the output at a radius reveals which row it sampled.
    let colors: Vec<Pixel> = (0..size * size)
        .map(|index| {
            let v = ((index / size) * 4) as u8;
            Pixel::rgba(v, v, v, 255)
        })
        .collect();

    let value_at_radius = |filter: Filter, radius: f64| {
        let mut editor = image(size as u32, size as u32, &colors);
        editor.execute(Command::ApplyFilter { filter }).unwrap();
        let out = pixels(&editor);
        let centre = size as f64 / 2.0;
        // Average the four compass points, so one pixel of sampling noise cannot decide it.
        let mut total = 0i32;
        for angle in [
            0.0,
            std::f64::consts::FRAC_PI_2,
            std::f64::consts::PI,
            3.0 * std::f64::consts::FRAC_PI_2,
        ] {
            let x = (centre + radius * angle.cos()).floor() as usize;
            let y = (centre + radius * angle.sin()).floor() as usize;
            total += i32::from(out[(y.min(size - 1) * size + x.min(size - 1)) * 4]);
        }
        total / 4
    };

    let stereographic = value_at_radius(
        Filter::StereographicProjection {
            zoom: 1.0,
            inverse: false,
        },
        24.0,
    );
    // The polar INVERSE, not the forward one: `to_polar: true` produces an (angle, radius)
    // rectangle in which a cartesian radius means nothing, so comparing against it would have
    // compared a real profile with a coincidence. The inverse IS radial -- cartesian radius maps
    // linearly to input row -- which is what makes it the right counterpart.
    let polar = value_at_radius(Filter::PolarCoordinates { to_polar: false }, 24.0);

    assert!(
        (stereographic - 104).abs() <= 8,
        "the atan profile must read near 104 at radius 24, got {stereographic}"
    );
    assert!(
        (polar - 136).abs() <= 8,
        "and the linear polar remap near 136, got {polar}"
    );
    assert!(
        polar - stereographic > 16,
        "the two profiles must be clearly apart: {polar} against {stereographic}"
    );
}

/// The nadir — the input's bottom row — lands at the centre of the little planet.
///
/// The defining look of the effect. Input row 0 is colatitude 0, which is radius 0, so the output
/// centre must sample the TOP of the input. Names the wrong behaviour: a projection that put the
/// zenith at the centre instead would read the bottom row there, and on this input those are 0 and
/// 252.
#[test]
fn stereographic_puts_the_pole_at_the_centre() {
    let size = 64usize;
    let colors: Vec<Pixel> = (0..size * size)
        .map(|index| {
            let v = ((index / size) * 4) as u8;
            Pixel::rgba(v, v, v, 255)
        })
        .collect();

    let mut editor = image(size as u32, size as u32, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::StereographicProjection {
                zoom: 1.0,
                inverse: false,
            },
        })
        .unwrap();
    let out = pixels(&editor);

    let index = (size / 2) * size + size / 2;
    assert!(
        out[index * 4] <= 12,
        "the centre must sample the input's first row, got {}; the opposite pole would read 252",
        out[index * 4]
    );
}

/// Constant latitude becomes a circle: four compass points at one radius agree.
#[test]
fn stereographic_maps_latitudes_to_circles() {
    let size = 64usize;
    let colors: Vec<Pixel> = (0..size * size)
        .map(|index| {
            let v = ((index / size) * 4) as u8;
            Pixel::rgba(v, v, v, 255)
        })
        .collect();

    let mut editor = image(size as u32, size as u32, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::StereographicProjection {
                zoom: 1.0,
                inverse: false,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    let centre = size as f64 / 2.0;

    for radius in [8.0f64, 16.0, 24.0] {
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

/// Constant longitude becomes a radial spoke.
///
/// The converse of the circles test, and together they pin the mapping's two axes. A single bright
/// column in the input must come out as a ray from the centre, so the bright pixels must spread
/// across many radii at nearly one angle rather than forming a ring.
#[test]
fn stereographic_maps_longitudes_to_spokes() {
    let size = 64usize;
    let colors: Vec<Pixel> = (0..size * size)
        .map(|index| {
            let v = if index % size == 0 { 250u8 } else { 10 };
            Pixel::rgba(v, v, v, 255)
        })
        .collect();

    let mut editor = image(size as u32, size as u32, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::StereographicProjection {
                zoom: 1.0,
                inverse: false,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    let centre = size as f64 / 2.0;

    let bright: Vec<(f64, f64)> = (0..size * size)
        .filter(|index| out[index * 4] > 128)
        .map(|index| {
            let dx = (index % size) as f64 + 0.5 - centre;
            let dy = (index / size) as f64 + 0.5 - centre;
            (dx.hypot(dy), dy.atan2(dx).rem_euclid(std::f64::consts::TAU))
        })
        .collect();

    assert!(
        bright.len() >= 8,
        "the bright column must survive the projection, found {} pixels",
        bright.len()
    );

    // Spread across radii, concentrated in angle -- a ring would be the other way round.
    let radii: Vec<f64> = bright.iter().map(|(r, _)| *r).collect();
    let radius_spread = radii.iter().cloned().fold(0.0f64, f64::max)
        - radii.iter().cloned().fold(f64::INFINITY, f64::min);
    let angles: Vec<f64> = bright.iter().map(|(_, a)| *a).collect();
    let angle_spread = angles.iter().cloned().fold(0.0f64, f64::max)
        - angles.iter().cloned().fold(f64::INFINITY, f64::min);

    assert!(
        radius_spread > 16.0,
        "a spoke must span many radii, spanned {radius_spread:.1}"
    );
    assert!(
        angle_spread < 0.5,
        "and sit at nearly one angle, spanned {angle_spread:.2} radians"
    );
}

/// `zoom` scales how much of the sphere lands in the frame.
///
/// At a given output radius, a larger zoom must show a latitude nearer the pole — so on a
/// row-graded input the value at a fixed radius must fall as zoom rises.
#[test]
fn stereographic_zoom_scales_the_sphere() {
    let size = 64usize;
    let colors: Vec<Pixel> = (0..size * size)
        .map(|index| {
            let v = ((index / size) * 4) as u8;
            Pixel::rgba(v, v, v, 255)
        })
        .collect();

    let at_radius = |zoom: f64| {
        let mut editor = image(size as u32, size as u32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::StereographicProjection {
                    zoom,
                    inverse: false,
                },
            })
            .unwrap();
        let out = pixels(&editor);
        let index = (size / 2) * size + (size / 2 + 16);
        i32::from(out[index * 4])
    };

    let tight = at_radius(0.5);
    let wide = at_radius(2.0);
    assert!(
        wide < tight,
        "a larger zoom must show a latitude nearer the pole at the same radius: {wide} against \
         {tight}"
    );
}

/// The two directions are different transforms.
#[test]
fn stereographic_inverse_differs_from_forward() {
    let colors: Vec<Pixel> = (0..48 * 48)
        .map(|index| {
            let x = (index % 48) as u8;
            let y = (index / 48) as u8;
            Pixel::rgba(x * 5, y * 5, 90, 255)
        })
        .collect();

    let under = |inverse: bool| {
        let mut editor = image(48, 48, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::StereographicProjection { zoom: 1.0, inverse },
            })
            .unwrap();
        pixels(&editor)
    };

    assert_ne!(under(false), under(true), "the two directions must differ");
}

/// A flat field survives either direction exactly.
#[test]
fn stereographic_on_a_flat_field_changes_nothing() {
    let colors = vec![Pixel::rgba(70, 130, 180, 255); 48 * 48];
    for inverse in [false, true] {
        let mut editor = image(48, 48, &colors);
        let before = pixels(&editor);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::StereographicProjection { zoom: 1.0, inverse },
            })
            .unwrap();
        assert_eq!(
            pixels(&editor),
            before,
            "a flat field must survive inverse={inverse} exactly"
        );
    }
}

/// Zoom outside our recorded range is refused.
#[test]
fn stereographic_refuses_an_out_of_range_zoom() {
    let colors = vec![Pixel::rgba(100, 100, 100, 255); 64];
    for zoom in [0.0f64, -1.0, 100.0, f64::NAN] {
        let mut editor = image(8, 8, &colors);
        assert!(
            editor
                .execute(Command::ApplyFilter {
                    filter: Filter::StereographicProjection {
                        zoom,
                        inverse: false,
                    },
                })
                .is_err(),
            "a zoom of {zoom} must be refused"
        );
    }
}

/// A saved command with no fields still loads, defaulting to the forward projection at zoom 1.
#[test]
fn stereographic_deserialises_with_defaults() {
    let filter: Filter = serde_json::from_str(r#"{"kind":"stereographic_projection"}"#)
        .expect("older saved commands must still load");
    match filter {
        Filter::StereographicProjection { zoom, inverse } => {
            assert_eq!(zoom, 1.0);
            assert!(!inverse);
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

/// An equirectangular test image whose value encodes its own longitude.
///
/// Column 0 is longitude −180°, the middle column is 0°, so reading a value back says which
/// longitude the view was pointing at. That is what makes the sign convention measurable.
fn longitude_coded(size: usize) -> Vec<Pixel> {
    (0..size * size)
        .map(|index| {
            let v = ((index % size) * 4) as u8;
            Pixel::rgba(v, v, v, 255)
        })
        .collect()
}

/// Looking straight ahead samples the panorama's centre.
///
/// The baseline the sign tests are measured against: with every angle at zero the centre ray is
/// `+z`, which is longitude 0 and latitude 0, so it must read the middle of the input.
#[test]
fn panorama_default_view_looks_at_the_centre() {
    let size = 64usize;
    let colors = longitude_coded(size);
    let mut editor = image(size as u32, size as u32, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::PanoramaProjection {
                pan: 0.0,
                tilt: 0.0,
                spin: 0.0,
                zoom: 100.0,
                inverse: false,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    let centre = (size / 2) * size + size / 2;
    // The middle column of a 64-wide image codes to 32*4 = 128.
    assert!(
        i32::from(out[centre * 4]).abs_diff(128) <= 8,
        "the centre must look at longitude 0, which codes 128, got {}",
        out[centre * 4]
    );
}

/// `pan` is MINUS yaw, which is the thing only the propgui could have told me.
///
/// Read from `gimppropgui-panorama-projection.c`, twice and in both directions: the widget sets
/// `"pan", -yaw` and reads back `-pan`. So a positive `pan` turns the view one specific way, and a
/// sign error is invisible to every other test here.
///
/// Both values named. The input codes longitude into its columns at 4 per column, so longitude 0 is
/// 128. A `pan` of +90° must move the view a quarter turn, landing the centre on a column 16 away
/// — value 64 or 192 depending on the sign. With the convention as read it is **64**; a flipped
/// sign gives **192**.
#[test]
fn panorama_pan_sign_follows_the_propgui() {
    let size = 64usize;
    let colors = longitude_coded(size);

    let centre_value = |pan: f64| {
        let mut editor = image(size as u32, size as u32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::PanoramaProjection {
                    pan,
                    tilt: 0.0,
                    spin: 0.0,
                    zoom: 100.0,
                    inverse: false,
                },
            })
            .unwrap();
        let out = pixels(&editor);
        i32::from(out[((size / 2) * size + size / 2) * 4])
    };

    let right = centre_value(90.0);
    let left = centre_value(-90.0);

    assert!(
        right.abs_diff(64) <= 8,
        "pan +90 must look a quarter turn one way, which codes 64, got {right}; the opposite \
         sign convention would give 192"
    );
    assert!(
        left.abs_diff(192) <= 8,
        "and pan -90 the other way, which codes 192, got {left}"
    );
    assert_ne!(right, left, "the two must not coincide");
}

/// `tilt` moves the view vertically, and a full quarter turn reaches the pole.
#[test]
fn panorama_tilt_moves_the_view_vertically() {
    let size = 64usize;
    // Latitude-coded this time: value encodes the row.
    let colors: Vec<Pixel> = (0..size * size)
        .map(|index| {
            let v = ((index / size) * 4) as u8;
            Pixel::rgba(v, v, v, 255)
        })
        .collect();

    let centre_value = |tilt: f64| {
        let mut editor = image(size as u32, size as u32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::PanoramaProjection {
                    pan: 0.0,
                    tilt,
                    spin: 0.0,
                    zoom: 100.0,
                    inverse: false,
                },
            })
            .unwrap();
        let out = pixels(&editor);
        i32::from(out[((size / 2) * size + size / 2) * 4])
    };

    let level = centre_value(0.0);
    let up = centre_value(45.0);
    let down = centre_value(-45.0);

    assert!(
        level.abs_diff(128) <= 8,
        "level must look at the equator, which codes 128, got {level}"
    );
    assert_ne!(up, down, "tilting up and down must differ");
    assert!(
        (up < level && down > level) || (up > level && down < level),
        "and must move to opposite sides of the equator: {up}, {level}, {down}"
    );
}

/// `spin` rolls the output about its own centre, leaving the centre pixel where it is.
///
/// That is what distinguishes a roll from a pan or a tilt: the view direction does not change, so
/// the centre must still read longitude 0 while the rest of the frame rotates.
#[test]
fn panorama_spin_rolls_the_frame_but_not_the_centre() {
    let size = 64usize;
    let colors = longitude_coded(size);

    let under = |spin: f64| {
        let mut editor = image(size as u32, size as u32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::PanoramaProjection {
                    pan: 0.0,
                    tilt: 0.0,
                    spin,
                    zoom: 100.0,
                    inverse: false,
                },
            })
            .unwrap();
        pixels(&editor)
    };

    let plain = under(0.0);
    let rolled = under(90.0);
    assert_ne!(plain, rolled, "a roll must change the frame");

    let centre = ((size / 2) * size + size / 2) * 4;
    assert!(
        i32::from(rolled[centre]).abs_diff(i32::from(plain[centre])) <= 8,
        "but the centre looks the same way, so it must not move: {} against {}",
        rolled[centre],
        plain[centre]
    );
}

/// `zoom` is a PERCENTAGE and narrows the field of view as it rises.
///
/// The unit comes from the propgui's `100.0 * zoom`, so 100 is the neutral value rather than 1. A
/// narrower view means neighbouring output pixels sample nearer-together longitudes, so the spread
/// of values across a row must shrink.
#[test]
fn panorama_zoom_is_a_percentage_that_narrows_the_view() {
    let size = 64usize;
    let colors = longitude_coded(size);

    let row_spread = |zoom: f64| {
        let mut editor = image(size as u32, size as u32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::PanoramaProjection {
                    pan: 0.0,
                    tilt: 0.0,
                    spin: 0.0,
                    zoom,
                    inverse: false,
                },
            })
            .unwrap();
        let out = pixels(&editor);
        let row: Vec<i32> = (0..size)
            .map(|x| i32::from(out[((size / 2) * size + x) * 4]))
            .collect();
        row.iter().max().copied().unwrap_or(0) - row.iter().min().copied().unwrap_or(0)
    };

    let wide = row_spread(50.0);
    let normal = row_spread(100.0);
    let narrow = row_spread(400.0);

    assert!(
        wide > narrow,
        "a larger zoom must narrow the view: spread {narrow} at 400% against {wide} at 50%"
    );
    assert!(
        normal < wide && normal > narrow,
        "and 100% must sit between them: {wide}, {normal}, {narrow}"
    );
}

/// Upstream's own declared range is enforced: 0.01 to 1000.
///
/// The only range in K.5 that is not ours — it is the propgui's `CLAMP (100.0 * zoom, 0.01,
/// 1000.0)`, read from source.
#[test]
fn panorama_enforces_upstreams_declared_zoom_range() {
    let colors = vec![Pixel::rgba(100, 100, 100, 255); 64];
    for zoom in [0.0f64, 0.009, 1000.1, 5_000.0, f64::NAN] {
        let mut editor = image(8, 8, &colors);
        assert!(
            editor
                .execute(Command::ApplyFilter {
                    filter: Filter::PanoramaProjection {
                        pan: 0.0,
                        tilt: 0.0,
                        spin: 0.0,
                        zoom,
                        inverse: false,
                    },
                })
                .is_err(),
            "a zoom of {zoom} is outside upstream's declared range and must be refused"
        );
    }

    // And the endpoints themselves are inside it.
    for zoom in [0.01f64, 1000.0] {
        let mut editor = image(8, 8, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::PanoramaProjection {
                    pan: 0.0,
                    tilt: 0.0,
                    spin: 0.0,
                    zoom,
                    inverse: false,
                },
            })
            .unwrap_or_else(|_| panic!("zoom {zoom} is upstream's own bound and must be accepted"));
    }
}

/// The inverse leaves nothing where the panorama faces away from the camera.
///
/// Half the sphere has no rectilinear image at all, so those pixels must be transparent rather than
/// quietly clamped to an edge sample — which would invent a view of something behind the lens.
#[test]
fn panorama_inverse_leaves_the_far_hemisphere_empty() {
    let size = 64usize;
    let colors = longitude_coded(size);
    let mut editor = image(size as u32, size as u32, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::PanoramaProjection {
                pan: 0.0,
                tilt: 0.0,
                spin: 0.0,
                zoom: 100.0,
                inverse: true,
            },
        })
        .unwrap();
    let out = pixels(&editor);

    // Column 0 is longitude -180, directly behind the camera.
    let behind = (size / 2) * size;
    assert_eq!(
        out[behind * 4 + 3],
        0,
        "what faces away from the camera has no image and must be left empty"
    );
    // And the centre, which faces the camera, must not be.
    let ahead = (size / 2) * size + size / 2;
    assert_eq!(
        out[ahead * 4 + 3],
        255,
        "what faces the camera must be drawn"
    );
}

/// A saved command with no fields loads at the neutral view.
#[test]
fn panorama_deserialises_with_defaults() {
    let filter: Filter = serde_json::from_str(r#"{"kind":"panorama_projection"}"#)
        .expect("older saved commands must still load");
    match filter {
        Filter::PanoramaProjection {
            pan,
            tilt,
            spin,
            zoom,
            inverse,
        } => {
            assert_eq!(pan, 0.0);
            assert_eq!(tilt, 0.0);
            assert_eq!(spin, 0.0);
            assert_eq!(
                zoom, 100.0,
                "the neutral zoom is 100 because upstream's unit is a percentage"
            );
            assert!(!inverse);
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

/// A row-major 3x3 identity, exactly as upstream's `add_transform` writes it.
const IDENTITY_3X3: [f64; 9] = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];

/// An identity transform leaves the image alone at any depth.
///
/// Exact, and the strongest check on the whole composition pipeline: every composed word is the
/// identity, so any error in the matrix product, the inverse, the sampling or the compositing order
/// shows up here as a changed pixel.
#[test]
fn recursive_identity_changes_nothing_at_any_depth() {
    let colors: Vec<Pixel> = (0..32 * 32)
        .map(|index| {
            let x = (index % 32) as u8;
            let y = (index / 32) as u8;
            Pixel::rgba(x * 8, y * 8, 90, 255)
        })
        .collect();

    for iterations in [1u32, 2, 5] {
        let mut editor = image(32, 32, &colors);
        let before = pixels(&editor);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::RecursiveTransform {
                    transforms: vec![IDENTITY_3X3],
                    iterations,
                },
            })
            .unwrap();
        assert_eq!(
            pixels(&editor),
            before,
            "the identity composed {iterations} times is still the identity"
        );
    }
}

/// A translation puts the copy exactly where the matrix says.
///
/// Checkable to the pixel, which is what makes it worth asserting over "something moved": a
/// translation by (8, 0) must reproduce the source column at x−8, and an inverted translation would
/// put it at x+8 instead.
#[test]
fn recursive_translation_lands_where_the_matrix_says() {
    let size = 32usize;
    // A single bright column, so its copy is unambiguous.
    let colors: Vec<Pixel> = (0..size * size)
        .map(|index| {
            let v = if index % size == 4 { 250u8 } else { 10 };
            Pixel::rgba(v, v, v, 255)
        })
        .collect();

    let mut editor = image(size as u32, size as u32, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::RecursiveTransform {
                // Translate by +8 in x.
                transforms: vec![[1.0, 0.0, 8.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]],
                iterations: 1,
            },
        })
        .unwrap();
    let out = pixels(&editor);

    let row = size / 2;
    assert!(
        out[(row * size + 4) * 4] > 128,
        "the original column must still be there"
    );
    assert!(
        out[(row * size + 12) * 4] > 128,
        "and the translated copy must be at x=12; an inverted translation would put it at x=-4"
    );
}

/// Deeper recursion produces more copies.
///
/// With a translation of 8 and a 32-wide image, each iteration adds one more copy until they run
/// off the edge — so the count of bright columns must rise with depth.
#[test]
fn recursive_depth_adds_copies() {
    let size = 32usize;
    let colors: Vec<Pixel> = (0..size * size)
        .map(|index| {
            let v = if index % size == 2 { 250u8 } else { 10 };
            Pixel::rgba(v, v, v, 255)
        })
        .collect();

    let bright_columns = |iterations: u32| {
        let mut editor = image(size as u32, size as u32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::RecursiveTransform {
                    transforms: vec![[1.0, 0.0, 8.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]],
                    iterations,
                },
            })
            .unwrap();
        let out = pixels(&editor);
        let row = size / 2;
        (0..size)
            .filter(|x| out[(row * size + x) * 4] > 128)
            .count()
    };

    let shallow = bright_columns(1);
    let deep = bright_columns(3);
    assert_eq!(shallow, 2, "one iteration gives the original plus one copy");
    assert_eq!(deep, 4, "three iterations give the original plus three");
}

/// The matrix is PROJECTIVE: the bottom row divides, it is not merely carried along.
///
/// This is what the nine numbers buy over an affine six, and the test took two attempts. My first
/// version compared the same matrix with `g = 0` against `g = 0.01` and asserted only that they
/// differ — which **passed** with the division removed, because the inverse's own `[8]` term is
/// computed from the whole matrix and so still varies with `g`. The test distinguished *some* pair
/// of behaviours rather than the one the defect changed, which is cycle 58's rule exactly.
///
/// This version names the position. A lone column at x = 36 under `g = 0.012` maps to
/// `36 / (0.012·36 + 1)` = **25.1**, so the output must hold exactly two bright columns, the
/// original at 36 and its copy at **25**. With a constant denominator the mapping is affine, the
/// copy lands on top of the original, and there is only **one** column — measured, both ways.
#[test]
fn recursive_transform_is_projective_not_affine() {
    let size = 48usize;
    let colors: Vec<Pixel> = (0..size * size)
        .map(|index| {
            let v = if index % size == 36 { 250u8 } else { 10 };
            Pixel::rgba(v, v, v, 255)
        })
        .collect();

    let mut editor = image(size as u32, size as u32, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::RecursiveTransform {
                // Pure perspective: identity but for the bottom row.
                transforms: vec![[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.012, 0.0, 1.0]],
                iterations: 1,
            },
        })
        .unwrap();
    let out = pixels(&editor);

    let bright: Vec<usize> = (0..size)
        .filter(|x| out[((size / 2) * size + x) * 4] > 128)
        .collect();

    assert_eq!(
        bright,
        vec![25, 36],
        "the perspective copy must land at 36/(0.012*36+1) = 25; an affine reading puts it back \
         on top of the original and leaves only [36]"
    );
}

/// A singular matrix is REFUSED, not skipped.
///
/// A degenerate transform collapses the plane to a line or a point, so there is no image of it to
/// draw. Skipping it would quietly draw fewer copies than asked for; falling back to the identity
/// would put the untransformed picture on screen as though it had been requested.
#[test]
fn recursive_singular_matrix_is_refused() {
    let colors = vec![Pixel::rgba(100, 100, 100, 255); 64];
    let singular = [
        // Two identical rows: determinant zero.
        [1.0, 2.0, 3.0, 1.0, 2.0, 3.0, 0.0, 0.0, 1.0],
        // An all-zero matrix.
        [0.0; 9],
        // A scale of zero.
        [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
    ];
    for matrix in singular {
        let mut editor = image(8, 8, &colors);
        assert!(
            editor
                .execute(Command::ApplyFilter {
                    filter: Filter::RecursiveTransform {
                        transforms: vec![matrix],
                        iterations: 1,
                    },
                })
                .is_err(),
            "a singular matrix must be refused: {matrix:?}"
        );
    }
}

/// An empty list is refused, because upstream's own list can never be empty.
///
/// `remove_transform` truncates at the last `;` but is guarded by `if (delim)`, so a one-entry list
/// survives the button. Enforcing that here makes the invariant explicit rather than accidental.
#[test]
fn recursive_empty_list_is_refused() {
    let colors = vec![Pixel::rgba(100, 100, 100, 255); 64];
    let mut editor = image(8, 8, &colors);
    assert!(
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::RecursiveTransform {
                    transforms: vec![],
                    iterations: 1,
                },
            })
            .is_err(),
        "upstream's list cannot be emptied, so neither can ours"
    );
}

/// Zero iterations, and a request whose composed word count would explode, are both refused.
#[test]
fn recursive_refuses_out_of_range_depth_and_explosive_requests() {
    let colors = vec![Pixel::rgba(100, 100, 100, 255); 64];
    let shrink = [0.5, 0.0, 1.0, 0.0, 0.5, 1.0, 0.0, 0.0, 1.0];

    let mut editor = image(8, 8, &colors);
    assert!(
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::RecursiveTransform {
                    transforms: vec![shrink],
                    iterations: 0,
                },
            })
            .is_err(),
        "a depth of zero is not a recursion"
    );

    let mut editor = image(8, 8, &colors);
    assert!(
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::RecursiveTransform {
                    transforms: vec![shrink],
                    iterations: 99,
                },
            })
            .is_err(),
        "a depth past our recorded cap must be refused"
    );

    // Four transforms at six iterations is 4^6 = 4096 words, past the copy cap.
    let mut editor = image(8, 8, &colors);
    assert!(
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::RecursiveTransform {
                    transforms: vec![shrink; 4],
                    iterations: 6,
                },
            })
            .is_err(),
        "an n^k explosion must be refused rather than run"
    );
}

/// Several transforms compose as an iterated function system, not as a flat list.
///
/// With two transforms at depth two there must be words of length two — AA, AB, BA, BB — so the
/// result must differ from applying each transform once. A flat implementation that only ever
/// applied the given matrices would make these identical.
#[test]
fn recursive_several_transforms_compose() {
    let size = 32usize;
    let colors: Vec<Pixel> = (0..size * size)
        .map(|index| {
            let v = if index % size == 2 { 250u8 } else { 10 };
            Pixel::rgba(v, v, v, 255)
        })
        .collect();

    let pair = vec![
        [1.0, 0.0, 6.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        [1.0, 0.0, 0.0, 0.0, 1.0, 6.0, 0.0, 0.0, 1.0],
    ];

    let under = |iterations: u32| {
        let mut editor = image(size as u32, size as u32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::RecursiveTransform {
                    transforms: pair.clone(),
                    iterations,
                },
            })
            .unwrap();
        pixels(&editor)
    };

    assert_ne!(
        under(1),
        under(2),
        "depth two must add the length-two words; a flat list would not"
    );
}

/// A saved command without `iterations` still loads, at the shallowest depth.
#[test]
fn recursive_deserialises_without_iterations() {
    let filter: Filter = serde_json::from_str(
        r#"{"kind":"recursive_transform","transforms":[[1,0,0,0,1,0,0,0,1]]}"#,
    )
    .expect("older saved commands must still load");
    match filter {
        Filter::RecursiveTransform {
            transforms,
            iterations,
        } => {
            assert_eq!(transforms.len(), 1);
            assert_eq!(iterations, 1, "the default depth must be the shallowest");
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

/// An image whose value encodes its own angle about the centre, so a fold is readable.
fn angle_coded(size: usize) -> Vec<Pixel> {
    let centre = size as f64 / 2.0;
    (0..size * size)
        .map(|index| {
            let x = (index % size) as f64 + 0.5 - centre;
            let y = (index / size) as f64 + 0.5 - centre;
            let angle = y.atan2(x).rem_euclid(std::f64::consts::TAU);
            let v = (angle / std::f64::consts::TAU * 250.0) as u8;
            Pixel::rgba(v, v, v, 255)
        })
        .collect()
}

/// The result has n-fold ROTATIONAL symmetry: turning it by 2π/n changes nothing.
///
/// The defining property of a kaleidoscope, and exactly checkable. Sampled at a fixed radius so
/// every comparison is between points the fold must map to the same place.
#[test]
fn mirrors_output_has_n_fold_rotational_symmetry() {
    let size = 64usize;
    let colors = angle_coded(size);
    let centre = size as f64 / 2.0;

    for mirrors in [2u32, 3, 6] {
        let mut editor = image(size as u32, size as u32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Mirrors { mirrors },
            })
            .unwrap();
        let out = pixels(&editor);

        let sample = |angle: f64, radius: f64| {
            let x = (centre + radius * angle.cos())
                .floor()
                .clamp(0.0, (size - 1) as f64) as usize;
            let y = (centre + radius * angle.sin())
                .floor()
                .clamp(0.0, (size - 1) as f64) as usize;
            i32::from(out[(y * size + x) * 4])
        };

        let period = std::f64::consts::TAU / f64::from(mirrors);
        for step in 0..i64::from(mirrors) {
            let here = sample(0.6, 20.0);
            let turned = sample(0.6 + period * step as f64, 20.0);
            assert!(
                (here - turned).abs() <= 12,
                "with {mirrors} mirrors a turn of {step} periods must change nothing: \
                 {here} against {turned}"
            );
        }
    }
}

/// And MIRROR symmetry: reflecting within a period changes nothing either.
///
/// This is what separates a kaleidoscope from a plain rotational repeat. Dropping the reflection
/// would leave the rotational test above passing while this one fails, which is why both are here.
#[test]
fn mirrors_output_is_reflected_within_each_period() {
    let size = 64usize;
    let colors = angle_coded(size);
    let centre = size as f64 / 2.0;
    let mirrors = 4u32;

    let mut editor = image(size as u32, size as u32, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Mirrors { mirrors },
        })
        .unwrap();
    let out = pixels(&editor);

    let sample = |angle: f64| {
        let x = (centre + 20.0 * angle.cos())
            .floor()
            .clamp(0.0, (size - 1) as f64) as usize;
        let y = (centre + 20.0 * angle.sin())
            .floor()
            .clamp(0.0, (size - 1) as f64) as usize;
        i32::from(out[(y * size + x) * 4])
    };

    let period = std::f64::consts::TAU / f64::from(mirrors);
    for fraction in [0.1f64, 0.2, 0.3] {
        let forward = sample(period * fraction);
        let reflected = sample(period * (1.0 - fraction));
        assert!(
            (forward - reflected).abs() <= 12,
            "angle {fraction} of a period and its reflection must agree: {forward} against \
             {reflected}"
        );
    }
}

/// Points already inside the first wedge are left exactly where they are.
///
/// The fold must be the identity on its own source wedge — otherwise it is not a fold but a
/// general warp. Exact, and the sharpest check on the `rem_euclid` and the reflection boundary.
#[test]
fn mirrors_leaves_the_source_wedge_untouched() {
    let size = 64usize;
    let colors = angle_coded(size);
    let centre = size as f64 / 2.0;
    let mirrors = 4u32;
    let period = std::f64::consts::TAU / f64::from(mirrors);

    let mut editor = image(size as u32, size as u32, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Mirrors { mirrors },
        })
        .unwrap();
    let out = pixels(&editor);

    let mut checked = 0usize;
    for y in 0..size {
        for x in 0..size {
            let dx = x as f64 + 0.5 - centre;
            let dy = y as f64 + 0.5 - centre;
            let radius = dx.hypot(dy);
            if radius < 4.0 || radius > centre - 2.0 {
                continue;
            }
            let angle = dy.atan2(dx).rem_euclid(std::f64::consts::TAU);
            // Strictly inside the first half-period, away from the fold boundary where rounding
            // can take a sample either side.
            if !(0.15 * period..0.35 * period).contains(&angle) {
                continue;
            }
            checked += 1;
            let index = y * size + x;
            assert_eq!(
                out[index * 4],
                colors[index].r,
                "({x}, {y}) is inside the source wedge and must be untouched"
            );
        }
    }
    assert!(
        checked > 50,
        "the source wedge must actually contain pixels to check, got {checked}"
    );
}

/// A radially symmetric image survives the fold, and the residual is SAMPLING not geometry.
///
/// Every angle carries the same value on concentric rings, so a fold about the centre cannot move
/// anything — up to the nearest-neighbour resampling at the ring boundaries.
///
/// My first version used rings of 0.5 pixel period and asserted 90% held; it measured 64%. The
/// premise was wrong rather than the filter: a pattern finer than the pixel grid is not radially
/// symmetric *at pixel resolution* at all, so no resampling could preserve it.
///
/// This version asserts the TREND, which is far stronger than any single threshold: as the rings
/// get coarser the hold rate must strictly improve, because the error is confined to boundary
/// pixels and widening the bands reduces their share. Measured 1994, 2060, 2078 of 2304 at ring
/// widths 2, 4 and 6. A wrong mapping would not improve with ring width at all.
#[test]
fn mirrors_leaves_a_radially_symmetric_image_alone() {
    let size = 48usize;
    let centre = size as f64 / 2.0;

    let held_at = |ring_width: f64| {
        let colors: Vec<Pixel> = (0..size * size)
            .map(|index| {
                let x = (index % size) as f64 + 0.5 - centre;
                let y = (index / size) as f64 + 0.5 - centre;
                let ring = (x.hypot(y) / ring_width) as u32 % 2;
                let v = if ring == 0 { 40u8 } else { 210 };
                Pixel::rgba(v, v, v, 255)
            })
            .collect();

        let mut editor = image(size as u32, size as u32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Mirrors { mirrors: 5 },
            })
            .unwrap();
        let out = pixels(&editor);
        (0..size * size)
            .filter(|index| out[index * 4] == colors[*index].r)
            .count()
    };

    let narrow = held_at(2.0);
    let medium = held_at(4.0);
    let wide = held_at(6.0);

    assert!(
        narrow < medium && medium < wide,
        "coarser rings must hold strictly better, since the error lives on boundaries: \
         {narrow}, {medium}, {wide}"
    );
    assert!(
        wide * 100 >= size * size * 88,
        "and at the coarse end almost everything must hold: {wide} of {}",
        size * size
    );
}

/// A flat field survives any mirror count exactly.
#[test]
fn mirrors_on_a_flat_field_changes_nothing() {
    let colors = vec![Pixel::rgba(70, 130, 180, 255); 48 * 48];
    for mirrors in [1u32, 3, 8] {
        let mut editor = image(48, 48, &colors);
        let before = pixels(&editor);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Mirrors { mirrors },
            })
            .unwrap();
        assert_eq!(
            pixels(&editor),
            before,
            "a flat field must survive {mirrors} mirrors exactly"
        );
    }
}

/// Different mirror counts give different images.
#[test]
fn mirrors_count_changes_the_result() {
    let colors = angle_coded(48);
    let under = |mirrors: u32| {
        let mut editor = image(48, 48, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Mirrors { mirrors },
            })
            .unwrap();
        pixels(&editor)
    };
    assert_ne!(under(2), under(5), "the count must matter");
}

/// Zero mirrors, and a count past our cap, are refused.
#[test]
fn mirrors_refuses_an_out_of_range_count() {
    let colors = vec![Pixel::rgba(100, 100, 100, 255); 64];
    for mirrors in [0u32, 500] {
        let mut editor = image(8, 8, &colors);
        assert!(
            editor
                .execute(Command::ApplyFilter {
                    filter: Filter::Mirrors { mirrors },
                })
                .is_err(),
            "a count of {mirrors} must be refused"
        );
    }
}

/// Zero displacement is the identity, and is accepted rather than refused.
#[test]
fn shift_zero_amount_is_the_identity() {
    let colors: Vec<Pixel> = (0..32 * 32)
        .map(|index| {
            let x = (index % 32) as u8;
            let y = (index / 32) as u8;
            Pixel::rgba(x * 8, y * 8, 90, 255)
        })
        .collect();
    let mut editor = image(32, 32, &colors);
    let before = pixels(&editor);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Shift {
                amount: 0,
                axis: ShiftAxis::Horizontal,
            },
        })
        .expect("a neutral setting is a meaningful request");
    assert_eq!(
        pixels(&editor),
        before,
        "zero displacement must move nothing"
    );
}

/// Rows move INDEPENDENTLY — which is what separates this from `gimp:offset`.
///
/// Upstream ships a separate uniform translation, so a shift that moved every row by the same
/// amount would be a duplicate of it. This is the test that excludes that reading: at least two
/// rows must end up displaced by different amounts.
///
/// Measured as each row's own displacement, recovered by finding where its single bright pixel
/// landed, so the comparison is between numbers rather than between whole images.
#[test]
fn shift_moves_rows_independently() {
    let size = 32usize;
    // One bright pixel per row, all in the same column, so each row's displacement is readable.
    let colors: Vec<Pixel> = (0..size * size)
        .map(|index| {
            let v = if index % size == 16 { 250u8 } else { 10 };
            Pixel::rgba(v, v, v, 255)
        })
        .collect();

    let mut editor = image(size as u32, size as u32, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Shift {
                amount: 6,
                axis: ShiftAxis::Horizontal,
            },
        })
        .unwrap();
    let out = pixels(&editor);

    let displacements: Vec<i64> = (0..size)
        .map(|y| {
            let found = (0..size)
                .find(|x| out[(y * size + x) * 4] > 128)
                .expect("every row keeps its bright pixel");
            found as i64 - 16
        })
        .collect();

    let distinct: std::collections::HashSet<i64> = displacements.iter().copied().collect();
    assert!(
        distinct.len() > 1,
        "rows must move by different amounts; a uniform shift would duplicate gimp:offset, and \
         every row here moved by {:?}",
        distinct
    );
    assert!(
        displacements.iter().all(|d| d.abs() <= 6),
        "and no row may move further than the amount asked for: {displacements:?}"
    );
}

/// Within a line the displacement is CONSTANT, so a row is a rotation of the original row.
///
/// This is what makes it a shift rather than per-pixel noise, and wrapping is what makes the
/// invariant exact: a row's pixels must be the same multiset, merely rotated. A per-pixel
/// displacement would scramble the row and fail this while still passing the test above.
#[test]
fn shift_preserves_each_row_as_a_rotation() {
    let size = 32usize;
    let colors: Vec<Pixel> = (0..size * size)
        .map(|index| {
            let x = (index % size) as u8;
            let y = (index / size) as u8;
            Pixel::rgba(x * 8, y * 4, 90, 255)
        })
        .collect();

    let mut editor = image(size as u32, size as u32, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Shift {
                amount: 9,
                axis: ShiftAxis::Horizontal,
            },
        })
        .unwrap();
    let out = pixels(&editor);

    for y in 0..size {
        let mut before: Vec<u8> = (0..size).map(|x| colors[y * size + x].r).collect();
        let mut after: Vec<u8> = (0..size).map(|x| out[(y * size + x) * 4]).collect();
        before.sort_unstable();
        after.sort_unstable();
        assert_eq!(
            before, after,
            "row {y} must hold the same pixels, merely rotated"
        );
    }
}

/// The vertical axis shifts COLUMNS, and the two axes are not the same transform.
///
/// Names the mistake: an implementation ignoring the axis would make these identical. The column
/// invariant is checked the same way as the row one, so a filter honouring only one axis fails.
#[test]
fn shift_vertical_axis_moves_columns() {
    let size = 32usize;
    let colors: Vec<Pixel> = (0..size * size)
        .map(|index| {
            let x = (index % size) as u8;
            let y = (index / size) as u8;
            Pixel::rgba(x * 8, y * 4, 90, 255)
        })
        .collect();

    let under = |axis: ShiftAxis| {
        let mut editor = image(size as u32, size as u32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Shift { amount: 7, axis },
            })
            .unwrap();
        pixels(&editor)
    };

    let horizontal = under(ShiftAxis::Horizontal);
    let vertical = under(ShiftAxis::Vertical);
    assert_ne!(
        horizontal, vertical,
        "the axis must matter; ignoring it would make these identical"
    );

    // Each COLUMN must now be a rotation of the original column.
    for x in 0..size {
        let mut before: Vec<u8> = (0..size).map(|y| colors[y * size + x].g).collect();
        let mut after: Vec<u8> = (0..size)
            .map(|y| vertical[(y * size + x) * 4 + 1])
            .collect();
        before.sort_unstable();
        after.sort_unstable();
        assert_eq!(
            before, after,
            "column {x} must hold the same pixels, merely rotated"
        );
    }
}

/// A larger amount reaches further.
#[test]
fn shift_amount_bounds_the_displacement() {
    let size = 64usize;
    let colors: Vec<Pixel> = (0..size * size)
        .map(|index| {
            let v = if index % size == 32 { 250u8 } else { 10 };
            Pixel::rgba(v, v, v, 255)
        })
        .collect();

    let widest = |amount: u32| {
        let mut editor = image(size as u32, size as u32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Shift {
                    amount,
                    axis: ShiftAxis::Horizontal,
                },
            })
            .unwrap();
        let out = pixels(&editor);
        (0..size)
            .map(|y| {
                let found = (0..size)
                    .find(|x| out[(y * size + x) * 4] > 128)
                    .expect("bright pixel survives");
                (found as i64 - 32).abs()
            })
            .max()
            .expect("non-empty")
    };

    let small = widest(3);
    let large = widest(20);
    assert!(
        small <= 3,
        "a bound of 3 must be respected, reached {small}"
    );
    assert!(
        large > small,
        "a larger bound must actually be used: {large} against {small}"
    );
    assert!(large <= 20, "but still respected, reached {large}");
}

/// A flat field is unchanged, and the same request twice gives the same image.
#[test]
fn shift_is_deterministic_and_leaves_a_flat_field_alone() {
    let flat = vec![Pixel::rgba(70, 130, 180, 255); 32 * 32];
    let mut editor = image(32, 32, &flat);
    let before = pixels(&editor);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Shift {
                amount: 8,
                axis: ShiftAxis::Horizontal,
            },
        })
        .unwrap();
    assert_eq!(
        pixels(&editor),
        before,
        "displacing a flat field cannot change it"
    );

    let textured: Vec<Pixel> = (0..32 * 32)
        .map(|index| Pixel::rgba((index * 7) as u8, (index * 3) as u8, 90, 255))
        .collect();
    let run = || {
        let mut editor = image(32, 32, &textured);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Shift {
                    amount: 8,
                    axis: ShiftAxis::Horizontal,
                },
            })
            .unwrap();
        pixels(&editor)
    };
    assert_eq!(run(), run(), "the same shift must be the same shift twice");
}

/// An amount past our recorded cap is refused.
#[test]
fn shift_refuses_an_out_of_range_amount() {
    let colors = vec![Pixel::rgba(100, 100, 100, 255); 64];
    let mut editor = image(8, 8, &colors);
    assert!(
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Shift {
                    amount: 5_000,
                    axis: ShiftAxis::Horizontal,
                },
            })
            .is_err(),
        "an amount past the cap must be refused"
    );
}

/// A saved command without the axis still loads, defaulting to horizontal.
#[test]
fn shift_deserialises_without_the_axis() {
    let filter: Filter = serde_json::from_str(r#"{"kind":"shift","amount":5}"#)
        .expect("older saved commands must still load");
    match filter {
        Filter::Shift { amount, axis } => {
            assert_eq!(amount, 5);
            assert_eq!(axis, ShiftAxis::Horizontal);
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

/// An index of 1.0 is air and refracts nothing, so it is EXACTLY the identity.
///
/// The exactness anchor for the whole refraction derivation: at `η = 1` the vector form collapses to
/// the incident ray, so the travel distance is exactly `z` and the sampled point is the pixel
/// itself. Any sign error in the normal, the Snell factor or the ray march shows up here.
#[test]
fn apply_lens_index_one_is_the_identity() {
    let colors: Vec<Pixel> = (0..32 * 32)
        .map(|index| {
            let x = (index % 32) as u8;
            let y = (index / 32) as u8;
            Pixel::rgba(x * 8, y * 8, 90, 255)
        })
        .collect();
    let mut editor = image(32, 32, &colors);
    let before = pixels(&editor);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ApplyLens {
                refraction_index: 1.0,
                surroundings: LensSurroundings::Keep,
                background: Pixel::rgba(0, 0, 0, 255),
            },
        })
        .unwrap();
    assert_eq!(
        pixels(&editor),
        before,
        "air refracts nothing, so every pixel must be untouched"
    );
}

/// The lens is ELLIPTICAL, which is the one thing the description says outright.
///
/// On a wide canvas the inscribed circle has radius `height/2`, while the ellipse reaches to
/// `width/2` along x. A pixel between those two — inside the ellipse, outside the circle — must be
/// refracted. A circular implementation would leave it untouched, and nothing else in this file
/// would notice.
///
/// Names the test point: on a 64×32 image the circle stops at radius 16 but the ellipse reaches
/// x = 32, so the pixel at (52, 16) is 20 from the centre, outside the circle and inside the
/// ellipse.
#[test]
fn apply_lens_region_is_elliptical_not_circular() {
    let width = 64usize;
    let height = 32usize;
    let colors: Vec<Pixel> = (0..width * height)
        .map(|index| {
            let x = (index % width) as u8;
            let y = (index / width) as u8;
            Pixel::rgba(x * 4, y * 8, 90, 255)
        })
        .collect();

    let mut editor = image(width as u32, height as u32, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ApplyLens {
                refraction_index: 1.6,
                surroundings: LensSurroundings::Keep,
                background: Pixel::rgba(0, 0, 0, 255),
            },
        })
        .unwrap();
    let out = pixels(&editor);

    // Confirm the point really is outside the inscribed circle and inside the ellipse.
    let (px, py) = (52usize, 16usize);
    let dx = px as f64 + 0.5 - 32.0;
    let dy = py as f64 + 0.5 - 16.0;
    assert!(
        dx.hypot(dy) > 16.0,
        "the test point must lie outside the inscribed circle"
    );
    assert!(
        (dx * dx) / (32.0 * 32.0) + (dy * dy) / (16.0 * 16.0) < 1.0,
        "and inside the ellipse"
    );

    let index = py * width + px;
    assert_ne!(
        out[index * 4],
        colors[index].r,
        "a pixel inside the ellipse but outside the circle must be refracted; a circular lens \
         would leave it exactly as it was"
    );
}

/// The exact centre is untouched: the normal points along the view axis, so nothing bends.
#[test]
fn apply_lens_leaves_the_centre_untouched() {
    let size = 32usize;
    let colors: Vec<Pixel> = (0..size * size)
        .map(|index| {
            let x = (index % size) as u8;
            let y = (index / size) as u8;
            Pixel::rgba(x * 8, y * 8, 90, 255)
        })
        .collect();
    let mut editor = image(size as u32, size as u32, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ApplyLens {
                refraction_index: 2.0,
                surroundings: LensSurroundings::Keep,
                background: Pixel::rgba(0, 0, 0, 255),
            },
        })
        .unwrap();
    let out = pixels(&editor);
    let index = (size / 2) * size + size / 2;
    assert_eq!(
        out[index * 4],
        colors[index].r,
        "on the lens axis the normal is the view direction and nothing refracts"
    );
}

/// A higher index bends further, so the lens magnifies more.
///
/// Measured as how far the sampled point travels: with a row-graded input the value at a fixed
/// offset from the centre reveals which row was read, and a stronger lens must pull from further
/// away.
#[test]
fn apply_lens_higher_index_bends_further() {
    let size = 64usize;
    let colors: Vec<Pixel> = (0..size * size)
        .map(|index| {
            let v = ((index / size) * 4) as u8;
            Pixel::rgba(v, v, v, 255)
        })
        .collect();

    let at_offset = |refraction_index: f64| {
        let mut editor = image(size as u32, size as u32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::ApplyLens {
                    refraction_index,
                    surroundings: LensSurroundings::Keep,
                    background: Pixel::rgba(0, 0, 0, 255),
                },
            })
            .unwrap();
        let out = pixels(&editor);
        // Half way down the lens, off the axis.
        i32::from(out[((size / 2 + 12) * size + size / 2) * 4])
    };

    let plain = at_offset(1.0);
    let mild = at_offset(1.3);
    let strong = at_offset(2.5);

    assert_ne!(mild, plain, "a real index must bend something");
    assert!(
        (strong - plain).abs() > (mild - plain).abs(),
        "a higher index must bend further: {plain}, {mild}, {strong}"
    );
}

/// The three surroundings options do three different things outside the lens.
///
/// Three options and four strings upstream — lines 444 and 445 are one radio whose label depends on
/// whether the image is indexed — so counting strings would have produced a fourth value this test
/// could never satisfy.
#[test]
fn apply_lens_surroundings_options_are_distinct() {
    let size = 32usize;
    let colors = vec![Pixel::rgba(200, 200, 200, 255); size * size];

    let corner = |surroundings: LensSurroundings| {
        let mut editor = image(size as u32, size as u32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::ApplyLens {
                    refraction_index: 1.5,
                    surroundings,
                    background: Pixel::rgba(0, 0, 255, 255),
                },
            })
            .unwrap();
        let out = pixels(&editor);
        // (0, 0) is outside any inscribed ellipse.
        [out[0], out[1], out[2], out[3]]
    };

    assert_eq!(
        corner(LensSurroundings::Keep),
        [200, 200, 200, 255],
        "Keep must leave the surroundings exactly as they were"
    );
    assert_eq!(
        corner(LensSurroundings::Background),
        [0, 0, 255, 255],
        "Background must paint the given colour"
    );
    assert_eq!(
        corner(LensSurroundings::Transparent),
        [0, 0, 0, 0],
        "Transparent must clear it"
    );
}

/// On a square canvas the lens is radially symmetric: four compass points agree.
#[test]
fn apply_lens_is_radially_symmetric_on_a_square_canvas() {
    let size = 64usize;
    let centre = size as f64 / 2.0;
    let colors: Vec<Pixel> = (0..size * size)
        .map(|index| {
            let x = (index % size) as f64 + 0.5 - centre;
            let y = (index / size) as f64 + 0.5 - centre;
            let v = ((x.hypot(y) / 6.0) as u32 % 2) * 170 + 40;
            Pixel::rgba(v as u8, v as u8, v as u8, 255)
        })
        .collect();

    let mut editor = image(size as u32, size as u32, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ApplyLens {
                refraction_index: 1.8,
                surroundings: LensSurroundings::Keep,
                background: Pixel::rgba(0, 0, 0, 255),
            },
        })
        .unwrap();
    let out = pixels(&editor);

    for radius in [8.0f64, 16.0, 22.0] {
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
            high - low <= 40,
            "at radius {radius} the compass points must agree on a square canvas: {values:?}"
        );
    }
}

/// An index below 1.0 is refused: that is a medium less dense than air, not a lens.
#[test]
fn apply_lens_refuses_an_index_below_air() {
    let colors = vec![Pixel::rgba(100, 100, 100, 255); 64];
    for refraction_index in [0.9f64, 0.0, -1.0, 500.0, f64::NAN] {
        let mut editor = image(8, 8, &colors);
        assert!(
            editor
                .execute(Command::ApplyFilter {
                    filter: Filter::ApplyLens {
                        refraction_index,
                        surroundings: LensSurroundings::Keep,
                        background: Pixel::rgba(0, 0, 0, 255),
                    },
                })
                .is_err(),
            "an index of {refraction_index} must be refused"
        );
    }
}

/// A saved command with only the index still loads.
#[test]
fn apply_lens_deserialises_with_defaults() {
    let filter: Filter = serde_json::from_str(r#"{"kind":"apply_lens","refraction_index":1.7}"#)
        .expect("older saved commands must still load");
    match filter {
        Filter::ApplyLens {
            refraction_index,
            surroundings,
            ..
        } => {
            assert_eq!(refraction_index, 1.7);
            assert_eq!(
                surroundings,
                LensSurroundings::Keep,
                "the default must leave the surroundings alone"
            );
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

/// Upstream's two presets, built from the action strings CHARACTER BY CHARACTER.
///
/// `(lower-threshold 0.000000) (upper-threshold 1.000000) (rate 1.000000) (top yes) (left yes)
/// (right yes) (bottom yes) (value yes) (alpha no)` — identical in both, so `mode` is the only
/// difference between Dilate and Erode.
fn preset(mode: PropagateMode) -> Filter {
    Filter::ValuePropagate {
        mode,
        lower_threshold: 0.0,
        upper_threshold: 1.0,
        rate: 1.0,
        left: true,
        right: true,
        top: true,
        bottom: true,
        value: true,
        alpha: false,
        foreground: Pixel::rgba(255, 0, 0, 255),
        background: Pixel::rgba(255, 255, 255, 255),
    }
}

/// A bright speck on dark ground: Dilate GROWS it, Erode REMOVES it.
///
/// This is the read fact, and it is the strongest evidence in the group: upstream's two action
/// strings are character-identical but for `(mode white)` against `(mode black)`, and their own
/// descriptions are "Grow lighter areas" against "Grow darker areas". So one filter with one
/// property changed must produce those two opposite results.
///
/// Names the wrong values first. A single bright pixel with four neighbours: under Dilate the
/// bright count must RISE from 1 to 5, and under Erode it must FALL to 0. An implementation that
/// ignored the mode would give the same count twice.
#[test]
fn value_propagate_dilate_and_erode_differ_only_in_mode() {
    let size = 16usize;
    let mut colors = vec![Pixel::rgba(20, 20, 20, 255); size * size];
    colors[8 * size + 8] = Pixel::rgba(240, 240, 240, 255);

    let bright_count = |mode: PropagateMode| {
        let mut editor = image(size as u32, size as u32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: preset(mode),
            })
            .unwrap();
        let out = pixels(&editor);
        (0..size * size).filter(|i| out[i * 4] > 128).count()
    };

    let dilated = bright_count(PropagateMode::White);
    let eroded = bright_count(PropagateMode::Black);

    assert_eq!(
        dilated, 5,
        "Dilate must grow the speck into its four enabled neighbours"
    );
    assert_eq!(
        eroded, 0,
        "Erode must grow the dark around it until it is gone"
    );
    assert_ne!(
        dilated, eroded,
        "the mode is the only difference between the two presets, so it must make a difference"
    );
}

/// A rate of 0 is the identity: nothing moves toward the propagated value.
#[test]
fn value_propagate_zero_rate_is_the_identity() {
    let size = 16usize;
    let mut colors = vec![Pixel::rgba(20, 20, 20, 255); size * size];
    colors[8 * size + 8] = Pixel::rgba(240, 240, 240, 255);

    let mut editor = image(size as u32, size as u32, &colors);
    let before = pixels(&editor);
    let Filter::ValuePropagate { mode, .. } = preset(PropagateMode::White) else {
        unreachable!()
    };
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ValuePropagate {
                mode,
                lower_threshold: 0.0,
                upper_threshold: 1.0,
                rate: 0.0,
                left: true,
                right: true,
                top: true,
                bottom: true,
                value: true,
                alpha: false,
                foreground: Pixel::rgba(255, 0, 0, 255),
                background: Pixel::rgba(255, 255, 255, 255),
            },
        })
        .unwrap();
    assert_eq!(pixels(&editor), before, "a rate of zero must move nothing");
}

/// A rate between 0 and 1 lands between the two: it is a blend, not a switch.
#[test]
fn value_propagate_rate_blends() {
    let size = 16usize;
    let mut colors = vec![Pixel::rgba(0, 0, 0, 255); size * size];
    colors[8 * size + 8] = Pixel::rgba(200, 200, 200, 255);

    let neighbour = |rate: f64| {
        let mut editor = image(size as u32, size as u32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::ValuePropagate {
                    mode: PropagateMode::White,
                    lower_threshold: 0.0,
                    upper_threshold: 1.0,
                    rate,
                    left: true,
                    right: true,
                    top: true,
                    bottom: true,
                    value: true,
                    alpha: false,
                    foreground: Pixel::rgba(255, 0, 0, 255),
                    background: Pixel::rgba(255, 255, 255, 255),
                },
            })
            .unwrap();
        i32::from(pixels(&editor)[(8 * size + 9) * 4])
    };

    let full = neighbour(1.0);
    let half = neighbour(0.5);
    assert_eq!(
        full, 200,
        "at full rate the neighbour takes the whole value"
    );
    assert_eq!(
        half, 100,
        "at half rate it must land half way, not snap to either end"
    );
}

/// Every direction off is the identity, and one direction alone propagates only that way.
///
/// Varies ONE thing: the same image under `right` alone and under `left` alone. A filter ignoring
/// the direction flags would give the same image twice, and one spreading everywhere regardless
/// would fail the all-off case.
#[test]
fn value_propagate_directions_are_honoured_separately() {
    let size = 16usize;
    let mut colors = vec![Pixel::rgba(20, 20, 20, 255); size * size];
    colors[8 * size + 8] = Pixel::rgba(240, 240, 240, 255);

    let under = |left: bool, right: bool, top: bool, bottom: bool| {
        let mut editor = image(size as u32, size as u32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::ValuePropagate {
                    mode: PropagateMode::White,
                    lower_threshold: 0.0,
                    upper_threshold: 1.0,
                    rate: 1.0,
                    left,
                    right,
                    top,
                    bottom,
                    value: true,
                    alpha: false,
                    foreground: Pixel::rgba(255, 0, 0, 255),
                    background: Pixel::rgba(255, 255, 255, 255),
                },
            })
            .unwrap();
        pixels(&editor)
    };

    let none = under(false, false, false, false);
    let flat: Vec<u8> = colors.iter().flat_map(|p| [p.r, p.g, p.b, p.a]).collect();
    assert_eq!(
        none, flat,
        "with every direction off there is no neighbour to propagate from"
    );

    let rightward = under(false, true, false, false);
    let leftward = under(true, false, false, false);
    assert_ne!(
        rightward, leftward,
        "the direction flags must each do something of their own"
    );

    // `right` enabled means a pixel reads its right-hand neighbour, so the speck spreads LEFT.
    assert!(
        rightward[(8 * size + 7) * 4] > 128,
        "with `right` on, the pixel left of the speck must read it"
    );
    assert!(
        rightward[(8 * size + 9) * 4] < 128,
        "and the pixel to its right must not"
    );
}

/// The threshold band gates the DONOR, so a lower threshold above a bright speck stops it spreading.
///
/// Names the wrong value: the speck sits at 240/255 ≈ 0.94, so a lower threshold of 0.98 must leave
/// the bright count at exactly 1 — the speck itself, unspread. A filter that gated the TARGET
/// rather than the donor would still spread it.
#[test]
fn value_propagate_threshold_band_gates_the_donor() {
    let size = 16usize;
    let mut colors = vec![Pixel::rgba(20, 20, 20, 255); size * size];
    colors[8 * size + 8] = Pixel::rgba(240, 240, 240, 255);

    let bright_count = |lower_threshold: f64, upper_threshold: f64| {
        let mut editor = image(size as u32, size as u32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::ValuePropagate {
                    mode: PropagateMode::White,
                    lower_threshold,
                    upper_threshold,
                    rate: 1.0,
                    left: true,
                    right: true,
                    top: true,
                    bottom: true,
                    value: true,
                    alpha: false,
                    foreground: Pixel::rgba(255, 0, 0, 255),
                    background: Pixel::rgba(255, 255, 255, 255),
                },
            })
            .unwrap();
        let out = pixels(&editor);
        (0..size * size).filter(|i| out[i * 4] > 128).count()
    };

    assert_eq!(
        bright_count(0.0, 1.0),
        5,
        "the full band lets the speck spread"
    );
    assert_eq!(
        bright_count(0.98, 1.0),
        1,
        "a lower threshold above the speck's own value stops it donating"
    );
    assert_eq!(
        bright_count(0.0, 0.5),
        1,
        "and an upper threshold below it does the same"
    );
}

/// `value no, alpha no` is the identity: both gates shut and nothing is propagated.
///
/// `(alpha no)` is read from source in both presets, so this is the one default taken from upstream
/// rather than chosen.
#[test]
fn value_propagate_both_channel_gates_off_is_the_identity() {
    let size = 16usize;
    let mut colors = vec![Pixel::rgba(20, 20, 20, 255); size * size];
    colors[8 * size + 8] = Pixel::rgba(240, 240, 240, 255);

    let mut editor = image(size as u32, size as u32, &colors);
    let before = pixels(&editor);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ValuePropagate {
                mode: PropagateMode::White,
                lower_threshold: 0.0,
                upper_threshold: 1.0,
                rate: 1.0,
                left: true,
                right: true,
                top: true,
                bottom: true,
                value: false,
                alpha: false,
                foreground: Pixel::rgba(255, 0, 0, 255),
                background: Pixel::rgba(255, 255, 255, 255),
            },
        })
        .unwrap();
    assert_eq!(
        pixels(&editor),
        before,
        "with both channel gates shut there is nothing to write"
    );
}

/// `MoreOpaque` grows opaque regions, and it needs `alpha` on to do anything.
///
/// The alpha modes compare the alpha channel rather than brightness, which is what separates them
/// from White and Black. With `alpha` off they are inert — that pairing is the discriminating case.
#[test]
fn value_propagate_more_opaque_works_on_alpha() {
    let size = 16usize;
    let mut colors = vec![Pixel::rgba(100, 100, 100, 0); size * size];
    colors[8 * size + 8] = Pixel::rgba(100, 100, 100, 255);

    let opaque_count = |alpha: bool| {
        let mut editor = image(size as u32, size as u32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::ValuePropagate {
                    mode: PropagateMode::MoreOpaque,
                    lower_threshold: 0.0,
                    upper_threshold: 1.0,
                    rate: 1.0,
                    left: true,
                    right: true,
                    top: true,
                    bottom: true,
                    value: false,
                    alpha,
                    foreground: Pixel::rgba(255, 0, 0, 255),
                    background: Pixel::rgba(255, 255, 255, 255),
                },
            })
            .unwrap();
        let out = pixels(&editor);
        (0..size * size).filter(|i| out[i * 4 + 3] > 128).count()
    };

    assert_eq!(
        opaque_count(true),
        5,
        "the opaque pixel must spread into its four neighbours"
    );
    assert_eq!(
        opaque_count(false),
        1,
        "with the alpha gate shut an alpha mode can write nothing"
    );
}

/// `OnlyForeground` propagates the foreground colour and nothing else.
#[test]
fn value_propagate_only_foreground_spreads_just_that_colour() {
    let size = 16usize;
    let mut colors = vec![Pixel::rgba(30, 30, 30, 255); size * size];
    colors[8 * size + 8] = Pixel::rgba(255, 0, 0, 255);
    // A bright pixel that is NOT the foreground colour, which must not spread.
    colors[4 * size + 4] = Pixel::rgba(250, 250, 250, 255);

    let mut editor = image(size as u32, size as u32, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ValuePropagate {
                mode: PropagateMode::OnlyForeground,
                lower_threshold: 0.0,
                upper_threshold: 1.0,
                rate: 1.0,
                left: true,
                right: true,
                top: true,
                bottom: true,
                value: true,
                alpha: false,
                foreground: Pixel::rgba(255, 0, 0, 255),
                background: Pixel::rgba(255, 255, 255, 255),
            },
        })
        .unwrap();
    let out = pixels(&editor);

    let is_red = |i: usize| out[i * 4] > 200 && out[i * 4 + 1] < 60 && out[i * 4 + 2] < 60;
    assert_eq!(
        (0..size * size).filter(|&i| is_red(i)).count(),
        5,
        "the foreground pixel must spread into its four neighbours"
    );
    // The white pixel's neighbours must still be the dark ground.
    assert!(
        out[(4 * size + 5) * 4] < 128,
        "a bright pixel that is not the foreground colour must not spread"
    );
}

/// `ForegroundToPeaks` writes the foreground colour at local extremes, not everywhere.
///
/// A peak is a pixel that beats every enabled neighbour. The speck is one; the flat ground is not,
/// since each of its pixels ties with its neighbours. So the count must be exactly 1 — a filter
/// treating ties as peaks would repaint the whole image.
#[test]
fn value_propagate_foreground_to_peaks_marks_only_extremes() {
    let size = 16usize;
    let mut colors = vec![Pixel::rgba(60, 60, 60, 255); size * size];
    colors[8 * size + 8] = Pixel::rgba(240, 240, 240, 255);

    let mut editor = image(size as u32, size as u32, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::ValuePropagate {
                mode: PropagateMode::ForegroundToPeaks,
                lower_threshold: 0.0,
                upper_threshold: 1.0,
                rate: 1.0,
                left: true,
                right: true,
                top: true,
                bottom: true,
                value: true,
                alpha: false,
                foreground: Pixel::rgba(0, 255, 0, 255),
                background: Pixel::rgba(255, 255, 255, 255),
            },
        })
        .unwrap();
    let out = pixels(&editor);
    let green = (0..size * size)
        .filter(|&i| out[i * 4 + 1] > 200 && out[i * 4] < 60)
        .count();
    assert_eq!(
        green, 1,
        "only the one extremum is a peak; a tie is not, or the whole flat ground would qualify"
    );
}

/// Out-of-range scalars and an inverted band are refused.
#[test]
fn value_propagate_refuses_bad_parameters() {
    let colors = vec![Pixel::rgba(100, 100, 100, 255); 64];
    let attempt = |lower_threshold: f64, upper_threshold: f64, rate: f64| {
        let mut editor = image(8, 8, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::ValuePropagate {
                    mode: PropagateMode::White,
                    lower_threshold,
                    upper_threshold,
                    rate,
                    left: true,
                    right: true,
                    top: true,
                    bottom: true,
                    value: true,
                    alpha: false,
                    foreground: Pixel::rgba(255, 0, 0, 255),
                    background: Pixel::rgba(255, 255, 255, 255),
                },
            })
            .is_err()
    };
    assert!(attempt(-0.1, 1.0, 1.0), "a threshold below zero is refused");
    assert!(attempt(0.0, 1.5, 1.0), "a threshold above one is refused");
    assert!(attempt(0.0, 1.0, 2.0), "a rate above one is refused");
    assert!(attempt(0.8, 0.2, 1.0), "an inverted band is refused");
    assert!(attempt(f64::NAN, 1.0, 1.0), "a non-finite bound is refused");
}

/// A saved command with only the mode still loads, and `alpha` defaults OFF as both presets set it.
#[test]
fn value_propagate_deserialises_with_the_presets_defaults() {
    let filter: Filter = serde_json::from_str(r#"{"kind":"value_propagate","mode":"black"}"#)
        .expect("older saved commands must still load");
    match filter {
        Filter::ValuePropagate {
            mode,
            lower_threshold,
            upper_threshold,
            rate,
            left,
            right,
            top,
            bottom,
            value,
            alpha,
            ..
        } => {
            assert_eq!(mode, PropagateMode::Black);
            assert_eq!(lower_threshold, 0.0);
            assert_eq!(upper_threshold, 1.0);
            assert_eq!(rate, 1.0);
            assert!(left && right && top && bottom, "all four directions on");
            assert!(value, "`(value yes)` in both presets");
            assert!(!alpha, "`(alpha no)` in both presets -- read, not chosen");
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

/// The three metrics are three different distances, measured where they must disagree.
///
/// A single background pixel in a white field, read at a DIAGONAL offset of (3, 4):
///
/// | metric | distance |
/// |---|---|
/// | Chebyshev | `max(3, 4)` = 4 |
/// | Euclidean | `hypot(3, 4)` = 5 |
/// | Manhattan | `3 + 4` = 7 |
///
/// Predicted before running and measured exactly 4, 7, 5.
///
/// **The diagonal is the whole point.** At an AXIS offset of (3, 0) all three metrics return 3 —
/// measured, not assumed — so an axis-aligned test would distinguish none of them while looking
/// like it tested the metric. That is the cycle-58 shape: a test that separates some pair of
/// behaviours is not a test that separates the pair the parameter changes.
#[test]
fn distance_transform_metrics_disagree_on_the_diagonal() {
    let size = 32usize;
    let mut colors = vec![Pixel::rgba(255, 255, 255, 255); size * size];
    colors[16 * size + 16] = Pixel::rgba(0, 0, 0, 255);

    let at = |metric: DistanceMetric, x: usize, y: usize| {
        let mut editor = image(size as u32, size as u32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::DistanceTransform {
                    metric,
                    threshold: 0.5,
                    normalize: false,
                },
            })
            .unwrap();
        i32::from(pixels(&editor)[(y * size + x) * 4])
    };

    // Diagonal offset (3, 4) from the hole.
    assert_eq!(
        at(DistanceMetric::Chebyshev, 19, 20),
        4,
        "Chebyshev is the larger axis step"
    );
    assert_eq!(
        at(DistanceMetric::Euclidean, 19, 20),
        5,
        "Euclidean is the straight line"
    );
    assert_eq!(
        at(DistanceMetric::Manhattan, 19, 20),
        7,
        "Manhattan is the sum of the axis steps"
    );

    // And the axis offset that proves why the diagonal was needed.
    for metric in [
        DistanceMetric::Chebyshev,
        DistanceMetric::Euclidean,
        DistanceMetric::Manhattan,
    ] {
        assert_eq!(
            at(metric, 19, 16),
            3,
            "on an axis every metric agrees, so an axis test would prove nothing about {metric:?}"
        );
    }
}

/// A pixel outside the set has distance zero, and the set's own pixels grow inward from its edge.
#[test]
fn distance_transform_is_zero_outside_and_grows_inward() {
    let size = 32usize;
    // A filled square from 8 to 23 inclusive, 16 wide.
    let colors: Vec<Pixel> = (0..size * size)
        .map(|index| {
            let x = index % size;
            let y = index / size;
            if (8..24).contains(&x) && (8..24).contains(&y) {
                Pixel::rgba(255, 255, 255, 255)
            } else {
                Pixel::rgba(0, 0, 0, 255)
            }
        })
        .collect();

    let mut editor = image(size as u32, size as u32, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::DistanceTransform {
                metric: DistanceMetric::Chebyshev,
                threshold: 0.5,
                normalize: false,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    let at = |x: usize, y: usize| i32::from(out[(y * size + x) * 4]);

    assert_eq!(at(0, 0), 0, "a pixel outside the set is zero");
    assert_eq!(at(4, 16), 0, "and so is one just outside the edge");
    assert_eq!(
        at(8, 16),
        1,
        "the first pixel inside is one step from outside"
    );
    assert_eq!(at(9, 16), 2, "and the next is two");
    // The deepest point of a 16-wide band under Chebyshev.
    assert_eq!(
        at(15, 15),
        8,
        "the centre of a 16-wide square is 8 steps from the nearest outside pixel"
    );
    // Monotone along a row into the square.
    let row: Vec<i32> = (8..16).map(|x| at(x, 15)).collect();
    assert!(
        row.windows(2).all(|w| w[1] >= w[0]),
        "distance must not fall as you move inward: {row:?}"
    );
}

/// The threshold decides what the set IS, so moving it past the image's value changes everything.
///
/// A mid-grey field: at a threshold below its value the whole image is the set and distances are
/// large; above it, nothing is in the set and every distance is zero. Names the wrong behaviour —
/// a filter ignoring the threshold would give the same picture twice.
#[test]
fn distance_transform_threshold_defines_the_set() {
    let size = 32usize;
    let colors = vec![Pixel::rgba(128, 128, 128, 255); size * size];

    let centre = |threshold: f64| {
        let mut editor = image(size as u32, size as u32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::DistanceTransform {
                    metric: DistanceMetric::Chebyshev,
                    threshold,
                    normalize: false,
                },
            })
            .unwrap();
        i32::from(pixels(&editor)[(16 * size + 16) * 4])
    };

    let included = centre(0.2);
    let excluded = centre(0.8);
    assert!(
        included > 0,
        "below the grey's own value the whole field is the set, so the centre is far from its edge"
    );
    assert_eq!(
        excluded, 0,
        "above it nothing is in the set, so every distance is zero"
    );
}

/// Normalisation stretches the largest distance present to 255.
///
/// Without it the raw distance is written, which for a small shape is a nearly black image. The
/// discriminating pair: the same shape, one flag changed, and the peak must move from 8 to 255.
#[test]
fn distance_transform_normalise_stretches_to_full_range() {
    let size = 32usize;
    let colors: Vec<Pixel> = (0..size * size)
        .map(|index| {
            let x = index % size;
            let y = index / size;
            if (8..24).contains(&x) && (8..24).contains(&y) {
                Pixel::rgba(255, 255, 255, 255)
            } else {
                Pixel::rgba(0, 0, 0, 255)
            }
        })
        .collect();

    let peak = |normalize: bool| {
        let mut editor = image(size as u32, size as u32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::DistanceTransform {
                    metric: DistanceMetric::Chebyshev,
                    threshold: 0.5,
                    normalize,
                },
            })
            .unwrap();
        let out = pixels(&editor);
        (0..size * size)
            .map(|i| i32::from(out[i * 4]))
            .max()
            .expect("non-empty")
    };

    assert_eq!(peak(false), 8, "raw distances are written as they are");
    assert_eq!(
        peak(true),
        255,
        "normalised, the deepest point becomes white"
    );
}

/// An empty set normalises to black rather than dividing by zero.
#[test]
fn distance_transform_handles_an_empty_set() {
    let colors = vec![Pixel::rgba(0, 0, 0, 255); 32 * 32];
    let mut editor = image(32, 32, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::DistanceTransform {
                metric: DistanceMetric::Euclidean,
                threshold: 0.5,
                normalize: true,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    assert!(
        (0..32 * 32).all(|i| out[i * 4] == 0),
        "with nothing in the set every distance is zero and normalising must not divide by it"
    );
}

/// The Euclidean path is EXACT, asserted where no grid metric can agree with it.
///
/// **The first version of this test did not test its own name.** It used a quarter-plane, where the
/// nearest non-set pixel is always axis-aligned and `min(x, y)` — so Manhattan returns the same
/// numbers, and routing Euclidean through the chamfer left every assertion passing. The cycle-58
/// shape, in my own test: it distinguished *some* pair of behaviours, not the pair the defect
/// changes.
///
/// So the geometry is a single hole and the offsets are Pythagorean triples, where the exact
/// Euclidean distance is a whole number and the grid metrics are provably elsewhere:
///
/// | offset | Euclidean | Manhattan | Chebyshev |
/// |---|---|---|---|
/// | (3, 4) | 5 | 7 | 4 |
/// | (6, 8) | 10 | 14 | 8 |
/// | (5, 12) | 13 | 17 | 12 |
/// | (20, 21) | 29 | 41 | 21 |
///
/// Exact integers under one metric and three different wrong answers under the others, so an
/// inexact transform cannot pass by coincidence.
#[test]
fn distance_transform_euclidean_is_exact() {
    let size = 64usize;
    let (hx, hy) = (30usize, 30usize);
    let mut colors = vec![Pixel::rgba(255, 255, 255, 255); size * size];
    colors[hy * size + hx] = Pixel::rgba(0, 0, 0, 255);

    let mut editor = image(size as u32, size as u32, &colors);
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::DistanceTransform {
                metric: DistanceMetric::Euclidean,
                threshold: 0.5,
                normalize: false,
            },
        })
        .unwrap();
    let out = pixels(&editor);
    let at = |x: usize, y: usize| i32::from(out[(y * size + x) * 4]);

    for (dx, dy, exact, manhattan, chebyshev) in [
        (3usize, 4usize, 5, 7, 4),
        (6, 8, 10, 14, 8),
        (5, 12, 13, 17, 12),
        (20, 21, 29, 41, 21),
    ] {
        let measured = at(hx + dx, hy - dy.min(hy));
        // Read from the quadrant that stays inside the canvas.
        let measured = if hy >= dy {
            measured
        } else {
            at(hx + dx, hy + dy)
        };
        assert_eq!(
            measured, exact,
            "at ({dx}, {dy}) the exact distance is {exact}; Manhattan would say {manhattan} and \
             Chebyshev {chebyshev}"
        );
    }
}

/// An out-of-range threshold is refused.
#[test]
fn distance_transform_refuses_a_bad_threshold() {
    let colors = vec![Pixel::rgba(100, 100, 100, 255); 64];
    for threshold in [-0.1f64, 1.5, f64::NAN] {
        let mut editor = image(8, 8, &colors);
        assert!(
            editor
                .execute(Command::ApplyFilter {
                    filter: Filter::DistanceTransform {
                        metric: DistanceMetric::Euclidean,
                        threshold,
                        normalize: false,
                    },
                })
                .is_err(),
            "a threshold of {threshold} must be refused"
        );
    }
}

/// A saved command with only the metric still loads.
#[test]
fn distance_transform_deserialises_with_defaults() {
    let filter: Filter =
        serde_json::from_str(r#"{"kind":"distance_transform","metric":"manhattan"}"#)
            .expect("older saved commands must still load");
    match filter {
        Filter::DistanceTransform {
            metric,
            threshold,
            normalize,
        } => {
            assert_eq!(metric, DistanceMetric::Manhattan);
            assert_eq!(threshold, 0.5);
            assert!(!normalize, "the raw distance is the unsurprising default");
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

/// A noisy field, so a segmentation really has something to reduce.
fn superpixel_noise(size: usize) -> Vec<Pixel> {
    (0..size * size)
        .map(|i| {
            let h = (i as u64)
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            Pixel::rgba((h >> 33) as u8, (h >> 41) as u8, (h >> 49) as u8, 255)
        })
        .collect()
}

/// A hard vertical edge at x = 20, off every grid line for a spacing of 32 (centres at 16 and 48).
fn superpixel_edge(size: usize) -> Vec<Pixel> {
    (0..size * size)
        .map(|i| {
            if i % size < 20 {
                Pixel::rgba(20, 20, 20, 255)
            } else {
                Pixel::rgba(235, 235, 235, 255)
            }
        })
        .collect()
}

fn distinct_shades(out: &[u8]) -> Vec<i32> {
    let mut shades: Vec<i32> = out.chunks(4).map(|c| i32::from(c[0])).collect();
    shades.sort_unstable();
    shades.dedup();
    shades
}

/// Both operations reduce the image to exactly one colour per grid cell.
///
/// That is what a superpixel segmentation IS, and the count is exact rather than approximate: a
/// 64-pixel image at spacing 16 has `ceil(64/16)² = 16` cells and at spacing 32 has 4. Measured
/// from 4096 distinct input colours down to 16 and 4.
#[test]
fn superpixels_reduce_the_image_to_one_colour_per_cell() {
    let size = 64usize;
    let colors = superpixel_noise(size);

    let input: Vec<u8> = colors.iter().flat_map(|p| [p.r, p.g, p.b, p.a]).collect();
    let input_distinct = input
        .chunks(4)
        .map(|c| (c[0], c[1], c[2]))
        .collect::<std::collections::HashSet<_>>()
        .len();
    assert_eq!(input_distinct, 4096, "the input really is all different");

    let count = |filter: Filter| {
        let mut editor = image(size as u32, size as u32, &colors);
        editor.execute(Command::ApplyFilter { filter }).unwrap();
        pixels(&editor)
            .chunks(4)
            .map(|c| (c[0], c[1], c[2]))
            .collect::<std::collections::HashSet<_>>()
            .len()
    };

    for (cluster_size, cells) in [(16u32, 16usize), (32, 4)] {
        assert_eq!(
            count(Filter::Slic {
                cluster_size,
                compactness: 10.0,
                iterations: 10,
            }),
            cells,
            "SLIC at spacing {cluster_size} must give exactly {cells} regions"
        );
        assert_eq!(
            count(Filter::Waterpixels {
                cluster_size,
                regularization: 1.0,
            }),
            cells,
            "waterpixels at spacing {cluster_size} must give exactly {cells} regions"
        );
    }
}

/// The two operations are DIFFERENT, which upstream shipping both is what requires.
///
/// Cycle 68's route in the opposite direction: there, a duplicate would have been a contradiction
/// and so excluded a reading; here the same argument forbids implementing one as an alias of the
/// other.
///
/// **The first version of this test passed while waterpixels literally called `slic_segment`.** It
/// compared `Waterpixels { regularization: 1.0 }` against `Slic { compactness: 10.0 }`, and the
/// alias forwarded the regularisation as the compactness — so the two differed because 1.0 is not
/// 10.0, not because the mechanisms differ. It asserted that *some* pair of calls differ, which a
/// parameter difference satisfies on its own.
///
/// So the comparison now holds the parameter FIXED and varies only the mechanism, at several
/// values. Under the alias these are identical by construction; under two real mechanisms they are
/// not. That is the pair the defect changes.
#[test]
fn superpixels_slic_and_waterpixels_are_not_the_same_filter() {
    let size = 64usize;
    let colors = superpixel_noise(size);

    // The canvas is built ONCE and cloned per filter, rather than rebuilt six times.
    //
    // `image()` fills a canvas by issuing a select-and-fill COMMAND PAIR per pixel, so a 64-pixel
    // canvas costs 4096 command pairs with history recorded for each. This test ran three weights
    // against two filters and so paid that six times: 47.56 seconds, the slowest single test in the
    // suite, measured at AUDIT-8. Cloning the built `Document` instead pays it once.
    let built = image(size as u32, size as u32, &colors).document().clone();
    let under = |filter: Filter| {
        let mut editor = Editor::new(built.clone()).expect("editor");
        editor.execute(Command::ApplyFilter { filter }).unwrap();
        pixels(&editor)
    };

    for weight in [0.0f64, 1.0, 10.0] {
        let water = under(Filter::Waterpixels {
            cluster_size: 32,
            regularization: weight,
        });
        let slic = under(Filter::Slic {
            cluster_size: 32,
            compactness: weight,
            iterations: 10,
        });
        assert_ne!(
            water, slic,
            "at weight {weight} a watershed on a gradient must not agree with k-means on colour \
             and space; upstream ships both names, so one cannot be an alias of the other"
        );
    }
}

/// The iteration count changes the result — the word "Iterative" is in the operation's own title.
#[test]
fn superpixels_slic_iterations_matter() {
    let size = 64usize;
    let colors = superpixel_noise(size);

    let under = |iterations: u32| {
        let mut editor = image(size as u32, size as u32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Slic {
                    cluster_size: 32,
                    compactness: 10.0,
                    iterations,
                },
            })
            .unwrap();
        pixels(&editor)
    };

    assert_ne!(
        under(1),
        under(10),
        "one pass is not ten; an implementation ignoring the count would make these equal"
    );
}

/// Compactness changes the segmentation — on an image where it CAN.
///
/// **Three geometries were measured before one showed the parameter at all.** On a hard two-tone
/// edge SLIC returns the same two shades `[20, 235]` at every compactness from 0.1 to 500, because
/// the colour term is larger by orders of magnitude and the spatial term cannot reach it. The same
/// was true of an edge placed where colour and space disagree. Compactness is only observable where
/// colour differences are SMALL, so the input here is a gentle diagonal ramp.
///
/// Measured: at compactness 1 the four cells keep four distinct means; at 500 two of them converge
/// and only three remain.
#[test]
fn superpixels_slic_compactness_matters_where_it_can() {
    let size = 64usize;
    // A gentle diagonal ramp: neighbouring colours differ by a couple of levels, so the spatial
    // term is comparable to the colour term.
    let ramp: Vec<Pixel> = (0..size * size)
        .map(|i| {
            let x = (i % size) as u32;
            let y = (i / size) as u32;
            let v = ((x + y) * 2).min(255) as u8;
            Pixel::rgba(v, v, v, 255)
        })
        .collect();

    let shades = |compactness: f64| {
        let mut editor = image(size as u32, size as u32, &ramp);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Slic {
                    cluster_size: 32,
                    compactness,
                    iterations: 10,
                },
            })
            .unwrap();
        distinct_shades(&pixels(&editor))
    };

    let loose = shades(1.0);
    let tight = shades(500.0);
    assert_eq!(loose.len(), 4, "loose clustering keeps all four cell means");
    assert_eq!(
        tight.len(),
        3,
        "tight clustering pulls two cells onto the same mean"
    );
    assert_ne!(loose, tight, "so the two settings are not the same picture");

    // And on a hard edge it genuinely does nothing, which is why the ramp was needed.
    let edge = superpixel_edge(size);
    let edge_shades = |compactness: f64| {
        let mut editor = image(size as u32, size as u32, &edge);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Slic {
                    cluster_size: 32,
                    compactness,
                    iterations: 10,
                },
            })
            .unwrap();
        distinct_shades(&pixels(&editor))
    };
    assert_eq!(
        edge_shades(0.1),
        vec![20, 235],
        "a hard edge segments perfectly however loose the clustering"
    );
    assert_eq!(
        edge_shades(500.0),
        vec![20, 235],
        "and however tight -- so a hard edge cannot test compactness"
    );
}

/// Waterpixels' regularization moves the cell boundary from the image's edge to the grid.
///
/// The clearest discriminator in this item, and exactly interpretable at both ends. The image has a
/// hard edge at x = 20 while the grid midline is at x = 32:
///
/// | regularization | boundary |
/// |---|---|
/// | 0 | 20 — the true edge |
/// | 2 | 21 |
/// | 10 | 30 |
/// | 50 | 32 — the grid midline |
///
/// At zero the flood follows the image alone and lands on the edge; raised far enough it lands on
/// the grid, where the seeds' own distances meet. Measured as the sweep above before being asserted.
#[test]
fn superpixels_waterpixels_regularization_moves_the_boundary_to_the_grid() {
    let size = 64usize;
    let colors = superpixel_edge(size);

    let boundary = |regularization: f64| {
        let mut editor = image(size as u32, size as u32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Waterpixels {
                    cluster_size: 32,
                    regularization,
                },
            })
            .unwrap();
        let out = pixels(&editor);
        let row: Vec<i32> = (0..size)
            .map(|x| i32::from(out[(32 * size + x) * 4]))
            .collect();
        (1..size)
            .find(|&x| (row[x] - row[x - 1]).abs() > 40)
            .expect("the row must change somewhere")
    };

    assert_eq!(
        boundary(0.0),
        20,
        "with no regularisation the cells follow the image's own edge"
    );
    assert_eq!(
        boundary(50.0),
        32,
        "driven hard, they fall back to the grid midline"
    );
    // Monotone in between, which is what makes it a weight rather than a switch.
    let sweep: Vec<usize> = [0.0f64, 2.0, 10.0, 50.0]
        .iter()
        .map(|&r| boundary(r))
        .collect();
    assert!(
        sweep.windows(2).all(|w| w[1] >= w[0]),
        "the boundary must move toward the grid as the weight rises: {sweep:?}"
    );
}

/// A flat field is unchanged by either: every region's mean is the one colour present.
#[test]
fn superpixels_leave_a_flat_field_alone() {
    let flat = vec![Pixel::rgba(90, 140, 190, 255); 64 * 64];
    for filter in [
        Filter::Slic {
            cluster_size: 32,
            compactness: 10.0,
            iterations: 10,
        },
        Filter::Waterpixels {
            cluster_size: 32,
            regularization: 1.0,
        },
    ] {
        let mut editor = image(64, 64, &flat);
        let before = pixels(&editor);
        editor.execute(Command::ApplyFilter { filter }).unwrap();
        assert_eq!(
            pixels(&editor),
            before,
            "a field of one colour has one mean, so segmenting it changes nothing"
        );
    }
}

/// Both are deterministic: the seeds are a grid and the flood order is an integer key.
#[test]
fn superpixels_are_deterministic() {
    let colors = superpixel_noise(48);
    for filter in [
        Filter::Slic {
            cluster_size: 16,
            compactness: 10.0,
            iterations: 4,
        },
        Filter::Waterpixels {
            cluster_size: 16,
            regularization: 1.0,
        },
    ] {
        let run = || {
            let mut editor = image(48, 48, &colors);
            editor
                .execute(Command::ApplyFilter {
                    filter: filter.clone(),
                })
                .unwrap();
            pixels(&editor)
        };
        assert_eq!(run(), run(), "the same segmentation twice");
    }
}

/// Out-of-range parameters are refused, including a zero spacing and a zero iteration count.
#[test]
fn superpixels_refuse_bad_parameters() {
    let colors = vec![Pixel::rgba(100, 100, 100, 255); 64];
    let refused = |filter: Filter| {
        let mut editor = image(8, 8, &colors);
        editor.execute(Command::ApplyFilter { filter }).is_err()
    };

    assert!(
        refused(Filter::Slic {
            cluster_size: 0,
            compactness: 10.0,
            iterations: 4,
        }),
        "a spacing of zero would leave no grid at all"
    );
    assert!(
        refused(Filter::Slic {
            cluster_size: 16,
            compactness: 10.0,
            iterations: 0,
        }),
        "zero iterations of an iterative algorithm is refused, unlike a zero RATE which is a \
         meaningful neutral setting"
    );
    assert!(
        refused(Filter::Slic {
            cluster_size: 16,
            compactness: f64::NAN,
            iterations: 4,
        }),
        "a non-finite compactness is refused"
    );
    assert!(
        refused(Filter::Waterpixels {
            cluster_size: 0,
            regularization: 1.0,
        }),
        "and the same spacing rule applies to waterpixels"
    );
    assert!(
        refused(Filter::Waterpixels {
            cluster_size: 16,
            regularization: -1.0,
        }),
        "a negative weight is refused"
    );
}

/// Saved commands with no parameters still load, on both.
#[test]
fn superpixels_deserialise_with_defaults() {
    let slic: Filter =
        serde_json::from_str(r#"{"kind":"slic"}"#).expect("older saved commands must load");
    match slic {
        Filter::Slic {
            cluster_size,
            compactness,
            iterations,
        } => {
            assert_eq!(cluster_size, 32);
            assert_eq!(compactness, 10.0);
            assert_eq!(iterations, 10);
        }
        other => panic!("wrong variant: {other:?}"),
    }

    let water: Filter =
        serde_json::from_str(r#"{"kind":"waterpixels"}"#).expect("older saved commands must load");
    match water {
        Filter::Waterpixels {
            cluster_size,
            regularization,
        } => {
            assert_eq!(cluster_size, 32);
            assert_eq!(regularization, 1.0);
        }
        other => panic!("wrong variant: {other:?}"),
    }
}
