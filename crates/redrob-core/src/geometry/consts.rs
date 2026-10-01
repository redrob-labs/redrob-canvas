// SPDX-License-Identifier: GPL-3.0-or-later
//!
//! Ported from Graphite `node-graph/libraries/vector-types/src/vector/algorithms/consts.rs`
//! at commit d10ccbedac06497ee4766a7061d29281ee5df811, under Apache-2.0.
//! Apache section 4(b) notice: this file was changed on 2026-09-30.
//! CHANGED: reformatted to this project's rustfmt settings. Upstream uses hard tabs at
//! max_width 200; this project uses spaces at 100, and rustfmt's file-level `ignore` is
//! nightly-only so a carve-out was not available on stable. To diff against upstream
//! despite it, use `git diff --ignore-all-space` or re-run upstream's rustfmt.toml over a
//! copy before comparing.
//! CHANGED: 3 item(s) raised from `fn`/`pub(crate)` to `pub`. Upstream kept them
//! internal because its only callers were bezpath_algorithms.rs and shapes.rs in the
//! same crate, and those are not part of this tranche -- they reach into Graphite's
//! core-types. Here these functions ARE the value being ported, so they are the
//! crate's surface rather than a private detail.
//! Unchanged apart from this notice: it is three numeric tolerances.

/// Minimum allowable separation between adjacent `t` values when calculating curve intersections
pub const MIN_SEPARATION_VALUE: f64 = 5. * 1e-3;

/// Threshold for comparing floating point values in intersection and centroid math.
pub const MAX_ABSOLUTE_DIFFERENCE: f64 = 1e-3;

/// Maximum distance at which two points are treated as one and the same point.
pub const MAX_COINCIDENT_POINT_DISTANCE: f64 = 1e-7;
