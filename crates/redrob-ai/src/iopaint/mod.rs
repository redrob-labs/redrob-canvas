// SPDX-License-Identifier: MIT OR Apache-2.0

//! The local IOPaint engine: lifecycle ([`Engine`]), HTTP API ([`Client`]), pixel transport
//! ([`transport`]) and model lists.

pub mod client;
pub mod engine;
pub mod models;
pub mod transport;

pub use client::{Client, InpaintOptions, PluginModels, ServerConfig};
pub use engine::{
    Device, Engine, EnginePaths, EngineState, InstallMethod, Plugin, batch_arguments, free_port,
    install_steps, pinned_packages, server_arguments, torch_index_url, torch_packages,
};
pub use models::{
    DEFAULT_FILL_MODEL, DEFAULT_REMOVE_BG_MODEL, DIFFUSION_MODELS, ERASE_MODELS, REMOVE_BG_MODELS,
    is_diffusion_model, is_known_model,
};
pub use transport::{Transport, TransportMode};
