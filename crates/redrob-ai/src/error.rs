// SPDX-License-Identifier: MIT OR Apache-2.0

/// Result type used by this crate.
pub type Result<T> = std::result::Result<T, AiError>;

/// Everything that can go wrong in the AI layer.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AiError {
    /// The IOPaint virtual environment has no `iopaint` executable.
    #[error("the AI engine (IOPaint) is not installed")]
    NotInstalled,

    /// Neither `uv` nor Python 3.11 was found, so there is nothing to install with.
    #[error("install uv (docs.astral.sh/uv) or Python 3.11 first, then try again")]
    NoInstaller,

    /// An operation needs the engine in the `Ready` state.
    #[error("{0}: start the AI engine first")]
    NotReady(String),

    /// An operation arrived while another request was in flight.
    #[error("{0}: the AI engine is still working")]
    Busy(String),

    /// `iopaint start` exited, or never answered `/api/v1/server-config`.
    #[error("the AI engine did not become ready")]
    NotReadyInTime,

    /// An install step or the server process failed.
    #[error("{0}")]
    Process(String),

    /// The IOPaint server answered with a non-200 status; `detail` is its `{detail}` (or
    /// `{errors}`) field, else the body.
    #[error("AI engine error: {detail}")]
    Server { status: u16, detail: String },

    /// The server returned bytes that are not a PNG, or the PNG could not be encoded.
    #[error("PNG: {0}")]
    Png(String),

    /// A result image or mask does not have the size the operation needs.
    #[error("{0}")]
    Geometry(String),

    #[cfg(feature = "engine")]
    #[error("HTTP transport failed")]
    Http(#[source] reqwest::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("invalid server response")]
    Json(#[source] serde_json::Error),

    #[error(transparent)]
    Trace(#[from] redrob_vector_trace::TraceError),

    #[cfg(feature = "engine")]
    #[error(transparent)]
    Agent(#[from] redrob_agent::Error),
}

#[cfg(feature = "engine")]
impl From<reqwest::Error> for AiError {
    fn from(error: reqwest::Error) -> Self {
        Self::Http(error)
    }
}

impl From<serde_json::Error> for AiError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}
