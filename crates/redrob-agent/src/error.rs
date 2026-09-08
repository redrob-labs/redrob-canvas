// SPDX-License-Identifier: GPL-3.0-or-later

use reqwest::StatusCode;

/// Result type used by this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors produced by the Redrob client and agent session.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("invalid configuration: {0}")]
    InvalidConfiguration(String),

    #[error("an API key is required for this operation")]
    MissingApiKey,

    #[error("HTTP transport failed")]
    Http(#[source] reqwest::Error),

    #[error("Redrob returned HTTP {status}: {body}")]
    HttpStatus { status: StatusCode, body: String },

    #[error("invalid server response")]
    Decode(#[source] serde_json::Error),

    #[error("invalid SSE stream: {0}")]
    Sse(String),

    #[error("Redrob protocol error: {0}")]
    Protocol(String),

    #[error("Redrob completion exceeded the {resource} limit ({limit} bytes or items)")]
    CompletionLimitExceeded {
        resource: &'static str,
        limit: usize,
    },

    #[error("device authorization is pending")]
    AuthorizationPending,

    #[error("device authorization polling must slow down")]
    AuthorizationSlowDown,

    #[error("device authorization was denied")]
    AccessDenied,

    #[error("device authorization expired")]
    ExpiredToken,

    #[error("device authorization was cancelled")]
    Cancelled,

    #[error("device authorization timed out")]
    AuthorizationTimedOut,

    #[error("tool `{0}` is not declared for this session")]
    UnknownTool(String),

    #[error("approval was denied for mutating tool `{0}`")]
    ApprovalDenied(String),

    #[error("agent exceeded the configured tool round limit ({0})")]
    ToolRoundLimit(usize),

    #[error("tool or approval operation failed: {0}")]
    Tool(String),
}

impl From<reqwest::Error> for Error {
    fn from(error: reqwest::Error) -> Self {
        Self::Http(error)
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Decode(error)
    }
}
