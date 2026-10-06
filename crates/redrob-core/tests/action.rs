//! P14. Actions: recorded commands, saved as JSON, played back as one undo step.

use redrob_core::{
    ACTION_FORMAT, Action, Command, CoreError, Document, Editor, Filter, LayerId,
    MAX_ACTION_COMMANDS,
};

#[path = "common/canvas.rs"]
mod canvas;

fn steps() -> Vec<Command> {
    vec![
        Command::ApplyFilter {
            filter: Filter::Invert,
        },
        Command::AddLayer {
            id: LayerId::new(),
            name: "From the action".into(),
            index: 1,
        },
    ]
}

fn editor() -> Editor {
    canvas::editor(
        2,
        1,
        &[
            redrob_core::Pixel::rgba(10, 20, 30, 255),
            redrob_core::Pixel::rgba(200, 100, 50, 255),
        ],
    )
}

#[test]
fn playing_an_action_matches_doing_the_steps_by_hand() {
    let mut by_hand = editor();
    for step in steps() {
        by_hand.execute(step).unwrap();
    }
    let mut played = editor();
    played
        .play_action(&Action::new("Invert and add", steps()))
        .unwrap();

    assert_eq!(
        played.render_snapshot().unwrap().pixels(),
        by_hand.render_snapshot().unwrap().pixels()
    );
    assert_eq!(
        played.document().layers().len(),
        by_hand.document().layers().len()
    );
}

#[test]
fn an_action_is_one_undo_step() {
    let mut editor = editor();
    let before = editor.render_snapshot().unwrap().pixels().to_vec();
    let depth = editor.undo_depth();
    editor
        .play_action(&Action::new("Two steps", steps()))
        .unwrap();
    assert_eq!(
        editor.undo_depth(),
        depth + 1,
        "two steps must undo together"
    );
    editor.undo().unwrap();
    assert_eq!(
        editor.render_snapshot().unwrap().pixels(),
        before.as_slice()
    );
    assert_eq!(editor.document().layers().len(), 1);
}

#[test]
fn a_failing_step_rolls_the_whole_action_back() {
    let mut editor = editor();
    let before = editor.render_snapshot().unwrap().pixels().to_vec();
    let depth = editor.undo_depth();
    let mut bad = steps();
    // Step 3: a layer that does not exist.
    bad.push(Command::SetLayerOpacity {
        id: LayerId::new(),
        opacity: 0.5,
    });
    let result = editor.play_action(&Action::new("Breaks at three", bad));
    let Err(CoreError::InvalidAction(message)) = result else {
        panic!("expected InvalidAction, got {result:?}")
    };
    assert!(message.contains("step 3"), "{message}");
    assert_eq!(
        editor.render_snapshot().unwrap().pixels(),
        before.as_slice(),
        "invert was kept"
    );
    assert_eq!(
        editor.document().layers().len(),
        1,
        "the added layer was kept"
    );
    assert_eq!(
        editor.undo_depth(),
        depth,
        "a failed action must leave no history"
    );
    assert!(!editor.is_group_active());
}

#[test]
fn the_file_round_trips_and_its_envelope_is_checked() {
    let action = Action::new("Round trip", steps());
    let bytes = action.to_json().unwrap();
    assert_eq!(Action::from_json(&bytes).unwrap(), action);
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["format"], ACTION_FORMAT);
    assert_eq!(value["version"], 1);

    let refuse = |value: serde_json::Value| {
        Action::from_json(&serde_json::to_vec(&value).unwrap()).expect_err(&value.to_string());
    };
    let commands = value["commands"].clone();
    refuse(
        serde_json::json!({ "format": "photoshop", "version": 1, "name": "x", "commands": commands }),
    );
    refuse(
        serde_json::json!({ "format": ACTION_FORMAT, "version": 2, "name": "x", "commands": commands }),
    );
    refuse(
        serde_json::json!({ "format": ACTION_FORMAT, "version": 1, "name": " ", "commands": commands }),
    );
    refuse(
        serde_json::json!({ "format": ACTION_FORMAT, "version": 1, "name": "x", "commands": [] }),
    );
    refuse(serde_json::json!({
        "format": ACTION_FORMAT, "version": 1, "name": "x", "commands": commands, "script": "rm -rf"
    }));
    assert!(Action::from_json(b"not json").is_err());
}

#[test]
fn caret_moves_and_oversized_actions_are_refused() {
    let caret: Command = serde_json::from_value(serde_json::json!({
        "type": "set_text_caret", "id": LayerId::new(), "insert": 0, "anchor": 0
    }))
    .unwrap();
    assert!(Action::new("Caret", vec![caret]).validate().is_err());
    let many = vec![
        Command::ApplyFilter {
            filter: Filter::Invert,
        };
        MAX_ACTION_COMMANDS + 1
    ];
    assert!(Action::new("Too many", many).validate().is_err());
}

#[test]
fn an_action_cannot_start_inside_another_group() {
    let mut editor = Editor::new(Document::new(4, 4).unwrap()).unwrap();
    editor.begin_group("outer").unwrap();
    assert!(editor.play_action(&Action::new("Nested", steps())).is_err());
    editor.cancel_group().unwrap();
}
