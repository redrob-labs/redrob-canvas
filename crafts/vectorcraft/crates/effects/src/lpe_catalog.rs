//! The catalogue entries and dispatch of the Effect › Path Effects family ([`crate::lpe_geo`],
//! [`crate::lpe_draw`], [`crate::lpe_link`]), kept in one place so the catalogue in `lib.rs` and
//! [`crate::apply_one`] each add one line for the whole family.

use redrob_vector_geom::{PathData, Rect};
use serde_json::{Value, json};

use crate::lpe_draw::{self as draw, *};
use crate::lpe_geo::{self as geo, *};
use crate::lpe_link::{self as link, *};
use crate::{EffectInfo, GeomContext, Lengths};

/// The menu the family lives in.
pub const MENU: &[&str] = &["Effect", "Path Effects"];

/// Every effect id of the family, for tests and listings.
pub const ALL: [&str; 31] = [
    CLONE_ORIGINAL,
    ATTACH_PATH,
    BOUNDING_BOX,
    FILL_BETWEEN_MANY,
    FILL_BETWEEN_STROKES,
    POWER_CLIP,
    CONSTRUCT_GRID,
    ELLIPSE_5,
    ELLIPSE_FROM_POINTS,
    CIRCLE_3,
    CIRCLE_CENTER_RADIUS,
    LINE_SEGMENT,
    PARALLEL,
    PERPENDICULAR_BISECTOR,
    ANGLE_BISECTOR,
    TANGENT_TO_CURVE,
    TRANSFORM_2,
    SHOW_HANDLES,
    MEASURE_SEGMENTS,
    RULER,
    INTERPOLATE_POINTS,
    KNOT,
    VONKOCH,
    GEARS,
    EMBROIDERY_STITCH,
    STITCH_SUBPATHS,
    HATCHES,
    SKETCH,
    JOIN_TYPE,
    SIMPLIFY,
    SLICE,
];

/// Is `id` one of the family's effects?
pub fn is_path_effect(id: &str) -> bool {
    ALL.contains(&id)
}

/// The distance parameters of the family's effects.
pub fn lengths_of(id: &str) -> Option<Lengths> {
    let always = |always| Some(Lengths { always, absolute: &[] });
    match id {
        ATTACH_PATH => always(&["handleLength"]),
        PARALLEL => always(&["distance", "startLength", "endLength"]),
        PERPENDICULAR_BISECTOR => always(&["length"]),
        ANGLE_BISECTOR | TANGENT_TO_CURVE => always(&["startLength", "endLength"]),
        SHOW_HANDLES => always(&["nodeSize", "handleSize"]),
        MEASURE_SEGMENTS => always(&["offset", "fontSize", "textOffset"]),
        RULER => always(&["minorLength", "majorLength", "shift"]),
        KNOT => always(&["gapWidth"]),
        HATCHES => always(&["bend", "thickness"]),
        SKETCH => always(&["strokeLength", "displacement", "tremble"]),
        JOIN_TYPE => always(&["width"]),
        SIMPLIFY => always(&["threshold"]),
        SLICE => always(&["gap"]),
        _ => None,
    }
}

/// The family's catalogue entries.
pub fn entries() -> Vec<EffectInfo> {
    let g = |id: &'static str, label, params, defaults| EffectInfo {
        id,
        label,
        menu: MENU,
        params,
        defaults,
        raster: false,
        lengths: lengths_of(id).unwrap_or_default(),
    };
    vec![
        g(
            CLONE_ORIGINAL,
            "Clone Original…",
            "{source: object id (the path whose geometry this one shows; edits to it follow), copyStyle: bool (false; true also takes the original's fills and strokes, keeping this object's own effects)} a live copy of another path's geometry with a style of its own (Effect Options › Select Linked Object selects the original)",
            json!({"source": 0, "copyStyle": false}),
        ),
        g(
            ATTACH_PATH,
            "Attach Path…",
            "{source: object id (the path attached to), end: \"start\"|\"end\"|\"both\" (\"start\"; which end of this path is attached), startPosition: 0..1 (0; where along the linked path the start attaches, by length), endPosition: 0..1 (1; the end), curve: bool (true; the attached anchor's handle follows the linked path's tangent), handleLength: pt (0..10000, 20)} links an end of the path to a point on another path, following it as that path is edited",
            json!({"source": 0, "end": "start", "startPosition": 0.0, "endPosition": 1.0, "curve": true, "handleLength": 20.0}),
        ),
        g(
            BOUNDING_BOX,
            "Bounding Box…",
            "{source: object id (the object measured), visual: bool (false; true measures the strokes too)} replaces the path by a rectangle tracking the linked object's bounds",
            json!({"source": 0, "visual": false}),
        ),
        g(
            FILL_BETWEEN_MANY,
            "Fill Between Many…",
            "{sources: [object ids] (the paths enclosing the region, in order), method: \"original\"|\"bspline\"|\"spiro\" (\"original\"; how each linked path is read: as it is, or its anchors through a B-spline or a Spiro spline), autoReverse: bool (true; a linked path is reversed when its end meets the previous one), join: bool (true; the linked paths joined into one subpath), close: bool (true)} the path becomes the region the linked paths enclose, following them (Expand Appearance keeps the fill)",
            json!({"sources": [], "method": "original", "autoReverse": true, "join": true, "close": true}),
        ),
        g(
            FILL_BETWEEN_STROKES,
            "Fill Between Strokes…",
            "{source: object id (the second stroked path), source2?: object id (a path to use instead of this one as the first), reverseSecond: bool (true; the linked path runs back so two paths drawn the same way enclose a band), join: bool (true), close: bool (true)} fills the area between this path and a linked one",
            json!({"source": 0, "reverseSecond": true, "join": true, "close": true}),
        ),
        g(
            POWER_CLIP,
            "Power Clip…",
            "{inverse: bool (false; true shows everything but the clipping path, within 10000 pt of it), hideClip: bool (false; true hides the clipping path's own fill and stroke)} on a clip group's clipping path (Object › Clipping Mask › Power Clip puts it there): the clip stays editable and can be inverted; releasing the clip keeps the path",
            json!({"inverse": false, "hideClip": false}),
        ),
        g(
            CONSTRUCT_GRID,
            "Construct Grid…",
            "{rows: integer (1..500, 5), columns: integer (1..500, 5)} from a three-anchor path (an origin and the ends of its two axes) a grid of that many cells along those axes, skewed as they are",
            json!({"rows": 5, "columns": 5}),
        ),
        g(
            ELLIPSE_5,
            "Ellipse by 5 Points…",
            "{arc: bool (false; only the arc from the first point to the fifth), slice: bool (false; the arc closed through the centre), closeArc: bool (false; the arc closed by a chord), drawAxes: bool (false), drawFrame: bool (false; the bounding parallelogram)} the ellipse through the path's first five anchors",
            json!({"arc": false, "slice": false, "closeArc": false, "drawAxes": false, "drawFrame": false}),
        ),
        g(
            ELLIPSE_FROM_POINTS,
            "Ellipse from Points…",
            "{method: \"auto\"|\"circle\"|\"isometric\"|\"steiner\"|\"steinerIn\"|\"fivePoints\" (\"auto\"; auto: two anchors a circle on their diameter, three the circle through them, four the ellipse in their parallelogram, five the ellipse through them; circle: the best circle through them all; isometric: the ellipse in the parallelogram of the first three; steiner: the ellipse through the first three as a triangle's vertices; steinerIn: the one touching its sides; fivePoints: the conic through five), arc: bool (false; from the first anchor to the last), slice: bool (false; the arc closed through the centre), closeArc: bool (false; closed by a chord), drawAxes: bool (false), drawFrame: bool (false)} a circle or ellipse from 2 to 5 anchors",
            json!({"method": "auto", "arc": false, "slice": false, "closeArc": false, "drawAxes": false, "drawFrame": false}),
        ),
        g(
            CIRCLE_3,
            "Circle by 3 Points…",
            "{arc: bool (false; only the arc from the first point through the second to the third), slice: bool (false; closed through the centre), closeArc: bool (false; closed by a chord)} the circle through the path's first three anchors",
            json!({"arc": false, "slice": false, "closeArc": false}),
        ),
        g(CIRCLE_CENTER_RADIUS, "Circle by Center and Radius", "{} the circle centred on the first anchor through the second", json!({})),
        g(
            LINE_SEGMENT,
            "Line Segment…",
            "{type: \"closed\"|\"openStart\"|\"openEnd\"|\"infinite\" (\"closed\"; between the two anchors, or reaching 10000 pt past the start, the end, or both)} the straight line through the path's first two anchors",
            json!({"type": "closed"}),
        ),
        g(
            PARALLEL,
            "Parallel…",
            "{distance: pt (20; to the left of the segment, negative to the right), throughThird: bool (true; with a third anchor, the line passes through it instead), startLength: pt (0; past the start), endLength: pt (0; past the end)} a line parallel to the segment between the first two anchors",
            json!({"distance": 20.0, "throughThird": true, "startLength": 0.0, "endLength": 0.0}),
        ),
        g(
            PERPENDICULAR_BISECTOR,
            "Perpendicular Bisector…",
            "{length: pt (0..10000, 100; centred on the midpoint)} the perpendicular bisector of the segment between the first two anchors",
            json!({"length": 100.0}),
        ),
        g(
            ANGLE_BISECTOR,
            "Angle Bisector…",
            "{startLength: pt (0..10000, 0; behind the middle anchor), endLength: pt (0..10000, 100; into the angle)} from a three-anchor path, the bisector of the angle at the middle anchor",
            json!({"startLength": 0.0, "endLength": 100.0}),
        ),
        g(
            TANGENT_TO_CURVE,
            "Tangent to Curve…",
            "{position: 0..1 (0.5; where along the path, by length), angle: degrees (0; turned from the tangent), startLength: pt (0..10000, 100), endLength: pt (0..10000, 100), keepPath: bool (false; the path itself too)} the tangent line at a point on the curve",
            json!({"position": 0.5, "angle": 0.0, "startLength": 100.0, "endLength": 100.0, "keepPath": false}),
        ),
        g(
            TRANSFORM_2,
            "Transform by 2 Points…",
            "{from: \"x,y x,y\" (the two knots in box coordinates, 0,0 the top left and 1,1 the bottom right of the object's bounds; \"0,0.5 1,0.5\"), to: \"x,y x,y\" (where the knots go, in the same coordinates; \"0,0.5 1,0.5\"), elastic: bool (false; true stretches along the knots' axis only), lockLength: bool (false; the knots keep their distance: move and turn only), lockAngle: bool (false; the knots keep their direction: move and scale only)} moves, turns and scales the object so its two knots land on the two targets",
            json!({"from": "0,0.5 1,0.5", "to": "0,0.5 1,0.5", "elastic": false, "lockLength": false, "lockAngle": false}),
        ),
        g(
            SHOW_HANDLES,
            "Show Handles…",
            "{nodeSize: pt (0..100, 4; the squares at the anchors), handleSize: pt (0..100, 3; the circles at the handles), showNodes: bool (true), showHandles: bool (true), showPath: bool (true)} draws the path's anchors and handles as geometry, for tutorials",
            json!({"nodeSize": 4.0, "handleSize": 3.0, "showNodes": true, "showHandles": true, "showPath": true}),
        ),
        g(
            MEASURE_SEGMENTS,
            "Measure Segments…",
            "{unit: \"pt\"|\"px\"|\"mm\"|\"cm\"|\"in\"|\"pc\" (\"pt\"), precision: integer (0..8, 2), scale: % (100; the drawing scale the lengths are read at), offset: pt (10; the dimension line's distance from the segment, negative for the other side), textOffset: pt (2), fontSize: pt (8), arrows: bool (true), helperLines: bool (true; ticks from the segment's ends to the dimension line), arcLength: bool (false; a curve's length along it, not its chord), hideCollinear: bool (true; straight segments continuing each other measured as one), showUnit: bool (true), flip: bool (false; the other side), showPath: bool (true)} a dimension line with the length of every segment",
            json!({"unit": "pt", "precision": 2, "scale": 100.0, "offset": 10.0, "textOffset": 2.0, "fontSize": 8.0, "arrows": true, "helperLines": true, "arcLength": false, "hideCollinear": true, "showUnit": true, "flip": false, "showPath": true}),
        ),
        g(
            RULER,
            "Ruler…",
            "{markDistance: number (20; in unit), unit: \"pt\"|\"px\"|\"mm\"|\"cm\"|\"in\"|\"pc\" (\"pt\"), majorEvery: integer (1..1000, 5; every n-th mark is major), minorLength: pt (5), majorLength: pt (10), offset: number (0; in unit, before the first mark), direction: \"left\"|\"right\"|\"both\" (\"left\"; which side of the path the marks stand on), shift: pt (0; the marks moved across the path), border: bool (true; the path itself), endMarks: bool (true; a mark at the end of an open path)} tick marks along the path",
            json!({"markDistance": 20.0, "unit": "pt", "majorEvery": 5, "minorLength": 5.0, "majorLength": 10.0, "offset": 0.0, "direction": "left", "shift": 0.0, "border": true, "endMarks": true}),
        ),
        g(
            INTERPOLATE_POINTS,
            "Interpolate Points…",
            "{interpolator: \"cubicBezierFit\"|\"centripetalCatmullRom\"|\"spiro\"|\"linear\" (\"centripetalCatmullRom\")} a smooth curve through the path's anchors",
            json!({"interpolator": "centripetalCatmullRom"}),
        ),
        g(
            KNOT,
            "Knot…",
            "{gapWidth: pt (0..10000, 3), addStrokeWidth: bool (true; the gap grows by the stroke's weight), switcher: \"alternate\"|\"firstOver\"|\"secondOver\" (\"alternate\"; which strand goes over at each crossing), switch: [crossing numbers] (crossings, counted along the path from 1, whose over and under are swapped), inverse: bool (false; every crossing swapped)} interrupts the strand underneath where the path crosses itself, so it reads as a knot",
            json!({"gapWidth": 3.0, "addStrokeWidth": true, "switcher": "alternate", "switch": [], "inverse": false}),
        ),
        g(
            VONKOCH,
            "VonKoch…",
            "{iterations: integer (0..12, 3), reference: \"endpoints\"|\"firstSegment\" (\"endpoints\"; the generator's segment that stands for one segment of the figure), maxComplexity: integer (10..200000, 5000; the most anchors drawn), drawAll: bool (false; every generation), showGenerator: bool (false)} a fractal: every segment replaced by a scaled copy of the path, again and again",
            json!({"iterations": 3, "reference": "endpoints", "maxComplexity": 5000, "drawAll": false, "showGenerator": false}),
        ),
        g(
            GEARS,
            "Gears…",
            "{teeth: integer (3..500, 10; the first gear's), pressureAngle: degrees (5..45, 20)} the path's anchors become the centres of a chain of meshing gears, the first reaching halfway to the second",
            json!({"teeth": 10, "pressureAngle": 20.0}),
        ),
        g(
            EMBROIDERY_STITCH,
            "Embroidery Stitch…",
            "{ordering: \"asIs\"|\"nearest\" (\"asIs\"; nearest visits each next the nearest unvisited subpath, reversing it when that is shorter), connection: \"straight\"|\"curve\"|\"jump\" (\"straight\"; how consecutive subpaths are joined: a line, a curve following their tangents, or a jump without geometry)} connects the subpaths into one continuous stitching path",
            json!({"ordering": "asIs", "connection": "straight"}),
        ),
        g(
            STITCH_SUBPATHS,
            "Stitch Sub-Paths…",
            "{lines: integer (1..2000, 10; stitches between each pair of consecutive subpaths), startSpacing: 0..1 (1; how much of the first subpath the stitches spread over), endSpacing: 0..1 (1; of the second), startVariation: 0..1 (0), endVariation: 0..1 (0), pattern: \"x,y x,y …\" (the stitch's shape in a unit box, 0,0 its start and 1,0 its end; \"0,0 1,0\"), keepPaths: bool (true), seed: int (0)} connects consecutive subpaths by stitch lines shaped like a pattern",
            json!({"lines": 10, "startSpacing": 1.0, "endSpacing": 1.0, "startVariation": 0.0, "endVariation": 0.0, "pattern": "0,0 1,0", "keepPaths": true, "seed": 0}),
        ),
        g(
            HATCHES,
            "Hatches (Rough)…",
            "{angle: degrees (45), frequency: lines per 100 pt (0.1..1000, 10), growth: -0.9..10 (0; the spacing grows by this share from the first line to the last), bend: pt (0; how far each line bows), bendAngle: degrees (90; which way it bows), randomness: 0..1 (0.2), thickness: pt (1), thicknessVariation: 0..1 (0.3), clip: bool (true; the lines clipped to the shape), seed: int (0)} fills the shape with hand-drawn-looking hatch lines",
            json!({"angle": 45.0, "frequency": 10.0, "growth": 0.0, "bend": 0.0, "bendAngle": 90.0, "randomness": 0.2, "thickness": 1.0, "thicknessVariation": 0.3, "clip": true, "seed": 0}),
        ),
        g(
            SKETCH,
            "Sketch…",
            "{strokes: integer (1..20, 2; layers of strokes along the path), strokeLength: pt (60), overlap: 0..0.95 (0.3; of a stroke's length), displacement: pt (2; how far strokes are pushed off the path), tremble: pt (0.5), constructionLines: integer (0..1000, 3; tangent guide lines along the path), constructionLength: 0..100 (1; their length as a multiple of the stroke length), seed: int (0)} replaces the path by overlapping pencil-like strokes",
            json!({"strokes": 2, "strokeLength": 60.0, "overlap": 0.3, "displacement": 2.0, "tremble": 0.5, "constructionLines": 3, "constructionLength": 1.0, "seed": 0}),
        ),
        g(
            JOIN_TYPE,
            "Join Type…",
            "{width: pt (0; 0 takes the stroke's weight), join: \"bevel\"|\"round\"|\"miter\"|\"miterClip\"|\"extrapolated\"|\"extrapolatedAlt\"|\"extrapolatedSmooth\" (\"extrapolated\"; the extrapolated joins continue the incoming side's curvature as an arc, the outgoing side's, or a smooth curve between them), miterLimit: 1..500 (4), cap: \"butt\"|\"round\"|\"square\" (\"butt\")} the stroke as a filled outline with joins of extra kinds, its width editable live",
            json!({"width": 0.0, "join": "extrapolated", "miterLimit": 4.0, "cap": "butt"}),
        ),
        g(
            SIMPLIFY,
            "Simplify…",
            "{steps: integer (1..10, 1; passes), threshold: pt (0.5; how far the simplified path may stray; with separately off, a percentage of the object's size), smoothAngle: degrees (0..180, 30; anchors turning by more stay corners), straightLines: bool (false), separately: bool (true; each subpath fitted on its own)} live node reduction",
            json!({"steps": 1, "threshold": 0.5, "smoothAngle": 30.0, "straightLines": false, "separately": true}),
        ),
        g(
            SLICE,
            "Slice…",
            "{point: \"x,y\" (a point on the cutting line in box coordinates, 0,0 the top left and 1,1 the bottom right of the object's bounds; \"0.5,0.5\"), angle: degrees (90; the line's direction, counterclockwise from 3 o'clock), side: \"left\"|\"right\"|\"both\" (\"left\"; which half is kept), gap: pt (0; both halves pushed apart)} cuts the object along a line, live; Object › Path › Live Slice makes a pair of halves (opposite sides) that keep their own styles, and Fuse Slice joins them back",
            json!({"point": "0.5,0.5", "angle": 90.0, "side": "left", "gap": 0.0}),
        ),
    ]
}

/// Evaluate the family's effect `id` on `path` (`None` for an id outside the family).
pub fn apply(id: &str, p: &Value, path: &PathData, b: Rect, ctx: &GeomContext) -> Option<PathData> {
    Some(match id {
        CLONE_ORIGINAL => link::clone_original(path, p),
        ATTACH_PATH => link::attach_path(path, p),
        BOUNDING_BOX => link::bounding_box(path, p),
        FILL_BETWEEN_MANY => link::fill_between_many(path, p),
        FILL_BETWEEN_STROKES => link::fill_between_strokes(path, p),
        // Power Clip changes how a clipping path clips, not its geometry (`clip_outline`).
        POWER_CLIP => path.clone(),
        CONSTRUCT_GRID => geo::construct_grid(path, p),
        ELLIPSE_5 => geo::ellipse_by_five_points(path, p),
        ELLIPSE_FROM_POINTS => geo::ellipse_from_points(path, p),
        CIRCLE_3 => geo::circle_by_three_points(path, p),
        CIRCLE_CENTER_RADIUS => geo::circle_by_center_and_radius(path, p),
        LINE_SEGMENT => geo::line_segment(path, p),
        PARALLEL => geo::parallel(path, p),
        PERPENDICULAR_BISECTOR => geo::perpendicular_bisector(path, p),
        ANGLE_BISECTOR => geo::angle_bisector(path, p),
        TANGENT_TO_CURVE => geo::tangent_to_curve(path, p),
        TRANSFORM_2 => geo::transform_by_two_points(path, b, p),
        SHOW_HANDLES => draw::show_handles(path, p),
        MEASURE_SEGMENTS => draw::measure_segments(path, p),
        RULER => draw::ruler(path, p),
        INTERPOLATE_POINTS => draw::interpolate_points(path, p),
        KNOT => draw::knot(path, p, ctx.stroke),
        VONKOCH => draw::von_koch(path, p),
        GEARS => draw::gears(path, p),
        EMBROIDERY_STITCH => draw::embroidery_stitch(path, p),
        STITCH_SUBPATHS => draw::stitch_subpaths(path, p),
        HATCHES => draw::hatches(path, b, p, ctx.rule),
        SKETCH => draw::sketch(path, p),
        JOIN_TYPE => draw::join_type(path, p, ctx.stroke),
        SIMPLIFY => draw::simplify(path, b, p),
        SLICE => draw::slice(path, b, p, ctx.rule),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_is_dispatched_and_listed() {
        let path = PathData::single(redrob_vector_geom::SubPath::polyline(
            &[redrob_vector_geom::Point::new(0.0, 0.0), redrob_vector_geom::Point::new(50.0, 10.0), redrob_vector_geom::Point::new(100.0, 0.0)],
            false,
        ));
        let b = path.bounds().unwrap();
        for e in entries() {
            assert!(is_path_effect(e.id), "{}", e.id);
            assert!(e.menu == MENU);
            assert!(apply(e.id, &e.defaults, &path, b, &GeomContext::default()).is_some(), "{}", e.id);
            assert!(crate::effect_info(e.id).is_some(), "{} missing from the catalogue", e.id);
            assert!(crate::is_geometry(e.id), "{}", e.id);
        }
        assert_eq!(entries().len(), 31);
    }
}
