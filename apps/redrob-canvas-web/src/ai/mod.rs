// SPDX-License-Identifier: MIT OR Apache-2.0

//! The AI layer as the web shell has it: none. The IOPaint engine is a local Python process and
//! the agent runs on tokio, neither of which a browser tab has, so this module keeps the exact
//! surface `shell.rs` and `handoff.rs` use from `crate::ai` (the desktop module is
//! apps/redrob-canvas/src/ai/) and answers every tool with [`DESKTOP_ONLY`]. The mode bridges are
//! the desktop file itself (`bridge`, pure document adapters), so the Mode menu's hand-offs,
//! including the in-process Image Trace, work in the browser.

#[path = "../../../redrob-canvas/src/ai/bridge.rs"]
pub mod bridge;
pub mod jobs;
pub mod panel;
pub mod tools;

use std::path::PathBuf;

use serde_json::Value;

use self::jobs::{JobInput, JobQueue, JobResult, ToolKind};
use self::tools::{ToolReply, ToolRequest};
use crate::shell::Mode;

/// What the dock and every disabled tool say.
pub const DESKTOP_ONLY: &str = "AI tools need the desktop app";
/// Where the desktop app is.
pub const DESKTOP_URL: &str = "https://github.com/redrob-labs/redrob-canvas/releases/latest";

/// What the panel and the menu ask the shell to do (the desktop's list, so `shell.rs` matches
/// every arm).
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Run(ToolKind),
    Install,
    Start,
    Stop,
    SetModel,
    AgentSend(String),
    AgentReset,
    CancelJob,
}

/// The cards' parameters: nothing to set without an engine.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolSettings {}

/// Stands in for the engine handle the desktop `AiPanel::engine` returns: never present.
pub struct NoEngine;

impl NoEngine {
    pub fn config(&self) -> Option<NoConfig> {
        None
    }
}

/// Stands in for the engine's configuration.
pub struct NoConfig;

impl NoConfig {
    pub fn has_plugin(&self, _plugin: &str) -> bool {
        false
    }
}

/// The right dock's state.
pub struct AiPanel {
    pub open: bool,
    pub settings: ToolSettings,
    pub jobs: JobQueue,
    pub log: Vec<String>,
    /// The last outcome: (line, is_error).
    pub status: Option<(String, bool)>,
    /// Actions queued by the menu, taken by the shell before the dock draws.
    pub queued: Vec<Action>,
}

impl Default for AiPanel {
    fn default() -> Self {
        Self::new(None)
    }
}

impl AiPanel {
    /// `data_dir` is where the desktop installs the engine; the browser has none.
    pub fn new(_data_dir: Option<PathBuf>) -> Self {
        Self {
            open: false,
            settings: ToolSettings::default(),
            jobs: JobQueue::new(),
            log: vec![DESKTOP_ONLY.to_owned()],
            status: None,
            queued: Vec::new(),
        }
    }

    pub fn engine(&self) -> Option<&NoEngine> {
        None
    }

    pub fn is_busy(&self) -> bool {
        false
    }

    pub fn wants_repaint(&self) -> bool {
        false
    }

    pub fn push_log(&mut self, line: impl Into<String>) {
        self.log.push(line.into());
    }

    pub fn drain_engine_log(&mut self) {}

    pub fn install(&mut self) {
        self.status = Some((DESKTOP_ONLY.to_owned(), true));
    }

    pub fn start(&mut self) {
        self.status = Some((DESKTOP_ONLY.to_owned(), true));
    }

    pub fn stop(&mut self) {}

    pub fn set_model(&mut self) {}

    pub fn start_job(
        &mut self,
        _kind: ToolKind,
        _input: JobInput,
        _reply: Option<ToolReply>,
    ) -> Result<(), String> {
        Err(DESKTOP_ONLY.to_owned())
    }

    pub fn poll_job(&mut self) -> Option<JobResult> {
        None
    }

    pub fn take_tool_requests(&mut self) -> Vec<ToolRequest> {
        Vec::new()
    }

    pub fn refresh_info(
        &self,
        _mode: Mode,
        _caps: &bridge::Capabilities,
        _size: Option<(u32, u32)>,
    ) {
    }

    pub fn agent_send(&mut self, _text: String) {
        self.status = Some((DESKTOP_ONLY.to_owned(), true));
    }

    pub fn agent_reset(&mut self) {}

    pub fn poll_agent(&mut self) {}
}

/// The IOPaint plugin `kind` needs: no engine, so none to check.
pub fn plugin_for(_kind: ToolKind, _settings: &ToolSettings) -> Option<&'static str> {
    None
}

/// Gathers a job's inputs from the active mode: there is no engine to hand them to.
pub fn gather_input(
    _bridge: &mut dyn bridge::ModeBridge,
    _kind: ToolKind,
    _settings: &ToolSettings,
    _args: Option<&Value>,
) -> Result<JobInput, String> {
    Err(DESKTOP_ONLY.to_owned())
}
