//! Resource storages, bundles, native preset files and palette file readers (`resources.*`).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always};
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// Resource storages (placeholder, filled in below).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct StorageState {}

pub fn specs() -> Vec<CommandSpec> {
    let _ = (bad, always, json!({}), Value::Null);
    Vec::new()
}
