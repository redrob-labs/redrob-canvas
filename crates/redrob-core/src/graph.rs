// SPDX-License-Identifier: GPL-3.0-or-later

//! Operation-graph model: a chain of image operations applied in order to one buffer, re-derived from
//! GEGL's node-graph idea (behaviour studied, no code copied). GEGL is a directed graph of operation
//! nodes passing buffers between them; the common authoring case — and all our tool surface needs —
//! is a LINEAR chain (source -> op -> op -> sink), so we model that: an ordered list of nodes, each a
//! pure buffer->buffer transform, evaluated front to back. A branching graph can always be flattened
//! to this form for a single output.
//!
//! Each node is one of our existing [`crate::Filter`]s (so the whole filter library is reusable as
//! graph operations) plus an opacity the node's result is blended back over its input with — the
//! "amount" dial GEGL exposes on most operations. This keeps the graph a thin orchestration layer
//! over the filters rather than a second implementation of them.

use serde::{Deserialize, Serialize};

use crate::command::Filter;

/// One node in a linear operation graph.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OpNode {
    /// The operation this node performs.
    pub filter: Filter,
    /// 0..=1: how strongly the node's output replaces its input (1 = fully, 0 = passthrough). Lets a
    /// graph dial an operation's strength the way GEGL's `opacity`/`value` properties do.
    #[serde(default = "one")]
    pub amount: f32,
    /// When false the node is skipped (a disabled graph node). Default true.
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn one() -> f32 {
    1.0
}
fn yes() -> bool {
    true
}

impl OpNode {
    pub fn new(filter: Filter) -> Self {
        Self {
            filter,
            amount: 1.0,
            enabled: true,
        }
    }

    pub fn is_valid(&self) -> bool {
        self.amount.is_finite() && (0.0..=1.0).contains(&self.amount)
    }
}

/// A linear operation graph: nodes evaluated in order.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OpGraph {
    pub nodes: Vec<OpNode>,
}

impl OpGraph {
    pub const MAX_NODES: usize = 64;

    pub fn is_valid(&self) -> bool {
        self.nodes.len() <= Self::MAX_NODES && self.nodes.iter().all(OpNode::is_valid)
    }
}
