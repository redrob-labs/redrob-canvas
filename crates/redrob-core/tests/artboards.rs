//! Artboards, batch 4 item L8.

use redrob_core::{Artboard, Command, Document, Editor, LayerId, Pixel};

fn render(editor: &Editor) -> Vec<u8> {
    editor.render_snapshot().unwrap().pixels().to_vec()
}

fn board(x: i32, y: i32, w: u32, h: u32, background: Option<[u8; 3]>) -> Artboard {
    Artboard {
        x,
        y,
        width: w,
        height: h,
        background,
    }
}

/// A 16x16 document whose only layer is filled red and moved into an artboard group.
fn setup() -> (Editor, LayerId) {
    let mut editor = Editor::new(Document::new(16, 16).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    editor.execute(Command::SelectAll).unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(255, 0, 0, 255),
        })
        .unwrap();
    editor.execute(Command::ClearSelection).unwrap();
    let group = LayerId::new();
    editor
        .execute(Command::AddGroup {
            id: group,
            name: "Artboard 1".into(),
            parent: None,
            sibling_index: 1,
        })
        .unwrap();
    editor
        .execute(Command::MoveNode {
            id: layer,
            parent: Some(group),
            sibling_index: 0,
        })
        .unwrap();
    (editor, group)
}

fn at(pixels: &[u8], x: usize, y: usize) -> &[u8] {
    &pixels[(y * 16 + x) * 4..(y * 16 + x) * 4 + 4]
}

#[test]
fn children_are_clipped_to_the_artboard() {
    let (mut editor, group) = setup();
    editor
        .execute(Command::SetArtboard {
            id: group,
            artboard: Some(board(4, 4, 8, 8, None)),
        })
        .unwrap();
    let pixels = render(&editor);
    assert_eq!(at(&pixels, 5, 5), &[255, 0, 0, 255]);
    assert_eq!(at(&pixels, 1, 1)[3], 0, "outside the board is empty");
    assert_eq!(at(&pixels, 12, 12)[3], 0);
}

#[test]
fn the_background_shows_under_transparent_children() {
    let (mut editor, group) = setup();
    // Erase the red so only the board's white shows.
    let layer = editor
        .document()
        .nodes()
        .iter()
        .find(|n| n.parent_id() == Some(group))
        .unwrap()
        .id();
    editor
        .execute(Command::SetActiveLayer { id: layer })
        .unwrap();
    editor.execute(Command::SelectAll).unwrap();
    editor.execute(Command::Clear).unwrap();
    editor
        .execute(Command::SetArtboard {
            id: group,
            artboard: Some(board(0, 0, 8, 8, Some([255, 255, 255]))),
        })
        .unwrap();
    let pixels = render(&editor);
    assert_eq!(at(&pixels, 2, 2), &[255, 255, 255, 255]);
    assert_eq!(at(&pixels, 10, 10)[3], 0);
}

#[test]
fn only_groups_can_be_artboards_and_empty_boards_are_refused() {
    let (mut editor, group) = setup();
    let layer = editor
        .document()
        .nodes()
        .iter()
        .find(|n| n.parent_id() == Some(group))
        .unwrap()
        .id();
    assert!(
        editor
            .execute(Command::SetArtboard {
                id: layer,
                artboard: Some(board(0, 0, 4, 4, None))
            })
            .is_err()
    );
    assert!(
        editor
            .execute(Command::SetArtboard {
                id: group,
                artboard: Some(board(0, 0, 0, 4, None))
            })
            .is_err()
    );
    // Clearing turns it back into a plain group: no clipping.
    editor
        .execute(Command::SetArtboard {
            id: group,
            artboard: Some(board(0, 0, 4, 4, None)),
        })
        .unwrap();
    editor
        .execute(Command::SetArtboard {
            id: group,
            artboard: None,
        })
        .unwrap();
    assert_eq!(at(&render(&editor), 10, 10), &[255, 0, 0, 255]);
}

#[test]
fn the_artboard_survives_a_save_round_trip() {
    let (mut editor, group) = setup();
    let b = board(1, 2, 3, 4, Some([9, 8, 7]));
    editor
        .execute(Command::SetArtboard {
            id: group,
            artboard: Some(b),
        })
        .unwrap();
    let json = serde_json::to_string(editor.document()).unwrap();
    let back: Document = serde_json::from_str(&json).unwrap();
    assert_eq!(back.layer(group).unwrap().artboard(), Some(b));
}

// ---- U9: the artboard handle ----

/// The artboard at (4,4) 8x8 holding a 2x2 red dot at (5,5), and the dot layer's id.
fn dotted_board() -> (Editor, LayerId, LayerId) {
    let mut editor = Editor::new(Document::new(16, 16).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    editor
        .execute(Command::SelectRectangle {
            rect: redrob_core::Rect::new(5, 5, 2, 2),
            mode: Default::default(),
        })
        .unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(255, 0, 0, 255),
        })
        .unwrap();
    editor.execute(Command::ClearSelection).unwrap();
    let group = LayerId::new();
    editor
        .execute(Command::AddGroup {
            id: group,
            name: "Artboard 1".into(),
            parent: None,
            sibling_index: 1,
        })
        .unwrap();
    editor
        .execute(Command::MoveNode {
            id: layer,
            parent: Some(group),
            sibling_index: 0,
        })
        .unwrap();
    editor
        .execute(Command::SetArtboard {
            id: group,
            artboard: Some(board(4, 4, 8, 8, None)),
        })
        .unwrap();
    (editor, group, layer)
}

fn cel_at(editor: &Editor, layer: LayerId, x: usize, y: usize) -> [u8; 4] {
    let p = editor.document().layer(layer).unwrap().pixels();
    let o = (y * 16 + x) * 4;
    [p[o], p[o + 1], p[o + 2], p[o + 3]]
}

#[test]
fn moving_an_artboard_moves_its_contents_with_it() {
    let (mut editor, group, layer) = dotted_board();
    editor
        .execute(Command::MoveArtboard {
            id: group,
            dx: 3,
            dy: -2,
        })
        .unwrap();
    assert_eq!(
        editor.document().layer(group).unwrap().artboard(),
        Some(board(7, 2, 8, 8, None))
    );
    assert_eq!(cel_at(&editor, layer, 8, 3), [255, 0, 0, 255], "the dot moved too");
    assert_eq!(cel_at(&editor, layer, 5, 5), [0, 0, 0, 0], "and left its old place");
    // What shows is the same picture, shifted.
    let shown = render(&editor);
    assert_eq!(at(&shown, 8, 3), &[255, 0, 0, 255]);
}

#[test]
fn an_artboard_move_is_one_undo_step() {
    let (mut editor, group, layer) = dotted_board();
    editor
        .execute(Command::MoveArtboard {
            id: group,
            dx: 2,
            dy: 2,
        })
        .unwrap();
    editor.undo().unwrap();
    assert_eq!(
        editor.document().layer(group).unwrap().artboard(),
        Some(board(4, 4, 8, 8, None))
    );
    assert_eq!(cel_at(&editor, layer, 5, 5), [255, 0, 0, 255]);
}

#[test]
fn a_position_locked_member_refuses_the_whole_move() {
    let (mut editor, group, layer) = dotted_board();
    editor
        .execute(Command::SetLayerLocks {
            id: layer,
            locks: redrob_core::LayerLocks {
                position: true,
                ..Default::default()
            },
        })
        .unwrap();
    assert!(
        editor
            .execute(Command::MoveArtboard {
                id: group,
                dx: 1,
                dy: 0,
            })
            .is_err()
    );
    assert_eq!(
        editor.document().layer(group).unwrap().artboard(),
        Some(board(4, 4, 8, 8, None)),
        "nothing moved"
    );
}

#[test]
fn only_an_artboard_can_be_moved_this_way() {
    let (mut editor, _group, layer) = dotted_board();
    assert!(
        editor
            .execute(Command::MoveArtboard {
                id: layer,
                dx: 1,
                dy: 1,
            })
            .is_err()
    );
}
