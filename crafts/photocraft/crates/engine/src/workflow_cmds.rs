//! Workflow helpers: action search, reference images, pre-installed scripts (placeholder).

use serde::{Deserialize, Serialize};

use crate::commands::CommandSpec;

/// Reference images docker state (placeholder, filled in below).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReferenceState {}

pub fn specs() -> Vec<CommandSpec> {
    Vec::new()
}
