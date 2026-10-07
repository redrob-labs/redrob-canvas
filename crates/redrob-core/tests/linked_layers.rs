//! Linked layers (Photoshop's Link Layers), batch 4 item M11.

use redrob_core::{
    Affine2D, Command, CoreError, Document, Editor, LayerId, LayerLocks, Pixel, Rect, SamplingMode,
};

fn dot(editor: &mut Editor, x: i32) {
    editor
        .execute(Command::SelectRectangle {
            rect: Rect {
                x,
                y: 0,
                width: 1,
                height: 1,
            },
            mode: Default::default(),
        })
        .unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(0, 0, 0, 255),
        })
        .unwrap();
    editor.execute(Command::ClearSelection).unwrap();
}

fn two_linked() -> (Editor, LayerId, LayerId) {
    let mut editor = Editor::new(Document::new(8, 2).unwrap()).unwrap();
    let a = editor.document().active_layer_id();
    dot(&mut editor, 0);
    let b = LayerId::new();
    editor
        .execute(Command::AddLayer {
            id: b,
            name: "B".into(),
            index: 1,
        })
        .unwrap();
    dot(&mut editor, 2);
    editor
        .execute(Command::LinkLayers {
            ids: vec![a, b],
            link: true,
        })
        .unwrap();
    (editor, a, b)
}

fn alpha(editor: &Editor, id: LayerId, x: usize) -> u8 {
    editor.document().layer(id).unwrap().pixels()[x * 4 + 3]
}

fn shift_right() -> Command {
    Command::TransformActive {
        transform: Affine2D {
            m11: 1.0,
            m12: 0.0,
            m21: 0.0,
            m22: 1.0,
            tx: 3.0,
            ty: 0.0,
        },
        sampling: SamplingMode::Nearest,
    }
}

#[test]
fn moving_one_linked_layer_moves_the_other() {
    let (mut editor, a, b) = two_linked();
    editor.execute(shift_right()).unwrap();
    assert_eq!(alpha(&editor, b, 5), 255, "the active layer moved");
    assert_eq!(alpha(&editor, a, 3), 255, "the linked layer moved with it");
    assert_eq!(
        editor.document().active_layer_id(),
        b,
        "the active layer is unchanged"
    );
    editor.undo().unwrap();
    assert_eq!(alpha(&editor, a, 0), 255, "one undo puts both back");
}

#[test]
fn unlinked_layers_move_alone() {
    let (mut editor, a, b) = two_linked();
    editor
        .execute(Command::LinkLayers {
            ids: vec![a, b],
            link: false,
        })
        .unwrap();
    editor.execute(shift_right()).unwrap();
    assert_eq!(alpha(&editor, a, 0), 255);
    assert!(editor.document().layer(a).unwrap().link().is_none());
}

#[test]
fn a_position_locked_partner_stops_the_move() {
    let (mut editor, a, _) = two_linked();
    editor
        .execute(Command::SetLayerLocks {
            id: a,
            locks: LayerLocks {
                position: true,
                ..LayerLocks::default()
            },
        })
        .unwrap();
    assert!(matches!(
        editor.execute(shift_right()),
        Err(CoreError::LayerLocked { .. })
    ));
}
