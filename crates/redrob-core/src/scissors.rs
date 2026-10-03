//! Intelligent scissors / magnetic selection (C.4), re-derived from GIMP's `iscissors` and Krita's
//! magnetic select, not copied.
//!
//! The user drops anchor points; between each pair the path snaps to the strongest edge. The classic
//! algorithm (Mortensen & Barrett's "live wire") builds a per-pixel cost that is LOW on edges, then
//! finds the least-cost path between the two anchors with Dijkstra. The cost here is `1 - gradient`,
//! so a strong gradient (an edge) is cheap to travel along and the path hugs it; a flat region is
//! expensive, so the path crosses it in a straight line rather than wandering.
//!
//! The whole traced boundary (every segment joined) is returned as a polygon, which the selection
//! rasteriser then fills.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// Sobel gradient magnitude at every pixel, 0..=1 (1 = strongest edge). Luma of the RGB, so a colour
/// edge with equal luma is invisible to the wire -- the same limitation GIMP's grayscale cost has.
fn gradient_map(pixels: &[u8], width: u32, height: u32) -> Vec<f32> {
    let w = width as usize;
    let h = height as usize;
    let luma = |x: usize, y: usize| -> f32 {
        let o = (y * w + x) * 4;
        0.299 * f32::from(pixels[o])
            + 0.587 * f32::from(pixels[o + 1])
            + 0.114 * f32::from(pixels[o + 2])
    };
    let mut out = vec![0.0_f32; w * h];
    let mut max = 1.0_f32;
    for y in 0..h {
        for x in 0..w {
            let xm = x.saturating_sub(1);
            let xp = (x + 1).min(w - 1);
            let ym = y.saturating_sub(1);
            let yp = (y + 1).min(h - 1);
            let gx = (luma(xp, ym) + 2.0 * luma(xp, y) + luma(xp, yp))
                - (luma(xm, ym) + 2.0 * luma(xm, y) + luma(xm, yp));
            let gy = (luma(xm, yp) + 2.0 * luma(x, yp) + luma(xp, yp))
                - (luma(xm, ym) + 2.0 * luma(x, ym) + luma(xp, ym));
            let g = (gx * gx + gy * gy).sqrt();
            out[y * w + x] = g;
            if g > max {
                max = g;
            }
        }
    }
    for v in &mut out {
        *v /= max;
    }
    out
}

#[derive(Clone, Copy)]
struct State {
    cost: f32,
    index: usize,
}
impl PartialEq for State {
    fn eq(&self, other: &Self) -> bool {
        self.cost == other.cost
    }
}
impl Eq for State {}
impl Ord for State {
    fn cmp(&self, other: &Self) -> Ordering {
        // Min-heap: reverse so the smallest cost pops first. NaN never occurs (costs are finite).
        other
            .cost
            .partial_cmp(&self.cost)
            .unwrap_or(Ordering::Equal)
    }
}
impl PartialOrd for State {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Least-cost edge path from `start` to `goal` over the gradient cost map, returned as pixel points
/// (inclusive of both ends). A straight fallback is used if the search is bounded out.
fn live_wire(
    cost: &[f32],
    width: u32,
    height: u32,
    start: (u32, u32),
    goal: (u32, u32),
    budget: usize,
) -> Vec<(f32, f32)> {
    let w = width as usize;
    let h = height as usize;
    let idx = |x: u32, y: u32| y as usize * w + x as usize;
    let start_i = idx(start.0, start.1);
    let goal_i = idx(goal.0, goal.1);
    let mut dist = vec![f32::INFINITY; w * h];
    let mut prev = vec![usize::MAX; w * h];
    let mut heap = BinaryHeap::new();
    dist[start_i] = 0.0;
    heap.push(State {
        cost: 0.0,
        index: start_i,
    });
    let mut visited = 0usize;
    // Destructured as `reached`, NOT `cost`: the parameter of that name is the gradient cost MAP, and
    // shadowing it here makes `cost[ni]` below index a single f32 instead of the map.
    while let Some(State {
        cost: reached,
        index,
    }) = heap.pop()
    {
        if index == goal_i {
            break;
        }
        if reached > dist[index] {
            continue;
        }
        visited += 1;
        if visited > budget {
            break;
        }
        let x = (index % w) as i32;
        let y = (index / w) as i32;
        for (dx, dy) in [
            (-1, 0),
            (1, 0),
            (0, -1),
            (0, 1),
            (-1, -1),
            (1, -1),
            (-1, 1),
            (1, 1),
        ] {
            let nx = x + dx;
            let ny = y + dy;
            if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                continue;
            }
            let ni = ny as usize * w + nx as usize;
            // Edge cost: cheap where the gradient is high. Diagonal steps cost sqrt(2) more length.
            let diag = if dx != 0 && dy != 0 {
                std::f32::consts::SQRT_2
            } else {
                1.0
            };
            let step = (1.0 - cost[ni]) * diag + 0.001;
            let nd = dist[index] + step;
            if nd < dist[ni] {
                dist[ni] = nd;
                prev[ni] = index;
                heap.push(State {
                    cost: nd,
                    index: ni,
                });
            }
        }
    }
    if prev[goal_i] == usize::MAX && goal_i != start_i {
        // Search did not reach the goal within budget: fall back to the straight segment.
        return vec![
            (start.0 as f32, start.1 as f32),
            (goal.0 as f32, goal.1 as f32),
        ];
    }
    let mut path = Vec::new();
    let mut cur = goal_i;
    loop {
        path.push(((cur % w) as f32, (cur / w) as f32));
        if cur == start_i {
            break;
        }
        cur = prev[cur];
        if cur == usize::MAX {
            break;
        }
    }
    path.reverse();
    path
}

/// Trace the full magnetic boundary through the anchors (implicitly closed) and return it as a
/// polygon of pixel points. `anchors` are (x, y) in canvas pixels.
pub fn magnetic_boundary(
    pixels: &[u8],
    width: u32,
    height: u32,
    anchors: &[(u32, u32)],
    budget: usize,
) -> Vec<(f32, f32)> {
    if anchors.len() < 2 {
        return Vec::new();
    }
    let cost = gradient_map(pixels, width, height);
    let mut polygon = Vec::new();
    for i in 0..anchors.len() {
        let a = anchors[i];
        let b = anchors[(i + 1) % anchors.len()];
        let segment = live_wire(&cost, width, height, a, b, budget);
        // Drop the duplicated join between segments.
        if polygon.is_empty() {
            polygon.extend(segment);
        } else {
            polygon.extend(segment.into_iter().skip(1));
        }
    }
    polygon
}
