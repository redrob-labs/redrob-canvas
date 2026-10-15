// SPDX-License-Identifier: MIT OR Apache-2.0

//! The agent's tools. `redrob_ai::agent` calls [`ModeTools::execute`] off the UI thread, but the
//! document lives on it: a tool call becomes a [`ToolRequest`] the shell takes out of a channel
//! each frame, runs against the mode bridge (and the job queue for the long ones), and answers
//! through the request's [`ToolReply`]. The tool thread blocks on that reply, which is fine: it is
//! the agent's `spawn_blocking` task, not the UI.

use std::{
    sync::{Mutex, mpsc},
    time::Duration,
};

use redrob_ai::agent::{FunctionTool, ModeTools};
use serde_json::{Value, json};

use super::jobs::ToolKind;
use crate::shell::Mode;

/// How long a tool call waits for the UI thread (a diffusion inpaint on a CPU takes minutes).
pub const TOOL_TIMEOUT: Duration = Duration::from_secs(20 * 60);

/// The agent's own tools (beside the [`ToolKind`] ones).
pub const SNAPSHOT_INFO: &str = "snapshot_info";
pub const SWITCH_MODE: &str = "switch_mode";

/// Answers one tool call (`Ok` = the JSON the model sees).
pub type ToolReply = mpsc::Sender<Result<Value, String>>;

/// One tool call waiting for the UI thread.
pub struct ToolRequest {
    pub name: String,
    pub arguments: Value,
    pub reply: ToolReply,
}

/// What the tools know without asking the UI thread, refreshed by the shell each frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModeInfo {
    pub mode: Option<Mode>,
    pub document_size: Option<(u32, u32)>,
    pub has_selection: bool,
    pub engine_state: String,
}

/// The shell's [`ModeTools`]: declares the tools and forwards every call to the UI thread.
pub struct ShellTools {
    requests: Mutex<mpsc::Sender<ToolRequest>>,
    info: Mutex<ModeInfo>,
}

impl ShellTools {
    /// The tools and the receiver the shell polls.
    pub fn new() -> (Self, mpsc::Receiver<ToolRequest>) {
        let (tx, rx) = mpsc::channel();
        (
            Self {
                requests: Mutex::new(tx),
                info: Mutex::new(ModeInfo::default()),
            },
            rx,
        )
    }

    /// Called by the shell each frame so `snapshot_info` answers without a round trip.
    pub fn set_info(&self, info: ModeInfo) {
        if let Ok(mut slot) = self.info.lock() {
            *slot = info;
        }
    }

    pub fn info(&self) -> ModeInfo {
        self.info.lock().map(|i| i.clone()).unwrap_or_default()
    }

    fn forward(&self, name: &str, arguments: Value) -> Result<Value, String> {
        let (reply_tx, reply_rx) = mpsc::channel();
        let sent = self
            .requests
            .lock()
            .map_err(|_| "the tool channel is poisoned".to_string())?
            .send(ToolRequest {
                name: name.to_owned(),
                arguments,
                reply: reply_tx,
            });
        sent.map_err(|_| "the editor is gone".to_string())?;
        match reply_rx.recv_timeout(TOOL_TIMEOUT) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => Err("the tool did not finish in time".into()),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err("the editor dropped the tool call".into())
            }
        }
    }
}

/// The JSON `snapshot_info` returns.
pub fn snapshot_info_json(info: &ModeInfo) -> Value {
    json!({
        "mode": info.mode.map(|m| m.label().to_lowercase()),
        "width": info.document_size.map(|s| s.0),
        "height": info.document_size.map(|s| s.1),
        "has_document": info.document_size.is_some(),
        "has_selection": info.has_selection,
        "engine": info.engine_state,
    })
}

fn object(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
    })
}

/// The function tools the agent declares, in panel order.
pub fn tool_list() -> Vec<FunctionTool> {
    let tool = |name: &str, parameters: Value, description: &str, mutating: bool| {
        FunctionTool::new(name, parameters)
            .map(|t| t.with_description(description).mutating(mutating))
            .ok()
    };
    [
        tool(
            SNAPSHOT_INFO,
            object(json!({}), &[]),
            "The current mode (photo, vector, layout), the document size in pixels, whether a \
             selection exists and the AI engine's state.",
            false,
        ),
        tool(
            ToolKind::Erase.tool_name(),
            object(json!({}), &[]),
            "Erase what is inside the current selection and fill it from the surroundings (Photo \
             mode, needs a selection).",
            true,
        ),
        tool(
            ToolKind::Inpaint.tool_name(),
            object(
                json!({
                    "prompt": { "type": "string", "description": "What to paint in the selection" },
                    "negative_prompt": { "type": "string" },
                }),
                &["prompt"],
            ),
            "Replace the current selection with what the prompt describes (Photo mode, needs a \
             selection and a diffusion model).",
            true,
        ),
        tool(
            ToolKind::Outpaint.tool_name(),
            object(
                json!({
                    "left": { "type": "integer", "minimum": 0 },
                    "top": { "type": "integer", "minimum": 0 },
                    "right": { "type": "integer", "minimum": 0 },
                    "bottom": { "type": "integer", "minimum": 0 },
                    "prompt": { "type": "string" },
                }),
                &[],
            ),
            "Grow the canvas by the given pixels on each side and paint the new border to match \
             (Photo mode).",
            true,
        ),
        tool(
            ToolKind::RemoveBackground.tool_name(),
            object(json!({}), &[]),
            "Cut the subject out of the picture onto a new transparent layer.",
            true,
        ),
        tool(
            ToolKind::Upscale.tool_name(),
            object(
                json!({ "scale": { "type": "integer", "enum": [2, 3, 4] } }),
                &[],
            ),
            "Upscale the picture 2x, 3x or 4x (RealESRGAN) onto a new layer.",
            true,
        ),
        tool(
            ToolKind::FaceRestore.tool_name(),
            object(json!({}), &[]),
            "Restore faces in the picture (GFPGAN) onto a new layer.",
            true,
        ),
        tool(
            ToolKind::SelectSubject.tool_name(),
            object(json!({}), &[]),
            "Select the main subject of the picture (Photo mode).",
            true,
        ),
        tool(
            ToolKind::Vectorize.tool_name(),
            object(
                json!({
                    "preset": {
                        "type": "string",
                        "description": "An Image Trace preset name",
                        "enum": redrob_ai::vectorize::preset_names(),
                    },
                }),
                &[],
            ),
            "Trace the picture into vector paths and open them in Vector mode.",
            true,
        ),
        tool(
            SWITCH_MODE,
            object(
                json!({ "mode": { "type": "string", "enum": ["photo", "vector", "layout"] } }),
                &["mode"],
            ),
            "Switch the window to another editor mode.",
            false,
        ),
    ]
    .into_iter()
    .flatten()
    .collect()
}

impl ModeTools for ShellTools {
    fn list_tools(&self) -> Vec<FunctionTool> {
        tool_list()
    }

    fn execute(&self, name: &str, arguments: Value) -> Result<Value, String> {
        if name == SNAPSHOT_INFO {
            return Ok(snapshot_info_json(&self.info()));
        }
        if name != SWITCH_MODE && ToolKind::from_tool_name(name).is_none() {
            return Err(format!("unknown tool {name}"));
        }
        self.forward(name, arguments)
    }
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_kind_is_declared_plus_the_two_shell_tools() {
        let tools = tool_list();
        assert_eq!(tools.len(), ToolKind::ALL.len() + 2);
        for kind in ToolKind::ALL {
            assert!(tools.iter().any(|t| t.name == kind.tool_name()), "{kind:?}");
        }
        assert!(tools.iter().any(|t| t.name == SNAPSHOT_INFO));
        assert!(tools.iter().any(|t| t.name == SWITCH_MODE));
    }

    #[test]
    fn snapshot_info_answers_locally_and_other_calls_go_to_the_ui_thread() {
        let (tools, rx) = ShellTools::new();
        tools.set_info(ModeInfo {
            mode: Some(Mode::Vector),
            document_size: Some((640, 480)),
            has_selection: false,
            engine_state: "stopped".into(),
        });
        let info = tools.execute(SNAPSHOT_INFO, json!({})).unwrap();
        assert_eq!(info["mode"], "vector");
        assert_eq!(info["width"], 640);
        assert_eq!(info["has_document"], true);
        assert!(rx.try_recv().is_err(), "no round trip for snapshot_info");
        assert!(tools.execute("nope", json!({})).is_err());
        // A forwarded call blocks until the UI thread answers.
        let worker =
            std::thread::spawn(move || tools.execute(SWITCH_MODE, json!({ "mode": "layout" })));
        let request = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(request.name, SWITCH_MODE);
        assert_eq!(request.arguments["mode"], "layout");
        request.reply.send(Ok(json!({ "ok": true }))).unwrap();
        assert_eq!(worker.join().unwrap().unwrap(), json!({ "ok": true }));
        assert_eq!(parse_mode(" Layout "), Some(Mode::Layout));
        assert_eq!(parse_mode("3d"), None);
    }
}
