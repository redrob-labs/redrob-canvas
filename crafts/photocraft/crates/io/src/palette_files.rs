//! Palette files beyond `.aco`/`.ase`: `.gpl` text palettes, Microsoft RIFF palettes (`.pal`),
//! Adobe Color Tables (`.act`), JASC/Paint Shop Pro palettes (`.pal`/`.psp`), Scribus colour sets
//! (`.xml`), SwatchBooker books (`.sbz`), Adobe Color Books (`.acb`) and the native zip palette
//! (`.kpl`: `colorset.xml` plus a `mimetype` entry). Every reader works from the public format
//! descriptions: a text header and `R G B name` lines; the RIFF `PAL ` chunk with its `data`
//! LOGPALETTE; 768 bytes of RGB with an optional four-byte trailer; `JASC-PAL`, `0100`, a count and
//! `R G B` lines; `<SCRIBUSCOLORS>` with `<COLOR>` children; a zip holding `swatchbook.xml` with
//! `<color>` materials; `8BCB` with UTF-16 strings and 3- or 4-byte colour codes; and the
//! `<ColorSet>` document with `<ColorSetEntry>` and `<Group>` children.
//!
//! Colours come back in their own model so the caller can keep them that way (CMYK and Lab
//! swatches stay inks and Lab values, as in the Swatches panel).

use redrob_photo_format::zip::{ZipReader, ZipWriter};

/// Largest palette file read.
pub const MAX_BYTES: usize = 32 << 20;
/// Most colours read from one file.
pub const MAX_COLORS: usize = 65_536;
/// Largest entry read out of a zip palette.
const MAX_ENTRY: usize = 16 << 20;

/// A palette colour in its own model (all components 0..1 except Lab: L* 0..100, a*, b*).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PalColor {
    Rgb([f32; 3]),
    Cmyk([f32; 4]),
    Lab([f32; 3]),
    /// Black ink 0..1 (0 = white).
    Gray(f32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PalEntry {
    pub name: String,
    pub color: PalColor,
    /// A spot colour (where the format says so).
    pub spot: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PalGroup {
    /// Empty = the palette's loose colours.
    pub name: String,
    pub entries: Vec<PalEntry>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Palette {
    pub name: String,
    pub columns: u32,
    pub groups: Vec<PalGroup>,
    pub warnings: Vec<String>,
}

impl Palette {
    pub fn count(&self) -> usize {
        self.groups.iter().map(|g| g.entries.len()).sum()
    }
    fn loose(&mut self) -> &mut PalGroup {
        if self.groups.first().is_none_or(|g| !g.name.is_empty()) {
            self.groups.insert(0, PalGroup::default());
        }
        &mut self.groups[0]
    }
}

/// The palette file kinds this module reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteKind {
    Gpl,
    RiffPal,
    Act,
    JascPal,
    ScribusXml,
    Sbz,
    Acb,
    Kpl,
}

impl PaletteKind {
    pub const ALL: [PaletteKind; 8] = [
        PaletteKind::Gpl,
        PaletteKind::RiffPal,
        PaletteKind::Act,
        PaletteKind::JascPal,
        PaletteKind::ScribusXml,
        PaletteKind::Sbz,
        PaletteKind::Acb,
        PaletteKind::Kpl,
    ];

    pub fn id(self) -> &'static str {
        match self {
            PaletteKind::Gpl => "gpl",
            PaletteKind::RiffPal => "riff",
            PaletteKind::Act => "act",
            PaletteKind::JascPal => "psp",
            PaletteKind::ScribusXml => "xml",
            PaletteKind::Sbz => "sbz",
            PaletteKind::Acb => "acb",
            PaletteKind::Kpl => "kpl",
        }
    }

    /// The kind named by a format id or file extension.
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim_start_matches('.').to_ascii_lowercase().as_str() {
            "gpl" => PaletteKind::Gpl,
            "riff" | "pal" => PaletteKind::RiffPal,
            "act" => PaletteKind::Act,
            "psp" | "jasc" | "psppalette" => PaletteKind::JascPal,
            "xml" | "scribus" => PaletteKind::ScribusXml,
            "sbz" => PaletteKind::Sbz,
            "acb" => PaletteKind::Acb,
            "kpl" => PaletteKind::Kpl,
            _ => return None,
        })
    }
}

/// Tell a palette file's kind from its bytes (and the extension, for the two that have no
/// signature: `.act` and a zip without a telling entry).
pub fn detect(bytes: &[u8], ext: Option<&str>) -> Option<PaletteKind> {
    let head = &bytes[..bytes.len().min(64)];
    if head.starts_with(b"GIMP Palette") {
        return Some(PaletteKind::Gpl);
    }
    if head.starts_with(b"JASC-PAL") {
        return Some(PaletteKind::JascPal);
    }
    if head.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"PAL ".as_slice()) {
        return Some(PaletteKind::RiffPal);
    }
    if head.starts_with(b"8BCB") {
        return Some(PaletteKind::Acb);
    }
    if head.starts_with(b"PK\x03\x04") {
        if let Ok(z) = ZipReader::new(bytes) {
            if z.find("colorset.xml").is_some() {
                return Some(PaletteKind::Kpl);
            }
            if z.find("swatchbook.xml").is_some() {
                return Some(PaletteKind::Sbz);
            }
        }
        return ext.and_then(PaletteKind::parse).filter(|k| matches!(k, PaletteKind::Sbz | PaletteKind::Kpl));
    }
    let text = String::from_utf8_lossy(head);
    if text.trim_start().starts_with('<') && String::from_utf8_lossy(&bytes[..bytes.len().min(4096)]).contains("SCRIBUSCOLORS") {
        return Some(PaletteKind::ScribusXml);
    }
    match ext.and_then(PaletteKind::parse) {
        Some(PaletteKind::Act) if bytes.len() == 768 || bytes.len() == 772 => Some(PaletteKind::Act),
        Some(k @ (PaletteKind::Gpl | PaletteKind::JascPal | PaletteKind::ScribusXml)) => Some(k),
        _ => None,
    }
}

/// Read a palette file of `kind`.
pub fn read(bytes: &[u8], kind: PaletteKind, fallback_name: &str) -> Result<Palette, String> {
    if bytes.len() > MAX_BYTES {
        return Err(format!("palette file is larger than {} MiB", MAX_BYTES >> 20));
    }
    let mut pal = match kind {
        PaletteKind::Gpl => read_gpl(bytes)?,
        PaletteKind::RiffPal => read_riff(bytes)?,
        PaletteKind::Act => read_act(bytes)?,
        PaletteKind::JascPal => read_jasc(bytes)?,
        PaletteKind::ScribusXml => read_scribus(bytes)?,
        PaletteKind::Sbz => read_sbz(bytes)?,
        PaletteKind::Acb => read_acb(bytes)?,
        PaletteKind::Kpl => read_kpl(bytes)?,
    };
    if pal.name.trim().is_empty() {
        pal.name = fallback_name.trim().to_string();
    }
    pal.groups.retain(|g| !g.entries.is_empty());
    if pal.count() == 0 {
        return Err("the file holds no colours".into());
    }
    if pal.count() > MAX_COLORS {
        return Err(format!("the file holds more than {MAX_COLORS} colours"));
    }
    Ok(pal)
}

fn byte_color(r: u8, g: u8, b: u8) -> PalColor {
    PalColor::Rgb([f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0])
}

fn entry(name: impl Into<String>, color: PalColor) -> PalEntry {
    PalEntry { name: name.into(), color, spot: false }
}

// ---------------------------------------------------------------------------
// Text palette (.gpl)
// ---------------------------------------------------------------------------

/// The `.gpl` header line (its fixed magic), optional `Name:` and `Columns:` lines, `#` comments,
/// then `R G B [name]`.
pub fn read_gpl(bytes: &[u8]) -> Result<Palette, String> {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = text.lines();
    if !lines.next().is_some_and(|l| l.trim_start().starts_with("GIMP Palette")) {
        return Err("not a .gpl palette (missing header)".into());
    }
    let mut pal = Palette::default();
    for line in lines {
        let l = line.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        if let Some(n) = l.strip_prefix("Name:") {
            pal.name = n.trim().to_string();
            continue;
        }
        if let Some(c) = l.strip_prefix("Columns:") {
            pal.columns = c.trim().parse().unwrap_or(0);
            continue;
        }
        let mut it = l.split_whitespace();
        let (Some(r), Some(g), Some(b)) = (it.next(), it.next(), it.next()) else {
            pal.warnings.push(format!("ignored line: {l}"));
            continue;
        };
        let (Ok(r), Ok(g), Ok(b)) = (r.parse::<u8>(), g.parse::<u8>(), b.parse::<u8>()) else {
            pal.warnings.push(format!("ignored line: {l}"));
            continue;
        };
        let name = it.collect::<Vec<_>>().join(" ");
        let i = pal.count();
        pal.loose().entries.push(entry(if name.is_empty() { format!("Colour {}", i + 1) } else { name }, byte_color(r, g, b)));
        if pal.count() > MAX_COLORS {
            return Err(format!("more than {MAX_COLORS} colours"));
        }
    }
    Ok(pal)
}

/// Write a `.gpl` palette.
pub fn write_gpl(pal: &Palette) -> Vec<u8> {
    let mut out = String::from("GIMP Palette\n");
    out.push_str(&format!("Name: {}\n", pal.name));
    if pal.columns > 0 {
        out.push_str(&format!("Columns: {}\n", pal.columns));
    }
    out.push_str("#\n");
    for g in &pal.groups {
        if !g.name.is_empty() {
            out.push_str(&format!("# {}\n", g.name));
        }
        for e in &g.entries {
            let [r, gg, b] = to_rgb8(e.color);
            out.push_str(&format!("{r:3} {gg:3} {b:3}\t{}\n", e.name));
        }
    }
    out.into_bytes()
}

/// A colour as 8-bit sRGB-ish values (CMYK and Lab by the naive conversions, for formats that
/// only hold RGB).
pub fn to_rgb8(c: PalColor) -> [u8; 3] {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    match c {
        PalColor::Rgb([r, g, b]) => [q(r), q(g), q(b)],
        PalColor::Cmyk([cc, m, y, k]) => [q((1.0 - cc) * (1.0 - k)), q((1.0 - m) * (1.0 - k)), q((1.0 - y) * (1.0 - k))],
        PalColor::Gray(k) => [q(1.0 - k), q(1.0 - k), q(1.0 - k)],
        PalColor::Lab([l, a, b]) => {
            let [r, g, bb] = lab_to_rgb(l, a, b);
            [q(r), q(g), q(bb)]
        }
    }
}

/// CIE L*a*b* (D50) to sRGB, the textbook conversion (gamut clipped).
pub fn lab_to_rgb(l: f32, a: f32, b: f32) -> [f32; 3] {
    let fy = (l + 16.0) / 116.0;
    let fx = fy + a / 500.0;
    let fz = fy - b / 200.0;
    let finv = |t: f32| if t > 6.0 / 29.0 { t * t * t } else { 3.0 * (6.0f32 / 29.0).powi(2) * (t - 4.0 / 29.0) };
    let (xn, yn, zn) = (0.9642, 1.0, 0.8251);
    let (x, y, z) = (xn * finv(fx), yn * finv(fy), zn * finv(fz));
    // D50 XYZ to linear sRGB (Bradford-adapted matrix).
    let r = 3.1339 * x - 1.6169 * y - 0.4906 * z;
    let g = -0.9785 * x + 1.9160 * y + 0.0333 * z;
    let bl = 0.0720 * x - 0.2290 * y + 1.4057 * z;
    let gamma = |v: f32| {
        let v = v.clamp(0.0, 1.0);
        if v <= 0.003_130_8 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
    };
    [gamma(r), gamma(g), gamma(bl)]
}

// ---------------------------------------------------------------------------
// Microsoft RIFF palette (.pal)
// ---------------------------------------------------------------------------

fn le_u16(b: &[u8], at: usize) -> Result<u16, String> {
    b.get(at..at + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| "truncated palette".to_string())
}

fn le_u32(b: &[u8], at: usize) -> Result<u32, String> {
    b.get(at..at + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| "truncated palette".to_string())
}

/// `RIFF` size `PAL ` then chunks; the `data` chunk is a LOGPALETTE: version, count, `count`
/// PALETTEENTRY (r, g, b, flags).
pub fn read_riff(bytes: &[u8]) -> Result<Palette, String> {
    if !bytes.starts_with(b"RIFF") || bytes.get(8..12) != Some(b"PAL ".as_slice()) {
        return Err("not a RIFF palette".into());
    }
    let mut pal = Palette::default();
    let mut at = 12usize;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = le_u32(bytes, at + 4)? as usize;
        let body = bytes.get(at + 8..at + 8 + size).ok_or_else(|| "truncated RIFF chunk".to_string())?;
        if id == b"data" {
            let count = le_u16(body, 2)? as usize;
            if count > MAX_COLORS {
                return Err(format!("more than {MAX_COLORS} colours"));
            }
            for i in 0..count {
                let Some(e) = body.get(4 + i * 4..8 + i * 4) else {
                    pal.warnings.push(format!("the palette declares {count} colours but holds {i}"));
                    break;
                };
                pal.loose().entries.push(entry(format!("Colour {}", i + 1), byte_color(e[0], e[1], e[2])));
            }
        }
        at += 8 + size + (size & 1);
    }
    Ok(pal)
}

/// Write a RIFF palette (RGB only).
pub fn write_riff(pal: &Palette) -> Vec<u8> {
    let colors: Vec<[u8; 3]> = pal.groups.iter().flat_map(|g| g.entries.iter().map(|e| to_rgb8(e.color))).take(u16::MAX as usize).collect();
    let mut data = Vec::with_capacity(4 + colors.len() * 4);
    data.extend_from_slice(&0x0300u16.to_le_bytes());
    data.extend_from_slice(&(colors.len() as u16).to_le_bytes());
    for c in &colors {
        data.extend_from_slice(&[c[0], c[1], c[2], 0]);
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((4 + 8 + data.len()) as u32).to_le_bytes());
    out.extend_from_slice(b"PAL ");
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
    out
}

// ---------------------------------------------------------------------------
// Adobe Color Table (.act)
// ---------------------------------------------------------------------------

/// 256 RGB triples; an optional trailer gives the colour count and a transparent index.
pub fn read_act(bytes: &[u8]) -> Result<Palette, String> {
    if bytes.len() != 768 && bytes.len() != 772 {
        return Err(format!("an .act colour table is 768 or 772 bytes, not {}", bytes.len()));
    }
    let count = if bytes.len() == 772 { usize::from(u16::from_be_bytes([bytes[768], bytes[769]])).min(256) } else { 256 };
    let mut pal = Palette::default();
    for i in 0..count {
        let c = &bytes[i * 3..i * 3 + 3];
        pal.loose().entries.push(entry(format!("Colour {}", i + 1), byte_color(c[0], c[1], c[2])));
    }
    if bytes.len() == 772 {
        let t = u16::from_be_bytes([bytes[770], bytes[771]]);
        if t != 0xFFFF {
            pal.warnings.push(format!("index {t} is the transparent colour"));
        }
    }
    Ok(pal)
}

/// Write a colour table: up to 256 colours, padded with black, with the count trailer.
pub fn write_act(pal: &Palette) -> Vec<u8> {
    let colors: Vec<[u8; 3]> = pal.groups.iter().flat_map(|g| g.entries.iter().map(|e| to_rgb8(e.color))).take(256).collect();
    let mut out = Vec::with_capacity(772);
    for i in 0..256 {
        out.extend_from_slice(&colors.get(i).copied().unwrap_or([0, 0, 0]));
    }
    out.extend_from_slice(&(colors.len() as u16).to_be_bytes());
    out.extend_from_slice(&0xFFFFu16.to_be_bytes());
    out
}

// ---------------------------------------------------------------------------
// JASC / Paint Shop Pro palette
// ---------------------------------------------------------------------------

/// `JASC-PAL`, `0100`, the count, then `R G B` lines.
pub fn read_jasc(bytes: &[u8]) -> Result<Palette, String> {
    let text = String::from_utf8_lossy(bytes);
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    if lines.next() != Some("JASC-PAL") {
        return Err("not a JASC palette (missing header)".into());
    }
    let _version = lines.next();
    let count: usize = lines.next().and_then(|c| c.parse().ok()).ok_or_else(|| "missing colour count".to_string())?;
    if count > MAX_COLORS {
        return Err(format!("more than {MAX_COLORS} colours"));
    }
    let mut pal = Palette::default();
    for (i, l) in lines.take(count).enumerate() {
        let v: Vec<u8> = l.split_whitespace().take(3).filter_map(|t| t.parse().ok()).collect();
        if v.len() < 3 {
            pal.warnings.push(format!("ignored line: {l}"));
            continue;
        }
        pal.loose().entries.push(entry(format!("Colour {}", i + 1), byte_color(v[0], v[1], v[2])));
    }
    Ok(pal)
}

/// Write a JASC palette.
pub fn write_jasc(pal: &Palette) -> Vec<u8> {
    let colors: Vec<[u8; 3]> = pal.groups.iter().flat_map(|g| g.entries.iter().map(|e| to_rgb8(e.color))).collect();
    let mut out = format!("JASC-PAL\r\n0100\r\n{}\r\n", colors.len());
    for c in colors {
        out.push_str(&format!("{} {} {}\r\n", c[0], c[1], c[2]));
    }
    out.into_bytes()
}

// ---------------------------------------------------------------------------
// Scribus colour set (.xml)
// ---------------------------------------------------------------------------

fn hex_pair(s: &str, i: usize) -> Option<f32> {
    s.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok()).map(|v| f32::from(v) / 255.0)
}

fn attr_f(n: roxmltree::Node<'_, '_>, k: &str) -> Option<f32> {
    n.attribute(k).and_then(|v| v.trim().parse::<f32>().ok()).filter(|v| v.is_finite())
}

/// `<SCRIBUSCOLORS Name="…"><COLOR NAME="…" RGB="#rrggbb" | CMYK="#ccmmyykk" | SPACE="RGB|CMYK|Lab"
/// R= G= B= | C= M= Y= K= | L= A= B= Spot="1"/></SCRIBUSCOLORS>`.
pub fn read_scribus(bytes: &[u8]) -> Result<Palette, String> {
    let text = String::from_utf8_lossy(bytes);
    let doc = roxmltree::Document::parse(&text).map_err(|e| format!("not a Scribus colour set: {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "SCRIBUSCOLORS" {
        return Err("not a Scribus colour set (missing SCRIBUSCOLORS)".into());
    }
    let mut pal = Palette { name: root.attribute("Name").unwrap_or("").to_string(), ..Default::default() };
    for n in root.descendants().filter(|n| n.is_element() && n.tag_name().name() == "COLOR") {
        let name = n.attribute("NAME").unwrap_or("").to_string();
        let color = if let Some(rgb) = n.attribute("RGB").map(|s| s.trim_start_matches('#')) {
            match (hex_pair(rgb, 0), hex_pair(rgb, 2), hex_pair(rgb, 4)) {
                (Some(r), Some(g), Some(b)) => PalColor::Rgb([r, g, b]),
                _ => {
                    pal.warnings.push(format!("bad RGB value for {name}"));
                    continue;
                }
            }
        } else if let Some(cmyk) = n.attribute("CMYK").map(|s| s.trim_start_matches('#')) {
            match (hex_pair(cmyk, 0), hex_pair(cmyk, 2), hex_pair(cmyk, 4), hex_pair(cmyk, 6)) {
                (Some(c), Some(m), Some(y), Some(k)) => PalColor::Cmyk([c, m, y, k]),
                _ => {
                    pal.warnings.push(format!("bad CMYK value for {name}"));
                    continue;
                }
            }
        } else {
            match n.attribute("SPACE").map(str::to_ascii_uppercase).as_deref() {
                Some("RGB") => match (attr_f(n, "R"), attr_f(n, "G"), attr_f(n, "B")) {
                    (Some(r), Some(g), Some(b)) => PalColor::Rgb([r / 255.0, g / 255.0, b / 255.0]),
                    _ => continue,
                },
                Some("CMYK") => match (attr_f(n, "C"), attr_f(n, "M"), attr_f(n, "Y"), attr_f(n, "K")) {
                    (Some(c), Some(m), Some(y), Some(k)) => PalColor::Cmyk([c / 100.0, m / 100.0, y / 100.0, k / 100.0]),
                    _ => continue,
                },
                Some("LAB") => match (attr_f(n, "L"), attr_f(n, "A"), attr_f(n, "B")) {
                    (Some(l), Some(a), Some(b)) => PalColor::Lab([l, a, b]),
                    _ => continue,
                },
                _ => {
                    pal.warnings.push(format!("colour {name} has no value"));
                    continue;
                }
            }
        };
        let spot = n.attribute("Spot").is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
        let i = pal.count();
        pal.loose().entries.push(PalEntry { name: if name.is_empty() { format!("Colour {}", i + 1) } else { name }, color, spot });
        if pal.count() > MAX_COLORS {
            return Err(format!("more than {MAX_COLORS} colours"));
        }
    }
    Ok(pal)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Write a Scribus colour set.
pub fn write_scribus(pal: &Palette) -> Vec<u8> {
    let mut out = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<SCRIBUSCOLORS Name=\"{}\">\n", xml_escape(&pal.name));
    for g in &pal.groups {
        for e in &g.entries {
            let spot = if e.spot { " Spot=\"1\"" } else { "" };
            match e.color {
                PalColor::Cmyk([c, m, y, k]) => {
                    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
                    out.push_str(&format!("  <COLOR NAME=\"{}\" CMYK=\"#{:02x}{:02x}{:02x}{:02x}\"{spot}/>\n", xml_escape(&e.name), q(c), q(m), q(y), q(k)));
                }
                PalColor::Lab([l, a, b]) => {
                    out.push_str(&format!("  <COLOR NAME=\"{}\" SPACE=\"Lab\" L=\"{l}\" A=\"{a}\" B=\"{b}\"{spot}/>\n", xml_escape(&e.name)));
                }
                c => {
                    let [r, g2, b] = to_rgb8(c);
                    out.push_str(&format!("  <COLOR NAME=\"{}\" RGB=\"#{r:02x}{g2:02x}{b:02x}\"{spot}/>\n", xml_escape(&e.name)));
                }
            }
        }
    }
    out.push_str("</SCRIBUSCOLORS>\n");
    out.into_bytes()
}

// ---------------------------------------------------------------------------
// SwatchBooker (.sbz)
// ---------------------------------------------------------------------------

/// A zip holding `swatchbook.xml`: `<SwatchBook>` with `<materials>` of `<color>` elements
/// (`<dc:identifier>`, `<dc:title>`, `<values model="RGB|CMYK|Lab|GRAY">c c c</values>`) and
/// `<book>` of `<swatch material="id"/>` and `<group>` elements that order them.
pub fn read_sbz(bytes: &[u8]) -> Result<Palette, String> {
    let zip = ZipReader::new(bytes).map_err(|e| format!("not a SwatchBooker file: {e}"))?;
    let xml = zip.read_by_name("swatchbook.xml", MAX_ENTRY).map_err(|e| format!("not a SwatchBooker file: {e}"))?;
    let text = String::from_utf8_lossy(&xml);
    let doc = roxmltree::Document::parse(&text).map_err(|e| format!("bad swatchbook.xml: {e}"))?;
    let root = doc.root_element();
    let mut pal = Palette::default();
    let child_text = |n: roxmltree::Node<'_, '_>, tag: &str| -> Option<String> {
        n.children().find(|c| c.is_element() && c.tag_name().name() == tag).and_then(|c| c.text()).map(|t| t.trim().to_string())
    };
    if let Some(meta) = root.children().find(|c| c.is_element() && c.tag_name().name() == "metadata") {
        pal.name = child_text(meta, "title").unwrap_or_default();
    }
    // Materials by identifier.
    let mut materials: std::collections::HashMap<String, PalEntry> = std::collections::HashMap::new();
    for c in root.descendants().filter(|n| n.is_element() && n.tag_name().name() == "color") {
        let id = child_text(c, "identifier").unwrap_or_default();
        let title = child_text(c, "title").unwrap_or_else(|| id.clone());
        let spot = c.attribute("usage").is_some_and(|u| u.eq_ignore_ascii_case("spot"));
        let Some(values) = c.children().find(|n| n.is_element() && n.tag_name().name() == "values") else { continue };
        let model = values.attribute("model").unwrap_or("RGB").to_ascii_uppercase();
        let v: Vec<f32> = values.text().unwrap_or("").split_whitespace().filter_map(|t| t.parse().ok()).collect();
        let color = match (model.as_str(), v.as_slice()) {
            ("RGB", [r, g, b, ..]) => PalColor::Rgb([*r, *g, *b]),
            ("CMYK", [cc, m, y, k, ..]) => PalColor::Cmyk([*cc, *m, *y, *k]),
            ("LAB", [l, a, b, ..]) => PalColor::Lab([*l, *a, *b]),
            ("GRAY", [g, ..]) => PalColor::Gray(1.0 - g),
            _ => {
                pal.warnings.push(format!("colour {title} uses the {model} model, which is not read"));
                continue;
            }
        };
        materials.insert(id, PalEntry { name: title, color, spot });
    }
    // The book orders them (groups become groups); materials no swatch names come last.
    let mut used = std::collections::HashSet::new();
    fn walk(
        n: roxmltree::Node<'_, '_>,
        group: &mut PalGroup,
        pal: &mut Palette,
        materials: &std::collections::HashMap<String, PalEntry>,
        used: &mut std::collections::HashSet<String>,
    ) {
        for c in n.children().filter(|c| c.is_element()) {
            match c.tag_name().name() {
                "swatch" => {
                    if let Some(id) = c.attribute("material")
                        && let Some(e) = materials.get(id)
                    {
                        used.insert(id.to_string());
                        group.entries.push(e.clone());
                    }
                }
                "group" => {
                    let title = c
                        .children()
                        .find(|m| m.is_element() && m.tag_name().name() == "metadata")
                        .and_then(|m| m.children().find(|t| t.is_element() && t.tag_name().name() == "title"))
                        .and_then(|t| t.text())
                        .unwrap_or("Group")
                        .trim()
                        .to_string();
                    let mut g = PalGroup { name: title, entries: Vec::new() };
                    walk(c, &mut g, pal, materials, used);
                    pal.groups.push(g);
                }
                _ => {}
            }
        }
    }
    if let Some(book) = root.children().find(|c| c.is_element() && c.tag_name().name() == "book") {
        let mut loose = PalGroup::default();
        walk(book, &mut loose, &mut pal, &materials, &mut used);
        if !loose.entries.is_empty() {
            pal.groups.insert(0, loose);
        }
    }
    let mut rest: Vec<(&String, &PalEntry)> = materials.iter().filter(|(id, _)| !used.contains(*id)).collect();
    rest.sort_by(|a, b| a.0.cmp(b.0));
    for (_, e) in rest {
        pal.loose().entries.push(e.clone());
    }
    Ok(pal)
}

// ---------------------------------------------------------------------------
// Adobe Color Book (.acb)
// ---------------------------------------------------------------------------

fn be_u16(b: &[u8], at: usize) -> Result<u16, String> {
    b.get(at..at + 2).map(|s| u16::from_be_bytes([s[0], s[1]])).ok_or_else(|| "truncated colour book".to_string())
}

fn be_u32(b: &[u8], at: usize) -> Result<u32, String> {
    b.get(at..at + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| "truncated colour book".to_string())
}

/// A length-prefixed UTF-16BE string; returns it and the position after it.
fn utf16_string(b: &[u8], at: usize) -> Result<(String, usize), String> {
    let n = be_u32(b, at)? as usize;
    if n > 1 << 20 {
        return Err("colour book string is too long".into());
    }
    let s = b.get(at + 4..at + 4 + n * 2).ok_or_else(|| "truncated colour book string".to_string())?;
    let units: Vec<u16> = s.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
    Ok((String::from_utf16_lossy(&units), at + 4 + n * 2))
}

/// Strip the `$$$/colorbook/…/…=` resource key a book's strings often start with.
fn book_text(s: &str) -> String {
    match s.rfind('=') {
        Some(i) if s.starts_with("$$$") => s[i + 1..].trim().to_string(),
        _ => s.trim().to_string(),
    }
}

/// `8BCB`, version, book id, title, colour name prefix and suffix, description, colour count,
/// page size, page selector offset, colour space (0 RGB, 2 CMYK, 7 Lab), then per colour its
/// name, a 6-byte code and 3 or 4 component bytes.
pub fn read_acb(bytes: &[u8]) -> Result<Palette, String> {
    if !bytes.starts_with(b"8BCB") {
        return Err("not an Adobe colour book (missing 8BCB)".into());
    }
    let _version = be_u16(bytes, 4)?;
    let _id = be_u16(bytes, 6)?;
    let (title, at) = utf16_string(bytes, 8)?;
    let (prefix, at) = utf16_string(bytes, at)?;
    let (suffix, at) = utf16_string(bytes, at)?;
    let (_description, at) = utf16_string(bytes, at)?;
    let count = be_u16(bytes, at)? as usize;
    let page = be_u16(bytes, at + 2)?;
    let _selector = be_u16(bytes, at + 4)?;
    let space = be_u16(bytes, at + 6)?;
    let mut at = at + 8;
    let (prefix, suffix) = (book_text(&prefix), book_text(&suffix));
    let mut pal = Palette { name: book_text(&title), columns: u32::from(page), ..Default::default() };
    for i in 0..count {
        let (name, next) = utf16_string(bytes, at)?;
        bytes.get(next..next + 6).ok_or_else(|| "truncated colour book".to_string())?;
        let comps = if space == 2 { 4 } else { 3 };
        let c = bytes.get(next + 6..next + 6 + comps).ok_or_else(|| "truncated colour book".to_string())?;
        at = next + 6 + comps;
        let color = match space {
            0 => byte_color(c[0], c[1], c[2]),
            2 => PalColor::Cmyk([1.0 - f32::from(c[0]) / 255.0, 1.0 - f32::from(c[1]) / 255.0, 1.0 - f32::from(c[2]) / 255.0, 1.0 - f32::from(c[3]) / 255.0]),
            7 => PalColor::Lab([f32::from(c[0]) / 255.0 * 100.0, f32::from(c[1]) - 128.0, f32::from(c[2]) - 128.0]),
            s => return Err(format!("colour book colour space {s} is not supported")),
        };
        let name = book_text(&name);
        if name.is_empty() {
            // Page separators carry no name.
            continue;
        }
        pal.loose().entries.push(PalEntry { name: format!("{prefix}{name}{suffix}"), color, spot: true });
        if i > MAX_COLORS {
            return Err(format!("more than {MAX_COLORS} colours"));
        }
    }
    Ok(pal)
}

// ---------------------------------------------------------------------------
// Native zip palette (.kpl)
// ---------------------------------------------------------------------------

/// The mimetype entry of a `.kpl`.
pub const KPL_MIMETYPE: &str = "application/x-krita-palette";

/// A zip holding `colorset.xml`: `<ColorSet name= comment= columns= rows=>` with
/// `<ColorSetEntry name= id= spot= bitdepth=>` children holding one of `<RGB r g b>`, `<CMYK c m
/// y k>`, `<Lab L a b>`, `<Gray g>` (components 0..1, Lab 0..100), grouped by `<Group name=>`.
pub fn read_kpl(bytes: &[u8]) -> Result<Palette, String> {
    let zip = ZipReader::new(bytes).map_err(|e| format!("not a zip palette: {e}"))?;
    let xml = zip.read_by_name("colorset.xml", MAX_ENTRY).map_err(|e| format!("not a zip palette: {e}"))?;
    let text = String::from_utf8_lossy(&xml);
    let doc = roxmltree::Document::parse(&text).map_err(|e| format!("bad colorset.xml: {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "ColorSet" {
        return Err("bad colorset.xml (missing ColorSet)".into());
    }
    let mut pal = Palette {
        name: root.attribute("name").unwrap_or("").to_string(),
        columns: root.attribute("columns").and_then(|c| c.parse().ok()).unwrap_or(0),
        ..Default::default()
    };
    fn entry_of(n: roxmltree::Node<'_, '_>, i: usize) -> Option<PalEntry> {
        let name = n.attribute("name").filter(|s| !s.trim().is_empty()).map_or_else(|| format!("Colour {}", i + 1), str::to_string);
        let spot = n.attribute("spot").is_some_and(|v| v == "true" || v == "1");
        let v = n.children().find(|c| c.is_element())?;
        let f = |k: &str| v.attribute(k).and_then(|s| s.trim().parse::<f32>().ok()).filter(|x| x.is_finite());
        let color = match v.tag_name().name() {
            "RGB" | "sRGB" => PalColor::Rgb([f("r")?, f("g")?, f("b")?]),
            "CMYK" => PalColor::Cmyk([f("c")?, f("m")?, f("y")?, f("k")?]),
            "Lab" | "LAB" => PalColor::Lab([f("L").or_else(|| f("l"))?, f("a")?, f("b")?]),
            "Gray" | "GRAY" => PalColor::Gray(1.0 - f("g")?),
            _ => return None,
        };
        Some(PalEntry { name, color, spot })
    }
    let mut i = 0usize;
    for c in root.children().filter(|c| c.is_element()) {
        match c.tag_name().name() {
            "ColorSetEntry" => {
                match entry_of(c, i) {
                    Some(e) => pal.loose().entries.push(e),
                    None => pal.warnings.push(format!("entry {} has no readable colour", i + 1)),
                }
                i += 1;
            }
            "Group" => {
                let mut g = PalGroup { name: c.attribute("name").unwrap_or("Group").to_string(), entries: Vec::new() };
                for e in c.children().filter(|e| e.is_element() && e.tag_name().name() == "ColorSetEntry") {
                    match entry_of(e, i) {
                        Some(x) => g.entries.push(x),
                        None => pal.warnings.push(format!("entry {} has no readable colour", i + 1)),
                    }
                    i += 1;
                }
                pal.groups.push(g);
            }
            _ => {}
        }
        if i > MAX_COLORS {
            return Err(format!("more than {MAX_COLORS} colours"));
        }
    }
    Ok(pal)
}

/// Write a native zip palette (`mimetype`, `colorset.xml`, `profiles.xml`).
pub fn write_kpl(pal: &Palette) -> Result<Vec<u8>, String> {
    let mut xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<ColorSet version=\"2.0\" name=\"{}\" comment=\"\" columns=\"{}\" rows=\"{}\">\n",
        xml_escape(&pal.name),
        pal.columns.max(1),
        pal.count().div_ceil(pal.columns.max(1) as usize).max(1)
    );
    let mut id = 0usize;
    let mut push_entry = |xml: &mut String, e: &PalEntry, indent: &str| {
        id += 1;
        xml.push_str(&format!("{indent}<ColorSetEntry name=\"{}\" id=\"{id}\" spot=\"{}\" bitdepth=\"F32\">\n", xml_escape(&e.name), e.spot));
        match e.color {
            PalColor::Rgb([r, g, b]) => xml.push_str(&format!("{indent}  <RGB r=\"{r}\" g=\"{g}\" b=\"{b}\" space=\"sRGB-elle-V2-srgbtrc.icc\"/>\n")),
            PalColor::Cmyk([c, m, y, k]) => xml.push_str(&format!("{indent}  <CMYK c=\"{c}\" m=\"{m}\" y=\"{y}\" k=\"{k}\" space=\"Chemical proof\"/>\n")),
            PalColor::Lab([l, a, b]) => xml.push_str(&format!("{indent}  <Lab L=\"{l}\" a=\"{a}\" b=\"{b}\" space=\"Lab identity built-in\"/>\n")),
            PalColor::Gray(k) => xml.push_str(&format!("{indent}  <Gray g=\"{}\" space=\"Gray-D50-elle-V2-srgbtrc.icc\"/>\n", 1.0 - k)),
        }
        xml.push_str(&format!("{indent}</ColorSetEntry>\n"));
    };
    for g in &pal.groups {
        if g.name.is_empty() {
            for e in &g.entries {
                push_entry(&mut xml, e, "  ");
            }
        } else {
            xml.push_str(&format!("  <Group name=\"{}\" rows=\"{}\">\n", xml_escape(&g.name), g.entries.len().div_ceil(pal.columns.max(1) as usize).max(1)));
            for e in &g.entries {
                push_entry(&mut xml, e, "    ");
            }
            xml.push_str("  </Group>\n");
        }
    }
    xml.push_str("</ColorSet>\n");
    let mut zip = ZipWriter::new();
    zip.add("mimetype", KPL_MIMETYPE.as_bytes()).map_err(|e| e.to_string())?;
    zip.add("colorset.xml", xml.as_bytes()).map_err(|e| e.to_string())?;
    zip.add("profiles.xml", b"<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Profiles/>\n").map_err(|e| e.to_string())?;
    zip.finish().map_err(|e| e.to_string())
}

/// Write a palette in `kind` (the formats with a writer).
pub fn write(pal: &Palette, kind: PaletteKind) -> Result<Vec<u8>, String> {
    Ok(match kind {
        PaletteKind::Gpl => write_gpl(pal),
        PaletteKind::RiffPal => write_riff(pal),
        PaletteKind::Act => write_act(pal),
        PaletteKind::JascPal => write_jasc(pal),
        PaletteKind::ScribusXml => write_scribus(pal),
        PaletteKind::Kpl => write_kpl(pal)?,
        PaletteKind::Sbz | PaletteKind::Acb => return Err(format!("{} palettes are import only", kind.id())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pal() -> Palette {
        Palette {
            name: "Test".into(),
            columns: 2,
            groups: vec![
                PalGroup {
                    name: String::new(),
                    entries: vec![entry("Red", PalColor::Rgb([1.0, 0.0, 0.0])), entry("Ink", PalColor::Cmyk([0.0, 0.0, 0.0, 1.0]))],
                },
                PalGroup { name: "Grays".into(), entries: vec![entry("Mid", PalColor::Gray(0.5))] },
            ],
            warnings: Vec::new(),
        }
    }

    #[test]
    fn gpl_round_trips_rgb() {
        let bytes = write_gpl(&pal());
        assert_eq!(detect(&bytes, None), Some(PaletteKind::Gpl));
        let back = read(&bytes, PaletteKind::Gpl, "x").unwrap();
        assert_eq!(back.name, "Test");
        assert_eq!(back.count(), 3);
        assert_eq!(back.groups[0].entries[0].name, "Red");
    }

    #[test]
    fn riff_act_jasc_round_trip_counts() {
        for k in [PaletteKind::RiffPal, PaletteKind::Act, PaletteKind::JascPal] {
            let bytes = write(&pal(), k).unwrap();
            let back = read(&bytes, k, "x").unwrap();
            assert_eq!(back.count(), 3, "{k:?}");
            if k != PaletteKind::Act {
                assert_eq!(detect(&bytes, None), Some(k));
            }
        }
    }

    #[test]
    fn scribus_and_kpl_keep_models() {
        let s = write_scribus(&pal());
        let back = read(&s, PaletteKind::ScribusXml, "x").unwrap();
        assert!(matches!(back.groups[0].entries[1].color, PalColor::Cmyk(_)));
        let k = write_kpl(&pal()).unwrap();
        assert_eq!(detect(&k, None), Some(PaletteKind::Kpl));
        let back = read(&k, PaletteKind::Kpl, "x").unwrap();
        assert_eq!(back.groups.len(), 2);
        assert_eq!(back.groups[1].name, "Grays");
        assert!(matches!(back.groups[1].entries[0].color, PalColor::Gray(g) if (g - 0.5).abs() < 1e-5));
    }

    #[test]
    fn acb_reads_lab_book() {
        let mut b = Vec::new();
        b.extend_from_slice(b"8BCB");
        b.extend_from_slice(&1u16.to_be_bytes());
        b.extend_from_slice(&3000u16.to_be_bytes());
        let s = |t: &str, out: &mut Vec<u8>| {
            let u: Vec<u16> = t.encode_utf16().collect();
            out.extend_from_slice(&(u.len() as u32).to_be_bytes());
            for c in u {
                out.extend_from_slice(&c.to_be_bytes());
            }
        };
        s("$$$/colorbook/Book/title=My Book", &mut b);
        s("", &mut b);
        s(" C", &mut b);
        s("", &mut b);
        b.extend_from_slice(&1u16.to_be_bytes());
        b.extend_from_slice(&9u16.to_be_bytes());
        b.extend_from_slice(&0u16.to_be_bytes());
        b.extend_from_slice(&7u16.to_be_bytes());
        s("$$$/colorbook/Book/Red=Red", &mut b);
        b.extend_from_slice(b"RED   ");
        b.extend_from_slice(&[128, 200, 150]);
        let pal = read(&b, PaletteKind::Acb, "x").unwrap();
        assert_eq!(pal.name, "My Book");
        assert_eq!(pal.groups[0].entries[0].name, "Red C");
        assert!(matches!(pal.groups[0].entries[0].color, PalColor::Lab([l, a, _]) if (l - 50.2).abs() < 0.1 && (a - 72.0).abs() < 1e-3));
    }
}
