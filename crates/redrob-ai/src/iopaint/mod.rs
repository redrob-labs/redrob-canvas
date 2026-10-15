// SPDX-License-Identifier: MIT OR Apache-2.0

//! The local IOPaint engine: lifecycle ([`Engine`]), HTTP API ([`Client`]) and model lists.

pub mod client;
pub mod engine;
pub mod models;

pub use client::{Client, InpaintOptions, PluginModels, ServerConfig};
pub use engine::{
    Device, Engine, EnginePaths, EngineState, InstallMethod, Plugin, batch_arguments, free_port,
    install_steps, pinned_packages, server_arguments, torch_index_url, torch_packages,
};
pub use models::{DIFFUSION_MODELS, ERASE_MODELS, is_diffusion_model, is_known_model};
