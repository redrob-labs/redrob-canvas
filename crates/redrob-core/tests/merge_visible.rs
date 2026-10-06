//! Layer > Merge visible (Ctrl+Shift+E) and Layer > Flatten image, batch 4 item H3.

use redrob_core::{Command, CoreError, Document, Editor, LayerId, Pixel};

/// bottom (red, visible), middle (blue, hidden), top (half green, visible).
fn three_layers() -> (Editor, [LayerId; 3]) {
    let mut editor = Editor::new(Document::new(5, 5).unwrap()).unwrap();
    let bottom = editor.document().active_layer_id();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(200, 20, 20, 255),
        })
        .unwrap();
    let middle = LayerId::new();
    editor
        .execute(Command::AddLayer {
            id: middle,
            name: "Hidden".into(),
            index: 1,
        })
        .unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(20, 20, 200, 255),
        })
        .unwrap();
    editor
        .execute(Command::SetLayerVisibility {
            id: middle,
            visible: false,
        })
        .unwrap();
    let top = LayerId::new();
    editor
        .execute(Command::AddLayer {
            id: top,
            name: "Top".into(),
            index: 2,
        })
        .unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(20, 200, 20, 128),
        })
        .unwrap();
    (editor, [bottom, middle, top])
}

fn roots(editor: &Editor) -> Vec<LayerId> {
    editor
        .document()
        .nodes()
        .iter()
        .filter(|n| n.parent_id().is_none())
        .map(|n| n.id())
        .collect()
}

#[test]
fn merge_visible_looks_the_same_and_keeps_the_hidden_layer() {
    let (mut editor, [_, middle, _]) = three_layers();
    let before = editor.render_snapshot().unwrap().pixels().to_vec();
    let merged = LayerId::new();
    editor
        .execute(Command::MergeVisible { id: merged })
        .unwrap();
    assert_eq!(
        roots(&editor),
        vec![middle, merged],
        "the merged layer takes the topmost slot"
    );
    assert_eq!(editor.document().active_layer_id(), merged);
    assert_eq!(editor.render_snapshot().unwrap().pixels(), &before[..]);
}

#[test]
fn flatten_leaves_one_opaque_layer_over_the_background_colour() {
    let (mut editor, _) = three_layers();
    let flat = LayerId::new();
    editor
        .execute(Command::FlattenImage {
            id: flat,
            background: Pixel::rgba(255, 255, 255, 255),
        })
        .unwrap();
    assert_eq!(roots(&editor), vec![flat]);
    let pixels = editor.document().layer(flat).unwrap().pixels();
    assert!(
        pixels.chunks(4).all(|p| p[3] == 255),
        "no transparency left"
    );
}

#[test]
fn merge_visible_is_one_undo_step() {
    let (mut editor, ids) = three_layers();
    editor
        .execute(Command::MergeVisible { id: LayerId::new() })
        .unwrap();
    editor.undo().unwrap();
    assert_eq!(roots(&editor), ids.to_vec());
}

#[test]
fn a_hidden_child_goes_with_its_visible_group() {
    let (mut editor, [bottom, middle, top]) = three_layers();
    let group = LayerId::new();
    editor
        .execute(Command::AddGroup {
            id: group,
            name: "G".into(),
            parent: None,
            sibling_index: 3,
        })
        .unwrap();
    editor
        .execute(Command::MoveNode {
            id: middle,
            parent: Some(group),
            sibling_index: 0,
        })
        .unwrap();
    let merged = LayerId::new();
    editor
        .execute(Command::MergeVisible { id: merged })
        .unwrap();
    let doc = editor.document();
    for gone in [bottom, middle, top, group] {
        assert!(doc.layer(gone).is_none());
    }
    assert_eq!(roots(&editor), vec![merged]);
}

#[test]
fn nothing_visible_is_refused() {
    let (mut editor, ids) = three_layers();
    for id in ids {
        editor
            .execute(Command::SetLayerVisibility { id, visible: false })
            .unwrap();
    }
    assert!(matches!(
        editor.execute(Command::MergeVisible { id: LayerId::new() }),
        Err(CoreError::NothingVisibleToMerge)
    ));
    assert_eq!(roots(&editor), ids.to_vec());
}

#[test]
fn the_commands_round_trip_as_json() {
    for command in [
        Command::MergeVisible { id: LayerId::new() },
        Command::FlattenImage {
            id: LayerId::new(),
            background: Pixel::rgba(1, 2, 3, 255),
        },
    ] {
        let json = serde_json::to_string(&command).unwrap();
        assert!(
            json.contains("merge_visible") || json.contains("flatten_image"),
            "{json}"
        );
        assert_eq!(serde_json::from_str::<Command>(&json).unwrap(), command);
    }
}
