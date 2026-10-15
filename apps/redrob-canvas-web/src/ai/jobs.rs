// SPDX-License-Identifier: MIT OR Apache-2.0

//! The AI tools' names and the job types the shell applies, as the desktop defines them
//! (apps/redrob-canvas/src/ai/jobs.rs), without the queue that would run them: in the browser a
//! job never starts, so the queue is always idle.

use std::time::Duration;

use redrob_ai::vectorize::TraceResult;
use redrob_ai::{Expand, Mask, Rgba8Image};

use super::bridge::Capabilities;
use super::tools::ToolReply;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ToolKind {
    Erase,
    Inpaint,
    Outpaint,
    RemoveBackground,
    Upscale,
    FaceRestore,
    SelectSubject,
    Vectorize,
}

impl ToolKind {
    pub const ALL: [ToolKind; 8] = [
        ToolKind::Erase,
        ToolKind::Inpaint,
        ToolKind::Outpaint,
        ToolKind::RemoveBackground,
        ToolKind::Upscale,
        ToolKind::FaceRestore,
        ToolKind::SelectSubject,
        ToolKind::Vectorize,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ToolKind::Erase => "Erase",
            ToolKind::Inpaint => "Inpaint",
            ToolKind::Outpaint => "Outpaint / Expand canvas",
            ToolKind::RemoveBackground => "Remove background",
            ToolKind::Upscale => "Upscale",
            ToolKind::FaceRestore => "Face restore",
            ToolKind::SelectSubject => "Select subject",
            ToolKind::Vectorize => "Vectorize",
        }
    }

    /// The agent's tool name.
    pub fn tool_name(self) -> &'static str {
        match self {
            ToolKind::Erase => "erase_selection",
            ToolKind::Inpaint => "inpaint",
            ToolKind::Outpaint => "outpaint",
            ToolKind::RemoveBackground => "remove_background",
            ToolKind::Upscale => "upscale",
            ToolKind::FaceRestore => "face_restore",
            ToolKind::SelectSubject => "select_subject",
            ToolKind::Vectorize => "vectorize",
        }
    }

    pub fn from_tool_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.tool_name() == name)
    }

    /// In the browser nothing is available; the menu's hover text says why.
    pub fn available(self, _caps: &Capabilities) -> bool {
        false
    }

    pub fn unavailable_reason(self, _caps: &Capabilities) -> Option<&'static str> {
        Some(super::DESKTOP_ONLY)
    }
}

/// Everything a job needs: never gathered here.
#[derive(Clone, Debug, PartialEq)]
pub enum JobInput {}

/// What a finished job hands back to the UI thread, to apply through the bridge. The desktop's
/// variants, so `shell.rs` matches every one.
#[derive(Clone, Debug, PartialEq)]
pub enum JobOutput {
    Layer {
        name: String,
        image: Rgba8Image,
        offset: (i32, i32),
    },
    Expanded {
        name: String,
        image: Rgba8Image,
        expand: Expand,
    },
    Selection(Mask),
    Trace {
        preset: String,
        result: TraceResult,
        size: (u32, u32),
    },
}

/// A job's result, delivered through the queue.
pub struct JobResult {
    pub kind: ToolKind,
    pub output: Result<JobOutput, String>,
    pub elapsed: Duration,
    pub reply: Option<ToolReply>,
}

/// The one-job queue: always empty here.
#[derive(Default)]
pub struct JobQueue {}

impl JobQueue {
    pub fn new() -> Self {
        Self {}
    }

    pub fn is_busy(&self) -> bool {
        false
    }

    pub fn cancel(&mut self) -> Option<ToolReply> {
        None
    }
}
