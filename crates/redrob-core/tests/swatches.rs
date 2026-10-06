//! Swatch files (.gpl / .aco), batch 4 item M7.

use redrob_core::Pixel;
use redrob_core::swatches::{parse_swatches, write_gpl};

#[test]
fn a_gimp_palette_round_trips() {
    let colours = vec![Pixel::rgba(255, 0, 0, 255), Pixel::rgba(1, 2, 3, 255)];
    let text = write_gpl("Mine", &colours);
    assert!(text.starts_with("GIMP Palette\nName: Mine\n"));
    assert_eq!(parse_swatches(text.as_bytes()).unwrap(), colours);
}

#[test]
fn gimp_comments_and_names_are_skipped() {
    let text =
        b"GIMP Palette\nName: X\nColumns: 4\n# comment\n 10  20  30\tBlue-ish\n\n300 0 0 bad\n";
    assert_eq!(
        parse_swatches(text).unwrap(),
        vec![Pixel::rgba(10, 20, 30, 255)]
    );
}

#[test]
fn a_photoshop_aco_v1_reads_rgb_and_gray() {
    // version 1, 2 colours: RGB (0) 0xffff/0x8000/0 and grayscale (8) 0 (= white).
    let mut aco = vec![0, 1, 0, 2];
    aco.extend_from_slice(&[0, 0, 0xff, 0xff, 0x80, 0x00, 0, 0, 0, 0]);
    aco.extend_from_slice(&[0, 8, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(
        parse_swatches(&aco).unwrap(),
        vec![
            Pixel::rgba(255, 128, 0, 255),
            Pixel::rgba(255, 255, 255, 255)
        ]
    );
}

#[test]
fn a_truncated_aco_is_refused() {
    assert!(parse_swatches(&[0, 1, 0, 5, 0, 0]).is_err());
    assert!(parse_swatches(b"not swatches").is_err());
}
