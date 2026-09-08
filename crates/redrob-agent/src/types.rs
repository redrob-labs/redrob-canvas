// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Poll, Waker},
    time::{Duration, Instant},
};

use futures_util::future::poll_fn;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Error, Result};

pub(crate) const DEFAULT_BASE_URL: &str = "https://console.redrob.ai/api/backend/v1";

/// API credential whose `Debug` output is always redacted.
#[derive(Clone, Eq, PartialEq)]
pub struct ApiKey(Box<str>);

impl ApiKey {
    pub fn new(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(Error::InvalidConfiguration(
                "API key must not be empty".into(),
            ));
        }
        if value.contains(['\r', '\n']) {
            return Err(Error::InvalidConfiguration(
                "API key must not contain line breaks".into(),
            ));
        }
        Ok(Self(value.into_boxed_str()))
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ApiKey([REDACTED])")
    }
}

/// Redrob transport configuration.
#[derive(Clone)]
pub struct RedrobConfig {
    pub(crate) base_url: String,
    pub(crate) api_key: Option<ApiKey>,
    pub(crate) request_timeout: Duration,
}

impl RedrobConfig {
    pub fn new(api_key: ApiKey) -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.into(),
            api_key: Some(api_key),
            request_timeout: Duration::from_secs(60),
        }
    }

    /// Creates a client configuration suitable for starting device authorization.
    pub fn without_api_key() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.into(),
            api_key: None,
            request_timeout: Duration::from_secs(60),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.request_timeout = timeout;
        self
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn request_timeout(&self) -> Duration {
        self.request_timeout
    }

    pub fn has_api_key(&self) -> bool {
        self.api_key.is_some()
    }
}

impl fmt::Debug for RedrobConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RedrobConfig")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("request_timeout", &self.request_timeout)
            .finish()
    }
}

/// Provider-neutral model metadata returned by `GET /models`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Model {
    pub id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub owned_by: Option<String>,
    pub created: Option<u64>,
}

/// A typed chat message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum ChatMessage {
    System {
        content: String,
    },
    User {
        content: String,
    },
    Assistant {
        content: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tool_calls: Vec<ToolCall>,
    },
    Tool {
        tool_call_id: String,
        content: String,
    },
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self::System {
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self::User {
            content: content.into(),
        }
    }

    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self::Tool {
            tool_call_id: tool_call_id.into(),
            content: content.into(),
        }
    }
}

/// JSON-Schema function tool declaration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FunctionTool {
    pub name: String,
    pub description: Option<String>,
    pub parameters: Value,
    /// Whether execution can mutate user or external state.
    #[serde(default)]
    pub mutating: bool,
}

impl FunctionTool {
    pub fn new(name: impl Into<String>, parameters: Value) -> Result<Self> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(Error::InvalidConfiguration(
                "tool name must not be empty".into(),
            ));
        }
        if !parameters.is_object() {
            return Err(Error::InvalidConfiguration(
                "tool parameters must be a JSON Schema object".into(),
            ));
        }
        Ok(Self {
            name,
            description: None,
            parameters,
            mutating: false,
        })
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn mutating(mut self, mutating: bool) -> Self {
        self.mutating = mutating;
        self
    }
}

/// Provider-neutral chat request. The default model is `auto`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<FunctionTool>,
}

impl ChatRequest {
    pub fn new(messages: Vec<ChatMessage>) -> Self {
        Self {
            model: "auto".into(),
            messages,
            tools: Vec::new(),
        }
    }

    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    pub fn with_tools(mut self, tools: Vec<FunctionTool>) -> Self {
        self.tools = tools;
        self
    }
}

/// Finite resource limits applied while accumulating a streamed completion.
///
/// All string limits are measured in UTF-8 bytes. Tool argument limits measure
/// the raw streamed JSON fragments before parsing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CompletionLimits {
    pub max_assistant_text_bytes: usize,
    pub max_tool_calls: usize,
    pub max_tool_argument_bytes: usize,
    pub max_total_tool_argument_bytes: usize,
    pub max_tool_call_id_bytes: usize,
    pub max_tool_name_bytes: usize,
}

impl Default for CompletionLimits {
    fn default() -> Self {
        Self {
            max_assistant_text_bytes: 1024 * 1024,
            max_tool_calls: 128,
            max_tool_argument_bytes: 1024 * 1024,
            max_total_tool_argument_bytes: 4 * 1024 * 1024,
            max_tool_call_id_bytes: 4 * 1024,
            max_tool_name_bytes: 4 * 1024,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// Parsed only after the streamed call has completed.
    pub arguments: Value,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    Stop,
    ToolCalls,
    Length,
    ContentFilter,
    Other(String),
}

/// Individual events produced by a streaming chat request.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ChatEvent {
    TextDelta(String),
    ToolCallDelta {
        index: usize,
        id: Option<String>,
        name: Option<String>,
        arguments_delta: Option<String>,
    },
    Usage(Usage),
    FinishReason(FinishReason),
    Done,
}

/// Fully accumulated assistant response.
#[derive(Clone, Debug, PartialEq)]
pub struct AssistantResponse {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Option<Usage>,
    pub finish_reason: Option<FinishReason>,
}

/// Device authorization details. The device code is retained privately and is
/// never included in `Debug` output.
#[derive(Clone)]
pub struct DeviceAuthorization {
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: Option<String>,
    pub expires_in: Duration,
    pub interval: Duration,
    pub(crate) device_code: Box<str>,
    pub(crate) received_at: Instant,
}

impl fmt::Debug for DeviceAuthorization {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeviceAuthorization")
            .field("device_code", &"[REDACTED]")
            .field("user_code", &self.user_code)
            .field("verification_uri", &self.verification_uri)
            .field("verification_uri_complete", &self.verification_uri_complete)
            .field("expires_in", &self.expires_in)
            .field("interval", &self.interval)
            .finish()
    }
}

/// Bounded device-token polling controls.
#[derive(Clone, Copy, Debug)]
pub struct DevicePollOptions {
    pub max_wait: Duration,
    pub min_interval: Duration,
    pub max_interval: Duration,
    pub slow_down_increment: Duration,
}

impl Default for DevicePollOptions {
    fn default() -> Self {
        Self {
            max_wait: Duration::from_secs(10 * 60),
            min_interval: Duration::from_secs(1),
            max_interval: Duration::from_secs(30),
            slow_down_increment: Duration::from_secs(5),
        }
    }
}

/// Cloneable cancellation signal used by device polling.
#[derive(Clone, Default)]
pub struct CancellationToken {
    inner: Arc<CancellationInner>,
}

#[derive(Default)]
struct CancellationInner {
    cancelled: AtomicBool,
    wakers: Mutex<Vec<Waker>>,
}

impl fmt::Debug for CancellationToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        if !self.inner.cancelled.swap(true, Ordering::AcqRel) {
            let wakers = std::mem::take(
                &mut *self
                    .inner
                    .wakers
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            );
            for waker in wakers {
                waker.wake();
            }
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }

    pub(crate) async fn cancelled(&self) {
        poll_fn(|context| {
            if self.is_cancelled() {
                return Poll::Ready(());
            }
            let mut wakers = self
                .inner
                .wakers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if self.is_cancelled() {
                return Poll::Ready(());
            }
            if !wakers.iter().any(|waker| waker.will_wake(context.waker())) {
                wakers.push(context.waker().clone());
            }
            Poll::Pending
        })
        .await
    }
}

/// Provider-neutral agent loop settings.
#[derive(Clone, Debug)]
pub struct AgentSessionConfig {
    pub model: String,
    pub tools: Vec<FunctionTool>,
    pub max_tool_rounds: usize,
}

impl Default for AgentSessionConfig {
    fn default() -> Self {
        Self {
            model: "auto".into(),
            tools: Vec::new(),
            max_tool_rounds: 8,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolOutput {
    pub content: String,
}

impl ToolOutput {
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
        }
    }
}
