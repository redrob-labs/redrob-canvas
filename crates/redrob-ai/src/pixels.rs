// SPDX-License-Identifier: MIT OR Apache-2.0

//! The crate's own pixel buffers, with the PNG and geometry helpers the IOPaint client needs.
//!
//! IOPaint takes and returns PNGs; every editor in the shell has its own raster type. These two
//! plain structs are the bridge: `width * height * 4` RGBA8 bytes, or `width * height` L8 bytes,
//! row-major, top row first.

use std::io::Cursor;

use base64::Engine as _;
use image::{GrayImage, ImageFormat, RgbImage, RgbaImage};

use crate::{AiError, Result};

/// An RGBA8 raster (straight alpha, row-major, top row first).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rgba8Image {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// An 8-bit single-channel mask (0 = outside, 255 = inside), same layout as [`Rgba8Image`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mask {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// How many pixels to add on each side when outpainting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Expand {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

/// The most pixels an outpaint may add on one side.
pub const MAX_EXPAND: u32 = 4096;

/// A pixel rectangle (`x`, `y` top-left, `width × height`), inclusive-exclusive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// `x + width` (one past the right edge).
    pub fn right(&self) -> u32 {
        self.x + self.width
    }

    /// `y + height` (one past the bottom edge).
    pub fn bottom(&self) -> u32 {
        self.y + self.height
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub fn area(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }

    pub fn contains(&self, x: u32, y: u32) -> bool {
        x >= self.x && y >= self.y && x < self.right() && y < self.bottom()
    }

    /// The overlap of two rectangles (empty when they do not meet).
    pub fn intersect(&self, other: &Rect) -> Rect {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = self.right().min(other.right());
        let y1 = self.bottom().min(other.bottom());
        if x1 <= x0 || y1 <= y0 {
            return Rect::default();
        }
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }

    /// Whether `other` lies entirely inside this rectangle.
    pub fn covers(&self, other: &Rect) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.right() <= self.right()
            && other.bottom() <= self.bottom()
    }
}

impl Expand {
    pub fn total(&self) -> u32 {
        self.left + self.top + self.right + self.bottom
    }

    /// At least one side grows, and no side by more than [`MAX_EXPAND`] pixels.
    pub fn is_valid(&self) -> bool {
        self.total() > 0
            && [self.left, self.top, self.right, self.bottom]
                .iter()
                .all(|&side| side <= MAX_EXPAND)
    }
}

fn checked_len(width: u32, height: u32, channels: usize) -> Result<usize> {
    (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(channels))
        .ok_or_else(|| AiError::Geometry(format!("{width} × {height} does not fit in memory")))
}

impl Rgba8Image {
    /// An image from RGBA bytes; the length must be `width * height * 4`.
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Result<Self> {
        if data.len() != checked_len(width, height, 4)? {
            return Err(AiError::Geometry(format!(
                "RGBA buffer of {} bytes is not {width} × {height} × 4",
                data.len()
            )));
        }
        Ok(Self {
            width,
            height,
            data,
        })
    }

    /// A `width × height` image filled with one colour.
    pub fn filled(width: u32, height: u32, rgba: [u8; 4]) -> Result<Self> {
        let len = checked_len(width, height, 4)?;
        let mut data = Vec::with_capacity(len);
        for _ in 0..len / 4 {
            data.extend_from_slice(&rgba);
        }
        Ok(Self {
            width,
            height,
            data,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// A fully transparent `width × height` image.
    pub fn transparent(width: u32, height: u32) -> Result<Self> {
        Self::filled(width, height, [0, 0, 0, 0])
    }

    /// An opaque image from `width * height * 3` RGB bytes (alpha 255 everywhere).
    pub fn from_rgb(width: u32, height: u32, rgb: &[u8]) -> Result<Self> {
        if rgb.len() != checked_len(width, height, 3)? {
            return Err(AiError::Geometry(format!(
                "RGB buffer of {} bytes is not {width} × {height} × 3",
                rgb.len()
            )));
        }
        let mut data = Vec::with_capacity(rgb.len() / 3 * 4);
        for px in rgb.chunks_exact(3) {
            data.extend_from_slice(&[px[0], px[1], px[2], 255]);
        }
        Ok(Self {
            width,
            height,
            data,
        })
    }

    pub fn rect(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    /// The pixels inside `rect`, which must lie inside the image.
    pub fn crop(&self, rect: Rect) -> Result<Self> {
        if !self.rect().covers(&rect) {
            return Err(AiError::Geometry(format!(
                "crop {rect:?} leaves the {} × {} image",
                self.width, self.height
            )));
        }
        let row = rect.width as usize * 4;
        let mut data = Vec::with_capacity(row * rect.height as usize);
        for y in rect.y..rect.bottom() {
            let i = (y as usize * self.width as usize + rect.x as usize) * 4;
            data.extend_from_slice(&self.data[i..i + row]);
        }
        Ok(Self {
            width: rect.width,
            height: rect.height,
            data,
        })
    }

    /// Copies `src` into this image with its top-left at (`x`, `y`); the source must fit.
    pub fn blit(&mut self, src: &Rgba8Image, x: u32, y: u32) -> Result<()> {
        let dst = Rect::new(x, y, src.width, src.height);
        if !self.rect().covers(&dst) {
            return Err(AiError::Geometry(format!(
                "blit of {} × {} at ({x}, {y}) leaves the {} × {} image",
                src.width, src.height, self.width, self.height
            )));
        }
        let row = src.width as usize * 4;
        for sy in 0..src.height as usize {
            let s = sy * row;
            let d = ((y as usize + sy) * self.width as usize + x as usize) * 4;
            self.data[d..d + row].copy_from_slice(&src.data[s..s + row]);
        }
        Ok(())
    }

    /// Blends `src` over this image at (`x`, `y`) with `weight(sx, sy)` in 0..=1 per source
    /// pixel: `weight` 1 takes the source, 0 keeps what is there. Transparent source pixels are
    /// skipped; where this image is still transparent the source is taken whole, so a feathered
    /// tile never fades into nothing.
    pub fn blend_weighted(
        &mut self,
        src: &Rgba8Image,
        x: u32,
        y: u32,
        weight: impl Fn(u32, u32) -> f32,
    ) -> Result<()> {
        let dst = Rect::new(x, y, src.width, src.height);
        if !self.rect().covers(&dst) {
            return Err(AiError::Geometry(format!(
                "blend of {} × {} at ({x}, {y}) leaves the {} × {} image",
                src.width, src.height, self.width, self.height
            )));
        }
        for sy in 0..src.height {
            for sx in 0..src.width {
                let s = (sy as usize * src.width as usize + sx as usize) * 4;
                let d = ((y + sy) as usize * self.width as usize + (x + sx) as usize) * 4;
                let src_px = &src.data[s..s + 4];
                if src_px[3] == 0 {
                    continue;
                }
                let dst_px = &mut self.data[d..d + 4];
                let w = if dst_px[3] == 0 {
                    1.0
                } else {
                    weight(sx, sy).clamp(0.0, 1.0)
                };
                for (dst, &new) in dst_px.iter_mut().zip(src_px) {
                    let v = f32::from(*dst) * (1.0 - w) + f32::from(new) * w;
                    *dst = v.round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        Ok(())
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        match self.data.get(i..i + 4) {
            Some(px) => [px[0], px[1], px[2], px[3]],
            None => [0; 4],
        }
    }

    /// Decodes a PNG (any colour type) to RGBA8.
    pub fn from_png(bytes: &[u8]) -> Result<Self> {
        let decoded = image::load_from_memory_with_format(bytes, ImageFormat::Png)
            .map_err(|e| AiError::Png(e.to_string()))?
            .into_rgba8();
        Ok(Self {
            width: decoded.width(),
            height: decoded.height(),
            data: decoded.into_raw(),
        })
    }

    /// Encodes as an RGBA PNG.
    pub fn to_png(&self) -> Result<Vec<u8>> {
        let img = RgbaImage::from_raw(self.width, self.height, self.data.clone())
            .ok_or_else(|| AiError::Png("RGBA buffer does not match its size".into()))?;
        encode_png(|out| img.write_to(out, ImageFormat::Png))
    }

    /// Transparency composited over white, encoded as an opaque RGB PNG. IOPaint hands the
    /// source's alpha back unchanged, so an erased stroke on a transparent canvas came back as a
    /// dark silhouette; sending opaque pixels avoids that, and the result's alpha is the mask.
    pub fn to_opaque_png(&self) -> Result<Vec<u8>> {
        let flat = self.flatten_over_white();
        let img = RgbImage::from_raw(self.width, self.height, flat)
            .ok_or_else(|| AiError::Png("RGB buffer does not match its size".into()))?;
        encode_png(|out| img.write_to(out, ImageFormat::Png))
    }

    /// RGB8 bytes of this image composited over white.
    pub fn flatten_over_white(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.width as usize * self.height as usize * 3);
        for px in self.data.chunks_exact(4) {
            let a = u32::from(px[3]);
            for &c in &px[..3] {
                // c * a + 255 * (255 - a), rounded, over 255.
                let v = (u32::from(c) * a + 255 * (255 - a) + 127) / 255;
                out.push(v.min(255) as u8);
            }
        }
        out
    }

    /// `data:image/png;base64,...` of the RGBA PNG.
    pub fn to_data_url(&self) -> Result<String> {
        Ok(data_url(&self.to_png()?))
    }

    /// `data:image/png;base64,...` of the opaque RGB PNG.
    pub fn to_opaque_data_url(&self) -> Result<String> {
        Ok(data_url(&self.to_opaque_png()?))
    }

    /// Multiplies this image's alpha by `mask` (255 keeps, 0 clears) so only the masked area is
    /// visible: a result layer that can be hidden or deleted without touching the original.
    pub fn clip_alpha(&mut self, mask: &Mask) -> Result<()> {
        if mask.width != self.width || mask.height != self.height {
            return Err(AiError::Geometry(format!(
                "mask {} × {} does not match image {} × {}",
                mask.width, mask.height, self.width, self.height
            )));
        }
        for (px, &m) in self.data.chunks_exact_mut(4).zip(&mask.data) {
            px[3] = ((u32::from(px[3]) * u32::from(m) + 127) / 255) as u8;
        }
        Ok(())
    }

    /// `(result.width / width, result.height / height)` when `result` is a uniform upscale of a
    /// `width × height` canvas (RealESRGAN), else `None`.
    pub fn uniform_scale_from(&self, width: u32, height: u32) -> Option<f64> {
        if width == 0 || height == 0 || self.width < width {
            return None;
        }
        let sx = f64::from(self.width) / f64::from(width);
        let sy = f64::from(self.height) / f64::from(height);
        ((sx - sy).abs() < 0.02).then_some(sx)
    }
}

impl Mask {
    /// A mask from L8 bytes; the length must be `width * height`.
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Result<Self> {
        if data.len() != checked_len(width, height, 1)? {
            return Err(AiError::Geometry(format!(
                "mask buffer of {} bytes is not {width} × {height}",
                data.len()
            )));
        }
        Ok(Self {
            width,
            height,
            data,
        })
    }

    pub fn filled(width: u32, height: u32, value: u8) -> Result<Self> {
        let len = checked_len(width, height, 1)?;
        Ok(Self {
            width,
            height,
            data: vec![value; len],
        })
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub fn get(&self, x: u32, y: u32) -> u8 {
        self.data
            .get(y as usize * self.width as usize + x as usize)
            .copied()
            .unwrap_or(0)
    }

    /// Number of pixels inside (non-zero).
    pub fn coverage(&self) -> usize {
        self.data.iter().filter(|&&v| v != 0).count()
    }

    pub fn rect(&self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }

    /// The tightest rectangle around the non-zero pixels, or `None` when nothing is inside.
    pub fn bounding_box(&self) -> Option<Rect> {
        let w = self.width as usize;
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
        for (y, row) in self.data.chunks_exact(w.max(1)).enumerate() {
            let Some(first) = row.iter().position(|&v| v != 0) else {
                continue;
            };
            let last = row.iter().rposition(|&v| v != 0).unwrap_or(first);
            x0 = x0.min(first);
            x1 = x1.max(last + 1);
            y0 = y0.min(y);
            y1 = y1.max(y + 1);
        }
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        Some(Rect::new(
            x0 as u32,
            y0 as u32,
            (x1 - x0) as u32,
            (y1 - y0) as u32,
        ))
    }

    /// The mask inside `rect`, which must lie inside the mask.
    pub fn crop(&self, rect: Rect) -> Result<Self> {
        if !self.rect().covers(&rect) {
            return Err(AiError::Geometry(format!(
                "crop {rect:?} leaves the {} × {} mask",
                self.width, self.height
            )));
        }
        let row = rect.width as usize;
        let mut data = Vec::with_capacity(row * rect.height as usize);
        for y in rect.y..rect.bottom() {
            let i = y as usize * self.width as usize + rect.x as usize;
            data.extend_from_slice(&self.data[i..i + row]);
        }
        Ok(Self {
            width: rect.width,
            height: rect.height,
            data,
        })
    }

    /// Sets every pixel inside `rect` to `value`.
    pub fn fill_rect(&mut self, rect: Rect, value: u8) {
        let rect = self.rect().intersect(&rect);
        for y in rect.y..rect.bottom() {
            let i = y as usize * self.width as usize + rect.x as usize;
            self.data[i..i + rect.width as usize].fill(value);
        }
    }

    /// Decodes a PNG to L8 (colour PNGs are converted to luma).
    pub fn from_png(bytes: &[u8]) -> Result<Self> {
        let decoded = image::load_from_memory_with_format(bytes, ImageFormat::Png)
            .map_err(|e| AiError::Png(e.to_string()))?
            .into_luma8();
        Ok(Self {
            width: decoded.width(),
            height: decoded.height(),
            data: decoded.into_raw(),
        })
    }

    /// IOPaint's gen-mask plugins return the frontend's mask: selected pixels painted in a
    /// translucent colour (alpha > 0). This reads that PNG and thresholds its alpha.
    pub fn from_alpha_png(bytes: &[u8]) -> Result<Self> {
        let rgba = Rgba8Image::from_png(bytes)?;
        let data = rgba
            .data
            .chunks_exact(4)
            .map(|px| if px[3] > 0 { 255 } else { 0 })
            .collect();
        Ok(Self {
            width: rgba.width,
            height: rgba.height,
            data,
        })
    }

    /// Encodes as a grayscale PNG.
    pub fn to_png(&self) -> Result<Vec<u8>> {
        let img = GrayImage::from_raw(self.width, self.height, self.data.clone())
            .ok_or_else(|| AiError::Png("mask buffer does not match its size".into()))?;
        encode_png(|out| img.write_to(out, ImageFormat::Png))
    }

    pub fn to_data_url(&self) -> Result<String> {
        Ok(data_url(&self.to_png()?))
    }
}

fn encode_png(
    write: impl FnOnce(&mut Cursor<&mut Vec<u8>>) -> image::ImageResult<()>,
) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    write(&mut Cursor::new(&mut bytes)).map_err(|e| AiError::Png(e.to_string()))?;
    Ok(bytes)
}

/// `data:image/png;base64,...` over raw PNG bytes.
pub fn data_url(png: &[u8]) -> String {
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    )
}

/// The PNG bytes inside a `data:image/png;base64,...` URL.
pub fn png_from_data_url(url: &str) -> Result<Vec<u8>> {
    let encoded = url
        .strip_prefix("data:image/png;base64,")
        .ok_or_else(|| AiError::Png("not a PNG data URL".into()))?;
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|e| AiError::Png(e.to_string()))
}

/// The canvas and mask an outpaint sends: `image` flattened over white sits inside a black
/// canvas grown by `expand`; the mask is 255 on the new border (fill this) and 0 over the picture.
/// Both are opaque, so the border does not come back transparent with the source's alpha.
pub fn pad_for_outpaint(image: &Rgba8Image, expand: Expand) -> Result<(Rgba8Image, Mask)> {
    if !expand.is_valid() {
        return Err(AiError::Geometry(format!(
            "give 1 to {MAX_EXPAND} pixels on at least one side"
        )));
    }
    let width = image
        .width
        .checked_add(expand.left)
        .and_then(|w| w.checked_add(expand.right))
        .ok_or_else(|| AiError::Geometry("expanded width overflows".into()))?;
    let height = image
        .height
        .checked_add(expand.top)
        .and_then(|h| h.checked_add(expand.bottom))
        .ok_or_else(|| AiError::Geometry("expanded height overflows".into()))?;
    let mut padded = Rgba8Image::filled(width, height, [0, 0, 0, 255])?;
    let mut mask = Mask::filled(width, height, 255)?;
    let flat = image.flatten_over_white();
    let src_w = image.width as usize;
    for y in 0..image.height as usize {
        let dst_y = y + expand.top as usize;
        let dst_x0 = expand.left as usize;
        let row = &flat[y * src_w * 3..(y + 1) * src_w * 3];
        for (x, px) in row.chunks_exact(3).enumerate() {
            let i = (dst_y * width as usize + dst_x0 + x) * 4;
            padded.data[i..i + 3].copy_from_slice(px);
            padded.data[i + 3] = 255;
        }
        let m0 = dst_y * width as usize + dst_x0;
        mask.data[m0..m0 + src_w].fill(0);
    }
    Ok((padded, mask))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn checker(w: u32, h: u32) -> Rgba8Image {
        let mut data = Vec::new();
        for y in 0..h {
            for x in 0..w {
                if (x + y) % 2 == 0 {
                    data.extend_from_slice(&[255, 0, 0, 255]);
                } else {
                    data.extend_from_slice(&[0, 0, 255, 128]);
                }
            }
        }
        Rgba8Image::new(w, h, data).unwrap()
    }

    #[test]
    fn png_round_trip_keeps_rgba_bytes() {
        let img = checker(5, 3);
        let back = Rgba8Image::from_png(&img.to_png().unwrap()).unwrap();
        assert_eq!(back, img);
    }

    #[test]
    fn mask_png_round_trip_keeps_l8_bytes() {
        let mask = Mask::new(3, 2, vec![0, 255, 7, 9, 0, 255]).unwrap();
        let back = Mask::from_png(&mask.to_png().unwrap()).unwrap();
        assert_eq!(back, mask);
    }

    #[test]
    fn data_url_round_trip() {
        let img = checker(2, 2);
        let url = img.to_data_url().unwrap();
        assert!(url.starts_with("data:image/png;base64,"));
        let png = png_from_data_url(&url).unwrap();
        assert_eq!(Rgba8Image::from_png(&png).unwrap(), img);
        assert!(png_from_data_url("data:text/plain,hi").is_err());
    }

    #[test]
    fn opaque_flattens_alpha_over_white() {
        let img = Rgba8Image::new(2, 1, vec![0, 0, 0, 0, 0, 0, 0, 255]).unwrap();
        assert_eq!(img.flatten_over_white(), vec![255, 255, 255, 0, 0, 0]);
        let half = Rgba8Image::new(1, 1, vec![0, 0, 0, 128]).unwrap();
        assert_eq!(half.flatten_over_white(), vec![127, 127, 127]);
        // The opaque PNG decodes with alpha 255 everywhere.
        let back = Rgba8Image::from_png(&img.to_opaque_png().unwrap()).unwrap();
        assert_eq!(back.data, vec![255, 255, 255, 255, 0, 0, 0, 255]);
    }

    #[test]
    fn outpaint_padding_geometry_on_4x4() {
        let img = Rgba8Image::filled(4, 4, [10, 20, 30, 255]).unwrap();
        let expand = Expand {
            left: 1,
            top: 2,
            right: 3,
            bottom: 0,
        };
        let (padded, mask) = pad_for_outpaint(&img, expand).unwrap();
        assert_eq!((padded.width, padded.height), (8, 6));
        assert_eq!((mask.width, mask.height), (8, 6));
        // The picture sits at x 1..5, y 2..6: mask 0 there, 255 elsewhere.
        assert_eq!(mask.coverage(), 8 * 6 - 16);
        for y in 0..6 {
            for x in 0..8 {
                let inside = (1..5).contains(&x) && (2..6).contains(&y);
                assert_eq!(mask.get(x, y), if inside { 0 } else { 255 }, "({x}, {y})");
                let expected = if inside {
                    [10, 20, 30, 255]
                } else {
                    [0, 0, 0, 255]
                };
                assert_eq!(padded.pixel(x, y), expected, "({x}, {y})");
            }
        }
    }

    #[test]
    fn outpaint_rejects_zero_and_oversized_expand() {
        let img = checker(4, 4);
        assert!(pad_for_outpaint(&img, Expand::default()).is_err());
        let huge = Expand {
            left: MAX_EXPAND + 1,
            ..Expand::default()
        };
        assert!(pad_for_outpaint(&img, huge).is_err());
    }

    #[test]
    fn clip_alpha_multiplies_by_mask() {
        let mut img = Rgba8Image::filled(2, 1, [1, 2, 3, 255]).unwrap();
        let mask = Mask::new(2, 1, vec![0, 255]).unwrap();
        img.clip_alpha(&mask).unwrap();
        assert_eq!(img.data, vec![1, 2, 3, 0, 1, 2, 3, 255]);
        let wrong = Mask::filled(3, 1, 255).unwrap();
        assert!(img.clip_alpha(&wrong).is_err());
    }

    #[test]
    fn alpha_png_becomes_binary_mask() {
        let rgba = Rgba8Image::new(2, 1, vec![9, 9, 9, 0, 9, 9, 9, 40]).unwrap();
        let mask = Mask::from_alpha_png(&rgba.to_png().unwrap()).unwrap();
        assert_eq!(mask.data, vec![0, 255]);
    }

    #[test]
    fn uniform_scale_detects_upscales_only() {
        let up = Rgba8Image::filled(8, 6, [0; 4]).unwrap();
        assert_eq!(up.uniform_scale_from(4, 3), Some(2.0));
        assert_eq!(up.uniform_scale_from(4, 4), None);
        assert_eq!(up.uniform_scale_from(16, 12), None);
    }

    #[test]
    fn constructors_check_lengths() {
        assert!(Rgba8Image::new(2, 2, vec![0; 15]).is_err());
        assert!(Mask::new(2, 2, vec![0; 3]).is_err());
    }
}
