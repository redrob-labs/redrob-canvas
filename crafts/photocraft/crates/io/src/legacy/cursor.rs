//! Windows cursors (`.cur`: an icon directory of type 2 whose entries carry the hotspot in
//! place of planes and bit depth) and animated cursors (`.ani`: a RIFF `ACON` with an `anih`
//! header, optional `rate` and `seq` chunks, and a `LIST fram` of `icon` chunks, each a complete
//! cursor or icon file). The pixel data goes through the ICO codec.

use redrob_photo_codecs::{EncodeOptions, Format, Image};
use redrob_photo_doc::Document;

use super::{Rgba, bad, le16, le32};
use crate::IoError;

const MAX_FRAMES: usize = 1024;

fn rgba_of(img: &Image, warnings: Vec<String>) -> Result<Rgba, IoError> {
    let (w, h) = img.dimensions();
    let mut out = Rgba::new(w, h)?;
    out.data = img.to_rgba8();
    out.warnings = warnings;
    Ok(out)
}

/// An ICO byte image of a `.cur`: type 1, hotspots cleared, so the ICO decoder picks the
/// largest image by size.
fn as_ico(bytes: &[u8]) -> Result<(Vec<u8>, (u16, u16)), IoError> {
    if bytes.len() < 6 {
        return Err(bad("CUR", "file too short"));
    }
    let mut b = bytes.to_vec();
    b[2] = 1;
    b[3] = 0;
    let n = le16(&b, 4).unwrap_or(0) as usize;
    let mut hot = (0, 0);
    for i in 0..n {
        let at = 6 + i * 16;
        if at + 16 > b.len() {
            break;
        }
        if i == 0 {
            hot = (le16(&b, at + 4).unwrap_or(0), le16(&b, at + 6).unwrap_or(0));
        }
        b[at + 4..at + 8].copy_from_slice(&[0, 0, 0, 0]);
    }
    Ok((b, hot))
}

pub fn decode_cur(bytes: &[u8]) -> Result<Rgba, IoError> {
    let (ico, hot) = as_ico(bytes)?;
    let img = redrob_photo_codecs::decode_as(Format::Ico, &ico)?;
    let mut warnings = Vec::new();
    if hot != (0, 0) {
        warnings.push(format!("cursor hotspot ({}, {}) is not kept", hot.0, hot.1));
    }
    rgba_of(&img, warnings)
}

/// Every image of an `.ani`: (pixels, display time in milliseconds) per step, in `seq` order.
pub fn ani_frames(bytes: &[u8]) -> Result<Vec<(Image, u32)>, IoError> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"ACON" {
        return Err(bad("ANI", "not a RIFF ACON file"));
    }
    let mut icons: Vec<&[u8]> = Vec::new();
    let mut rates: Vec<u32> = Vec::new();
    let mut seq: Vec<u32> = Vec::new();
    let mut default_rate = 10u32;
    let mut raw_icons = false;
    walk(&bytes[12..], 0, &mut |id, data| {
        match id {
            b"anih" if data.len() >= 36 => {
                default_rate = le32(data, 28).unwrap_or(10).max(1);
                raw_icons = le32(data, 32).unwrap_or(1) & 1 == 0;
            }
            b"rate" => rates = data.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect(),
            b"seq " => seq = data.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect(),
            b"icon" => {
                if icons.len() < MAX_FRAMES {
                    icons.push(data);
                }
            }
            _ => {}
        }
        Ok(())
    })?;
    if icons.is_empty() {
        return Err(bad("ANI", "no icon frames"));
    }
    if raw_icons {
        return Err(bad("ANI", "raw bitmap frames (no icon data) are not supported"));
    }
    let steps: Vec<u32> = if seq.is_empty() { (0..icons.len() as u32).collect() } else { seq };
    let mut out = Vec::with_capacity(steps.len());
    for (i, &s) in steps.iter().enumerate().take(MAX_FRAMES) {
        let icon = icons.get(s as usize).ok_or_else(|| bad("ANI", format!("sequence refers to frame {s} of {}", icons.len())))?;
        let (ico, _) = as_ico(icon)?;
        let img = redrob_photo_codecs::decode_as(Format::Ico, &ico)?;
        let jiffies = rates.get(i).copied().unwrap_or(default_rate).max(1);
        out.push((img, jiffies * 1000 / 60));
    }
    Ok(out)
}

/// RIFF chunk walk: `f(id, payload)` for every chunk, descending into `LIST`s.
fn walk<'a>(mut b: &'a [u8], depth: usize, f: &mut dyn FnMut(&[u8; 4], &'a [u8]) -> Result<(), IoError>) -> Result<(), IoError> {
    if depth > 8 {
        return Err(bad("ANI", "RIFF lists nested too deeply"));
    }
    while b.len() >= 8 {
        let id: [u8; 4] = [b[0], b[1], b[2], b[3]];
        let len = le32(b, 4).unwrap_or(0) as usize;
        let data = b.get(8..8 + len).ok_or_else(|| bad("ANI", "chunk runs past the end of the file"))?;
        if &id == b"LIST" {
            walk(data.get(4..).unwrap_or(&[]), depth + 1, f)?;
        } else {
            f(&id, data)?;
        }
        let skip = 8 + len + (len & 1);
        b = b.get(skip..).unwrap_or(&[]);
    }
    Ok(())
}

pub fn decode_ani(bytes: &[u8]) -> Result<Rgba, IoError> {
    let frames = ani_frames(bytes)?;
    let n = frames.len();
    let (first, _) = frames.into_iter().next().ok_or_else(|| bad("ANI", "no frames"))?;
    let mut warnings = Vec::new();
    if n > 1 {
        warnings.push(format!("animated cursor: {n} frames; the first opened (Import Video Frames to Layers opens them all)"));
    }
    rgba_of(&first, warnings)
}

fn ico_bytes(w: u32, h: u32, rgba: &[u8]) -> Result<Vec<u8>, IoError> {
    if w > 256 || h > 256 {
        return Err(IoError::Unsupported(format!("cursors are at most 256 × 256 pixels ({w} × {h})")));
    }
    let img = Image::from_u8(w, h, redrob_photo_codecs::ChannelLayout::Rgba, rgba.to_vec())?;
    Ok(redrob_photo_codecs::encode(&img, Format::Ico, &EncodeOptions::default())?)
}

/// A `.cur` with the hotspot at `hot` (the top-left corner when `None`).
pub fn encode_cur(w: u32, h: u32, rgba: &[u8], hot: Option<(u16, u16)>, _warnings: &mut [String]) -> Result<Vec<u8>, IoError> {
    let mut b = ico_bytes(w, h, rgba)?;
    if b.len() < 22 {
        return Err(bad("CUR", "icon encoder produced no directory"));
    }
    b[2] = 2;
    let (hx, hy) = hot.unwrap_or((0, 0));
    b[10..12].copy_from_slice(&hx.to_le_bytes());
    b[12..14].copy_from_slice(&hy.to_le_bytes());
    Ok(b)
}

fn chunk(out: &mut Vec<u8>, id: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(id);
    out.extend((data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    if data.len() & 1 == 1 {
        out.push(0);
    }
}

/// An `.ani` whose frames are the document's animation frames ([`crate::anim_layers::frames`]:
/// the layers with their `(delay)` name tags, or the flattened image alone).
pub fn encode_ani(doc: &Document, w: u32, h: u32, rgba: &[u8], warnings: &mut Vec<String>) -> Result<Vec<u8>, IoError> {
    let frames = crate::anim_layers::frames(doc, warnings)?;
    let frames: Vec<(Vec<u8>, u32)> = if frames.len() > 1 { frames.into_iter().map(|f| (f.rgba, f.delay_ms)).collect() } else { vec![(rgba.to_vec(), 100)] };
    let mut icons = Vec::with_capacity(frames.len());
    let mut rates = Vec::with_capacity(frames.len());
    for (px, delay) in &frames {
        icons.push(encode_cur(w, h, px, None, &mut [])?);
        rates.push((u64::from(*delay) * 60 / 1000).clamp(1, u64::from(u32::MAX)) as u32);
    }
    let n = icons.len() as u32;
    let mut anih = Vec::with_capacity(36);
    anih.extend(36u32.to_le_bytes());
    anih.extend(n.to_le_bytes()); // frames
    anih.extend(n.to_le_bytes()); // steps
    anih.extend([0u32; 4].iter().flat_map(|v| v.to_le_bytes())); // width, height, bit count, planes (icon data)
    anih.extend(rates.first().copied().unwrap_or(6).to_le_bytes());
    anih.extend(1u32.to_le_bytes()); // flags: icon data
    let mut body = b"ACON".to_vec();
    chunk(&mut body, b"anih", &anih);
    let rate_bytes: Vec<u8> = rates.iter().flat_map(|r| r.to_le_bytes()).collect();
    chunk(&mut body, b"rate", &rate_bytes);
    let mut fram = b"fram".to_vec();
    for icon in &icons {
        chunk(&mut fram, b"icon", icon);
    }
    chunk(&mut body, b"LIST", &fram);
    let mut out = b"RIFF".to_vec();
    out.extend((body.len() as u32).to_le_bytes());
    out.extend(body);
    Ok(out)
}
