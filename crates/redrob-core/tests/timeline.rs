// SPDX-License-Identifier: GPL-3.0-or-later

use redrob_core::{
    Command, CoreError, Document, Editor, FrameId, Navigation, Pixel, timeline_frame_duration_ms,
};

fn frame_ids(editor: &Editor) -> Vec<FrameId> {
    editor
        .document()
        .timeline()
        .frames()
        .iter()
        .map(|frame| frame.id())
        .collect()
}

#[test]
fn frame_crud_is_ordered_typed_and_transactional() {
    let mut editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
    let one = FrameId::new(1);
    let two = FrameId::new(2);
    editor
        .execute(Command::AddFrame { id: one, index: 1 })
        .unwrap();
    editor
        .execute(Command::DuplicateFrame {
            source: FrameId::DEFAULT,
            id: two,
            index: 1,
        })
        .unwrap();
    assert_eq!(frame_ids(&editor), [FrameId::DEFAULT, two, one]);

    let generation = editor.generation();
    let before = editor.document().clone();
    assert!(matches!(
        editor.execute(Command::AddFrame { id: one, index: 0 }),
        Err(CoreError::DuplicateFrameId(id)) if id == one
    ));
    assert!(matches!(
        editor.execute(Command::MoveFrame {
            id: one,
            new_index: 3,
        }),
        Err(CoreError::FrameIndexOutOfBounds { index: 3, len: 3 })
    ));
    assert_eq!(editor.generation(), generation);
    assert_eq!(editor.document(), &before);

    editor.execute(Command::RemoveFrame { id: two }).unwrap();
    editor.execute(Command::RemoveFrame { id: one }).unwrap();
    assert!(matches!(
        editor.execute(Command::RemoveFrame {
            id: FrameId::DEFAULT,
        }),
        Err(CoreError::LastFrame)
    ));
}

#[test]
fn command_playback_stop_is_atomic_and_rejections_preserve_all_state() {
    let mut editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
    editor
        .navigate(Navigation::SetPlaying { playing: true })
        .unwrap();
    let before = editor.document().clone();
    let before_generation = editor.generation();
    let before_render = editor.render_snapshot().unwrap().pixels().to_vec();
    let before_can_undo = editor.can_undo();
    let before_can_redo = editor.can_redo();

    assert!(matches!(
        editor.execute(Command::AddFrame {
            id: FrameId::DEFAULT,
            index: 1,
        }),
        Err(CoreError::DuplicateFrameId(id)) if id == FrameId::DEFAULT
    ));
    assert_eq!(editor.document(), &before);
    assert_eq!(editor.generation(), before_generation);
    assert_eq!(
        editor.render_snapshot().unwrap().pixels(),
        before_render.as_slice()
    );
    assert_eq!(editor.can_undo(), before_can_undo);
    assert_eq!(editor.can_redo(), before_can_redo);
    assert!(editor.document().timeline().playback().playing);

    let viewed_frame = editor.document().current_frame_id();
    let changes = editor
        .execute(Command::Fill {
            color: Pixel::rgba(1, 2, 3, 255),
        })
        .unwrap();
    assert_eq!(editor.generation(), before_generation + 1);
    assert_eq!(editor.document().current_frame_id(), viewed_frame);
    assert!(!editor.document().timeline().playback().playing);
    assert!(changes.timeline_changed);
    assert!(changes.navigation_changed);
    assert!(editor.can_undo());
    assert!(!editor.can_redo());
    editor.undo().unwrap();
    assert!(matches!(editor.undo(), Err(CoreError::NothingToUndo)));
}

#[test]
fn unavailable_history_actions_preserve_active_playback_and_state() {
    let mut editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
    editor
        .navigate(Navigation::SetPlaying { playing: true })
        .unwrap();

    for unavailable in [Editor::undo, Editor::redo] {
        let before = editor.document().clone();
        let generation = editor.generation();
        let render = editor.render_snapshot().unwrap().pixels().to_vec();
        let can_undo = editor.can_undo();
        let can_redo = editor.can_redo();
        assert!(unavailable(&mut editor).is_err());
        assert_eq!(editor.document(), &before);
        assert_eq!(editor.generation(), generation);
        assert_eq!(
            editor.render_snapshot().unwrap().pixels(),
            render.as_slice()
        );
        assert_eq!(editor.can_undo(), can_undo);
        assert_eq!(editor.can_redo(), can_redo);
        assert!(editor.document().timeline().playback().playing);
    }
}

#[test]
fn fps_is_authoritative_for_existing_added_and_duplicated_frames() {
    let mut editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
    editor
        .execute(Command::SetTimelineFps { fps: 24.0 })
        .unwrap();
    let duration = timeline_frame_duration_ms(24.0).unwrap();
    assert_eq!(duration, 42);
    editor
        .execute(Command::AddFrame {
            id: FrameId::new(10),
            index: 1,
        })
        .unwrap();
    editor
        .execute(Command::DuplicateFrame {
            source: FrameId::DEFAULT,
            id: FrameId::new(11),
            index: 2,
        })
        .unwrap();
    assert!(
        editor
            .document()
            .timeline()
            .frames()
            .iter()
            .all(|frame| frame.duration_ms() == duration)
    );
    assert!(matches!(
        editor.execute(Command::SetTimelineFps { fps: 0.0 }),
        Err(CoreError::InvalidTimelineFps)
    ));
    assert!(matches!(
        editor.execute(Command::SetTimelineFps { fps: 240.01 }),
        Err(CoreError::InvalidTimelineFps)
    ));
}

#[test]
fn sparse_frames_render_transparent_and_materialize_only_on_mutation() {
    let mut editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    let blank = FrameId::new(5);
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(9, 8, 7, 255),
        })
        .unwrap();
    editor
        .execute(Command::AddFrame {
            id: blank,
            index: 1,
        })
        .unwrap();
    editor
        .navigate(Navigation::SetCurrentFrame { id: blank })
        .unwrap();
    assert_eq!(editor.render_snapshot().unwrap().pixels(), [0, 0, 0, 0]);
    assert!(
        !editor
            .document()
            .layer(layer)
            .unwrap()
            .has_raster_cel(blank)
    );

    editor
        .execute(Command::Fill {
            color: Pixel::rgba(1, 2, 3, 255),
        })
        .unwrap();
    assert!(
        editor
            .document()
            .layer(layer)
            .unwrap()
            .has_raster_cel(blank)
    );
    assert_eq!(
        editor
            .document()
            .layer(layer)
            .unwrap()
            .raster_pixels(FrameId::DEFAULT)
            .unwrap(),
        [9, 8, 7, 255]
    );
    assert_eq!(editor.render_snapshot().unwrap().pixels(), [1, 2, 3, 255]);
}

#[test]
fn duplicate_uses_cow_and_edit_detaches_only_the_destination() {
    let mut editor = Editor::new(Document::new(2, 1).unwrap()).unwrap();
    let layer = editor.document().active_layer_id();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(4, 5, 6, 255),
        })
        .unwrap();
    let duplicate = FrameId::new(7);
    editor
        .execute(Command::DuplicateFrame {
            source: FrameId::DEFAULT,
            id: duplicate,
            index: 1,
        })
        .unwrap();
    let cels = editor
        .document()
        .layer(layer)
        .unwrap()
        .raster_cels()
        .unwrap();
    assert!(cels[0].storage().shares_storage_with(cels[1].storage()));

    editor
        .navigate(Navigation::SetCurrentFrame { id: duplicate })
        .unwrap();
    editor.execute(Command::Clear).unwrap();
    let cels = editor
        .document()
        .layer(layer)
        .unwrap()
        .raster_cels()
        .unwrap();
    assert!(!cels[0].storage().shares_storage_with(cels[1].storage()));
    assert_eq!(
        editor
            .document()
            .layer(layer)
            .unwrap()
            .raster_pixels(FrameId::DEFAULT)
            .unwrap(),
        [4, 5, 6, 255, 4, 5, 6, 255]
    );
}

#[test]
fn range_order_move_and_remove_current_fallback_are_deterministic() {
    let mut editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
    let one = FrameId::new(1);
    let two = FrameId::new(2);
    for (id, index) in [(one, 1), (two, 2)] {
        editor.execute(Command::AddFrame { id, index }).unwrap();
    }
    editor
        .execute(Command::SetPlaybackRange {
            start: one,
            end: two,
        })
        .unwrap();
    assert!(matches!(
        editor.execute(Command::MoveFrame {
            id: two,
            new_index: 0,
        }),
        Err(CoreError::PlaybackRangeOrder)
    ));
    editor
        .navigate(Navigation::SetCurrentFrame { id: one })
        .unwrap();
    let changes = editor.execute(Command::RemoveFrame { id: one }).unwrap();
    let timeline = editor.document().timeline();
    assert_eq!(timeline.current_frame(), two);
    assert_eq!(timeline.playback().range_start, two);
    assert_eq!(timeline.playback().range_end, two);
    assert!(changes.timeline_changed);
    assert!(changes.navigation_changed);
    assert!(changes.canvas_changed);
}

#[test]
fn navigation_advances_generation_without_touching_history_and_playback_stops_at_end() {
    let mut editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
    let one = FrameId::new(1);
    editor
        .execute(Command::AddFrame { id: one, index: 1 })
        .unwrap();
    editor
        .execute(Command::SetPlaybackRange {
            start: FrameId::DEFAULT,
            end: one,
        })
        .unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(1, 1, 1, 255),
        })
        .unwrap();
    editor.undo().unwrap();
    assert!(editor.can_undo());
    assert!(editor.can_redo());
    let before_generation = editor.generation();
    let navigation = editor
        .navigate(Navigation::SetCurrentFrame { id: one })
        .unwrap();
    assert!(navigation.timeline_changed);
    assert!(navigation.navigation_changed);
    assert!(navigation.canvas_changed);
    assert!(editor.generation() > before_generation);
    assert!(editor.can_undo());
    assert!(editor.can_redo());

    editor
        .navigate(Navigation::SetPlaying { playing: true })
        .unwrap();
    assert!(editor.document().timeline().playback().playing);
    editor.navigate(Navigation::AdvancePlayback).unwrap();
    assert!(!editor.document().timeline().playback().playing);
    assert_eq!(editor.document().timeline().current_frame(), one);
    assert!(editor.can_redo());
}

#[test]
fn loop_end_wraps_to_range_start_and_keeps_playing_history_neutral() {
    let mut editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
    let end = FrameId::new(1);
    editor
        .execute(Command::AddFrame { id: end, index: 1 })
        .unwrap();
    editor
        .execute(Command::SetPlaybackRange {
            start: FrameId::DEFAULT,
            end,
        })
        .unwrap();
    editor
        .execute(Command::SetLooping { looping: true })
        .unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel::rgba(1, 1, 1, 255),
        })
        .unwrap();
    editor.undo().unwrap();
    editor
        .navigate(Navigation::SetCurrentFrame { id: end })
        .unwrap();
    editor
        .navigate(Navigation::SetPlaying { playing: true })
        .unwrap();

    let generation = editor.generation();
    let can_undo = editor.can_undo();
    let can_redo = editor.can_redo();
    let changes = editor.navigate(Navigation::AdvancePlayback).unwrap();

    assert_eq!(
        editor.document().timeline().current_frame(),
        FrameId::DEFAULT
    );
    assert!(editor.document().timeline().playback().playing);
    assert_eq!(editor.generation(), generation + 1);
    assert_eq!(editor.can_undo(), can_undo);
    assert_eq!(editor.can_redo(), can_redo);
    assert!(changes.canvas_changed);
    assert!(changes.timeline_changed);
    assert!(changes.navigation_changed);
}

#[test]
fn undo_redo_preserve_a_live_view_frame_and_force_playback_off() {
    let mut editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
    let one = FrameId::new(1);
    editor
        .execute(Command::AddFrame { id: one, index: 1 })
        .unwrap();
    editor
        .execute(Command::SetPlaybackRange {
            start: one,
            end: one,
        })
        .unwrap();
    editor
        .navigate(Navigation::SetCurrentFrame { id: one })
        .unwrap();
    editor
        .execute(Command::SetLooping { looping: true })
        .unwrap();
    editor
        .navigate(Navigation::SetPlaying { playing: true })
        .unwrap();
    let before_undo = editor.generation();
    let undo = editor.undo().unwrap();
    assert_eq!(editor.generation(), before_undo + 1);
    assert_eq!(editor.document().timeline().current_frame(), one);
    assert!(!editor.document().timeline().playback().playing);
    assert!(undo.timeline_changed);
    assert!(undo.navigation_changed);
    assert!(undo.canvas_changed);
    editor
        .navigate(Navigation::SetPlaying { playing: true })
        .unwrap();
    let before_redo = editor.generation();
    let redo = editor.redo().unwrap();
    assert_eq!(editor.generation(), before_redo + 1);
    assert_eq!(editor.document().timeline().current_frame(), one);
    assert!(!editor.document().timeline().playback().playing);
    assert!(redo.timeline_changed);
    assert!(redo.navigation_changed);
    assert!(redo.canvas_changed);
}

#[test]
fn starting_playback_outside_range_reports_the_implicit_canvas_switch() {
    let mut editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
    let one = FrameId::new(1);
    editor
        .execute(Command::AddFrame { id: one, index: 1 })
        .unwrap();
    editor
        .navigate(Navigation::SetCurrentFrame { id: one })
        .unwrap();
    let can_undo = editor.can_undo();
    let can_redo = editor.can_redo();
    let changes = editor
        .navigate(Navigation::SetPlaying { playing: true })
        .unwrap();
    assert!(changes.canvas_changed);
    assert!(changes.navigation_changed);
    assert_eq!(
        editor.document().timeline().current_frame(),
        FrameId::DEFAULT
    );
    assert_eq!(editor.can_undo(), can_undo);
    assert_eq!(editor.can_redo(), can_redo);
}

#[test]
fn duplicate_enforces_logical_storage_budget_before_cow_clone() {
    let mut editor = Editor::new(Document::new(4096, 4096).unwrap()).unwrap();
    for value in 1..15 {
        editor
            .execute(Command::DuplicateFrame {
                source: FrameId::DEFAULT,
                id: FrameId::new(value),
                index: value as usize,
            })
            .unwrap();
    }
    let before = editor.document().clone();
    let generation = editor.generation();
    assert!(matches!(
        editor.execute(Command::DuplicateFrame {
            source: FrameId::DEFAULT,
            id: FrameId::new(15),
            index: 15,
        }),
        Err(CoreError::DocumentLimitExceeded("stored raster bytes"))
    ));
    assert_eq!(editor.generation(), generation);
    assert_eq!(editor.document(), &before);
}

#[test]
fn rejected_timeline_relationships_preserve_document_and_generation() {
    let mut editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
    let one = FrameId::new(1);
    editor
        .execute(Command::AddFrame { id: one, index: 1 })
        .unwrap();
    let cases = [
        Command::DuplicateFrame {
            source: FrameId::new(99),
            id: FrameId::new(2),
            index: 2,
        },
        Command::DuplicateFrame {
            source: FrameId::DEFAULT,
            id: one,
            index: 2,
        },
        Command::DuplicateFrame {
            source: FrameId::DEFAULT,
            id: FrameId::new(2),
            index: 3,
        },
        Command::RemoveFrame {
            id: FrameId::new(99),
        },
        Command::MoveFrame {
            id: FrameId::new(99),
            new_index: 0,
        },
        Command::SetPlaybackRange {
            start: one,
            end: FrameId::DEFAULT,
        },
        Command::SetPlaybackRange {
            start: FrameId::new(99),
            end: one,
        },
    ];
    for command in cases {
        let before = editor.document().clone();
        let generation = editor.generation();
        assert!(editor.execute(command).is_err());
        assert_eq!(editor.generation(), generation);
        assert_eq!(editor.document(), &before);
    }
}

#[test]
fn removing_last_position_current_uses_previous_frame_and_clamps_range_end() {
    let mut editor = Editor::new(Document::new(1, 1).unwrap()).unwrap();
    let one = FrameId::new(1);
    let two = FrameId::new(2);
    editor
        .execute(Command::AddFrame { id: one, index: 1 })
        .unwrap();
    editor
        .execute(Command::AddFrame { id: two, index: 2 })
        .unwrap();
    editor
        .execute(Command::SetPlaybackRange {
            start: FrameId::DEFAULT,
            end: two,
        })
        .unwrap();
    editor
        .navigate(Navigation::SetCurrentFrame { id: two })
        .unwrap();
    editor.execute(Command::RemoveFrame { id: two }).unwrap();
    assert_eq!(editor.document().timeline().current_frame(), one);
    assert_eq!(editor.document().timeline().playback().range_end, one);
}
