// SPDX-License-Identifier: GPL-3.0-or-later

//! Bounded, platform-independent rasterization for the intentionally small v2
//! semantic surface. Coordinates are quantized to 1/256 pixel and coverage is
//! evaluated on a fixed 4x4 grid. No platform painter, font lookup, GPU, or
//! locale-dependent text service is used.

use font8x8::{BASIC_FONTS, UnicodeFonts};

use crate::render::source_over;
use crate::{
    CoreError, FillRule, MAX_FONT_FAMILY_BYTES, MAX_FONT_ID_BYTES, MAX_PATH_COMMANDS_PER_PATH,
    MAX_TEXT_CONTENT_BYTES, MAX_VECTOR_PATHS, NodeContent, PathCommand, Pixel, Result, StrokeStyle,
    TextContent, VectorContent, VectorPath,
};

pub const FIXED_SCALE: i64 = 256;
pub const CUBIC_STEPS: usize = 16;
pub const MAX_SEMANTIC_COORDINATE: f32 = 1_048_576.0;
pub const MAX_SEMANTIC_SEGMENTS: usize = 1_000_000;
pub const MAX_SEMANTIC_SAMPLE_EDGE_VISITS: u64 = 256 * 1024 * 1024;
pub const MAX_TEXT_WORK: u64 = 64 * 1024 * 1024;
pub const EMBEDDED_FONT_ID: &str = "font8x8-basic-0.3.1";

const SAMPLE_OFFSETS: [i64; 4] = [32, 96, 160, 224];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Point {
    x: i64,
    y: i64,
}

#[derive(Clone, Copy, Debug)]
struct Edge {
    a: Point,
    b: Point,
}

#[derive(Debug)]
struct PreparedPath<'a> {
    path: &'a VectorPath,
    edges: Vec<Edge>,
    bounds: Option<(i64, i64, i64, i64)>,
}

pub(crate) fn quantize(value: f32) -> Result<i64> {
    if !value.is_finite() || value.abs() > MAX_SEMANTIC_COORDINATE {
        return Err(CoreError::InvalidSemanticGeometry);
    }
    Ok((f64::from(value) * FIXED_SCALE as f64).round() as i64)
}

pub(crate) fn validate_text(text: &TextContent) -> Result<()> {
    if text.text.len() > MAX_TEXT_CONTENT_BYTES {
        return Err(CoreError::DocumentLimitExceeded("text content bytes"));
    }
    if text.font_family.len() > MAX_FONT_FAMILY_BYTES {
        return Err(CoreError::DocumentLimitExceeded("font family bytes"));
    }
    if text.font_id.len() > MAX_FONT_ID_BYTES {
        return Err(CoreError::DocumentLimitExceeded("font id bytes"));
    }
    if text.font_id != EMBEDDED_FONT_ID
        || text.font_family.trim().is_empty()
        || !text.font_size.is_finite()
        || text.font_size <= 0.0
        || text.font_size > 4096.0
        || quantize(text.font_size)? <= 0
    {
        return Err(CoreError::InvalidSemanticStyle);
    }
    quantize(text.origin_x)?;
    quantize(text.origin_y)?;
    for character in text.text.chars() {
        if character != '\n' && !character.is_ascii_graphic() && character != ' ' {
            return Err(CoreError::UnsupportedTextGlyph(character));
        }
        if character != '\n' && BASIC_FONTS.get(character).is_none() {
            return Err(CoreError::UnsupportedTextGlyph(character));
        }
    }
    Ok(())
}

pub(crate) fn validate_vector(vector: &VectorContent) -> Result<()> {
    if vector.paths.len() > MAX_VECTOR_PATHS {
        return Err(CoreError::DocumentLimitExceeded("vector path count"));
    }
    for path in &vector.paths {
        if path.commands.len() > MAX_PATH_COMMANDS_PER_PATH {
            return Err(CoreError::DocumentLimitExceeded("path commands per path"));
        }
        validate_path(path)?;
    }
    Ok(())
}

fn validate_path(path: &VectorPath) -> Result<()> {
    path_edges(path).map(|_| ())
}

pub fn validate_semantic_content(content: &NodeContent, width: u32, height: u32) -> Result<()> {
    match content {
        NodeContent::Text { text } => validate_text(text)?,
        NodeContent::Vector { vector } => validate_vector(vector)?,
        _ => return Err(CoreError::UnsupportedNodeContent(content.kind())),
    }
    preflight(content, width, height)
}

pub(crate) fn preflight(content: &NodeContent, width: u32, height: u32) -> Result<()> {
    match content {
        NodeContent::Text { text } => preflight_text(text, width, height),
        NodeContent::Vector { vector } => {
            prepare_vector(vector, width, height)?;
            Ok(())
        }
        _ => Ok(()),
    }
}

pub(crate) fn rasterize(content: &NodeContent, width: u32, height: u32) -> Result<Vec<u8>> {
    preflight(content, width, height)?;
    let byte_len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(CoreError::DocumentLimitExceeded("semantic raster bytes"))?;
    let mut output = vec![0_u8; byte_len];
    match content {
        NodeContent::Text { text } => rasterize_text(text, width, height, &mut output)?,
        NodeContent::Vector { vector } => rasterize_vector(vector, width, height, &mut output)?,
        _ => return Err(CoreError::UnsupportedNodeContent(content.kind())),
    }
    Ok(output)
}

fn text_layout(text: &TextContent) -> Result<(i64, i64, i64, usize, usize)> {
    validate_text(text)?;
    let origin_x = quantize(text.origin_x)?;
    let origin_y = quantize(text.origin_y)?;
    let size = quantize(text.font_size)?;
    let mut columns = 0_usize;
    let mut max_columns = 0_usize;
    let mut lines = 1_usize;
    for character in text.text.chars() {
        if character == '\n' {
            max_columns = max_columns.max(columns);
            columns = 0;
            lines = lines
                .checked_add(1)
                .ok_or(CoreError::SemanticWorkLimitExceeded)?;
        } else {
            columns = columns
                .checked_add(1)
                .ok_or(CoreError::SemanticWorkLimitExceeded)?;
        }
    }
    max_columns = max_columns.max(columns);
    Ok((origin_x, origin_y, size, max_columns, lines))
}

fn clipped_fixed_bounds(
    min_x: i64,
    min_y: i64,
    max_x: i64,
    max_y: i64,
    width: u32,
    height: u32,
) -> Option<(u32, u32, u32, u32)> {
    let canvas_x = i64::from(width) * FIXED_SCALE;
    let canvas_y = i64::from(height) * FIXED_SCALE;
    let x0 = min_x.div_euclid(FIXED_SCALE).clamp(0, i64::from(width));
    let y0 = min_y.div_euclid(FIXED_SCALE).clamp(0, i64::from(height));
    let x1 = (max_x + FIXED_SCALE - 1)
        .div_euclid(FIXED_SCALE)
        .clamp(0, i64::from(width));
    let y1 = (max_y + FIXED_SCALE - 1)
        .div_euclid(FIXED_SCALE)
        .clamp(0, i64::from(height));
    if min_x >= canvas_x || min_y >= canvas_y || max_x <= 0 || max_y <= 0 || x0 >= x1 || y0 >= y1 {
        None
    } else {
        Some((x0 as u32, y0 as u32, x1 as u32, y1 as u32))
    }
}

fn preflight_text(text: &TextContent, width: u32, height: u32) -> Result<()> {
    let (origin_x, origin_y, size, columns, lines) = text_layout(text)?;
    let max_x = origin_x
        .checked_add(
            (columns as i64)
                .checked_mul(size)
                .ok_or(CoreError::SemanticWorkLimitExceeded)?,
        )
        .ok_or(CoreError::SemanticWorkLimitExceeded)?;
    let max_y = origin_y
        .checked_add(
            (lines as i64)
                .checked_mul(size)
                .ok_or(CoreError::SemanticWorkLimitExceeded)?,
        )
        .ok_or(CoreError::SemanticWorkLimitExceeded)?;
    let pixels = clipped_fixed_bounds(origin_x, origin_y, max_x, max_y, width, height)
        .map_or(0_u64, |(x0, y0, x1, y1)| {
            u64::from(x1 - x0) * u64::from(y1 - y0)
        });
    let work = pixels
        .checked_mul(16)
        .ok_or(CoreError::SemanticWorkLimitExceeded)?;
    if work > MAX_TEXT_WORK {
        return Err(CoreError::SemanticWorkLimitExceeded);
    }
    Ok(())
}

fn rasterize_text(text: &TextContent, width: u32, height: u32, output: &mut [u8]) -> Result<()> {
    let (origin_x, origin_y, size, columns, lines) = text_layout(text)?;
    let max_x = origin_x + columns as i64 * size;
    let max_y = origin_y + lines as i64 * size;
    let Some((x0, y0, x1, y1)) =
        clipped_fixed_bounds(origin_x, origin_y, max_x, max_y, width, height)
    else {
        return Ok(());
    };
    let rows = text.text.split('\n').collect::<Vec<_>>();
    for y in y0..y1 {
        for x in x0..x1 {
            let mut covered = 0_u8;
            for sample_y in SAMPLE_OFFSETS {
                for sample_x in SAMPLE_OFFSETS {
                    let px = i64::from(x) * FIXED_SCALE + sample_x;
                    let py = i64::from(y) * FIXED_SCALE + sample_y;
                    let local_x = px - origin_x;
                    let local_y = py - origin_y;
                    if local_x < 0 || local_y < 0 {
                        continue;
                    }
                    let row_index = (local_y / size) as usize;
                    let column_index = (local_x / size) as usize;
                    let Some(line) = rows.get(row_index) else {
                        continue;
                    };
                    let Some(character) =
                        line.as_bytes().get(column_index).copied().map(char::from)
                    else {
                        continue;
                    };
                    let cell_x = ((local_x % size) * 8 / size) as usize;
                    let cell_y = ((local_y % size) * 8 / size) as usize;
                    let glyph = BASIC_FONTS
                        .get(character)
                        .ok_or(CoreError::UnsupportedTextGlyph(character))?;
                    if glyph[cell_y] & (1 << cell_x) != 0 {
                        covered += 1;
                    }
                }
            }
            write_coverage(output, width, x, y, text.color, covered);
        }
    }
    Ok(())
}

fn prepare_vector<'a>(
    vector: &'a VectorContent,
    width: u32,
    height: u32,
) -> Result<Vec<PreparedPath<'a>>> {
    validate_vector(vector)?;
    let mut prepared = Vec::with_capacity(vector.paths.len());
    let mut total_segments = 0_usize;
    let mut total_visits = 0_u64;
    for path in &vector.paths {
        let edges = flatten(path)?;
        total_segments = total_segments
            .checked_add(edges.len())
            .ok_or(CoreError::SemanticWorkLimitExceeded)?;
        if total_segments > MAX_SEMANTIC_SEGMENTS {
            return Err(CoreError::SemanticWorkLimitExceeded);
        }
        let mut bounds = edge_bounds(&edges);
        if let (Some(bounds), Some(stroke)) = (&mut bounds, path.stroke) {
            let radius = (quantize(stroke.width)? + 1) / 2;
            bounds.0 -= radius;
            bounds.1 -= radius;
            bounds.2 += radius;
            bounds.3 += radius;
        }
        let pixels = bounds
            .and_then(|(a, b, c, d)| clipped_fixed_bounds(a, b, c, d, width, height))
            .map_or(0_u64, |(x0, y0, x1, y1)| {
                u64::from(x1 - x0) * u64::from(y1 - y0)
            });
        let edge_count =
            u64::try_from(edges.len()).map_err(|_| CoreError::SemanticWorkLimitExceeded)?;
        let paint_branches = u64::from(path.fill.is_some())
            .checked_add(u64::from(path.stroke.is_some()))
            .ok_or(CoreError::SemanticWorkLimitExceeded)?;
        let visits = pixels
            .checked_mul(16)
            .and_then(|value| value.checked_mul(edge_count))
            .and_then(|value| value.checked_mul(paint_branches))
            .ok_or(CoreError::SemanticWorkLimitExceeded)?;
        total_visits = total_visits
            .checked_add(visits)
            .ok_or(CoreError::SemanticWorkLimitExceeded)?;
        if total_visits > MAX_SEMANTIC_SAMPLE_EDGE_VISITS {
            return Err(CoreError::SemanticWorkLimitExceeded);
        }
        prepared.push(PreparedPath {
            path,
            edges,
            bounds,
        });
    }
    Ok(prepared)
}

fn flatten(path: &VectorPath) -> Result<Vec<Edge>> {
    path_edges(path)
}

fn path_edges(path: &VectorPath) -> Result<Vec<Edge>> {
    if path.fill.is_none() && path.stroke.is_none() {
        return Err(CoreError::InvalidSemanticStyle);
    }
    if let Some(StrokeStyle { width, .. }) = path.stroke
        && (!width.is_finite() || width <= 0.0 || width > 4096.0 || quantize(width)? <= 0)
    {
        return Err(CoreError::InvalidSemanticStyle);
    }

    let mut edges = Vec::new();
    let mut current: Option<Point> = None;
    let mut start: Option<Point> = None;
    let mut drawable = false;
    let mut closed = false;

    let close_filled_subpath = |edges: &mut Vec<Edge>,
                                current: Option<Point>,
                                start: Option<Point>,
                                drawable: bool|
     -> Result<()> {
        if path.fill.is_some() && drawable {
            let from = current.ok_or(CoreError::InvalidSemanticPath)?;
            let to = start.ok_or(CoreError::InvalidSemanticPath)?;
            push_nonzero_edge(edges, from, to)?;
        }
        Ok(())
    };

    for command in &path.commands {
        match *command {
            PathCommand::MoveTo { x, y } => {
                close_filled_subpath(&mut edges, current, start, drawable)?;
                let point = Point {
                    x: quantize(x)?,
                    y: quantize(y)?,
                };
                current = Some(point);
                start = Some(point);
                drawable = false;
                closed = false;
            }
            PathCommand::LineTo { x, y } => {
                if closed {
                    return Err(CoreError::InvalidSemanticPath);
                }
                let from = current.ok_or(CoreError::InvalidSemanticPath)?;
                let to = Point {
                    x: quantize(x)?,
                    y: quantize(y)?,
                };
                if from != to {
                    push_nonzero_edge(&mut edges, from, to)?;
                    drawable = true;
                }
                current = Some(to);
            }
            PathCommand::CubicTo {
                control1_x,
                control1_y,
                control2_x,
                control2_y,
                x,
                y,
            } => {
                if closed {
                    return Err(CoreError::InvalidSemanticPath);
                }
                let p0 = current.ok_or(CoreError::InvalidSemanticPath)?;
                let p1 = Point {
                    x: quantize(control1_x)?,
                    y: quantize(control1_y)?,
                };
                let p2 = Point {
                    x: quantize(control2_x)?,
                    y: quantize(control2_y)?,
                };
                let p3 = Point {
                    x: quantize(x)?,
                    y: quantize(y)?,
                };
                if p0 != p1 || p0 != p2 || p0 != p3 {
                    let before = edges.len();
                    let mut previous = p0;
                    for step in 1..=CUBIC_STEPS {
                        let point = cubic_point(p0, p1, p2, p3, step as i64);
                        push_nonzero_edge(&mut edges, previous, point)?;
                        previous = point;
                    }
                    drawable |= edges.len() != before;
                }
                current = Some(p3);
            }
            PathCommand::Close => {
                if closed || !drawable {
                    return Err(CoreError::InvalidSemanticPath);
                }
                let from = current.ok_or(CoreError::InvalidSemanticPath)?;
                let to = start.ok_or(CoreError::InvalidSemanticPath)?;
                // An explicit return-to-start followed by Close is valid; the
                // zero-length closing edge is a deterministic no-op.
                push_nonzero_edge(&mut edges, from, to)?;
                current = Some(to);
                closed = true;
            }
        }
    }
    if !closed {
        close_filled_subpath(&mut edges, current, start, drawable)?;
    }
    if edges.is_empty() || edges.len() > MAX_SEMANTIC_SEGMENTS {
        return Err(CoreError::InvalidSemanticPath);
    }
    Ok(edges)
}

fn push_nonzero_edge(edges: &mut Vec<Edge>, a: Point, b: Point) -> Result<()> {
    if a == b {
        return Ok(());
    }
    if edges.len() >= MAX_SEMANTIC_SEGMENTS {
        return Err(CoreError::SemanticWorkLimitExceeded);
    }
    edges.push(Edge { a, b });
    Ok(())
}

fn rounded_div(numerator: i128, denominator: i128) -> i64 {
    if numerator >= 0 {
        ((numerator + denominator / 2) / denominator) as i64
    } else {
        ((numerator - denominator / 2) / denominator) as i64
    }
}

fn cubic_point(p0: Point, p1: Point, p2: Point, p3: Point, step: i64) -> Point {
    let n = CUBIC_STEPS as i64;
    let inverse = n - step;
    let denominator = i128::from(n).pow(3);
    let coordinate = |a: i64, b: i64, c: i64, d: i64| {
        let numerator = i128::from(inverse.pow(3)) * i128::from(a)
            + 3 * i128::from(inverse.pow(2) * step) * i128::from(b)
            + 3 * i128::from(inverse * step.pow(2)) * i128::from(c)
            + i128::from(step.pow(3)) * i128::from(d);
        rounded_div(numerator, denominator)
    };
    Point {
        x: coordinate(p0.x, p1.x, p2.x, p3.x),
        y: coordinate(p0.y, p1.y, p2.y, p3.y),
    }
}

fn edge_bounds(edges: &[Edge]) -> Option<(i64, i64, i64, i64)> {
    let first = edges.first()?;
    let mut bounds = (first.a.x, first.a.y, first.a.x, first.a.y);
    for edge in edges {
        for point in [edge.a, edge.b] {
            bounds.0 = bounds.0.min(point.x);
            bounds.1 = bounds.1.min(point.y);
            bounds.2 = bounds.2.max(point.x);
            bounds.3 = bounds.3.max(point.y);
        }
    }
    Some(bounds)
}

fn rasterize_vector(
    vector: &VectorContent,
    width: u32,
    height: u32,
    output: &mut [u8],
) -> Result<()> {
    for prepared in prepare_vector(vector, width, height)? {
        let Some((min_x, min_y, max_x, max_y)) = prepared.bounds else {
            continue;
        };
        let Some((x0, y0, x1, y1)) =
            clipped_fixed_bounds(min_x, min_y, max_x, max_y, width, height)
        else {
            continue;
        };
        let stroke_radius = prepared
            .path
            .stroke
            .map(|stroke| (quantize(stroke.width).expect("validated") + 1) / 2);
        for y in y0..y1 {
            for x in x0..x1 {
                let mut fill_coverage = 0_u8;
                let mut stroke_coverage = 0_u8;
                for sample_y in SAMPLE_OFFSETS {
                    for sample_x in SAMPLE_OFFSETS {
                        let point = Point {
                            x: i64::from(x) * FIXED_SCALE + sample_x,
                            y: i64::from(y) * FIXED_SCALE + sample_y,
                        };
                        if prepared.path.fill.is_some()
                            && point_in_path(point, &prepared.edges, prepared.path.fill_rule)
                        {
                            fill_coverage += 1;
                        }
                        if let Some(radius) = stroke_radius
                            && prepared
                                .edges
                                .iter()
                                .any(|edge| point_near_edge(point, *edge, radius))
                        {
                            stroke_coverage += 1;
                        }
                    }
                }
                if let Some(fill) = prepared.path.fill {
                    write_coverage(output, width, x, y, fill, fill_coverage);
                }
                if let Some(stroke) = prepared.path.stroke {
                    write_coverage(output, width, x, y, stroke.color, stroke_coverage);
                }
            }
        }
    }
    Ok(())
}

fn point_in_path(point: Point, edges: &[Edge], rule: FillRule) -> bool {
    let mut winding = 0_i32;
    let mut odd = false;
    for edge in edges {
        let cross = i128::from(edge.b.x - edge.a.x) * i128::from(point.y - edge.a.y)
            - i128::from(edge.b.y - edge.a.y) * i128::from(point.x - edge.a.x);
        if edge.a.y <= point.y && point.y < edge.b.y && cross > 0 {
            winding += 1;
            odd = !odd;
        } else if edge.b.y <= point.y && point.y < edge.a.y && cross < 0 {
            winding -= 1;
            odd = !odd;
        }
    }
    match rule {
        FillRule::NonZero => winding != 0,
        FillRule::EvenOdd => odd,
    }
}

fn point_near_edge(point: Point, edge: Edge, radius: i64) -> bool {
    let dx = edge.b.x - edge.a.x;
    let dy = edge.b.y - edge.a.y;
    let px = point.x - edge.a.x;
    let py = point.y - edge.a.y;
    let length_squared = i128::from(dx) * i128::from(dx) + i128::from(dy) * i128::from(dy);
    let radius_squared = i128::from(radius) * i128::from(radius);
    if length_squared == 0 {
        return i128::from(px) * i128::from(px) + i128::from(py) * i128::from(py) <= radius_squared;
    }
    let projection = i128::from(px) * i128::from(dx) + i128::from(py) * i128::from(dy);
    if projection <= 0 {
        return i128::from(px) * i128::from(px) + i128::from(py) * i128::from(py) <= radius_squared;
    }
    if projection >= length_squared {
        let qx = point.x - edge.b.x;
        let qy = point.y - edge.b.y;
        return i128::from(qx) * i128::from(qx) + i128::from(qy) * i128::from(qy) <= radius_squared;
    }
    let cross = i128::from(dx) * i128::from(py) - i128::from(dy) * i128::from(px);
    cross * cross <= radius_squared * length_squared
}

fn write_coverage(output: &mut [u8], width: u32, x: u32, y: u32, color: Pixel, coverage: u8) {
    if coverage == 0 {
        return;
    }
    let offset = (y as usize * width as usize + x as usize) * 4;
    let source = Pixel::rgba(
        color.r,
        color.g,
        color.b,
        ((u16::from(color.a) * u16::from(coverage) + 8) / 16) as u8,
    );
    source_over(Pixel::from_slice(&output[offset..offset + 4]), source)
        .write_to(&mut output[offset..offset + 4]);
}
