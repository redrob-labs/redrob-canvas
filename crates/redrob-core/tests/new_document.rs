//! File > New (Ctrl+N), batch 4 item H4.

use redrob_core::{Document, Pixel};

#[test]
fn a_new_document_has_one_layer_filled_with_the_colour() {
    let document = Document::new_filled(4, 3, Pixel::rgba(255, 255, 255, 255)).unwrap();
    assert_eq!((document.width(), document.height()), (4, 3));
    assert_eq!(document.nodes().len(), 1);
    assert!(document.nodes()[0].pixels().iter().all(|b| *b == 255));
}

#[test]
fn a_transparent_new_document_is_empty() {
    let document = Document::new_filled(2, 2, Pixel::rgba(9, 9, 9, 0)).unwrap();
    assert!(document.nodes()[0].pixels().iter().all(|b| *b == 0));
}

#[test]
fn a_zero_size_is_refused() {
    assert!(Document::new_filled(0, 5, Pixel::rgba(0, 0, 0, 255)).is_err());
}
