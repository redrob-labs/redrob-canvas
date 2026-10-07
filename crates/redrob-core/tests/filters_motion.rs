//! K.8, the motion blur family.
//!
//! Upstream ships `motion-blur-linear`, `motion-blur-circular` and `motion-blur-zoom` as three
//! separate operations with three separate propguis, so by the source-7 rule they are three
//! mechanisms rather than one filter with different numbers. The structural difference is exact:
//! linear moves samples along a fixed direction, circular along an arc -- so it PRESERVES each
//! sample's radius from the centre -- and zoom radially, so it preserves each sample's ANGLE.
//!
//! Our shipped `MotionBlur` is the linear one only. AUDIT-5 recorded that wrongly as "the three
//! motion blurs we fold into one variant"; cycle 65 corrected it, and this file covers the two the
//! correction left outstanding.

use redrob_core::{Command, Editor, Filter, Pixel, Rect, SelectionMode};

#[path = "common/canvas.rs"]
mod canvas;

/// Build a test canvas. Memoised on its content by `common/canvas.rs`.
fn image(width: u32, height: u32, colors: &[Pixel]) -> Editor {
    canvas::editor(width, height, colors)
}

fn pixels(editor: &Editor) -> Vec<u8> {
    editor.document().layers()[0].pixels().to_vec()
}

const SIZE: usize = 48;
const CENTRE: f64 = 24.0;

/// A black field with one white pixel, which is the only input that can show where a blur moves
/// light TO without the answer being confounded by neighbouring sources.
fn dot(x: usize, y: usize) -> Vec<Pixel> {
    let mut field = vec![
        Pixel {
            r: 0,
            g: 0,
            b: 0,
            a: 255
        };
        SIZE * SIZE
    ];
    field[y * SIZE + x] = Pixel {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    };
    field
}

fn under(field: &[Pixel], filter: Filter) -> Vec<u8> {
    let mut editor = image(SIZE as u32, SIZE as u32, field);
    editor
        .execute(Command::ApplyFilter { filter })
        .expect("filter");
    pixels(&editor)
}

fn circular(angle: f64) -> Filter {
    Filter::MotionBlurCircular {
        center_x: 0.5,
        center_y: 0.5,
        angle,
    }
}

fn zoom(factor: f64) -> Filter {
    Filter::MotionBlurZoom {
        center_x: 0.5,
        center_y: 0.5,
        factor,
    }
}

/// THE assertion about the pair, and the strongest form this work has produced: the two filters are
/// exact complements, so one test states both mechanisms at once.
///
/// Circular moves samples along an arc, so it preserves each sample's RADIUS and spends its budget
/// on bearing. Zoom moves them along a radial ray, so it preserves each sample's BEARING and spends
/// its budget on radius. Measured on the same dot 16 px right of centre:
///
/// | | radial span | angular span |
/// |---|---|---|
/// | zoom, factor 1 | 8.00 | 0.00 |
/// | circular, 60 degrees | 0.97 | 59.49 |
#[test]
fn zoom_and_circular_are_exact_complements() {
    let field = dot(40, 24);

    let radial = lit_polar(&under(&field, zoom(1.0)));
    let (zoom_r_low, zoom_r_high) = span(radial.iter().map(|&(r, _)| r));
    let (zoom_a_low, zoom_a_high) = span(radial.iter().map(|&(_, a)| a));

    let arc = lit_polar(&under(&field, circular(60.0)));
    let (arc_r_low, arc_r_high) = span(arc.iter().map(|&(r, _)| r));
    let (arc_a_low, arc_a_high) = span(arc.iter().map(|&(_, a)| a));

    assert!(
        (zoom_a_high - zoom_a_low).abs() < 0.01,
        "zoom keeps every sample on its own bearing: {zoom_a_low:.2}..{zoom_a_high:.2}"
    );
    assert!(
        zoom_r_high - zoom_r_low > 5.0,
        "and spends its budget on radius: {zoom_r_low:.2}..{zoom_r_high:.2}"
    );

    assert!(
        arc_r_high - arc_r_low < 1.0,
        "circular keeps every sample on its own circle: {arc_r_low:.2}..{arc_r_high:.2}"
    );
    assert!(
        arc_a_high - arc_a_low > 50.0,
        "and spends its budget on bearing: {arc_a_low:.1}..{arc_a_high:.1}"
    );
}

/// The bearing is preserved EXACTLY, not approximately: scaling a radius leaves the angle alone, so
/// every lit pixel shares the source's bearing to the floating-point bit.
///
/// Measured for factors 1, 0.5 and -0.5 alike: angles span 0.00 to 0.00.
#[test]
fn zoom_preserves_the_bearing_exactly() {
    let field = dot(40, 24);
    for factor in [1.0f64, 0.5, -0.5] {
        let lit = lit_polar(&under(&field, zoom(factor)));
        let (low, high) = span(lit.iter().map(|&(_, a)| a));
        assert!(
            low.abs() < 0.01 && high.abs() < 0.01,
            "factor {factor} must keep the source's bearing of 0: {low:.2}..{high:.2}"
        );
    }
}

/// The lit radii are arithmetic, and the arithmetic is worth stating because my prediction was
/// BACKWARDS.
///
/// I predicted factor 1 would light radii 16..32, reasoning that the blur reaches outward. It
/// measured 8..16. The reason is that the filter reads FROM the scaled position, so an output pixel
/// at radius `r` is lit when `r * (1 + factor * s) == 16` for some `s` in `0..1` -- giving
/// `r = 16 / (1 + factor * s)`, which for factor 1 is `16/2 .. 16/1`, exactly 8..16.
///
/// Written down beforehand and wrong, which is worth as much as a right one: the measurement
/// explains the error exactly, and the test now pins the real relation rather than my reading of it.
#[test]
fn zoom_lit_radii_follow_the_reciprocal_of_the_scale() {
    let field = dot(40, 24);
    let lit = lit_polar(&under(&field, zoom(1.0)));
    let (low, high) = span(lit.iter().map(|&(r, _)| r));

    assert!(
        (low - 8.0).abs() < 0.01,
        "16 / (1 + 1) is 8, not 32: got {low:.2}"
    );
    assert!(
        (high - 16.0).abs() < 0.01,
        "and 16 / (1 + 0) is the source itself: got {high:.2}"
    );
}

/// A pixel at the centre has distance 0, so its ray has no length, so it takes exactly one sample --
/// itself. That makes the centre pixel an exact fixed point at every factor.
///
/// # What this test deliberately does NOT claim
///
/// The first probe asked whether the whole IMAGE was unchanged with the dot at the centre, and for a
/// negative factor it is not: exactly three pixels change, (23,23), (24,23) and (23,24), each from 0
/// to 128. They read the centre because `.round()` is half-away-from-zero, so `24 + (-0.5)` gives
/// 23.5 which rounds to 24, while `24 + 0.5` gives 24.5 which rounds to 25 and does not. That is a
/// half-pixel directional bias in the family's shared sampling, not a property of zoom, and the
/// claim here is narrowed to what is actually exact.
#[test]
fn zoom_centre_pixel_is_an_exact_fixed_point() {
    let field = dot(24, 24);
    for factor in [1.0f64, 0.5, -0.1, -0.5] {
        let out = under(&field, zoom(factor));
        assert_eq!(
            out[(24 * SIZE + 24) * 4],
            255,
            "the centre takes one sample -- itself -- at factor {factor}"
        );
    }
}

/// Factor 0 is a scale of exactly 1, so every sample is the pixel itself.
#[test]
fn zoom_zero_factor_is_the_identity() {
    let field = dot(40, 24);
    let before: Vec<u8> = field.iter().flat_map(|p| [p.r, p.g, p.b, p.a]).collect();
    assert_eq!(
        under(&field, zoom(0.0)),
        before,
        "a scale of 1 changes nothing"
    );
}

/// The range `-0.5..1.0` is READ verbatim from `CLAMP (radius / 100.0, -0.5, 1.0)`, and this test
/// pins the ASYMMETRY: both endpoints are accepted and both neighbours are refused, so a symmetric
/// guess of `±1` or `0..1` fails it.
///
/// The asymmetry is not arbitrary -- `1 + 1.0` is twice the radius and `1 + -0.5` is half it, so the
/// range is symmetric in the SCALE.
#[test]
fn zoom_factor_range_is_asymmetric_exactly_as_read() {
    let field = dot(40, 24);

    for good in [-0.5f64, 1.0] {
        let mut editor = image(SIZE as u32, SIZE as u32, &field);
        assert!(
            editor
                .execute(Command::ApplyFilter { filter: zoom(good) })
                .is_ok(),
            "{good} is an endpoint of the read range and must be accepted"
        );
    }

    for bad in [-0.51f64, 1.01, f64::NAN] {
        let mut editor = image(SIZE as u32, SIZE as u32, &field);
        assert!(
            editor
                .execute(Command::ApplyFilter { filter: zoom(bad) })
                .is_err(),
            "{bad} is outside the read range and must be refused"
        );
    }
}

/// The centre is NORMALISED, inverting `x1 / area->width`. At 0.25 on a 48-square canvas the fixed
/// point is pixel 12.
#[test]
fn zoom_centre_is_normalised() {
    let field = dot(12, 12);
    let out = under(
        &field,
        Filter::MotionBlurZoom {
            center_x: 0.25,
            center_y: 0.25,
            factor: 1.0,
        },
    );
    assert_eq!(
        out[(12 * SIZE + 12) * 4],
        255,
        "0.25 of 48 is pixel 12, so a dot there is the fixed point"
    );
}

/// Three properties, matching the propgui's three.
#[test]
fn zoom_deserialises_with_three_fields() {
    let filter: Filter =
        serde_json::from_str(r#"{"kind":"motion_blur_zoom"}"#).expect("deserialise");
    match filter {
        Filter::MotionBlurZoom {
            center_x,
            center_y,
            factor,
        } => {
            assert!(
                (center_x - 0.5).abs() < f64::EPSILON && (center_y - 0.5).abs() < f64::EPSILON,
                "a normalised centre defaults to the middle"
            );
            assert!((factor - 0.1).abs() < f64::EPSILON, "chosen default factor");
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

/// Where the light ended up, as polar coordinates about the canvas centre.
fn lit_polar(out: &[u8]) -> Vec<(f64, f64)> {
    (0..SIZE * SIZE)
        .filter(|i| out[i * 4] > 0)
        .map(|i| {
            let dx = (i % SIZE) as f64 - CENTRE;
            let dy = (i / SIZE) as f64 - CENTRE;
            ((dx * dx + dy * dy).sqrt(), dy.atan2(dx).to_degrees())
        })
        .collect()
}

fn span(values: impl Iterator<Item = f64>) -> (f64, f64) {
    values.fold((f64::MAX, -f64::MAX), |(lo, hi), v| (lo.min(v), hi.max(v)))
}

/// THE assertion about the difference, and the one that makes this a separate mechanism rather than
/// the linear blur with other numbers.
///
/// Circular blur moves every sample along an arc about the centre, so the light stays on the circle
/// it started on. Linear blur moves it along a fixed direction, which crosses circles.
///
/// Measured on a dot 16 px right of the centre: the circular output's radii span 15.52 to 16.49 --
/// under one pixel -- while the linear output's span 8.00 to 23.00, fifteen times wider.
#[test]
fn circular_preserves_the_radius_where_linear_does_not() {
    let field = dot(40, 24);

    let arc = lit_polar(&under(&field, circular(60.0)));
    let (arc_low, arc_high) = span(arc.iter().map(|&(r, _)| r));
    assert!(
        arc_high - arc_low < 1.0,
        "an arc keeps the light on its own circle: {arc_low:.2}..{arc_high:.2}"
    );

    let line = lit_polar(&under(
        &field,
        Filter::MotionBlur {
            angle_degrees: 0.0,
            distance: 16,
        },
    ));
    let (line_low, line_high) = span(line.iter().map(|&(r, _)| r));
    assert!(
        line_high - line_low > 10.0,
        "a straight line crosses circles: {line_low:.2}..{line_high:.2}"
    );
}

/// The arc is CENTRED on each pixel's own position and spans the whole requested angle, which is
/// why the sign of `angle` cannot be observed: a symmetric span is unchanged by flipping it.
///
/// Measured for a requested 60 degrees: the lit set spans -29.7 to +29.7 about the source's own
/// bearing, which is 59.4 degrees of a requested 60.
#[test]
fn circular_arc_is_centred_on_the_pixel_and_spans_the_angle() {
    let field = dot(40, 24);
    let lit = lit_polar(&under(&field, circular(60.0)));
    let (low, high) = span(lit.iter().map(|&(_, a)| a));

    assert!(
        (low + high).abs() < 2.0,
        "the arc is centred on the source's own bearing of 0 degrees: {low:.1}..{high:.1}"
    );
    assert!(
        (high - low - 60.0).abs() < 2.0,
        "and spans the requested 60 degrees: {:.1}",
        high - low
    );
}

/// The centre is the rotation's fixed point, so it cannot move at ANY angle -- including a full
/// turn, where every sample lands back on the pixel itself.
#[test]
fn circular_centre_is_a_fixed_point_at_every_angle() {
    let field = dot(24, 24);
    let before: Vec<u8> = field.iter().flat_map(|p| [p.r, p.g, p.b, p.a]).collect();

    for angle in [0.0f64, 30.0, 180.0, 360.0] {
        let out = under(&field, circular(angle));
        assert_eq!(
            out, before,
            "a dot at the centre of rotation must not move at angle {angle}"
        );
    }
}

/// Angle 0 is no arc at all, so the single sample is the pixel itself.
#[test]
fn circular_zero_angle_is_the_identity() {
    let field = dot(40, 24);
    let before: Vec<u8> = field.iter().flat_map(|p| [p.r, p.g, p.b, p.a]).collect();
    assert_eq!(under(&field, circular(0.0)), before, "no arc means no blur");
}

/// A wider arc reaches further. Measured: 3, 11 and 28 lit pixels at 10, 30 and 90 degrees.
#[test]
fn circular_wider_angle_spreads_further() {
    let field = dot(40, 24);
    let lit = |angle: f64| {
        let out = under(&field, circular(angle));
        (0..SIZE * SIZE).filter(|i| out[i * 4] > 0).count()
    };

    let narrow = lit(10.0);
    let middle = lit(30.0);
    let wide = lit(90.0);
    assert!(
        narrow < middle && middle < wide,
        "a wider arc lights more: {narrow} < {middle} < {wide}"
    );
}

/// The centre is NORMALISED, inverting the propgui's `x1 / area->width`. At 0.25 on a 48-square
/// canvas the fixed point is pixel 12, not pixel 0.
#[test]
fn circular_centre_is_normalised() {
    let field = dot(12, 12);
    let before: Vec<u8> = field.iter().flat_map(|p| [p.r, p.g, p.b, p.a]).collect();

    let out = under(
        &field,
        Filter::MotionBlurCircular {
            center_x: 0.25,
            center_y: 0.25,
            angle: 90.0,
        },
    );
    assert_eq!(
        out, before,
        "0.25 of 48 is pixel 12, so a dot there is the fixed point"
    );
}

/// `0..360` is READ from the propgui's `if (angle < 0) angle += 360`.
#[test]
fn circular_refuses_an_angle_outside_the_declared_interval() {
    let field = dot(40, 24);
    for bad in [
        circular(-1.0),
        circular(360.5),
        circular(f64::NAN),
        Filter::MotionBlurCircular {
            center_x: f64::INFINITY,
            center_y: 0.5,
            angle: 10.0,
        },
    ] {
        let mut editor = image(SIZE as u32, SIZE as u32, &field);
        assert!(
            editor
                .execute(Command::ApplyFilter { filter: bad })
                .is_err(),
            "an angle outside the propgui's own interval must be refused"
        );
    }
}

/// Three properties, matching the propgui's three.
#[test]
fn circular_deserialises_with_three_fields() {
    let filter: Filter =
        serde_json::from_str(r#"{"kind":"motion_blur_circular"}"#).expect("deserialise");
    match filter {
        Filter::MotionBlurCircular {
            center_x,
            center_y,
            angle,
        } => {
            assert!(
                (center_x - 0.5).abs() < f64::EPSILON && (center_y - 0.5).abs() < f64::EPSILON,
                "a normalised centre defaults to the middle"
            );
            assert!((angle - 5.0).abs() < f64::EPSILON, "chosen default angle");
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

/// Keeps `Rect` and `SelectionMode` live for the shared canvas helper's signature.
#[test]
fn canvas_helper_is_wired() {
    let field = dot(1, 1);
    let mut editor = image(SIZE as u32, SIZE as u32, &field);
    editor
        .execute(Command::SelectRectangle {
            rect: Rect::new(0, 0, 1, 1),
            mode: SelectionMode::Replace,
        })
        .expect("select");
    editor.execute(Command::ClearSelection).expect("clear");
    assert_eq!(pixels(&editor).len(), SIZE * SIZE * 4);
}
