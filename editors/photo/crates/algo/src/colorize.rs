//! Colorize mask: key strokes of colour placed inside a line-art layer's regions are propagated
//! into those regions. The ink (dark, opaque pixels; optionally detected edges too) is the
//! barrier, gaps in it up to a size are closed, every non-ink pixel takes the colour of the
//! nearest key stroke it can be reached from, the colour runs under the ink so no halo is left,
//! and a transparent key keeps its region clear.

use redrob_photo_geom::Rect;

/// A key stroke: a polyline of `width` pixels in `color` (`None`: the transparent key).
#[derive(Clone, Debug, PartialEq)]
pub struct KeyStroke {
    pub color: Option<[u8; 4]>,
    pub points: Vec<(f32, f32)>,
    pub width: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorizeOptions {
    /// Luma (0..1) below which an opaque pixel is ink.
    pub threshold: f32,
    /// Also treat strong luma edges as ink.
    pub edge_detection: bool,
    /// Gaps in the ink up to this many pixels are closed.
    pub gap_close: u32,
    /// 0..100: how much of the colour under the ink is removed (0 = colour runs fully under the
    /// lines, 100 = it stops at them).
    pub clean_up: f32,
}

impl Default for ColorizeOptions {
    fn default() -> Self {
        Self { threshold: 0.9, edge_detection: true, gap_close: 8, clean_up: 0.0 }
    }
}

fn luma(p: [u8; 4]) -> u32 {
    (u32::from(p[0]) * 299 + u32::from(p[1]) * 587 + u32::from(p[2]) * 114) / 1000
}

/// Which pixels of `img` are ink: opaque and dark, plus (with edge detection) pixels whose luma
/// differs sharply from a neighbour's.
pub fn ink_mask(img: &[[u8; 4]], w: usize, h: usize, threshold: f32, edge_detection: bool) -> Vec<bool> {
    let cut = (threshold.clamp(0.0, 1.0) * 255.0) as u32;
    let mut ink: Vec<bool> = img.iter().take(w * h).map(|p| p[3] > 32 && luma(*p) < cut).collect();
    if edge_detection {
        let lum: Vec<i32> = img.iter().take(w * h).map(|p| if p[3] > 32 { luma(*p) as i32 } else { 255 }).collect();
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if ink[i] {
                    continue;
                }
                let l = lum[i];
                let right = if x + 1 < w { lum[i + 1] } else { l };
                let down = if y + 1 < h { lum[i + w] } else { l };
                if (l - right).abs() > 96 || (l - down).abs() > 96 {
                    ink[i] = true;
                }
            }
        }
    }
    ink
}

/// Binary dilation by a square of radius `r`.
fn dilate(m: &[bool], w: usize, h: usize, r: usize) -> Vec<bool> {
    if r == 0 {
        return m.to_vec();
    }
    let mut rows = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let (lo, hi) = (x.saturating_sub(r), (x + r).min(w - 1));
            rows[y * w + x] = m[y * w + lo..=y * w + hi].iter().any(|v| *v);
        }
    }
    let mut out = vec![false; w * h];
    for x in 0..w {
        for y in 0..h {
            let (lo, hi) = (y.saturating_sub(r), (y + r).min(h - 1));
            out[y * w + x] = (lo..=hi).any(|yy| rows[yy * w + x]);
        }
    }
    out
}

/// Rasterize the key strokes onto a label map (stroke index + 1, 0 = none) over `area`.
fn rasterize(strokes: &[KeyStroke], area: Rect) -> Vec<u32> {
    let (w, h) = (area.width() as usize, area.height() as usize);
    let mut labels = vec![0u32; w * h];
    for (k, s) in strokes.iter().enumerate() {
        let r = (s.width.max(1.0) / 2.0).max(0.5);
        let mut stamp = |cx: f32, cy: f32| {
            let (x0, x1) = ((cx - r).floor() as i32, (cx + r).ceil() as i32);
            let (y0, y1) = ((cy - r).floor() as i32, (cy + r).ceil() as i32);
            for y in y0..=y1 {
                for x in x0..=x1 {
                    if !area.contains(x, y) {
                        continue;
                    }
                    let (dx, dy) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - cy);
                    if dx * dx + dy * dy <= r * r + 0.25 {
                        labels[(y - area.y0) as usize * w + (x - area.x0) as usize] = k as u32 + 1;
                    }
                }
            }
        };
        if s.points.is_empty() {
            continue;
        }
        if s.points.len() == 1 {
            stamp(s.points[0].0, s.points[0].1);
            continue;
        }
        for pair in s.points.windows(2) {
            let ((ax, ay), (bx, by)) = (pair[0], pair[1]);
            let len = ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt();
            let steps = (len / (r.max(0.5))).ceil().max(1.0) as usize;
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                stamp(ax + (bx - ax) * t, ay + (by - ay) * t);
            }
        }
    }
    labels
}

/// The colorize mask's output over `area`: straight RGBA per pixel (transparent where no key
/// reaches, under a transparent key, and over the ink when `clean_up` says so).
pub fn colorize(img: &[[u8; 4]], area: Rect, strokes: &[KeyStroke], opts: &ColorizeOptions) -> Vec<[u8; 4]> {
    let (w, h) = (area.width() as usize, area.height() as usize);
    let mut out = vec![[0u8; 4]; w * h];
    if w == 0 || h == 0 || img.len() < w * h || strokes.is_empty() {
        return out;
    }
    let ink = ink_mask(img, w, h, opts.threshold, opts.edge_detection);
    let half_gap = (opts.gap_close as usize).div_ceil(2).min(64);
    let barrier = dilate(&ink, w, h, half_gap);
    let mut labels = rasterize(strokes, area);
    // Key strokes painted over the barrier still seed: a key crossing a line colours both sides.
    // Multi-source breadth-first flood over the open pixels.
    let mut queue: std::collections::VecDeque<usize> = (0..w * h).filter(|i| labels[*i] != 0 && !barrier[*i]).collect();
    while let Some(i) = queue.pop_front() {
        let l = labels[i];
        let (x, y) = (i % w, i / w);
        for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
            let (nx, ny) = (x as i32 + dx, y as i32 + dy);
            if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                continue;
            }
            let j = ny as usize * w + nx as usize;
            if labels[j] == 0 && !barrier[j] {
                labels[j] = l;
                queue.push_back(j);
            }
        }
    }
    // Under the ink: the nearest coloured pixel's label, by repeated dilation, as far as the
    // closing band reaches less the clean-up share.
    let reach = ((half_gap as f32 + 1.0) * (1.0 - opts.clean_up.clamp(0.0, 100.0) / 100.0)).round() as usize;
    for _ in 0..reach {
        let prev = labels.clone();
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if prev[i] != 0 || !barrier[i] {
                    continue;
                }
                for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    let l = prev[ny as usize * w + nx as usize];
                    if l != 0 {
                        labels[i] = l;
                        break;
                    }
                }
            }
        }
        if labels == prev {
            break;
        }
    }
    for (o, l) in out.iter_mut().zip(&labels) {
        if *l == 0 {
            continue;
        }
        if let Some(c) = strokes[*l as usize - 1].color {
            *o = c;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u8 = 255;

    #[test]
    fn keys_fill_their_regions_and_run_under_the_ink() {
        // Two cells split by a black line at x = 5 with a one-pixel gap at y = 3.
        let area = Rect::new(0, 0, 11, 7);
        let img: Vec<[u8; 4]> = (0..11 * 7).map(|i| if i % 11 == 5 && i / 11 != 3 { [0, 0, 0, W] } else { [W, W, W, W] }).collect();
        let strokes = [
            KeyStroke { color: Some([W, 0, 0, W]), points: vec![(1.5, 1.5), (2.5, 1.5)], width: 1.0 },
            KeyStroke { color: Some([0, 0, W, W]), points: vec![(8.5, 1.5)], width: 1.0 },
        ];
        let opts = ColorizeOptions { threshold: 0.5, edge_detection: false, gap_close: 2, clean_up: 0.0 };
        let out = colorize(&img, area, &strokes, &opts);
        let at = |x: usize, y: usize| out[y * 11 + x];
        assert_eq!(at(1, 5), [W, 0, 0, W], "the left cell is red");
        assert_eq!(at(9, 5), [0, 0, W, W], "the right cell is blue");
        assert_ne!(at(5, 1)[3], 0, "the line itself is coloured underneath");
        // Without gap closing the gap at y = 3 lets red through into the right cell first.
        let leak = colorize(&img, area, &strokes, &ColorizeOptions { gap_close: 0, ..opts });
        assert!(leak[3 * 11 + 5] != [0, 0, 0, 0]);
        // A transparent key keeps its region clear; a region with no key stays clear.
        let strokes2 = [KeyStroke { color: None, points: vec![(1.5, 1.5)], width: 1.0 }];
        let out2 = colorize(&img, area, &strokes2, &opts);
        assert_eq!(out2[5 * 11 + 1], [0, 0, 0, 0]);
        assert_eq!(out2[5 * 11 + 9], [0, 0, 0, 0]);
        // Full clean-up keeps the colour off the ink.
        let clean = colorize(&img, area, &strokes, &ColorizeOptions { clean_up: 100.0, ..opts });
        assert_eq!(clean[11 + 5], [0, 0, 0, 0]);
        assert_eq!(clean[5 * 11 + 1], [W, 0, 0, W]);
        assert!(colorize(&img, area, &[], &opts).iter().all(|p| p[3] == 0));
    }

    #[test]
    fn edge_detection_adds_sharp_edges_to_the_ink() {
        let img: Vec<[u8; 4]> = (0..6 * 2).map(|i| if i % 6 < 3 { [W, W, W, W] } else { [40, 40, 40, W] }).collect();
        let plain = ink_mask(&img, 6, 2, 0.1, false);
        assert!(plain.iter().all(|v| !*v), "nothing is darker than 10 %");
        let edged = ink_mask(&img, 6, 2, 0.1, true);
        assert!(edged[2] && !edged[0], "the pixel before the jump is an edge");
    }
}
