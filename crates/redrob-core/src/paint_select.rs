// SPDX-License-Identifier: GPL-3.0-or-later

//! Paint select: rough strokes refine an existing selection (L.5).
//!
//! Re-derived from `app/tools/gimppaintselecttool.c` and
//! `app/tools/gimppaintselectoptions.c` (GPL-3.0-or-later), pinned and attributed in
//! `docs/upstream-sources.toml`. Registered upstream as `gimp-paint-select-tool`, immediately after
//! `gimp_foreground_select_tool_register` in `gimp-tools.c` — the two are neighbours and paint
//! select even reuses `GIMP_HELP_TOOL_FOREGROUND_SELECT`, which is upstream itself saying they are
//! one family with two interactions.
//!
//! # What this is NOT, because we already ship its neighbour
//!
//! `Command::SelectForeground` is the other tool: foreground and background scribbles handed over
//! together, every pixel labelled by whichever sample set its colour is nearer in Lab. It does not
//! read the existing selection at all.
//!
//! Paint select is the refining one. **One stroke carries ONE label**, and the context it works
//! against is the selection as it already stands. That is visible in the tool: the trimap is reset
//! to grey on every button press (*"Always reset the scribbles to start with a blank slate"*), and
//! `painting_op` is latched from the options at that moment, so a single stroke can only ever write
//! one value. The accumulated state lives in the selection, not in the scribbles.
//!
//! # The trimap, read exactly
//!
//! Three values in a `Y float` buffer: unknown is **grey `#888`**, a scribble under
//! `GIMP_CHANNEL_OP_ADD` writes **1.0**, and a scribble under **anything else** writes **0.0** —
//! the branch is `painting_op == GIMP_CHANNEL_OP_ADD ? 1.f : 0.f`, so replace and intersect take
//! the background value along with subtract.
//!
//! # The asymmetric threshold is the load-bearing read
//!
//! The existing selection reaches the operation through a `gegl:threshold` whose value is switched
//! with the operation: **0.99 for ADD, 0.01 for everything else**. One threshold for both would be
//! the obvious simplification and would be wrong in one direction, and the reason falls out of what
//! each direction needs from the mask:
//!
//! - growing, the mask is a SEED. Only pixels that are already confidently selected can be trusted
//!   to belong to the object, so the cut is high and the seed is the selection's core.
//! - shrinking, the mask is a BOUND. Everything outside the selection is known not to be part of
//!   what is being removed, so the cut is low and the bound is the selection's full extent.
//!
//! # The kernel is OURS, and that is the honest part of this port
//!
//! The segmentation itself is `gegl:paint-select`, and **GEGL is the one upstream not vendored
//! here** — the same single cause that has five filters parked. So the envelope above is read and
//! the kernel below is derived, and it will not agree pixel-for-pixel with upstream.
//!
//! It is derived from mechanisms this repository already ships rather than invented: the edge cost
//! is the luma-gradient cost the lazy brush and the scissors both use, and the background seed is
//! the canvas border, the convention enclose-and-fill already relies on. There is deliberately no
//! invented threshold constant — a pixel belongs to the object when the cheapest path to it from
//! the object seeds is cheaper than the cheapest path from the border, which is a watershed between
//! two seed sets and needs no tuning value.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// Default `stroke-width`, read from `gimppaintselectoptions.c`.
pub const PAINT_SELECT_DEFAULT_STROKE_WIDTH: u32 = 50;

/// `stroke-width` bounds, read from the same declaration (`1, 6000, 50`).
pub const PAINT_SELECT_MIN_STROKE_WIDTH: u32 = 1;
pub const PAINT_SELECT_MAX_STROKE_WIDTH: u32 = 6000;

/// The mask cut applied to the existing selection when GROWING it (`gegl:threshold` at 0.99).
pub const PAINT_SELECT_ADD_MASK_CUT: f32 = 0.99;

/// The mask cut applied when shrinking it (`gegl:threshold` at 0.01).
pub const PAINT_SELECT_REMOVE_MASK_CUT: f32 = 0.01;

struct Node(f64, usize);

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0 && self.1 == other.1
    }
}
impl Eq for Node {}
impl PartialOrd for Node {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Node {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reversed so `BinaryHeap` behaves as a min-heap on distance, matching the lazy brush.
        other
            .0
            .partial_cmp(&self.0)
            .unwrap_or(Ordering::Equal)
            .then_with(|| other.1.cmp(&self.1))
    }
}

/// Cheapest-path distance from every seed in `seeds`, over the luma-gradient edge cost.
///
/// `INFINITY` where no seed exists or the budget ran out. The cost of ENTERING a cell is
/// `0.01 + edge^2 * 8`, the lazy brush's formula unchanged: a flat region is nearly free to cross
/// and a sharp line is dear, which is what makes the watershed land on the drawn edge.
fn cost_field(
    pixels: &[u8],
    width: usize,
    height: usize,
    seeds: &[bool],
    budget: usize,
) -> Vec<f64> {
    let n = width * height;
    let luma = |i: usize| -> f64 {
        let o = i * 4;
        0.299 * f64::from(pixels[o])
            + 0.587 * f64::from(pixels[o + 1])
            + 0.114 * f64::from(pixels[o + 2])
    };
    let edge = |x: usize, y: usize| -> f64 {
        let xm = x.saturating_sub(1);
        let xp = (x + 1).min(width - 1);
        let ym = y.saturating_sub(1);
        let yp = (y + 1).min(height - 1);
        let gx = (luma(y * width + xp) - luma(y * width + xm)).abs();
        let gy = (luma(yp * width + x) - luma(ym * width + x)).abs();
        (gx + gy) / 510.0
    };

    let mut dist = vec![f64::INFINITY; n];
    let mut heap = BinaryHeap::new();
    for (index, &seeded) in seeds.iter().enumerate() {
        if seeded {
            dist[index] = 0.0;
            heap.push(Node(0.0, index));
        }
    }

    let mut visits = 0usize;
    while let Some(Node(d, index)) = heap.pop() {
        if d > dist[index] {
            continue;
        }
        visits += 1;
        if visits > budget {
            break;
        }
        let (x, y) = (index % width, index / width);
        let relax = |nx: usize, ny: usize, heap: &mut BinaryHeap<Node>, dist: &mut Vec<f64>| {
            let next = ny * width + nx;
            let step = 0.01 + edge(nx, ny).powi(2) * 8.0;
            let candidate = d + step;
            if candidate < dist[next] {
                dist[next] = candidate;
                heap.push(Node(candidate, next));
            }
        };
        if x > 0 {
            relax(x - 1, y, &mut heap, &mut dist);
        }
        if x + 1 < width {
            relax(x + 1, y, &mut heap, &mut dist);
        }
        if y > 0 {
            relax(x, y - 1, &mut heap, &mut dist);
        }
        if y + 1 < height {
            relax(x, y + 1, &mut heap, &mut dist);
        }
    }
    dist
}

/// Every pixel on the canvas border, the background seed set.
fn border_seeds(width: usize, height: usize) -> Vec<bool> {
    let mut seeds = vec![false; width * height];
    for x in 0..width {
        seeds[x] = true;
        seeds[(height - 1) * width + x] = true;
    }
    for y in 0..height {
        seeds[y * width] = true;
        seeds[y * width + width - 1] = true;
    }
    seeds
}

/// The region a stroke's scribbles belong to, as an 8-bit coverage mask.
///
/// `object` is the stroke's own dabs, plus the thresholded selection core when growing; `excluded`
/// marks pixels that are known NOT to be part of the region, which is how the shrinking direction
/// uses the selection's full extent. A pixel joins the region when it is cheaper to reach from the
/// object seeds than from the background seeds.
pub(crate) fn region(
    pixels: &[u8],
    width: u32,
    height: u32,
    object: &[bool],
    excluded: &[bool],
    budget: usize,
) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let n = w * h;
    if n == 0 || !object.iter().any(|&seeded| seeded) {
        return vec![0u8; n];
    }

    // The background seeds are the canvas border plus anything the caller ruled out. Both are
    // "definitely not this region", so they compete against the object from the same field.
    let mut background = border_seeds(w, h);
    for (slot, &ruled_out) in background.iter_mut().zip(excluded.iter()) {
        *slot |= ruled_out;
    }

    let to_object = cost_field(pixels, w, h, object, budget);
    let to_background = cost_field(pixels, w, h, &background, budget);

    (0..n)
        .map(|i| {
            // A seeded pixel is in by construction, which matters when a pixel is seeded on BOTH
            // sides (a scribble drawn on the canvas border): its two distances are then equally
            // zero and the comparison alone would drop it. Elsewhere the cheaper field wins, and
            // ties go to the background so a pixel the two reach equally is not swept in.
            if object[i] || to_object[i] < to_background[i] {
                255
            } else {
                0
            }
        })
        .collect()
}

/// A round dab of diameter `stroke_width` centred on each scribble point.
///
/// Upstream builds this with `gimp_scan_convert_stroke` over a degenerate two-point polyline —
/// both points at the centre, the second nudged by 0.01 — with `GIMP_JOIN_ROUND` and
/// `GIMP_CAP_ROUND`, which is a single round dab. `radius = size / 2` is INTEGER division there, so
/// an even and the odd width below it share a radius.
pub(crate) fn scribble_mask(
    width: u32,
    height: u32,
    scribbles: &[(u32, u32)],
    stroke_width: u32,
) -> Vec<bool> {
    let (w, h) = (width as usize, height as usize);
    let mut mask = vec![false; w * h];
    let radius = (stroke_width / 2) as i64;
    for &(cx, cy) in scribbles {
        let (cx, cy) = (i64::from(cx), i64::from(cy));
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                if dx * dx + dy * dy > radius * radius {
                    continue;
                }
                let (x, y) = (cx + dx, cy + dy);
                if x < 0 || y < 0 || x >= w as i64 || y >= h as i64 {
                    continue;
                }
                mask[y as usize * w + x as usize] = true;
            }
        }
    }
    // A zero radius still marks the centre, so a one-pixel stroke width is not a no-op.
    if radius == 0 {
        for &(cx, cy) in scribbles {
            if (cx as usize) < w && (cy as usize) < h {
                mask[cy as usize * w + cx as usize] = true;
            }
        }
    }
    mask
}
