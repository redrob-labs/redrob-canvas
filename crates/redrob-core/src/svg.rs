// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;

use base64::Engine;
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use svgtypes::{PathParser, PathSegment};

use crate::{
    BlendMode, Document, DocumentImportBuilder, EMBEDDED_FONT_ID, ExportOptions, FileFormat,
    FillRule, FormatError, FormatWarning, FrameId, ImportNode, ImportOptions, LossPolicy, NodeId,
    NodeKind, PathCommand, Pixel, RasterCel, Result, StrokeStyle, TextContent, VectorContent,
    VectorPath,
};

const MAX_SVG_XML_BYTES: usize = 16 * 1024 * 1024;
const MAX_SVG_ATTRIBUTES: usize = 64;
const REDROB_NAMESPACE: &str = "https://redrob.graphics/ns/1";

pub(crate) fn has_svg_root(bytes: &[u8]) -> bool {
    // Detection only needs a bounded prefix. Oversized SVG input must still route
    // to `xml_guard` so callers receive the typed SVG byte-limit error.
    let prefix = &bytes[..bytes.len().min(4_096)];
    let Ok(text) = std::str::from_utf8(prefix) else {
        return false;
    };
    let text = text.trim_start_matches('\u{feff}').trim_start();
    text.starts_with("<svg") || text.starts_with("<?xml") && text.contains("<svg")
}

fn xml_guard(bytes: &[u8]) -> Result<()> {
    if bytes.len() > MAX_SVG_XML_BYTES {
        return Err(FormatError::LimitExceeded("SVG XML bytes").into());
    }
    let lower = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    for forbidden in [
        "<!doctype",
        "<!entity",
        "<script",
        "<style",
        "<use",
        "<filter",
        "<clippath",
        "<mask",
        "<animate",
        "<set",
        "<lineargradient",
        "<radialgradient",
        "<pattern",
    ] {
        if lower.contains(forbidden) {
            return Err(FormatError::UnsupportedFeature("forbidden SVG construct").into());
        }
    }
    Ok(())
}

fn attrs(reader: &Reader<&[u8]>, start: &BytesStart<'_>) -> Result<HashMap<String, String>> {
    let mut values = HashMap::new();
    for attribute in start.attributes() {
        if values.len() >= MAX_SVG_ATTRIBUTES {
            return Err(FormatError::LimitExceeded("SVG attributes per element").into());
        }
        let attribute = attribute.map_err(|_| FormatError::Malformed("invalid SVG attribute"))?;
        let name = std::str::from_utf8(attribute.key.as_ref())
            .map_err(|_| FormatError::Malformed("non-UTF-8 SVG name"))?
            .to_owned();
        let value = attribute
            .decode_and_unescape_value(reader.decoder())
            .map_err(|_| FormatError::Malformed("invalid SVG attribute value"))?
            .into_owned();
        if values.insert(name, value).is_some() {
            return Err(FormatError::Malformed("duplicate SVG attribute").into());
        }
    }
    Ok(values)
}

fn parse_number(value: &str) -> Result<f32> {
    if value.is_empty()
        || value.contains(['e', 'E'])
        || value
            .bytes()
            .any(|byte| !(byte.is_ascii_digit() || matches!(byte, b'+' | b'-' | b'.')))
    {
        return Err(FormatError::Malformed("invalid SVG number").into());
    }
    let number = value
        .parse::<f32>()
        .map_err(|_| FormatError::Malformed("invalid SVG number"))?;
    if !number.is_finite() {
        return Err(FormatError::Malformed("non-finite SVG number").into());
    }
    Ok(number)
}

fn parse_dimension(value: &str) -> Result<u32> {
    let numeric = value.strip_suffix("px").unwrap_or(value);
    let number = parse_number(numeric)?;
    if number <= 0.0 || number.fract() != 0.0 || number > u32::MAX as f32 {
        return Err(FormatError::Malformed("SVG dimensions must be positive integer px").into());
    }
    Ok(number as u32)
}

fn parse_integer_coordinate(value: &str) -> Result<i32> {
    let number = parse_number(value)?;
    if number.fract() != 0.0
        || f64::from(number) < f64::from(i32::MIN)
        || f64::from(number) > f64::from(i32::MAX)
    {
        return Err(FormatError::UnsupportedFeature(
            "SVG raster offsets must be integer coordinates",
        )
        .into());
    }
    Ok(number as i32)
}

fn parse_opacity(value: Option<String>) -> Result<f32> {
    let value = value.map_or(Ok(1.0), |value| parse_number(&value))?;
    if !(0.0..=1.0).contains(&value) {
        return Err(FormatError::Malformed("invalid SVG opacity").into());
    }
    Ok(value)
}

fn fold_alpha(alpha: u8, opacity: f32) -> u8 {
    (f32::from(alpha) * opacity).round().clamp(0.0, 255.0) as u8
}

fn parse_color(value: &str) -> Result<Option<Pixel>> {
    if value == "none" {
        return Ok(None);
    }
    let hex = value
        .strip_prefix('#')
        .ok_or(FormatError::UnsupportedFeature(
            "only literal hexadecimal SVG colors are supported",
        ))?;
    let expand = |value: u8| value * 17;
    let byte = |pair: &str| {
        u8::from_str_radix(pair, 16).map_err(|_| FormatError::Malformed("invalid SVG color"))
    };
    let pixel = match hex.len() {
        3 => Pixel::rgba(
            expand(byte(&hex[0..1])?),
            expand(byte(&hex[1..2])?),
            expand(byte(&hex[2..3])?),
            255,
        ),
        4 => Pixel::rgba(
            expand(byte(&hex[0..1])?),
            expand(byte(&hex[1..2])?),
            expand(byte(&hex[2..3])?),
            expand(byte(&hex[3..4])?),
        ),
        6 => Pixel::rgba(byte(&hex[0..2])?, byte(&hex[2..4])?, byte(&hex[4..6])?, 255),
        8 => Pixel::rgba(
            byte(&hex[0..2])?,
            byte(&hex[2..4])?,
            byte(&hex[4..6])?,
            byte(&hex[6..8])?,
        ),
        _ => return Err(FormatError::Malformed("invalid SVG color length").into()),
    };
    Ok(Some(pixel))
}

fn parse_blend(value: Option<String>) -> Result<BlendMode> {
    match value.as_deref().unwrap_or("normal") {
        "normal" => Ok(BlendMode::Normal),
        "multiply" => Ok(BlendMode::Multiply),
        "screen" => Ok(BlendMode::Screen),
        "overlay" => Ok(BlendMode::Overlay),
        "plus" => Ok(BlendMode::Add),
        _ => Err(FormatError::UnsupportedFeature("unknown SVG blend mode").into()),
    }
}

#[derive(Debug)]
struct Common {
    name: String,
    visible: bool,
    opacity: f32,
    blend: BlendMode,
}

fn common(
    values: &mut HashMap<String, String>,
    fallback: &str,
    redrob_namespace: bool,
) -> Result<Common> {
    if values.keys().any(|key| key.starts_with("redrob:")) && !redrob_namespace {
        return Err(FormatError::Malformed("undeclared Redrob SVG namespace").into());
    }
    if values.contains_key("style")
        || values.contains_key("transform")
        || values.contains_key("class")
        || values.keys().any(|key| key.starts_with("on"))
    {
        return Err(FormatError::UnsupportedFeature("SVG CSS, transforms, and handlers").into());
    }
    let name = values
        .remove("redrob:name")
        .or_else(|| values.remove("id"))
        .unwrap_or_else(|| fallback.to_owned());
    let visible = match values.remove("visibility").as_deref().unwrap_or("visible") {
        "visible" => true,
        "hidden" => false,
        _ => return Err(FormatError::Malformed("invalid SVG visibility").into()),
    };
    if values.remove("display").as_deref() == Some("none") {
        return Err(
            FormatError::UnsupportedFeature("SVG display:none is not in the subset").into(),
        );
    }
    Ok(Common {
        name,
        visible,
        opacity: parse_opacity(values.remove("opacity"))?,
        blend: parse_blend(values.remove("redrob:blend"))?,
    })
}

fn styled_path(
    values: &mut HashMap<String, String>,
    commands: Vec<PathCommand>,
) -> Result<VectorContent> {
    let fill_opacity = parse_opacity(values.remove("fill-opacity"))?;
    let stroke_opacity = parse_opacity(values.remove("stroke-opacity"))?;
    let mut fill = parse_color(values.remove("fill").as_deref().unwrap_or("#000000"))?;
    if let Some(color) = &mut fill {
        color.a = fold_alpha(color.a, fill_opacity);
    }
    let mut stroke_color = values
        .remove("stroke")
        .map(|value| parse_color(&value))
        .transpose()?
        .flatten();
    if let Some(color) = &mut stroke_color {
        color.a = fold_alpha(color.a, stroke_opacity);
    }
    let stroke_width = values
        .remove("stroke-width")
        .map(|value| parse_number(value.strip_suffix("px").unwrap_or(&value)))
        .transpose()?
        .unwrap_or(1.0);
    let fill_rule = match values.remove("fill-rule").as_deref().unwrap_or("nonzero") {
        "nonzero" => FillRule::NonZero,
        "evenodd" => FillRule::EvenOdd,
        _ => return Err(FormatError::UnsupportedFeature("unknown SVG fill-rule").into()),
    };
    for unsupported in [
        "stroke-dasharray",
        "stroke-linecap",
        "stroke-linejoin",
        "vector-effect",
    ] {
        if values.contains_key(unsupported) {
            return Err(FormatError::UnsupportedFeature("unsupported SVG stroke semantics").into());
        }
    }
    if !values.is_empty() {
        return Err(FormatError::UnsupportedFeature("unknown SVG shape attribute").into());
    }
    Ok(VectorContent {
        paths: vec![VectorPath {
            commands,
            fill,
            stroke: stroke_color.map(|color| StrokeStyle {
                color,
                width: stroke_width,
            }),
            fill_rule,
        }],
    })
}

fn parse_path(data: &str) -> Result<Vec<PathCommand>> {
    let mut output = Vec::new();
    let (mut x, mut y) = (0.0_f64, 0.0_f64);
    let (mut sub_x, mut sub_y) = (0.0_f64, 0.0_f64);
    let mut previous_control = None::<(f64, f64)>;
    for segment in PathParser::from(data) {
        let segment = segment.map_err(|_| FormatError::Malformed("invalid SVG path data"))?;
        if output.len() >= crate::MAX_PATH_COMMANDS_PER_PATH {
            return Err(FormatError::LimitExceeded("SVG path commands").into());
        }
        match segment {
            PathSegment::MoveTo { abs, x: nx, y: ny } => {
                if abs {
                    x = nx;
                    y = ny;
                } else {
                    x += nx;
                    y += ny;
                }
                sub_x = x;
                sub_y = y;
                output.push(PathCommand::MoveTo {
                    x: x as f32,
                    y: y as f32,
                });
                previous_control = None;
            }
            PathSegment::LineTo { abs, x: nx, y: ny } => {
                if abs {
                    x = nx;
                    y = ny;
                } else {
                    x += nx;
                    y += ny;
                }
                output.push(PathCommand::LineTo {
                    x: x as f32,
                    y: y as f32,
                });
                previous_control = None;
            }
            PathSegment::HorizontalLineTo { abs, x: nx } => {
                x = if abs { nx } else { x + nx };
                output.push(PathCommand::LineTo {
                    x: x as f32,
                    y: y as f32,
                });
                previous_control = None;
            }
            PathSegment::VerticalLineTo { abs, y: ny } => {
                y = if abs { ny } else { y + ny };
                output.push(PathCommand::LineTo {
                    x: x as f32,
                    y: y as f32,
                });
                previous_control = None;
            }
            PathSegment::CurveTo {
                abs,
                x1,
                y1,
                x2,
                y2,
                x: nx,
                y: ny,
            } => {
                let (x1, y1, x2, y2, nx, ny) = if abs {
                    (x1, y1, x2, y2, nx, ny)
                } else {
                    (x + x1, y + y1, x + x2, y + y2, x + nx, y + ny)
                };
                output.push(PathCommand::CubicTo {
                    control1_x: x1 as f32,
                    control1_y: y1 as f32,
                    control2_x: x2 as f32,
                    control2_y: y2 as f32,
                    x: nx as f32,
                    y: ny as f32,
                });
                x = nx;
                y = ny;
                previous_control = Some((x2, y2));
            }
            PathSegment::SmoothCurveTo {
                abs,
                x2,
                y2,
                x: nx,
                y: ny,
            } => {
                let (control1_x, control1_y) = previous_control
                    .map_or((x, y), |(previous_x, previous_y)| {
                        (2.0 * x - previous_x, 2.0 * y - previous_y)
                    });
                let (x2, y2, nx, ny) = if abs {
                    (x2, y2, nx, ny)
                } else {
                    (x + x2, y + y2, x + nx, y + ny)
                };
                output.push(PathCommand::CubicTo {
                    control1_x: control1_x as f32,
                    control1_y: control1_y as f32,
                    control2_x: x2 as f32,
                    control2_y: y2 as f32,
                    x: nx as f32,
                    y: ny as f32,
                });
                x = nx;
                y = ny;
                previous_control = Some((x2, y2));
            }
            PathSegment::ClosePath { .. } => {
                output.push(PathCommand::Close);
                x = sub_x;
                y = sub_y;
                previous_control = None;
            }
            _ => {
                return Err(FormatError::UnsupportedFeature("unsupported SVG path command").into());
            }
        }
    }
    Ok(output)
}

fn points(value: &str, close: bool) -> Result<Vec<PathCommand>> {
    let numbers = value
        .split(|character: char| character.is_ascii_whitespace() || character == ',')
        .filter(|value| !value.is_empty())
        .map(parse_number)
        .collect::<Result<Vec<_>>>()?;
    if numbers.len() < 4 || numbers.len() % 2 != 0 {
        return Err(FormatError::Malformed("invalid SVG points").into());
    }
    let mut commands = vec![PathCommand::MoveTo {
        x: numbers[0],
        y: numbers[1],
    }];
    for pair in numbers[2..].chunks_exact(2) {
        commands.push(PathCommand::LineTo {
            x: pair[0],
            y: pair[1],
        });
    }
    if close {
        commands.push(PathCommand::Close);
    }
    Ok(commands)
}

fn shape(
    name: &[u8],
    mut values: HashMap<String, String>,
    parent: Option<NodeId>,
    redrob_namespace: bool,
) -> Result<ImportNode> {
    let fallback = std::str::from_utf8(name).unwrap_or("Shape");
    let common = common(&mut values, fallback, redrob_namespace)?;
    let commands = match name {
        b"path" => parse_path(
            &values
                .remove("d")
                .ok_or(FormatError::Malformed("missing SVG path data"))?,
        )?,
        b"rect" => {
            if values.contains_key("rx") || values.contains_key("ry") {
                return Err(FormatError::UnsupportedFeature("rounded SVG rectangles").into());
            }
            let x = values
                .remove("x")
                .map_or(Ok(0.0), |value| parse_number(&value))?;
            let y = values
                .remove("y")
                .map_or(Ok(0.0), |value| parse_number(&value))?;
            let width = parse_number(
                &values
                    .remove("width")
                    .ok_or(FormatError::Malformed("missing rect width"))?,
            )?;
            let height = parse_number(
                &values
                    .remove("height")
                    .ok_or(FormatError::Malformed("missing rect height"))?,
            )?;
            if width <= 0.0 || height <= 0.0 {
                return Err(FormatError::Malformed("SVG rect dimensions must be positive").into());
            }
            vec![
                PathCommand::MoveTo { x, y },
                PathCommand::LineTo { x: x + width, y },
                PathCommand::LineTo {
                    x: x + width,
                    y: y + height,
                },
                PathCommand::LineTo { x, y: y + height },
                PathCommand::Close,
            ]
        }
        b"line" => {
            let x1 = parse_number(&values.remove("x1").unwrap_or_else(|| "0".into()))?;
            let y1 = parse_number(&values.remove("y1").unwrap_or_else(|| "0".into()))?;
            let x2 = parse_number(&values.remove("x2").unwrap_or_else(|| "0".into()))?;
            let y2 = parse_number(&values.remove("y2").unwrap_or_else(|| "0".into()))?;
            values.entry("fill".into()).or_insert_with(|| "none".into());
            vec![
                PathCommand::MoveTo { x: x1, y: y1 },
                PathCommand::LineTo { x: x2, y: y2 },
            ]
        }
        b"polyline" | b"polygon" => points(
            &values
                .remove("points")
                .ok_or(FormatError::Malformed("missing SVG points"))?,
            name == b"polygon",
        )?,
        _ => return Err(FormatError::UnsupportedFeature("unknown SVG shape").into()),
    };
    Ok(
        ImportNode::vector(common.name, styled_path(&mut values, commands)?)
            .with_parent(parent)
            .with_visibility(common.visible)
            .with_opacity(common.opacity)
            .with_blend_mode(common.blend),
    )
}

fn place_png(
    source: &[u8],
    source_width: u32,
    source_height: u32,
    width: u32,
    height: u32,
    x: i32,
    y: i32,
) -> Vec<u8> {
    let mut output = vec![0; width as usize * height as usize * 4];
    for sy in 0..source_height {
        for sx in 0..source_width {
            let dx = i64::from(x) + i64::from(sx);
            let dy = i64::from(y) + i64::from(sy);
            if (0..i64::from(width)).contains(&dx) && (0..i64::from(height)).contains(&dy) {
                let from = (sy as usize * source_width as usize + sx as usize) * 4;
                let to = (dy as usize * width as usize + dx as usize) * 4;
                output[to..to + 4].copy_from_slice(&source[from..from + 4]);
            }
        }
    }
    output
}

struct TextDraft {
    values: HashMap<String, String>,
    text: String,
    parent: Option<NodeId>,
}

pub(crate) fn import_svg(
    bytes: &[u8],
    options: &ImportOptions,
) -> Result<(Document, Vec<FormatWarning>)> {
    xml_guard(bytes)?;
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(false);
    let mut buffer = Vec::new();
    let mut builder = None::<DocumentImportBuilder>;
    let mut dimensions = None;
    let mut parent_stack = Vec::<NodeId>::new();
    let mut text = None::<TextDraft>;
    let mut warnings = Vec::new();
    let mut saw_root = false;
    let mut root_closed = false;
    let mut redrob_namespace = false;
    loop {
        match reader
            .read_event_into(&mut buffer)
            .map_err(|_| FormatError::Malformed("invalid SVG XML"))?
        {
            Event::Decl(_) => {}
            Event::Start(start) if start.name().as_ref() == b"svg" => {
                if saw_root {
                    return Err(FormatError::Malformed("multiple SVG roots").into());
                }
                saw_root = true;
                let mut values = attrs(&reader, &start)?;
                let width = parse_dimension(
                    &values
                        .remove("width")
                        .ok_or(FormatError::Malformed("missing SVG width"))?,
                )?;
                let height = parse_dimension(
                    &values
                        .remove("height")
                        .ok_or(FormatError::Malformed("missing SVG height"))?,
                )?;
                if let Some(view_box) = values.remove("viewBox") {
                    let parts = view_box
                        .split_ascii_whitespace()
                        .map(parse_number)
                        .collect::<Result<Vec<_>>>()?;
                    if parts != [0.0, 0.0, width as f32, height as f32] {
                        return Err(FormatError::UnsupportedFeature(
                            "SVG viewBox must exactly match the canvas",
                        )
                        .into());
                    }
                }
                if values.remove("xmlns").as_deref() != Some("http://www.w3.org/2000/svg") {
                    return Err(FormatError::Malformed("missing SVG namespace").into());
                }
                if let Some(namespace) = values.remove("xmlns:redrob") {
                    if namespace != REDROB_NAMESPACE {
                        return Err(FormatError::Malformed("wrong Redrob SVG namespace").into());
                    }
                    redrob_namespace = true;
                }
                if !values.is_empty() {
                    return Err(
                        FormatError::UnsupportedFeature("unknown SVG root attribute").into(),
                    );
                }
                dimensions = Some((width, height));
                builder = Some(DocumentImportBuilder::new(width, height)?);
            }
            Event::Start(start) if start.name().as_ref() == b"g" && !root_closed => {
                let mut values = attrs(&reader, &start)?;
                let common = common(&mut values, "Group", redrob_namespace)?;
                if !values.is_empty() {
                    return Err(
                        FormatError::UnsupportedFeature("unknown SVG group attribute").into(),
                    );
                }
                let node = ImportNode::group(common.name)
                    .with_parent(parent_stack.last().copied())
                    .with_visibility(common.visible)
                    .with_opacity(common.opacity)
                    .with_blend_mode(common.blend);
                let id = node.id();
                builder
                    .as_mut()
                    .ok_or(FormatError::Malformed("SVG element before root"))?
                    .push_node(node)?;
                parent_stack.push(id);
            }
            Event::Empty(start)
                if matches!(
                    start.name().as_ref(),
                    b"path" | b"rect" | b"line" | b"polyline" | b"polygon"
                ) && !root_closed =>
            {
                let node = shape(
                    start.name().as_ref(),
                    attrs(&reader, &start)?,
                    parent_stack.last().copied(),
                    redrob_namespace,
                )?;
                builder
                    .as_mut()
                    .ok_or(FormatError::Malformed("SVG shape before root"))?
                    .push_node(node)?;
            }
            Event::Start(start) if start.name().as_ref() == b"text" && !root_closed => {
                if !redrob_namespace {
                    return Err(FormatError::Malformed("undeclared Redrob SVG namespace").into());
                }
                let values = attrs(&reader, &start)?;
                if values.get("redrob:kind").map(String::as_str) != Some("font8x8") {
                    return Err(FormatError::UnsupportedFeature("generic SVG text").into());
                }
                text = Some(TextDraft {
                    values,
                    text: String::new(),
                    parent: parent_stack.last().copied(),
                });
            }
            Event::Text(value) if text.is_some() => {
                let value = value
                    .decode()
                    .map_err(|_| FormatError::Malformed("invalid SVG text"))?;
                let value = quick_xml::escape::unescape(&value)
                    .map_err(|_| FormatError::Malformed("invalid SVG text entity"))?;
                text.as_mut().unwrap().text.push_str(&value);
                if text.as_ref().unwrap().text.len() > crate::MAX_TEXT_CONTENT_BYTES {
                    return Err(FormatError::LimitExceeded("SVG text bytes").into());
                }
            }
            Event::GeneralRef(reference) if text.is_some() => {
                let decoded = match reference.as_ref() {
                    b"lt" => '<',
                    b"gt" => '>',
                    b"amp" => '&',
                    b"quot" => '"',
                    b"apos" => '\'',
                    _ => {
                        return Err(FormatError::UnsupportedFeature(
                            "unsupported SVG entity reference",
                        )
                        .into());
                    }
                };
                text.as_mut().unwrap().text.push(decoded);
            }
            Event::End(end) if end.name().as_ref() == b"text" => {
                let TextDraft {
                    mut values,
                    text: content,
                    parent,
                } = text
                    .take()
                    .ok_or(FormatError::Malformed("unbalanced SVG text"))?;
                values.remove("redrob:kind");
                let common = common(&mut values, "Text", redrob_namespace)?;
                let x = parse_number(&values.remove("x").unwrap_or_else(|| "0".into()))?;
                let y = parse_number(&values.remove("y").unwrap_or_else(|| "0".into()))?;
                let font_size = parse_number(
                    values
                        .remove("font-size")
                        .ok_or(FormatError::Malformed("missing SVG text font-size"))?
                        .trim_end_matches("px"),
                )?;
                let color = parse_color(
                    &values
                        .remove("fill")
                        .ok_or(FormatError::Malformed("missing SVG text fill"))?,
                )?
                .ok_or(FormatError::Malformed("SVG text fill cannot be none"))?;
                let family = values
                    .remove("font-family")
                    .unwrap_or_else(|| "font8x8".into());
                let font_id = values
                    .remove("redrob:font-id")
                    .unwrap_or_else(|| EMBEDDED_FONT_ID.into());
                if !values.is_empty() {
                    return Err(FormatError::UnsupportedFeature(
                        "unknown Redrob SVG text attribute",
                    )
                    .into());
                }
                builder.as_mut().unwrap().push_node(
                    ImportNode::text(
                        common.name,
                        TextContent {
                            text: content,
                            font_family: family,
                            font_size,
                            color,
                            origin_x: x,
                            origin_y: y,
                            font_id,
                        },
                    )
                    .with_parent(parent)
                    .with_visibility(common.visible)
                    .with_opacity(common.opacity)
                    .with_blend_mode(common.blend),
                )?;
            }
            Event::Empty(start) if start.name().as_ref() == b"image" && !root_closed => {
                if options.loss_policy() == LossPolicy::RejectLoss {
                    return Err(FormatError::LossRequired(
                        "SVG embedded raster import requires AllowLoss",
                    )
                    .into());
                }
                let (width, height) =
                    dimensions.ok_or(FormatError::Malformed("SVG image before root"))?;
                let mut values = attrs(&reader, &start)?;
                let common = common(&mut values, "Image", redrob_namespace)?;
                let href = values
                    .remove("href")
                    .or_else(|| values.remove("xlink:href"))
                    .ok_or(FormatError::Malformed("missing SVG image href"))?;
                let encoded = href.strip_prefix("data:image/png;base64,").ok_or(
                    FormatError::UnsupportedFeature("external or non-PNG SVG image"),
                )?;
                if encoded.len() > crate::MAX_FORMAT_INPUT_BYTES {
                    return Err(FormatError::LimitExceeded("SVG data PNG bytes").into());
                }
                let png = base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .map_err(|_| FormatError::Malformed("invalid SVG data PNG"))?;
                let (source_width, source_height, source) =
                    crate::formats::decode_rgba(&png, FileFormat::Png)?;
                let x =
                    parse_integer_coordinate(&values.remove("x").unwrap_or_else(|| "0".into()))?;
                let y =
                    parse_integer_coordinate(&values.remove("y").unwrap_or_else(|| "0".into()))?;
                let declared_width = parse_dimension(
                    &values
                        .remove("width")
                        .ok_or(FormatError::Malformed("missing SVG image width"))?,
                )?;
                let declared_height = parse_dimension(
                    &values
                        .remove("height")
                        .ok_or(FormatError::Malformed("missing SVG image height"))?,
                )?;
                if declared_width != source_width
                    || declared_height != source_height
                    || !values.is_empty()
                {
                    return Err(
                        FormatError::UnsupportedFeature("SVG image scaling or attributes").into(),
                    );
                }
                let node = ImportNode::raster(
                    common.name,
                    vec![RasterCel::new(
                        FrameId::DEFAULT,
                        place_png(&source, source_width, source_height, width, height, x, y),
                    )],
                )
                .with_parent(parent_stack.last().copied())
                .with_visibility(common.visible)
                .with_opacity(common.opacity)
                .with_blend_mode(common.blend);
                let id = node.id();
                builder.as_mut().unwrap().push_node(node)?;
                warnings.push(FormatWarning::EmbeddedRasterData { node: id });
            }
            Event::End(end) if end.name().as_ref() == b"g" => {
                parent_stack
                    .pop()
                    .ok_or(FormatError::Malformed("unbalanced SVG group"))?;
            }
            Event::End(end) if end.name().as_ref() == b"svg" => {
                if root_closed || !parent_stack.is_empty() || text.is_some() {
                    return Err(FormatError::Malformed("unbalanced SVG root").into());
                }
                root_closed = true;
            }
            Event::Text(value) if value.as_ref().iter().all(u8::is_ascii_whitespace) => {}
            Event::DocType(_) => {
                return Err(FormatError::UnsupportedFeature("SVG DTD/entity content").into());
            }
            Event::Eof => break,
            _ => {
                return Err(FormatError::UnsupportedFeature(
                    "element outside the deterministic SVG subset",
                )
                .into());
            }
        }
        buffer.clear();
    }
    if !parent_stack.is_empty() || text.is_some() || !saw_root || !root_closed {
        return Err(FormatError::Malformed("unbalanced SVG XML").into());
    }
    Ok((
        builder
            .ok_or(FormatError::Malformed("missing SVG root"))?
            .build()?,
        warnings,
    ))
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn color(color: Pixel) -> String {
    format!(
        "#{:02X}{:02X}{:02X}{:02X}",
        color.r, color.g, color.b, color.a
    )
}

fn blend(mode: BlendMode) -> &'static str {
    match mode {
        BlendMode::Normal => "normal",
        BlendMode::Multiply => "multiply",
        BlendMode::Screen => "screen",
        BlendMode::Overlay => "overlay",
        BlendMode::Add => "plus",
    }
}

fn common_xml(node: &crate::Layer) -> String {
    format!(
        "redrob:name=\"{}\" visibility=\"{}\" opacity=\"{}\" redrob:blend=\"{}\"",
        escape(node.name()),
        if node.is_visible() {
            "visible"
        } else {
            "hidden"
        },
        node.opacity(),
        blend(node.blend_mode())
    )
}

fn path_data(commands: &[PathCommand]) -> Result<String> {
    let mut data = String::new();
    for command in commands {
        if !data.is_empty() {
            data.push(' ');
        }
        let value = match *command {
            PathCommand::MoveTo { x, y } => format!("M{x} {y}"),
            PathCommand::LineTo { x, y } => format!("L{x} {y}"),
            PathCommand::CubicTo {
                control1_x,
                control1_y,
                control2_x,
                control2_y,
                x,
                y,
            } => format!("C{control1_x} {control1_y} {control2_x} {control2_y} {x} {y}"),
            PathCommand::Close => "Z".to_owned(),
        };
        if data.len().saturating_add(value.len()) > MAX_SVG_XML_BYTES {
            return Err(FormatError::OutputTooLarge.into());
        }
        data.push_str(&value);
    }
    Ok(data)
}

fn raw_pixels(
    document: &Document,
    node: &crate::Layer,
    frame: FrameId,
    warnings: &mut Vec<FormatWarning>,
    policy: LossPolicy,
) -> Result<Vec<u8>> {
    let mut pixels = node.raster_pixels(frame).map_or_else(
        |_| vec![0; document.width() as usize * document.height() as usize * 4],
        <[u8]>::to_vec,
    );
    if let Some(mask) = node.mask() {
        if policy == LossPolicy::RejectLoss {
            return Err(FormatError::LossRequired("SVG cannot preserve raster mask source").into());
        }
        if mask.is_enabled() {
            for (pixel, coverage) in pixels.chunks_exact_mut(4).zip(mask.pixels()) {
                pixel[3] = ((u16::from(pixel[3]) * u16::from(*coverage) + 127) / 255) as u8;
            }
            warnings.push(FormatWarning::BakedRasterMask { node: node.id() });
        } else {
            warnings.push(FormatWarning::OmittedDisabledMask { node: node.id() });
        }
    }
    Ok(pixels)
}

fn ensure_output_capacity(current: usize, additional: usize) -> Result<()> {
    if current
        .checked_add(additional)
        .is_none_or(|total| total > MAX_SVG_XML_BYTES)
    {
        return Err(FormatError::OutputTooLarge.into());
    }
    Ok(())
}

fn base64_encoded_len(bytes: usize) -> Result<usize> {
    bytes
        .checked_add(2)
        .and_then(|value| value.checked_div(3))
        .and_then(|value| value.checked_mul(4))
        .ok_or_else(|| FormatError::OutputTooLarge.into())
}

fn write_nodes(
    output: &mut String,
    document: &Document,
    parent: Option<NodeId>,
    frame: FrameId,
    options: &ExportOptions,
    warnings: &mut Vec<FormatWarning>,
    indent: usize,
) -> Result<()> {
    for node in document
        .nodes()
        .iter()
        .filter(|node| node.parent_id() == parent)
    {
        let padding = "  ".repeat(indent);
        match node.kind() {
            NodeKind::Group => {
                if let Some(mask) = node.mask() {
                    if mask.is_enabled() {
                        return Err(FormatError::UnsupportedFeature(
                            "masked SVG groups cannot be represented safely",
                        )
                        .into());
                    }
                    if options.loss_policy() == LossPolicy::RejectLoss {
                        return Err(FormatError::LossRequired(
                            "SVG cannot preserve disabled group mask source",
                        )
                        .into());
                    }
                    warnings.push(FormatWarning::OmittedDisabledMask { node: node.id() });
                }
                output.push_str(&format!("{padding}<g {}>\n", common_xml(node)));
                write_nodes(
                    output,
                    document,
                    Some(node.id()),
                    frame,
                    options,
                    warnings,
                    indent + 1,
                )?;
                output.push_str(&format!("{padding}</g>\n"));
            }
            NodeKind::Vector => {
                let crate::NodeContent::Vector { vector } = node.content() else {
                    unreachable!()
                };
                if vector.paths.len() != 1 {
                    return Err(FormatError::UnsupportedFeature(
                        "SVG export requires exactly one path per vector node",
                    )
                    .into());
                }
                let path = &vector.paths[0];
                let fill = path.fill.map_or_else(|| "none".into(), color);
                let stroke = path
                    .stroke
                    .map_or_else(|| "none".into(), |stroke| color(stroke.color));
                let width = path.stroke.map_or(1.0, |stroke| stroke.width);
                let rule = if path.fill_rule == FillRule::EvenOdd {
                    "evenodd"
                } else {
                    "nonzero"
                };
                let data = path_data(&path.commands)?;
                output.push_str(&format!("{padding}<path {} d=\"{data}\" fill=\"{fill}\" stroke=\"{stroke}\" stroke-width=\"{width}\" fill-rule=\"{rule}\"/>\n", common_xml(node)));
            }
            NodeKind::Text => {
                let crate::NodeContent::Text { text } = node.content() else {
                    unreachable!()
                };
                output.push_str(&format!("{padding}<text {} redrob:kind=\"font8x8\" redrob:font-id=\"{}\" x=\"{}\" y=\"{}\" font-size=\"{}px\" font-family=\"{}\" fill=\"{}\">{}</text>\n", common_xml(node), escape(&text.font_id), text.origin_x, text.origin_y, text.font_size, escape(&text.font_family), color(text.color), escape(&text.text)));
            }
            NodeKind::Raster => {
                if options.loss_policy() == LossPolicy::RejectLoss {
                    return Err(
                        FormatError::LossRequired("SVG raster export requires AllowLoss").into(),
                    );
                }
                let raw_bytes = (document.width() as usize)
                    .checked_mul(document.height() as usize)
                    .and_then(|pixels| pixels.checked_mul(4))
                    .ok_or(FormatError::OutputTooLarge)?;
                let filtered_bytes = raw_bytes
                    .checked_add(document.height() as usize)
                    .ok_or(FormatError::OutputTooLarge)?;
                let deflate_overhead = filtered_bytes
                    .checked_div(65_535)
                    .and_then(|blocks| blocks.checked_add(1))
                    .and_then(|blocks| blocks.checked_mul(5))
                    .ok_or(FormatError::OutputTooLarge)?;
                let png_upper_bound = filtered_bytes
                    .checked_add(deflate_overhead)
                    .and_then(|bytes| bytes.checked_add(1_024))
                    .ok_or(FormatError::OutputTooLarge)?;
                ensure_output_capacity(output.len(), base64_encoded_len(png_upper_bound)?)?;
                let pixels = raw_pixels(document, node, frame, warnings, options.loss_policy())?;
                let png = crate::formats::encode_png(document.width(), document.height(), &pixels)?;
                ensure_output_capacity(output.len(), base64_encoded_len(png.len())?)?;
                let encoded = base64::engine::general_purpose::STANDARD.encode(png);
                output.push_str(&format!("{padding}<image {} x=\"0\" y=\"0\" width=\"{}\" height=\"{}\" href=\"data:image/png;base64,{encoded}\"/>\n", common_xml(node), document.width(), document.height()));
                warnings.push(FormatWarning::EmbeddedRasterData { node: node.id() });
            }
        }
        if output.len() > MAX_SVG_XML_BYTES {
            return Err(FormatError::OutputTooLarge.into());
        }
    }
    Ok(())
}

pub(crate) fn export_svg(
    document: &Document,
    frame: FrameId,
    options: &ExportOptions,
) -> Result<(Vec<u8>, Vec<FormatWarning>)> {
    let mut warnings = Vec::new();
    crate::formats::check_metadata_loss(document, options.loss_policy(), &mut warnings)?;
    if document.timeline().frames().len() > 1 {
        if options.loss_policy() == LossPolicy::RejectLoss {
            return Err(FormatError::LossRequired("SVG export omits other frames").into());
        }
        warnings.push(FormatWarning::OmittedFrames { exported: frame });
    }
    if document.is_selection_active() {
        if options.loss_policy() == LossPolicy::RejectLoss {
            return Err(FormatError::LossRequired("SVG export omits selection").into());
        }
        warnings.push(FormatWarning::OmittedSelection);
    }
    let mut output = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:redrob=\"{REDROB_NAMESPACE}\" width=\"{}px\" height=\"{}px\" viewBox=\"0 0 {} {}\">\n",
        document.width(),
        document.height(),
        document.width(),
        document.height()
    );
    write_nodes(
        &mut output,
        document,
        None,
        frame,
        options,
        &mut warnings,
        1,
    )?;
    output.push_str("</svg>\n");
    if output.len() > MAX_SVG_XML_BYTES {
        return Err(FormatError::OutputTooLarge.into());
    }
    Ok((output.into_bytes(), warnings))
}
