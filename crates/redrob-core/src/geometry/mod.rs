// SPDX-License-Identifier: GPL-3.0-or-later
//! Curve and point geometry ported from Graphite.
//!
//! # Provenance
//!
//! Ported from Graphite's `node-graph/libraries/vector-types` at commit
//! `d10ccbedac06497ee4766a7061d29281ee5df811`, under the Apache License 2.0 — which is
//! the option this product elects of Graphite's Apache-or-MIT offer, because Graphite
//! ships no `NOTICE` file so section 4(d) obliges nothing while section 3 grants an
//! express patent licence. Apache-2.0 into GPL-3.0-or-later is a permitted one-way
//! direction. See `UPSTREAM_NOTICES.md`.
//!
//! Per Apache section 4(b), every file here carries a notice stating that it was
//! changed and how.
//!
//! # Why this subset
//!
//! `docs/graphite-port-boundary.md` measured the boundary: this product is a layer
//! stack and Graphite is a node graph, so its graph crates are not portable without
//! replacing this product's document model. Its `libraries/` are a different matter —
//! all twelve have zero references to the graph — and `vector-types` is the largest
//! architecture-neutral block in either upstream at 10,588 lines.
//!
//! This is its first tranche: the five algorithm files that depend on nothing inside
//! Graphite beyond two conversion helpers, so they compile here with only `kurbo` and
//! `glam` added. A Bezier path is a Bezier path whether a graph node or a layer owns
//! it, which is what makes any of this portable at all.
//!
//! Not yet ported from the same crate: `bezpath_algorithms.rs`, `shapes.rs`,
//! `offset_bezpath.rs` and `merge_by_distance.rs`, which reach into Graphite's
//! `core-types`; and `gradient.rs` (3,057 lines), `vector_attributes.rs` and
//! `vector_modification.rs`, which carry its vector document representation.

pub mod bezpath_algorithms;
pub mod consts;
pub mod convert;
pub mod glam_ext;
pub mod intersection;
pub mod misc;
pub mod offset_bezpath;
pub mod poisson_disk;
pub mod polynomial;
pub mod shapes;
pub mod spline;
pub mod util;

pub use convert::{dvec2_to_point, point_to_dvec2};
