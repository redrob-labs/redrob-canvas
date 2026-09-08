// SPDX-License-Identifier: GPL-3.0-or-later

//! Async Redrob API client and provider-neutral tool-using agent sessions.
//!
//! Provider wire formats are intentionally private. Applications interact with
//! typed chat messages, function tools, [`RedrobClient`], and [`AgentSession`].

mod client;
mod error;
mod session;
mod sse;
mod types;
mod wire;

pub use client::{ChatEventStream, RedrobClient};
pub use error::{Error, Result};
pub use session::{AgentSession, ApprovalGate, ChatProvider, ToolExecutor};
pub use types::{
    AgentSessionConfig, ApiKey, AssistantResponse, CancellationToken, ChatEvent, ChatMessage,
    ChatRequest, CompletionLimits, DeviceAuthorization, DevicePollOptions, FinishReason,
    FunctionTool, Model, RedrobConfig, ToolCall, ToolOutput, Usage,
};
