// SPDX-License-Identifier: GPL-3.0-or-later

//! M7: colour swatch files. Reads GIMP palettes (`.gpl`, text) and Photoshop swatches (`.aco`,
//! binary, version 1 or 2, RGB / HSB / grayscale entries), and writes `.gpl`. Swatches are
//! application state, not document content, so this module only converts.

use crate::document::Pixel;
use crate::error::{CoreError, Result};

/// A palette holds at most this many colours; larger files are cut, not refused.
pub const MAX_SWATCHES: usize = 4_096;

/// Parses either format, recognised by content (ACO starts with a big-endian version 1 or 2).
pub fn parse_swatches(bytes: &[u8]) -> Result<Vec<Pixel>> {
    if bytes.starts_with(b"GIMP Palette") {
        return parse_gpl(bytes);
    }
    if bytes.len() >= 4 && bytes[0] == 0 && (bytes[1] == 1 || bytes[1] == 2) {
        return parse_aco(bytes);
    }
    Err(CoreError::InvalidSemanticStyle)
}

fn parse_gpl(bytes: &[u8]) -> Result<Vec<Pixel>> {
    let text = std::str::from_utf8(bytes).map_err(|_| CoreError::InvalidSemanticStyle)?;
    let mut colours = Vec::new();
    for line in text.lines().skip(1) {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with('#')
            || line.starts_with("Name:")
            || line.starts_with("Columns:")
        {
            continue;
        }
        let mut parts = line.split_whitespace();
        let channel = |part: Option<&str>| {
            part.and_then(|p| p.parse::<u16>().ok())
                .filter(|v| *v <= 255)
        };
        if let (Some(r), Some(g), Some(b)) = (
            channel(parts.next()),
            channel(parts.next()),
            channel(parts.next()),
        ) {
            colours.push(Pixel::rgba(r as u8, g as u8, b as u8, 255));
            if colours.len() == MAX_SWATCHES {
                break;
            }
        }
    }
    Ok(colours)
}

fn parse_aco(bytes: &[u8]) -> Result<Vec<Pixel>> {
    let u16_at = |at: usize| -> Option<u16> {
        Some(u16::from_be_bytes([*bytes.get(at)?, *bytes.get(at + 1)?]))
    };
    let count = usize::from(u16_at(2).ok_or(CoreError::InvalidSemanticStyle)?);
    let mut colours = Vec::new();
    let mut at = 4;
    for _ in 0..count.min(MAX_SWATCHES) {
        let space = u16_at(at).ok_or(CoreError::InvalidSemanticStyle)?;
        let w = |i: usize| u16_at(at + 2 + i * 2).unwrap_or(0);
        let byte = |v: u16| (v >> 8) as u8;
        let colour = match space {
            0 => Some(Pixel::rgba(byte(w(0)), byte(w(1)), byte(w(2)), 255)),
            1 => Some(hsb(
                f32::from(w(0)) / 65535.0,
                f32::from(w(1)) / 65535.0,
                f32::from(w(2)) / 65535.0,
            )),
            // Grayscale: 0..10000.
            8 => {
                let v = (255.0 - f32::from(w(0).min(10_000)) / 10_000.0 * 255.0).round() as u8;
                Some(Pixel::rgba(v, v, v, 255))
            }
            // CMYK, Lab and the rest need a colour profile; they are skipped rather than guessed.
            _ => None,
        };
        colours.extend(colour);
        at += 10;
    }
    if at > bytes.len() {
        return Err(CoreError::InvalidSemanticStyle);
    }
    Ok(colours)
}

fn hsb(h: f32, s: f32, v: f32) -> Pixel {
    let sector = (h * 6.0).rem_euclid(6.0);
    let f = sector.fract();
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    let (r, g, b) = match sector as u32 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    let byte = |c: f32| (c * 255.0).round().clamp(0.0, 255.0) as u8;
    Pixel::rgba(byte(r), byte(g), byte(b), 255)
}

/// A GIMP palette file (`.gpl`) for these colours; alpha is not part of the format.
pub fn write_gpl(name: &str, colours: &[Pixel]) -> String {
    let clean: String = name.chars().filter(|c| !c.is_control()).collect();
    let mut out = format!(
        "GIMP Palette\nName: {}\nColumns: 8\n#\n",
        if clean.trim().is_empty() {
            "Swatches"
        } else {
            clean.trim()
        }
    );
    for c in colours.iter().take(MAX_SWATCHES) {
        out.push_str(&format!(
            "{:3} {:3} {:3}\t#{:02x}{:02x}{:02x}\n",
            c.r, c.g, c.b, c.r, c.g, c.b
        ));
    }
    out
}
