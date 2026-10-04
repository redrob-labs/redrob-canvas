// SPDX-License-Identifier: GPL-3.0-or-later

//! X PixMap (M.7c, third of M.7's four formats).
//!
//! Re-derived from `plug-ins/common/file-xpm.c` (GPL-3.0-or-later), pinned in
//! `docs/upstream-sources.toml`. No usable pure-Rust XPM codec exists — searched in cycle 136:
//! `xpm 0.1.0` on the registry is a **package manager**, not this format, and `ez-pixmap`
//! describes itself as "naive" and "xpm-like". So the codec is written here.
//!
//! **Upstream does not parse XPM itself — it calls libXpm** (`XpmReadFileToXpmImage`). So the
//! text grammar comes from the format, and what is re-derived from GIMP's own code is everything
//! it does *around* that call. Four rules, each with a test:
//!
//! 1. **The colour spec is chosen by PREFERENCE ORDER, not by taking the first present.**
//!    `parse_colors` tries `c_color`, then `g_color`, then `g4_color`, then `m_color` — colour,
//!    then greyscale, then four-level grey, then mono. One entry may declare several visual types
//!    and the best available wins.
//! 2. **The default spec is the literal string `"None"`**, and `"None"` means TRANSPARENT.
//!    `parse_colors` initialises `colorspec = "None"` before looking at anything, so an entry
//!    declaring no usable visual becomes transparent rather than becoming an error or black.
//! 3. **A transparent entry is all zeros, alpha included.** The branch for `"None"` does `j += 4`
//!    over a `g_new0` buffer, leaving RGBA `(0, 0, 0, 0)`; opaque entries set alpha to `~0`.
//! 4. **Export derives `cpp` from an alphabet of 92 characters whose FIRST is a space**, and
//!    converts the palette index to base 92 **least-significant digit first**:
//!    `cpp = 1 + (gint) (log (ncolors) / log (sizeof (linenoise) - 1.0))`, then
//!    `charnum = indtemp % base; indtemp /= base; *p++ = linenoise[charnum]`. When the image has
//!    alpha, `ncolors++` and **entry 0 is reserved as `"None"`**.

use crate::{FormatError, Result};
use std::collections::HashMap;

/// Upstream's `linenoise`, measured rather than counted by eye: 92 characters, the first a SPACE.
/// `sizeof (linenoise) - 1` in upstream is this string's length, because `sizeof` counts the NUL.
const LINENOISE: &[u8] =
    b" .+@#$%&*=-;>,')!~{]^/(_:<[}|1234567890abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ`";

/// The magic upstream registers: `0, string,/*\040XPM\040*/`, where `\040` is a space.
const XPM_MAGIC: &[u8] = b"/* XPM */";

pub(crate) struct Xpm {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub(crate) fn looks_like_xpm(bytes: &[u8]) -> bool {
    bytes.starts_with(XPM_MAGIC)
}

/// Pull out the double-quoted string literals in order.
///
/// An XPM file is C source, and everything the format carries lives in those literals: the values
/// line, then the colour table, then the pixel rows. Escapes are honoured only as far as `\"` and
/// `\\`, which is all a conformant XPM needs.
fn string_literals(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '"' {
            continue;
        }
        let mut literal = String::new();
        while let Some(inner) = chars.next() {
            match inner {
                '"' => break,
                '\\' => {
                    if let Some(escaped) = chars.next() {
                        literal.push(escaped);
                    }
                }
                other => literal.push(other),
            }
        }
        out.push(literal);
    }
    out
}

/// Parse one colour spec to RGBA.
///
/// `None` is transparent — rule 2 above. The `#` forms are the three widths X11 defines. A NAME is
/// resolved from a small table: upstream hands names to `XParseColor` or `gdk_rgba_parse`, which
/// carry the whole X11 colour database, and reproducing that here would be inventing rather than
/// re-deriving. An unrecognised name is REFUSED by name instead of being guessed at — upstream
/// ignores `gdk_rgba_parse`'s failure return and uses whatever was left in the struct, which is
/// undefined, and replicating undefined behaviour is worse than refusing.
fn parse_color(spec: &str) -> Result<[u8; 4]> {
    if spec.eq_ignore_ascii_case("none") {
        // All zeros, alpha included -- rule 3.
        return Ok([0, 0, 0, 0]);
    }
    if let Some(hex) = spec.strip_prefix('#') {
        let per = match hex.len() {
            3 => 1,
            6 => 2,
            12 => 4,
            _ => {
                return Err(FormatError::UnsupportedFeature(
                    "XPM hex colour is not 3, 6 or 12 digits",
                )
                .into());
            }
        };
        let mut channel = [0u8; 3];
        for (index, slot) in channel.iter_mut().enumerate() {
            let piece = &hex[index * per..(index + 1) * per];
            let value = u32::from_str_radix(piece, 16)
                .map_err(|_| FormatError::UnsupportedFeature("XPM hex colour is not hex"))?;
            // Scale whatever width was given up to 8 bits: one digit is 0..15, four are 0..65535.
            *slot = match per {
                1 => (value * 17) as u8,
                2 => value as u8,
                _ => (value >> 8) as u8,
            };
        }
        return Ok([channel[0], channel[1], channel[2], 255]);
    }
    let named: &[(&str, [u8; 3])] = &[
        ("black", [0, 0, 0]),
        ("white", [255, 255, 255]),
        ("red", [255, 0, 0]),
        ("green", [0, 255, 0]),
        ("blue", [0, 0, 255]),
        ("cyan", [0, 255, 255]),
        ("magenta", [255, 0, 255]),
        ("yellow", [255, 255, 0]),
        ("gray", [190, 190, 190]),
        ("grey", [190, 190, 190]),
    ];
    for (name, rgb) in named {
        if spec.eq_ignore_ascii_case(name) {
            return Ok([rgb[0], rgb[1], rgb[2], 255]);
        }
    }
    Err(FormatError::UnsupportedFeature("XPM colour name is not recognised").into())
}

/// Choose the spec for one colour-table line by upstream's preference order.
///
/// The line is `<cpp chars><key> <spec>...`, where a key is `c`, `g`, `g4`, `m` or `s`. `s` is a
/// symbolic name and is never a colour. The default is `"None"` — rule 2 — so a line with no
/// usable visual is transparent, not an error.
fn spec_for_line(line: &str, cpp: usize) -> Result<[u8; 4]> {
    if line.len() < cpp {
        return Err(FormatError::UnsupportedFeature("XPM colour line is too short").into());
    }
    let rest = &line[cpp..];
    let tokens: Vec<&str> = rest.split_whitespace().collect();

    let mut best: HashMap<&str, String> = HashMap::new();
    let mut index = 0usize;
    while index < tokens.len() {
        let key = tokens[index];
        if !matches!(key, "c" | "g" | "g4" | "m" | "s") {
            index += 1;
            continue;
        }
        // A spec can be several words ("light sea green"), so take tokens until the next key.
        let mut value = Vec::new();
        index += 1;
        while index < tokens.len() && !matches!(tokens[index], "c" | "g" | "g4" | "m" | "s") {
            value.push(tokens[index]);
            index += 1;
        }
        if !value.is_empty() {
            best.entry(key).or_insert_with(|| value.join(" "));
        }
    }

    // Rule 1: colour, then grey, then four-level grey, then mono. Not "first one present".
    for key in ["c", "g", "g4", "m"] {
        if let Some(spec) = best.get(key) {
            return parse_color(spec);
        }
    }
    // Rule 2: the default is "None".
    parse_color("None")
}

pub(crate) fn decode(bytes: &[u8]) -> Result<Xpm> {
    if !looks_like_xpm(bytes) {
        return Err(FormatError::UnsupportedFeature("not an XPM image").into());
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| FormatError::UnsupportedFeature("XPM is not valid UTF-8"))?;
    let literals = string_literals(text);
    let values = literals
        .first()
        .ok_or(FormatError::UnsupportedFeature("XPM has no values line"))?;

    let fields: Vec<&str> = values.split_whitespace().collect();
    if fields.len() < 4 {
        return Err(FormatError::UnsupportedFeature("XPM values line is short").into());
    }
    let parse = |at: usize| -> Result<usize> {
        fields[at]
            .parse::<usize>()
            .map_err(|_| FormatError::UnsupportedFeature("XPM values line is not numeric").into())
    };
    let width = parse(0)?;
    let height = parse(1)?;
    let ncolors = parse(2)?;
    let cpp = parse(3)?;

    if width == 0 || height == 0 || cpp == 0 {
        return Err(FormatError::UnsupportedFeature("XPM declares an empty image").into());
    }
    crate::document::pixel_count(width as u32, height as u32)?;

    if literals.len() < 1 + ncolors + height {
        return Err(FormatError::UnsupportedFeature("XPM has too few strings").into());
    }

    // The colour table: the first `cpp` characters are the key this entry is drawn with.
    let mut palette: HashMap<&str, [u8; 4]> = HashMap::new();
    for line in literals.iter().skip(1).take(ncolors) {
        if line.len() < cpp {
            return Err(FormatError::UnsupportedFeature("XPM colour key is truncated").into());
        }
        palette.insert(&line[..cpp], spec_for_line(line, cpp)?);
    }

    let mut rgba = Vec::with_capacity(width * height * 4);
    for row in literals.iter().skip(1 + ncolors).take(height) {
        if row.len() < width * cpp {
            return Err(FormatError::UnsupportedFeature("XPM pixel row is short").into());
        }
        for x in 0..width {
            let key = &row[x * cpp..(x + 1) * cpp];
            let entry = palette.get(key).ok_or(FormatError::UnsupportedFeature(
                "XPM pixel names no known colour",
            ))?;
            rgba.extend_from_slice(entry);
        }
    }

    Ok(Xpm {
        width: width as u32,
        height: height as u32,
        rgba,
    })
}

/// The index-to-characters conversion, **least-significant digit FIRST**.
///
/// Upstream writes `charnum = indtemp % base; indtemp /= base; *p++ = linenoise[charnum]`, so the
/// low digit lands in the first character. Most people would write it the other way round, which
/// is exactly why this has its own test.
fn key_for(index: usize, cpp: usize) -> String {
    let base = LINENOISE.len();
    let mut remaining = index;
    let mut out = String::with_capacity(cpp);
    for _ in 0..cpp {
        out.push(LINENOISE[remaining % base] as char);
        remaining /= base;
    }
    out
}

/// `cpp = 1 + (gint) (log (ncolors) / log (base))`, with C's integer truncation.
fn cpp_for(ncolors: usize) -> usize {
    if ncolors <= 1 {
        return 1;
    }
    let base = LINENOISE.len() as f64;
    1 + ((ncolors as f64).ln() / base.ln()) as usize
}

pub(crate) fn encode(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    let (w, h) = (width as usize, height as usize);

    // Any pixel below full opacity is carried by the reserved transparent entry, the way upstream
    // reserves index 0 when the drawable has alpha.
    let alpha_used = rgba.chunks_exact(4).any(|pixel| pixel[3] != 255);

    // Collect the distinct opaque colours, in first-seen order so the output is deterministic.
    let mut order: Vec<[u8; 3]> = Vec::new();
    let mut seen: HashMap<[u8; 3], usize> = HashMap::new();
    for pixel in rgba.chunks_exact(4) {
        if pixel[3] != 255 {
            continue;
        }
        let key = [pixel[0], pixel[1], pixel[2]];
        seen.entry(key).or_insert_with(|| {
            order.push(key);
            order.len() - 1
        });
    }

    let ncolors = order.len() + usize::from(alpha_used);
    if ncolors == 0 {
        return Err(FormatError::UnsupportedFeature("XPM needs at least one colour").into());
    }
    let cpp = cpp_for(ncolors);

    // Rule 4: when alpha is used, entry 0 is "None" and the real colours start at 1.
    let offset = usize::from(alpha_used);

    let mut out = String::new();
    out.push_str("/* XPM */\n");
    out.push_str("static char * image[] = {\n");
    out.push_str(&format!("\"{w} {h} {ncolors} {cpp}\",\n"));
    if alpha_used {
        out.push_str(&format!("\"{}\tc None\",\n", key_for(0, cpp)));
    }
    for (index, rgb) in order.iter().enumerate() {
        out.push_str(&format!(
            "\"{}\tc #{:02X}{:02X}{:02X}\",\n",
            key_for(index + offset, cpp),
            rgb[0],
            rgb[1],
            rgb[2]
        ));
    }
    for y in 0..h {
        out.push('"');
        for x in 0..w {
            let at = (y * w + x) * 4;
            let key = if rgba[at + 3] != 255 {
                key_for(0, cpp)
            } else {
                let index = seen[&[rgba[at], rgba[at + 1], rgba[at + 2]]];
                key_for(index + offset, cpp)
            };
            out.push_str(&key);
        }
        out.push('"');
        if y + 1 < h {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str("};\n");
    Ok(out.into_bytes())
}
