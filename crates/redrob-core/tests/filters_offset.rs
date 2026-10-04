//! L.2, wrap-around layer offset — `gimp:offset`.
//!
//! Transcribed from `app/operations/gimpoperationoffset.c`, which is vendored because this is one of
//! GIMP's own operations rather than a GEGL one.

#[path = "common/canvas.rs"]
mod canvas;

use redrob_core::{Command, Filter, OffsetType, Pixel};

const N: u32 = 12;

const FILL: Pixel = Pixel {
    r: 9,
    g: 9,
    b: 9,
    a: 255,
};

/// Every column carries a distinct red value, so a horizontal shift is readable straight off the
/// row. A flat canvas could not show a shift at all.
fn columns() -> Vec<Pixel> {
    let mut field = Vec::new();
    for _y in 0..N {
        for x in 0..N {
            field.push(Pixel {
                r: (x * 20 + 5) as u8,
                g: 60,
                b: 30,
                a: 255,
            });
        }
    }
    field
}

fn offset(x: i32, y: i32, offset_type: OffsetType) -> Vec<u8> {
    let mut editor = canvas::editor(N, N, &columns());
    editor
        .execute(Command::ApplyFilter {
            filter: Filter::Offset {
                x,
                y,
                offset_type,
                color: FILL,
            },
        })
        .expect("filter");
    editor.document().layers()[0].pixels().to_vec()
}

/// The red channel across row 5.
fn row(pixels: &[u8]) -> Vec<u8> {
    (0..N).map(|x| pixels[((5 * N + x) * 4) as usize]).collect()
}

const INPUT: [u8; 12] = [5, 25, 45, 65, 85, 105, 125, 145, 165, 185, 205, 225];

/// Wrapping takes a **positive modulo** into `0..extent`:
///
/// ```c
/// *x %= bounds.width;  if (*x < 0) *x += bounds.width;
/// ```
///
/// So a negative offset is exactly equal to its positive complement. Not "visually similar" --
/// byte-identical, which is what the test asserts.
#[test]
fn offset_wrap_makes_a_negative_shift_equal_its_positive_complement() {
    assert_eq!(
        row(&offset(1, 0, OffsetType::WrapAround)),
        vec![225, 5, 25, 45, 65, 85, 105, 125, 145, 165, 185, 205],
        "column 0 takes what was the last column"
    );

    assert_eq!(
        offset(-1, 0, OffsetType::WrapAround),
        offset((N - 1) as i32, 0, OffsetType::WrapAround),
        "-1 and width-1 are the same offset after the positive modulo"
    );
}

/// The sharpest consequence of the read normalisation: **one number, two opposite results.**
///
/// Upstream calls `get_offset` and only THEN tests `if (x == 0 && y == 0)`. Under wrapping, an offset
/// of exactly one full extent takes the modulo to 0 and so passes the input straight through. Under a
/// fill type it is instead `CLAMP (x, -width, +width)`, which keeps `width` and pushes the entire
/// image off the canvas.
///
/// A single implementation that normalised both types the same way could not produce both of these.
#[test]
fn offset_by_one_full_extent_passes_through_or_vacates_depending_on_type() {
    assert_eq!(
        row(&offset(N as i32, 0, OffsetType::WrapAround)),
        INPUT.to_vec(),
        "wrapping reduces a full extent to zero, so nothing moves"
    );

    let vacated = offset(N as i32, 0, OffsetType::Color);
    assert!(
        vacated.chunks(4).all(|p| p == [9, 9, 9, 255]),
        "clamping keeps the full extent, so the whole canvas is background"
    );
}

/// The two fill types differ only in what fills the vacated area, and the shifted content is
/// identical in both. Asserted as one claim about the difference.
#[test]
fn offset_fill_types_differ_only_in_the_vacated_area() {
    let coloured = offset(3, 0, OffsetType::Color);
    let clear = offset(3, 0, OffsetType::Transparent);

    assert_eq!(
        row(&coloured),
        vec![9, 9, 9, 5, 25, 45, 65, 85, 105, 125, 145, 165],
        "three columns of fill, then the image"
    );

    for x in 0..3 {
        let at = ((5 * N + x) * 4) as usize;
        assert_eq!(coloured[at + 3], 255, "the colour fill is opaque");
        assert_eq!(clear[at + 3], 0, "the transparent fill is not");
    }
    for x in 3..N {
        let at = ((5 * N + x) * 4) as usize;
        assert_eq!(
            coloured[at..at + 4],
            clear[at..at + 4],
            "beyond the vacated area the two types agree exactly"
        );
    }
}

/// Both axes, and the vertical one wraps by the same rule.
#[test]
fn offset_wraps_vertically_too() {
    let down = offset(0, 2, OffsetType::WrapAround);
    let original = offset(0, 0, OffsetType::WrapAround);

    for x in 0..N {
        assert_eq!(
            down[(x * 4) as usize],
            original[(((N - 2) * N + x) * 4) as usize],
            "row 0 takes what was row {}",
            N - 2
        );
    }
}

/// `x` and `y` are `g_param_spec_int` over `G_MININT, G_MAXINT, 0` — signed and unbounded, with zero
/// the default. A zero offset returns the input untouched.
#[test]
fn offset_deserialises_to_a_no_op() {
    let filter: Filter = serde_json::from_str(r#"{"kind":"offset"}"#).expect("deserialise");
    match filter {
        Filter::Offset {
            x,
            y,
            offset_type,
            color: _,
        } => {
            assert_eq!((x, y), (0, 0));
            assert_eq!(offset_type, OffsetType::Color, "the enum's first member");
        }
        other => panic!("wrong variant: {other:?}"),
    }

    assert_eq!(row(&offset(0, 0, OffsetType::Color)), INPUT.to_vec());
}
