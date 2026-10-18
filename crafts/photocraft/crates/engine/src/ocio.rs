// SPDX-License-Identifier: MIT OR Apache-2.0
//! OpenColorIO display transforms, read from an OCIO configuration file (`config.ocio`, YAML)
//! with its LUT files, and applied to the canvas display as a scene-linear view transform:
//! input colour space → (exposure) → reference → look → display / view → (gamma), with the
//! component view (one channel or the luminance) and a locked colour. This is a clean-room
//! reader of the published OCIO configuration format (the `.ocio` YAML schema and the `.spi1d`,
//! `.spi3d` and `.cube` LUT formats), not a port of the OpenColorIO library: the subset
//! understood is the one display configs use (matrix, exponent, log, file, colour-space, group,
//! CDL and look transforms, in either direction).
//!
//! `OcioSettings` is kept in Color Settings (persisted with the preferences); `OcioState`
//! holds the parsed config and is rebuilt when the path changes. `view.ocio` changes the
//! settings, `view.ocioExposure` nudges the exposure (the Y-key drag), and
//! [`crate::proof_sim::display_lut_with`] applies [`OcioState::processor`] to the canvas LUT.

#![allow(clippy::needless_range_loop)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

// ------------------------------------------------------------------ settings

/// Which components the canvas shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Components {
    #[default]
    Rgb,
    Red,
    Green,
    Blue,
    Luminance,
}

impl Components {
    pub const ALL: [Components; 5] = [Components::Rgb, Components::Red, Components::Green, Components::Blue, Components::Luminance];
    pub fn id(self) -> &'static str {
        match self {
            Components::Rgb => "rgb",
            Components::Red => "red",
            Components::Green => "green",
            Components::Blue => "blue",
            Components::Luminance => "luminance",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.id() == s.to_ascii_lowercase())
    }
    /// Show only these components of a displayed colour.
    pub fn apply(self, c: [f32; 3]) -> [f32; 3] {
        match self {
            Components::Rgb => c,
            Components::Red => [c[0]; 3],
            Components::Green => [c[1]; 3],
            Components::Blue => [c[2]; 3],
            Components::Luminance => [0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]; 3],
        }
    }
}

/// The LUT Management docker's settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct OcioSettings {
    /// Use OCIO for the canvas display (off: the ICC display transform).
    pub enabled: bool,
    /// Path of the `.ocio` configuration file (empty: the built-in minimal config with linear
    /// and sRGB spaces).
    pub config: String,
    /// Input colour space of the document's pixels (a colour space or role name).
    pub input_space: String,
    pub display: String,
    pub view: String,
    /// A look name, or several separated by commas (`+look`, `-look` as OCIO writes them);
    /// empty = the view's own looks.
    pub look: String,
    pub components: Components,
    /// Stops, −10…10.
    pub exposure: f32,
    /// 0.1…5, 1 = none.
    pub gamma: f32,
    /// Lock colour: the colour selectors and swatches show their colours unaffected by the
    /// display transform (the exposure and gamma do not change what a picked colour looks like
    /// in the panels).
    pub lock_color: bool,
}

impl Default for OcioSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            config: String::new(),
            input_space: "scene_linear".into(),
            display: String::new(),
            view: String::new(),
            look: String::new(),
            components: Components::Rgb,
            exposure: 0.0,
            gamma: 1.0,
            lock_color: false,
        }
    }
}

impl OcioSettings {
    pub fn is_identity(&self) -> bool {
        !self.enabled
    }
}

// ------------------------------------------------------------------ YAML subset

/// A YAML node of the subset OCIO configs use: scalars, sequences and mappings, with an
/// optional `!<Tag>`.
#[derive(Clone, Debug, PartialEq)]
pub enum Yaml {
    Scalar(String),
    Seq(Vec<Yaml>),
    Map(Vec<(String, Yaml)>),
    Tagged(String, Box<Yaml>),
}

impl Yaml {
    fn untagged(&self) -> &Yaml {
        match self {
            Yaml::Tagged(_, inner) => inner.untagged(),
            other => other,
        }
    }
    pub fn tag(&self) -> Option<&str> {
        match self {
            Yaml::Tagged(t, _) => Some(t),
            _ => None,
        }
    }
    pub fn get(&self, key: &str) -> Option<&Yaml> {
        match self.untagged() {
            Yaml::Map(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self.untagged() {
            Yaml::Scalar(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_f32(&self) -> Option<f32> {
        self.as_str().and_then(|s| s.trim().parse::<f32>().ok())
    }
    pub fn as_seq(&self) -> Option<&[Yaml]> {
        match self.untagged() {
            Yaml::Seq(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_map(&self) -> Option<&[(String, Yaml)]> {
        match self.untagged() {
            Yaml::Map(m) => Some(m),
            _ => None,
        }
    }
    /// A sequence of numbers (a flow list, or a scalar holding one number).
    pub fn numbers(&self) -> Vec<f32> {
        match self.untagged() {
            Yaml::Seq(s) => s.iter().filter_map(Yaml::as_f32).collect(),
            Yaml::Scalar(_) => self.as_f32().into_iter().collect(),
            _ => Vec::new(),
        }
    }
    fn str_of(&self, key: &str) -> String {
        self.get(key).and_then(Yaml::as_str).unwrap_or("").trim().to_string()
    }
}

struct Line {
    indent: usize,
    text: String,
}

/// Parse a YAML document of the OCIO subset. Comments and blank lines are dropped; block
/// mappings and sequences by indentation; flow collections `{…}` / `[…]` on one line (or
/// continued over the following lines until the brackets balance); `!<Tag>` before a value;
/// single- and double-quoted scalars.
pub fn parse_yaml(text: &str) -> std::result::Result<Yaml, String> {
    let mut lines: Vec<Line> = Vec::new();
    let mut pending: Option<Line> = None;
    let mut depth: i32 = 0;
    for raw in text.lines() {
        let raw = raw.trim_end_matches('\r');
        let stripped = strip_comment(raw);
        if let Some(p) = pending.as_mut() {
            p.text.push(' ');
            p.text.push_str(stripped.trim());
            depth += bracket_depth(stripped);
            if depth <= 0 {
                if let Some(p) = pending.take() {
                    lines.push(p);
                }
                depth = 0;
            }
            continue;
        }
        if stripped.trim().is_empty() || stripped.trim_start().starts_with("---") || stripped.trim_start().starts_with("%YAML") {
            continue;
        }
        let indent = stripped.len() - stripped.trim_start().len();
        let line = Line { indent, text: stripped.trim().to_string() };
        depth = bracket_depth(&line.text);
        if depth > 0 {
            pending = Some(line);
        } else {
            depth = 0;
            lines.push(line);
        }
    }
    if let Some(p) = pending {
        lines.push(p);
    }
    let mut pos = 0;
    let node = parse_block(&lines, &mut pos, 0)?;
    Ok(node)
}

fn strip_comment(s: &str) -> &str {
    let mut in_single = false;
    let mut in_double = false;
    let b = s.as_bytes();
    for i in 0..b.len() {
        match b[i] {
            b'\'' if !in_double => in_single = !in_single,
            b'"' if !in_single => in_double = !in_double,
            b'#' if !in_single && !in_double && (i == 0 || b[i - 1] == b' ' || b[i - 1] == b'\t') => return &s[..i],
            _ => {}
        }
    }
    s
}

fn bracket_depth(s: &str) -> i32 {
    let mut d = 0;
    let mut in_single = false;
    let mut in_double = false;
    for c in s.chars() {
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '{' | '[' if !in_single && !in_double => d += 1,
            '}' | ']' if !in_single && !in_double => d -= 1,
            _ => {}
        }
    }
    d
}

fn parse_block(lines: &[Line], pos: &mut usize, indent: usize) -> std::result::Result<Yaml, String> {
    let Some(first) = lines.get(*pos) else { return Ok(Yaml::Scalar(String::new())) };
    if first.text.starts_with("- ") || first.text == "-" {
        let mut items = Vec::new();
        while let Some(l) = lines.get(*pos) {
            if l.indent != indent || !(l.text.starts_with("- ") || l.text == "-") {
                if l.indent > indent {
                    return Err(format!("unexpected indentation at `{}`", l.text));
                }
                break;
            }
            let rest = l.text[1..].trim_start().to_string();
            *pos += 1;
            if rest.is_empty() {
                let child =
                    if lines.get(*pos).is_some_and(|n| n.indent > indent) { parse_block(lines, pos, lines[*pos].indent)? } else { Yaml::Scalar(String::new()) };
                items.push(child);
            } else if is_map_entry(&rest) && !rest.starts_with('{') && !rest.starts_with('[') && !rest.starts_with("!<") {
                // `- key: value` starts an inline mapping whose further keys are indented by 2.
                let sub_indent = indent + 2;
                let mut entries = vec![map_entry(&rest, lines, pos, sub_indent)?];
                while let Some(l) = lines.get(*pos) {
                    if l.indent != sub_indent || l.text.starts_with("- ") {
                        break;
                    }
                    let t = l.text.clone();
                    *pos += 1;
                    entries.push(map_entry(&t, lines, pos, sub_indent)?);
                }
                items.push(Yaml::Map(entries));
            } else {
                items.push(parse_value(&rest, lines, pos, indent)?);
            }
        }
        return Ok(Yaml::Seq(items));
    }
    let mut entries = Vec::new();
    while let Some(l) = lines.get(*pos) {
        if l.indent != indent {
            if l.indent > indent {
                return Err(format!("unexpected indentation at `{}`", l.text));
            }
            break;
        }
        if l.text.starts_with("- ") {
            break;
        }
        let t = l.text.clone();
        *pos += 1;
        entries.push(map_entry(&t, lines, pos, indent)?);
    }
    Ok(Yaml::Map(entries))
}

fn is_map_entry(s: &str) -> bool {
    split_key(s).is_some()
}

/// Split `key: value` at the first `:` that is followed by a space or ends the text, outside
/// quotes and brackets.
fn split_key(s: &str) -> Option<(&str, &str)> {
    let b = s.as_bytes();
    let (mut sq, mut dq, mut depth) = (false, false, 0);
    for i in 0..b.len() {
        match b[i] {
            b'\'' if !dq => sq = !sq,
            b'"' if !sq => dq = !dq,
            b'{' | b'[' if !sq && !dq => depth += 1,
            b'}' | b']' if !sq && !dq => depth -= 1,
            b':' if !sq && !dq && depth == 0 && (i + 1 == b.len() || b[i + 1] == b' ') => {
                let key = s[..i].trim();
                if key.is_empty() || key.starts_with("!<") {
                    return None;
                }
                return Some((key, s[i + 1..].trim()));
            }
            _ => {}
        }
    }
    None
}

fn map_entry(text: &str, lines: &[Line], pos: &mut usize, indent: usize) -> std::result::Result<(String, Yaml), String> {
    let (key, rest) = split_key(text).ok_or_else(|| format!("expected `key: value`, found `{text}`"))?;
    let key = unquote(key);
    if rest.is_empty() || rest.starts_with("!<") && rest.trim_end().ends_with('>') {
        // The value is the indented block below (possibly tagged).
        let tag = rest.strip_prefix("!<").and_then(|t| t.strip_suffix('>')).map(str::to_string);
        let child = if lines.get(*pos).is_some_and(|n| n.indent > indent) { parse_block(lines, pos, lines[*pos].indent)? } else { Yaml::Scalar(String::new()) };
        return Ok((
            key,
            match tag {
                Some(t) => Yaml::Tagged(t, Box::new(child)),
                None => child,
            },
        ));
    }
    Ok((key, parse_value(rest, lines, pos, indent)?))
}

fn parse_value(s: &str, lines: &[Line], pos: &mut usize, indent: usize) -> std::result::Result<Yaml, String> {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("!<") {
        let end = rest.find('>').ok_or_else(|| format!("unterminated tag in `{s}`"))?;
        let tag = rest[..end].to_string();
        let after = rest[end + 1..].trim();
        if after.is_empty() {
            let child =
                if lines.get(*pos).is_some_and(|n| n.indent > indent) { parse_block(lines, pos, lines[*pos].indent)? } else { Yaml::Scalar(String::new()) };
            return Ok(Yaml::Tagged(tag, Box::new(child)));
        }
        return Ok(Yaml::Tagged(tag, Box::new(parse_value(after, lines, pos, indent)?)));
    }
    if s.starts_with('{') || s.starts_with('[') {
        let mut i = 0;
        let v = parse_flow(s, &mut i)?;
        return Ok(v);
    }
    Ok(Yaml::Scalar(unquote(s)))
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\''))) {
        s[1..s.len() - 1].replace("\\\"", "\"").replace("''", "'")
    } else {
        s.to_string()
    }
}

fn parse_flow(s: &str, i: &mut usize) -> std::result::Result<Yaml, String> {
    let b = s.as_bytes();
    skip_ws(b, i);
    match b.get(*i) {
        Some(b'[') => {
            *i += 1;
            let mut items = Vec::new();
            loop {
                skip_ws(b, i);
                match b.get(*i) {
                    Some(b']') => {
                        *i += 1;
                        return Ok(Yaml::Seq(items));
                    }
                    Some(b',') => {
                        *i += 1;
                    }
                    None => return Err("unterminated `[`".into()),
                    _ => items.push(parse_flow(s, i)?),
                }
            }
        }
        Some(b'{') => {
            *i += 1;
            let mut entries = Vec::new();
            loop {
                skip_ws(b, i);
                match b.get(*i) {
                    Some(b'}') => {
                        *i += 1;
                        return Ok(Yaml::Map(entries));
                    }
                    Some(b',') => {
                        *i += 1;
                    }
                    None => return Err("unterminated `{`".into()),
                    _ => {
                        let key = flow_scalar(s, i, true)?;
                        skip_ws(b, i);
                        if b.get(*i) != Some(&b':') {
                            return Err(format!("expected `:` after `{key}`"));
                        }
                        *i += 1;
                        skip_ws(b, i);
                        let v = parse_flow(s, i)?;
                        entries.push((unquote(&key), v));
                    }
                }
            }
        }
        Some(b'!') if s[*i..].starts_with("!<") => {
            let rest = &s[*i + 2..];
            let end = rest.find('>').ok_or("unterminated tag")?;
            let tag = rest[..end].to_string();
            *i += 2 + end + 1;
            skip_ws(b, i);
            let inner = parse_flow(s, i)?;
            Ok(Yaml::Tagged(tag, Box::new(inner)))
        }
        Some(_) => Ok(Yaml::Scalar(unquote(&flow_scalar(s, i, false)?))),
        None => Ok(Yaml::Scalar(String::new())),
    }
}

fn skip_ws(b: &[u8], i: &mut usize) {
    while *i < b.len() && (b[*i] == b' ' || b[*i] == b'\t') {
        *i += 1;
    }
}

fn flow_scalar(s: &str, i: &mut usize, key: bool) -> std::result::Result<String, String> {
    let b = s.as_bytes();
    let start = *i;
    if matches!(b.get(*i), Some(b'"') | Some(b'\'')) {
        let q = b[*i];
        *i += 1;
        while *i < b.len() && b[*i] != q {
            *i += 1;
        }
        *i = (*i + 1).min(b.len());
        return Ok(s[start..*i].to_string());
    }
    while *i < b.len() {
        let c = b[*i];
        if c == b',' || c == b']' || c == b'}' || (key && c == b':') {
            break;
        }
        *i += 1;
    }
    Ok(s[start..*i].trim().to_string())
}

// ------------------------------------------------------------------ LUTs

/// A LUT read from a file.
#[derive(Clone, Debug, PartialEq)]
pub enum Lut {
    /// 1D: `table[i]` = output for input `from[0] + i × (from[1] − from[0]) / (n − 1)`.
    Lut1d { from: [f32; 2], table: Vec<[f32; 3]> },
    /// 3D: `size³` RGB entries, red varying fastest (the `.spi3d` / `.cube` order).
    Lut3d { size: usize, table: Vec<[f32; 3]>, domain_min: [f32; 3], domain_max: [f32; 3] },
}

impl Lut {
    pub fn parse(name: &str, text: &str) -> std::result::Result<Lut, String> {
        let ext = Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        match ext.as_str() {
            "spi1d" => parse_spi1d(text),
            "spi3d" => parse_spi3d(text),
            "cube" => parse_cube(text),
            other => Err(format!("unsupported LUT format `.{other}` (spi1d, spi3d, cube)")),
        }
    }

    pub fn apply(&self, c: [f32; 3]) -> [f32; 3] {
        match self {
            Lut::Lut1d { from, table } => {
                let n = table.len();
                if n == 0 {
                    return c;
                }
                let span = (from[1] - from[0]).abs().max(1e-9);
                let mut out = c;
                for (k, v) in out.iter_mut().enumerate() {
                    let t = ((*v - from[0]) / span).clamp(0.0, 1.0) * (n - 1) as f32;
                    let i = (t.floor() as usize).min(n - 1);
                    let j = (i + 1).min(n - 1);
                    let f = t - i as f32;
                    *v = table[i][k] + (table[j][k] - table[i][k]) * f;
                }
                out
            }
            Lut::Lut3d { size, table, domain_min, domain_max } => {
                let n = *size;
                if n < 2 || table.len() < n * n * n {
                    return c;
                }
                let idx = |r: usize, g: usize, b: usize| table[r + g * n + b * n * n];
                let mut t = [0.0f32; 3];
                for k in 0..3 {
                    let span = (domain_max[k] - domain_min[k]).abs().max(1e-9);
                    t[k] = ((c[k] - domain_min[k]) / span).clamp(0.0, 1.0) * (n - 1) as f32;
                }
                let i0 = t.map(|v| (v.floor() as usize).min(n - 1));
                let i1 = i0.map(|v| (v + 1).min(n - 1));
                let f = [t[0] - i0[0] as f32, t[1] - i0[1] as f32, t[2] - i0[2] as f32];
                let lerp = |a: [f32; 3], b: [f32; 3], x: f32| [a[0] + (b[0] - a[0]) * x, a[1] + (b[1] - a[1]) * x, a[2] + (b[2] - a[2]) * x];
                let c00 = lerp(idx(i0[0], i0[1], i0[2]), idx(i1[0], i0[1], i0[2]), f[0]);
                let c10 = lerp(idx(i0[0], i1[1], i0[2]), idx(i1[0], i1[1], i0[2]), f[0]);
                let c01 = lerp(idx(i0[0], i0[1], i1[2]), idx(i1[0], i0[1], i1[2]), f[0]);
                let c11 = lerp(idx(i0[0], i1[1], i1[2]), idx(i1[0], i1[1], i1[2]), f[0]);
                let c0 = lerp(c00, c10, f[1]);
                let c1 = lerp(c01, c11, f[1]);
                lerp(c0, c1, f[2])
            }
        }
    }

    /// The inverse of a monotone 1D LUT by search; a 3D LUT inverts approximately (its
    /// identity-domain guess), as OCIO configs rarely need one.
    pub fn apply_inverse(&self, c: [f32; 3]) -> [f32; 3] {
        match self {
            Lut::Lut1d { from, table } => {
                let n = table.len();
                if n < 2 {
                    return c;
                }
                let mut out = c;
                for (k, v) in out.iter_mut().enumerate() {
                    let rising = table[n - 1][k] >= table[0][k];
                    let (mut lo, mut hi) = (0usize, n - 1);
                    while hi - lo > 1 {
                        let mid = (lo + hi) / 2;
                        if (table[mid][k] <= *v) == rising { lo = mid } else { hi = mid }
                    }
                    let (a, b) = (table[lo][k], table[hi][k]);
                    let f = if (b - a).abs() > 1e-9 { ((*v - a) / (b - a)).clamp(0.0, 1.0) } else { 0.0 };
                    let t = (lo as f32 + f) / (n - 1) as f32;
                    *v = from[0] + t * (from[1] - from[0]);
                }
                out
            }
            Lut::Lut3d { .. } => c,
        }
    }
}

fn parse_spi1d(text: &str) -> std::result::Result<Lut, String> {
    let mut from = [0.0f32, 1.0];
    let mut components = 1usize;
    let mut table = Vec::new();
    let mut in_table = false;
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() {
            continue;
        }
        if in_table {
            if l == "}" {
                break;
            }
            let v: Vec<f32> = l.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            if v.is_empty() {
                continue;
            }
            table.push(match (components, v.len()) {
                (1, _) => [v[0]; 3],
                (_, n) if n >= 3 => [v[0], v[1], v[2]],
                _ => [v[0]; 3],
            });
            continue;
        }
        if let Some(r) = l.strip_prefix("From") {
            let v: Vec<f32> = r.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            if v.len() >= 2 {
                from = [v[0], v[1]];
            }
        } else if let Some(r) = l.strip_prefix("Components") {
            components = r.trim().parse().unwrap_or(1);
        } else if l == "{" {
            in_table = true;
        }
    }
    if table.len() < 2 {
        return Err("spi1d: no table".into());
    }
    Ok(Lut::Lut1d { from, table })
}

fn parse_spi3d(text: &str) -> std::result::Result<Lut, String> {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let first = lines.next().ok_or("spi3d: empty")?;
    if !first.eq_ignore_ascii_case("SPILUT 1.0") {
        return Err("spi3d: not a SPILUT 1.0 file".into());
    }
    let _dims = lines.next();
    let sizes: Vec<usize> = lines.next().ok_or("spi3d: no size")?.split_whitespace().filter_map(|x| x.parse().ok()).collect();
    let n = *sizes.first().ok_or("spi3d: no size")?;
    if n < 2 || sizes.iter().any(|s| *s != n) {
        return Err("spi3d: only cubic LUTs are supported".into());
    }
    let mut table = vec![[0.0f32; 3]; n * n * n];
    for l in lines {
        let v: Vec<f32> = l.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        if v.len() < 6 {
            continue;
        }
        let (r, g, b) = (v[0] as usize, v[1] as usize, v[2] as usize);
        if r < n && g < n && b < n {
            table[r + g * n + b * n * n] = [v[3], v[4], v[5]];
        }
    }
    Ok(Lut::Lut3d { size: n, table, domain_min: [0.0; 3], domain_max: [1.0; 3] })
}

fn parse_cube(text: &str) -> std::result::Result<Lut, String> {
    let mut size1 = 0usize;
    let mut size3 = 0usize;
    let mut dmin = [0.0f32; 3];
    let mut dmax = [1.0f32; 3];
    let mut rows: Vec<[f32; 3]> = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let up = l.to_ascii_uppercase();
        if let Some(r) = up.strip_prefix("LUT_1D_SIZE") {
            size1 = r.trim().parse().map_err(|_| "cube: bad LUT_1D_SIZE")?;
        } else if let Some(r) = up.strip_prefix("LUT_3D_SIZE") {
            size3 = r.trim().parse().map_err(|_| "cube: bad LUT_3D_SIZE")?;
        } else if let Some(r) = up.strip_prefix("DOMAIN_MIN") {
            let v: Vec<f32> = r.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            if v.len() >= 3 {
                dmin = [v[0], v[1], v[2]];
            }
        } else if let Some(r) = up.strip_prefix("DOMAIN_MAX") {
            let v: Vec<f32> = r.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            if v.len() >= 3 {
                dmax = [v[0], v[1], v[2]];
            }
        } else if up.starts_with("TITLE") || up.starts_with("LUT_1D_INPUT_RANGE") || up.starts_with("LUT_3D_INPUT_RANGE") {
            if let Some(r) = up.strip_prefix("LUT_1D_INPUT_RANGE").or_else(|| up.strip_prefix("LUT_3D_INPUT_RANGE")) {
                let v: Vec<f32> = r.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                if v.len() >= 2 {
                    dmin = [v[0]; 3];
                    dmax = [v[1]; 3];
                }
            }
        } else {
            let v: Vec<f32> = l.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            if v.len() >= 3 {
                rows.push([v[0], v[1], v[2]]);
            }
        }
    }
    if size3 >= 2 && rows.len() >= size3 * size3 * size3 {
        rows.truncate(size3 * size3 * size3);
        return Ok(Lut::Lut3d { size: size3, table: rows, domain_min: dmin, domain_max: dmax });
    }
    if size1 >= 2 && rows.len() >= size1 {
        rows.truncate(size1);
        return Ok(Lut::Lut1d { from: [dmin[0], dmax[0]], table: rows });
    }
    Err("cube: no LUT table".into())
}

// ------------------------------------------------------------------ config model

#[derive(Clone, Debug, PartialEq)]
pub enum Transform {
    Identity,
    /// 4×4 row-major matrix (RGBA) and offset, as OCIO writes them.
    Matrix {
        m: [f32; 16],
        offset: [f32; 4],
    },
    Exponent([f32; 4]),
    ExponentLinear {
        gamma: [f32; 4],
        offset: [f32; 4],
    },
    Log {
        base: f32,
    },
    LogAffine {
        base: f32,
        lin_slope: [f32; 3],
        lin_offset: [f32; 3],
        log_slope: [f32; 3],
        log_offset: [f32; 3],
    },
    Cdl {
        slope: [f32; 3],
        offset: [f32; 3],
        power: [f32; 3],
        sat: f32,
    },
    File(Arc<Lut>),
    ColorSpace {
        src: String,
        dst: String,
    },
    Look {
        looks: String,
        src: String,
        dst: String,
    },
    Group(Vec<Transform>),
    Inverse(Box<Transform>),
    /// A built-in or unsupported transform kept by name; applied as identity and reported.
    Unsupported(String),
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct ColorSpaceDef {
    pub name: String,
    pub family: String,
    pub aliases: Vec<String>,
    pub is_data: bool,
    pub to_reference: Option<Transform>,
    pub from_reference: Option<Transform>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct View {
    pub name: String,
    pub colorspace: String,
    pub looks: String,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Display {
    pub name: String,
    pub views: Vec<View>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Look {
    pub name: String,
    pub process_space: String,
    pub transform: Option<Transform>,
    pub inverse_transform: Option<Transform>,
}

/// A parsed OCIO configuration.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Config {
    pub version: String,
    pub roles: BTreeMap<String, String>,
    pub displays: Vec<Display>,
    pub active_displays: Vec<String>,
    pub active_views: Vec<String>,
    pub colorspaces: Vec<ColorSpaceDef>,
    pub looks: Vec<Look>,
    pub search_path: Vec<String>,
    /// Directory the config was read from (LUT files resolve against `search_path` under it).
    pub dir: PathBuf,
    /// LUT files that could not be read, and transforms not understood.
    pub warnings: Vec<String>,
}

fn parse_transform(node: &Yaml, dir: &Path, search: &[String], warnings: &mut Vec<String>) -> Transform {
    let Some(tag) = node.tag() else {
        // An untagged sequence is a group.
        if let Some(seq) = node.as_seq() {
            return Transform::Group(seq.iter().map(|n| parse_transform(n, dir, search, warnings)).collect());
        }
        return Transform::Identity;
    };
    let inverse = node.str_of("direction").eq_ignore_ascii_case("inverse");
    let t = match tag {
        "MatrixTransform" => {
            let m = node.get("matrix").map(Yaml::numbers).unwrap_or_default();
            let o = node.get("offset").map(Yaml::numbers).unwrap_or_default();
            let mut mat = [0.0f32; 16];
            for (i, v) in [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0].iter().enumerate() {
                mat[i] = *m.get(i).unwrap_or(v);
            }
            let mut off = [0.0f32; 4];
            for (i, slot) in off.iter_mut().enumerate() {
                *slot = *o.get(i).unwrap_or(&0.0);
            }
            Transform::Matrix { m: mat, offset: off }
        }
        "ExponentTransform" => {
            let v = node.get("value").map(Yaml::numbers).unwrap_or_default();
            Transform::Exponent(vec4(&v, 1.0))
        }
        "ExponentWithLinearTransform" => {
            let g = node.get("gamma").map(Yaml::numbers).unwrap_or_default();
            let o = node.get("offset").map(Yaml::numbers).unwrap_or_default();
            Transform::ExponentLinear { gamma: vec4(&g, 1.0), offset: vec4(&o, 0.0) }
        }
        "LogTransform" => Transform::Log { base: node.get("base").and_then(Yaml::as_f32).unwrap_or(2.0) },
        "LogAffineTransform" | "LogCameraTransform" => Transform::LogAffine {
            base: node.get("base").and_then(Yaml::as_f32).unwrap_or(2.0),
            lin_slope: vec3(&node.get("linSideSlope").map(Yaml::numbers).unwrap_or_default(), 1.0),
            lin_offset: vec3(&node.get("linSideOffset").map(Yaml::numbers).unwrap_or_default(), 0.0),
            log_slope: vec3(&node.get("logSideSlope").map(Yaml::numbers).unwrap_or_default(), 1.0),
            log_offset: vec3(&node.get("logSideOffset").map(Yaml::numbers).unwrap_or_default(), 0.0),
        },
        "CDLTransform" => Transform::Cdl {
            slope: vec3(&node.get("slope").map(Yaml::numbers).unwrap_or_default(), 1.0),
            offset: vec3(&node.get("offset").map(Yaml::numbers).unwrap_or_default(), 0.0),
            power: vec3(&node.get("power").map(Yaml::numbers).unwrap_or_default(), 1.0),
            sat: node.get("sat").and_then(Yaml::as_f32).unwrap_or(1.0),
        },
        "FileTransform" => {
            let src = node.str_of("src");
            match load_lut(dir, search, &src) {
                Ok(l) => Transform::File(Arc::new(l)),
                Err(e) => {
                    warnings.push(format!("{src}: {e}"));
                    Transform::Unsupported(format!("FileTransform {src}"))
                }
            }
        }
        "ColorSpaceTransform" => Transform::ColorSpace { src: node.str_of("src"), dst: node.str_of("dst") },
        "LookTransform" => Transform::Look { looks: node.str_of("looks"), src: node.str_of("src"), dst: node.str_of("dst") },
        "GroupTransform" => {
            Transform::Group(node.get("children").and_then(Yaml::as_seq).unwrap_or(&[]).iter().map(|n| parse_transform(n, dir, search, warnings)).collect())
        }
        "AllocationTransform" | "RangeTransform" => Transform::Identity,
        other => {
            let name = node.str_of("style");
            warnings.push(format!("{other}{} is not supported and is applied as identity", if name.is_empty() { String::new() } else { format!(" ({name})") }));
            Transform::Unsupported(other.to_string())
        }
    };
    if inverse { Transform::Inverse(Box::new(t)) } else { t }
}

fn vec4(v: &[f32], d: f32) -> [f32; 4] {
    let mut out = [d; 4];
    for (i, slot) in out.iter_mut().enumerate() {
        if let Some(x) = v.get(i) {
            *slot = *x;
        } else if i == 3 && !v.is_empty() {
            *slot = d;
        } else if !v.is_empty() {
            *slot = v[0];
        }
    }
    out
}

fn vec3(v: &[f32], d: f32) -> [f32; 3] {
    let mut out = [d; 3];
    for (i, slot) in out.iter_mut().enumerate() {
        if let Some(x) = v.get(i) {
            *slot = *x;
        } else if !v.is_empty() {
            *slot = v[0];
        }
    }
    out
}

fn load_lut(dir: &Path, search: &[String], src: &str) -> std::result::Result<Lut, String> {
    if src.is_empty() {
        return Err("no src".into());
    }
    let mut candidates: Vec<PathBuf> = Vec::new();
    let p = Path::new(src);
    if p.is_absolute() {
        candidates.push(p.to_path_buf());
    }
    for s in search.iter().chain(std::iter::once(&String::from("."))) {
        for part in s.split(':') {
            candidates.push(dir.join(part).join(src));
        }
    }
    for c in candidates {
        if c.is_file() {
            let text = std::fs::read_to_string(&c).map_err(|e| format!("{}: {e}", c.display()))?;
            return Lut::parse(src, &text);
        }
    }
    Err("file not found on the search path".into())
}

impl Config {
    /// The built-in configuration: a scene-linear reference with sRGB and Rec. 709 displays,
    /// so OCIO mode works without a config file.
    pub fn builtin() -> Self {
        let srgb_from_ref = Transform::Unsupported("sRGB-curve".into());
        let mut c = Config { version: "2.0".into(), dir: PathBuf::new(), ..Default::default() };
        c.roles.insert("scene_linear".into(), "Linear".into());
        c.roles.insert("reference".into(), "Linear".into());
        c.roles.insert("default".into(), "sRGB".into());
        c.roles.insert("color_picking".into(), "sRGB".into());
        c.colorspaces.push(ColorSpaceDef { name: "Linear".into(), family: "linear".into(), ..Default::default() });
        c.colorspaces.push(ColorSpaceDef { name: "sRGB".into(), family: "display".into(), from_reference: Some(srgb_from_ref), ..Default::default() });
        c.colorspaces.push(ColorSpaceDef {
            name: "Rec.709".into(),
            family: "display".into(),
            from_reference: Some(Transform::Unsupported("Rec709-curve".into())),
            ..Default::default()
        });
        c.colorspaces.push(ColorSpaceDef { name: "Raw".into(), family: "raw".into(), is_data: true, ..Default::default() });
        c.displays.push(Display {
            name: "sRGB".into(),
            views: vec![
                View { name: "Standard".into(), colorspace: "sRGB".into(), looks: String::new() },
                View { name: "Raw".into(), colorspace: "Raw".into(), looks: String::new() },
            ],
        });
        c.displays.push(Display { name: "Rec.709".into(), views: vec![View { name: "Standard".into(), colorspace: "Rec.709".into(), looks: String::new() }] });
        c
    }

    /// Parse a config file's text. `dir` is where its LUT files are looked up.
    pub fn parse(text: &str, dir: &Path) -> std::result::Result<Self, String> {
        let root = parse_yaml(text)?;
        let mut cfg = Config { dir: dir.to_path_buf(), ..Default::default() };
        cfg.version = root.str_of("ocio_profile_version");
        if let Some(sp) = root.get("search_path") {
            cfg.search_path = match sp.untagged() {
                Yaml::Seq(s) => s.iter().filter_map(Yaml::as_str).map(str::to_string).collect(),
                _ => sp.as_str().map(|s| s.split(':').map(str::to_string).collect()).unwrap_or_default(),
            };
        }
        if let Some(roles) = root.get("roles").and_then(Yaml::as_map) {
            for (k, v) in roles {
                cfg.roles.insert(k.clone(), v.as_str().unwrap_or("").to_string());
            }
        }
        let mut warnings = Vec::new();
        let (dir, search) = (cfg.dir.clone(), cfg.search_path.clone());
        if let Some(displays) = root.get("displays").and_then(Yaml::as_map) {
            for (name, views) in displays {
                let mut d = Display { name: name.clone(), views: Vec::new() };
                for v in views.as_seq().unwrap_or(&[]) {
                    d.views.push(View { name: v.str_of("name"), colorspace: v.str_of("colorspace"), looks: v.str_of("looks") });
                }
                cfg.displays.push(d);
            }
        }
        for (key, slot) in [("active_displays", &mut cfg.active_displays), ("active_views", &mut cfg.active_views)] {
            if let Some(n) = root.get(key) {
                *slot = match n.untagged() {
                    Yaml::Seq(s) => s.iter().filter_map(Yaml::as_str).map(str::to_string).collect(),
                    _ => n.as_str().map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()).unwrap_or_default(),
                };
            }
        }
        for l in root.get("looks").and_then(Yaml::as_seq).unwrap_or(&[]) {
            cfg.looks.push(Look {
                name: l.str_of("name"),
                process_space: l.str_of("process_space"),
                transform: l.get("transform").map(|t| parse_transform(t, &dir, &search, &mut warnings)),
                inverse_transform: l.get("inverse_transform").map(|t| parse_transform(t, &dir, &search, &mut warnings)),
            });
        }
        for key in ["colorspaces", "display_colorspaces"] {
            for cs in root.get(key).and_then(Yaml::as_seq).unwrap_or(&[]) {
                cfg.colorspaces.push(ColorSpaceDef {
                    name: cs.str_of("name"),
                    family: cs.str_of("family"),
                    aliases: cs.get("aliases").and_then(Yaml::as_seq).unwrap_or(&[]).iter().filter_map(Yaml::as_str).map(str::to_string).collect(),
                    is_data: cs.str_of("isdata").eq_ignore_ascii_case("true"),
                    to_reference: cs.get("to_reference").or(cs.get("to_scene_reference")).map(|t| parse_transform(t, &dir, &search, &mut warnings)),
                    from_reference: cs.get("from_reference").or(cs.get("from_scene_reference")).map(|t| parse_transform(t, &dir, &search, &mut warnings)),
                });
            }
        }
        if cfg.colorspaces.is_empty() {
            return Err("the config defines no colorspaces".into());
        }
        cfg.warnings = warnings;
        Ok(cfg)
    }

    /// Read and parse a config file.
    pub fn load(path: &Path) -> std::result::Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        Self::parse(&text, &dir)
    }

    /// A colour space by name, alias or role.
    pub fn colorspace(&self, name: &str) -> Option<&ColorSpaceDef> {
        let n = name.trim();
        if n.is_empty() {
            return None;
        }
        if let Some(cs) = self.colorspaces.iter().find(|c| c.name.eq_ignore_ascii_case(n) || c.aliases.iter().any(|a| a.eq_ignore_ascii_case(n))) {
            return Some(cs);
        }
        self.roles.iter().find(|(r, _)| r.eq_ignore_ascii_case(n)).and_then(|(_, target)| {
            self.colorspaces.iter().find(|c| c.name.eq_ignore_ascii_case(target) || c.aliases.iter().any(|a| a.eq_ignore_ascii_case(target)))
        })
    }

    pub fn display(&self, name: &str) -> Option<&Display> {
        if name.trim().is_empty() {
            return self
                .displays
                .iter()
                .find(|d| self.active_displays.is_empty() || self.active_displays.iter().any(|a| a.eq_ignore_ascii_case(&d.name)))
                .or(self.displays.first());
        }
        self.displays.iter().find(|d| d.name.eq_ignore_ascii_case(name.trim()))
    }

    pub fn view<'a>(&'a self, display: &'a Display, name: &str) -> Option<&'a View> {
        if name.trim().is_empty() {
            return display
                .views
                .iter()
                .find(|v| self.active_views.is_empty() || self.active_views.iter().any(|a| a.eq_ignore_ascii_case(&v.name)))
                .or(display.views.first());
        }
        display.views.iter().find(|v| v.name.eq_ignore_ascii_case(name.trim()))
    }

    pub fn look(&self, name: &str) -> Option<&Look> {
        self.looks.iter().find(|l| l.name.eq_ignore_ascii_case(name.trim()))
    }

    /// Colour space names, in config order (roles are not listed).
    pub fn colorspace_names(&self) -> Vec<String> {
        self.colorspaces.iter().map(|c| c.name.clone()).collect()
    }

    // --- evaluation

    fn apply_transform(&self, t: &Transform, c: [f32; 3], inverse: bool, depth: u32) -> [f32; 3] {
        if depth > 32 {
            return c;
        }
        match t {
            Transform::Identity | Transform::Unsupported(_) => builtin_curve(t, c, inverse),
            Transform::Matrix { m, offset } => {
                if inverse {
                    let inv = invert_affine(m, offset);
                    match inv {
                        Some((mi, oi)) => mat_apply(&mi, &oi, c),
                        None => c,
                    }
                } else {
                    mat_apply(m, offset, c)
                }
            }
            Transform::Exponent(g) => {
                let mut o = c;
                for k in 0..3 {
                    let e = if inverse { 1.0 / g[k].max(1e-6) } else { g[k] };
                    o[k] = o[k].max(0.0).powf(e);
                }
                o
            }
            Transform::ExponentLinear { gamma, offset } => {
                let mut o = c;
                for k in 0..3 {
                    let (g, off) = (gamma[k].max(1e-6), offset[k].clamp(0.0, 0.9));
                    // Moncurve: linear segment near 0, (x + off)/(1 + off) ^ g above the break.
                    let brk = off / (g - 1.0).max(1e-6);
                    let slope = ((g - 1.0) / off.max(1e-9)) * ((off * g) / ((g - 1.0) * (1.0 + off))).powf(g);
                    if !inverse {
                        o[k] = if o[k] < brk { o[k] * slope } else { ((o[k] + off) / (1.0 + off)).max(0.0).powf(g) };
                    } else {
                        let ybrk = brk * slope;
                        o[k] = if o[k] < ybrk { o[k] / slope.max(1e-9) } else { o[k].max(0.0).powf(1.0 / g) * (1.0 + off) - off };
                    }
                }
                o
            }
            Transform::Log { base } => {
                let b = if *base > 1.0 { *base } else { 2.0 };
                if inverse { c.map(|v| b.powf(v)) } else { c.map(|v| v.max(1e-9).ln() / b.ln()) }
            }
            Transform::LogAffine { base, lin_slope, lin_offset, log_slope, log_offset } => {
                let b = if *base > 1.0 { *base } else { 2.0 };
                let mut o = c;
                for k in 0..3 {
                    if !inverse {
                        let lin = (lin_slope[k] * o[k] + lin_offset[k]).max(1e-9);
                        o[k] = log_slope[k] * (lin.ln() / b.ln()) + log_offset[k];
                    } else {
                        let l = (o[k] - log_offset[k]) / log_slope[k].max(1e-9);
                        o[k] = (b.powf(l) - lin_offset[k]) / lin_slope[k].max(1e-9);
                    }
                }
                o
            }
            Transform::Cdl { slope, offset, power, sat } => {
                let mut o = c;
                if !inverse {
                    for k in 0..3 {
                        o[k] = (o[k] * slope[k] + offset[k]).max(0.0).powf(power[k]);
                    }
                    let l = 0.2126 * o[0] + 0.7152 * o[1] + 0.0722 * o[2];
                    o = o.map(|v| l + (v - l) * sat);
                } else {
                    let l = 0.2126 * o[0] + 0.7152 * o[1] + 0.0722 * o[2];
                    o = o.map(|v| l + (v - l) / sat.max(1e-6));
                    for k in 0..3 {
                        o[k] = (o[k].max(0.0).powf(1.0 / power[k].max(1e-6)) - offset[k]) / slope[k].max(1e-9);
                    }
                }
                o
            }
            Transform::File(l) => {
                if inverse {
                    l.apply_inverse(c)
                } else {
                    l.apply(c)
                }
            }
            Transform::ColorSpace { src, dst } => {
                if inverse {
                    self.convert(c, dst, src, depth + 1)
                } else {
                    self.convert(c, src, dst, depth + 1)
                }
            }
            Transform::Look { looks, src, dst } => {
                let mut o = if src.is_empty() { c } else { self.convert(c, src, "reference", depth + 1) };
                o = self.apply_looks(o, looks, inverse, depth + 1);
                if !dst.is_empty() {
                    o = self.convert(o, "reference", dst, depth + 1);
                }
                o
            }
            Transform::Group(children) => {
                if inverse {
                    children.iter().rev().fold(c, |acc, t| self.apply_transform(t, acc, true, depth + 1))
                } else {
                    children.iter().fold(c, |acc, t| self.apply_transform(t, acc, false, depth + 1))
                }
            }
            Transform::Inverse(inner) => self.apply_transform(inner, c, !inverse, depth + 1),
        }
    }

    /// Colour `c` in space `src` → the reference.
    fn to_reference(&self, c: [f32; 3], cs: &ColorSpaceDef, depth: u32) -> [f32; 3] {
        if cs.is_data {
            return c;
        }
        match (&cs.to_reference, &cs.from_reference) {
            (Some(t), _) => self.apply_transform(t, c, false, depth),
            (None, Some(f)) => self.apply_transform(f, c, true, depth),
            (None, None) => c,
        }
    }

    fn convert_from_reference(&self, c: [f32; 3], cs: &ColorSpaceDef, depth: u32) -> [f32; 3] {
        if cs.is_data {
            return c;
        }
        match (&cs.from_reference, &cs.to_reference) {
            (Some(f), _) => self.apply_transform(f, c, false, depth),
            (None, Some(t)) => self.apply_transform(t, c, true, depth),
            (None, None) => c,
        }
    }

    /// Convert between two colour spaces (names, aliases or roles; `reference` is the
    /// reference space itself).
    pub fn convert(&self, c: [f32; 3], src: &str, dst: &str, depth: u32) -> [f32; 3] {
        let is_ref = |n: &str| n.eq_ignore_ascii_case("reference") && self.colorspace(n).is_none();
        let mid = if is_ref(src) {
            c
        } else {
            match self.colorspace(src) {
                Some(cs) => self.to_reference(c, cs, depth),
                None => c,
            }
        };
        if is_ref(dst) {
            return mid;
        }
        match self.colorspace(dst) {
            Some(cs) => self.convert_from_reference(mid, cs, depth),
            None => mid,
        }
    }

    fn apply_looks(&self, c: [f32; 3], looks: &str, inverse: bool, depth: u32) -> [f32; 3] {
        let mut o = c;
        let items: Vec<(bool, &str)> = looks
            .split([',', ':'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| match s.strip_prefix('-') {
                Some(n) => (true, n.trim()),
                None => (false, s.strip_prefix('+').unwrap_or(s).trim()),
            })
            .collect();
        for (neg, name) in items {
            let Some(look) = self.look(name) else { continue };
            let inv = neg != inverse;
            let ps = if look.process_space.is_empty() { "reference".to_string() } else { look.process_space.clone() };
            o = self.convert(o, "reference", &ps, depth);
            o = match (inv, &look.transform, &look.inverse_transform) {
                (false, Some(t), _) => self.apply_transform(t, o, false, depth),
                (true, _, Some(it)) => self.apply_transform(it, o, false, depth),
                (true, Some(t), None) => self.apply_transform(t, o, true, depth),
                _ => o,
            };
            o = self.convert(o, &ps, "reference", depth);
        }
        o
    }

    /// Build the display processor for the settings: input space → exposure → reference → looks
    /// → view colour space → gamma → components. Errors name a space, display or view the
    /// config does not have.
    pub fn processor(&self, st: &OcioSettings) -> std::result::Result<Processor, String> {
        let input = self.colorspace(&st.input_space).ok_or_else(|| format!("no input colour space or role `{}`", st.input_space))?.clone();
        let display = self.display(&st.display).ok_or_else(|| format!("no display `{}`", st.display))?;
        let view = self.view(display, &st.view).ok_or_else(|| format!("no view `{}` on display `{}`", st.view, display.name))?.clone();
        let view_cs = self.colorspace(&view.colorspace).ok_or_else(|| format!("the view's colour space `{}` is not defined", view.colorspace))?.clone();
        let looks = if st.look.trim().is_empty() { view.looks.clone() } else { st.look.clone() };
        Ok(Processor {
            config: Arc::new(self.clone()),
            input,
            view_cs,
            looks,
            exposure: st.exposure.clamp(-10.0, 10.0),
            gamma: if st.gamma.is_finite() && st.gamma > 0.05 { st.gamma.min(5.0) } else { 1.0 },
            components: st.components,
        })
    }
}

fn mat_apply(m: &[f32; 16], o: &[f32; 4], c: [f32; 3]) -> [f32; 3] {
    [m[0] * c[0] + m[1] * c[1] + m[2] * c[2] + o[0], m[4] * c[0] + m[5] * c[1] + m[6] * c[2] + o[1], m[8] * c[0] + m[9] * c[1] + m[10] * c[2] + o[2]]
}

fn invert_affine(m: &[f32; 16], o: &[f32; 4]) -> Option<([f32; 16], [f32; 4])> {
    let a = [[m[0], m[1], m[2]], [m[4], m[5], m[6]], [m[8], m[9], m[10]]];
    let det = a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1]) - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
        + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0]);
    if det.abs() < 1e-12 {
        return None;
    }
    let d = 1.0 / det;
    let inv = [
        [(a[1][1] * a[2][2] - a[1][2] * a[2][1]) * d, (a[0][2] * a[2][1] - a[0][1] * a[2][2]) * d, (a[0][1] * a[1][2] - a[0][2] * a[1][1]) * d],
        [(a[1][2] * a[2][0] - a[1][0] * a[2][2]) * d, (a[0][0] * a[2][2] - a[0][2] * a[2][0]) * d, (a[0][2] * a[1][0] - a[0][0] * a[1][2]) * d],
        [(a[1][0] * a[2][1] - a[1][1] * a[2][0]) * d, (a[0][1] * a[2][0] - a[0][0] * a[2][1]) * d, (a[0][0] * a[1][1] - a[0][1] * a[1][0]) * d],
    ];
    let mut mi = [0.0f32; 16];
    mi[15] = 1.0;
    for r in 0..3 {
        for cidx in 0..3 {
            mi[r * 4 + cidx] = inv[r][cidx];
        }
    }
    // x = A⁻¹ (y − o)
    let oi = [
        -(inv[0][0] * o[0] + inv[0][1] * o[1] + inv[0][2] * o[2]),
        -(inv[1][0] * o[0] + inv[1][1] * o[1] + inv[1][2] * o[2]),
        -(inv[2][0] * o[0] + inv[2][1] * o[1] + inv[2][2] * o[2]),
        0.0,
    ];
    Some((mi, oi))
}

/// The built-in curves of [`Config::builtin`] (and the identity).
fn builtin_curve(t: &Transform, c: [f32; 3], inverse: bool) -> [f32; 3] {
    match t {
        Transform::Unsupported(name) if name == "sRGB-curve" => {
            if inverse {
                c.map(redrob_photo_color::convert::srgb_to_linear)
            } else {
                c.map(|v| redrob_photo_color::convert::linear_to_srgb(v.clamp(0.0, 1.0)))
            }
        }
        Transform::Unsupported(name) if name == "Rec709-curve" => {
            if inverse {
                c.map(|v| if v < 0.081 { v / 4.5 } else { ((v + 0.099) / 1.099).powf(1.0 / 0.45) })
            } else {
                c.map(|v| {
                    let v = v.clamp(0.0, 1.0);
                    if v < 0.018 { v * 4.5 } else { 1.099 * v.powf(0.45) - 0.099 }
                })
            }
        }
        _ => c,
    }
}

/// A ready display transform.
#[derive(Clone, Debug)]
pub struct Processor {
    config: Arc<Config>,
    input: ColorSpaceDef,
    view_cs: ColorSpaceDef,
    looks: String,
    exposure: f32,
    gamma: f32,
    components: Components,
}

impl Processor {
    /// Display a document colour `c` (in the input space): the displayed, display-encoded RGB.
    pub fn apply(&self, c: [f32; 3]) -> [f32; 3] {
        let cfg = &self.config;
        let mut lin = cfg.to_reference(c, &self.input, 0);
        if self.exposure != 0.0 {
            let k = 2f32.powf(self.exposure);
            lin = lin.map(|v| v * k);
        }
        if !self.looks.is_empty() {
            lin = cfg.apply_looks(lin, &self.looks, false, 0);
        }
        let mut out = cfg.convert_from_reference(lin, &self.view_cs, 0);
        if (self.gamma - 1.0).abs() > 1e-4 {
            out = out.map(|v| v.max(0.0).powf(1.0 / self.gamma));
        }
        self.components.apply(out.map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }))
    }

    /// A cache key of what this processor does.
    pub fn signature(&self) -> String {
        format!("{}|{}|{}|{}|{}|{:?}", self.input.name, self.view_cs.name, self.looks, self.exposure, self.gamma, self.components)
    }
}

// ------------------------------------------------------------------ session state

/// The loaded configuration (`Session::color.ocio`), loaded on first use and reloaded when the
/// settings name another file. Behind a mutex so the display LUT builder (which has `&ColorState`)
/// can load it.
#[derive(Default)]
pub struct OcioState {
    inner: std::sync::Mutex<Inner>,
}

#[derive(Default)]
struct Inner {
    /// The path the config was loaded from (empty: the built-in config).
    loaded_path: String,
    config: Option<Arc<Config>>,
    error: Option<String>,
}

impl OcioState {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// (Re)load the config named by the settings when it changed.
    pub fn sync(&self, st: &OcioSettings) {
        let mut g = self.lock();
        if g.config.is_some() && g.loaded_path == st.config {
            return;
        }
        g.loaded_path = st.config.clone();
        if st.config.trim().is_empty() {
            g.config = Some(Arc::new(Config::builtin()));
            g.error = None;
            return;
        }
        match Config::load(Path::new(&st.config)) {
            Ok(c) => {
                g.config = Some(Arc::new(c));
                g.error = None;
            }
            Err(e) => {
                g.config = Some(Arc::new(Config::builtin()));
                g.error = Some(e);
            }
        }
    }

    pub fn config(&self, st: &OcioSettings) -> Option<Arc<Config>> {
        self.sync(st);
        self.lock().config.clone()
    }

    pub fn error(&self) -> Option<String> {
        self.lock().error.clone()
    }

    pub fn loaded_path(&self) -> String {
        self.lock().loaded_path.clone()
    }

    /// The processor for the settings, when OCIO display is on and the settings resolve.
    pub fn processor(&self, st: &OcioSettings) -> Option<Processor> {
        if !st.enabled {
            return None;
        }
        self.config(st).and_then(|c| c.processor(st).ok())
    }
}

/// The active OCIO processor for the canvas, when OCIO display is on and resolves.
pub fn active(c: &crate::color_cmds::ColorState) -> Option<Processor> {
    c.ocio.processor(&c.settings.ocio)
}

/// A cache key for the OCIO part of a display LUT.
pub fn signature(c: &crate::color_cmds::ColorState) -> String {
    match active(c) {
        Some(p) => p.signature(),
        None => String::new(),
    }
}

// ------------------------------------------------------------------ commands

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn report(s: &Session) -> Value {
    let st = &s.color.settings.ocio;
    let cfg = s.color.ocio.config(st);
    let displays: Vec<Value> = cfg
        .as_ref()
        .map(|c| c.displays.iter().map(|d| json!({ "name": d.name, "views": d.views.iter().map(|v| json!({"name": v.name, "colorspace": v.colorspace, "looks": v.looks})).collect::<Vec<_>>() })).collect())
        .unwrap_or_default();
    let processor_error = cfg.as_ref().and_then(|c| c.processor(st).err());
    json!({
        "settings": st,
        "configLoaded": s.color.ocio.loaded_path(),
        "configError": s.color.ocio.error(),
        "processorError": processor_error,
        "warnings": cfg.as_ref().map(|c| c.warnings.clone()).unwrap_or_default(),
        "colorspaces": cfg.as_ref().map(|c| c.colorspace_names()).unwrap_or_default(),
        "roles": cfg.as_ref().map(|c| c.roles.clone()).unwrap_or_default(),
        "displays": displays,
        "looks": cfg.as_ref().map(|c| c.looks.iter().map(|l| l.name.clone()).collect::<Vec<_>>()).unwrap_or_default(),
        "active": active(&s.color).is_some(),
    })
}

/// `view.ocio`: read or change the LUT Management settings.
fn ocio(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "view.ocio";
    let mut next = s.color.settings.ocio.clone();
    if p.get("reset").and_then(Value::as_bool) == Some(true) {
        next = OcioSettings::default();
    }
    let str_of = |k: &str| p.get(k).and_then(Value::as_str).map(|v| v.trim().to_string());
    if let Some(v) = p.get("enabled").and_then(Value::as_bool) {
        next.enabled = v;
    }
    if let Some(v) = p.get("lockColor").and_then(Value::as_bool) {
        next.lock_color = v;
    }
    if let Some(v) = str_of("config") {
        next.config = v;
    }
    if let Some(v) = str_of("inputSpace") {
        next.input_space = v;
    }
    if let Some(v) = str_of("display") {
        next.display = v;
    }
    if let Some(v) = str_of("view") {
        next.view = v;
    }
    if let Some(v) = str_of("look") {
        next.look = v;
    }
    if let Some(v) = str_of("components") {
        next.components = Components::parse(&v).ok_or_else(|| bad(cmd, "`components` must be rgb|red|green|blue|luminance"))?;
    }
    if let Some(v) = p.get("exposure") {
        next.exposure = v.as_f64().filter(|x| x.is_finite() && (-10.0..=10.0).contains(x)).ok_or_else(|| bad(cmd, "`exposure` must be -10..10 stops"))? as f32;
    }
    if let Some(v) = p.get("gamma") {
        next.gamma = v.as_f64().filter(|x| x.is_finite() && (0.1..=5.0).contains(x)).ok_or_else(|| bad(cmd, "`gamma` must be 0.1..5"))? as f32;
    }
    if next != s.color.settings.ocio {
        s.color.settings.ocio = next;
        s.prefs.edit(|_| ());
    }
    Ok(report(s))
}

/// `view.ocioExposure`: nudge the exposure by `delta` stops (the Y-key drag; `set` sets it).
fn ocio_exposure(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "view.ocioExposure";
    let cur = s.color.settings.ocio.exposure;
    let next = match (p.get("set").and_then(Value::as_f64), p.get("delta").and_then(Value::as_f64)) {
        (Some(v), _) if v.is_finite() => v as f32,
        (_, Some(d)) if d.is_finite() => cur + d as f32,
        _ => return Err(bad(cmd, "`delta` or `set` must be a finite number of stops")),
    }
    .clamp(-10.0, 10.0);
    if next != cur {
        s.color.settings.ocio.exposure = next;
        s.prefs.edit(|_| ());
    }
    Ok(json!({ "exposure": next, "gamma": s.color.settings.ocio.gamma, "active": active(&s.color).is_some() }))
}

/// `view.ocioConvert`: convert a colour between two spaces of the loaded config (for the colour
/// selectors' proofing and the locked colour).
fn ocio_convert(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "view.ocioConvert";
    let st = s.color.settings.ocio.clone();
    let cfg = s.color.ocio.config(&st).ok_or_else(|| bad(cmd, "no config loaded"))?;
    let color = p
        .get("color")
        .and_then(Value::as_array)
        .filter(|a| a.len() >= 3)
        .map(|a| [a[0].as_f64().unwrap_or(0.0) as f32, a[1].as_f64().unwrap_or(0.0) as f32, a[2].as_f64().unwrap_or(0.0) as f32])
        .ok_or_else(|| bad(cmd, "`color` must be [r, g, b]"))?;
    let src = p.get("src").and_then(Value::as_str).unwrap_or(&st.input_space).to_string();
    let dst = p.get("dst").and_then(Value::as_str).map(str::to_string);
    let out = match dst {
        Some(d) => cfg.convert(color, &src, &d, 0),
        None => cfg.processor(&st).map_err(|e| bad(cmd, e))?.apply(color),
    };
    Ok(json!({ "color": out }))
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "view.ocio",
            label: "LUT Management (OCIO)",
            menu: &[],
            shortcut: None,
            params: r##"{"enabled":bool?,"config":path?="" (built-in config),"inputSpace":name|role?,"display":name?,"view":name?,"look":names?,"components":"rgb|red|green|blue|luminance"?,"exposure":-10..10?,"gamma":0.1..5?,"lockColor":bool?,"reset":bool?} → {settings, configLoaded, configError, processorError, warnings, colorspaces, roles, displays, looks, active}"##,
            enabled: always,
            run: ocio,
            journal: true,
        },
        CommandSpec {
            id: "view.ocioExposure",
            label: "OCIO Exposure",
            menu: &[],
            shortcut: None,
            params: r##"{"delta":stops? (the Y-key drag),"set":stops?} → {exposure, gamma, active}"##,
            enabled: always,
            run: ocio_exposure,
            journal: true,
        },
        CommandSpec {
            id: "view.ocioConvert",
            label: "OCIO Convert Colour",
            menu: &[],
            shortcut: None,
            params: r##"{"color":[r,g,b],"src":space?=inputSpace,"dst":space? (omit: through the display transform)} → {color}"##,
            enabled: always,
            run: ocio_convert,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"
ocio_profile_version: 1

search_path: luts
strictparsing: true
luma: [0.2126, 0.7152, 0.0722]

roles:
  scene_linear: linear
  reference: linear
  color_picking: sRGB

displays:
  default:
    - !<View> {name: sRGB, colorspace: sRGB}
    - !<View> {name: Raw, colorspace: raw}
    - !<View> {name: Film, colorspace: sRGB, looks: +warm}

active_displays: [default]
active_views: [sRGB, Film, Raw]

looks:
  - !<Look>
    name: warm
    process_space: linear
    transform: !<CDLTransform> {slope: [1.1, 1.0, 0.9], offset: [0, 0, 0], power: [1, 1, 1], sat: 1.0}

colorspaces:
  - !<ColorSpace>
    name: linear
    family: ""
    bitdepth: 32f
    isdata: false
    allocation: lg2
    allocationvars: [-15, 6]

  - !<ColorSpace>
    name: raw
    isdata: true

  - !<ColorSpace>
    name: sRGB
    family: display  # a comment
    from_reference: !<GroupTransform>
      children:
        - !<MatrixTransform> {matrix: [2, 0, 0, 0, 0, 2, 0, 0, 0, 0, 2, 0, 0, 0, 0, 1]}
        - !<ExponentTransform> {value: [0.5, 0.5, 0.5, 1], direction: forward}
"#;

    #[test]
    fn parses_an_ocio_config() {
        let cfg = Config::parse(CONFIG, Path::new("/nonexistent")).unwrap();
        assert_eq!(cfg.roles.get("scene_linear").map(String::as_str), Some("linear"));
        assert_eq!(cfg.displays.len(), 1);
        assert_eq!(cfg.displays[0].views.len(), 3);
        assert_eq!(cfg.displays[0].views[2].looks, "+warm");
        assert_eq!(cfg.colorspaces.len(), 3);
        assert!(cfg.colorspace("raw").unwrap().is_data);
        assert_eq!(cfg.looks.len(), 1);
        assert!(cfg.warnings.is_empty(), "{:?}", cfg.warnings);
    }

    #[test]
    fn processor_applies_matrix_exponent_exposure_and_look() {
        let cfg = Config::parse(CONFIG, Path::new("/nonexistent")).unwrap();
        let st = OcioSettings { enabled: true, input_space: "scene_linear".into(), display: "default".into(), view: "sRGB".into(), ..Default::default() };
        let p = cfg.processor(&st).unwrap();
        // 0.125 × 2 = 0.25, ^0.5 = 0.5
        let o = p.apply([0.125, 0.125, 0.125]);
        assert!((o[0] - 0.5).abs() < 1e-4, "{o:?}");
        let st2 = OcioSettings { exposure: 1.0, ..st.clone() };
        let o2 = cfg.processor(&st2).unwrap().apply([0.125; 3]);
        assert!((o2[0] - (0.5f32).sqrt()).abs() < 1e-4, "{o2:?}");
        let st3 = OcioSettings { view: "Film".into(), ..st.clone() };
        let o3 = cfg.processor(&st3).unwrap().apply([0.125; 3]);
        assert!(o3[0] > o3[2], "the warm look lifts red over blue: {o3:?}");
        let raw = cfg.processor(&OcioSettings { view: "Raw".into(), ..st }).unwrap().apply([0.3, 0.2, 0.1]);
        assert_eq!(raw, [0.3, 0.2, 0.1]);
    }

    #[test]
    fn inverse_conversions_round_trip() {
        let cfg = Config::parse(CONFIG, Path::new("/nonexistent")).unwrap();
        let c = [0.2, 0.4, 0.6];
        let shown = cfg.convert(c, "linear", "sRGB", 0);
        let back = cfg.convert(shown, "sRGB", "linear", 0);
        assert!(back.iter().zip(c).all(|(a, b)| (a - b).abs() < 1e-4), "{back:?}");
    }

    #[test]
    fn cube_and_spi1d_luts_parse_and_apply() {
        let cube = "TITLE \"t\"\nLUT_1D_SIZE 3\n0 0 0\n0.25 0.25 0.25\n1 1 1\n";
        let l = Lut::parse("a.cube", cube).unwrap();
        assert!((l.apply([0.75; 3])[0] - 0.625).abs() < 1e-5);
        let spi = "Version 1\nFrom 0.0 1.0\nLength 3\nComponents 1\n{\n0.0\n0.5\n1.0\n}\n";
        let l1 = Lut::parse("b.spi1d", spi).unwrap();
        assert!((l1.apply([0.25; 3])[1] - 0.25).abs() < 1e-5);
        assert!((l1.apply_inverse([0.5; 3])[0] - 0.5).abs() < 1e-5);
        let spi3 = "SPILUT 1.0\n3 3\n2 2 2\n0 0 0 0 0 0\n1 0 0 1 0 0\n0 1 0 0 1 0\n1 1 0 1 1 0\n0 0 1 0 0 1\n1 0 1 1 0 1\n0 1 1 0 1 1\n1 1 1 1 1 1\n";
        let l3 = Lut::parse("c.spi3d", spi3).unwrap();
        let o = l3.apply([0.3, 0.6, 0.9]);
        assert!(o.iter().zip([0.3, 0.6, 0.9]).all(|(a, b)| (a - b).abs() < 1e-5), "{o:?}");
    }

    #[test]
    fn builtin_config_shows_linear_as_srgb() {
        let cfg = Config::builtin();
        let st = OcioSettings { enabled: true, ..Default::default() };
        let p = cfg.processor(&st).unwrap();
        let o = p.apply([0.2140; 3]);
        assert!((o[0] - 0.5).abs() < 0.01, "{o:?}");
        let lum = cfg.processor(&OcioSettings { components: Components::Luminance, ..st }).unwrap().apply([1.0, 0.0, 0.0]);
        assert!(lum[0] == lum[1] && lum[0] < 0.6);
    }

    #[test]
    fn yaml_flow_and_block_forms_agree() {
        let a = parse_yaml("a: {x: 1, y: [1, 2]}\n").unwrap();
        let b = parse_yaml("a:\n  x: 1\n  y:\n    - 1\n    - 2\n").unwrap();
        assert_eq!(a, b);
        let t = parse_yaml("t: !<Foo> {k: 'v: w'}\n").unwrap();
        assert_eq!(t.get("t").and_then(Yaml::tag), Some("Foo"));
        assert_eq!(t.get("t").unwrap().str_of("k"), "v: w");
    }
}
