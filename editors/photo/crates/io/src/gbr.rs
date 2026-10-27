//! Bitmap brushes (`.gbr`) and animated image hoses (`.gih`) → brush presets.
//!
//! Both are read from the public format descriptions only. A `.gbr` is one bitmap tip with a
//! small big-endian header (header size, version 1–3, width, height, bytes per pixel, the four-byte format
//! magic and spacing from version 2 on, then the NUL-terminated name) followed by the pixels: one
//! byte per pixel for a grayscale mask (255 paints), four for a colour brush (RGBA). A `.gih` is
//! two text lines (the name; the cell count and `key:value` parameters: `ncells`, `step`, `dim`,
//! `cols`, `rows`, `cellwidth`, `cellheight`, `placement`, `rank0..3`, `sel0..3`) followed by that
//! many `.gbr` brushes back to back; the selection modes (incremental, angular, random, velocity,
//! pressure, xtilt, ytilt) say which cell each dab paints with, per dimension.
//!
//! A colour `.gbr` keeps its colours: the tip is its alpha, the colours ride along as the brush's
//! colour bitmap and the preset paints in the Color Stamp tip mode. Colour hose cells become paint
//! masks (alpha times darkness, as Define Brush Preset reads a selection) and the import notes it.

use redrob_photo_paint::{BrushPreset, BrushSettings, ColorTile, GrayTile, HoseDimension, HoseSelection, ImageHose, TipMode, TipShape};

use crate::abr_map::{MAX_TIP_EDGE, fit};

/// Largest accepted brush side (a hose cell or a single tip) before parsing fails.
pub const MAX_SIDE: u32 = 16384;
/// Largest sample count of one brush accepted (64 Mi samples).
pub const MAX_SAMPLES: u64 = 1 << 26;
/// Most cells read from one hose.
pub const MAX_CELLS: usize = 4096;
/// Most dimensions a hose declares.
pub const MAX_DIMS: usize = 4;

/// The presets read from one file plus everything that could not be mapped.
#[derive(Clone, Debug, Default)]
pub struct BrushImport {
    pub presets: Vec<BrushPreset>,
    pub warnings: Vec<String>,
}

/// One parsed `.gbr` brush.
#[derive(Clone, Debug, PartialEq)]
pub struct GbrBrush {
    pub name: String,
    pub version: u32,
    pub width: u32,
    pub height: u32,
    /// Spacing as a fraction of the diameter; `None` for a version-1 file (no spacing field).
    pub spacing: Option<f32>,
    /// Four bytes per pixel in the file (a colour brush).
    pub color: bool,
    /// The paint mask, 1 = full (a colour brush: alpha times darkness, the mask a hose cell uses).
    pub tile: GrayTile,
    /// A colour brush's own alpha (the tip the Color Stamp mode paints with) and colours.
    pub stamp: Option<(GrayTile, ColorTile)>,
}

fn be_u32(b: &[u8], at: usize) -> Result<u32, String> {
    let s = b.get(at..at + 4).ok_or_else(|| "truncated brush header".to_string())?;
    Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

/// Is there a plausible `.gbr` header at the start of `b`?
pub fn looks_like_gbr(b: &[u8]) -> bool {
    let Ok(header) = be_u32(b, 0) else { return false };
    let Ok(version) = be_u32(b, 4) else { return false };
    (1..=3).contains(&version) && header >= 20 && header as usize <= b.len() && (version == 1 || b.get(20..24) == Some(b"GIMP".as_slice()))
}

/// Parse one `.gbr` brush at the start of `b`; returns it and the bytes it took.
pub fn parse_gbr(b: &[u8]) -> Result<(GbrBrush, usize), String> {
    let header = be_u32(b, 0)? as usize;
    let version = be_u32(b, 4)?;
    let width = be_u32(b, 8)?;
    let height = be_u32(b, 12)?;
    let bytes = be_u32(b, 16)?;
    if !(1..=3).contains(&version) {
        return Err(format!("unsupported brush version {version}"));
    }
    let fixed = if version == 1 { 20 } else { 28 };
    if header < fixed || header > b.len() {
        return Err(format!("bad brush header size {header}"));
    }
    let mut spacing = None;
    if version >= 2 {
        if b.get(20..24) != Some(b"GIMP".as_slice()) {
            return Err("missing brush magic".into());
        }
        spacing = Some((be_u32(b, 24)? as f32 / 100.0).clamp(0.01, 10.0));
    }
    let color = match bytes {
        1 => false,
        4 => true,
        _ => return Err(format!("unsupported brush depth: {bytes} bytes per pixel")),
    };
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE || u64::from(width) * u64::from(height) > MAX_SAMPLES {
        return Err(format!("bad brush size {width}×{height}"));
    }
    let name_bytes = &b[fixed..header];
    let name_end = name_bytes.iter().position(|&c| c == 0).unwrap_or(name_bytes.len());
    let name = String::from_utf8_lossy(&name_bytes[..name_end]).trim().to_string();
    let n = width as usize * height as usize;
    let data_len = n * bytes as usize;
    let end = header.checked_add(data_len).ok_or_else(|| "brush data overflows".to_string())?;
    let px = b.get(header..end).ok_or_else(|| format!("brush \"{name}\" is truncated ({} of {data_len} data bytes)", b.len().saturating_sub(header)))?;
    let v: Vec<f32> = if color {
        px.as_chunks::<4>()
            .0
            .iter()
            .map(|c| {
                let lum = (0.299 * f32::from(c[0]) + 0.587 * f32::from(c[1]) + 0.114 * f32::from(c[2])) / 255.0;
                ((1.0 - lum) * f32::from(c[3]) / 255.0).clamp(0.0, 1.0)
            })
            .collect()
    } else {
        px.iter().map(|&g| f32::from(g) / 255.0).collect()
    };
    let (w, h, v) = fit(width, height, v, MAX_TIP_EDGE);
    let tile = GrayTile::from_f32(w, h, &v);
    // A colour brush also keeps its colours: alpha as the tip, RGB as the colour bitmap.
    let stamp = color.then(|| {
        let chunks = px.as_chunks::<4>().0;
        let plane = |i: usize| fit(width, height, chunks.iter().map(|c| f32::from(c[i]) / 255.0).collect(), MAX_TIP_EDGE);
        let (aw, ah, a) = plane(3);
        let (_, _, r) = plane(0);
        let (_, _, g) = plane(1);
        let (_, _, b2) = plane(2);
        let rgb: Vec<u8> = (0..r.len()).flat_map(|i| [(r[i] * 255.0 + 0.5) as u8, (g[i] * 255.0 + 0.5) as u8, (b2[i] * 255.0 + 0.5) as u8]).collect();
        (GrayTile::from_f32(aw, ah, &a), ColorTile::from_rgb8(aw, ah, &rgb))
    });
    Ok((GbrBrush { name, version, width, height, spacing, color, tile, stamp }, end))
}

fn preset(name: String, brush: BrushSettings, group: &str) -> BrushPreset {
    BrushPreset { name, brush, builtin: false, group: group.to_string(), folder: Vec::new(), captures: Default::default() }
}

fn settings(tile: GrayTile, size: f32, spacing: Option<f32>) -> BrushSettings {
    BrushSettings {
        pressure_size: false,
        spacing: spacing.unwrap_or(0.25),
        size: size.clamp(1.0, redrob_photo_paint::MAX_BRUSH_SIZE),
        tip: TipShape::Sampled(tile),
        ..Default::default()
    }
}

/// A `.gbr` file as one preset in group `group`; `fallback_name` names a brush whose header has no name.
pub fn read_gbr(bytes: &[u8], group: &str, fallback_name: &str) -> Result<BrushImport, String> {
    let (b, used) = parse_gbr(bytes).map_err(|e| format!("not a readable brush file (.gbr): {e}"))?;
    let mut out = BrushImport::default();
    if used < bytes.len() {
        out.warnings.push(format!("{} trailing bytes after the brush were ignored", bytes.len() - used));
    }
    if (b.width, b.height) != (b.tile.width, b.tile.height) {
        out.warnings.push(format!("tips larger than {MAX_TIP_EDGE} px were downsampled"));
    }
    let name = if b.name.is_empty() { fallback_name.trim().to_string() } else { b.name.clone() };
    let name = if name.is_empty() { "Imported Brush".to_string() } else { name };
    let size = b.width.max(b.height) as f32;
    let mut brush = match b.stamp {
        // A colour brush: its alpha is the tip, its colours paint as an image stamp.
        Some((alpha, colors)) => {
            let mut brush = settings(alpha, size, b.spacing);
            brush.tip_color = Some(colors);
            brush.tip_mode = TipMode::ColorStamp;
            brush
        }
        None => settings(b.tile, size, b.spacing),
    };
    brush.tip_scale = 1.0;
    out.presets.push(preset(name, brush, group));
    Ok(out)
}

/// The second header line of a hose, parsed: the cell count and its `key:value` parameters.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GihHeader {
    pub name: String,
    pub ncells: usize,
    pub step: Option<f32>,
    pub dims: Vec<HoseDimension>,
    /// Parameters read but not used (reported).
    pub other: Vec<String>,
}

fn selection(s: &str) -> Option<HoseSelection> {
    Some(match s.trim().to_ascii_lowercase().as_str() {
        "incremental" | "constant" => HoseSelection::Incremental,
        "angular" => HoseSelection::Angular,
        "random" => HoseSelection::Random,
        "velocity" => HoseSelection::Velocity,
        "pressure" => HoseSelection::Pressure,
        "xtilt" => HoseSelection::TiltX,
        "ytilt" => HoseSelection::TiltY,
        _ => return None,
    })
}

/// Parse the two text lines of a hose header; returns it and where the first cell starts.
pub fn parse_gih_header(b: &[u8]) -> Result<(GihHeader, usize), String> {
    // Both lines end in '\n'; a header longer than this is not a hose.
    const MAX_HEADER: usize = 4096;
    let head = &b[..b.len().min(MAX_HEADER)];
    let nl1 = head.iter().position(|&c| c == b'\n').ok_or_else(|| "no name line".to_string())?;
    let nl2 = head[nl1 + 1..].iter().position(|&c| c == b'\n').map(|i| nl1 + 1 + i).ok_or_else(|| "no parameter line".to_string())?;
    let name = String::from_utf8_lossy(&head[..nl1]).trim().to_string();
    let params = String::from_utf8_lossy(&head[nl1 + 1..nl2]).to_string();
    let mut words = params.split_whitespace();
    let ncells: usize = words.next().and_then(|w| w.parse().ok()).filter(|n| (1..=MAX_CELLS).contains(n)).ok_or_else(|| "bad cell count".to_string())?;
    let mut h = GihHeader { name, ncells, ..Default::default() };
    let mut ranks = [None::<u32>; MAX_DIMS];
    let mut sels = [None::<HoseSelection>; MAX_DIMS];
    let mut dim = 1usize;
    let mut unknown_sel = Vec::new();
    for w in words {
        let Some((k, v)) = w.split_once(':') else {
            h.other.push(w.to_string());
            continue;
        };
        match k {
            "ncells" => {}
            "step" => h.step = v.parse::<f32>().ok().filter(|s| s.is_finite() && *s > 0.0).map(|s| (s / 100.0).clamp(0.01, 10.0)),
            "dim" => dim = v.parse::<usize>().ok().filter(|d| (1..=MAX_DIMS).contains(d)).ok_or_else(|| format!("bad dimension count `{v}`"))?,
            _ if k.starts_with("rank") => {
                if let (Ok(i), Ok(r)) = (k[4..].parse::<usize>(), v.parse::<u32>())
                    && let Some(slot) = ranks.get_mut(i)
                {
                    *slot = Some(r.max(1));
                }
            }
            _ if k.starts_with("sel") => {
                if let Ok(i) = k[3..].parse::<usize>()
                    && let Some(slot) = sels.get_mut(i)
                {
                    match selection(v) {
                        Some(s) => *slot = Some(s),
                        None => unknown_sel.push(v.to_string()),
                    }
                }
            }
            _ => h.other.push(w.to_string()),
        }
    }
    for (rank, sel) in ranks.iter().zip(&sels).take(dim) {
        h.dims.push(HoseDimension { rank: rank.unwrap_or(if dim == 1 { ncells as u32 } else { 1 }), selection: sel.unwrap_or_default() });
    }
    if !unknown_sel.is_empty() {
        h.other.push(format!("unknown selection mode(s) {} read as incremental", unknown_sel.join(", ")));
    }
    Ok((h, nl2 + 1))
}

/// A `.gih` file as one animated preset in group `group`.
pub fn read_gih(bytes: &[u8], group: &str, fallback_name: &str) -> Result<BrushImport, String> {
    let (h, mut at) = parse_gih_header(bytes).map_err(|e| format!("not a readable animated brush (.gih): {e}"))?;
    let mut out = BrushImport::default();
    let mut cells = Vec::with_capacity(h.ncells);
    let (mut color, mut downsampled, mut spacing) = (false, false, h.step);
    let mut size = 1.0f32;
    for i in 0..h.ncells {
        if at >= bytes.len() {
            out.warnings.push(format!("the hose declares {} cells but holds {i}", h.ncells));
            break;
        }
        let (b, used) = parse_gbr(&bytes[at..]).map_err(|e| format!("animated brush cell {}: {e}", i + 1))?;
        at += used;
        color |= b.color;
        downsampled |= (b.width, b.height) != (b.tile.width, b.tile.height);
        size = size.max(b.width.max(b.height) as f32);
        if spacing.is_none() {
            spacing = b.spacing;
        }
        cells.push(b.tile);
    }
    if cells.is_empty() {
        return Err("the animated brush holds no cells".into());
    }
    if at < bytes.len() {
        out.warnings.push(format!("{} trailing bytes after the last cell were ignored", bytes.len() - at));
    }
    if color {
        out.warnings.push("colour cells paint with the brush colour; their colours became the paint mask".into());
    }
    if downsampled {
        out.warnings.push(format!("cells larger than {MAX_TIP_EDGE} px were downsampled"));
    }
    if !h.other.is_empty() {
        out.warnings.push(format!("hose parameters without an equivalent were ignored: {}", h.other.join(", ")));
    }
    let name = if h.name.is_empty() { fallback_name.trim().to_string() } else { h.name.clone() };
    let name = if name.is_empty() { "Animated Brush".to_string() } else { name };
    let first = cells[0].clone();
    let mut brush = settings(first, size, spacing);
    brush.hose = Some(ImageHose { cells, dims: h.dims });
    out.presets.push(preset(name, brush, group));
    Ok(out)
}

/// Read a `.gbr` or `.gih` by content (a hose starts with text, a brush with its binary header).
pub fn read_brush_file(bytes: &[u8], group: &str, fallback_name: &str) -> Result<BrushImport, String> {
    if looks_like_gbr(bytes) { read_gbr(bytes, group, fallback_name) } else { read_gih(bytes, group, fallback_name) }
}
