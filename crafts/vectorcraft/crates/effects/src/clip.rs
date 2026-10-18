//! The region a clip group clips to ([`Node::clip_outline`]) with text outlined by the font engine
//! and several shapes united, so the renderer and the SVG and PDF writers clip alike. A clipping
//! path carrying Power Clip's inverse clips to everything but itself ([`crate::lpe_link`]).

use redrob_vector_doc::{Appearance, Node, NodeKind};
use redrob_vector_geom::{BezPath, FillRule, PathData};

/// The clipping path `clip` of a clip group as one path and fill rule: compound paths keep their
/// holes, even-odd paths their rule, text clips by its glyph outlines and a group by the union of
/// its members. `None` when there is nothing to clip by (the clipped art is then hidden).
pub fn clip_outline(clip: &Node) -> Option<(BezPath, FillRule)> {
    let (bp, rule) = clip.clip_outline(Some(&outline_text), &unite)?;
    // Power Clip's inverse: everything but the clipping path shows.
    if crate::lpe_link::power_clip_inverse(&clip.appearance.effects) {
        return Some((crate::lpe_link::inverse_region(&bp, rule).to_bezpath(), FillRule::NonZero));
    }
    Some((bp, rule))
}

/// Text as one path of its glyph outlines, in document space.
fn outline_text(n: &Node) -> Option<Node> {
    let NodeKind::Text(t) = &n.kind else { return None };
    let mut bp = redrob_vector_text::layout(redrob_vector_text::FontDb::global(), t).to_bezpath();
    bp.apply_affine(t.xf);
    Some(Node::path(n.id, PathData::from_bezpath(&bp), Appearance::default()))
}

fn unite(shapes: &[(BezPath, FillRule)]) -> BezPath {
    let paths: Vec<(PathData, FillRule)> = shapes.iter().map(|(bp, r)| (PathData::from_bezpath(bp), *r)).collect();
    let refs: Vec<(&PathData, FillRule)> = paths.iter().map(|(p, r)| (p, *r)).collect();
    redrob_vector_pathops::unite_all(&refs).to_bezpath()
}
