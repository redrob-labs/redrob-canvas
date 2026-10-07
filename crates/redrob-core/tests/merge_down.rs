//! Layer > Merge down (Photoshop's Ctrl+E), batch 4 item H2.

use redrob_core::{BlendMode, Command, CoreError, Document, Editor, LayerId, Pixel};

/// A green bottom layer and a half-transparent multiply top layer over it.
fn two_layers() -> (Editor, LayerId, LayerId) {
    let mut editor = Editor::new(Document::new(6, 6).unwrap()).unwrap();
    let bottom = editor.document().active_layer_id();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(30, 200, 60, 255),
        })
        .unwrap();
    let top = LayerId::new();
    editor
        .execute(Command::AddLayer {
            id: top,
            name: "Top".into(),
            index: 1,
        })
        .unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(220, 40, 90, 255),
        })
        .unwrap();
    editor
        .execute(Command::SetLayerOpacity {
            id: top,
            opacity: 0.5,
        })
        .unwrap();
    editor
        .execute(Command::SetLayerBlendMode {
            id: top,
            mode: BlendMode::Multiply,
        })
        .unwrap();
    (editor, bottom, top)
}

#[test]
fn merging_down_looks_the_same_and_leaves_one_layer() {
    let (mut editor, bottom, top) = two_layers();
    let before = editor.render_snapshot().unwrap().pixels().to_vec();
    editor.execute(Command::MergeDown { id: top }).unwrap();
    let doc = editor.document();
    assert!(doc.layer(top).is_none());
    assert_eq!(doc.active_layer_id(), bottom);
    assert_eq!(editor.render_snapshot().unwrap().pixels(), &before[..]);
}

#[test]
fn merge_down_is_one_undo_step() {
    let (mut editor, bottom, top) = two_layers();
    let pixels = editor.document().layer(bottom).unwrap().pixels().to_vec();
    editor.execute(Command::MergeDown { id: top }).unwrap();
    editor.undo().unwrap();
    let doc = editor.document();
    assert!(doc.layer(top).is_some());
    assert_eq!(doc.layer(bottom).unwrap().pixels(), &pixels[..]);
}

#[test]
fn the_bottom_layer_cannot_merge_down() {
    let (mut editor, bottom, _) = two_layers();
    assert!(matches!(
        editor.execute(Command::MergeDown { id: bottom }),
        Err(CoreError::NothingBelowToMerge(_))
    ));
}

#[test]
fn a_hidden_layer_is_refused_not_thrown_away() {
    let (mut editor, _, top) = two_layers();
    editor
        .execute(Command::SetLayerVisibility {
            id: top,
            visible: false,
        })
        .unwrap();
    assert!(matches!(
        editor.execute(Command::MergeDown { id: top }),
        Err(CoreError::MergeHiddenLayer(_))
    ));
    assert!(editor.document().layer(top).is_some());
}

#[test]
fn a_group_below_is_not_a_merge_target() {
    let (mut editor, bottom, top) = two_layers();
    let group = LayerId::new();
    editor
        .execute(Command::AddGroup {
            id: group,
            name: "G".into(),
            parent: None,
            sibling_index: 1,
        })
        .unwrap();
    let _ = bottom;
    // Order is now bottom, group, top: the node below `top` is the group.
    assert!(matches!(
        editor.execute(Command::MergeDown { id: top }),
        Err(CoreError::NothingBelowToMerge(_))
    ));
}

#[test]
fn the_command_round_trips_as_json() {
    let command = Command::MergeDown { id: LayerId::new() };
    let json = serde_json::to_string(&command).unwrap();
    assert!(json.contains("\"type\":\"merge_down\""), "{json}");
    assert_eq!(serde_json::from_str::<Command>(&json).unwrap(), command);
}
