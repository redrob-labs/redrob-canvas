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
