//! Fill-tool region refinements on [`Region`]s from `selection`: anti-aliasing a hard region,
//! growing it until the darkest or most opaque pixels, feathering its edge, the regions a drag
//! passes over, and the regions an enclosing shape surrounds (Enclose and Fill).

use redrob_photo_geom::Rect;

use crate::selection::{Region, WandOptions, wand_region_with};

/// Coverage (0..255) of `region` over a `w × h` window at `origin`.
fn window(region: &Region, origin: (i32, i32), w: usize, h: usize) -> Vec<u8> {
    let mut cov = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            cov[y * w + x] = (region.at(origin.0 + x as i32, origin.1 + y as i32) * 255.0).round() as u8;
        }
    }
    cov
}

fn region_from(cov: &[u8], w: usize, h: usize, origin: (i32, i32)) -> Option<Region> {
    let (mut bx0, mut by0, mut bx1, mut by1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    for (i, v) in cov.iter().enumerate() {
        if *v == 0 {
            continue;
        }
        let (x, y) = (i % w, i / w);
        (bx0, bx1, by0, by1) = (bx0.min(x), bx1.max(x), by0.min(y), by1.max(y));
    }
    if bx0 == usize::MAX {
        return None;
    }
    let bw = bx1 + 1 - bx0;
    let mut mask = Vec::with_capacity(bw * (by1 + 1 - by0));
    for y in by0..=by1 {
        mask.extend_from_slice(&cov[y * w + bx0..y * w + bx1 + 1]);
    }
    Some(Region { bbox: Rect::new(origin.0 + bx0 as i32, origin.1 + by0 as i32, origin.0 + bx1 as i32 + 1, origin.1 + by1 as i32 + 1), mask })
}

/// Luma of an 8-bit pixel weighted by its alpha, 0..255 (transparent reads bright: it is not
/// a dark line).
fn darkness(p: [u8; 4]) -> u8 {
    let luma = (u32::from(p[0]) * 299 + u32::from(p[1]) * 587 + u32::from(p[2]) * 114) / 1000;
    let a = u32::from(p[3]);
    (255 - (255 - luma) * a / 255) as u8
}

impl Region {
    /// This hard-edged region anti-aliased: edge pixels blend with their 3×3 neighbourhood by
    /// the same rule the wand uses, so a boundary or close-gap fill meets its boundary softly.
    /// The box grows by one pixel (clipped to `area`) for the fringe.
    pub fn antialiased(&self, area: Rect) -> Region {
        let big = Rect::new(self.bbox.x0 - 1, self.bbox.y0 - 1, self.bbox.x1 + 1, self.bbox.y1 + 1).intersect(&area);
        if big.is_empty() {
            return self.clone();
        }
        let (w, h) = (big.width() as usize, big.height() as usize);
        let cov = window(self, (big.x0, big.y0), w, h);
        let mut out = cov.clone();
        for y in 0..h {
            for x in 0..w {
                let v = cov[y * w + x];
                let mut sum = 0u32;
                let mut edge = false;
                for dy in -1i32..=1 {
                    for dx in -1i32..=1 {
                        let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                        let n = if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 { v } else { cov[ny as usize * w + nx as usize] };
                        edge |= n != v;
                        sum += u32::from(n);
                    }
                }
                if edge {
                    let avg = sum as f32 / (9.0 * 255.0);
                    let f = if v > 127 { 0.5 + 0.5 * avg } else { 0.5 * avg };
                    out[y * w + x] = (f * 255.0).round() as u8;
                }
            }
        }
        region_from(&out, w, h, (big.x0, big.y0)).unwrap_or_else(|| self.clone())
    }

    /// This region grown outward by up to `px` pixels, one ring at a time, where each ring stops
    /// at pixels darker than `stop_darkness` (0..255, with `stop_darkest`) or more opaque than
    /// `stop_alpha` (with `stop_opaque`): the fill reaches under anti-aliased line edges but
    /// never over the lines. Without either stop it is a plain [`Region::grow`].
    #[allow(clippy::too_many_arguments)]
    pub fn grow_until(&self, px: u32, area: Rect, img: &[[u8; 4]], stop_darkest: bool, stop_darkness: u8, stop_opaque: bool, stop_alpha: u8) -> Region {
        if !(stop_darkest || stop_opaque) {
            return self.grow(px, area);
        }
        let r = (px as usize).min(256);
        if r == 0 {
            return self.clone();
        }
        let big = Rect::new(self.bbox.x0 - r as i32, self.bbox.y0 - r as i32, self.bbox.x1 + r as i32, self.bbox.y1 + r as i32).intersect(&area);
        if big.is_empty() {
            return self.clone();
        }
        let (w, h) = (big.width() as usize, big.height() as usize);
        let aw = area.width() as usize;
        let mut cov = window(self, (big.x0, big.y0), w, h);
        let blocked = |x: usize, y: usize| -> bool {
            let (ax, ay) = ((big.x0 + x as i32 - area.x0) as usize, (big.y0 + y as i32 - area.y0) as usize);
            let Some(p) = img.get(ay * aw + ax) else { return true };
            (stop_darkest && darkness(*p) < stop_darkness) || (stop_opaque && p[3] > stop_alpha)
        };
        for _ in 0..r {
            let prev = cov.clone();
            for y in 0..h {
                for x in 0..w {
                    if prev[y * w + x] != 0 || blocked(x, y) {
                        continue;
                    }
                    let mut best = 0u8;
                    for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                        let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                        if nx >= 0 && ny >= 0 && nx < w as i32 && ny < h as i32 {
                            best = best.max(prev[ny as usize * w + nx as usize]);
                        }
                    }
                    cov[y * w + x] = best;
                }
            }
            if cov == prev {
                break;
            }
        }
        region_from(&cov, w, h, (big.x0, big.y0)).unwrap_or_else(|| self.clone())
    }

    /// This region with its edge feathered over `radius` pixels (a Gaussian blur of the
    /// coverage), clipped to `area`.
    pub fn feathered(&self, radius: f32, area: Rect) -> Region {
        if !(radius > 0.0) {
            return self.clone();
        }
        let pad = (radius * 3.0).ceil() as i32;
        let big = Rect::new(self.bbox.x0 - pad, self.bbox.y0 - pad, self.bbox.x1 + pad, self.bbox.y1 + pad).intersect(&area);
        if big.is_empty() {
            return self.clone();
        }
        let (w, h) = (big.width() as usize, big.height() as usize);
        let cov: Vec<f32> = window(self, (big.x0, big.y0), w, h).into_iter().map(|v| f32::from(v) / 255.0).collect();
        let soft = crate::selection::feather(&cov, w, h, radius);
        let out: Vec<u8> = soft.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8).collect();
        region_from(&out, w, h, (big.x0, big.y0)).unwrap_or_else(|| self.clone())
    }

    /// The whole of `area` at full coverage (the selection-shape fill mode: the selection itself
    /// weights the fill).
    pub fn full(area: Rect) -> Option<Region> {
        if area.is_empty() {
            return None;
        }
        Some(Region { bbox: area, mask: vec![255; area.width() as usize * area.height() as usize] })
    }

    /// The union of two regions (maximum coverage).
    pub fn union(&self, other: &Region) -> Region {
        let b = self.bbox.union(&other.bbox);
        let (w, h) = (b.width() as usize, b.height() as usize);
        let mut mask = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                let (px, py) = (b.x0 + x as i32, b.y0 + y as i32);
                mask[y * w + x] = ((self.at(px, py).max(other.at(px, py))) * 255.0).round() as u8;
            }
        }
        Region { bbox: b, mask }
    }
}

/// Drag fill: the contiguous regions under each of `seeds`, filled once each. With
/// `similar_only`, only regions whose colour is within `tolerance` of the first seed's colour
/// (any colour otherwise). Returns the union, or `None` when no seed hit the area.
pub fn drag_regions(
    img: &[[u8; 4]],
    area: Rect,
    seeds: &[(i32, i32)],
    tolerance: f32,
    anti_alias: bool,
    similar_only: bool,
    opts: WandOptions,
) -> Option<Region> {
    let (w, h) = (area.width() as usize, area.height() as usize);
    if w == 0 || h == 0 || img.len() < w * h {
        return None;
    }
    let mut done = vec![false; w * h];
    let mut first: Option<[u8; 4]> = None;
    let tol = (tolerance + 1e-3).floor().max(0.0) as i32;
    let mut out: Option<Region> = None;
    for &(x, y) in seeds {
        if !area.contains(x, y) {
            continue;
        }
        let i = (y - area.y0) as usize * w + (x - area.x0) as usize;
        if done[i] {
            continue;
        }
        let here = img[i];
        match first {
            None => first = Some(here),
            Some(f) if similar_only => {
                let close = (0..4).all(|c| (i32::from(here[c]) - i32::from(f[c])).abs() <= tol);
                if !close {
                    continue;
                }
            }
            _ => {}
        }
        let Some(r) = wand_region_with(img, area, (x, y), tolerance, true, anti_alias, opts) else { continue };
        for yy in r.bbox.y0..r.bbox.y1 {
            for xx in r.bbox.x0..r.bbox.x1 {
                if r.at(xx, yy) > 0.5 {
                    done[(yy - area.y0) as usize * w + (xx - area.x0) as usize] = true;
                }
            }
        }
        out = Some(match out {
            None => r,
            Some(acc) => acc.union(&r),
        });
    }
    out
}

/// Which regions inside an enclosing shape Enclose and Fill fills.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EncloseRule {
    /// Every region the shape fully surrounds.
    All,
    /// Only regions of (about) this colour.
    Color([u8; 4]),
    /// Only transparent regions.
    Transparent,
    /// Only regions whose surroundings are (about) this colour.
    SurroundedBy([u8; 4]),
}

/// Enclose and Fill: the colour regions (contiguous pixels within `tolerance` of each other,
/// 4-connected) that lie completely inside `enclosure` (true where the drawn shape covers, over
/// `area`), chosen by `rule`, less the exclusions: `exclude_transparent` drops transparent
/// regions, `exclude_color` regions of that colour. A region touching the shape's edge is not
/// enclosed. `None` when nothing qualifies.
#[allow(clippy::too_many_arguments)]
pub fn enclosed_regions(
    img: &[[u8; 4]],
    area: Rect,
    enclosure: &[bool],
    tolerance: f32,
    rule: EncloseRule,
    exclude_transparent: bool,
    exclude_color: Option<[u8; 4]>,
    anti_alias: bool,
) -> Option<Region> {
    let (w, h) = (area.width() as usize, area.height() as usize);
    if w == 0 || h == 0 || img.len() < w * h || enclosure.len() < w * h {
        return None;
    }
    let tol = (tolerance + 1e-3).floor().max(0.0) as i32;
    let close = |a: [u8; 4], b: [u8; 4]| (a[3] == 0 && b[3] == 0) || (0..4).all(|c| (i32::from(a[c]) - i32::from(b[c])).abs() <= tol);
    // Label the connected colour regions inside the enclosure; a region that touches a pixel
    // outside it (or the area's edge) is open, not enclosed.
    let mut label = vec![0u32; w * h];
    let mut next = 1u32;
    let mut open = vec![false];
    let mut seeds: Vec<usize> = vec![0];
    let mut sizes = vec![0usize];
    let mut stack = Vec::new();
    for start in 0..w * h {
        if !enclosure[start] || label[start] != 0 {
            continue;
        }
        let id = next;
        next += 1;
        open.push(false);
        seeds.push(start);
        sizes.push(0);
        let target = img[start];
        label[start] = id;
        stack.push(start);
        while let Some(i) = stack.pop() {
            sizes[id as usize] += 1;
            let (x, y) = (i % w, i / w);
            if x == 0 || y == 0 || x + 1 == w || y + 1 == h {
                open[id as usize] = true;
            }
            for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                    continue;
                }
                let j = ny as usize * w + nx as usize;
                if !close(img[j], target) {
                    continue;
                }
                if !enclosure[j] {
                    open[id as usize] = true;
                    continue;
                }
                if label[j] == 0 {
                    label[j] = id;
                    stack.push(j);
                }
            }
        }
    }
    // Which labels qualify.
    let mut keep = vec![false; next as usize];
    for id in 1..next as usize {
        if open[id] {
            continue;
        }
        let c = img[seeds[id]];
        let ok = match rule {
            EncloseRule::All => true,
            EncloseRule::Color(want) => close(c, want),
            EncloseRule::Transparent => c[3] == 0,
            EncloseRule::SurroundedBy(want) => {
                // Most of the pixels just outside the region are the surrounding colour.
                let (mut total, mut hits) = (0usize, 0usize);
                for i in 0..w * h {
                    if label[i] != id as u32 {
                        continue;
                    }
                    let (x, y) = (i % w, i / w);
                    for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                        let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                        if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                            continue;
                        }
                        let j = ny as usize * w + nx as usize;
                        if label[j] != id as u32 {
                            total += 1;
                            hits += usize::from(close(img[j], want));
                        }
                    }
                }
                total > 0 && hits * 10 >= total * 9
            }
        };
        let excluded = (exclude_transparent && c[3] == 0) || exclude_color.is_some_and(|x| close(c, x));
        keep[id] = ok && !excluded;
    }
    if !keep.iter().any(|k| *k) {
        return None;
    }
    let cov: Vec<u8> = label.iter().map(|l| if keep[*l as usize] { 255 } else { 0 }).collect();
    let r = region_from(&cov, w, h, (area.x0, area.y0))?;
    Some(if anti_alias { r.antialiased(area) } else { r })
}

/// `true` inside the polygon `points` over `area` (even-odd), the enclosing shape of a lasso or
/// rectangle drag; `None` for fewer than three points.
pub fn polygon_mask(points: &[(f32, f32)], area: Rect) -> Option<Vec<bool>> {
    if points.len() < 3 {
        return None;
    }
    Some(crate::selection::polygon(points, area, false).into_iter().map(|v| v > 0.5).collect())
}

/// `true` inside the ellipse inscribed in `(x0, y0)..(x1, y1)` over `area`.
pub fn ellipse_mask(x0: f32, y0: f32, x1: f32, y1: f32, area: Rect) -> Vec<bool> {
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let (rx, ry) = (((x1 - x0) / 2.0).abs().max(0.5), ((y1 - y0) / 2.0).abs().max(0.5));
    let (w, h) = (area.width() as usize, area.height() as usize);
    let mut out = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let (px, py) = (area.x0 as f32 + x as f32 + 0.5, area.y0 as f32 + y as f32 + 0.5);
            out[y * w + x] = ((px - cx) / rx).powi(2) + ((py - cy) / ry).powi(2) <= 1.0;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u8 = 255;
    fn img(w: usize, h: usize, f: impl Fn(usize, usize) -> [u8; 4]) -> Vec<[u8; 4]> {
        (0..w * h).map(|i| f(i % w, i / w)).collect()
    }

    #[test]
    fn antialias_feather_and_grow_until_stop_at_lines() {
        let area = Rect::new(0, 0, 10, 10);
        let r = Region::full(Rect::new(3, 3, 6, 6)).unwrap();
        let aa = r.antialiased(area);
        assert_eq!(aa.bbox, Rect::new(2, 2, 7, 7));
        assert!(aa.at(2, 2) > 0.0 && aa.at(2, 2) < 0.5, "{}", aa.at(2, 2));
        assert!(aa.at(3, 3) > 0.5 && aa.at(3, 3) < 1.0);
        assert_eq!(aa.at(4, 4), 1.0);
        let f = r.feathered(1.0, area);
        assert!(f.bbox.width() > 3 && f.at(4, 4) > 0.5 && f.at(2, 4) > 0.0);
        // A black line at x = 7 stops the grow; without stops it crosses.
        let px = img(10, 10, |x, _| if x == 7 { [0, 0, 0, W] } else { [W, W, W, W] });
        let g = r.grow_until(3, area, &px, true, 128, false, 0);
        assert_eq!(g.at(6, 4), 1.0);
        assert_eq!(g.at(7, 4), 0.0, "the line blocks");
        assert_eq!(g.at(8, 4), 0.0, "and so does everything past it");
        let plain = r.grow_until(3, area, &px, false, 0, false, 0);
        assert_eq!(plain.at(8, 4), 1.0);
        // A transparent window stops an opaque-stop grow nowhere; an opaque wall does.
        let px2 = img(10, 10, |x, _| if x == 1 { [0, 0, 0, W] } else { [0, 0, 0, 0] });
        let g2 = r.grow_until(3, area, &px2, false, 0, true, 128);
        assert_eq!(g2.at(1, 4), 0.0);
        assert_eq!(g2.at(0, 4), 0.0);
        assert_eq!(g2.at(2, 4), 1.0);
        let u = r.union(&Region::full(Rect::new(8, 8, 9, 9)).unwrap());
        assert_eq!(u.bbox, Rect::new(3, 3, 9, 9));
        assert_eq!(u.at(8, 8), 1.0);
        assert_eq!(u.at(7, 7), 0.0);
    }

    #[test]
    fn drag_fill_covers_each_region_once_and_can_limit_to_similar_colours() {
        let area = Rect::new(0, 0, 12, 4);
        // Three cells: white | red | white, split by black lines at x = 3 and x = 7.
        let px = img(12, 4, |x, _| match x {
            3 | 7 => [0, 0, 0, W],
            4..=6 => [W, 0, 0, W],
            _ => [W, W, W, W],
        });
        let seeds = [(1, 1), (2, 2), (5, 1), (9, 1)];
        let any = drag_regions(&px, area, &seeds, 0.0, false, false, WandOptions::default()).unwrap();
        assert_eq!(any.at(1, 1), 1.0);
        assert_eq!(any.at(5, 1), 1.0);
        assert_eq!(any.at(9, 1), 1.0);
        assert_eq!(any.at(3, 1), 0.0);
        let similar = drag_regions(&px, area, &seeds, 0.0, false, true, WandOptions::default()).unwrap();
        assert_eq!(similar.at(1, 1), 1.0);
        assert_eq!(similar.at(5, 1), 0.0, "red is not white");
        assert_eq!(similar.at(9, 1), 1.0);
        assert!(drag_regions(&px, area, &[(50, 50)], 0.0, false, false, WandOptions::default()).is_none());
    }

    #[test]
    fn enclose_and_fill_takes_only_fully_surrounded_regions() {
        let area = Rect::new(0, 0, 12, 12);
        // A black ring around a transparent hole at (4..8, 4..8), on a white background.
        let px = img(12, 12, |x, y| {
            let on_ring = (3..=8).contains(&x) && (3..=8).contains(&y) && !((4..8).contains(&x) && (4..8).contains(&y));
            if on_ring {
                [0, 0, 0, W]
            } else if (4..8).contains(&x) && (4..8).contains(&y) {
                [0, 0, 0, 0]
            } else {
                [W, W, W, W]
            }
        });
        let shape = polygon_mask(&[(1.0, 1.0), (11.0, 1.0), (11.0, 11.0), (1.0, 11.0)], area).unwrap();
        let all = enclosed_regions(&px, area, &shape, 0.0, EncloseRule::All, false, None, false).unwrap();
        assert_eq!(all.at(5, 5), 1.0, "the hole is enclosed");
        assert_eq!(all.at(4, 3), 1.0, "so is the ring");
        assert_eq!(all.at(1, 1), 0.0, "the background reaches the shape's edge");
        let t = enclosed_regions(&px, area, &shape, 0.0, EncloseRule::Transparent, false, None, false).unwrap();
        assert_eq!(t.at(5, 5), 1.0);
        assert_eq!(t.at(4, 3), 0.0);
        let s = enclosed_regions(&px, area, &shape, 0.0, EncloseRule::SurroundedBy([0, 0, 0, W]), false, None, false).unwrap();
        assert_eq!(s.at(5, 5), 1.0);
        assert_eq!(s.at(4, 3), 0.0, "the ring is surrounded by white and transparent, not black");
        assert!(enclosed_regions(&px, area, &shape, 0.0, EncloseRule::All, true, Some([0, 0, 0, W]), false).is_none(), "everything excluded");
        let c = enclosed_regions(&px, area, &shape, 0.0, EncloseRule::Color([0, 0, 0, W]), false, None, true).unwrap();
        assert_eq!(c.at(5, 5), 0.0);
        assert!(c.at(4, 3) > 0.5);
        // A shape that cuts through the ring leaves nothing enclosed but the hole.
        let half = polygon_mask(&[(1.0, 1.0), (6.0, 1.0), (6.0, 11.0), (1.0, 11.0)], area).unwrap();
        let h = enclosed_regions(&px, area, &half, 0.0, EncloseRule::All, false, None, false);
        assert!(h.is_none(), "{h:?}");
        let e = ellipse_mask(2.0, 2.0, 10.0, 10.0, area);
        assert!(e[6 * 12 + 6] && !e[0]);
    }
}
