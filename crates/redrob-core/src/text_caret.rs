// SPDX-License-Identifier: GPL-3.0-or-later

//! On-canvas text editing: a caret, a selection and the moves that change them (L.7).
//!
//! Re-derived from `app/tools/gimptexttool-editor.c` (GPL-3.0-or-later), pinned and attributed in
//! `docs/upstream-sources.toml`. `gimp_text_tool_move_cursor` is the whole contract and it is
//! readable end to end.
//!
//! # The caret lives on the EDITOR, not on the document
//!
//! Upstream keeps it on the TOOL — `text_tool->x_pos`, and the insert/selection marks on the
//! tool's buffer — not on the text object. That is followed here for a reason that matters beyond
//! symmetry: a caret is editing state, so putting it in `TextContent` would serialise a cursor
//! into every saved project and change the bytes of files that have no caret in them.
//!
//! # Three rules, each of which a naive implementation gets wrong
//!
//! **1. Moving out of a selection does NOT advance.** With a selection and no extend, upstream
//! orders the two iters by the direction of travel, takes the one in that direction as the caret,
//! and sets `cancel_selection = TRUE` — which then SUPPRESSES the step entirely for logical and
//! visual positions. So pressing Right with text selected puts the caret at the selection's end and
//! stops there. The naive version collapses and then moves, landing one position too far. Its own
//! comment says it: *"when there is a selection, moving the cursor without extending it should move
//! the cursor to the end of the selection that is in moving direction"*.
//!
//! **2. The remembered column is cleared by every move except a vertical one.** `x_pos` is
//! initialised to `-1` at the top of the function and assigned to the tool unconditionally at the
//! bottom (`text_tool->x_pos = x_pos;`); only the display-lines case ever computes a real value. So
//! walking down through a short line and back up returns to the ORIGINAL column, while any
//! horizontal step forgets it.
//!
//! **3. The remembered column is used only when the caret was clamped.** The guard is
//! `x_pos != -1 && (x_pos <= logical.x || x_pos >= logical.x + logical.width)` — the memory applies
//! when the current position is OUTSIDE the line's horizontal extent, which is exactly the case
//! where a short line clamped the caret.
//!
//! # Pending edits are applied first
//!
//! The function opens by flushing `text_tool->pending`, with the reason written down: *"there would
//! be inconsistencies between the text_tool->buffer and layout. This could result in crashes. See
//! bug 751333."* Here the text and the caret are read from the same `&str` in one call, so there is
//! no pending queue to flush — recorded rather than ported, because the hazard it guards does not
//! exist in this shape.
//!
//! # Deliberate divergences, each filed
//!
//! - **Char offsets, not byte offsets.** A byte caret can split a UTF-8 sequence; a char caret
//!   cannot. Upstream steps *visible cursor positions*, which also skips invisible runs — we have
//!   no invisible runs, so the two coincide, but a GRAPHEME CLUSTER does not: a combining mark is
//!   two char steps here and one cursor position there.
//! - **Display lines are logical lines.** Upstream's display-lines case walks the Pango layout, so
//!   a wrapped line is several display lines. `TextContent` has no wrapping, so the lines are the
//!   `\n`-delimited ones.
//! - **Visual positions are not ported.** That case calls `pango_layout_move_cursor_visually`,
//!   which is bidi reordering inside Pango and unreadable here. Refused by name rather than
//!   approximated, the same judgement as `trc`'s `Perceptual`.
//! - **`GTK_MOVEMENT_PAGES` falls through to `BUFFER_ENDS`** in upstream's own switch
//!   (`case GTK_MOVEMENT_PAGES: case GTK_MOVEMENT_BUFFER_ENDS:` share one body, with the comment
//!   `/* well... */`). So a page move is a buffer-ends move there, and this enum has no separate
//!   page variant for that reason.

use serde::{Deserialize, Serialize};

/// Which step a caret move takes, mirroring the `GtkMovementStep` cases upstream handles.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaretMovement {
    /// One cursor position per count, in logical (string) order.
    LogicalPositions,
    /// Word ends forward, word starts backward. Forward past the last word stops at the line end.
    Words,
    /// Up or down a line, keeping the remembered column.
    DisplayLines,
    /// The start or end of the current line.
    ParagraphEnds,
    /// The start or end of the whole text. Upstream's `PAGES` case shares this body.
    BufferEnds,
}

/// A caret and the other end of its selection.
///
/// `insert` is the caret; `anchor` is upstream's `selection_bound`. They are equal when nothing is
/// selected. Both are CHAR offsets into the text — see the module note on why not bytes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct TextCaret {
    insert: usize,
    anchor: usize,
    /// Upstream's `x_pos`: the column a vertical walk is trying to return to.
    ///
    /// `None` is its `-1`. Cleared by every move except a vertical one, which is what makes a walk
    /// down through a short line and back up land where it started.
    remembered_column: Option<usize>,
}

impl TextCaret {
    /// A caret at `offset` with nothing selected, clamped into `text`.
    pub fn at(text: &str, offset: usize) -> Self {
        let offset = offset.min(text.chars().count());
        Self {
            insert: offset,
            anchor: offset,
            remembered_column: None,
        }
    }

    /// A caret with a selection from `anchor` to `insert`, both clamped into `text`.
    pub fn with_selection(text: &str, anchor: usize, insert: usize) -> Self {
        let len = text.chars().count();
        Self {
            insert: insert.min(len),
            anchor: anchor.min(len),
            remembered_column: None,
        }
    }

    pub fn insert(&self) -> usize {
        self.insert
    }

    pub fn anchor(&self) -> usize {
        self.anchor
    }

    pub fn remembered_column(&self) -> Option<usize> {
        self.remembered_column
    }

    /// Whether anything is selected.
    pub fn has_selection(&self) -> bool {
        self.insert != self.anchor
    }

    /// The selected range as `(start, end)` char offsets, ordered.
    pub fn selection(&self) -> (usize, usize) {
        if self.insert <= self.anchor {
            (self.insert, self.anchor)
        } else {
            (self.anchor, self.insert)
        }
    }
}

/// The line containing `offset`, as `(line index, column, line start, line end)` in chars.
///
/// Line ends are exclusive of the `\n`, so a caret at the end of a line and one at the start of the
/// next are different offsets, which is what lets `ParagraphEnds` be a no-op when already there.
fn line_of(text: &str, offset: usize) -> (usize, usize, usize, usize) {
    let mut line = 0usize;
    let mut start = 0usize;
    for (index, ch) in text.chars().enumerate() {
        if index == offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            start = index + 1;
        }
    }
    let mut end = start;
    for ch in text.chars().skip(start) {
        if ch == '\n' {
            break;
        }
        end += 1;
    }
    (line, offset.saturating_sub(start), start, end)
}

/// The `(start, end)` char offsets of line `wanted`, or `None` past the last line.
fn line_bounds(text: &str, wanted: usize) -> Option<(usize, usize)> {
    let mut line = 0usize;
    let mut start = 0usize;
    let total = text.chars().count();
    for (index, ch) in text.chars().enumerate() {
        if ch == '\n' {
            if line == wanted {
                return Some((start, index));
            }
            line += 1;
            start = index + 1;
        }
    }
    if line == wanted {
        Some((start, total))
    } else {
        None
    }
}

/// Move the caret, following `gimp_text_tool_move_cursor`.
///
/// `count` is signed; negative moves backward. `extend` keeps the anchor where it is.
pub fn move_caret(
    text: &str,
    caret: TextCaret,
    movement: CaretMovement,
    count: i32,
    extend: bool,
) -> TextCaret {
    let len = text.chars().count();
    let mut insert = caret.insert.min(len);
    let anchor = caret.anchor.min(len);

    // Rule 1. Without extend, a selection is left by moving TO its end in the direction of travel,
    // and the step itself is then suppressed for logical positions.
    let mut cancel_selection = false;
    let new_anchor = if extend {
        anchor
    } else {
        if insert != anchor {
            // `gtk_text_iter_order` puts the smaller first; the direction picks which end the caret
            // takes.
            insert = if count > 0 {
                insert.max(anchor)
            } else {
                insert.min(anchor)
            };
            cancel_selection = true;
        }
        insert
    };

    let mut remembered_column = None;

    match movement {
        CaretMovement::LogicalPositions => {
            if !cancel_selection {
                insert = if count >= 0 {
                    insert.saturating_add(count as usize).min(len)
                } else {
                    insert.saturating_sub(count.unsigned_abs() as usize)
                };
            }
        }
        CaretMovement::Words => {
            let chars: Vec<char> = text.chars().collect();
            let mut steps = count.unsigned_abs();
            while steps > 0 {
                if count > 0 {
                    // Forward to the next word END. Failing to find one stops at the line end,
                    // which is upstream's `forward_to_line_end` fallback.
                    let mut i = insert;
                    while i < len && !chars[i].is_alphanumeric() {
                        i += 1;
                    }
                    if i >= len {
                        insert = line_of(text, insert).3;
                        break;
                    }
                    while i < len && chars[i].is_alphanumeric() {
                        i += 1;
                    }
                    insert = i;
                } else {
                    // Backward to the previous word START.
                    let mut i = insert;
                    while i > 0 && !chars[i - 1].is_alphanumeric() {
                        i -= 1;
                    }
                    while i > 0 && chars[i - 1].is_alphanumeric() {
                        i -= 1;
                    }
                    insert = i;
                }
                steps -= 1;
            }
        }
        CaretMovement::DisplayLines => {
            let (line, column, _, _) = line_of(text, insert);
            // Rule 3. The memory applies only when the caret sits outside the line's extent, which
            // is exactly where a short line clamped it.
            let (_, _, _, line_end) = line_of(text, insert);
            let current_width = line_end - line_of(text, insert).2;
            let wanted = match caret.remembered_column {
                Some(remembered) if column >= current_width => remembered,
                _ => column,
            };
            let target = line as i64 + i64::from(count);
            if target >= 0 {
                if let Some((start, end)) = line_bounds(text, target as usize) {
                    insert = (start + wanted).min(end);
                    // Rule 2. Only this case remembers a column.
                    remembered_column = Some(wanted);
                }
            } else {
                insert = 0;
                remembered_column = Some(wanted);
            }
        }
        CaretMovement::ParagraphEnds => {
            let (_, _, start, end) = line_of(text, insert);
            if count < 0 {
                insert = start;
            } else if count > 0 {
                insert = end;
            }
        }
        CaretMovement::BufferEnds => {
            if count < 0 {
                insert = 0;
            } else if count > 0 {
                insert = len;
            }
        }
    }

    TextCaret {
        insert,
        anchor: if extend { new_anchor } else { insert },
        remembered_column,
    }
}

/// Replace the selection (or insert at the caret) with `inserted`, returning the new text and
/// caret.
///
/// The caret lands after the inserted run with nothing selected, which is what
/// `gimp_text_buffer_insert` leaves behind.
pub fn insert_at_caret(text: &str, caret: TextCaret, inserted: &str) -> (String, TextCaret) {
    let (start, end) = caret.selection();
    let chars: Vec<char> = text.chars().collect();
    let mut out: String = chars[..start.min(chars.len())].iter().collect();
    out.push_str(inserted);
    out.extend(chars[end.min(chars.len())..].iter());
    let caret = TextCaret::at(&out, start + inserted.chars().count());
    (out, caret)
}

/// Delete the selection, or one position in `direction` when nothing is selected.
///
/// A selection is deleted whatever the direction — the direction only decides which side of a bare
/// caret goes, so backspace and delete are one operation with a sign.
pub fn delete_at_caret(text: &str, caret: TextCaret, direction: i32) -> (String, TextCaret) {
    let chars: Vec<char> = text.chars().collect();
    let (mut start, mut end) = caret.selection();
    if start == end {
        if direction < 0 {
            start = start.saturating_sub(1);
        } else {
            end = (end + 1).min(chars.len());
        }
    }
    let mut out: String = chars[..start.min(chars.len())].iter().collect();
    out.extend(chars[end.min(chars.len())..].iter());
    let caret = TextCaret::at(&out, start);
    (out, caret)
}
