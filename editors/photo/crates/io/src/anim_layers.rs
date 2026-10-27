//! Animation frames from layers: each top-level layer is one frame, and its name carries the
//! frame's timing and disposal in parentheses, `Frame 3 (250ms) (replace)`. `(NNNms)` or `(NNN)`
//! is the delay in milliseconds (100 when absent); `(combine)` draws the frame over what the
//! earlier frames left (the default), `(replace)` shows the layer alone. The GIF, WebP, MNG, FLI
//! and ANI writers take these frames; a document with one layer is one frame.

use redrob_photo_doc::Document;

use crate::IoError;

/// How a frame relates to the one before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposal {
    Combine,
    Replace,
}

/// What a layer name says about its frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tags {
    pub delay_ms: Option<u32>,
    pub disposal: Option<Disposal>,
}

/// The `(…)` tags of `name`: a number (with or without `ms`) is the delay, `combine` and
/// `replace` the disposal; anything else in parentheses is left alone.
pub fn tags(name: &str) -> Tags {
    let mut t = Tags { delay_ms: None, disposal: None };
    let mut rest = name;
    while let Some(open) = rest.find('(') {
        let Some(close) = rest[open..].find(')') else { break };
        let inner = rest[open + 1..open + close].trim().to_ascii_lowercase();
        if let Some(ms) = inner.strip_suffix("ms").map(str::trim).or(Some(inner.as_str())).and_then(|s| s.parse::<u32>().ok()) {
            t.delay_ms = Some(ms);
        } else if inner == "combine" {
            t.disposal = Some(Disposal::Combine);
        } else if inner == "replace" {
            t.disposal = Some(Disposal::Replace);
        }
        rest = &rest[open + close + 1..];
    }
    t
}

/// `name` with its frame tags removed (what an imported frame's layer is called).
pub fn strip_tags(name: &str) -> String {
    let mut out = String::new();
    let mut rest = name;
    while let Some(open) = rest.find('(') {
        let Some(close) = rest[open..].find(')') else { break };
        let inner = rest[open + 1..open + close].trim().to_ascii_lowercase();
        let is_tag = inner == "combine"
            || inner == "replace"
            || inner.strip_suffix("ms").map(str::trim).or(Some(inner.as_str())).is_some_and(|s| s.parse::<u32>().is_ok());
        if is_tag {
            out.push_str(rest[..open].trim_end());
            out.push(' ');
        } else {
            out.push_str(&rest[..open + close + 1]);
        }
        rest = &rest[open + close + 1..];
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The layer name for frame `index` (1-based) with `delay_ms` and `disposal` as tags.
pub fn frame_name(base: &str, index: usize, delay_ms: u32, disposal: Disposal) -> String {
    let d = match disposal {
        Disposal::Combine => "combine",
        Disposal::Replace => "replace",
    };
    format!("{base} {index} ({delay_ms}ms) ({d})")
}

/// One rendered frame.
pub struct Frame {
    pub w: u32,
    pub h: u32,
    /// Straight RGBA8 of the whole canvas.
    pub rgba: Vec<u8>,
    pub delay_ms: u32,
    pub disposal: Disposal,
    pub name: String,
}

/// Largest layer count rendered as frames.
pub const MAX_FRAMES: usize = 4096;

/// Every visible top-level layer of `doc` as a frame, bottom to top. A `combine` frame is the
/// composite of itself over all earlier frames; a `replace` frame is the layer alone.
pub fn frames(doc: &Document, warnings: &mut Vec<String>) -> Result<Vec<Frame>, IoError> {
    let layers: Vec<usize> = doc.layers.iter().enumerate().filter(|(_, l)| l.visible).map(|(i, _)| i).collect();
    if layers.len() > MAX_FRAMES {
        return Err(IoError::Unsupported(format!("{} layers; animations are limited to {MAX_FRAMES} frames", layers.len())));
    }
    if layers.len() <= 1 {
        let (w, h, rgba) = crate::legacy::flatten(doc, warnings)?;
        let name = doc.layers.first().map(|l| l.name.clone()).unwrap_or_default();
        let t = tags(&name);
        return Ok(vec![Frame { w, h, rgba, delay_ms: t.delay_ms.unwrap_or(100), disposal: Disposal::Replace, name: strip_tags(&name) }]);
    }
    let mut out = Vec::with_capacity(layers.len());
    for (k, &i) in layers.iter().enumerate() {
        let t = tags(&doc.layers[i].name);
        let disposal = t.disposal.unwrap_or(Disposal::Combine);
        let mut d = doc.clone();
        for (j, l) in d.layers.iter_mut().enumerate() {
            l.visible = match disposal {
                Disposal::Replace => j == i,
                Disposal::Combine => layers[..=k].contains(&j),
            };
        }
        let (w, h, rgba) = crate::legacy::flatten(&d, warnings)?;
        out.push(Frame { w, h, rgba, delay_ms: t.delay_ms.unwrap_or(100), disposal, name: strip_tags(&doc.layers[i].name) });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_are_read_and_stripped() {
        let t = tags("Frame 2 (250ms) (replace)");
        assert_eq!(t.delay_ms, Some(250));
        assert_eq!(t.disposal, Some(Disposal::Replace));
        assert_eq!(tags("Frame (40)").delay_ms, Some(40));
        assert_eq!(tags("Frame (combine)").disposal, Some(Disposal::Combine));
        assert_eq!(tags("plain").delay_ms, None);
        assert_eq!(strip_tags("Frame 2 (250ms) (replace)"), "Frame 2");
        assert_eq!(strip_tags("Logo (final) (100ms)"), "Logo (final)");
        assert_eq!(frame_name("Frame", 1, 100, Disposal::Combine), "Frame 1 (100ms) (combine)");
    }
}
