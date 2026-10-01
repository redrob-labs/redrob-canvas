// SPDX-License-Identifier: GPL-3.0-or-later
//!
//! Items extracted from Graphite `node-graph/libraries/vector-types/src/vector/misc.rs`
//! at commit d10ccbedac06497ee4766a7061d29281ee5df811, under Apache-2.0.
//! Apache section 4(b) notice: this file was changed on 2026-09-30.
//! CHANGED: extracted four items -- ArcType, SpiralType, PointSpacingType and
//! bezpath_from_anchors_and_handles, with their impls -- from a 769-line module. The rest of
//! that module carries Graphite's vector DOCUMENT representation, which is the node-graph
//! model this product does not have; see docs/graphite-port-boundary.md.
//! CHANGED: reformatted to this project's rustfmt settings. Use
//! `git diff --ignore-all-space` to compare against upstream.
//! CHANGED: Graphite's graph and UI metadata stripped from all three enums -- DynAny,
//! node_macro::ChoiceType, #[widget(Radio)], #[widget(Dropdown)] and the tsify cfg_attr.
//! Those describe how Graphite puts these choices in its NODE UI: #[widget(Radio)] is
//! literally a radio button. The enum variants are the behaviour and they are kept; the
//! presentation is rejected, the same call docs/krita-paintops-coverage.md records for
//! Krita's option widgets. serde becomes unconditional because this crate has no matching
//! feature flag.

use super::convert::dvec2_to_point;
use glam::DVec2;
use kurbo::BezPath;

#[repr(C)]
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum ArcType {
    /// Leaves the two ends of the arc unconnected.
    #[default]
    Open = 0,
    /// Connects the two ends of the arc with a straight line.
    Closed,
    /// Connects the two ends of the arc to its center, forming a wedge.
    PieSlice,
}

#[derive(
    Default, Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum SpiralType {
    #[default]
    Archimedean,
    Logarithmic,
}

#[repr(C)]
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum PointSpacingType {
    #[default]
    /// The desired spacing distance between points.
    Separation,
    /// The exact number of points to span the path.
    Quantity,
}

/// Stitches anchors into a path, emitting a cubic when both facing handles exist, a quadratic when only one does, and a line otherwise.
///
/// Each item is an anchor position paired with its incoming and outgoing handle positions, in absolute coordinates.
pub fn bezpath_from_anchors_and_handles(
    anchors: impl IntoIterator<Item = (DVec2, Option<DVec2>, Option<DVec2>)>,
    closed: bool,
) -> BezPath {
    let mut bezpath = BezPath::new();
    let mut anchors = anchors.into_iter();

    let Some((first_anchor, first_in_handle, first_out_handle)) = anchors.next() else {
        return bezpath;
    };
    bezpath.move_to(dvec2_to_point(first_anchor));
    let mut out_handle = first_out_handle;

    let connect_to = |bezpath: &mut BezPath,
                      out_handle: Option<DVec2>,
                      anchor: DVec2,
                      in_handle: Option<DVec2>| match (out_handle, in_handle) {
        (Some(handle_start), Some(handle_end)) => bezpath.curve_to(
            dvec2_to_point(handle_start),
            dvec2_to_point(handle_end),
            dvec2_to_point(anchor),
        ),
        (None, None) => bezpath.line_to(dvec2_to_point(anchor)),
        (None, Some(handle)) | (Some(handle), None) => {
            bezpath.quad_to(dvec2_to_point(handle), dvec2_to_point(anchor))
        }
    };

    for (anchor, in_handle, anchor_out_handle) in anchors {
        connect_to(&mut bezpath, out_handle, anchor, in_handle);
        out_handle = anchor_out_handle;
    }

    if closed {
        connect_to(&mut bezpath, out_handle, first_anchor, first_in_handle);
        bezpath.close_path();
    }

    bezpath
}
