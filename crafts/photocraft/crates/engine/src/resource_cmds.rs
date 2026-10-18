//! Resource storages, bundles and native preset files (`resources.*`, `brush.presets.exportKpp`,
//! `brush.presets.importKpp`, `gradient.presets.importSvg` / `exportSvg`).
//!
//! * Storages: where resources come from (the user folder, imported bundles, Adobe brush and
//!   style libraries, the open document, memory). Each can be deactivated, which parks the
//!   brush presets it brought until it is activated again (`resources.storages.*`).
//! * Bundles: a zip of `paintoppresets/*.kpp`, `brushes/*.gbr|.gih|.png`, `gradients/*.ggr|.svg`,
//!   `palettes/*.gpl|.kpl`, `patterns/*.pat`, with `META-INF/manifest.xml` and `meta.xml`
//!   (Dublin Core metadata) and a `preview.png`. Import reads every entry it knows through the
//!   matching import command; create writes the chosen presets, palettes and gradients
//!   (`resources.bundle.*`).
//! * Native preset files: a `.kpp` is a PNG whose `tEXt` chunks carry `version` and `preset`, the
//!   latter an XML `<Preset name paintopid version>` of `<param name type>` children. Ours writes
//!   the full brush settings as one JSON param (`redrob/brush`) plus the required tip and
//!   pattern file names (`requiredBrushFile`, `requiredPatternFile`); the PNG is the preset's
//!   thumbnail. A foreign pixel-brush preset's `brush_definition` auto brush, opacity, flow,
//!   spacing and pressure switches are mapped when its own param is absent.
//! * SVG gradients: `<linearGradient>` / `<radialGradient>` stops with `stop-color` and
//!   `stop-opacity` (attribute or `style`), one preset per gradient element.
//!
//! The PNG chunks are walked here (signature, length / type / data / CRC); the pixels go through
//! the codecs crate. Nothing here reads another application's source: the formats are their
//! public container layouts.

use std::io::{Read, Write};

use redrob_photo_paint::tile::{b64_decode, b64_encode};
use redrob_photo_paint::{BrushPreset, BrushSettings, ColorTile, GeneratedTip, MaskType, TipGeometry, TipShape};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, always};
use crate::presets::gradients::{GradientPreset, StopColor};
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn flag(p: &Value, k: &str, d: bool) -> bool {
    p.get(k).and_then(Value::as_bool).unwrap_or(d)
}

fn str_opt<'a>(p: &'a Value, k: &str) -> Option<&'a str> {
    p.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())
}

/// Largest resource file read or written.
pub const MAX_FILE_BYTES: usize = 256 << 20;
/// Largest entry read from a bundle.
const MAX_ENTRY: usize = 64 << 20;

// ---------------------------------------------------------------------------
// PNG chunks
// ---------------------------------------------------------------------------

const PNG_SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// `(type, data)` of every chunk of a PNG, in order.
pub fn png_chunks(bytes: &[u8]) -> Result<Vec<([u8; 4], &[u8])>> {
    if !bytes.starts_with(&PNG_SIG) {
        return Err(EngineError::Other("not a PNG file".into()));
    }
    let mut out = Vec::new();
    let mut at = 8usize;
    while at + 8 <= bytes.len() {
        let len = u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]) as usize;
        let ty = [bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]];
        let data = bytes.get(at + 8..at + 8 + len).ok_or_else(|| EngineError::Other("truncated PNG chunk".into()))?;
        out.push((ty, data));
        at += 12 + len;
        if &ty == b"IEND" {
            break;
        }
    }
    Ok(out)
}

/// The `tEXt`, `zTXt` and `iTXt` entries of a PNG as `(keyword, text)`.
pub fn png_text(bytes: &[u8]) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    for (ty, data) in png_chunks(bytes)? {
        let Some(nul) = data.iter().position(|&b| b == 0) else { continue };
        let key = String::from_utf8_lossy(&data[..nul]).into_owned();
        match &ty {
            b"tEXt" => out.push((key, String::from_utf8_lossy(&data[nul + 1..]).into_owned())),
            b"zTXt" => {
                let Some(comp) = data.get(nul + 2..) else { continue };
                let mut text = Vec::new();
                let mut dec = flate2::read::ZlibDecoder::new(comp);
                if (&mut dec).take(MAX_ENTRY as u64).read_to_end(&mut text).is_ok() {
                    out.push((key, String::from_utf8_lossy(&text).into_owned()));
                }
            }
            b"iTXt" => {
                // compression flag, method, language tag NUL, translated keyword NUL, text.
                let rest = &data[nul + 1..];
                if rest.len() < 2 {
                    continue;
                }
                let compressed = rest[0] == 1;
                let rest = &rest[2..];
                let Some(l) = rest.iter().position(|&b| b == 0) else { continue };
                let rest = &rest[l + 1..];
                let Some(t) = rest.iter().position(|&b| b == 0) else { continue };
                let text = &rest[t + 1..];
                if compressed {
                    let mut buf = Vec::new();
                    let mut dec = flate2::read::ZlibDecoder::new(text);
                    if (&mut dec).take(MAX_ENTRY as u64).read_to_end(&mut buf).is_ok() {
                        out.push((key, String::from_utf8_lossy(&buf).into_owned()));
                    }
                } else {
                    out.push((key, String::from_utf8_lossy(text).into_owned()));
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

fn png_chunk(out: &mut Vec<u8>, ty: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut body = Vec::with_capacity(4 + data.len());
    body.extend_from_slice(ty);
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&redrob_photo_format::zip::crc32(&body).to_be_bytes());
}

/// An 8-bit RGBA PNG with `tEXt` chunks (`texts`, written before the image data).
pub fn png_encode(width: u32, height: u32, rgba: &[u8], texts: &[(&str, &str)]) -> Result<Vec<u8>> {
    if rgba.len() != (width as usize) * (height as usize) * 4 || width == 0 || height == 0 {
        return Err(EngineError::Other("PNG pixel buffer size mismatch".into()));
    }
    let mut out = Vec::with_capacity(rgba.len() / 2 + 1024);
    out.extend_from_slice(&PNG_SIG);
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    png_chunk(&mut out, b"IHDR", &ihdr);
    for (k, v) in texts {
        let mut data = Vec::with_capacity(k.len() + 1 + v.len());
        data.extend_from_slice(k.as_bytes());
        data.push(0);
        data.extend_from_slice(v.as_bytes());
        png_chunk(&mut out, b"tEXt", &data);
    }
    let stride = width as usize * 4;
    let mut raw = Vec::with_capacity(rgba.len() + height as usize);
    for row in rgba.chunks_exact(stride) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(&raw).map_err(|e| EngineError::Other(e.to_string()))?;
    let idat = enc.finish().map_err(|e| EngineError::Other(e.to_string()))?;
    png_chunk(&mut out, b"IDAT", &idat);
    png_chunk(&mut out, b"IEND", &[]);
    Ok(out)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

// ---------------------------------------------------------------------------
// Native preset files (.kpp)
// ---------------------------------------------------------------------------

/// The paint-op id our presets are written with.
pub const PAINTOP_ID: &str = "redrob-brush";
/// The `version` text chunk.
pub const KPP_VERSION: &str = "5.0";

/// Side of the thumbnail a preset is written with.
const KPP_THUMB: u32 = 200;

/// The file name a preset's tip is referenced by (its stored key, else a name from the preset).
fn tip_file_name(preset: &BrushPreset) -> Option<String> {
    match &preset.brush.tip {
        TipShape::Round => None,
        TipShape::Stored(r) => Some(format!("{}.png", r.key)),
        TipShape::Sampled(_) => Some(format!("{}_tip.png", preset.name.replace(['/', '\\'], "_"))),
    }
}

/// The file name the preset's bitmap pattern is referenced by.
fn pattern_file_name(preset: &BrushPreset) -> Option<String> {
    match &preset.brush.texture.pattern {
        redrob_photo_paint::Pattern::Stored(r) => Some(format!("{}.pat", r.key)),
        redrob_photo_paint::Pattern::Tile(_) => Some(format!("{}_pattern.pat", preset.name.replace(['/', '\\'], "_"))),
        redrob_photo_paint::Pattern::Procedural { .. } => None,
    }
}

/// The preset XML document a `.kpp` carries.
pub fn preset_xml(preset: &BrushPreset, brush_json: &str) -> String {
    let mut xml = format!("<Preset name=\"{}\" paintopid=\"{PAINTOP_ID}\" version=\"{KPP_VERSION}\">\n", xml_escape(&preset.name));
    xml.push_str(&format!(" <param name=\"redrob/brush\" type=\"string\">{}</param>\n", xml_escape(brush_json)));
    xml.push_str(&format!(" <param name=\"redrob/group\" type=\"string\">{}</param>\n", xml_escape(&preset.group)));
    if !preset.folder.is_empty() {
        xml.push_str(&format!(" <param name=\"redrob/folder\" type=\"string\">{}</param>\n", xml_escape(&preset.folder.join("/"))));
    }
    xml.push_str(&format!(" <param name=\"OpacityValue\" type=\"string\">{}</param>\n", preset.brush.opacity));
    xml.push_str(&format!(" <param name=\"FlowValue\" type=\"string\">{}</param>\n", preset.brush.flow));
    xml.push_str(&format!(" <param name=\"PressureSize\" type=\"string\">{}</param>\n", preset.brush.pressure_size));
    xml.push_str(&format!(" <param name=\"PressureOpacity\" type=\"string\">{}</param>\n", preset.brush.pressure_opacity));
    if let Some(f) = tip_file_name(preset) {
        xml.push_str(&format!(" <param name=\"requiredBrushFile\" type=\"string\">{}</param>\n", xml_escape(&f)));
    }
    if let Some(f) = pattern_file_name(preset) {
        xml.push_str(&format!(" <param name=\"requiredPatternFile\" type=\"string\">{}</param>\n", xml_escape(&f)));
    }
    xml.push_str("</Preset>\n");
    xml
}

/// The thumbnail pixels a preset is written with: its custom thumbnail, else a stroke preview.
fn preset_thumbnail_rgba(s: &Session, preset: &BrushPreset) -> (u32, u32, Vec<u8>) {
    if let Some(t) = s.tools.preset_thumbnails.get(&preset.name) {
        let mut rgba = Vec::with_capacity((t.width * t.height * 4) as usize);
        for px in t.data.chunks_exact(3) {
            rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
        }
        return (t.width, t.height, rgba);
    }
    let mut b = preset.brush.clone();
    let _ = s.load_brush_tips(&mut b);
    let tile = crate::brush_extras_cmds::compose_thumbnail("white", [0.0; 3], Some(&b), "none", [0.0; 3], "", [0.0; 3]);
    match tile {
        Some(t) => {
            let mut rgba = Vec::with_capacity((t.width * t.height * 4) as usize);
            for px in t.data.chunks_exact(3) {
                rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            (t.width, t.height, rgba)
        }
        None => (KPP_THUMB, KPP_THUMB, vec![255; (KPP_THUMB * KPP_THUMB * 4) as usize]),
    }
}

/// Encode one preset as a `.kpp` file.
pub fn kpp_bytes(s: &Session, preset: &BrushPreset) -> Result<Vec<u8>> {
    let mut b = preset.brush.clone();
    // Tips and patterns travel inside the file (the required-file names are for libraries that
    // keep them apart), so load them before serialising.
    let _ = s.load_brush_tips(&mut b);
    let brush_json = serde_json::to_string(&b).map_err(|e| EngineError::Other(e.to_string()))?;
    let xml = preset_xml(preset, &brush_json);
    let (w, h, rgba) = preset_thumbnail_rgba(s, preset);
    png_encode(w, h, &rgba, &[("version", KPP_VERSION), ("preset", &xml)])
}

/// `brush.presets.exportKpp`: write a preset (`name`, else the current one) as a `.kpp` file
/// to `path`, or return its bytes as base64.
fn export_kpp(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.presets.exportKpp";
    let name = match str_opt(p, "name") {
        Some(n) => n.to_string(),
        None => s.tools.current_preset.clone().ok_or_else(|| bad(cmd, "missing `name`"))?,
    };
    let preset = redrob_photo_paint::presets::find(&s.tools.presets, &name).cloned().ok_or_else(|| bad(cmd, format!("no brush preset named `{name}`")))?;
    let bytes = kpp_bytes(s, &preset)?;
    let mut out =
        json!({ "name": preset.name, "bytes": bytes.len(), "requiredBrushFile": tip_file_name(&preset), "requiredPatternFile": pattern_file_name(&preset) });
    match str_opt(p, "path") {
        Some(path) => {
            crate::file_cmds::write_file(path, &bytes)?;
            out["path"] = json!(path);
        }
        None => out["data"] = json!(b64_encode(&bytes)),
    }
    Ok(out)
}

/// A foreign `brush_definition` auto brush (`<Brush type="auto_brush"><MaskGenerator …/>`)
/// mapped onto our computed tip; `None` for a predefined (bitmap) tip the file does not hold.
fn map_brush_definition(xml: &str, b: &mut BrushSettings, warnings: &mut Vec<String>) {
    let Ok(doc) = roxmltree::Document::parse(xml) else {
        warnings.push("the brush definition could not be read".into());
        return;
    };
    let root = doc.root_element();
    let f = |n: roxmltree::Node<'_, '_>, k: &str| n.attribute(k).and_then(|v| v.trim().parse::<f32>().ok()).filter(|v| v.is_finite());
    if let Some(sp) = f(root, "spacing") {
        b.spacing = sp.clamp(0.01, 10.0);
    }
    if let Some(a) = f(root, "angle") {
        b.angle = a.to_degrees().rem_euclid(360.0);
    }
    if root.attribute("useAutoSpacing").is_some_and(|v| v == "true" || v == "1") {
        b.generated.auto_spacing = true;
        if let Some(c) = f(root, "autoSpacingCoeff") {
            b.generated.auto_spacing_coeff = c.max(0.01);
        }
    }
    match root.attribute("type") {
        Some("auto_brush") => {
            let Some(m) = root.children().find(|c| c.is_element() && c.tag_name().name() == "MaskGenerator") else { return };
            let d = f(m, "diameter").unwrap_or(b.size).clamp(0.5, redrob_photo_paint::MAX_BRUSH_SIZE);
            b.resize_with_dual_brush(d);
            b.roundness = f(m, "ratio").unwrap_or(1.0).clamp(0.01, 1.0);
            let mut g =
                GeneratedTip { shape: if m.attribute("type") == Some("rect") { TipGeometry::Square } else { TipGeometry::Circle }, ..GeneratedTip::default() };
            g.fade_x = f(m, "hfade").unwrap_or(1.0).clamp(0.0, 1.0);
            g.fade_y = f(m, "vfade").unwrap_or(g.fade_x).clamp(0.0, 1.0);
            g.lock_fade = (g.fade_x - g.fade_y).abs() < 1e-4;
            g.spikes = f(m, "spikes").map_or(0, |v| v.max(0.0) as u32);
            g.density = f(m, "density").map_or(1.0, |v| if v > 1.0 { v / 100.0 } else { v }).clamp(0.0, 1.0);
            g.randomness = f(m, "randomness").map_or(0.0, |v| if v > 1.0 { v / 100.0 } else { v }).clamp(0.0, 1.0);
            g.antialias = m.attribute("antialiasEdges").is_none_or(|v| v == "true" || v == "1");
            g.mask = match m.attribute("id") {
                Some("gauss") => MaskType::Gaussian,
                Some("soft") => MaskType::Soft,
                _ => MaskType::Default,
            };
            if g.mask == MaskType::Default {
                // The default mask reads hardness: the fade stands for it.
                b.hardness = g.fade_x.min(g.fade_y).clamp(0.0, 1.0);
            }
            b.generated = g;
            b.tip = TipShape::Round;
        }
        Some(other) => {
            warnings.push(format!("the preset's {other} tip bitmap is not in the file (import the brush file it names); a round tip stands in"));
            if let Some(d) = f(root, "diameter") {
                b.resize_with_dual_brush(d.clamp(0.5, redrob_photo_paint::MAX_BRUSH_SIZE));
            }
        }
        None => {}
    }
}

/// Read a `.kpp` into a preset: ours from its `redrob/brush` param, a foreign one from the
/// common pixel-brush params. Returns the preset, the thumbnail and warnings.
pub fn read_kpp(bytes: &[u8], fallback_name: &str) -> Result<(BrushPreset, Option<ColorTile>, Vec<String>)> {
    let texts = png_text(bytes)?;
    let preset_xml = texts
        .iter()
        .find(|(k, _)| k == "preset")
        .map(|(_, v)| v.as_str())
        .ok_or_else(|| EngineError::Other("the PNG has no `preset` text chunk: not a preset file".into()))?;
    let doc = roxmltree::Document::parse(preset_xml).map_err(|e| EngineError::Other(format!("bad preset XML: {e}")))?;
    let root = doc.root_element();
    let mut warnings = Vec::new();
    let name = root.attribute("name").map(str::trim).filter(|n| !n.is_empty()).unwrap_or(fallback_name).to_string();
    let paintop = root.attribute("paintopid").unwrap_or("").to_string();
    let param = |k: &str| {
        root.children().find(|c| c.is_element() && c.tag_name().name() == "param" && c.attribute("name") == Some(k)).and_then(|c| c.text()).map(str::to_string)
    };
    let mut brush = BrushSettings::default();
    let (mut group, mut folder) = (String::from("Imported Presets"), Vec::new());
    if let Some(j) = param("redrob/brush") {
        brush = serde_json::from_str(&j).map_err(|e| EngineError::Other(format!("bad brush settings in preset: {e}")))?;
        if let Some(g) = param("redrob/group").filter(|g| !g.trim().is_empty()) {
            group = g;
        }
        if let Some(f) = param("redrob/folder") {
            folder = f.split('/').filter(|s| !s.is_empty()).map(str::to_string).collect();
        }
    } else {
        if paintop != "paintbrush" && !paintop.is_empty() {
            warnings.push(format!("the preset uses the `{paintop}` engine; its common options were mapped onto the pixel brush"));
        }
        if let Some(def) = param("brush_definition") {
            map_brush_definition(&def, &mut brush, &mut warnings);
        }
        let num = |k: &str| param(k).and_then(|v| v.trim().parse::<f32>().ok()).filter(|v| v.is_finite());
        let boolean = |k: &str| param(k).map(|v| v.trim() == "true" || v.trim() == "1");
        if let Some(o) = num("OpacityValue") {
            brush.opacity = o.clamp(0.0, 1.0);
        }
        if let Some(f) = num("FlowValue") {
            brush.flow = f.clamp(0.0, 1.0);
        }
        if let Some(v) = boolean("PressureSize") {
            brush.pressure_size = v;
        }
        if let Some(v) = boolean("PressureOpacity") {
            brush.pressure_opacity = v;
        }
        if param("EraserMode").is_some_and(|v| v.trim() == "true") {
            brush.erase = true;
        }
        if let Some(f) = param("requiredBrushFile").filter(|f| !f.trim().is_empty()) {
            warnings.push(format!("the preset needs the tip file `{}`; import it with brush.presets.importBrushFile", f.trim()));
        }
        if let Some(f) = param("requiredPatternFile").filter(|f| !f.trim().is_empty()) {
            warnings.push(format!("the preset needs the pattern file `{}`; import it with pattern.import", f.trim()));
        }
    }
    // The PNG image is the thumbnail.
    let thumb =
        redrob_photo_codecs::decode(bytes).ok().filter(|img| img.width() > 0 && img.height() > 0 && img.width() <= 1024 && img.height() <= 1024).map(|img| {
            let rgba = img.to_rgba8();
            let rgb: Vec<u8> = rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect();
            ColorTile::from_rgb8(img.width(), img.height(), &rgb)
        });
    Ok((BrushPreset { name, brush, builtin: false, group, folder, captures: Default::default() }, thumb, warnings))
}

/// `brush.presets.importKpp`: read a `.kpp` file (`path` or `data`) into the presets.
fn import_kpp(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "brush.presets.importKpp";
    let (bytes, stem) = crate::preset_import_cmds::file_bytes(p, cmd, ".kpp preset file")?;
    let fallback =
        str_opt(p, "fileName").map(|n| std::path::Path::new(n).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()).unwrap_or(stem);
    let fallback = if fallback.trim().is_empty() { "Imported Preset".to_string() } else { fallback };
    let (mut preset, thumb, warnings) = read_kpp(&bytes, &fallback).map_err(|e| bad(cmd, e.to_string()))?;
    if let Some(g) = str_opt(p, "group") {
        preset.group = g.to_string();
    }
    if flag(p, "setCurrent", false) {
        s.tools.brush = preset.picked_over(&s.tools.brush);
        s.tools.current_preset = Some(preset.name.clone());
        s.tools.brush_changed();
    }
    let name = preset.name.clone();
    let group = preset.group.clone();
    if let Some(t) = thumb {
        s.tools.preset_thumbnails.insert(name.clone(), t);
    }
    crate::brush_cmds::upsert(s, preset);
    Ok(json!({ "name": name, "group": group, "warnings": warnings }))
}

// ---------------------------------------------------------------------------
// Storages
// ---------------------------------------------------------------------------

/// Where resources come from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StorageKind {
    /// The user's resource folder (the preset store).
    Folder,
    /// An imported resource bundle.
    Bundle,
    /// An imported Adobe brush (`.abr`) or style (`.asl`) library.
    AdobeLibrary,
    /// The open document (palettes it stores).
    Document,
    /// Resources made in this session and not saved anywhere.
    Memory,
}

impl StorageKind {
    pub fn name(self) -> &'static str {
        match self {
            StorageKind::Folder => "folder",
            StorageKind::Bundle => "bundle",
            StorageKind::AdobeLibrary => "adobeLibrary",
            StorageKind::Document => "document",
            StorageKind::Memory => "memory",
        }
    }
}

/// One storage: its kind, location, whether it is active, the preset groups it brought and,
/// while inactive, the presets parked away.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Storage {
    pub id: String,
    pub kind: StorageKind,
    pub name: String,
    pub location: String,
    pub active: bool,
    pub groups: Vec<String>,
    pub gradient_groups: Vec<String>,
    pub swatch_groups: Vec<String>,
    #[serde(skip)]
    pub parked: Vec<BrushPreset>,
    /// Bundle metadata (author, description, license, website) when known.
    pub meta: std::collections::BTreeMap<String, String>,
}

impl Default for Storage {
    fn default() -> Self {
        Self {
            id: String::new(),
            kind: StorageKind::Memory,
            name: String::new(),
            location: String::new(),
            active: true,
            groups: Vec::new(),
            gradient_groups: Vec::new(),
            swatch_groups: Vec::new(),
            parked: Vec::new(),
            meta: Default::default(),
        }
    }
}

/// The session's storages (the built-in ones are listed even though they hold no record).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StorageState {
    pub items: Vec<Storage>,
}

impl StorageState {
    fn unique_id(&self, base: &str) -> String {
        let base: String = base.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c.to_ascii_lowercase() } else { '-' }).collect();
        let base = if base.is_empty() { "storage".to_string() } else { base };
        if !self.items.iter().any(|s| s.id == base) {
            return base;
        }
        (2..).map(|i| format!("{base}-{i}")).find(|id| !self.items.iter().any(|s| s.id == *id)).unwrap_or(base)
    }

    /// Register (or replace) a storage; returns its id.
    pub fn register(&mut self, mut st: Storage) -> String {
        if let Some(i) = self.items.iter().position(|s| s.kind == st.kind && s.location == st.location && !st.location.is_empty()) {
            st.id = self.items[i].id.clone();
            self.items[i] = st;
            return self.items[i].id.clone();
        }
        if st.id.is_empty() {
            st.id = self.unique_id(&st.name);
        }
        let id = st.id.clone();
        self.items.push(st);
        id
    }
}

fn storage_json(st: &Storage, presets: usize) -> Value {
    json!({
        "id": st.id,
        "kind": st.kind.name(),
        "name": st.name,
        "location": st.location,
        "active": st.active,
        "groups": st.groups,
        "gradientGroups": st.gradient_groups,
        "swatchGroups": st.swatch_groups,
        "presets": presets,
        "meta": st.meta,
    })
}

/// `resources.storages.list`: every storage, the built-in ones first.
fn storages_list(s: &mut Session, _: &Value) -> Result<Value> {
    let folder = s.preset_store.as_ref().map(|_| "preset store").unwrap_or("(none attached)");
    let user_presets = s.tools.presets.iter().filter(|p| !p.builtin && !s.resource_storages.items.iter().any(|st| st.groups.contains(&p.group))).count();
    let doc_palettes = s.active().map_or(0, |d| d.doc.palettes.len());
    let mut items = vec![
        json!({ "id": "user", "kind": StorageKind::Folder.name(), "name": "User folder", "location": folder, "active": true, "builtin": true, "presets": user_presets }),
        json!({ "id": "builtin", "kind": StorageKind::Memory.name(), "name": "Built-in", "location": "", "active": true, "builtin": true, "presets": s.tools.presets.iter().filter(|p| p.builtin).count() }),
        json!({ "id": "document", "kind": StorageKind::Document.name(), "name": "Document", "location": s.active().and_then(|d| d.path.clone()).unwrap_or_default(), "active": s.active().is_some(), "builtin": true, "palettes": doc_palettes }),
    ];
    for st in &s.resource_storages.items {
        let n = if st.active { s.tools.presets.iter().filter(|p| st.groups.contains(&p.group)).count() } else { st.parked.len() };
        items.push(storage_json(st, n));
    }
    Ok(json!({ "storages": items }))
}

/// `resources.storages.activate`: activate or deactivate a storage. Deactivating parks the
/// brush presets it brought (and hides its gradient and swatch groups); activating restores them.
fn storages_activate(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "resources.storages.activate";
    let id = str_opt(p, "id").ok_or_else(|| bad(cmd, "missing `id`"))?.to_string();
    let active = p.get("active").and_then(Value::as_bool).unwrap_or(true);
    if matches!(id.as_str(), "user" | "builtin" | "document") {
        return Err(bad(cmd, format!("the `{id}` storage is always active")));
    }
    let i = s.resource_storages.items.iter().position(|st| st.id == id).ok_or_else(|| bad(cmd, format!("no storage `{id}`")))?;
    let was = s.resource_storages.items[i].active;
    if was == active {
        return Ok(json!({ "id": id, "active": active, "unchanged": true }));
    }
    let groups = s.resource_storages.items[i].groups.clone();
    if active {
        let parked = std::mem::take(&mut s.resource_storages.items[i].parked);
        for pr in parked {
            if !s.tools.presets.iter().any(|x| x.name.eq_ignore_ascii_case(&pr.name)) {
                s.tools.presets.push(pr);
            }
        }
    } else {
        let (parked, kept): (Vec<BrushPreset>, Vec<BrushPreset>) =
            std::mem::take(&mut s.tools.presets).into_iter().partition(|pr| !pr.builtin && groups.contains(&pr.group));
        s.tools.presets = kept;
        if s.tools.current_preset.as_deref().is_some_and(|c| parked.iter().any(|pr| pr.name.eq_ignore_ascii_case(c))) {
            s.tools.current_preset = None;
            s.tools.brush_changed();
        }
        s.resource_storages.items[i].parked = parked;
    }
    s.resource_storages.items[i].active = active;
    s.brush_presets_changed();
    let n = if active { s.tools.presets.iter().filter(|p| groups.contains(&p.group)).count() } else { s.resource_storages.items[i].parked.len() };
    Ok(json!({ "id": id, "active": active, "presets": n }))
}

/// `resources.storages.remove`: forget a storage and delete the presets it brought.
fn storages_remove(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "resources.storages.remove";
    let id = str_opt(p, "id").ok_or_else(|| bad(cmd, "missing `id`"))?.to_string();
    if matches!(id.as_str(), "user" | "builtin" | "document") {
        return Err(bad(cmd, format!("the `{id}` storage cannot be removed")));
    }
    let i = s.resource_storages.items.iter().position(|st| st.id == id).ok_or_else(|| bad(cmd, format!("no storage `{id}`")))?;
    let st = s.resource_storages.items.remove(i);
    let before = s.tools.presets.len();
    s.tools.presets.retain(|pr| pr.builtin || !st.groups.contains(&pr.group));
    let removed = before - s.tools.presets.len() + st.parked.len();
    for g in &st.gradient_groups {
        s.presets.gradients.retain(|x| x.name != *g);
    }
    for g in &st.swatch_groups {
        s.presets.swatches.retain(|x| x.name != *g);
    }
    s.brush_presets_changed();
    s.presets_changed();
    Ok(json!({ "id": id, "removedPresets": removed }))
}

/// `resources.storages.add`: register a storage for resources already imported (an Adobe
/// library's group, for example), so it can be deactivated like a bundle.
fn storages_add(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "resources.storages.add";
    let kind = match str_opt(p, "kind").unwrap_or("adobeLibrary") {
        "bundle" => StorageKind::Bundle,
        "adobeLibrary" => StorageKind::AdobeLibrary,
        "folder" => StorageKind::Folder,
        "memory" => StorageKind::Memory,
        other => return Err(bad(cmd, format!("unknown kind `{other}` (bundle, adobeLibrary, folder, memory)"))),
    };
    let groups: Vec<String> =
        p.get("groups").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    if groups.is_empty() {
        return Err(bad(cmd, "`groups` must name at least one preset group"));
    }
    let name = str_opt(p, "name").map(str::to_string).unwrap_or_else(|| groups[0].clone());
    let st = Storage { kind, name: name.clone(), location: str_opt(p, "location").unwrap_or("").to_string(), groups, ..Storage::default() };
    let id = s.resource_storages.register(st);
    Ok(json!({ "id": id, "name": name }))
}

// ---------------------------------------------------------------------------
// Bundles
// ---------------------------------------------------------------------------

/// The mimetype entry of a bundle.
pub const BUNDLE_MIMETYPE: &str = "application/x-krita-resourcebundle";

fn bundle_meta_xml(meta: &std::collections::BTreeMap<String, String>) -> String {
    let get = |k: &str| xml_escape(meta.get(k).map(String::as_str).unwrap_or(""));
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<meta:meta xmlns:meta=\"http://www.calligra.org/meta/\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n <meta:generator>Redrob Canvas</meta:generator>\n <dc:author>{}</dc:author>\n <dc:title>{}</dc:title>\n <dc:description>{}</dc:description>\n <dc:license>{}</dc:license>\n <dc:website>{}</dc:website>\n <dc:email>{}</dc:email>\n <dc:date>{}</dc:date>\n</meta:meta>\n",
        get("author"),
        get("title"),
        get("description"),
        get("license"),
        get("website"),
        get("email"),
        get("date")
    )
}

/// Parse `meta.xml` into its Dublin Core fields.
fn parse_bundle_meta(xml: &str) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    if let Ok(doc) = roxmltree::Document::parse(xml) {
        for n in doc.root_element().children().filter(|n| n.is_element()) {
            let k = n.tag_name().name();
            if let Some(t) = n.text().map(str::trim).filter(|t| !t.is_empty()) {
                out.insert(k.to_string(), t.to_string());
            }
        }
    }
    out
}

fn manifest_xml(entries: &[(String, String, Vec<String>)]) -> String {
    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" manifest:version=\"1.2\">\n <manifest:file-entry manifest:media-type=\"application/x-krita-resourcebundle\" manifest:full-path=\"/\"/>\n",
    );
    for (media, path, tags) in entries {
        xml.push_str(&format!(" <manifest:file-entry manifest:media-type=\"{}\" manifest:full-path=\"{}\">\n", xml_escape(media), xml_escape(path)));
        for t in tags {
            xml.push_str(&format!("  <manifest:tags><manifest:tag>{}</manifest:tag></manifest:tags>\n", xml_escape(t)));
        }
        xml.push_str(" </manifest:file-entry>\n");
    }
    xml.push_str("</manifest:manifest>\n");
    xml
}

/// Entries of the bundle's manifest: `(media type, path, tags)`.
fn parse_manifest(xml: &str) -> Vec<(String, String, Vec<String>)> {
    let mut out = Vec::new();
    let Ok(doc) = roxmltree::Document::parse(xml) else { return out };
    for n in doc.root_element().children().filter(|n| n.is_element() && n.tag_name().name() == "file-entry") {
        let media =
            n.attribute(("urn:oasis:names:tc:opendocument:xmlns:manifest:1.0", "media-type")).or_else(|| n.attribute("media-type")).unwrap_or("").to_string();
        let path =
            n.attribute(("urn:oasis:names:tc:opendocument:xmlns:manifest:1.0", "full-path")).or_else(|| n.attribute("full-path")).unwrap_or("").to_string();
        let tags: Vec<String> = n
            .descendants()
            .filter(|t| t.is_element() && t.tag_name().name() == "tag")
            .filter_map(|t| t.text())
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();
        if path != "/" && !path.is_empty() {
            out.push((media, path, tags));
        }
    }
    out
}

/// `resources.bundle.import`: read a bundle and import what it holds.
fn bundle_import(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "resources.bundle.import";
    let (bytes, stem) = crate::preset_import_cmds::file_bytes(p, cmd, ".bundle file")?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err(bad(cmd, "the bundle is too large"));
    }
    let zip = redrob_photo_format::zip::ZipReader::new(&bytes).map_err(|e| bad(cmd, format!("not a bundle (zip): {e}")))?;
    let meta = zip.read_by_name("meta.xml", MAX_ENTRY).ok().map(|m| parse_bundle_meta(&String::from_utf8_lossy(&m))).unwrap_or_default();
    let manifest = zip.read_by_name("META-INF/manifest.xml", MAX_ENTRY).map_err(|e| bad(cmd, format!("the bundle has no manifest: {e}")))?;
    let entries = parse_manifest(&String::from_utf8_lossy(&manifest));
    let name = str_opt(p, "name")
        .map(str::to_string)
        .or_else(|| meta.get("title").cloned())
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| if stem.is_empty() { "Bundle".to_string() } else { stem.clone() });
    let group = name.clone();
    let mut imported = json!({ "presets": [], "brushes": [], "gradients": [], "palettes": [], "patterns": [], "skipped": [] });
    let mut warnings: Vec<String> = Vec::new();
    let mut tags_by_preset: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
    let push = |v: &mut Value, k: &str, item: Value| {
        if let Some(a) = v[k].as_array_mut() {
            a.push(item);
        }
    };
    for (_media, path, tags) in &entries {
        let Some(e) = zip.find(path) else {
            warnings.push(format!("manifest names `{path}`, which the bundle does not hold"));
            continue;
        };
        let data = match zip.read(e, MAX_ENTRY) {
            Ok(d) => d,
            Err(err) => {
                warnings.push(format!("`{path}`: {err}"));
                continue;
            }
        };
        let file = std::path::Path::new(path);
        let folder = file.parent().and_then(|d| d.file_name()).and_then(|d| d.to_str()).unwrap_or("");
        let ext = file.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
        let file_name = file.file_name().and_then(|f| f.to_str()).unwrap_or(path);
        let b64 = b64_encode(&data);
        match (folder, ext.as_str()) {
            ("paintoppresets", "kpp") => match s.execute("brush.presets.importKpp", json!({ "data": b64, "fileName": file_name, "group": group })) {
                Ok(r) => {
                    if let Some(n) = r.get("name").and_then(Value::as_str) {
                        if !tags.is_empty() {
                            tags_by_preset.insert(n.to_string(), tags.clone());
                        }
                        push(&mut imported, "presets", json!(n));
                    }
                    if let Some(w) = r.get("warnings").and_then(Value::as_array) {
                        warnings.extend(w.iter().filter_map(Value::as_str).map(|w| format!("{file_name}: {w}")));
                    }
                }
                Err(err) => warnings.push(format!("{file_name}: {err}")),
            },
            ("brushes", "gbr" | "gih") => match s.execute("brush.presets.importBrushFile", json!({ "data": b64, "fileName": file_name, "group": group, "replace": false })) {
                Ok(r) => {
                    if let Some(a) = r.get("imported").and_then(Value::as_array) {
                        for n in a {
                            push(&mut imported, "brushes", n.clone());
                        }
                    }
                }
                Err(err) => warnings.push(format!("{file_name}: {err}")),
            },
            ("brushes", "png") => match s.execute("brush.presets.importImage", json!({ "data": b64, "fileName": file_name, "name": file.file_stem().and_then(|s| s.to_str()).unwrap_or("Brush"), "group": group, "select": false })) {
                Ok(r) => push(&mut imported, "brushes", r.get("name").cloned().unwrap_or(json!(file_name))),
                Err(err) => warnings.push(format!("{file_name}: {err}")),
            },
            ("gradients", "ggr") => match s.execute("gradient.presets.importGgr", json!({ "data": b64, "group": group })) {
                Ok(r) => {
                    if let Some(a) = r.get("imported").and_then(Value::as_array) {
                        for n in a {
                            push(&mut imported, "gradients", n.clone());
                        }
                    }
                }
                Err(err) => warnings.push(format!("{file_name}: {err}")),
            },
            ("gradients", "svg") => match s.execute("gradient.presets.importSvg", json!({ "data": b64, "group": group })) {
                Ok(r) => {
                    if let Some(a) = r.get("imported").and_then(Value::as_array) {
                        for n in a {
                            push(&mut imported, "gradients", n.clone());
                        }
                    }
                }
                Err(err) => warnings.push(format!("{file_name}: {err}")),
            },
            ("palettes", "gpl" | "kpl" | "aco" | "ase" | "act" | "pal") => {
                match s.execute("swatches.import", json!({ "data": b64, "format": ext, "group": file.file_stem().and_then(|s| s.to_str()).unwrap_or(&group) })) {
                    Ok(r) => {
                        if let Some(a) = r.get("groups").and_then(Value::as_array) {
                            for n in a {
                                push(&mut imported, "palettes", n.clone());
                            }
                        }
                    }
                    Err(err) => warnings.push(format!("{file_name}: {err}")),
                }
            }
            ("patterns", "pat") => match s.execute("pattern.import", json!({ "data": b64 })) {
                Ok(r) => {
                    if let Some(a) = r.get("imported").and_then(Value::as_array) {
                        for n in a {
                            push(&mut imported, "patterns", n.clone());
                        }
                    }
                }
                Err(err) => warnings.push(format!("{file_name}: {err}")),
            },
            _ => push(&mut imported, "skipped", json!(path)),
        }
    }
    for (name, tags) in tags_by_preset {
        for t in tags {
            let _ = s.execute("brush.presets.tag", json!({ "name": name, "tag": t }));
        }
    }
    let gradient_groups: Vec<String> = if imported["gradients"].as_array().is_some_and(|a| !a.is_empty()) { vec![group.clone()] } else { Vec::new() };
    let swatch_groups: Vec<String> =
        imported["palettes"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    let st = Storage {
        kind: StorageKind::Bundle,
        name: name.clone(),
        location: str_opt(p, "path").unwrap_or("").to_string(),
        groups: vec![group.clone()],
        gradient_groups,
        swatch_groups,
        meta,
        ..Storage::default()
    };
    let id = s.resource_storages.register(st);
    Ok(json!({ "storage": id, "name": name, "group": group, "imported": imported, "warnings": warnings }))
}

/// `resources.bundle.create`: write a bundle of the chosen presets (`presets`, or every preset
/// with one of `tags`), swatch groups (`palettes`) and gradient groups (`gradients`), with
/// `name`, `author`, `description`, `license`, `website`, `email`.
fn bundle_create(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "resources.bundle.create";
    let name = str_opt(p, "name").ok_or_else(|| bad(cmd, "missing `name`"))?.to_string();
    let names: Vec<String> =
        p.get("presets").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    let tags: Vec<String> =
        p.get("tags").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    let mut chosen: Vec<BrushPreset> = Vec::new();
    for n in &names {
        let pr = redrob_photo_paint::presets::find(&s.tools.presets, n).cloned().ok_or_else(|| bad(cmd, format!("no brush preset named `{n}`")))?;
        chosen.push(pr);
    }
    for pr in &s.tools.presets {
        if chosen.iter().any(|c| c.name == pr.name) {
            continue;
        }
        if s.tools.preset_tags.get(&pr.name).is_some_and(|t| t.iter().any(|x| tags.iter().any(|w| w.eq_ignore_ascii_case(x)))) {
            chosen.push(pr.clone());
        }
    }
    let palettes: Vec<String> =
        p.get("palettes").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    let gradients: Vec<String> =
        p.get("gradients").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    if chosen.is_empty() && palettes.is_empty() && gradients.is_empty() {
        return Err(bad(cmd, "nothing to bundle: give `presets`, `tags`, `palettes` or `gradients`"));
    }
    let mut meta = std::collections::BTreeMap::new();
    meta.insert("title".to_string(), name.clone());
    for k in ["author", "description", "license", "website", "email"] {
        if let Some(v) = str_opt(p, k) {
            meta.insert(k.to_string(), v.to_string());
        }
    }
    meta.insert("date".to_string(), format!("{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())));
    let safe = |n: &str| -> String { n.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { '_' }).collect() };
    let mut zip = redrob_photo_format::zip::ZipWriter::new();
    let mut entries: Vec<(String, String, Vec<String>)> = Vec::new();
    zip.add("mimetype", BUNDLE_MIMETYPE.as_bytes()).map_err(|e| bad(cmd, e.to_string()))?;
    for pr in &chosen {
        let bytes = kpp_bytes(s, pr)?;
        let path = format!("paintoppresets/{}.kpp", safe(&pr.name));
        zip.add(&path, &bytes).map_err(|e| bad(cmd, e.to_string()))?;
        let t = s.tools.preset_tags.get(&pr.name).cloned().unwrap_or_default();
        entries.push(("paintoppresets".to_string(), path, t));
    }
    for g in &palettes {
        let grp = s.presets.swatches.iter().find(|x| x.name == *g).ok_or_else(|| bad(cmd, format!("no swatch group `{g}`")))?;
        let pal = redrob_photo_io::palette_files::Palette {
            name: g.clone(),
            columns: 8,
            groups: vec![redrob_photo_io::palette_files::PalGroup {
                name: String::new(),
                entries: grp
                    .items
                    .iter()
                    .map(|sw| redrob_photo_io::palette_files::PalEntry {
                        name: sw.name.clone(),
                        color: match sw.color {
                            crate::presets::swatches::SwatchColor::Rgb(v) => redrob_photo_io::palette_files::PalColor::Rgb(v),
                            crate::presets::swatches::SwatchColor::Hsb(v) => {
                                redrob_photo_io::palette_files::PalColor::Rgb(crate::presets::swatches::hsb_to_rgb(v))
                            }
                            crate::presets::swatches::SwatchColor::Cmyk(v) => redrob_photo_io::palette_files::PalColor::Cmyk(v),
                            crate::presets::swatches::SwatchColor::Lab(v) => redrob_photo_io::palette_files::PalColor::Lab(v),
                            crate::presets::swatches::SwatchColor::Gray(k) => redrob_photo_io::palette_files::PalColor::Gray(k),
                            crate::presets::swatches::SwatchColor::Book { .. } => redrob_photo_io::palette_files::PalColor::Gray(0.5),
                        },
                        spot: sw.kind == crate::presets::swatches::SwatchKind::Spot,
                    })
                    .collect(),
            }],
            warnings: Vec::new(),
        };
        let bytes = redrob_photo_io::palette_files::write_kpl(&pal).map_err(|e| bad(cmd, e))?;
        let path = format!("palettes/{}.kpl", safe(g));
        zip.add(&path, &bytes).map_err(|e| bad(cmd, e.to_string()))?;
        entries.push(("palettes".to_string(), path, Vec::new()));
    }
    let (fg, bg) = (s.tools.foreground, s.tools.background);
    for g in &gradients {
        let grp = s.presets.gradients.iter().find(|x| x.name == *g).ok_or_else(|| bad(cmd, format!("no gradient group `{g}`")))?;
        for gr in &grp.items {
            let svg = gradient_svg(gr, fg, bg);
            let path = format!("gradients/{}.svg", safe(&gr.name));
            zip.add(&path, svg.as_bytes()).map_err(|e| bad(cmd, e.to_string()))?;
            entries.push(("gradients".to_string(), path, Vec::new()));
        }
    }
    // The preview: the first preset's thumbnail, else a plain tile.
    let (w, h, rgba) = match chosen.first() {
        Some(pr) => preset_thumbnail_rgba(s, pr),
        None => (64, 64, vec![230; 64 * 64 * 4]),
    };
    zip.add("preview.png", &png_encode(w, h, &rgba, &[])?).map_err(|e| bad(cmd, e.to_string()))?;
    zip.add("meta.xml", bundle_meta_xml(&meta).as_bytes()).map_err(|e| bad(cmd, e.to_string()))?;
    zip.add("META-INF/manifest.xml", manifest_xml(&entries).as_bytes()).map_err(|e| bad(cmd, e.to_string()))?;
    let bytes = zip.finish().map_err(|e| bad(cmd, e.to_string()))?;
    let mut out = json!({ "name": name, "presets": chosen.iter().map(|p| p.name.clone()).collect::<Vec<_>>(), "palettes": palettes, "gradients": gradients, "bytes": bytes.len() });
    match str_opt(p, "path") {
        Some(path) => {
            crate::file_cmds::write_file(path, &bytes)?;
            out["path"] = json!(path);
            // The bundle just written is a storage too (active, its presets being the ones chosen).
            let st = Storage { kind: StorageKind::Bundle, name: name.clone(), location: path.to_string(), groups: Vec::new(), meta, ..Storage::default() };
            out["storage"] = json!(s.resource_storages.register(st));
        }
        None => out["data"] = json!(b64_encode(&bytes)),
    }
    Ok(out)
}

/// `resources.bundle.list`: the imported bundles.
fn bundle_list(s: &mut Session, _: &Value) -> Result<Value> {
    let items: Vec<Value> = s
        .resource_storages
        .items
        .iter()
        .filter(|st| st.kind == StorageKind::Bundle)
        .map(|st| storage_json(st, if st.active { s.tools.presets.iter().filter(|p| st.groups.contains(&p.group)).count() } else { st.parked.len() }))
        .collect();
    Ok(json!({ "bundles": items }))
}

// ---------------------------------------------------------------------------
// SVG gradients
// ---------------------------------------------------------------------------

fn parse_css_color(s: &str) -> Option<[f32; 3]> {
    let s = s.trim();
    if let Some(h) = s.strip_prefix('#') {
        let v = |a: usize, b: usize| u8::from_str_radix(&h[a..b], 16).ok().map(|x| f32::from(x) / 255.0);
        return match h.len() {
            6 | 8 => Some([v(0, 2)?, v(2, 4)?, v(4, 6)?]),
            3 | 4 => {
                let d = |i: usize| u8::from_str_radix(&h[i..i + 1], 16).ok().map(|x| f32::from(x * 17) / 255.0);
                Some([d(0)?, d(1)?, d(2)?])
            }
            _ => None,
        };
    }
    if let Some(inner) = s.strip_prefix("rgb(").or_else(|| s.strip_prefix("rgba(")).and_then(|r| r.strip_suffix(')')) {
        let parts: Vec<&str> = inner.split(',').map(str::trim).collect();
        if parts.len() >= 3 {
            let c = |t: &str| -> Option<f32> {
                if let Some(pc) = t.strip_suffix('%') { pc.trim().parse::<f32>().ok().map(|v| v / 100.0) } else { t.parse::<f32>().ok().map(|v| v / 255.0) }
            };
            return Some([c(parts[0])?.clamp(0.0, 1.0), c(parts[1])?.clamp(0.0, 1.0), c(parts[2])?.clamp(0.0, 1.0)]);
        }
    }
    let named = match s.to_ascii_lowercase().as_str() {
        "black" => [0.0, 0.0, 0.0],
        "white" => [1.0, 1.0, 1.0],
        "red" => [1.0, 0.0, 0.0],
        "green" => [0.0, 0.5, 0.0],
        "lime" => [0.0, 1.0, 0.0],
        "blue" => [0.0, 0.0, 1.0],
        "yellow" => [1.0, 1.0, 0.0],
        "cyan" | "aqua" => [0.0, 1.0, 1.0],
        "magenta" | "fuchsia" => [1.0, 0.0, 1.0],
        "gray" | "grey" => [0.5, 0.5, 0.5],
        "orange" => [1.0, 0.647, 0.0],
        "purple" => [0.5, 0.0, 0.5],
        _ => return None,
    };
    Some(named)
}

fn style_prop<'a>(style: &'a str, key: &str) -> Option<&'a str> {
    style.split(';').filter_map(|d| d.split_once(':')).find(|(k, _)| k.trim() == key).map(|(_, v)| v.trim())
}

/// The gradients of an SVG document: `(id or name, stops (offset, rgb, opacity))`.
pub fn svg_gradients(text: &str) -> Result<Vec<(String, Vec<(f32, [f32; 3], f32)>)>> {
    let doc = roxmltree::Document::parse(text).map_err(|e| EngineError::Other(format!("not an SVG document: {e}")))?;
    let mut out = Vec::new();
    for (i, g) in doc.descendants().filter(|n| n.is_element() && matches!(n.tag_name().name(), "linearGradient" | "radialGradient")).enumerate() {
        let name = g
            .attribute("id")
            .or_else(|| g.attribute(("http://www.inkscape.org/namespaces/inkscape", "label")))
            .map(str::to_string)
            .unwrap_or_else(|| format!("Gradient {}", i + 1));
        let mut stops = Vec::new();
        for st in g.children().filter(|n| n.is_element() && n.tag_name().name() == "stop") {
            let offset = st
                .attribute("offset")
                .map(str::trim)
                .map(|o| if let Some(pc) = o.strip_suffix('%') { pc.trim().parse::<f32>().unwrap_or(0.0) / 100.0 } else { o.parse::<f32>().unwrap_or(0.0) })
                .unwrap_or(0.0)
                .clamp(0.0, 1.0);
            let style = st.attribute("style").unwrap_or("");
            let color = st.attribute("stop-color").or_else(|| style_prop(style, "stop-color")).and_then(parse_css_color).unwrap_or([0.0, 0.0, 0.0]);
            let opacity = st
                .attribute("stop-opacity")
                .or_else(|| style_prop(style, "stop-opacity"))
                .and_then(|o| o.trim().parse::<f32>().ok())
                .unwrap_or(1.0)
                .clamp(0.0, 1.0);
            stops.push((offset, color, opacity));
        }
        // A gradient may inherit its stops through xlink:href.
        if stops.is_empty() {
            continue;
        }
        stops.sort_by(|a, b| a.0.total_cmp(&b.0));
        out.push((name, stops));
    }
    Ok(out)
}

/// `gradient.presets.importSvg`: every gradient of an SVG file as a preset in `group`.
fn import_svg_gradient(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "gradient.presets.importSvg";
    let (bytes, stem) = crate::preset_import_cmds::file_bytes(p, cmd, ".svg file")?;
    let group = str_opt(p, "group").map(str::to_string).unwrap_or_else(|| if stem.is_empty() { "Imported Gradients".to_string() } else { stem.clone() });
    let text = String::from_utf8_lossy(&bytes);
    let found = svg_gradients(&text).map_err(|e| bad(cmd, e.to_string()))?;
    if found.is_empty() {
        return Err(bad(cmd, "the SVG holds no gradients"));
    }
    let mut imported = Vec::new();
    let mut items = Vec::new();
    for (name, stops) in found {
        let color_stops: Vec<(f32, StopColor)> = stops.iter().map(|(t, c, _)| (*t, StopColor::Rgb(*c))).collect();
        let mut opacity: Vec<(f32, f32)> = stops.iter().map(|(t, _, o)| (*t, *o)).collect();
        if opacity.iter().all(|(_, o)| *o >= 1.0 - 1e-4) {
            opacity.clear();
        }
        let preset = GradientPreset { name: name.clone(), stops: color_stops, opacity, segments: Vec::new() };
        imported.push(name);
        items.push(preset);
    }
    let st = &mut s.presets;
    if flag(p, "replace", false) {
        st.gradients.retain(|g| g.name != group);
    }
    match st.gradients.iter_mut().find(|g| g.name == group) {
        Some(grp) => {
            for it in items {
                grp.items.retain(|x| x.name != it.name);
                grp.items.push(it);
            }
        }
        None => st.gradients.push(crate::presets::Group::new(&group, items)),
    }
    s.presets_changed();
    Ok(json!({ "group": group, "imported": imported, "count": imported.len() }))
}

/// One gradient preset as an SVG document with a `<linearGradient>` (live foreground and
/// background stops resolved to the given colours).
pub fn gradient_svg(g: &GradientPreset, fg: [f32; 4], bg: [f32; 4]) -> String {
    let stops = g.resolve(fg, bg);
    let mut svg = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" xmlns:xlink=\"http://www.w3.org/1999/xlink\" width=\"256\" height=\"32\">\n <defs>\n  <linearGradient id=\"{}\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"0\">\n",
        xml_escape(&g.name)
    );
    for (t, c) in &stops {
        let hex = format!(
            "#{:02x}{:02x}{:02x}",
            (c[0].clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
            (c[1].clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
            (c[2].clamp(0.0, 1.0) * 255.0 + 0.5) as u8
        );
        svg.push_str(&format!("   <stop offset=\"{}%\" stop-color=\"{hex}\" stop-opacity=\"{}\"/>\n", (t.clamp(0.0, 1.0) * 100.0), c[3].clamp(0.0, 1.0)));
    }
    svg.push_str(&format!(" </linearGradient>\n </defs>\n <rect width=\"256\" height=\"32\" fill=\"url(#{})\"/>\n</svg>\n", xml_escape(&g.name)));
    svg
}

/// `gradient.presets.exportSvg`: a gradient preset (`name`, else the current gradient) as SVG.
fn export_svg_gradient(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "gradient.presets.exportSvg";
    let g = match str_opt(p, "name") {
        Some(n) => s
            .presets
            .gradients
            .iter()
            .flat_map(|grp| grp.items.iter())
            .find(|g| g.name == n)
            .cloned()
            .ok_or_else(|| bad(cmd, format!("no gradient named `{n}`")))?,
        None => s.presets.gradient.clone(),
    };
    let svg = gradient_svg(&g, s.tools.foreground, s.tools.background);
    let mut out = json!({ "name": g.name, "stops": g.stops.len(), "bytes": svg.len() });
    match str_opt(p, "path") {
        Some(path) => {
            crate::file_cmds::write_file(path, svg.as_bytes())?;
            out["path"] = json!(path);
        }
        None => out["svg"] = json!(svg),
    }
    Ok(out)
}

/// Decode the base64 `data` of a resource command (for callers outside this module).
pub fn decode_data(p: &Value, cmd: &str) -> Result<Vec<u8>> {
    let s = p.get("data").and_then(Value::as_str).ok_or_else(|| bad(cmd, "missing `data`"))?;
    b64_decode(s).ok_or_else(|| bad(cmd, "`data` is not valid base64"))
}

macro_rules! spec {
    ($id:literal, $label:literal, $params:literal, $run:expr, $journal:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[], shortcut: None, params: $params, enabled: always, run: $run, journal: $journal }
    };
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "brush.presets.exportKpp",
            "Export Brush Preset File",
            r##"{"name":string?=current preset,"path":".kpp file"? (else base64 `data`)} → {name, bytes, requiredBrushFile, requiredPatternFile, path|data}"##,
            export_kpp,
            false
        ),
        spec!(
            "brush.presets.importKpp",
            "Import Brush Preset File",
            r##"{"path":".kpp file"?,"data":base64?,"fileName":string?,"group":string?,"setCurrent":bool=false} → {name, group, warnings}"##,
            import_kpp,
            true
        ),
        spec!("resources.storages.list", "Resource Storages", "{} → {storages:[{id, kind, name, location, active, groups, presets}]}", storages_list, false),
        spec!("resources.storages.activate", "Activate Resource Storage", r##"{"id":string,"active":bool=true}"##, storages_activate, true),
        spec!("resources.storages.remove", "Remove Resource Storage", r##"{"id":string}"##, storages_remove, true),
        spec!(
            "resources.storages.add",
            "Register Resource Storage",
            r##"{"kind":"bundle|adobeLibrary|folder|memory"="adobeLibrary","name":string?,"location":string?,"groups":[preset group names]}"##,
            storages_add,
            true
        ),
        spec!(
            "resources.bundle.import",
            "Import Resource Bundle",
            r##"{"path":".bundle file"?,"data":base64?,"name":string?=the bundle's title} → {storage, name, group, imported:{presets, brushes, gradients, palettes, patterns, skipped}, warnings}"##,
            bundle_import,
            true
        ),
        spec!(
            "resources.bundle.create",
            "Create Resource Bundle",
            r##"{"name":string,"presets":[names]?,"tags":[tags]? (every preset carrying one),"palettes":[swatch groups]?,"gradients":[gradient groups]?,"author":string?,"description":string?,"license":string?,"website":string?,"email":string?,"path":".bundle file"? (else base64 `data`)}"##,
            bundle_create,
            false
        ),
        spec!("resources.bundle.list", "Resource Bundles", "{} → {bundles:[…]}", bundle_list, false),
        spec!(
            "gradient.presets.importSvg",
            "Import SVG Gradients",
            r##"{"path":".svg file"?,"data":base64?,"group":string?=file name,"replace":bool=false} → {group, imported, count}"##,
            import_svg_gradient,
            true
        ),
        spec!(
            "gradient.presets.exportSvg",
            "Export Gradient as SVG",
            r##"{"name":string?=current gradient,"path":".svg file"? (else `svg` text)}"##,
            export_svg_gradient,
            false
        ),
    ]
}
