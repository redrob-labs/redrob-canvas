//! Layer > Duplicate layer (Photoshop's Ctrl+J), batch 4 item H1.

use redrob_core::{Command, Document, Editor, LayerId, Pixel};

fn painted() -> (Editor, LayerId) {
    let mut editor = Editor::new(Document::new(8, 8).unwrap()).unwrap();
    let base = editor.document().active_layer_id();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(10, 200, 30, 255),
        })
        .unwrap();
    (editor, base)
}

fn sibling_order(editor: &Editor, parent: Option<LayerId>) -> Vec<LayerId> {
    editor
        .document()
        .nodes()
        .iter()
        .filter(|n| n.parent_id() == parent)
        .map(|n| n.id())
        .collect()
}

#[test]
fn duplicate_goes_just_above_with_the_same_pixels_and_becomes_active() {
    let (mut editor, base) = painted();
    let top = LayerId::new();
    editor
        .execute(Command::AddLayer {
            id: top,
            name: "Top".into(),
            index: 1,
        })
        .unwrap();
    let copy = LayerId::new();
    editor
        .execute(Command::DuplicateLayer {
            source: base,
            id: copy,
        })
        .unwrap();
    assert_eq!(sibling_order(&editor, None), vec![base, copy, top]);
    let doc = editor.document();
    assert_eq!(doc.active_layer_id(), copy);
    assert_eq!(
        doc.layer(copy).unwrap().pixels(),
        doc.layer(base).unwrap().pixels()
    );
    assert!(doc.layer(copy).unwrap().name().ends_with(" copy"));
}

#[test]
fn duplicate_is_one_undo_step() {
    let (mut editor, base) = painted();
    let before = editor.document().nodes().len();
    editor.execute(Command::duplicate_layer(base)).unwrap();
    assert_eq!(editor.document().nodes().len(), before + 1);
    editor.undo().unwrap();
    assert_eq!(editor.document().nodes().len(), before);
    assert_eq!(editor.document().active_layer_id(), base);
}

#[test]
fn a_group_is_copied_with_its_children_in_order() {
    let (mut editor, base) = painted();
    let group = LayerId::new();
    editor
        .execute(Command::AddGroup {
            id: group,
            name: "G".into(),
            parent: None,
            sibling_index: 1,
        })
        .unwrap();
    editor
        .execute(Command::MoveNode {
            id: base,
            parent: Some(group),
            sibling_index: 0,
        })
        .unwrap();
    let second = LayerId::new();
    editor
        .execute(Command::AddLayer {
            id: second,
            name: "B".into(),
            index: 1,
        })
        .unwrap();
    editor
        .execute(Command::MoveNode {
            id: second,
            parent: Some(group),
            sibling_index: 1,
        })
        .unwrap();

    let copy = LayerId::new();
    editor
        .execute(Command::DuplicateLayer {
            source: group,
            id: copy,
        })
        .unwrap();
    let inner = sibling_order(&editor, Some(copy));
    assert_eq!(inner.len(), 2);
    assert!(
        !inner.contains(&base) && !inner.contains(&second),
        "children are copies, not moves"
    );
    let doc = editor.document();
    assert_eq!(
        doc.layer(inner[0]).unwrap().pixels(),
        doc.layer(base).unwrap().pixels()
    );
    assert_eq!(doc.layer(inner[1]).unwrap().name(), "B");
    // The original group is untouched.
    assert_eq!(sibling_order(&editor, Some(group)), vec![base, second]);
}

#[test]
fn replaying_the_same_command_makes_the_same_ids() {
    // Actions replay recorded commands; a group copy must not depend on fresh random ids.
    let run = || {
        let (mut editor, base) = painted();
        let group = LayerId::from_uuid(uuid_from(1));
        editor
            .execute(Command::AddGroup {
                id: group,
                name: "G".into(),
                parent: None,
                sibling_index: 1,
            })
            .unwrap();
        editor
            .execute(Command::MoveNode {
                id: base,
                parent: Some(group),
                sibling_index: 0,
            })
            .unwrap();
        let copy = LayerId::from_uuid(uuid_from(2));
        editor
            .execute(Command::DuplicateLayer {
                source: group,
                id: copy,
            })
            .unwrap();
        let first = sibling_order(&editor, Some(copy));
        editor.undo().unwrap();
        editor
            .execute(Command::DuplicateLayer {
                source: group,
                id: copy,
            })
            .unwrap();
        (first, sibling_order(&editor, Some(copy)))
    };
    let (first, replayed) = run();
    assert_eq!(first, replayed);
}

#[test]
fn an_unknown_source_is_refused_and_changes_nothing() {
    let (mut editor, _) = painted();
    let before = editor.document().nodes().len();
    assert!(
        editor
            .execute(Command::duplicate_layer(LayerId::new()))
            .is_err()
    );
    assert_eq!(editor.document().nodes().len(), before);
}

#[test]
fn the_command_round_trips_as_json() {
    let command = Command::duplicate_layer(LayerId::new());
    let json = serde_json::to_string(&command).unwrap();
    assert!(json.contains("\"type\":\"duplicate_layer\""), "{json}");
    assert_eq!(serde_json::from_str::<Command>(&json).unwrap(), command);
}

fn uuid_from(n: u128) -> uuid::Uuid {
    uuid::Uuid::from_u128(n)
}
