// SPDX-License-Identifier: MIT OR Apache-2.0

//! Which pixels a request carries: the region of interest an inpaint sends instead of the whole
//! canvas, and the tiles an outpaint walks instead of one canvas-sized request.
//!
//! The GPU measurements behind this (`docs/gap/results/gpu-ai.toml`): a 4K erase computed in
//! 0.7 s but took 7.7 s end to end because the whole picture crossed the wire twice, and a 4K
//! outpaint ran the model over the full 8640 × 5760 canvas in one pass and asked for 36 GiB.
//! Pure geometry, no I/O, so every rule here has a table test.

use crate::pixels::{Mask, Rect};

/// Region edges snap outward to a multiple of this (the models work in 8 / 64 px blocks).
pub const ROI_ALIGN: u32 = 64;

/// Context around the mask an erase model sees (IOPaint's own crop margin).
pub const ROI_MARGIN: u32 = 128;

/// Context around the mask a prompted (diffusion) fill sees: more, so the generated content
/// matches its surroundings.
pub const ROI_MARGIN_PROMPTED: u32 = 256;

/// When the region would cover this share of the image or more, the whole image is sent.
pub const ROI_FULL_IMAGE_SHARE: f64 = 0.9;

/// The region an inpaint sends: the mask's bounding box grown by `margin` on every side,
/// snapped outward to [`ROI_ALIGN`] and clamped to the image. `None` when the mask is empty;
/// the full image when the region would be most of it anyway.
pub fn inpaint_region(mask: &Mask, margin: u32) -> Option<Rect> {
    let bbox = mask.bounding_box()?;
    let full = mask.rect();
    let x0 = bbox.x.saturating_sub(margin) / ROI_ALIGN * ROI_ALIGN;
    let y0 = bbox.y.saturating_sub(margin) / ROI_ALIGN * ROI_ALIGN;
    let x1 = (bbox.right().saturating_add(margin)).div_ceil(ROI_ALIGN) * ROI_ALIGN;
    let y1 = (bbox.bottom().saturating_add(margin)).div_ceil(ROI_ALIGN) * ROI_ALIGN;
    let x1 = x1.min(full.width);
    let y1 = y1.min(full.height);
    let region = Rect::new(x0, y0, x1 - x0, y1 - y0);
    if region.area() as f64 >= full.area() as f64 * ROI_FULL_IMAGE_SHARE {
        return Some(full);
    }
    Some(region)
}

/// The longest side of an outpaint tile.
pub const TILE: u32 = 1024;

/// How far neighbouring tiles along a strip overlap; the seam is feathered across this band.
pub const TILE_OVERLAP: u32 = 64;

/// The least already-known pixels a tile includes next to the pixels it paints.
pub const TILE_CONTEXT: u32 = 128;

/// One outpaint request. `rect` is what is sent (known context plus the pixels to paint);
/// `paint` is the part of it to fill, which includes the `feather_*` band already painted by
/// the previous tile along the strip, so the two results are blended across it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tile {
    pub rect: Rect,
    pub paint: Rect,
    /// Rows at the top of `paint` shared with the tile above (0 for the first tile).
    pub feather_top: u32,
    /// Columns at the left of `paint` shared with the tile to the left (0 for the first tile).
    pub feather_left: u32,
}

impl Tile {
    /// The blend weight of the new result at (`x`, `y`) inside `paint`: a ramp from 0 to 1
    /// across the feather band(s), 1 elsewhere.
    pub fn weight(&self, x: u32, y: u32) -> f32 {
        let ramp = |v: u32, band: u32| {
            if band == 0 || v >= band {
                1.0
            } else {
                (v as f32 + 0.5) / band as f32
            }
        };
        ramp(x, self.feather_left).min(ramp(y, self.feather_top))
    }
}

/// Where tiles walk along a strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Axis {
    /// The strip runs vertically (left / right of the picture): tiles step along `y`.
    Vertical,
    /// The strip runs horizontally (above / below the picture): tiles step along `x`.
    Horizontal,
}

/// The tiles that fill a canvas of `width × height` around `picture`, in the order to run
/// them: the left and right strips first (full picture height, columns outward from the
/// picture), then the top and bottom strips over the full width, whose tiles see the filled
/// sides as context. Every tile is at most [`TILE`] on a side and includes at least
/// [`TILE_CONTEXT`] known pixels next to what it paints (less only when the canvas has less).
/// Empty when `picture` is the whole canvas.
pub fn outpaint_tiles(picture: Rect, width: u32, height: u32) -> Vec<Tile> {
    let mut tiles = Vec::new();
    if picture.is_empty() || width < picture.right() || height < picture.bottom() {
        return tiles;
    }
    let strip_y = picture.y..picture.bottom();
    // Left: columns from picture.x down to 0.
    let mut known = picture.x;
    while known > 0 {
        let c = known.min(TILE - TILE_CONTEXT);
        let ctx = (TILE - c).min(width - known);
        let x = known - c;
        push_strip(
            &mut tiles,
            Axis::Vertical,
            Rect::new(x, 0, c + ctx, 0),
            Rect::new(x, 0, c, 0),
            strip_y.clone(),
        );
        known -= c;
    }
    // Right: columns from picture.right() up to width.
    let mut known = picture.right();
    while known < width {
        let c = (width - known).min(TILE - TILE_CONTEXT);
        let ctx = (TILE - c).min(known);
        push_strip(
            &mut tiles,
            Axis::Vertical,
            Rect::new(known - ctx, 0, ctx + c, 0),
            Rect::new(known, 0, c, 0),
            strip_y.clone(),
        );
        known += c;
    }
    let strip_x = 0..width;
    // Top: rows from picture.y up to 0, over the full width.
    let mut known = picture.y;
    while known > 0 {
        let c = known.min(TILE - TILE_CONTEXT);
        let ctx = (TILE - c).min(height - known);
        let y = known - c;
        push_strip(
            &mut tiles,
            Axis::Horizontal,
            Rect::new(0, y, 0, c + ctx),
            Rect::new(0, y, 0, c),
            strip_x.clone(),
        );
        known -= c;
    }
    // Bottom: rows from picture.bottom() down to height.
    let mut known = picture.bottom();
    while known < height {
        let c = (height - known).min(TILE - TILE_CONTEXT);
        let ctx = (TILE - c).min(known);
        push_strip(
            &mut tiles,
            Axis::Horizontal,
            Rect::new(0, known - ctx, 0, ctx + c),
            Rect::new(0, known, 0, c),
            strip_x.clone(),
        );
        known += c;
    }
    tiles
}

/// Steps tiles along `along` (the strip's long axis), `TILE` long with `TILE_OVERLAP` shared
/// between neighbours; the last tile is pulled back so it ends on the strip's end. `rect` and
/// `paint` carry the across-axis extent, the along-axis extent is filled in here.
fn push_strip(
    tiles: &mut Vec<Tile>,
    axis: Axis,
    rect: Rect,
    paint: Rect,
    along: std::ops::Range<u32>,
) {
    let start = along.start;
    let length = along.end.saturating_sub(along.start);
    if length == 0 {
        return;
    }
    let size = length.min(TILE);
    let step = size.saturating_sub(TILE_OVERLAP).max(1);
    let mut pos = 0;
    let mut previous_end: Option<u32> = None;
    loop {
        let offset = pos.min(length - size);
        let begin = start + offset;
        let band = previous_end.map_or(0, |end| end.saturating_sub(begin));
        let (tile_rect, tile_paint, feather_top, feather_left) = match axis {
            Axis::Vertical => (
                Rect::new(rect.x, begin, rect.width, size),
                Rect::new(paint.x, begin, paint.width, size),
                band,
                0,
            ),
            Axis::Horizontal => (
                Rect::new(begin, rect.y, size, rect.height),
                Rect::new(begin, paint.y, size, paint.height),
                0,
                band,
            ),
        };
        tiles.push(Tile {
            rect: tile_rect,
            paint: tile_paint,
            feather_top,
            feather_left,
        });
        previous_end = Some(begin + size);
        if offset + size >= length {
            break;
        }
        pos += step;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn mask_with_box(w: u32, h: u32, b: Rect) -> Mask {
        let mut mask = Mask::filled(w, h, 0).unwrap();
        mask.fill_rect(b, 255);
        mask
    }

    #[test]
    fn inpaint_region_grows_aligns_and_clamps() {
        // 5760 × 3840 picture, a 1287 × 858 mask in the middle (the 4K GPU case).
        let mask = mask_with_box(5760, 3840, Rect::new(2236, 1491, 1287, 858));
        let region = inpaint_region(&mask, ROI_MARGIN).unwrap();
        assert_eq!(region.x % ROI_ALIGN, 0);
        assert_eq!(region.y % ROI_ALIGN, 0);
        assert_eq!(region.right() % ROI_ALIGN, 0);
        assert_eq!(region.bottom() % ROI_ALIGN, 0);
        assert!(region.x <= 2236 - ROI_MARGIN && region.right() >= 2236 + 1287 + ROI_MARGIN);
        assert!(region.y <= 1491 - ROI_MARGIN && region.bottom() >= 1491 + 858 + ROI_MARGIN);
        // Far less than the picture went out.
        assert!(region.area() < mask.rect().area() / 5);
        // A mask touching the corner clamps at 0 and at the picture edge.
        let corner = mask_with_box(5760, 3840, Rect::new(0, 0, 10, 10));
        let region = inpaint_region(&corner, ROI_MARGIN_PROMPTED).unwrap();
        assert_eq!((region.x, region.y), (0, 0));
        assert_eq!(region.right(), (10 + ROI_MARGIN_PROMPTED).div_ceil(64) * 64);
        // A small picture is sent whole; an empty mask has no region.
        let small = mask_with_box(4, 4, Rect::new(1, 1, 2, 1));
        assert_eq!(
            inpaint_region(&small, ROI_MARGIN),
            Some(Rect::new(0, 0, 4, 4))
        );
        assert_eq!(inpaint_region(&Mask::filled(8, 8, 0).unwrap(), 128), None);
    }

    fn check_cover(picture: Rect, w: u32, h: u32) -> Vec<Tile> {
        let tiles = outpaint_tiles(picture, w, h);
        let mut painted = Mask::filled(w, h, 0).unwrap();
        for t in &tiles {
            assert!(t.rect.width <= TILE && t.rect.height <= TILE, "{t:?}");
            assert!(
                Rect::new(0, 0, w, h).covers(&t.rect),
                "{t:?} leaves the canvas"
            );
            assert!(t.rect.covers(&t.paint), "{t:?} paints outside itself");
            assert!(
                picture.intersect(&t.paint).is_empty(),
                "{t:?} paints the picture"
            );
            // Known context on at least one side of what is painted.
            let known_w = t.rect.width - t.paint.width;
            let known_h = t.rect.height - t.paint.height;
            assert!(
                known_w >= TILE_CONTEXT.min(w - t.paint.width)
                    || known_h >= TILE_CONTEXT.min(h - t.paint.height),
                "{t:?} has no context"
            );
            painted.fill_rect(t.paint, 255);
        }
        let border = w as usize * h as usize - picture.area() as usize;
        assert_eq!(
            painted.coverage(),
            border,
            "the border is not fully painted"
        );
        tiles
    }

    #[test]
    fn outpaint_tiles_cover_the_4k_and_8k_borders_in_small_tiles() {
        // portrait-4k 5760 × 3840 grown 25 % each side (the case that asked for 36 GiB).
        let tiles = check_cover(Rect::new(1440, 960, 5760, 3840), 8640, 5760);
        assert!(tiles.len() > 8);
        // landscape-8k 8900 × 6675 grown 25 %.
        let tiles = check_cover(Rect::new(2225, 1669, 8900, 6675), 13350, 10013);
        assert!(
            tiles
                .iter()
                .all(|t| t.rect.area() <= u64::from(TILE) * u64::from(TILE))
        );
        // An asymmetric expansion (right only) paints only the right strip.
        let tiles = check_cover(Rect::new(0, 0, 3000, 2000), 3300, 2000);
        assert!(tiles.iter().all(|t| t.paint.x >= 3000));
        // The first tile of a strip has no feather band, the next ones overlap by TILE_OVERLAP.
        assert_eq!(tiles[0].feather_top, 0);
        assert_eq!(tiles[1].feather_top, TILE_OVERLAP);
        assert_eq!(tiles[1].paint.y, tiles[0].paint.bottom() - TILE_OVERLAP);
        // The last tile ends exactly on the strip's end.
        assert_eq!(tiles.last().unwrap().paint.bottom(), 2000);
    }

    #[test]
    fn a_thick_border_is_walked_in_columns_that_use_the_previous_as_context() {
        // 2000 px to the left of a 1000 × 500 picture: 896, then 896, then 208.
        let tiles = check_cover(Rect::new(2000, 0, 1000, 500), 3000, 500);
        let xs: Vec<u32> = tiles.iter().map(|t| t.paint.x).collect();
        assert_eq!(xs, vec![1104, 208, 0]);
        assert_eq!(tiles[0].paint.width, 896);
        assert_eq!(tiles[0].rect.right(), 2128); // 128 px of the picture as context
        assert_eq!(tiles[2].paint.width, 208);
        assert_eq!(tiles[2].rect.width, 1024); // small paint, lots of context
    }

    #[test]
    fn feather_weights_ramp_across_the_band_only() {
        let tile = Tile {
            rect: Rect::new(0, 0, 100, 100),
            paint: Rect::new(0, 0, 100, 100),
            feather_top: 64,
            feather_left: 0,
        };
        assert!(tile.weight(50, 0) < 0.02);
        assert!((tile.weight(50, 31) - 0.4921875).abs() < 1e-6);
        assert_eq!(tile.weight(50, 64), 1.0);
        assert_eq!(tile.weight(0, 99), 1.0);
        let none = Tile {
            feather_top: 0,
            ..tile
        };
        assert_eq!(none.weight(0, 0), 1.0);
    }

    #[test]
    fn no_tiles_when_nothing_grows() {
        assert!(outpaint_tiles(Rect::new(0, 0, 10, 10), 10, 10).is_empty());
        assert!(outpaint_tiles(Rect::new(0, 0, 10, 10), 5, 5).is_empty());
    }
}
