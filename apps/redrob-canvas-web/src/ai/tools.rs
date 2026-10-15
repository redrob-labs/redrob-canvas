// SPDX-License-Identifier: MIT OR Apache-2.0

//! The agent's tool-call plumbing as the shell sees it (apps/redrob-canvas/src/ai/tools.rs):
//! the request type and the reply channel. No agent runs in the browser, so no request is ever
//! made; the types exist for `shell.rs`'s `handle_tool_request`.

use std::sync::mpsc;

use serde_json::Value;

use crate::shell::Mode;

pub const SWITCH_MODE: &str = "switch_mode";

/// Where a tool call's answer goes.
pub type ToolReply = mpsc::Sender<Result<Value, String>>;

/// One tool call from the agent, answered on the UI thread.
pub struct ToolRequest {
    pub name: String,
    pub arguments: Value,
    pub reply: ToolReply,
}

/// `"photo" | "vector" | "layout"` to a mode.
pub fn parse_mode(name: &str) -> Option<Mode> {
    match name.trim().to_ascii_lowercase().as_str() {
        "photo" => Some(Mode::Photo),
        "vector" => Some(Mode::Vector),
        "layout" => Some(Mode::Layout),
        _ => None,
    }
}
