// SPDX-License-Identifier: GPL-3.0-or-later

//! Guides, sample points and snapping (L.1).
//!
//! Re-derived from `app/core/gimpimage-snap.c`, `app/core/gimpguide.c` and
//! `app/core/gimpsamplepoint.c` (GPL-3.0-or-later), plus
//! `libs/ui/canvas/kis_guides_config.cpp` for the lock, which the first upstream does not have.
//! Both are pinned in `docs/upstream-sources.toml`.

use redrob_core::{
    Command, CoreError, Document, Editor, GuideId, GuideOrientation, GuideSettings, GuideStyle,
    LayerId, Pixel, SamplePointId, snap_x, snap_y,
};

fn canvas() -> Editor {
    Editor::new(Document::new(100, 100).unwrap()).unwrap()
}

fn add_guide(
    editor: &mut Editor,
    orientation: GuideOrientation,
    position: i32,
    style: GuideStyle,
) -> GuideId {
    let id = GuideId::new_v4();
    editor
        .execute(Command::AddGuide {
            id,
            orientation,
            position,
            style,
        })
        .unwrap();
    id
}

fn vertical(editor: &mut Editor, position: i32) -> GuideId {
    add_guide(
        editor,
        GuideOrientation::Vertical,
        position,
        GuideStyle::Normal,
    )
}

/// `dist < min(epsilon, mindist)` is STRICT, so the threshold is exclusive.
///
/// The value a `<=` implementation would give is named here: at `epsilon` exactly, both 60 and 40
/// would be pulled to 50. They are not. One pixel of boundary either side of every snap in the
/// product rides on this comparison.
#[test]
fn a_guide_snaps_strictly_inside_epsilon_and_not_at_it() {
    let mut editor = canvas();
    vertical(&mut editor, 50);
    let guides = editor.document().guides();
    let settings = GuideSettings::default();

    // Nine away: inside.
    assert_eq!(snap_x(guides, 100, 59.0, 10.0, &settings), (50.0, true));
    // Ten away on either side: the threshold itself, which does NOT snap.
    assert_eq!(snap_x(guides, 100, 60.0, 10.0, &settings), (60.0, false));
    assert_eq!(snap_x(guides, 100, 40.0, 10.0, &settings), (40.0, false));
}

/// The `min(epsilon, mindist)` half: a later candidate at the SAME distance does not replace the
/// current best, so ties go to whichever guide comes first in the list.
///
/// Asserted as one claim about the pair of orderings rather than two claims about two documents.
/// `x = 50` sits exactly between guides at 40 and 60, so the only thing that can decide the result
/// is the order, and an implementation comparing `<=` against `mindist` would answer 60 both times.
#[test]
fn the_first_guide_at_the_minimum_distance_wins_the_tie() {
    let mut forwards = canvas();
    vertical(&mut forwards, 40);
    vertical(&mut forwards, 60);

    let mut backwards = canvas();
    vertical(&mut backwards, 60);
    vertical(&mut backwards, 40);

    let settings = GuideSettings::default();
    assert_eq!(
        snap_x(forwards.document().guides(), 100, 50.0, 11.0, &settings),
        (40.0, true)
    );
    assert_eq!(
        snap_x(backwards.document().guides(), 100, 50.0, 11.0, &settings),
        (60.0, true)
    );
}

/// `gimp_guide_is_custom` is `style != NORMAL`, and the snap loop `continue`s over those.
///
/// One assertion about the DIFFERENCE between the two styles at one coordinate: the guide is at the
/// same place and the point is the same point, so style is the only variable. An implementation
/// that snapped to the whole list would look correct until a symmetry mode drew its construction
/// lines, at which point tools would start sticking to them.
#[test]
fn a_custom_guide_is_drawn_but_is_not_a_snapping_target() {
    let mut mode_drawn = canvas();
    add_guide(
        &mut mode_drawn,
        GuideOrientation::Vertical,
        50,
        GuideStyle::Mirror,
    );

    let mut user_placed = canvas();
    vertical(&mut user_placed, 50);

    let settings = GuideSettings::default();
    // Both documents hold exactly one guide at 50 — the custom one is stored and drawn.
    assert_eq!(mode_drawn.document().guides().len(), 1);
    assert!(mode_drawn.document().guides()[0].is_custom());
    assert!(!user_placed.document().guides()[0].is_custom());

    assert_eq!(
        snap_x(mode_drawn.document().guides(), 100, 52.0, 10.0, &settings),
        (52.0, false)
    );
    assert_eq!(
        snap_x(user_placed.document().guides(), 100, 52.0, 10.0, &settings),
        (50.0, true)
    );
}

/// The out-of-canvas gate is `coordinate < -epsilon || coordinate >= extent + epsilon`, and those
/// two bounds are NOT mirror images: `-epsilon` passes, `extent + epsilon` is rejected.
///
/// Measured through a guide that is itself outside the canvas, which is legal — upstream's add and
/// move paths validate nothing. A guide 5 past the right edge cannot be reached from `x = 110`,
/// while a guide 5 before the left edge IS reached from `x = -10`. Same distance, opposite
/// verdicts, decided entirely by which side of the canvas the coordinate is on. A symmetric range
/// check would answer the same on both.
#[test]
fn the_out_of_canvas_gate_is_asymmetric() {
    let settings = GuideSettings::default();

    let mut past_the_right = canvas();
    vertical(&mut past_the_right, 105);
    assert_eq!(
        snap_x(
            past_the_right.document().guides(),
            100,
            110.0,
            10.0,
            &settings
        ),
        (110.0, false)
    );

    let mut before_the_left = canvas();
    vertical(&mut before_the_left, -5);
    assert_eq!(
        snap_x(
            before_the_left.document().guides(),
            100,
            -10.0,
            10.0,
            &settings
        ),
        (-5.0, true)
    );
}

/// `snap-to-canvas` defaults to `FALSE` upstream while `snap-to-guides` defaults to `TRUE`.
///
/// Pinned because a uniform default is the tidier-looking choice: a document that snapped to its
/// own edges out of the box would fight every freehand placement near a border.
#[test]
fn the_canvas_edges_snap_only_when_asked() {
    let editor = canvas();
    let guides = editor.document().guides();

    let defaults = GuideSettings::default();
    assert!(!defaults.snap_to_canvas);
    assert!(defaults.snap_to_guides);
    assert_eq!(snap_x(guides, 100, 109.0, 10.0, &defaults), (109.0, false));

    let asked = GuideSettings {
        snap_to_canvas: true,
        ..GuideSettings::default()
    };
    // Both edges participate: the far one at the extent and the origin at 0.
    assert_eq!(snap_x(guides, 100, 109.0, 10.0, &asked), (100.0, true));
    assert_eq!(snap_x(guides, 100, -9.0, 10.0, &asked), (0.0, true));
}

/// Each axis keeps its own `mindist`, so a coordinate can snap on one and stay free on the other.
///
/// A true nearest-POINT search would be the plausible alternative and is wrong: dragging along a
/// vertical guide would have the `y` dragged to the guide's own position too.
#[test]
fn each_axis_snaps_independently() {
    let mut editor = canvas();
    vertical(&mut editor, 50);
    let guides = editor.document().guides();
    let settings = GuideSettings::default();

    assert_eq!(
        editor.document().snap_point(52.0, 52.0, 10.0, 10.0),
        (50.0, 52.0, true)
    );
    // The vertical guide is not a candidate for a `y` at all, whatever its position.
    assert_eq!(snap_y(guides, 100, 52.0, 10.0, &settings), (52.0, false));
}

/// Lock and snap are independent, which is the one place the two upstreams disagree.
///
/// One assertion about the DIFFERENCE: under a single locked document, every mutation is refused
/// and the snap is unchanged. An implementation that folded the lock into the snap — the obvious
/// reading of "locked" — would pass a test that only checked the refusals.
#[test]
fn locking_the_guides_blocks_mutation_and_leaves_snapping_alone() {
    let mut editor = canvas();
    let id = vertical(&mut editor, 50);
    let settings = GuideSettings::default();

    let unlocked = snap_x(editor.document().guides(), 100, 52.0, 10.0, &settings);
    assert_eq!(unlocked, (50.0, true));

    editor
        .execute(Command::SetGuideSettings {
            settings: GuideSettings {
                lock_guides: true,
                ..GuideSettings::default()
            },
        })
        .unwrap();

    assert!(matches!(
        editor.execute(Command::MoveGuide { id, position: 70 }),
        Err(CoreError::GuidesLocked)
    ));
    assert!(matches!(
        editor.execute(Command::RemoveGuide { id }),
        Err(CoreError::GuidesLocked)
    ));
    assert!(matches!(
        editor.execute(Command::AddGuide {
            id: GuideId::new_v4(),
            orientation: GuideOrientation::Horizontal,
            position: 10,
            style: GuideStyle::Normal,
        }),
        Err(CoreError::GuidesLocked)
    ));

    // The guide is still there, still where it was, and still a snapping target.
    assert_eq!(editor.document().guides().len(), 1);
    assert_eq!(editor.document().guides()[0].position(), 50);
    assert_eq!(
        editor.document().snap_point(52.0, 0.0, 10.0, 10.0),
        (50.0, 0.0, true)
    );
}

/// A sample point reads the COMPOSITE, which is the only reading that is useful while editing.
///
/// Red base, 50% blue on top. The composite is (128, 0, 128) — neither the top layer's own
/// (0, 0, 255) nor the base's (255, 0, 0), so reading either layer directly is distinguishable
/// here. A point off the canvas reads `None`; upstream validates nothing on add, so a point can
/// survive a crop and end up outside the image.
#[test]
fn a_sample_point_reads_the_composite_not_a_layer() {
    let mut editor = Editor::new(Document::new(2, 2).unwrap()).unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            },
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
    editor.execute(Command::SetActiveLayer { id: top }).unwrap();
    editor
        .execute(Command::Fill {
            color: Pixel {
                r: 0,
                g: 0,
                b: 255,
                a: 255,
            },
        })
        .unwrap();
    editor
        .execute(Command::SetLayerOpacity {
            id: top,
            opacity: 0.5,
        })
        .unwrap();

    let inside = SamplePointId::new_v4();
    let outside = SamplePointId::new_v4();
    editor
        .execute(Command::AddSamplePoint {
            id: inside,
            x: 0,
            y: 0,
        })
        .unwrap();
    editor
        .execute(Command::AddSamplePoint {
            id: outside,
            x: 9,
            y: 9,
        })
        .unwrap();

    let colors = editor.sample_point_colors().unwrap();
    assert_eq!(
        colors,
        vec![
            (
                inside,
                Some(Pixel {
                    r: 128,
                    g: 0,
                    b: 128,
                    a: 255
                })
            ),
            (outside, None),
        ]
    );
}

/// Moving and removing both kinds of item, and the errors for an id that is not there.
#[test]
fn guides_and_sample_points_move_and_remove_by_id() {
    let mut editor = canvas();
    let guide = vertical(&mut editor, 50);
    let point = SamplePointId::new_v4();
    editor
        .execute(Command::AddSamplePoint {
            id: point,
            x: 3,
            y: 4,
        })
        .unwrap();

    editor
        .execute(Command::MoveGuide {
            id: guide,
            position: 70,
        })
        .unwrap();
    editor
        .execute(Command::MoveSamplePoint {
            id: point,
            x: 7,
            y: 8,
        })
        .unwrap();
    assert_eq!(editor.document().guides()[0].position(), 70);
    assert_eq!(
        (
            editor.document().sample_points()[0].x(),
            editor.document().sample_points()[0].y()
        ),
        (7, 8)
    );

    let absent_guide = GuideId::new_v4();
    let absent_point = SamplePointId::new_v4();
    assert!(matches!(
        editor.execute(Command::MoveGuide {
            id: absent_guide,
            position: 1
        }),
        Err(CoreError::GuideNotFound(_))
    ));
    assert!(matches!(
        editor.execute(Command::RemoveSamplePoint { id: absent_point }),
        Err(CoreError::SamplePointNotFound(_))
    ));
    assert!(matches!(
        editor.execute(Command::AddGuide {
            id: guide,
            orientation: GuideOrientation::Vertical,
            position: 1,
            style: GuideStyle::Normal,
        }),
        Err(CoreError::DuplicateGuideId(_))
    ));

    editor.execute(Command::RemoveGuide { id: guide }).unwrap();
    editor
        .execute(Command::RemoveSamplePoint { id: point })
        .unwrap();
    assert!(editor.document().guides().is_empty());
    assert!(editor.document().sample_points().is_empty());
}

/// A project written before L.1 has none of these three fields, and loads as exactly that.
///
/// Measured rather than asserted: the fields are stripped from a real serialised document and the
/// result is read back, so `#[serde(default)]` is doing the work and not a hand-written fixture
/// that agrees with the code.
#[test]
fn a_document_written_before_guides_existed_loads_with_none() {
    let mut editor = canvas();
    vertical(&mut editor, 50);
    editor
        .execute(Command::AddSamplePoint {
            id: SamplePointId::new_v4(),
            x: 1,
            y: 2,
        })
        .unwrap();

    let mut value = serde_json::to_value(editor.document()).unwrap();
    let object = value.as_object_mut().unwrap();
    // Present before stripping — otherwise this test would pass on a typo in the field names.
    assert!(object.remove("guides").is_some());
    assert!(object.remove("sample_points").is_some());
    assert!(object.remove("guide_settings").is_some());

    let legacy: Document = serde_json::from_value(value).unwrap();
    assert!(legacy.guides().is_empty());
    assert!(legacy.sample_points().is_empty());
    assert_eq!(legacy.guide_settings(), GuideSettings::default());
}

/// Every new command round-trips through its wire form, the house check for a command addition.
#[test]
fn the_new_commands_round_trip_through_json() {
    let commands = vec![
        Command::AddGuide {
            id: GuideId::new_v4(),
            orientation: GuideOrientation::Horizontal,
            position: -7,
            style: GuideStyle::Mandala,
        },
        Command::MoveGuide {
            id: GuideId::new_v4(),
            position: 12,
        },
        Command::RemoveGuide {
            id: GuideId::new_v4(),
        },
        Command::AddSamplePoint {
            id: SamplePointId::new_v4(),
            x: 1,
            y: 2,
        },
        Command::MoveSamplePoint {
            id: SamplePointId::new_v4(),
            x: 3,
            y: 4,
        },
        Command::RemoveSamplePoint {
            id: SamplePointId::new_v4(),
        },
        Command::SetGuideSettings {
            settings: GuideSettings {
                show_guides: false,
                show_sample_points: false,
                snap_to_guides: false,
                snap_to_canvas: true,
                lock_guides: true,
            },
        },
    ];

    for command in commands {
        let json = serde_json::to_string(&command).unwrap();
        let decoded: Command = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, command);
    }

    // `style` is defaulted, so a caller that omits it places a user guide rather than failing.
    let without_style = r#"{"type":"add_guide","id":"00000000-0000-0000-0000-000000000001","orientation":"vertical","position":5}"#;
    let decoded: Command = serde_json::from_str(without_style).unwrap();
    assert!(matches!(
        decoded,
        Command::AddGuide {
            style: GuideStyle::Normal,
            ..
        }
    ));
}
