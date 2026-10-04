// SPDX-License-Identifier: GPL-3.0-or-later

//! On-canvas text editing with a caret (L.7).
//!
//! Re-derived from `app/tools/gimptexttool-editor.c` (GPL-3.0-or-later), pinned in
//! `docs/upstream-sources.toml`. `gimp_text_tool_move_cursor` is the contract.
//!
//! The fixture is `"abc\nde\nfghij"`, char offsets `a0 b1 c2 \n3 d4 e5 \n6 f7 g8 h9 i10 j11`, so
//! the lines are `0 = 0..3`, `1 = 4..6` (deliberately SHORT, which is what the remembered column
//! exists for) and `2 = 7..12`.

use redrob_core::{
    CaretMovement, Command, CoreError, Document, Editor, LayerId, NodeKind, Pixel, TextCaret,
    TextContent, delete_at_caret, insert_at_caret, move_caret,
};

const TEXT: &str = "abc\nde\nfghij";

fn caret_at(offset: usize) -> TextCaret {
    TextCaret::at(TEXT, offset)
}

fn moved(caret: TextCaret, movement: CaretMovement, count: i32, extend: bool) -> TextCaret {
    move_caret(TEXT, caret, movement, count, extend)
}

/// **Rule 1: moving out of a selection does not advance.**
///
/// Upstream orders the two iters by direction, takes the one in that direction, and sets
/// `cancel_selection = TRUE`, which then SUPPRESSES the step for logical positions. Its own comment
/// says so: *"moving the cursor without extending it should move the cursor to the end of the
/// selection that is in moving direction"*.
///
/// One assertion about the pair, so the claim is the difference rather than two separate facts: a
/// selection of `1..3`, moved Right, lands on **3** and moved Left lands on **1**. **The naive
/// implementation — collapse, then step — gives 4 and 0**, and both are named here so the test
/// cannot pass by coincidence. The third assertion is the control: with no selection the same Right
/// does step, so the suppression is about the selection and not about the movement.
#[test]
fn leaving_a_selection_lands_on_its_end_without_advancing() {
    let selected = TextCaret::with_selection(TEXT, 1, 3);
    assert!(selected.has_selection());

    let right = moved(selected, CaretMovement::LogicalPositions, 1, false);
    let left = moved(selected, CaretMovement::LogicalPositions, -1, false);
    assert_eq!(right.insert(), 3, "collapse-then-step would give 4");
    assert_eq!(left.insert(), 1, "collapse-then-step would give 0");
    // And the selection is gone, because `sel_start` is the cursor when not extending.
    assert!(!right.has_selection());
    assert!(!left.has_selection());

    // Control: without a selection the step is not suppressed.
    assert_eq!(
        moved(caret_at(1), CaretMovement::LogicalPositions, 1, false).insert(),
        2
    );
}

/// Extending keeps the anchor where it is, which is the other half of the same branch.
#[test]
fn extending_a_selection_keeps_the_anchor() {
    let extended = moved(caret_at(1), CaretMovement::LogicalPositions, 2, true);
    assert_eq!((extended.anchor(), extended.insert()), (1, 3));
    assert_eq!(extended.selection(), (1, 3));
}

/// **Rules 2 and 3: the remembered column survives a vertical walk through a short line.**
///
/// `x_pos` is `-1` at the top of upstream's function and assigned unconditionally at the bottom, so
/// only the display-lines case ever stores one. Line 1 is two chars long, so a caret at column 3
/// clamps there — and the memory is what brings it back.
///
/// Measured: `10` (line 2, column 3) up to **6**, up to **3**, down to **6**, down to **10**. The
/// round trip returning to the original offset is the whole claim; an implementation that read the
/// column back from the clamped position would come back to offset 9.
#[test]
fn a_vertical_walk_remembers_the_column_through_a_short_line() {
    let mut caret = caret_at(10);
    assert_eq!(caret.remembered_column(), None);

    caret = moved(caret, CaretMovement::DisplayLines, -1, false);
    assert_eq!((caret.insert(), caret.remembered_column()), (6, Some(3)));
    caret = moved(caret, CaretMovement::DisplayLines, -1, false);
    assert_eq!((caret.insert(), caret.remembered_column()), (3, Some(3)));
    caret = moved(caret, CaretMovement::DisplayLines, 1, false);
    assert_eq!(caret.insert(), 6);
    caret = moved(caret, CaretMovement::DisplayLines, 1, false);
    assert_eq!(caret.insert(), 10, "the walk returned to where it started");
}

/// **Rule 2's other half: a horizontal move CLEARS the memory.**
///
/// The discriminating test, and the first version of it was too weak — it stepped onto a line that
/// does not exist, so nothing moved and the assertion proved nothing either way.
///
/// Sharpened: Up clamps to line 1's end and remembers column 3; a Left then clears the memory and
/// leaves column 1; Down must use column 1, landing on **8**. **Keeping the memory would give 10**,
/// so the two outcomes differ and the assertion is about which one happens.
#[test]
fn a_horizontal_move_forgets_the_remembered_column() {
    let mut caret = caret_at(10);
    caret = moved(caret, CaretMovement::DisplayLines, -1, false);
    assert_eq!(caret.remembered_column(), Some(3));

    caret = moved(caret, CaretMovement::LogicalPositions, -1, false);
    assert_eq!((caret.insert(), caret.remembered_column()), (5, None));

    caret = moved(caret, CaretMovement::DisplayLines, 1, false);
    assert_eq!(caret.insert(), 8, "keeping the memory would give 10");
}

/// Words go to ENDS forward and STARTS backward, and running off the last word stops at the line
/// end — upstream's `forward_to_line_end` fallback when `forward_visible_word_ends` fails.
#[test]
fn words_move_to_ends_forward_and_starts_backward() {
    assert_eq!(
        moved(caret_at(0), CaretMovement::Words, 1, false).insert(),
        3
    );
    assert_eq!(
        moved(caret_at(11), CaretMovement::Words, -1, false).insert(),
        7
    );
    // From inside the last word, forward has no further word end, so it stops at the line end.
    assert_eq!(
        moved(caret_at(8), CaretMovement::Words, 1, false).insert(),
        12
    );
}

/// Paragraph ends are the current line's bounds; buffer ends are the whole text's. Upstream's
/// `GTK_MOVEMENT_PAGES` shares the buffer-ends body, which is why there is no page variant.
#[test]
fn paragraph_and_buffer_ends_go_to_the_line_and_the_text() {
    assert_eq!(
        moved(caret_at(8), CaretMovement::ParagraphEnds, 1, false).insert(),
        12
    );
    assert_eq!(
        moved(caret_at(8), CaretMovement::ParagraphEnds, -1, false).insert(),
        7
    );
    assert_eq!(
        moved(caret_at(5), CaretMovement::BufferEnds, 1, false).insert(),
        12
    );
    assert_eq!(
        moved(caret_at(5), CaretMovement::BufferEnds, -1, false).insert(),
        0
    );
}

/// Typing replaces a selection and leaves the caret after what was typed; deleting takes the
/// selection whichever way the sign points, and only a BARE caret is directional.
#[test]
fn typing_replaces_the_selection_and_deleting_is_directional_only_without_one() {
    let (text, caret) = insert_at_caret(TEXT, TextCaret::with_selection(TEXT, 1, 3), "X");
    assert_eq!(text, "aX\nde\nfghij");
    assert_eq!((caret.insert(), caret.has_selection()), (2, false));

    // A selection goes whichever way the sign points: both directions give the same text.
    let selected = TextCaret::with_selection(TEXT, 1, 3);
    let (forward, _) = delete_at_caret(TEXT, selected, 1);
    let (backward, after) = delete_at_caret(TEXT, selected, -1);
    assert_eq!(forward, "a\nde\nfghij");
    assert_eq!(backward, forward);
    assert_eq!(after.insert(), 1);

    // A bare caret IS directional.
    let (back, _) = delete_at_caret(TEXT, caret_at(2), -1);
    let (fwd, _) = delete_at_caret(TEXT, caret_at(2), 1);
    assert_eq!(back, "ac\nde\nfghij");
    assert_eq!(fwd, "ab\nde\nfghij");
}

/// Char offsets, not byte offsets: a caret can never land inside a multi-byte character.
///
/// `"é"` is two bytes and one char, so a byte-indexed caret at 1 would split it. Three accented
/// characters give a length of 3, and stepping once lands on 1 without panicking.
#[test]
fn the_caret_counts_characters_rather_than_bytes() {
    let text = "éàü";
    assert_eq!(text.len(), 6, "six bytes");
    let caret = TextCaret::at(text, 0);
    let stepped = move_caret(text, caret, CaretMovement::LogicalPositions, 1, false);
    assert_eq!(stepped.insert(), 1);
    let (edited, after) = insert_at_caret(text, stepped, "X");
    assert_eq!(edited, "éXàü");
    assert_eq!(after.insert(), 2);
    // And the buffer end is the char count, not the byte count.
    assert_eq!(
        move_caret(text, caret, CaretMovement::BufferEnds, 1, false).insert(),
        3
    );
}

fn text_document() -> (Editor, LayerId) {
    let mut editor = Editor::new(Document::new(8, 8).unwrap()).unwrap();
    let id = LayerId::new();
    editor
        .execute(Command::AddTextNode {
            id,
            name: "Caption".into(),
            parent: None,
            sibling_index: 1,
            text: TextContent {
                text: TEXT.into(),
                font_family: "sans".into(),
                font_size: 12.0,
                color: Pixel {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 255,
                },
                origin_x: 0.0,
                origin_y: 0.0,
                font_id: redrob_core::EMBEDDED_FONT_ID.into(),
            },
        })
        .unwrap();
    (editor, id)
}

/// The caret is EDITOR state, not document content — upstream keeps it on the tool.
///
/// So placing and moving a caret must leave the serialised document byte-identical, while TYPING
/// must not. Both halves asserted together, because the interesting failure is a caret that
/// silently dirties every save.
#[test]
fn moving_the_caret_leaves_the_document_untouched_while_typing_does_not() {
    let (mut editor, id) = text_document();
    assert_eq!(editor.document().layer(id).unwrap().kind(), NodeKind::Text);
    let before = serde_json::to_string(editor.document()).unwrap();

    editor
        .execute(Command::SetTextCaret {
            id,
            insert: 10,
            anchor: None,
        })
        .unwrap();
    editor
        .execute(Command::MoveTextCaret {
            id,
            movement: CaretMovement::DisplayLines,
            count: -1,
            extend: false,
        })
        .unwrap();
    assert_eq!(editor.text_caret(id).unwrap().insert(), 6);
    assert_eq!(
        serde_json::to_string(editor.document()).unwrap(),
        before,
        "a caret is not a document change"
    );

    editor
        .execute(Command::InsertAtTextCaret {
            id,
            text: "Z".into(),
        })
        .unwrap();
    assert_ne!(serde_json::to_string(editor.document()).unwrap(), before);
    assert_eq!(editor.text_caret(id).unwrap().insert(), 7);
}

/// Typing is undoable and moving the caret is not, which is the consequence of the split above.
///
/// **The assertion that matters here is `undo_depth`, not the document bytes.** Reverse-verification
/// found that out: routing a caret move through `SetTextContent` with unchanged text passed every
/// byte comparison, because the text really is unchanged — what it adds is a HISTORY ENTRY, and
/// only the depth can see that. The cycle-77 shape, where the injection is real and the test
/// measures a weaker quantity than the claim.
#[test]
fn typing_is_undoable_and_a_caret_move_is_not() {
    let (mut editor, id) = text_document();
    editor
        .execute(Command::SetTextCaret {
            id,
            insert: 3,
            anchor: None,
        })
        .unwrap();
    let depth_after_placing = editor.undo_depth();

    editor
        .execute(Command::InsertAtTextCaret {
            id,
            text: "!".into(),
        })
        .unwrap();
    let typed = serde_json::to_string(editor.document()).unwrap();
    assert!(typed.contains("abc!"));
    assert_eq!(
        editor.undo_depth(),
        depth_after_placing + 1,
        "typing pushed exactly one entry"
    );

    editor.undo().unwrap();
    let undone = serde_json::to_string(editor.document()).unwrap();
    assert!(!undone.contains("abc!"), "the typed character was undone");
    let depth_after_undo = editor.undo_depth();

    // A caret move adds NOTHING to undo. This is the assertion the byte comparison could not make.
    editor
        .execute(Command::MoveTextCaret {
            id,
            movement: CaretMovement::BufferEnds,
            count: 1,
            extend: false,
        })
        .unwrap();
    assert_eq!(
        editor.undo_depth(),
        depth_after_undo,
        "a caret move must not be undoable"
    );
    assert_eq!(serde_json::to_string(editor.document()).unwrap(), undone);

    // Placing a caret is the same: state, not an edit.
    editor
        .execute(Command::SetTextCaret {
            id,
            insert: 0,
            anchor: Some(2),
        })
        .unwrap();
    assert_eq!(editor.undo_depth(), depth_after_undo);
}

/// A caret command against a node that is not text is refused rather than silently ignored.
#[test]
fn a_caret_on_a_non_text_node_is_refused() {
    let (mut editor, _) = text_document();
    let raster = editor.document().layers()[0].id();
    assert_eq!(
        editor.document().layer(raster).unwrap().kind(),
        NodeKind::Raster
    );
    assert!(matches!(
        editor.execute(Command::SetTextCaret {
            id: raster,
            insert: 0,
            anchor: None,
        }),
        Err(CoreError::LayerNotFound(_))
    ));
}

/// The four commands round-trip, including the optional anchor and the default `extend`.
#[test]
fn the_caret_commands_round_trip_through_json() {
    let id = LayerId::new();
    let commands = vec![
        Command::SetTextCaret {
            id,
            insert: 4,
            anchor: Some(1),
        },
        Command::MoveTextCaret {
            id,
            movement: CaretMovement::Words,
            count: -2,
            extend: true,
        },
        Command::InsertAtTextCaret {
            id,
            text: "hi".into(),
        },
        Command::DeleteAtTextCaret { id, direction: -1 },
    ];
    for command in commands {
        let json = serde_json::to_string(&command).unwrap();
        let decoded: Command = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, command);
    }

    // `extend` defaults to false and `anchor` to None, so a caller may omit both.
    let terse = format!(
        r#"{{"type":"move_text_caret","id":"{id}","movement":"logical_positions","count":1}}"#
    );
    let decoded: Command = serde_json::from_str(&terse).unwrap();
    assert!(matches!(
        decoded,
        Command::MoveTextCaret { extend: false, .. }
    ));
}
