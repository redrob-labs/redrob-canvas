//! Autodesk Animator FLI (`.fli`, 320 × 200, 1/70 s ticks) and FLC (`.flc`, any size, millisecond
//! ticks): a 128-byte header, then frames of chunks. Chunks read: COLOR_256 (4), DELTA_FLC (7),
//! COLOR_64 (11), DELTA_FLI (12), BLACK (13), BYTE_RUN (15), LITERAL (16); PSTAMP (18) and the
//! prefix chunk are skipped. Writes BYTE_RUN frames with a 256-colour palette per frame.

use redrob_photo_codecs::{ChannelLayout, Image};
use redrob_photo_doc::Document;

use super::{Rgba, bad, le16, le32};
use crate::IoError;

const MAGIC_FLI: u16 = 0xAF11;
const MAGIC_FLC: u16 = 0xAF12;
const FRAME_MAGIC: u16 = 0xF1FA;
const MAX_FRAMES: usize = 65_535;

struct State {
    w: usize,
    h: usize,
    idx: Vec<u8>,
    pal: [[u8; 3]; 256],
}

impl State {
    fn rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.idx.len() * 4);
        for &i in &self.idx {
            let c = self.pal[i as usize];
            out.extend([c[0], c[1], c[2], 255]);
        }
        out
    }
}

fn colors(st: &mut State, data: &[u8], six_bit: bool) -> Result<(), IoError> {
    let packets = le16(data, 0).ok_or_else(|| bad("FLI", "colour chunk ends early"))? as usize;
    let mut i = 2usize;
    let mut at = 0usize;
    for _ in 0..packets {
        let skip = *data.get(i).ok_or_else(|| bad("FLI", "colour packet ends early"))? as usize;
        let mut count = *data.get(i + 1).ok_or_else(|| bad("FLI", "colour packet ends early"))? as usize;
        i += 2;
        if count == 0 {
            count = 256;
        }
        at += skip;
        for _ in 0..count {
            let c = data.get(i..i + 3).ok_or_else(|| bad("FLI", "colour packet ends early"))?;
            i += 3;
            if at < 256 {
                st.pal[at] = if six_bit { [c[0] << 2, c[1] << 2, c[2] << 2] } else { [c[0], c[1], c[2]] };
            }
            at += 1;
        }
    }
    Ok(())
}

fn byte_run(st: &mut State, data: &[u8]) -> Result<(), IoError> {
    let mut i = 0usize;
    for y in 0..st.h {
        // The packet count byte is obsolete (rows can need more than 255); run to the row's end.
        i += 1;
        let mut x = 0usize;
        while x < st.w {
            let n = *data.get(i).ok_or_else(|| bad("FLI", "byte run ends early"))? as i8;
            i += 1;
            if n > 0 {
                let v = *data.get(i).ok_or_else(|| bad("FLI", "byte run ends early"))?;
                i += 1;
                for _ in 0..n {
                    if x < st.w {
                        st.idx[y * st.w + x] = v;
                    }
                    x += 1;
                }
            } else if n < 0 {
                let n = (-(n as i16)) as usize;
                let lit = data.get(i..i + n).ok_or_else(|| bad("FLI", "byte run ends early"))?;
                i += n;
                for &v in lit {
                    if x < st.w {
                        st.idx[y * st.w + x] = v;
                    }
                    x += 1;
                }
            } else {
                break;
            }
        }
    }
    Ok(())
}

fn literal(st: &mut State, data: &[u8]) -> Result<(), IoError> {
    let n = st.w * st.h;
    let src = data.get(..n).ok_or_else(|| bad("FLI", "literal chunk ends early"))?;
    st.idx.copy_from_slice(src);
    Ok(())
}

fn delta_fli(st: &mut State, data: &[u8]) -> Result<(), IoError> {
    let first = le16(data, 0).ok_or_else(|| bad("FLI", "delta ends early"))? as usize;
    let lines = le16(data, 2).ok_or_else(|| bad("FLI", "delta ends early"))? as usize;
    let mut i = 4usize;
    for y in first..(first + lines).min(st.h) {
        let packets = *data.get(i).ok_or_else(|| bad("FLI", "delta ends early"))? as usize;
        i += 1;
        let mut x = 0usize;
        for _ in 0..packets {
            x += *data.get(i).ok_or_else(|| bad("FLI", "delta ends early"))? as usize;
            let n = *data.get(i + 1).ok_or_else(|| bad("FLI", "delta ends early"))? as i8;
            i += 2;
            if n > 0 {
                let lit = data.get(i..i + n as usize).ok_or_else(|| bad("FLI", "delta ends early"))?;
                i += n as usize;
                for &v in lit {
                    if x < st.w {
                        st.idx[y * st.w + x] = v;
                    }
                    x += 1;
                }
            } else if n < 0 {
                let v = *data.get(i).ok_or_else(|| bad("FLI", "delta ends early"))?;
                i += 1;
                for _ in 0..(-(n as i16)) {
                    if x < st.w {
                        st.idx[y * st.w + x] = v;
                    }
                    x += 1;
                }
            }
        }
    }
    Ok(())
}

fn delta_flc(st: &mut State, data: &[u8]) -> Result<(), IoError> {
    let lines = le16(data, 0).ok_or_else(|| bad("FLC", "delta ends early"))? as usize;
    let mut i = 2usize;
    let mut y = 0usize;
    for _ in 0..lines {
        // Opcode words: skip lines (negative, low 14 bits), last pixel (0x80xx), or packet count.
        let mut packets;
        loop {
            let word = le16(data, i).ok_or_else(|| bad("FLC", "delta ends early"))?;
            i += 2;
            match word >> 14 {
                0 => {
                    packets = word as usize;
                    break;
                }
                2 => {
                    // Last pixel of the line (odd widths); the next word is the packet count.
                    if y < st.h && st.w > 0 {
                        st.idx[y * st.w + st.w - 1] = (word & 0xff) as u8;
                    }
                }
                3 => y = y.wrapping_add((word as i16 as isize).unsigned_abs()),
                _ => return Err(bad("FLC", "bad delta opcode")),
            }
        }
        let mut x = 0usize;
        while packets > 0 {
            packets -= 1;
            x += *data.get(i).ok_or_else(|| bad("FLC", "delta ends early"))? as usize;
            let n = *data.get(i + 1).ok_or_else(|| bad("FLC", "delta ends early"))? as i8;
            i += 2;
            if n > 0 {
                let cnt = n as usize * 2;
                let lit = data.get(i..i + cnt).ok_or_else(|| bad("FLC", "delta ends early"))?;
                i += cnt;
                for &v in lit {
                    if y < st.h && x < st.w {
                        st.idx[y * st.w + x] = v;
                    }
                    x += 1;
                }
            } else if n < 0 {
                let pair = data.get(i..i + 2).ok_or_else(|| bad("FLC", "delta ends early"))?;
                i += 2;
                for _ in 0..(-(n as i16)) {
                    for &v in pair {
                        if y < st.h && x < st.w {
                            st.idx[y * st.w + x] = v;
                        }
                        x += 1;
                    }
                }
            }
        }
        y += 1;
    }
    Ok(())
}

/// Every frame of a FLI/FLC as RGBA with its delay in milliseconds.
pub fn fli_frames(bytes: &[u8]) -> Result<Vec<(Image, u32)>, IoError> {
    if bytes.len() < 128 {
        return Err(bad("FLI", "file too short"));
    }
    let magic = le16(bytes, 4).unwrap_or(0);
    let flc = match magic {
        MAGIC_FLI => false,
        MAGIC_FLC => true,
        _ => return Err(bad("FLI", "not a FLI/FLC animation")),
    };
    let frames = le16(bytes, 6).unwrap_or(0) as usize;
    let w = u32::from(le16(bytes, 8).unwrap_or(0));
    let h = u32::from(le16(bytes, 10).unwrap_or(0));
    let depth = le16(bytes, 12).unwrap_or(8);
    if depth != 8 {
        return Err(bad("FLI", format!("{depth}-bit frames are not supported")));
    }
    let speed = le32(bytes, 16).unwrap_or(0);
    let delay_ms = if flc { speed.max(1) } else { (speed.max(1) * 1000) / 70 };
    let probe = Rgba::new(w, h)?;
    drop(probe);
    let mut st = State { w: w as usize, h: h as usize, idx: vec![0; (w as usize) * (h as usize)], pal: [[0; 3]; 256] };
    let mut out = Vec::new();
    let mut at = 128usize;
    let n = frames.min(MAX_FRAMES);
    while out.len() < n && at + 16 <= bytes.len() {
        let size = le32(bytes, at).unwrap_or(0) as usize;
        let ftype = le16(bytes, at + 4).unwrap_or(0);
        if size < 16 {
            return Err(bad("FLI", "frame header too small"));
        }
        let end = at.saturating_add(size).min(bytes.len());
        if ftype == FRAME_MAGIC {
            let chunks = le16(bytes, at + 6).unwrap_or(0) as usize;
            let mut c = at + 16;
            for _ in 0..chunks {
                if c + 6 > end {
                    break;
                }
                let csize = le32(bytes, c).unwrap_or(0) as usize;
                let ctype = le16(bytes, c + 4).unwrap_or(0);
                if csize < 6 {
                    return Err(bad("FLI", "chunk header too small"));
                }
                let data = &bytes[(c + 6).min(end)..(c + csize).min(end)];
                match ctype {
                    4 => colors(&mut st, data, false)?,
                    11 => colors(&mut st, data, true)?,
                    7 => delta_flc(&mut st, data)?,
                    12 => delta_fli(&mut st, data)?,
                    13 => st.idx.fill(0),
                    15 => byte_run(&mut st, data)?,
                    16 => literal(&mut st, data)?,
                    _ => {}
                }
                c += csize;
            }
            out.push((Image::from_u8(w, h, ChannelLayout::Rgba, st.rgba())?, delay_ms));
        }
        // Prefix (0xF100) and unknown frame types are skipped.
        at = end;
    }
    if out.is_empty() {
        return Err(bad("FLI", "no frames"));
    }
    Ok(out)
}

pub fn decode(bytes: &[u8]) -> Result<Rgba, IoError> {
    let frames = fli_frames(bytes)?;
    let n = frames.len();
    let (first, _) = frames.into_iter().next().ok_or_else(|| bad("FLI", "no frames"))?;
    let (w, h) = first.dimensions();
    let mut out = Rgba::new(w, h)?;
    out.data = first.to_rgba8();
    if n > 1 {
        out.warnings.push(format!("animation: {n} frames; the first opened (Import Video Frames to Layers opens them all)"));
    }
    Ok(out)
}

fn chunk(out: &mut Vec<u8>, ctype: u16, data: &[u8]) {
    let mut len = data.len() + 6;
    out.extend((len as u32).to_le_bytes());
    out.extend(ctype.to_le_bytes());
    out.extend_from_slice(data);
    if len & 1 == 1 {
        out.push(0);
        len += 1;
    }
    // Patch the size to the padded length.
    let at = out.len() - len;
    out[at..at + 4].copy_from_slice(&(len as u32).to_le_bytes());
}

fn brun(w: usize, h: usize, idx: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(idx.len() / 2);
    for y in 0..h {
        let row = &idx[y * w..(y + 1) * w];
        out.push(0); // packet count: obsolete
        let mut i = 0;
        while i < w {
            let mut run = 1;
            while i + run < w && row[i + run] == row[i] && run < 127 {
                run += 1;
            }
            if run >= 2 {
                out.push(run as u8);
                out.push(row[i]);
                i += run;
            } else {
                let start = i;
                let mut j = i;
                while j < w && j - start < 127 && !(j + 1 < w && row[j] == row[j + 1]) {
                    j += 1;
                }
                if j == start {
                    j += 1;
                }
                out.push((-((j - start) as i16)) as u8);
                out.extend_from_slice(&row[start..j]);
                i = j;
            }
        }
    }
    out
}

/// A FLC (any size, millisecond speed) or FLI (320 × 200 only, 1/70 s speed) of the document's
/// animation frames, each a BYTE_RUN frame with its own 256-colour palette.
pub fn encode(doc: &Document, w: u32, h: u32, rgba: &[u8], flc: bool, warnings: &mut Vec<String>) -> Result<Vec<u8>, IoError> {
    if w > 65535 || h > 65535 {
        return Err(IoError::Unsupported("FLI/FLC frames are at most 65535 pixels on a side".into()));
    }
    if !flc && (w, h) != (320, 200) {
        warnings.push("FLI players expect 320 × 200; save as .flc for other sizes".into());
    }
    let frames = crate::anim_layers::frames(doc, warnings)?;
    let frames: Vec<(Vec<u8>, u32)> = if frames.len() > 1 { frames.into_iter().map(|f| (f.rgba, f.delay_ms)).collect() } else { vec![(rgba.to_vec(), 100)] };
    let (wu, hu) = (w as usize, h as usize);
    let mut body = Vec::new();
    let mut last_pal: Option<Vec<[u8; 3]>> = None;
    for (px, _) in &frames {
        let (pal, idx) = super::palettize(doc, w, h, px, 256)?;
        let mut fbody = Vec::new();
        let mut chunks = 0u16;
        if last_pal.as_ref() != Some(&pal) {
            let mut c = Vec::with_capacity(4 + pal.len() * 3);
            c.extend(1u16.to_le_bytes());
            c.push(0);
            c.push(if pal.len() == 256 { 0 } else { pal.len() as u8 });
            for p in &pal {
                c.extend(*p);
            }
            chunk(&mut fbody, 4, &c);
            chunks += 1;
            last_pal = Some(pal);
        }
        chunk(&mut fbody, 15, &brun(wu, hu, &idx));
        chunks += 1;
        let mut fh = Vec::with_capacity(16 + fbody.len());
        fh.extend(((16 + fbody.len()) as u32).to_le_bytes());
        fh.extend(FRAME_MAGIC.to_le_bytes());
        fh.extend(chunks.to_le_bytes());
        fh.extend([0u8; 8]);
        fh.extend(fbody);
        body.extend(fh);
    }
    let delay = frames.first().map(|f| f.1).unwrap_or(100).max(1);
    let speed = if flc { delay } else { (delay * 70 / 1000).max(1) };
    let mut out = vec![0u8; 128];
    out[0..4].copy_from_slice(&((128 + body.len()) as u32).to_le_bytes());
    out[4..6].copy_from_slice(&(if flc { MAGIC_FLC } else { MAGIC_FLI }).to_le_bytes());
    out[6..8].copy_from_slice(&(frames.len() as u16).to_le_bytes());
    out[8..10].copy_from_slice(&(w as u16).to_le_bytes());
    out[10..12].copy_from_slice(&(h as u16).to_le_bytes());
    out[12..14].copy_from_slice(&8u16.to_le_bytes());
    out[16..20].copy_from_slice(&speed.to_le_bytes());
    if flc {
        out[80..84].copy_from_slice(&128u32.to_le_bytes());
        let second = if frames.len() > 1 { 128 + le32(&body, 0).unwrap_or(0) } else { 128 };
        out[84..88].copy_from_slice(&second.to_le_bytes());
    }
    out.extend(body);
    if frames.len() == 1 {
        warnings.push("one frame: name layers with (delay) tags to export an animation".into());
    }
    Ok(out)
}
