// SPDX-License-Identifier: MIT OR Apache-2.0

//! AI tools shared by every Redrob Canvas mode.
//!
//! Three things live here, none of them user interface (the shell draws the panels):
//!
//! - [`iopaint`]: the local IOPaint engine (Sanster/IOPaint 1.6.0, Apache-2.0). Its models are
//!   PyTorch, so they run in IOPaint's own Python process: [`iopaint::Engine`] installs it into an
//!   app-owned virtual environment and starts `iopaint start` on a free loopback port, and
//!   [`iopaint::Client`] speaks its HTTP API (inpaint, outpaint, the plugins).
//! - [`vectorize`]: Image Trace, a thin wrapper over `redrob_vector_trace`.
//! - [`agent`]: the mode agent, an [`redrob_agent::AgentSession`] on its own runtime thread that
//!   calls whatever tools the current mode exposes through [`agent::ModeTools`].
//!
//! Pixels cross the boundary as the crate's own [`pixels::Rgba8Image`] and [`pixels::Mask`] so the
//! shell can bridge any editor's buffers without this crate depending on one of them.
//!
//! The `engine` feature (default) is the IOPaint engine and the agent; without it only the pixel
//! types and Image Trace are built, which is the wasm32 build the web shell makes.

#![deny(clippy::unwrap_used, clippy::expect_used)]

#[cfg(feature = "engine")]
pub mod agent;
mod error;
#[cfg(feature = "engine")]
pub mod iopaint;
pub mod pixels;
pub mod vectorize;

pub use error::{AiError, Result};
pub use pixels::{Expand, Mask, Rgba8Image};
