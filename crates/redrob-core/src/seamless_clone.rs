// SPDX-License-Identifier: GPL-3.0-or-later

//! Seamless clone: a patch dropped somewhere else, with its boundary made to disappear (L.6).
//!
//! Re-derived from `app/tools/gimpseamlessclonetool.c` and
//! `app/tools/gimpseamlesscloneoptions.c` (GPL-3.0-or-later), pinned and attributed in
//! `docs/upstream-sources.toml`.
//!
//! # The backlog called this a Poisson blend and the source contradicts that
//!
//! This item was filed as "Seamless clone (Poisson blend)". The tool has exactly ONE option, and
//! its blurb is *"Maximal scale of refinement points to be used for the **interpolation mesh**"*
//! (`max-refine-scale`, `0, 50, 5`). **A Poisson blend has no mesh and no refinement points** — it
//! solves a sparse linear system over every pixel of the region. A refinement scale for an
//! interpolation mesh describes the other family of methods, the ones whose whole purpose is to
//! reach the same result WITHOUT the Poisson solve, by interpolating the boundary mismatch across
//! the interior. So the single readable parameter is positive evidence against the name this item
//! was filed under, and implementing a Poisson solve would have been implementing something the
//! source argues against.
//!
//! # What IS readable, and what is not
//!
//! The tool's graph is fully readable:
//!
//! - the destination drawable is the clone node's `input`;
//! - the pasted buffer is its `aux`;
//! - its output goes through `svg:dst-over`, whose `aux` is the destination again.
//!
//! The kernel is `gegl:seamless-clone`, and **GEGL is the one upstream not vendored here** — the
//! same single cause that has five filters parked and that made L.5's kernel ours. So the envelope
//! is read and the interpolation below is derived.
//!
//! One thing deliberately NOT claimed: the operand order of `svg:dst-over`. Which of `input` and
//! `aux` that SVG operator treats as source and which as destination lives in GEGL, so it cannot be
//! read here; the wiring above is recorded as wiring rather than turned into a compositing claim.
//!
//! # The derivation, and why it is better supported than a guess
//!
//! The construction is the one the readable parameter names: take the colour MISMATCH between the
//! destination and the patch along the region's boundary, interpolate that mismatch across the
//! interior, and add it to the patch. At the boundary the interpolated mismatch is exactly the real
//! mismatch, so the seam closes by construction rather than by tuning; deep inside, the patch keeps
//! its own detail and only its overall colour is carried toward the destination.
//!
//! The interpolation is **mean-value coordinates**, which this repository already ships and uses
//! for the cage transform — so the mechanism is reused rather than invented, exactly as L.5's
//! kernel reused the lazy brush's edge cost. `max_refine_scale` is what it says it is: the
//! refinement of the boundary mesh, here the number of samples taken along each edge.

use crate::{CoreError, Pixel, Result};

/// `max-refine-scale` read from `gimpseamlesscloneoptions.c`: `0, 50, 5`.
pub const SEAMLESS_CLONE_DEFAULT_REFINE_SCALE: u32 = 5;
pub const SEAMLESS_CLONE_MAX_REFINE_SCALE: u32 = 50;

/// Boundary samples per edge at a given refinement scale.
///
/// `0` is the coarsest mesh the parameter allows and must still describe a closed region, so it
/// gives one sample per edge — the four corners. Each step adds one more sample per edge, so the
/// top of the range (50) is 51 per edge.
pub fn samples_per_edge(max_refine_scale: u32) -> u32 {
    max_refine_scale + 1
}

/// The boundary polygon of a `width` x `height` region at `(ox, oy)`, walked clockwise.
///
/// Sampled at `samples_per_edge` points along each of the four edges, corners included once. The
/// polygon is in the destination's coordinates; the matching patch position is the same point minus
/// the offset, which is what lets one walk serve both images.
pub(crate) fn boundary_polygon(
    ox: f64,
    oy: f64,
    width: f64,
    height: f64,
    per_edge: u32,
) -> Vec<(f64, f64)> {
    let n = per_edge.max(1) as usize;
    let mut points = Vec::with_capacity(n * 4);
    // Each edge contributes its start corner plus `n - 1` interior samples, so a corner is never
    // emitted twice and the walk closes exactly.
    let edge = |from: (f64, f64), to: (f64, f64), out: &mut Vec<(f64, f64)>| {
        for step in 0..n {
            let t = step as f64 / n as f64;
            out.push((from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t));
        }
    };
    let tl = (ox, oy);
    let tr = (ox + width - 1.0, oy);
    let br = (ox + width - 1.0, oy + height - 1.0);
    let bl = (ox, oy + height - 1.0);
    edge(tl, tr, &mut points);
    edge(tr, br, &mut points);
    edge(br, bl, &mut points);
    edge(bl, tl, &mut points);
    points
}

/// The per-channel mismatch between the destination and the patch at one boundary point.
///
/// Signed and in f64 because the correction is added back later: clamping here would lose the
/// direction of a correction that has to travel across the interior.
pub(crate) fn boundary_mismatch(destination: Pixel, patch: Pixel) -> [f64; 3] {
    [
        f64::from(destination.r) - f64::from(patch.r),
        f64::from(destination.g) - f64::from(patch.g),
        f64::from(destination.b) - f64::from(patch.b),
    ]
}

/// Guard the mesh against a region whose interpolation would be unbounded work.
///
/// One mean-value solve runs over every boundary vertex, so the cost is
/// `interior pixels x boundary points`. A budget rather than a size limit, because the two trade
/// off: a large region with a coarse mesh is cheap and a small region with the finest mesh is too.
pub(crate) fn check_budget(interior: u64, boundary: u64, budget: u64) -> Result<()> {
    let work = interior
        .checked_mul(boundary)
        .ok_or(CoreError::DocumentLimitExceeded(
            "seamless clone interpolation",
        ))?;
    if work > budget {
        return Err(CoreError::DocumentLimitExceeded(
            "seamless clone interpolation",
        ));
    }
    Ok(())
}
