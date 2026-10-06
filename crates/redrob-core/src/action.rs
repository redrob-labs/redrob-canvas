// SPDX-License-Identifier: GPL-3.0-or-later

//! Actions (P14): a recorded list of editor commands that can be saved and played back, the way
//! Photoshop's Actions and GIMP's recorded macros work.
//!
//! This is deliberately NOT a scripting language. An action holds only the typed [`Command`]s the
//! editor already validates one by one, so replaying a file can do nothing a user could not do by
//! hand -- no file access, no network, no code. The file is plain JSON:
//!
//! ```json
//! { "format": "redrob-action", "version": 1, "name": "Sepia frame", "commands": [ ... ] }
//! ```
//!
//! Playback runs every command inside one history group, so the whole action is one undo step,
//! and if any command fails the group is cancelled and the document is exactly as before.

use serde::{Deserialize, Serialize};

use crate::{ChangeSet, Command, CoreError, Editor, Result};

pub const ACTION_FORMAT: &str = "redrob-action";
pub const ACTION_VERSION: u32 = 1;
/// More than any hand-recorded action needs; a bound so a hostile file cannot queue unbounded work.
pub const MAX_ACTION_COMMANDS: usize = 4096;
pub const MAX_ACTION_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_ACTION_NAME_BYTES: usize = 256;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub format: String,
    pub version: u32,
    pub name: String,
    pub commands: Vec<Command>,
}

impl Action {
    pub fn new(name: impl Into<String>, commands: Vec<Command>) -> Self {
        Self {
            format: ACTION_FORMAT.into(),
            version: ACTION_VERSION,
            name: name.into(),
            commands,
        }
    }

    /// Parses and validates an action file. Command payloads are validated by the editor when
    /// they run; this checks the envelope and the limits.
    pub fn from_json(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_ACTION_BYTES {
            return Err(CoreError::InvalidAction("the file is too large".into()));
        }
        let action: Self = serde_json::from_slice(bytes)
            .map_err(|error| CoreError::InvalidAction(format!("not an action file: {error}")))?;
        action.validate()?;
        Ok(action)
    }

    pub fn to_json(&self) -> Result<Vec<u8>> {
        self.validate()?;
        serde_json::to_vec_pretty(self)
            .map_err(|error| CoreError::InvalidAction(format!("could not write: {error}")))
    }

    pub fn validate(&self) -> Result<()> {
        if self.format != ACTION_FORMAT {
            return Err(CoreError::InvalidAction(format!(
                "format is {:?}, expected {ACTION_FORMAT:?}",
                self.format
            )));
        }
        if self.version != ACTION_VERSION {
            return Err(CoreError::InvalidAction(format!(
                "version {} is not supported (this build reads {ACTION_VERSION})",
                self.version
            )));
        }
        if self.name.trim().is_empty() || self.name.len() > MAX_ACTION_NAME_BYTES {
            return Err(CoreError::InvalidAction("the name is empty or too long".into()));
        }
        if self.commands.is_empty() {
            return Err(CoreError::InvalidAction("the action has no steps".into()));
        }
        if self.commands.len() > MAX_ACTION_COMMANDS {
            return Err(CoreError::InvalidAction("the action has too many steps".into()));
        }
        if let Some(index) = self.commands.iter().position(|command| !is_recordable(command)) {
            return Err(CoreError::InvalidAction(format!(
                "step {} is not an edit an action may replay",
                index + 1
            )));
        }
        Ok(())
    }
}

/// Whether a command belongs in an action. Caret movement is editing state rather than an edit,
/// so a recording that contained it would replay a cursor, not a change. (Undo and redo are not
/// commands at all, so they can never be recorded.)
pub fn is_recordable(command: &Command) -> bool {
    !matches!(
        command,
        Command::SetTextCaret { .. }
            | Command::MoveTextCaret { .. }
            | Command::InsertAtTextCaret { .. }
            | Command::DeleteAtTextCaret { .. }
    )
}

impl Editor {
    /// Plays `action` as one undo step. On the first failing step everything the action did is
    /// rolled back and the error names that step.
    pub fn play_action(&mut self, action: &Action) -> Result<ChangeSet> {
        action.validate()?;
        if self.is_group_active() {
            return Err(CoreError::InvalidAction(
                "an action cannot start inside another grouped edit".into(),
            ));
        }
        self.begin_group(format!("Action: {}", action.name))?;
        for (index, command) in action.commands.iter().enumerate() {
            if let Err(error) = self.execute(command.clone()) {
                // Restores the document the group started from.
                self.cancel_group()?;
                return Err(CoreError::InvalidAction(format!(
                    "step {} failed, nothing was changed: {error}",
                    index + 1
                )));
            }
        }
        self.end_group()?;
        Ok(ChangeSet::whole_document(self.generation(), self.document()))
    }
}
