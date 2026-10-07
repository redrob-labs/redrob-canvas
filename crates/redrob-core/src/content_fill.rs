// SPDX-License-Identifier: GPL-3.0-or-later

//! M10: content-aware fill (Photoshop's Edit > Content-Aware Fill, Shift+F5) by PatchMatch.
//!
//! The hole is filled with TEXTURE from the rest of the image, not a smooth average: every hole
//! pixel is matched to a source patch that looks like its surroundings, the matches improve over a
//! few rounds of propagation and random search (Barnes et al., "PatchMatch", 2009), and each round
//! re-colours the hole by letting every overlapping patch vote. The random search uses a fixed
//! seed, so the same image and selection always give the same fill.
//!
//! 8-bit RGBA only, like the other region tools; the caller refuses deeper documents.

use crate::error::Result;

/// The hole is filled from a source window around it, this much bigger than the hole's box on
/// every side, so a small blemish on a large photo does not search the whole image.
const SOURCE_MARGIN: i64 = 96;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*: small, fast, and deterministic for a fixed seed.
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: i64) -> i64 {
        (self.next() % n.max(1) as u64) as i64
    }
}

/// Fills `hole` pixels of `pixels` (8-bit RGBA, `w` x `h`) in place. `pixels` should already hold
/// a rough guess in the hole (any smooth fill); it is refined from there. Returns without change
/// when there is no hole or no valid source patch.
pub(crate) fn fill(
    pixels: &mut [u8],
    w: usize,
    h: usize,
    hole: &[bool],
    radius: i64,
    rounds: usize,
) -> Result<()> {
    let Some((hx0, hy0, hx1, hy1)) = bounds(hole, w, h) else {
        return Ok(());
    };
    let (wi, hi) = (w as i64, h as i64);
    // Source window and the patch centres in it whose whole patch is known (outside the hole).
    let (sx0, sy0) = (
        (hx0 - SOURCE_MARGIN).max(radius),
        (hy0 - SOURCE_MARGIN).max(radius),
    );
    let (sx1, sy1) = (
        (hx1 + SOURCE_MARGIN).min(wi - 1 - radius),
        (hy1 + SOURCE_MARGIN).min(hi - 1 - radius),
    );
    if sx1 < sx0 || sy1 < sy0 {
        return Ok(());
    }
    // Summed-area table of the hole, so "does this patch touch the hole" is four lookups.
    let mut sat = vec![0_u32; (w + 1) * (h + 1)];
    for y in 0..h {
        let mut row = 0_u32;
        for x in 0..w {
            row += u32::from(hole[y * w + x]);
            sat[(y + 1) * (w + 1) + x + 1] = sat[y * (w + 1) + x + 1] + row;
        }
    }
    let touches_hole = |cx: i64, cy: i64| {
        let (x0, y0, x1, y1) = (
            (cx - radius) as usize,
            (cy - radius) as usize,
            (cx + radius + 1) as usize,
            (cy + radius + 1) as usize,
        );
        sat[y1 * (w + 1) + x1] + sat[y0 * (w + 1) + x0]
            - sat[y0 * (w + 1) + x1]
            - sat[y1 * (w + 1) + x0]
            > 0
    };
    let mut sources = Vec::new();
    for cy in sy0..=sy1 {
        for cx in sx0..=sx1 {
            if !touches_hole(cx, cy) {
                sources.push((cx, cy));
            }
        }
    }
    if sources.is_empty() {
        return Ok(());
    }
    let valid = |cx: i64, cy: i64| {
        cx >= sx0 && cy >= sy0 && cx <= sx1 && cy <= sy1 && !touches_hole(cx, cy)
    };
    // The targets: every hole pixel (patches centred near the edge are clipped to the image).
    let targets: Vec<(i64, i64)> = (hy0..=hy1)
        .flat_map(|y| (hx0..=hx1).map(move |x| (x, y)))
        .filter(|(x, y)| hole[*y as usize * w + *x as usize])
        .collect();
    let index_of = |x: i64, y: i64| y as usize * w + x as usize;
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut nnf: Vec<(i64, i64)> = targets
        .iter()
        .map(|_| sources[rng.below(sources.len() as i64) as usize])
        .collect();
    let mut slot = vec![usize::MAX; w * h];
    for (i, (x, y)) in targets.iter().enumerate() {
        slot[index_of(*x, *y)] = i;
    }

    for round in 0..rounds {
        crate::cancel::checkpoint()?;
        let distance = |pixels: &[u8], tx: i64, ty: i64, sx: i64, sy: i64, best: u64| -> u64 {
            let mut sum = 0_u64;
            for dy in -radius..=radius {
                let (ty2, sy2) = (ty + dy, sy + dy);
                if ty2 < 0 || ty2 >= hi {
                    continue;
                }
                for dx in -radius..=radius {
                    let tx2 = tx + dx;
                    if tx2 < 0 || tx2 >= wi {
                        continue;
                    }
                    let (t, s) = (index_of(tx2, ty2) * 4, index_of(sx + dx, sy2) * 4);
                    // Known pixels count fully; current guesses inside the hole count less.
                    let weight = if hole[index_of(tx2, ty2)] { 1 } else { 3 };
                    for c in 0..4 {
                        let d = i64::from(pixels[t + c]) - i64::from(pixels[s + c]);
                        sum += weight * (d * d) as u64;
                    }
                }
                if sum >= best {
                    return sum; // Already worse than the best: stop early.
                }
            }
            sum
        };
        let forward = round % 2 == 0;
        let order: Vec<usize> = if forward {
            (0..targets.len()).collect()
        } else {
            (0..targets.len()).rev().collect()
        };
        let step: i64 = if forward { -1 } else { 1 };
        for i in order {
            let (tx, ty) = targets[i];
            let (mut bx, mut by) = nnf[i];
            let mut best = distance(pixels, tx, ty, bx, by, u64::MAX);
            // Propagation: a neighbour's match, shifted by one, is often good here too.
            for (nx, ny) in [(tx + step, ty), (tx, ty + step)] {
                if nx < 0 || ny < 0 || nx >= wi || ny >= hi {
                    continue;
                }
                let n = slot[index_of(nx, ny)];
                if n == usize::MAX {
                    continue;
                }
                let (cx, cy) = (nnf[n].0 - (nx - tx), nnf[n].1 - (ny - ty));
                if valid(cx, cy) {
                    let d = distance(pixels, tx, ty, cx, cy, best);
                    if d < best {
                        (best, bx, by) = (d, cx, cy);
                    }
                }
            }
            // Random search in shrinking windows around the current best.
            let mut span = (sx1 - sx0).max(sy1 - sy0).max(1);
            while span >= 1 {
                let (cx, cy) = (
                    bx + rng.below(2 * span + 1) - span,
                    by + rng.below(2 * span + 1) - span,
                );
                if valid(cx, cy) {
                    let d = distance(pixels, tx, ty, cx, cy, best);
                    if d < best {
                        (best, bx, by) = (d, cx, cy);
                    }
                }
                span /= 2;
            }
            nnf[i] = (bx, by);
        }
        // Voting: every hole pixel takes the average of what each overlapping patch says it is.
        let mut acc = vec![[0_u32; 5]; targets.len()];
        for (i, (tx, ty)) in targets.iter().enumerate() {
            let (sx, sy) = nnf[i];
            for dy in -radius..=radius {
                for dx in -radius..=radius {
                    let (px, py) = (tx + dx, ty + dy);
                    if px < 0 || py < 0 || px >= wi || py >= hi {
                        continue;
                    }
                    let j = slot[index_of(px, py)];
                    if j == usize::MAX {
                        continue;
                    }
                    let s = index_of(sx + dx, sy + dy) * 4;
                    for c in 0..4 {
                        acc[j][c] += u32::from(pixels[s + c]);
                    }
                    acc[j][4] += 1;
                }
            }
        }
        for (j, (x, y)) in targets.iter().enumerate() {
            let n = acc[j][4].max(1);
            let t = index_of(*x, *y) * 4;
            for c in 0..4 {
                pixels[t + c] = ((acc[j][c] + n / 2) / n) as u8;
            }
        }
    }
    Ok(())
}

fn bounds(hole: &[bool], w: usize, h: usize) -> Option<(i64, i64, i64, i64)> {
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if hole[y * w + x] {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    (x0 != usize::MAX).then_some((x0 as i64, y0 as i64, x1 as i64, y1 as i64))
}
