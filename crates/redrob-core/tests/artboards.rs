//! Artboards, batch 4 item L8.

use redrob_core::{Artboard, Command, Document, Editor, LayerId, Pixel};

fn render(editor: &Editor) -> Vec<u8> {
    editor.render_snapshot().unwrap().pixels().to_vec()
}

fn board(x: i32, y: i32, w: u32, h: u32, background: Option<[u8; 3]>) -> Artboard {
    Artboard { x, y, width: w, height: h, background }
}

/// A 16x16 document whose only layer is filled red and moved into an artboard group.
fn setup() -> (Editor, LayerId) {
    let mut editor = Editor::new(Document::new(16, 16).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    editor.execute(Command::SelectAll).unwrap();
    editor.execute(Command::Fill { color: Pixel::rgba(255, 0, 0, 255) }).unwrap();
    editor.execute(Command::ClearSelection).unwrap();
    let group = LayerId::new();
    editor
        .execute(Command::AddGroup { id: group, name: "Artboard 1".into(), parent: None, sibling_index: 1 })
        .unwrap();
    editor.execute(Command::MoveNode { id: layer, parent: Some(group), sibling_index: 0 }).unwrap();
    (editor, group)
}

fn at(pixels: &[u8], x: usize, y: usize) -> &[u8] {
    &pixels[(y * 16 + x) * 4..(y * 16 + x) * 4 + 4]
}

#[test]
fn children_are_clipped_to_the_artboard() {
    let (mut editor, group) = setup();
    editor.execute(Command::SetArtboard { id: group, artboard: Some(board(4, 4, 8, 8, None)) }).unwrap();
    let pixels = render(&editor);
    assert_eq!(at(&pixels, 5, 5), &[255, 0, 0, 255]);
    assert_eq!(at(&pixels, 1, 1)[3], 0, "outside the board is empty");
    assert_eq!(at(&pixels, 12, 12)[3], 0);
}

#[test]
fn the_background_shows_under_transparent_children() {
    let (mut editor, group) = setup();
    // Erase the red so only the board's white shows.
    let layer = editor.document().nodes().iter().find(|n| n.parent_id() == Some(group)).unwrap().id();
    editor.execute(Command::SetActiveLayer { id: layer }).unwrap();
    editor.execute(Command::SelectAll).unwrap();
    editor.execute(Command::Clear).unwrap();
    editor.execute(Command::SetArtboard { id: group, artboard: Some(board(0, 0, 8, 8, Some([255, 255, 255]))) }).unwrap();
    let pixels = render(&editor);
    assert_eq!(at(&pixels, 2, 2), &[255, 255, 255, 255]);
    assert_eq!(at(&pixels, 10, 10)[3], 0);
}

#[test]
fn only_groups_can_be_artboards_and_empty_boards_are_refused() {
    let (mut editor, group) = setup();
    let layer = editor.document().nodes().iter().find(|n| n.parent_id() == Some(group)).unwrap().id();
    assert!(editor.execute(Command::SetArtboard { id: layer, artboard: Some(board(0, 0, 4, 4, None)) }).is_err());
    assert!(editor.execute(Command::SetArtboard { id: group, artboard: Some(board(0, 0, 0, 4, None)) }).is_err());
    // Clearing turns it back into a plain group: no clipping.
    editor.execute(Command::SetArtboard { id: group, artboard: Some(board(0, 0, 4, 4, None)) }).unwrap();
    editor.execute(Command::SetArtboard { id: group, artboard: None }).unwrap();
    assert_eq!(at(&render(&editor), 10, 10), &[255, 0, 0, 255]);
}

#[test]
fn the_artboard_survives_a_save_round_trip() {
    let (mut editor, group) = setup();
    let b = board(1, 2, 3, 4, Some([9, 8, 7]));
    editor.execute(Command::SetArtboard { id: group, artboard: Some(b) }).unwrap();
    let json = serde_json::to_string(editor.document()).unwrap();
    let back: Document = serde_json::from_str(&json).unwrap();
    assert_eq!(back.layer(group).unwrap().artboard(), Some(b));
}
