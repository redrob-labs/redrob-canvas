// SPDX-License-Identifier: GPL-3.0-or-later

//! H7: outline fonts (TrueType / OpenType), resolved by NAME, as Photoshop does.
//!
//! A text node stores only the font's name (`TextContent::font_family`) with `font_id` set to
//! [`SYSTEM_FONT_ID`]. The shell registers the font files it finds on the machine
//! ([`register_font`]); rendering looks the name up here. A document opened where its font is not
//! installed still renders -- with the built-in bitmap font -- and the shell can say which font is
//! missing ([`is_font_available`]). That is the trade the user chose: small files that may look
//! different on another computer, rather than fonts embedded in every document.
//!
//! Glyph outlines become an ordinary [`VectorContent`] fill, so text is drawn by the same
//! anti-aliased path filler as vector shapes.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use crate::document::{
    FillRule, MAX_PATH_COMMANDS_PER_PATH, MAX_VECTOR_PATHS, PathCommand, TextAlign, TextContent,
    VectorContent, VectorPath,
};
use crate::error::{CoreError, Result};

/// `font_id` of a text node whose font is looked up by name in the registry.
pub const SYSTEM_FONT_ID: &str = "system";
/// One registered file at most this large; real font files are a few MiB.
pub const MAX_FONT_FILE_BYTES: usize = 64 * 1024 * 1024;
/// At most this many names, so a hostile font folder cannot grow the registry without bound.
pub const MAX_FONT_NAMES: usize = 8_192;

struct Face {
    data: Arc<Vec<u8>>,
    index: u32,
}

fn registry() -> &'static RwLock<HashMap<String, Face>> {
    static REGISTRY: OnceLock<RwLock<HashMap<String, Face>>> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(HashMap::new()))
}

fn key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// Registers every face in a font file (a collection holds several) under its full name, and
/// under its family name when it is the family's regular face or the first one seen. Returns the
/// names added. Unparseable files are refused; nothing is registered from them.
pub fn register_font(bytes: Vec<u8>) -> Result<Vec<String>> {
    if bytes.len() > MAX_FONT_FILE_BYTES {
        return Err(CoreError::DocumentLimitExceeded("font file bytes"));
    }
    let faces = ttf_parser::fonts_in_collection(&bytes).unwrap_or(1).min(64);
    let data = Arc::new(bytes);
    let mut added = Vec::new();
    let mut map = registry()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for index in 0..faces {
        let Ok(face) = ttf_parser::Face::parse(&data, index) else {
            continue;
        };
        let name = |id: u16| {
            face.names()
                .into_iter()
                .filter(|n| n.name_id == id)
                .find_map(|n| n.to_string())
        };
        let family = name(ttf_parser::name_id::TYPOGRAPHIC_FAMILY)
            .or_else(|| name(ttf_parser::name_id::FAMILY));
        let full = name(ttf_parser::name_id::FULL_NAME);
        let style = name(ttf_parser::name_id::SUBFAMILY)
            .unwrap_or_default()
            .to_lowercase();
        let regular = matches!(style.as_str(), "regular" | "book" | "normal" | "roman" | "");
        for (label, replace) in [(full, true), (family, regular)] {
            let Some(label) = label.filter(|l| !l.trim().is_empty()) else {
                continue;
            };
            if map.len() >= MAX_FONT_NAMES {
                break;
            }
            let k = key(&label);
            if replace || !map.contains_key(&k) {
                if !map.contains_key(&k) {
                    added.push(label.clone());
                }
                map.insert(
                    k,
                    Face {
                        data: Arc::clone(&data),
                        index,
                    },
                );
            }
        }
    }
    if added.is_empty() && !map.values().any(|f| Arc::ptr_eq(&f.data, &data)) {
        return Err(CoreError::InvalidSemanticStyle);
    }
    Ok(added)
}

/// The names a font file would register, without registering it -- the shell indexes the font
/// folders with this and loads a file only when a document uses one of its names.
pub fn font_names(bytes: &[u8]) -> Vec<String> {
    let faces = ttf_parser::fonts_in_collection(bytes).unwrap_or(1).min(64);
    let mut names = Vec::new();
    for index in 0..faces {
        let Ok(face) = ttf_parser::Face::parse(bytes, index) else {
            continue;
        };
        for id in [
            ttf_parser::name_id::FULL_NAME,
            ttf_parser::name_id::TYPOGRAPHIC_FAMILY,
            ttf_parser::name_id::FAMILY,
        ] {
            if let Some(name) = face
                .names()
                .into_iter()
                .filter(|n| n.name_id == id)
                .find_map(|n| n.to_string())
            {
                if !name.trim().is_empty() && !names.contains(&name) {
                    names.push(name);
                }
            }
        }
    }
    names
}

/// Whether a font name resolves to a registered face.
pub fn is_font_available(name: &str) -> bool {
    registry()
        .read()
        .map(|map| map.contains_key(&key(name)))
        .unwrap_or(false)
}

struct Outline {
    commands: Vec<PathCommand>,
    origin_x: f32,
    baseline: f32,
    scale: f32,
    last: (f32, f32),
}

impl Outline {
    fn point(&self, x: f32, y: f32) -> (f32, f32) {
        // Font units are y-up; the canvas is y-down.
        (
            self.origin_x + x * self.scale,
            self.baseline - y * self.scale,
        )
    }
}

impl ttf_parser::OutlineBuilder for Outline {
    fn move_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.point(x, y);
        self.commands.push(PathCommand::MoveTo { x, y });
        self.last = (x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let (x, y) = self.point(x, y);
        self.commands.push(PathCommand::LineTo { x, y });
        self.last = (x, y);
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        // A quadratic is an exact cubic with its control points two thirds of the way along.
        let (qx, qy) = self.point(x1, y1);
        let (x, y) = self.point(x, y);
        let (sx, sy) = self.last;
        self.commands.push(PathCommand::CubicTo {
            control1_x: sx + (qx - sx) * 2.0 / 3.0,
            control1_y: sy + (qy - sy) * 2.0 / 3.0,
            control2_x: x + (qx - x) * 2.0 / 3.0,
            control2_y: y + (qy - y) * 2.0 / 3.0,
            x,
            y,
        });
        self.last = (x, y);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (c1x, c1y) = self.point(x1, y1);
        let (c2x, c2y) = self.point(x2, y2);
        let (x, y) = self.point(x, y);
        self.commands.push(PathCommand::CubicTo {
            control1_x: c1x,
            control1_y: c1y,
            control2_x: c2x,
            control2_y: c2y,
            x,
            y,
        });
        self.last = (x, y);
    }
    fn close(&mut self) {
        self.commands.push(PathCommand::Close);
    }
}

/// The text as filled glyph outlines, or `None` when the node does not name a registered font
/// (the caller then falls back to the bitmap font). Lines break at `\n`; paragraph text
/// (`box_width`) also wraps at spaces, measured with the font's own advances. Characters the font
/// has no glyph for advance by its missing-glyph width and draw nothing.
/// L13: the glyph ids a registered font shapes `line` into, in visual order (`None` when the
/// font is not registered). For tests and diagnostics: "fi" is one glyph in a font with that
/// ligature.
pub fn shaped_glyphs(family: &str, line: &str) -> Option<Vec<u16>> {
    let map = registry().read().ok()?;
    let entry = map.get(&key(family))?;
    let face = rustybuzz::Face::from_slice(&entry.data, entry.index)?;
    Some(
        shape_line(&face, line)
            .glyphs
            .iter()
            .map(|(id, _, _)| id.0)
            .collect(),
    )
}

pub(crate) fn outline_text(text: &TextContent) -> Option<Result<VectorContent>> {
    if text.font_id != SYSTEM_FONT_ID {
        return None;
    }
    let map = registry().read().ok()?;
    let entry = map.get(&key(&text.font_family))?;
    let face = rustybuzz::Face::from_slice(&entry.data, entry.index)?;
    Some(layout(&face, text))
}

/// One shaped line: each glyph with its pen position (font units, before scaling), and the
/// line's advance.
struct ShapedLine {
    glyphs: Vec<(ttf_parser::GlyphId, f32, f32)>,
    width: f32,
}

/// L13: OpenType shaping. GSUB turns "fi" into its ligature and picks contextual and
/// script forms (Arabic joining, Indic reordering, precomposed Hangul from jamo); GPOS places
/// pair kerning and combining marks; the `kern` table still applies to fonts that only have
/// that. Right-to-left runs come out in visual order, left to right.
fn shape_line(face: &rustybuzz::Face<'_>, line: &str) -> ShapedLine {
    let mut buffer = rustybuzz::UnicodeBuffer::new();
    buffer.push_str(line);
    buffer.guess_segment_properties();
    let shaped = rustybuzz::shape(face, &[], buffer);
    let (mut x, mut y) = (0_i32, 0_i32);
    let mut glyphs = Vec::with_capacity(shaped.len());
    for (info, pos) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
        let id = ttf_parser::GlyphId(u16::try_from(info.glyph_id).unwrap_or(0));
        glyphs.push((id, (x + pos.x_offset) as f32, (y + pos.y_offset) as f32));
        x += pos.x_advance;
        y += pos.y_advance;
    }
    ShapedLine {
        glyphs,
        width: x as f32,
    }
}

fn layout(face: &rustybuzz::Face<'_>, text: &TextContent) -> Result<VectorContent> {
    let scale = text.font_size / face.units_per_em().max(1) as f32;
    let measure = |s: &str| shape_line(face, s).width * scale;
    // Lines, wrapped to the box when there is one.
    let mut lines: Vec<String> = Vec::new();
    for hard in text.text.split('\n') {
        let Some(limit) = text.box_width else {
            lines.push(hard.to_string());
            continue;
        };
        let mut current = String::new();
        for word in hard.split(' ') {
            let candidate = if current.is_empty() {
                word.to_string()
            } else {
                format!("{current} {word}")
            };
            if measure(&candidate) <= limit || current.is_empty() {
                current = candidate;
            } else {
                lines.push(std::mem::take(&mut current));
                current = word.to_string();
            }
        }
        lines.push(current);
    }
    let ascender = f32::from(face.ascender()) * scale;
    let line_height = (f32::from(face.ascender()) - f32::from(face.descender())
        + f32::from(face.line_gap()))
        * scale;
    let widths: Vec<f32> = lines.iter().map(|l| measure(l)).collect();
    let frame = text
        .box_width
        .unwrap_or_else(|| widths.iter().copied().fold(0.0, f32::max));
    let mut paths = Vec::new();
    for (row, line) in lines.iter().enumerate() {
        let shift = match text.align {
            TextAlign::Left => 0.0,
            TextAlign::Center => (frame - widths[row]) / 2.0,
            TextAlign::Right => frame - widths[row],
        };
        let baseline = text.origin_y + ascender + row as f32 * line_height;
        let pen = text.origin_x + shift;
        for (glyph, gx, gy) in shape_line(face, line).glyphs {
            let (x, base) = (pen + gx * scale, baseline - gy * scale);
            let mut outline = Outline {
                commands: Vec::new(),
                origin_x: x,
                baseline: base,
                scale,
                last: (x, base),
            };
            if face.outline_glyph(glyph, &mut outline).is_some() && !outline.commands.is_empty() {
                if outline.commands.len() > MAX_PATH_COMMANDS_PER_PATH
                    || paths.len() >= MAX_VECTOR_PATHS
                {
                    return Err(CoreError::DocumentLimitExceeded("text glyph outlines"));
                }
                paths.push(VectorPath {
                    commands: outline.commands,
                    fill: Some(text.color),
                    stroke: None,
                    fill_rule: FillRule::NonZero,
                });
            }
        }
    }
    Ok(VectorContent { paths })
}
