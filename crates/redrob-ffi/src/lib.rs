// SPDX-License-Identifier: GPL-3.0-or-later

//! Stable C integration layer and Redrob graphics tool adapter.
//!
//! The C ABI is deliberately limited to opaque handles, fixed-layout scalar
//! structures, borrowed input byte spans, and explicitly freed output buffers.

mod abi;
mod graphics_tools;

pub use abi::*;
pub use graphics_tools::{
    GraphicsToolExecutor, ProposalAction, ProposalContext, ProposalDto, SharedEditor,
    graphics_tool_declarations, proposal_from_tool_call, proposal_tool_declarations,
};
