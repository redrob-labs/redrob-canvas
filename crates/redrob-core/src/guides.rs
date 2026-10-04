// SPDX-License-Identifier: GPL-3.0-or-later

//! Guides, sample points and snapping (L.1).
//!
//! Three things that are not layers and not tools: infinite lines the canvas aligns to, stored
//! positions whose colour a user watches while editing, and the rule that pulls a coordinate onto
//! the nearest of them.
//!
//! Re-derived from `app/core/gimpguide.c`, `app/core/gimpsamplepoint.c`,
//! `app/core/gimpimage-guides.c` and `app/core/gimpimage-snap.c` (GPL-3.0-or-later), which this
//! repository pins and attributes in `docs/upstream-sources.toml`, plus
//! `libs/ui/canvas/kis_guides_config.cpp` for the lock, which that upstream has and the other does
//! not.
//!
//! # The snap rule is three lines and every one of them decides something
//!
//! `gimp_image_snap_distance` is `dist < MIN (epsilon, *mindist)`, and both halves are load-bearing:
//!
//! - the comparison is **strict**, so a candidate exactly `epsilon` away does NOT snap. `<=` would
//!   make the threshold inclusive and move the boundary by one pixel at every call site.
//! - it compares against `min(epsilon, mindist)`, not `epsilon` alone, so a second candidate at the
//!   SAME distance as the current best does not replace it. **Ties go to the candidate seen first**,
//!   and the iteration order of the guide list is therefore observable.
//!
//! And the whole call returns early, snapping nothing, when the coordinate is outside
//! `[-epsilon, extent + epsilon]`. That is what keeps a guide from reaching a point that is nowhere
//! near the canvas.
//!
//! # A custom guide is not a snapping target
//!
//! There, a guide carries a style, and `gimp_guide_is_custom` is `style != NORMAL`. The snap loop
//! `continue`s over every custom guide. Those are the lines a symmetry or split-view mode draws for
//! itself; they are in the list, they are drawn, and a tool must not be dragged onto them. An
//! implementation that snapped to every guide in the list would look correct until a symmetry mode
//! was switched on.
//!
//! # Lock is not snapping, and this is the one place the two upstreams differ
//!
//! `app/` has no guide lock at all. The other upstream stores `lockGuides` next to `showGuides` and
//! `snapToGuides` in the document, and gates INTERACTION on it:
//! `showGuides() && !lockGuides() && hasGuides()` attaches the canvas event filter, and guide
//! creation returns early under the lock. So lock stops a guide being added, moved or removed — and
//! says nothing about snapping, which keeps working on locked guides. Folding the two together is
//! the obvious mistake and would make locking a document's guides quietly disable its alignment.
//!
//! That expression also requires `showGuides` for canvas dragging, which is NOT mirrored here:
//! a hidden guide cannot be grabbed with a mouse, but that is a fact about mice, not about the
//! document. Hiding is display; locking is stored state with a meaning.

use serde::{Deserialize, Serialize};

/// Stable identity for a guide, independent of its position in the list.
pub type GuideId = uuid::Uuid;

/// Stable identity for a sample point.
pub type SamplePointId = uuid::Uuid;

/// Which axis a guide is fixed on.
///
/// Upstream's property is a three-valued orientation whose default is `UNKNOWN`, because a
/// `GObject` property needs a value before anyone sets one. Here a guide cannot be built without
/// its axis, so the unknown case is unrepresentable rather than guarded.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuideOrientation {
    /// Fixed at a `y`; spans the full width.
    Horizontal,
    /// Fixed at an `x`; spans the full height.
    Vertical,
}

/// What drew a guide, which is also whether tools may snap to it.
///
/// Read from upstream's `GimpGuideStyle`. Its fifth member, `NONE`, is the property default and
/// means "nobody set one"; every guide it constructs is `NORMAL` or one of the three below, so the
/// unset case is left out here for the same reason as [`GuideOrientation`]'s.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuideStyle {
    /// A guide the user placed. The only style that is a snapping target.
    #[default]
    Normal,
    /// Drawn by a mirror symmetry.
    Mirror,
    /// Drawn by a mandala symmetry.
    Mandala,
    /// Drawn by a split-view comparison.
    SplitView,
}

impl GuideStyle {
    /// Whether this style marks a guide as drawn by a mode rather than placed by the user.
    ///
    /// `style != Normal`, which is upstream's `gimp_guide_is_custom` exactly. A custom guide is
    /// skipped by snapping and is not undoable.
    pub fn is_custom(self) -> bool {
        self != GuideStyle::Normal
    }
}

/// An infinite line the canvas aligns to.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Guide {
    id: GuideId,
    orientation: GuideOrientation,
    /// The coordinate on the guide's fixed axis.
    ///
    /// Signed, and deliberately NOT clamped to the canvas: upstream's add and move paths validate
    /// nothing, so a guide outside the image is legal and survives a crop. Its own `UNDEFINED`
    /// sentinel (`G_MININT`) is what a REMOVED guide's position is set to, and a removed guide is
    /// dropped from the list here, so there is no sentinel to represent.
    position: i32,
    style: GuideStyle,
}

impl Guide {
    pub(crate) fn new(
        id: GuideId,
        orientation: GuideOrientation,
        position: i32,
        style: GuideStyle,
    ) -> Self {
        Self {
            id,
            orientation,
            position,
            style,
        }
    }

    pub fn id(&self) -> GuideId {
        self.id
    }

    pub fn orientation(&self) -> GuideOrientation {
        self.orientation
    }

    pub fn position(&self) -> i32 {
        self.position
    }

    pub fn style(&self) -> GuideStyle {
        self.style
    }

    /// Whether a mode drew this guide, so snapping must skip it.
    pub fn is_custom(&self) -> bool {
        self.style.is_custom()
    }

    pub(crate) fn set_position(&mut self, position: i32) {
        self.position = position;
    }
}

/// A stored position whose colour the user watches.
///
/// Both coordinates are signed for the same reason a guide's position is: upstream stores them as
/// ints with a `G_MININT` sentinel and validates nothing on add.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SamplePoint {
    id: SamplePointId,
    x: i32,
    y: i32,
}

impl SamplePoint {
    pub(crate) fn new(id: SamplePointId, x: i32, y: i32) -> Self {
        Self { id, x, y }
    }

    pub fn id(&self) -> SamplePointId {
        self.id
    }

    pub fn x(&self) -> i32 {
        self.x
    }

    pub fn y(&self) -> i32 {
        self.y
    }

    pub(crate) fn set_position(&mut self, x: i32, y: i32) {
        self.x = x;
        self.y = y;
    }
}

/// How this document's guides and sample points behave and are drawn.
///
/// The defaults are upstream's own, which are not uniform: `show-guides` and `snap-to-guides` are
/// `TRUE` there while `snap-to-canvas` is `FALSE`, and the lock is `false` in the upstream that has
/// one. Picking a single default for all of them would be tidier and wrong — a document that
/// snapped to its canvas edges out of the box would fight every freehand placement.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GuideSettings {
    /// Draw the guides. Display only; snapping does not consult it.
    pub show_guides: bool,
    /// Draw the sample points.
    pub show_sample_points: bool,
    /// Pull coordinates onto guides.
    pub snap_to_guides: bool,
    /// Pull coordinates onto the canvas edges, `0` and the extent.
    pub snap_to_canvas: bool,
    /// Refuse to add, move or remove a guide. Does not affect snapping — see the module note.
    pub lock_guides: bool,
}

impl Default for GuideSettings {
    fn default() -> Self {
        Self {
            show_guides: true,
            show_sample_points: true,
            snap_to_guides: true,
            snap_to_canvas: false,
            lock_guides: false,
        }
    }
}

/// Whether a candidate is the closest snapping target seen so far, and if so take it.
///
/// `dist < min(epsilon, mindist)`, upstream's `gimp_image_snap_distance` verbatim in behaviour.
/// Both the strictness and the `min` are observable: see the module note.
fn snap_distance(
    unsnapped: f64,
    nearest: f64,
    epsilon: f64,
    mindist: &mut f64,
    target: &mut f64,
) -> bool {
    let dist = (nearest - unsnapped).abs();

    if dist < epsilon.min(*mindist) {
        *mindist = dist;
        *target = nearest;
        return true;
    }

    false
}

/// One axis of the snap. `extent` is the canvas width for `Vertical`, its height for `Horizontal`.
///
/// Returns the snapped coordinate and whether anything snapped. The orientation names the guides
/// that participate: a vertical guide is fixed at an `x`, so it is what an `x` snaps to.
fn snap_axis(
    guides: &[Guide],
    axis: GuideOrientation,
    extent: f64,
    coordinate: f64,
    epsilon: f64,
    settings: &GuideSettings,
) -> (f64, bool) {
    let mut target = coordinate;
    let mut mindist = f64::MAX;
    let mut snapped = false;

    // Upstream forces the guide flag off when the list is empty, then returns early when no flag is
    // left. The second half is what makes "no snapping configured" cost nothing.
    let to_guides = settings.snap_to_guides && guides.iter().any(|guide| guide.orientation == axis);
    if !to_guides && !settings.snap_to_canvas {
        return (target, false);
    }

    // Outside the canvas by more than the threshold, nothing snaps at all.
    if coordinate < -epsilon || coordinate >= extent + epsilon {
        return (target, false);
    }

    if to_guides {
        for guide in guides {
            // A guide a symmetry drew is drawn but is not a target.
            if guide.is_custom() || guide.orientation != axis {
                continue;
            }
            snapped |= snap_distance(
                coordinate,
                f64::from(guide.position),
                epsilon,
                &mut mindist,
                &mut target,
            );
        }
    }

    if settings.snap_to_canvas {
        snapped |= snap_distance(coordinate, 0.0, epsilon, &mut mindist, &mut target);
        snapped |= snap_distance(coordinate, extent, epsilon, &mut mindist, &mut target);
    }

    (target, snapped)
}

/// Snap an `x` onto the nearest vertical guide or canvas edge within `epsilon`.
pub fn snap_x(
    guides: &[Guide],
    width: u32,
    x: f64,
    epsilon: f64,
    settings: &GuideSettings,
) -> (f64, bool) {
    snap_axis(
        guides,
        GuideOrientation::Vertical,
        f64::from(width),
        x,
        epsilon,
        settings,
    )
}

/// Snap a `y` onto the nearest horizontal guide or canvas edge within `epsilon`.
pub fn snap_y(
    guides: &[Guide],
    height: u32,
    y: f64,
    epsilon: f64,
    settings: &GuideSettings,
) -> (f64, bool) {
    snap_axis(
        guides,
        GuideOrientation::Horizontal,
        f64::from(height),
        y,
        epsilon,
        settings,
    )
}

/// Snap both axes independently.
///
/// Independently, not as a point: upstream's `snap_x` and `snap_y` each keep their own `mindist`,
/// so a coordinate can snap on one axis and stay free on the other. A true nearest-point search
/// would refuse the common case of dragging along one guide.
///
/// `canvas` is `(width, height)` and `epsilon` is `(epsilon_x, epsilon_y)`, paired rather than
/// spread into four scalars because the pairs are read together and a transposed call would be
/// silent otherwise.
pub fn snap_point(
    guides: &[Guide],
    canvas: (u32, u32),
    point: (f64, f64),
    epsilon: (f64, f64),
    settings: &GuideSettings,
) -> (f64, f64, bool) {
    let (tx, snapped_x) = snap_x(guides, canvas.0, point.0, epsilon.0, settings);
    let (ty, snapped_y) = snap_y(guides, canvas.1, point.1, epsilon.1, settings);
    (tx, ty, snapped_x || snapped_y)
}
