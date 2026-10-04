//! K.7, light and shadow.

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
