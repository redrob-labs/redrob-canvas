// SPDX-License-Identifier: MIT OR Apache-2.0

//! Async client for the shared Redrob Code daemon and provider-neutral tool-using agent sessions.
//!
//! Chat goes to the one Redrob Code daemon on this machine (`<daemon>/v1/chat/completions`), which
//! holds the Redrob console key and calls the console with it; this crate never carries a console
//! key on a chat request and has no direct path to the console for chat. Provider wire formats are
//! intentionally private. Applications interact with typed chat messages, function tools,
//! [`RedrobClient`], and [`AgentSession`].

mod client;
mod engine;
mod error;
mod images;
mod session;
mod sse;
mod types;
mod wire;

pub use client::{ChatEventStream, DeviceFlowClient, RedrobClient};
pub use engine::{
    AccountStatus, DEFAULT_IDLE_EXIT_MINUTES, EngineConfig, INSTALL_URL, NOT_INSTALLED_MESSAGE,
    REDROB_PROVIDER_ID, unavailable_message,
};
pub use error::{Error, Result};
pub use images::{ImageEditRequest, ImageGenerationRequest, ImageModel, ImageResponse};
pub use redrob_engine_client::Unavailable;
pub use session::{AgentSession, ApprovalGate, ChatProvider, ToolExecutor};
pub use types::{
    AgentSessionConfig, ApiKey, AssistantResponse, CancellationToken, ChatEvent, ChatMessage,
    ChatRequest, CompletionLimits, DeviceAuthorization, DevicePollOptions, FinishReason,
    FunctionTool, Model, ToolCall, ToolOutput, Usage,
};
