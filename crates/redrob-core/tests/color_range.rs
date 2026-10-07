//! Select > Color Range, batch 4 item M6.

use redrob_core::{ColorRange, Command, Document, Editor, LayerLocks, Pixel, Rect};

/// Left half red, right half dark grey.
fn two_tone() -> Editor {
    let mut editor = Editor::new(Document::new(4, 1).unwrap()).unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(30, 30, 30, 255),
        })
        .unwrap();
    editor
        .execute(Command::SelectRectangle {
            rect: Rect {
                x: 0,
                y: 0,
                width: 2,
                height: 1,
            },
            mode: Default::default(),
        })
        .unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(220, 20, 20, 255),
        })
        .unwrap();
    editor.execute(Command::ClearSelection).unwrap();
    editor
}

fn pick(editor: &mut Editor, color: Pixel, fuzziness: u8, range: ColorRange) -> Vec<u8> {
    editor
        .execute(Command::SelectColorRange {
            color,
            fuzziness,
            range,
            mode: Default::default(),
        })
        .unwrap();
    editor.document().selection().mask().to_vec()
}

#[test]
fn sampled_colour_selects_matching_pixels_everywhere() {
    let mut editor = two_tone();
    let mask = pick(
        &mut editor,
        Pixel::rgba(220, 20, 20, 255),
        40,
        ColorRange::Sampled,
    );
    assert_eq!(mask, vec![255, 255, 0, 0]);
}

#[test]
fn fuzziness_gives_a_soft_edge() {
    let mut editor = two_tone();
    // Partly selected: a sample whose colour difference from the red is past half the
    // fuzziness but inside it. The difference is perceptual (Lab), so find such a sample.
    let red = Pixel::rgba(220, 20, 20, 255);
    let sample = (0..=255u8)
        .map(|g| Pixel::rgba(220, g, 20, 255))
        .find(|p| (24..=36).contains(&redrob_core::colour_difference(red, *p)))
        .expect("a sample 24..36 away");
    let mask = pick(&mut editor, sample, 40, ColorRange::Sampled);
    assert!(mask[0] > 0 && mask[0] < 255, "{mask:?}");
}

#[test]
fn shadows_pick_the_dark_half() {
    let mut editor = two_tone();
    let mask = pick(
        &mut editor,
        Pixel::rgba(0, 0, 0, 255),
        1,
        ColorRange::Shadows,
    );
    assert_eq!(&mask[2..], &[255, 255]);
    assert!(mask[0] < 255);
}

#[test]
fn a_pixel_locked_layer_can_still_be_selected_from() {
    // Selecting reads pixels; the pixel lock (M2) stops writes only.
    let mut editor = two_tone();
    let id = editor.document().active_layer_id();
    editor
        .execute(Command::SetLayerLocks {
            id,
            locks: LayerLocks {
                pixels: true,
                ..LayerLocks::default()
            },
        })
        .unwrap();
    pick(
        &mut editor,
        Pixel::rgba(220, 20, 20, 255),
        40,
        ColorRange::Sampled,
    );
}
