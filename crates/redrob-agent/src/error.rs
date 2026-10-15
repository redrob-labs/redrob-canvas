// SPDX-License-Identifier: MIT OR Apache-2.0

use reqwest::StatusCode;

/// Result type used by this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors produced by the Redrob client and agent session.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error("invalid configuration: {0}")]
    InvalidConfiguration(String),

    /// No shared Redrob Code daemon to talk to. There is deliberately no fallback to the Console:
    /// the daemon holds the key and applies masking, and a second path would hide whether it did.
    #[error("{}", crate::engine::unavailable_message(.0))]
    EngineUnavailable(redrob_engine_client::Unavailable),

    /// The daemon answered 401 with `code: "engine_not_authenticated"`: it has no Console
    /// credential of its own. The only condition under which an app should offer a sign-in.
    #[error("the Redrob Code engine is not signed in to the Redrob console")]
    EngineNotAuthenticated,

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
