//! K.7, light and shadow.

use redrob_core::{Command, Editor, Filter, FocusShape, Pixel};

#[path = "common/canvas.rs"]
mod canvas;

/// Build a test canvas. Memoised on its content by `common/canvas.rs`.
fn image(width: u32, height: u32, colors: &[Pixel]) -> Editor {
    canvas::editor(width, height, colors)
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

fn drop_shadow(offset_x: i32, offset_y: i32, radius: u32, opacity: f64) -> Filter {
    Filter::DropShadow {
        offset_x,
        offset_y,
        radius,
        color: Pixel {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        },
        opacity,
    }
}

/// A transparent field with one opaque square, which is the only shape a drop shadow can read.
///
/// # Why the square is BLACK and not white
///
/// It was white, and the alpha-versus-luminance injection then passed every test. The reason was in
/// the input, not the filter: a white opaque square on transparent black has alpha 255 and red 255
/// inside, and 0 and 0 outside, so the two readings are the SAME ARRAY and no assertion could tell
/// them apart. The same shape as cycles 80 and 83, where every input was axis-aligned.
///
/// An opaque BLACK square separates them: alpha is 255 inside while every colour channel is 0, so a
/// filter reading colour casts nothing at all.
fn square(size: usize, at: usize, side: usize) -> Vec<Pixel> {
    let clear = Pixel {
        r: 0,
        g: 0,
        b: 0,
        a: 0,
    };
    let solid = Pixel {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };
    let mut field = vec![clear; size * size];
    for y in at..at + side {
        for x in at..at + side {
            field[y * size + x] = solid;
        }
    }
    field
}

fn alpha_at(pixels: &[u8], size: usize, x: usize, y: usize) -> u8 {
    pixels[(y * size + x) * 4 + 3]
}

fn covered(pixels: &[u8]) -> usize {
    pixels.chunks_exact(4).filter(|p| p[3] > 0).count()
}

/// `filters-actions.c` gates drop shadow on `writable && alpha`, and this is why: the shadow's shape
/// IS the alpha channel, and the layer composites over its own shadow. So a layer with no
/// transparency hides its shadow completely.
///
/// Predicted before running, and exact: unchanged. The wrong behaviour -- drawing the shadow OVER
/// the layer, which is the single likeliest way to get this filter wrong -- would darken the whole
/// field instead, so this one assertion pins the compositing ORDER.
#[test]
fn drop_shadow_on_a_fully_opaque_layer_is_a_no_op() {
    let size = 32;
    let flat = vec![
        Pixel {
            r: 200,
            g: 60,
            b: 60,
            a: 255
        };
        size * size
    ];
    let before = flatten(&flat);
    let after = apply(size, &flat, drop_shadow(4, 4, 15, 60.0));
    assert_eq!(
        before, after,
        "an opaque layer hides the shadow it casts, so the filter must change nothing"
    );
}

/// The shadow is the alpha shape, moved. With no blur and full opacity it stays a hard square of
/// exactly the same area, and at offset 4 from a 4-wide square it does not overlap its caster.
///
/// Predicted before running: 32 covered pixels, 16 for the square and 16 for the shadow. Measured
/// 32. Had the shape come from LUMINANCE rather than alpha it would be 16, because the transparent
/// region is black and would cast nothing.
#[test]
fn drop_shadow_casts_the_alpha_shape_not_the_luminance() {
    let size = 32;
    let field = square(size, 8, 4);
    let out = apply(size, &field, drop_shadow(4, 4, 0, 100.0));

    assert_eq!(
        covered(&out),
        32,
        "16 pixels of square plus 16 of shadow, not overlapping at offset 4"
    );
    assert_eq!(
        alpha_at(&out, size, 13, 13),
        255,
        "the shadow sits at the offset square"
    );
    assert_eq!(
        alpha_at(&out, size, 9, 9),
        255,
        "the caster is still fully opaque"
    );
    assert_eq!(
        alpha_at(&out, size, 25, 25),
        0,
        "nothing is cast beyond the offset shape"
    );
}

/// A positive offset moves the shadow right and down -- `drop-shadow.scm` translates the shadow
/// layer by `+shadow-transl-x`, so the sign is read, not chosen.
#[test]
fn drop_shadow_offset_sign_moves_it_right_and_down() {
    let size = 32;
    let field = square(size, 8, 4);
    let out = apply(size, &field, drop_shadow(8, 0, 0, 100.0));

    assert_eq!(
        alpha_at(&out, size, 17, 9),
        255,
        "a positive x offset casts to the RIGHT"
    );
    assert_eq!(
        alpha_at(&out, size, 1, 9),
        0,
        "and so casts nothing to the left"
    );
}

/// `drop-shadow.scm`'s blur range starts at **0** and it gates the blur with `(>= shadow-blur 1.0)`,
/// so 0 is a legal hard-edged shadow rather than an error -- which is why this arm does not call
/// `validate_radius`. A larger radius spreads the shadow over more pixels.
///
/// Measured: 32 covered at radius 0 against 256 at radius 6.
#[test]
fn drop_shadow_radius_zero_is_legal_and_larger_radii_spread() {
    let size = 32;
    let field = square(size, 8, 4);

    let hard = covered(&apply(size, &field, drop_shadow(4, 4, 0, 100.0)));
    let soft = covered(&apply(size, &field, drop_shadow(4, 4, 6, 100.0)));

    assert_eq!(
        hard, 32,
        "radius 0 must be accepted, and cast a hard shadow"
    );
    assert!(
        soft > hard,
        "a blurred shadow must cover more than a hard one: {soft} vs {hard}"
    );
}

/// Opacity is the shadow layer's, so zero means no shadow at all.
#[test]
fn drop_shadow_zero_opacity_changes_nothing() {
    let size = 32;
    let field = square(size, 8, 4);
    let before = flatten(&field);
    let after = apply(size, &field, drop_shadow(4, 4, 15, 0.0));
    assert_eq!(before, after, "an invisible shadow must leave the layer be");
}

/// One assertion about the DIFFERENCE, which is the shape cycle 82 settled on.
///
/// Drop shadow darkens from UNDER the layer and bloom adds light over its face, so on a fully opaque
/// field they must part company: the shadow is hidden entirely while bloom still brightens.
///
/// # The field is grey 200, not white, and that is a measured correction
///
/// This test first used white and FAILED, because bloom is a no-op on saturated white: it ADDS
/// spill and clamps, so a channel already at 255 cannot move. "Bloom must show" was a claim written
/// before it was measured -- the same wrong-comment shape as cycles 49, 55, 66, 77, 83 and 84 -- and
/// this time the assertion caught it before anything rested on it. Grey 200 against a 0.5 threshold
/// is above the threshold and has headroom to brighten.
#[test]
fn drop_shadow_is_not_a_glow() {
    let size = 32;
    let grey = vec![
        Pixel {
            r: 200,
            g: 200,
            b: 200,
            a: 255
        };
        size * size
    ];
    let before = flatten(&grey);

    let shadowed = apply(size, &grey, drop_shadow(4, 4, 8, 100.0));
    let bloomed = apply(size, &grey, bloom(0.5, 8, 1.0));

    assert_eq!(
        before, shadowed,
        "the shadow is behind an opaque layer, so it cannot show"
    );
    assert_ne!(
        before, bloomed,
        "bloom works on the face of the layer, so it must show"
    );
}

#[allow(clippy::too_many_arguments)]
fn vignette(
    shape: FocusShape,
    radius: f64,
    proportion: f64,
    squeeze: f64,
    rotation: f64,
    softness: f64,
    gamma: f64,
) -> Filter {
    Filter::Vignette {
        shape,
        x: 0.5,
        y: 0.5,
        radius,
        proportion,
        squeeze,
        rotation,
        softness,
        gamma,
        // Black, which is upstream's own default and reduces the blend to what it was before the
        // parameter existed. A test that wanted a tint passes its own.
        color: Pixel::rgba(0, 0, 0, 255),
    }
}

fn apply_to(width: usize, height: usize, level: u8, filter: Filter) -> Vec<u8> {
    let field = vec![
        Pixel {
            r: level,
            g: level,
            b: level,
            a: 255
        };
        width * height
    ];
    let mut editor = image(width as u32, height as u32, &field);
    editor
        .execute(Command::ApplyFilter { filter })
        .expect("filter");
    pixels(&editor)
}

/// The sharpest single test of the propgui reading, and it pins TWO relations at once.
///
/// Rotation is applied to the sample before the shape metric, so a `Horizontal` region turned 90
/// degrees must reproduce a `Vertical` one -- but ONLY when the two extents are equal, because
/// rotating swaps the axes while the extents do not swap. `config_notify` says the extents are equal
/// exactly when `1 + (height/width - 1) * proportion` is 1, which is a square canvas OR
/// `proportion = 0`.
///
/// So this is a three-way assertion, and all three were predicted before running: equal on a square
/// canvas, NOT equal on 64x32, and equal again on 64x32 once `proportion` is 0. A filter that
/// ignored `proportion` would pass the first and third and fail the second; one that ignored
/// rotation would fail all three.
#[test]
fn vignette_rotation_and_proportion_together_decide_the_extents() {
    let square_rotated = apply_to(
        48,
        48,
        200,
        vignette(FocusShape::Horizontal, 1.0, 1.0, 0.0, 90.0, 0.5, 1.0),
    );
    let square_vertical = apply_to(
        48,
        48,
        200,
        vignette(FocusShape::Vertical, 1.0, 1.0, 0.0, 0.0, 0.5, 1.0),
    );
    assert_eq!(
        square_rotated, square_vertical,
        "on a square canvas the extents match, so turning Horizontal by 90 gives Vertical"
    );

    let wide_rotated = apply_to(
        64,
        32,
        200,
        vignette(FocusShape::Horizontal, 1.0, 1.0, 0.0, 90.0, 0.5, 1.0),
    );
    let wide_vertical = apply_to(
        64,
        32,
        200,
        vignette(FocusShape::Vertical, 1.0, 1.0, 0.0, 0.0, 0.5, 1.0),
    );
    assert_ne!(
        wide_rotated, wide_vertical,
        "at proportion 1 on a 64x32 canvas the extents differ, so they must NOT match"
    );

    let flat_rotated = apply_to(
        64,
        32,
        200,
        vignette(FocusShape::Horizontal, 1.0, 0.0, 0.0, 90.0, 0.5, 1.0),
    );
    let flat_vertical = apply_to(
        64,
        32,
        200,
        vignette(FocusShape::Vertical, 1.0, 0.0, 0.0, 0.0, 0.5, 1.0),
    );
    assert_eq!(
        flat_rotated, flat_vertical,
        "proportion 0 makes the region circular again, so they match even on 64x32"
    );
}

/// `gamma = log(0.5) / log(midpoint)` inverts to `midpoint = 0.5^(1/gamma)`, so the darkening curve
/// is exactly `t^gamma`. That is arithmetic, not a shape, and it can be checked to the byte.
///
/// On a 48-square canvas with `Square`, `proportion 0`, `radius 0.5` the extent is 12 px, so x = 36
/// sits at exactly `t = 0.5`. The kept fraction there is `1 - 0.5^gamma`, giving 58.6, 100, 150 and
/// 187.5 on a field of 200. Measured: 59, 100, 150, 188.
#[test]
fn vignette_gamma_is_the_curve_the_propgui_inverts() {
    for (gamma, expected) in [(0.5f64, 59u8), (1.0, 100), (2.0, 150), (4.0, 188)] {
        let out = apply_to(
            48,
            48,
            200,
            vignette(FocusShape::Square, 1.0, 0.0, 0.0, 0.0, 1.0, gamma),
        );
        let measured = out[(24 * 48 + 36) * 4];
        assert_eq!(
            measured, expected,
            "at t=0.5 and gamma={gamma} the kept fraction is 1 - 0.5^gamma"
        );
    }
}

/// `softness` is `1 - inner_limit`, so zero softness leaves no transition at all: the region is
/// untouched out to the limit and fully dark one pixel later.
///
/// # Why radius 0.5 and not 1
///
/// At radius 1 the hard edge falls exactly on the canvas boundary, so nothing darkens and the test
/// would pass against a filter that ignored `softness` entirely. The first probe did exactly that
/// and reported 200 everywhere. Radius 0.5 puts the edge at x = 36, inside the canvas.
#[test]
fn vignette_zero_softness_is_a_hard_edge() {
    let hard = apply_to(
        48,
        48,
        200,
        vignette(FocusShape::Square, 0.5, 0.0, 0.0, 0.0, 0.0, 1.0),
    );
    let at = |x: usize| hard[(24 * 48 + x) * 4];

    assert_eq!(at(36), 200, "the limit itself is still untouched");
    assert_eq!(at(37), 0, "and one pixel further is fully dark");

    let soft = apply_to(
        48,
        48,
        200,
        vignette(FocusShape::Square, 0.5, 0.0, 0.0, 0.0, 0.5, 1.0),
    );
    let partial = soft[(24 * 48 + 35) * 4];
    assert!(
        partial > 0 && partial < 200,
        "with softness there is a transition: got {partial}"
    );
}

/// The five shapes are `GimpLimitType`'s metrics, shared with focus-blur. At one radius they order
/// themselves by how soon the metric reaches 1: Diamond's `|u|+|v|` soonest, Square's
/// `max(|u|,|v|)` latest.
///
/// Measured darkened pixels on a 48-square canvas: Diamond 1991, Circle 1863, Square 1679.
#[test]
fn vignette_shapes_order_by_their_metric() {
    let darkened = |shape: FocusShape| {
        let out = apply_to(48, 48, 200, vignette(shape, 1.0, 1.0, 0.0, 0.0, 0.5, 1.0));
        (0..48 * 48).filter(|i| out[i * 4] < 200).count()
    };

    let diamond = darkened(FocusShape::Diamond);
    let circle = darkened(FocusShape::Circle);
    let square = darkened(FocusShape::Square);

    assert!(
        square < circle && circle < diamond,
        "metrics order the shapes: square {square} < circle {circle} < diamond {diamond}"
    );
}

/// `radius` is a NORMALISED diameter fraction -- `2.0 * radius / area->width` in the propgui -- not a
/// pixel distance. So 1.0 inscribes the canvas and 0.0 leaves no clear region at all.
///
/// This is the opposite convention from supernova, where the centre was normalised but the radius
/// was in pixels, and it is why both had to be read rather than assumed from the other.
#[test]
fn vignette_radius_is_a_normalised_diameter() {
    let inscribed = apply_to(
        48,
        48,
        200,
        vignette(FocusShape::Circle, 1.0, 1.0, 0.0, 0.0, 1.0, 1.0),
    );
    assert_eq!(
        inscribed[(24 * 48 + 24) * 4],
        200,
        "the centre of an inscribed circle is untouched"
    );
    assert_eq!(
        inscribed[(47 * 48 + 47) * 4],
        0,
        "and the corner, outside it, is fully dark"
    );

    let none = apply_to(
        48,
        48,
        200,
        vignette(FocusShape::Circle, 0.0, 1.0, 0.0, 0.0, 0.5, 1.0),
    );
    assert!(
        (0..48 * 48).all(|i| none[i * 4] == 0),
        "radius 0 leaves no clear region"
    );
}

/// `squeeze` scales the region's VERTICAL extent and leaves the horizontal one alone, because
/// `extent_y = extent_x * scale` and only `scale` depends on it.
///
/// Measured on a 48-square canvas with `proportion 0`: the right edge reads 17 at squeeze -0.5, 0
/// and +0.5 alike, while the bottom edge goes 200, 17, 0.
#[test]
fn vignette_squeeze_moves_only_the_vertical_extent() {
    let sample = |squeeze: f64| {
        let out = apply_to(
            48,
            48,
            200,
            vignette(FocusShape::Circle, 1.0, 0.0, squeeze, 0.0, 0.5, 1.0),
        );
        (out[(24 * 48 + 47) * 4], out[(47 * 48 + 24) * 4])
    };

    let (right_negative, bottom_negative) = sample(-0.5);
    let (right_zero, bottom_zero) = sample(0.0);
    let (right_positive, bottom_positive) = sample(0.5);

    assert_eq!(
        (right_negative, right_zero, right_positive),
        (right_zero, right_zero, right_zero),
        "the horizontal extent does not depend on squeeze"
    );
    assert!(
        bottom_negative > bottom_zero && bottom_zero > bottom_positive,
        "while the vertical one does: {bottom_negative} > {bottom_zero} > {bottom_positive}"
    );
}

/// Three of these bounds are not ours: `squeeze`'s `-1..1` is DERIVED from the propgui's
/// `±2/PI * atan(x)`, `rotation`'s `0..360` is READ from its `fmod(fmod(deg,360)+360,360)`, and
/// `gamma`'s ceiling is READ from `#define MAX_GAMMA 1000.0`.
#[test]
fn vignette_refuses_parameters_outside_the_declared_ranges() {
    for bad in [
        vignette(FocusShape::Circle, 1.0, 0.0, 1.5, 0.0, 0.5, 1.0),
        vignette(FocusShape::Circle, 1.0, 0.0, -1.5, 0.0, 0.5, 1.0),
        vignette(FocusShape::Circle, 1.0, 0.0, 0.0, 361.0, 0.5, 1.0),
        vignette(FocusShape::Circle, 1.0, 0.0, 0.0, -1.0, 0.5, 1.0),
        vignette(FocusShape::Circle, 1.0, 0.0, 0.0, 0.0, 0.5, 1_000.5),
        vignette(FocusShape::Circle, 1.0, 0.0, 0.0, 0.0, 0.5, 0.0),
        vignette(FocusShape::Circle, 1.0, 1.5, 0.0, 0.0, 0.5, 1.0),
        vignette(FocusShape::Circle, 1.0, 0.0, 0.0, 0.0, 1.5, 1.0),
        vignette(FocusShape::Circle, 5.0, 0.0, 0.0, 0.0, 0.5, 1.0),
        vignette(FocusShape::Circle, f64::NAN, 0.0, 0.0, 0.0, 0.5, 1.0),
    ] {
        let field = vec![
            Pixel {
                r: 200,
                g: 200,
                b: 200,
                a: 255
            };
            64
        ];
        let mut editor = image(8, 8, &field);
        assert!(
            editor
                .execute(Command::ApplyFilter { filter: bad })
                .is_err(),
            "a parameter outside the declared range must be refused"
        );
    }
}

/// Nine properties, matching the propgui's nine, and `shape` reuses the enum focus-blur already
/// declared because upstream reads the same `GimpLimitType` in both.
#[test]
fn vignette_deserialises_with_ten_fields() {
    let filter: Filter = serde_json::from_str(r#"{"kind":"vignette"}"#).expect("deserialise");
    match filter {
        Filter::Vignette {
            shape,
            x,
            y,
            radius,
            proportion,
            squeeze,
            rotation,
            softness,
            gamma,
            color,
        } => {
            assert_eq!(shape, FocusShape::Circle, "the enum's first variant");
            // K.17a added the tenth field, so the name above moved from nine. Asserted here rather
            // than bound and ignored -- clippy caught exactly that, and a bound-but-unchecked value
            // is the same mistake as a test that only checks a call succeeded.
            assert_eq!(color, Pixel::rgba(0, 0, 0, 255), "upstream's own \"black\"");
            assert!((x - 0.5).abs() < f64::EPSILON && (y - 0.5).abs() < f64::EPSILON);
            assert!((proportion - 1.0).abs() < f64::EPSILON);
            assert!(squeeze.abs() < f64::EPSILON && rotation.abs() < f64::EPSILON);
            // **K.17f moved these three to upstream's own values**, and each old one was a
            // special case rather than a choice: radius 1.0 inscribed the canvas exactly,
            // gamma 1.0 is the LINEAR curve of a property upstream calls "Falloff linearity",
            // and softness 0.5 narrowed the ramp.
            assert!(
                (radius - 1.2).abs() < f64::EPSILON,
                "was 1.0, inscribing the canvas; upstream reaches a fifth beyond it"
            );
            assert!((softness - 0.8).abs() < f64::EPSILON, "was 0.5");
            assert!(
                (gamma - 2.0).abs() < f64::EPSILON,
                "was 1.0, a linear curve"
            );
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

const NOVA_WHITE: Pixel = Pixel {
    r: 255,
    g: 255,
    b: 255,
    a: 255,
};

const NOVA_RED: Pixel = Pixel {
    r: 255,
    g: 0,
    b: 0,
    a: 255,
};

fn supernova(center: f64, radius: u32, spokes: u32, random_hue: f64, color: Pixel) -> Filter {
    Filter::Supernova {
        center_x: center,
        center_y: center,
        radius,
        color,
        spokes,
        random_hue,
    }
}

/// Count angular RUNS above a threshold around a ring.
///
/// # Why runs and not local maxima
///
/// The first probe counted local maxima and reported 32 for every spoke count, which measured the
/// RASTERISATION rather than the filter: 720 angular samples at radius 18 land repeatedly on the
/// same pixel, so the plateaus manufacture maxima. Counting rising edges of a threshold is immune to
/// that. Rule from cycle 64 -- a tool's output is not evidence until you have understood all of it.
fn ring_runs(pixels: &[u8], size: usize, ring: f64, threshold: u8) -> usize {
    let centre = size as f64 / 2.0;
    let samples = 1440;
    let above: Vec<bool> = (0..samples)
        .map(|i| {
            let a = i as f64 / samples as f64 * std::f64::consts::TAU;
            let x = (centre + ring * a.cos())
                .round()
                .clamp(0.0, size as f64 - 1.0) as usize;
            let y = (centre + ring * a.sin())
                .round()
                .clamp(0.0, size as f64 - 1.0) as usize;
            pixels[(y * size + x) * 4] > threshold
        })
        .collect();
    (0..samples)
        .filter(|&i| above[i] && !above[(i + samples - 1) % samples])
        .count()
}

fn coarse_colours(pixels: &[u8], size: usize) -> usize {
    let mut seen = std::collections::HashSet::new();
    for index in 0..size * size {
        let t = index * 4;
        if u16::from(pixels[t]) + u16::from(pixels[t + 1]) + u16::from(pixels[t + 2]) > 40 {
            seen.insert((pixels[t] / 48, pixels[t + 1] / 48, pixels[t + 2] / 48));
        }
    }
    seen.len()
}

/// `_Spokes:` is a COUNT, and `cos(angle * spokes)` peaks at exactly that many evenly spaced angles,
/// so the count is exact rather than approximate.
///
/// Measured at two different radii so the result cannot be an artefact of one ring: 3, 4, 6, 8 and
/// 12 spokes each give exactly that many angular runs.
#[test]
fn supernova_spoke_count_is_exact() {
    let size = 64;
    let field = flat(size, 0);

    for spokes in [3u32, 4, 6, 8, 12] {
        let out = apply(size, &field, supernova(0.5, 8, spokes, 0.0, NOVA_WHITE));
        assert_eq!(
            ring_runs(&out, size, 14.0, 20),
            spokes as usize,
            "{spokes} spokes must give {spokes} bright runs at radius 14"
        );
        assert_eq!(
            ring_runs(&out, size, 18.0, 10),
            spokes as usize,
            "and the same at radius 18, so it is not an artefact of one ring"
        );
    }
}

/// The propgui's own arithmetic is `x = x1 / area->width`, so the centre is NORMALISED to the
/// canvas. On a 64-square canvas 0.25 is therefore pixel 16.
///
/// This is the assertion that would catch reading it as pixels: at 0.25px the nova would sit in the
/// top-left corner and (16,16) would be black.
#[test]
fn supernova_centre_is_normalised_not_pixels() {
    let size = 64;
    let field = flat(size, 0);
    let out = apply(size, &field, supernova(0.25, 6, 6, 0.0, NOVA_WHITE));

    assert_eq!(
        red_at(&out, size, 16, 16),
        255,
        "0.25 of 64 is pixel 16, so the core is there"
    );
    assert_eq!(
        red_at(&out, size, 32, 32),
        0,
        "and not at the canvas centre"
    );
    assert_eq!(red_at(&out, size, 48, 48), 0, "nor anywhere beyond it");
}

/// `radius` is in PIXELS -- `sqrt (SQR (x2 - x1) + SQR (y2 - y1))` in the propgui -- so it is the
/// other half of the mixed-unit pair, and a larger value lights strictly more of the canvas.
///
/// Measured: 217, 839 and 2424 lit pixels at radius 4, 8 and 16.
#[test]
fn supernova_radius_is_in_pixels_and_scales_the_burst() {
    let size = 64;
    let field = flat(size, 0);

    let lit = |radius: u32| {
        let out = apply(size, &field, supernova(0.5, radius, 6, 0.0, NOVA_WHITE));
        (0..size * size).filter(|i| out[i * 4] > 0).count()
    };

    let small = lit(4);
    let medium = lit(8);
    let large = lit(16);
    assert!(
        small < medium && medium < large,
        "a pixel radius must scale the burst: {small} < {medium} < {large}"
    );
}

/// One assertion about the DIFFERENCE, and it records a real property of the colour space rather
/// than of the filter: `R_andom hue:` rotates HUE, and white has no hue to rotate.
///
/// # This test exists because the first probe measured nothing
///
/// The hue jitter was probed with a white nova and reported 6 distinct colours at both 0 and 180
/// degrees, which looked exactly like a parameter that does nothing. The cause was the input:
/// saturation 0 means every hue maps to the same grey. The same shape as cycle 87's white bloom and
/// cycle 89's diagonal flare -- an input that cannot see the thing being measured.
///
/// Measured: a RED nova goes 6 -> 26 distinct coarse colours, while a white one stays 6 -> 6.
#[test]
fn supernova_random_hue_needs_a_colour_to_rotate() {
    let size = 64;
    let field = flat(size, 0);

    let red_plain = coarse_colours(
        &apply(size, &field, supernova(0.5, 8, 6, 0.0, NOVA_RED)),
        size,
    );
    let red_jittered = coarse_colours(
        &apply(size, &field, supernova(0.5, 8, 6, 180.0, NOVA_RED)),
        size,
    );
    let white_plain = coarse_colours(
        &apply(size, &field, supernova(0.5, 8, 6, 0.0, NOVA_WHITE)),
        size,
    );
    let white_jittered = coarse_colours(
        &apply(size, &field, supernova(0.5, 8, 6, 180.0, NOVA_WHITE)),
        size,
    );

    assert!(
        red_jittered > red_plain,
        "a saturated nova must gain colours: {red_plain} -> {red_jittered}"
    );
    assert_eq!(
        white_plain, white_jittered,
        "an unsaturated one cannot, because white has no hue to rotate"
    );
}

/// The hue offset is a HASH OF THE SPOKE INDEX, not a PRNG, which is this crate's standing
/// invariant and why no seed parameter is declared. So two runs are byte-identical.
#[test]
fn supernova_is_deterministic_without_a_seed() {
    let size = 64;
    let field = flat(size, 0);
    let once = apply(size, &field, supernova(0.5, 8, 7, 200.0, NOVA_RED));
    let twice = apply(size, &field, supernova(0.5, 8, 7, 200.0, NOVA_RED));
    assert_eq!(once, twice, "a hash of an index replays exactly");
}

/// Ranges ours -- the strings give `_Radius:` and `_Spokes:` with no bound. `spokes` 0 is refused
/// because a starburst with no spokes is not one, and `random_hue` is in degrees.
#[test]
fn supernova_refuses_bad_parameters() {
    let size = 8;
    let field = flat(size, 0);
    for bad in [
        supernova(0.5, 0, 6, 0.0, NOVA_WHITE),
        supernova(0.5, 4, 0, 0.0, NOVA_WHITE),
        supernova(0.5, 4, 2_000, 0.0, NOVA_WHITE),
        supernova(0.5, 4, 6, -1.0, NOVA_WHITE),
        supernova(0.5, 4, 6, 361.0, NOVA_WHITE),
        supernova(f64::NAN, 4, 6, 0.0, NOVA_WHITE),
    ] {
        let mut editor = image(size as u32, size as u32, &field);
        assert!(
            editor
                .execute(Command::ApplyFilter { filter: bad })
                .is_err(),
            "an unusable parameter must be refused"
        );
    }
}

/// `Show _position` is dialog state, so the command carries six fields and no seventh.
#[test]
fn supernova_deserialises_with_six_fields() {
    let filter: Filter = serde_json::from_str(r#"{"kind":"supernova"}"#).expect("deserialise");
    match filter {
        Filter::Supernova {
            center_x,
            center_y,
            radius,
            color,
            spokes,
            random_hue,
        } => {
            assert!(
                (center_x - 0.5).abs() < f64::EPSILON && (center_y - 0.5).abs() < f64::EPSILON,
                "a normalised centre defaults to the middle"
            );
            assert_eq!(radius, 20, "chosen default radius");
            assert_eq!(spokes, 8, "chosen default spoke count");
            assert!(random_hue.abs() < f64::EPSILON, "no jitter by default");
            assert_eq!((color.r, color.g, color.b), (255, 255, 255), "white nova");
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

fn lens_flare(x: f64, y: f64) -> Filter {
    Filter::LensFlare { x, y }
}

fn flat(size: usize, level: u8) -> Vec<Pixel> {
    vec![
        Pixel {
            r: level,
            g: level,
            b: level,
            a: 255
        };
        size * size
    ]
}

fn red_at(pixels: &[u8], size: usize, x: usize, y: usize) -> u8 {
    pixels[(y * size + x) * 4]
}

/// The ONE derived fact in this filter, and the thing that makes it a lens flare rather than a glow:
/// light bouncing between lens elements reappears mirrored through the optical axis, so the ghost
/// sits at `(2*cx - x, 2*cy - y)` exactly, with no constant to choose.
///
/// # Why the flare is at (8,4) and not (8,8)
///
/// A diagonal flare cannot test this. Mirroring in x alone, in y alone, or in both all put the ghost
/// on the same diagonal, so the input would agree with three different implementations -- the
/// cycle-80, 83 and 87 trap, where every input was symmetric in the thing being measured. An
/// asymmetric flare separates them: the full mirror is (24,28), x-only would be (24,4) and y-only
/// (8,28), and those are three distinct pixels.
#[test]
fn lens_flare_ghost_is_mirrored_through_the_image_centre() {
    let size = 32;
    let field = flat(size, 40);
    let out = apply(size, &field, lens_flare(8.0, 4.0));

    assert_eq!(
        red_at(&out, size, 8, 4),
        255,
        "the core is brightest at the centre the caller named"
    );
    assert!(
        red_at(&out, size, 24, 28) > 100,
        "the ghost sits at the FULL mirror through the image centre"
    );
    assert_eq!(
        red_at(&out, size, 24, 4),
        40,
        "not at the x-only mirror, which a half-done reflection would light"
    );
    assert_eq!(
        red_at(&out, size, 8, 28),
        40,
        "and not at the y-only mirror either"
    );
}

/// Two distinct lobes along the axis, which is the structural signature. Measured on the diagonal:
/// a peak of 255 at index 8 and a separate peak of 155 at index 24, with untouched background
/// between them.
#[test]
fn lens_flare_has_two_separate_lobes() {
    let size = 32;
    let field = flat(size, 40);
    let out = apply(size, &field, lens_flare(8.0, 8.0));

    let diagonal: Vec<u8> = (0..size).map(|i| red_at(&out, size, i, i)).collect();
    let peaks = (1..size - 1)
        .filter(|&i| diagonal[i] > diagonal[i - 1] && diagonal[i] > diagonal[i + 1])
        .count();

    assert_eq!(
        peaks, 2,
        "a flare and its ghost, not one glow: {diagonal:?}"
    );
    assert_eq!(diagonal[8], 255, "the core");
    assert!(
        diagonal[16] == 40,
        "untouched background separates the lobes"
    );
    assert!(diagonal[24] > diagonal[16], "the ghost is a real lobe");
}

/// A flare ADDS light, and the action is not gated on `writable && alpha` upstream -- consistent,
/// because it needs no shape to read. So no pixel may darken.
#[test]
fn lens_flare_never_darkens_a_pixel() {
    let size = 32;
    let field = flat(size, 40);
    let out = apply(size, &field, lens_flare(8.0, 8.0));

    let darkened = (0..size * size).filter(|i| out[i * 4] < 40).count();
    assert_eq!(darkened, 0, "a flare only adds light");
}

/// The po strings give `_X:` and `_Y:` with no bounds, and a flare whose centre lies off the canvas
/// still throws light onto it. So the coordinate is not clamped to the image.
#[test]
fn lens_flare_centre_may_lie_outside_the_canvas() {
    let size = 32;
    let field = flat(size, 40);
    let out = apply(size, &field, lens_flare(-4.0, -4.0));

    assert!(
        red_at(&out, size, 0, 0) > 40,
        "light from an off-canvas flare still reaches the near corner"
    );
    assert_eq!(
        red_at(&out, size, 31, 31),
        40,
        "and does not reach the far one"
    );
}

/// Ranges OURS: finite, and inside the upstream image-size limit the grid filter already reads from
/// `libgimpbase/gimplimits.h`.
#[test]
fn lens_flare_refuses_an_unusable_centre() {
    let size = 8;
    let field = flat(size, 40);
    for bad in [
        lens_flare(f64::NAN, 0.0),
        lens_flare(0.0, f64::INFINITY),
        lens_flare(600_000.0, 0.0),
    ] {
        let mut editor = image(size as u32, size as u32, &field);
        assert!(
            editor
                .execute(Command::ApplyFilter { filter: bad })
                .is_err(),
            "an unusable centre must be refused"
        );
    }
}

/// `Show _position` is shared by exactly `lens-flare.c` and `nova.c` and toggles a crosshair in the
/// PREVIEW, so it is dialog state rather than a parameter. The command therefore carries two fields
/// and no third.
#[test]
fn lens_flare_deserialises_with_two_fields_only() {
    let filter: Filter = serde_json::from_str(r#"{"kind":"lens_flare"}"#).expect("deserialise");
    match filter {
        Filter::LensFlare { x, y } => {
            assert!(
                x.abs() < f64::EPSILON && y.abs() < f64::EPSILON,
                "chosen default centre"
            );
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

fn long_shadow(angle: f64, length: u32) -> Filter {
    Filter::LongShadow {
        angle,
        length,
        color: Pixel {
            r: 0,
            g: 0,
            b: 0,
            a: 255,
        },
    }
}

fn lit_columns(pixels: &[u8], size: usize, row: usize) -> Vec<usize> {
    (0..size)
        .filter(|x| pixels[(row * size + x) * 4 + 3] > 0)
        .collect()
}

/// The one assertion about the DIFFERENCE, which is what cycle 82 settled on, and here it is forced:
/// upstream ships BOTH `gegl:dropshadow` and `gegl:long-shadow`, so by the source-7 rule they are
/// two mechanisms and not one filter with different numbers.
///
/// Drop shadow DISPLACES one copy of the alpha shape. Long shadow fills the ENTIRE SWEPT PATH. At a
/// distance greater than the shape's own size the consequence is visible and exact: drop shadow
/// leaves a GAP between caster and shadow, and long shadow cannot.
///
/// Predicted before running, and both exact: 48 covered against 32, with the long shadow's row
/// contiguous from 8 to 19 while the drop shadow's is 8..11 and 16..19.
#[test]
fn long_shadow_sweeps_the_path_where_drop_shadow_leaves_a_gap() {
    let size = 32;
    let field = square(size, 8, 4);

    let swept = apply(size, &field, long_shadow(0.0, 8));
    let dropped = apply(size, &field, drop_shadow(8, 0, 0, 100.0));

    assert_eq!(
        covered(&swept),
        48,
        "the swept path is the caster plus every pixel along the ray"
    );
    assert_eq!(
        covered(&dropped),
        32,
        "a displaced copy is the caster plus one shape of equal area"
    );

    assert_eq!(
        lit_columns(&swept, size, 9),
        (8..=19).collect::<Vec<_>>(),
        "a long shadow is contiguous with the shape that casts it"
    );
    assert_eq!(
        lit_columns(&dropped, size, 9),
        vec![8, 9, 10, 11, 16, 17, 18, 19],
        "a drop shadow at a distance greater than the shape leaves a gap"
    );
}

/// y grows DOWNWARD on a canvas, so 45 degrees -- the conventional long-shadow direction and this
/// variant's recorded default -- falls right AND down, not merely right.
///
/// Measured: the diagonal pixel at (13,13) is shadowed while (16,8), the same distance away
/// horizontally, is not.
#[test]
fn long_shadow_angle_is_measured_with_y_downward() {
    let size = 32;
    let field = square(size, 8, 4);
    let diagonal = apply(size, &field, long_shadow(45.0, 8));

    assert_eq!(
        alpha_at(&diagonal, size, 13, 13),
        255,
        "45 degrees falls right and DOWN"
    );
    assert_eq!(
        alpha_at(&diagonal, size, 16, 8),
        0,
        "so it does not fall straight right, which is what 0 degrees would do"
    );
}

/// 270 degrees is the opposite vertical, so the shadow is cast UPWARD -- above the shape, not below.
/// Measured: column 9 lit at rows 2..=11, which is six rows of shadow above the four-row caster.
#[test]
fn long_shadow_casts_the_other_way_at_the_opposite_angle() {
    let size = 32;
    let field = square(size, 8, 4);
    let upward = apply(size, &field, long_shadow(270.0, 6));

    let rows: Vec<usize> = (0..size)
        .filter(|y| upward[(y * size + 9) * 4 + 3] > 0)
        .collect();
    assert_eq!(
        rows,
        (2..=11).collect::<Vec<_>>(),
        "270 degrees puts the shadow above the caster"
    );
}

/// `length` 0 is a legal request for no shadow, exactly as drop shadow's `radius` 0 is.
#[test]
fn long_shadow_zero_length_changes_nothing() {
    let size = 32;
    let field = square(size, 8, 4);
    let before = flatten(&field);
    let after = apply(size, &field, long_shadow(45.0, 0));
    assert_eq!(before, after, "no length means no shadow");
}

/// The same alpha gating as drop shadow -- `writable && alpha` -- and the same consequence: an
/// opaque layer composites over its own shadow and hides it completely.
#[test]
fn long_shadow_on_a_fully_opaque_layer_is_a_no_op() {
    let size = 32;
    let flat = vec![
        Pixel {
            r: 120,
            g: 180,
            b: 90,
            a: 255
        };
        size * size
    ];
    let before = flatten(&flat);
    let after = apply(size, &flat, long_shadow(45.0, 12));
    assert_eq!(before, after, "an opaque layer hides the shadow it casts");
}

/// There is no separate opacity parameter, because `color` is a `Pixel` whose own alpha carries it.
/// A half-transparent shadow colour must therefore give a half-strength shadow.
#[test]
fn long_shadow_opacity_rides_on_the_colours_own_alpha() {
    let size = 32;
    let field = square(size, 8, 4);

    let half = Filter::LongShadow {
        angle: 0.0,
        length: 8,
        color: Pixel {
            r: 0,
            g: 0,
            b: 0,
            a: 128,
        },
    };
    let out = apply(size, &field, half);

    let shadowed = alpha_at(&out, size, 15, 9);
    assert!(
        (120..=136).contains(&shadowed),
        "a colour at alpha 128 must cast at about half strength, got {shadowed}"
    );
    assert_eq!(
        alpha_at(&out, size, 9, 9),
        255,
        "the caster itself is untouched"
    );
}

/// Ranges OURS -- nothing upstream declares any, because nothing upstream declares the parameters.
#[test]
fn long_shadow_refuses_parameters_outside_our_ranges() {
    let size = 8;
    let field = square(size, 2, 2);
    for bad in [
        long_shadow(-1.0, 4),
        long_shadow(360.5, 4),
        long_shadow(f64::NAN, 4),
        long_shadow(45.0, 4_097),
    ] {
        let mut editor = image(size as u32, size as u32, &field);
        assert!(
            editor
                .execute(Command::ApplyFilter { filter: bad })
                .is_err(),
            "a parameter outside our declared range must be refused"
        );
    }
}

/// Both defaults are recorded CHOICES rather than readings -- no source states either.
#[test]
fn long_shadow_deserialises_with_our_recorded_choices() {
    let filter: Filter = serde_json::from_str(r#"{"kind":"long_shadow"}"#).expect("deserialise");
    match filter {
        Filter::LongShadow {
            angle,
            length,
            color,
        } => {
            assert!((angle - 45.0).abs() < f64::EPSILON, "chosen default angle");
            assert_eq!(length, 20, "chosen default length");
            assert_eq!(
                (color.r, color.g, color.b),
                (0, 0, 0),
                "a shadow's colour defaults to black"
            );
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

/// Every default READ from `drop-shadow.scm`'s own argument list: offsets 4, blur 15, opacity 60,
/// colour black.
#[test]
fn drop_shadow_deserialises_with_the_scripts_defaults() {
    let filter: Filter = serde_json::from_str(r#"{"kind":"drop_shadow"}"#).expect("deserialise");
    match filter {
        Filter::DropShadow {
            offset_x,
            offset_y,
            radius,
            color,
            opacity,
        } => {
            assert_eq!(
                (offset_x, offset_y),
                (4, 4),
                "Offset X / Offset Y default 4"
            );
            assert_eq!(radius, 15, "Blur radius defaults to 15");
            assert!(
                (opacity - 60.0).abs() < f64::EPSILON,
                "Opacity defaults to 60"
            );
            assert_eq!(
                (color.r, color.g, color.b),
                (0, 0, 0),
                "the script's Color argument is \"black\""
            );
        }
        other => panic!("wrong variant: {other:?}"),
    }
}

/// Both bounds READ from `drop-shadow.scm`: offsets `-4096..4096`, blur `0..1024`, opacity `0..100`.
#[test]
fn drop_shadow_refuses_parameters_outside_the_declared_ranges() {
    let size = 8;
    let field = square(size, 2, 2);
    for bad in [
        drop_shadow(4_097, 0, 4, 60.0),
        drop_shadow(0, -4_097, 4, 60.0),
        drop_shadow(4, 4, 1_025, 60.0),
        drop_shadow(4, 4, 4, 100.5),
        drop_shadow(4, 4, 4, -1.0),
        drop_shadow(4, 4, 4, f64::NAN),
    ] {
        let mut editor = image(size as u32, size as u32, &field);
        assert!(
            editor
                .execute(Command::ApplyFilter { filter: bad })
                .is_err(),
            "a parameter outside the script's own range must be refused"
        );
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

/// A saved command with nothing but the kind loads — upstream's values, in OUR units.
///
/// # All three of these are upstream's, and two needed converting to see that
///
/// `gegl:bloom` declares `threshold` and `strength` on `ui_range (0.0, 100.0)` while we carry both
/// on 0..1, so upstream's 50.0 is our 0.5 in each case. `threshold` already matched once converted
/// — it was one of the false divergences cycle 21's `DEFAULT_UNITS` removed. `strength` did not:
/// ours was 1.0 against upstream's 0.5, so the default glow was **double**.
///
/// The unconverted report read *"upstream 50.0 vs ours 1.0"*, a factor of fifty, where the real gap
/// is a factor of two. That is the reason the unit table exists at all: a reader triaging by
/// apparent size would have started here instead of on the genuine outliers.
///
/// `radius` is the odd one — upstream declares `10.0` as a float pixel-distance and we carry `u32`,
/// and the value already agreed.
#[test]
fn bloom_deserialises_with_upstreams_values_in_our_units() {
    let filter: Filter =
        serde_json::from_str(r#"{"kind":"bloom"}"#).expect("older saved commands must load");
    let Filter::Bloom {
        threshold,
        radius,
        strength,
    } = filter
    else {
        panic!("wrong variant");
    };
    assert_eq!(threshold, 0.5, "upstream's 50.0 on a 0..100 scale");
    assert_eq!(radius, 10, "upstream's 10.0, already agreeing");
    assert_eq!(
        strength, 0.5,
        "was 1.0 -- upstream's 50.0 is HALF our unit, not all of it"
    );

    // And the correction is visible: strength scales how much spill is added back, so halving it
    // must lift a bright patch's surroundings LESS. Measured on the ring around a white square.
    let surround = |strength: f64| {
        let mut colors = vec![Pixel::rgba(20, 20, 20, 255); 32 * 32];
        for y in 12..20 {
            for x in 12..20 {
                colors[y * 32 + x] = Pixel::rgba(255, 255, 255, 255);
            }
        }
        let mut editor = image(32, 32, &colors);
        editor
            .execute(Command::ApplyFilter {
                filter: Filter::Bloom {
                    threshold: 0.5,
                    radius: 6,
                    strength,
                },
            })
            .unwrap();
        let out = pixels(&editor);
        // A pixel just outside the square, where only spill can reach.
        i32::from(out[(16 * 32 + 23) * 4])
    };

    let half = surround(0.5);
    let full = surround(1.0);
    assert!(
        half > 20,
        "the spill must reach outside the square at all, got {half}"
    );
    assert!(
        half < full,
        "and half the strength must add less: {half} against {full}"
    );
}

/// K.17's first itemised fix: `gegl:vignette` declares `property_color (color, _("Color"), "black")`
/// and this product had no colour at all.
#[test]
fn a_black_vignette_colour_reproduces_the_filter_as_it_was_before_the_parameter_existed() {
    // **The property that makes this addition safe.** The blend became
    // `base * keep + colour * darkening`, which is `base * keep` exactly when the colour is zero --
    // so no document that omits the field can render differently. Asserted against the default
    // rather than argued for in a comment.
    let plain = apply_to(
        16,
        16,
        200,
        vignette(FocusShape::Circle, 0.5, 1.0, 0.0, 0.0, 1.0, 1.0),
    );
    let explicit_black = apply_to(
        16,
        16,
        200,
        Filter::Vignette {
            shape: FocusShape::Circle,
            x: 0.5,
            y: 0.5,
            radius: 0.5,
            proportion: 1.0,
            squeeze: 0.0,
            rotation: 0.0,
            softness: 1.0,
            gamma: 1.0,
            color: Pixel::rgba(0, 0, 0, 255),
        },
    );
    assert_eq!(plain, explicit_black);
}

#[test]
fn the_vignette_colour_is_what_the_edge_darkens_toward() {
    // A fully darkened corner must become the colour, not black. Red makes that unmistakable: a
    // black-blending implementation produces 0 in the red channel where this produces 255.
    let red = apply_to(
        16,
        16,
        200,
        Filter::Vignette {
            shape: FocusShape::Circle,
            x: 0.5,
            y: 0.5,
            radius: 0.5,
            proportion: 1.0,
            squeeze: 0.0,
            rotation: 0.0,
            softness: 1.0,
            gamma: 1.0,
            color: Pixel::rgba(255, 0, 0, 255),
        },
    );

    // The corner is outside the region, so it is fully darkened: pure colour.
    let corner = 0;
    assert_eq!(&red[corner..corner + 3], &[255, 0, 0]);

    // The centre is inside the inner limit, so it is untouched whatever the colour is.
    let centre = (8 * 16 + 8) * 4;
    assert_eq!(&red[centre..centre + 3], &[200, 200, 200]);
}

#[test]
fn the_vignette_colour_never_changes_alpha() {
    // Upstream keeps the alpha channel untouched, so the colour's own alpha is never consulted.
    // A transparent colour must tint exactly as an opaque one does.
    let opaque = apply_to(
        8,
        8,
        128,
        Filter::Vignette {
            shape: FocusShape::Circle,
            x: 0.5,
            y: 0.5,
            radius: 0.5,
            proportion: 1.0,
            squeeze: 0.0,
            rotation: 0.0,
            softness: 1.0,
            gamma: 1.0,
            color: Pixel::rgba(0, 255, 0, 255),
        },
    );
    let transparent = apply_to(
        8,
        8,
        128,
        Filter::Vignette {
            shape: FocusShape::Circle,
            x: 0.5,
            y: 0.5,
            radius: 0.5,
            proportion: 1.0,
            squeeze: 0.0,
            rotation: 0.0,
            softness: 1.0,
            gamma: 1.0,
            color: Pixel::rgba(0, 255, 0, 0),
        },
    );
    assert_eq!(opaque, transparent, "the colour's alpha must be ignored");

    // And every pixel keeps the source alpha, which here is opaque.
    assert!(opaque.chunks_exact(4).all(|pixel| pixel[3] == 255));
}

#[test]
fn a_zero_radius_fills_the_whole_image_with_the_colour() {
    // The degenerate branch used to write a hard-coded zero, which agreed with black by accident.
    // With a colour it has to write the colour, and a non-black one is the only way to tell the
    // two apart.
    let filled = apply_to(
        4,
        4,
        200,
        Filter::Vignette {
            shape: FocusShape::Circle,
            x: 0.5,
            y: 0.5,
            radius: 0.0,
            proportion: 1.0,
            squeeze: 0.0,
            rotation: 0.0,
            softness: 1.0,
            gamma: 1.0,
            color: Pixel::rgba(10, 20, 30, 255),
        },
    );
    for pixel in filled.chunks_exact(4) {
        assert_eq!(&pixel[..3], &[10, 20, 30]);
        assert_eq!(pixel[3], 255);
    }
}

#[test]
fn vignette_deserialises_its_colour_as_black_when_absent() {
    // Upstream's default is the string "black", so an omitted colour must be opaque black -- the
    // value that makes the filter behave as it did before the field existed.
    let filter: Filter = serde_json::from_str(r#"{"kind":"vignette"}"#).expect("deserialise");
    match filter {
        Filter::Vignette { color, .. } => {
            assert_eq!(color, Pixel::rgba(0, 0, 0, 255));
        }
        other => panic!("expected a vignette, got {other:?}"),
    }
}

/// The corrected vignette defaults reach the canvas, and the EDGE MIDPOINT is the place to look.
///
/// # A first version of this test looked at the corner, and was wrong
///
/// It asserted that upstream's `radius` of 1.2 leaves the corner uncrushed, on the reasoning that
/// 1.2 reaches "a fifth beyond the edge". **Measured: the corner still comes back 0.** The radius
/// is a portion of the half-WIDTH, and a square canvas puts its corner at `sqrt(2)` times the
/// half-width — about 1.414 — so 1.2 does not reach it. Sparing the corner would need a radius
/// above 1.414.
///
/// What 1.2 does reach beyond is the **edge midpoint**, at exactly 1.0 of the half-width, and that
/// is where the correction shows:
///
/// | point | distance | old (r 1.0, s 0.5) | new (r 1.2, s 0.8) |
/// |---|---|---|---|
/// | centre | 0.0 | untouched | untouched |
/// | edge midpoint | 1.0 | at `radius1` -> crushed | inside [0.24, 1.2] -> shaded |
/// | corner | ~1.414 | past `radius1` -> crushed | past `radius1` -> crushed |
///
/// So the claim this pins is narrower than the one I first wrote, and it is the one that is true.
#[test]
fn vignette_defaults_shade_the_edge_midpoint_instead_of_crushing_it() {
    let flat = vec![Pixel::rgba(200, 200, 200, 255); 33 * 33];
    let mut editor = image(33, 33, &flat);
    let filter: Filter = serde_json::from_str(r#"{"kind":"vignette"}"#).expect("deserialise");
    editor.execute(Command::ApplyFilter { filter }).unwrap();
    let out = pixels(&editor);

    let at = |x: usize, y: usize| i32::from(out[(y * 33 + x) * 4]);
    let centre = at(16, 16);
    let midpoint = at(32, 16);
    let corner = at(0, 0);

    assert_eq!(
        centre, 200,
        "the centre sits inside the falloff and is untouched"
    );
    assert!(
        midpoint < centre,
        "the edge midpoint must be darkened: {midpoint} against {centre}"
    );
    // The discriminating half: at radius 1.2 the midpoint is still INSIDE the falloff and keeps
    // some of the image. The old radius of 1.0 put it exactly AT the outer edge, crushed to the
    // colour. A floor of 1 rather than 0 is what separates "shaded" from "crushed".
    assert!(
        midpoint > 0,
        "and must keep some signal rather than being crushed, got {midpoint}"
    );
    // And the corner is asserted as crushed, so the table above stays honest rather than being a
    // comment nobody checks.
    assert_eq!(
        corner, 0,
        "the corner is past 1.2 of the half-width and is still crushed"
    );
}
