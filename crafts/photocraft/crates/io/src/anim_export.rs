//! Animation export from layers ([`crate::anim_layers`]): animated GIF (every frame quantised to
//! its own palette), animated WebP (`VP8X` + `ANIM` + one `ANMF` per frame, each frame's bitstream
//! taken from the still WebP encoder) and MNG (`MHDR`, a `FRAM` and a full PNG stream per frame,
//! `MEND`; the MNG-LC profile every MNG decoder reads). Frame delays and disposal come from the
//! layer names.

use redrob_photo_codecs::{ChannelLayout, EncodeOptions, Format, Image};
use redrob_photo_doc::Document;

use crate::anim_layers::{Disposal, Frame, frames};
use crate::{ExportResult, IoError};

/// Writes `doc`'s frames as `ext` (`gif`, `webp` or `mng`); `loop_forever` sets the loop count.
pub fn export_animation(doc: &Document, ext: &str, loop_forever: bool) -> Result<ExportResult, IoError> {
    let mut warnings = Vec::new();
    let fr = frames(doc, &mut warnings)?;
    if fr.len() == 1 {
        warnings.push("one frame: name layers with (delay) tags, one layer per frame, to export an animation".into());
    }
    let bytes = match ext.to_ascii_lowercase().as_str() {
        "gif" => gif(&fr, loop_forever)?,
        "webp" => webp(&fr, loop_forever)?,
        "mng" => mng(&fr, loop_forever)?,
        other => return Err(IoError::Unsupported(format!("animation export does not write .{other}; use gif, webp or mng"))),
    };
    Ok(ExportResult { bytes, warnings })
}

fn size(fr: &[Frame]) -> Result<(u32, u32), IoError> {
    let f = fr.first().ok_or_else(|| IoError::Unsupported("no frames".into()))?;
    Ok((f.w, f.h))
}

fn gif(fr: &[Frame], loop_forever: bool) -> Result<Vec<u8>, IoError> {
    use redrob_photo_algo::quantize::{self, Dither, Forced, PaletteKind};
    let (w, h) = size(fr)?;
    let mut gframes = Vec::with_capacity(fr.len());
    for f in fr {
        let mut px: Vec<[f32; 4]> =
            f.rgba.chunks_exact(4).map(|p| [f32::from(p[0]) / 255.0, f32::from(p[1]) / 255.0, f32::from(p[2]) / 255.0, f32::from(p[3]) / 255.0]).collect();
        let transparent = px.iter().any(|p| p[3] < 0.5);
        let visible: Vec<[f32; 4]> = px.iter().filter(|p| p[3] >= 0.5).map(|p| [p[0], p[1], p[2], 1.0]).collect();
        let src = if visible.is_empty() { &px } else { &visible };
        let mut pal = quantize::build_palette(src, PaletteKind::Adaptive, if transparent { 255 } else { 256 }, Forced::None).map_err(IoError::Unsupported)?;
        let t = transparent.then(|| {
            pal.truncate(255);
            pal.push(pal.first().copied().unwrap_or([0, 0, 0]));
            pal.len() - 1
        });
        let indices = quantize::quantize(&mut px, w as usize, &pal, Dither::None, 1.0, t);
        let delay_cs = (f.delay_ms / 10).clamp(1, 65535) as u16;
        gframes.push(redrob_photo_codecs::web::GifFrame { indices, palette: pal, transparent: t.map(|i| i as u8), delay_cs });
    }
    Ok(redrob_photo_codecs::web::encode_gif_animated(w, h, &gframes, loop_forever)?)
}

fn chunk(out: &mut Vec<u8>, fourcc: &[u8; 4], payload: &[u8]) {
    out.extend_from_slice(fourcc);
    out.extend((payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    if payload.len() & 1 == 1 {
        out.push(0);
    }
}

/// The `ALPH`, `VP8` and `VP8L` chunks of a still WebP, in file order, and whether it has alpha.
fn webp_bitstream(still: &[u8]) -> Result<(Vec<u8>, bool), IoError> {
    if still.len() < 12 || &still[0..4] != b"RIFF" || &still[8..12] != b"WEBP" {
        return Err(IoError::Unsupported("the WebP encoder produced no RIFF container".into()));
    }
    let mut at = 12usize;
    let mut out = Vec::new();
    let mut alpha = false;
    while at + 8 <= still.len() {
        let id = &still[at..at + 4];
        let len = u32::from_le_bytes([still[at + 4], still[at + 5], still[at + 6], still[at + 7]]) as usize;
        let end = (at + 8 + len).min(still.len());
        match id {
            b"ALPH" => {
                alpha = true;
                out.extend_from_slice(&still[at..end + (len & 1).min(still.len() - end)]);
            }
            b"VP8 " => out.extend_from_slice(&still[at..end + (len & 1).min(still.len() - end)]),
            b"VP8L" => {
                // Bit 4 of the first header byte after the signature says whether alpha is used.
                alpha |= still.get(at + 8 + 4).is_some_and(|b| b & 0x10 != 0);
                out.extend_from_slice(&still[at..end + (len & 1).min(still.len() - end)]);
            }
            _ => {}
        }
        at = end + (len & 1);
    }
    if out.is_empty() {
        return Err(IoError::Unsupported("the WebP encoder produced no bitstream chunk".into()));
    }
    Ok((out, alpha))
}

fn webp(fr: &[Frame], loop_forever: bool) -> Result<Vec<u8>, IoError> {
    let (w, h) = size(fr)?;
    if w > 16_777_216 || h > 16_777_216 {
        return Err(IoError::Unsupported("WebP frames are at most 16777216 pixels on a side".into()));
    }
    let opts = EncodeOptions { webp_lossless: true, ..EncodeOptions::default() };
    let mut body = b"WEBP".to_vec();
    let mut any_alpha = false;
    let mut anmfs = Vec::new();
    for f in fr {
        let img = Image::from_u8(f.w, f.h, ChannelLayout::Rgba, f.rgba.clone())?;
        let still = redrob_photo_codecs::encode(&img, Format::WebP, &opts)?;
        let (bits, alpha) = webp_bitstream(&still)?;
        any_alpha |= alpha;
        let mut anmf = Vec::with_capacity(16 + bits.len());
        // Frame X/Y (24-bit, halved), width-1 and height-1 (24-bit), duration (24-bit), flags.
        anmf.extend([0, 0, 0, 0, 0, 0]);
        anmf.extend(&(f.w - 1).to_le_bytes()[..3]);
        anmf.extend(&(f.h - 1).to_le_bytes()[..3]);
        anmf.extend(&f.delay_ms.min(0xFF_FFFF).to_le_bytes()[..3]);
        // Blending: a `replace` frame does not blend with the canvas; disposal: keep.
        anmf.push(if f.disposal == Disposal::Replace { 0b10 } else { 0 });
        anmf.extend(bits);
        anmfs.push(anmf);
    }
    let mut vp8x = vec![0u8; 10];
    vp8x[0] = 0x02 | if any_alpha { 0x10 } else { 0 }; // animation, alpha
    vp8x[4..7].copy_from_slice(&(w - 1).to_le_bytes()[..3]);
    vp8x[7..10].copy_from_slice(&(h - 1).to_le_bytes()[..3]);
    chunk(&mut body, b"VP8X", &vp8x);
    let mut anim = vec![0u8; 6];
    // Background colour (transparent black) then loop count (0 = forever).
    anim[4..6].copy_from_slice(&(if loop_forever { 0u16 } else { 1u16 }).to_le_bytes());
    chunk(&mut body, b"ANIM", &anim);
    for a in &anmfs {
        chunk(&mut body, b"ANMF", a);
    }
    let mut out = b"RIFF".to_vec();
    out.extend((body.len() as u32).to_le_bytes());
    out.extend(body);
    Ok(out)
}

fn png_chunk(out: &mut Vec<u8>, ty: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    let mut crc = Crc::new();
    crc.update(ty);
    crc.update(data);
    out.extend_from_slice(ty);
    out.extend_from_slice(data);
    out.extend(crc.finish().to_be_bytes());
}

/// CRC-32 (IEEE), as PNG chunks use.
struct Crc(u32);
impl Crc {
    fn new() -> Self {
        Crc(0xFFFF_FFFF)
    }
    fn update(&mut self, b: &[u8]) {
        for &x in b {
            let mut c = self.0 ^ u32::from(x);
            for _ in 0..8 {
                c = if c & 1 == 1 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            self.0 = (self.0 >> 8) ^ c;
        }
    }
    fn finish(self) -> u32 {
        !self.0
    }
}

fn mng(fr: &[Frame], loop_forever: bool) -> Result<Vec<u8>, IoError> {
    let (w, h) = size(fr)?;
    let mut out = vec![0x8A, b'M', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let play_time: u32 = fr.iter().map(|f| f.delay_ms).sum();
    let mut mhdr = Vec::with_capacity(28);
    mhdr.extend(w.to_be_bytes());
    mhdr.extend(h.to_be_bytes());
    mhdr.extend(1000u32.to_be_bytes()); // ticks per second
    mhdr.extend((fr.len() as u32).to_be_bytes()); // nominal layer count
    mhdr.extend((fr.len() as u32).to_be_bytes()); // nominal frame count
    mhdr.extend(play_time.to_be_bytes());
    // Simplicity profile: valid, simple MNG features, no JNG, transparency allowed.
    mhdr.extend(0x0000_0009u32.to_be_bytes());
    png_chunk(&mut out, b"MHDR", &mhdr);
    if loop_forever {
        // TERM: repeat from the start after the last frame, forever (iteration max 0x7FFFFFFF).
        let mut term = vec![3u8, 0u8];
        term.extend(0u32.to_be_bytes());
        term.extend(0x7FFF_FFFFu32.to_be_bytes());
        png_chunk(&mut out, b"TERM", &term);
    }
    // One white background so `replace` frames clear the earlier ones.
    png_chunk(&mut out, b"BACK", &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 1]);
    let opts = EncodeOptions::default();
    for f in fr {
        // FRAM: framing mode 3 (frame = one image, clear to background for replace) or 1
        // (combine: images accumulate), with the interframe delay in ticks.
        let mode = if f.disposal == Disposal::Replace { 3u8 } else { 1u8 };
        let mut fram = vec![mode];
        fram.push(0); // empty frame name terminator
        fram.push(2); // change interframe delay: for this and subsequent frames
        fram.push(0);
        fram.push(0);
        fram.push(0);
        fram.extend(f.delay_ms.to_be_bytes());
        png_chunk(&mut out, b"FRAM", &fram);
        let img = Image::from_u8(f.w, f.h, ChannelLayout::Rgba, f.rgba.clone())?;
        let png = redrob_photo_codecs::encode(&img, Format::Png, &opts)?;
        if png.len() <= 8 {
            return Err(IoError::Unsupported("the PNG encoder produced no chunks".into()));
        }
        out.extend_from_slice(&png[8..]);
    }
    png_chunk(&mut out, b"MEND", &[]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_png_reference() {
        let mut c = Crc::new();
        c.update(b"IEND");
        assert_eq!(c.finish(), 0xAE42_6082);
    }
}
