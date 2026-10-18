//! The isometric grid (Preferences › Guides, Grid & Slices › Grid type): three line families,
//! rising to the right at the right angle, rising to the left at the left angle, and vertical,
//! each spaced by the gridline distance and clipped to the document. Pure geometry; `rulers`
//! draws it.

/// A line family's segments across a `w × h` document: lines at `angle_deg` from the horizontal
/// (rising to the right for a positive angle, in y-down document space), `step` apart measured
/// perpendicular to them, offset by `offset` along y.
pub fn family(w: f64, h: f64, angle_deg: f64, step: f64, offset: f64) -> Vec<[[f64; 2]; 2]> {
    let mut out = Vec::new();
    if !(step > 0.0) || !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0 {
        return out;
    }
    let a = angle_deg.to_radians();
    // Direction of the lines (rising to the right: y decreases as x grows) and their normal.
    let d = [a.cos(), -a.sin()];
    let n = [-d[1], d[0]];
    // Range of the normal coordinate over the document's corners.
    let corners = [[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]];
    let proj = |p: [f64; 2]| p[0] * n[0] + p[1] * n[1];
    let (lo, hi) = corners.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), c| (lo.min(proj(*c)), hi.max(proj(*c))));
    let shift = offset * n[1];
    let first = ((lo - shift) / step).ceil() as i64;
    let last = ((hi - shift) / step).floor() as i64;
    if last < first || last - first > 100_000 {
        return out;
    }
    for k in first..=last {
        let c = k as f64 * step + shift;
        // A point on the line, then clip the infinite line to the rectangle.
        let o = [n[0] * c, n[1] * c];
        if let Some(seg) = clip(o, d, w, h) {
            out.push(seg);
        }
    }
    out
}

/// The vertical family: x = offset + k × step.
pub fn verticals(w: f64, h: f64, step: f64, offset: f64) -> Vec<[[f64; 2]; 2]> {
    let mut out = Vec::new();
    if !(step > 0.0) || w <= 0.0 {
        return out;
    }
    let mut x = offset.rem_euclid(step);
    if x <= 0.0 {
        x += step;
    }
    while x < w {
        out.push([[x, 0.0], [x, h]]);
        x += step;
    }
    out
}

/// The infinite line through `o` along `d` clipped to `[0, w] × [0, h]`.
fn clip(o: [f64; 2], d: [f64; 2], w: f64, h: f64) -> Option<[[f64; 2]; 2]> {
    let (mut t0, mut t1) = (f64::NEG_INFINITY, f64::INFINITY);
    for (p, dir, lo, hi) in [(o[0], d[0], 0.0, w), (o[1], d[1], 0.0, h)] {
        if dir.abs() < 1e-12 {
            if p < lo || p > hi {
                return None;
            }
            continue;
        }
        let (a, b) = ((lo - p) / dir, (hi - p) / dir);
        t0 = t0.max(a.min(b));
        t1 = t1.min(a.max(b));
    }
    (t1 - t0 > 1e-9).then(|| [[o[0] + d[0] * t0, o[1] + d[1] * t0], [o[0] + d[0] * t1, o[1] + d[1] * t1]])
}

/// Every segment of an isometric grid: the right-rising family at `right_deg`, the left-rising
/// one at `left_deg`, and verticals when `vertical`.
pub fn lines(w: f64, h: f64, step: f64, left_deg: f64, right_deg: f64, vertical: bool, offset: [f64; 2]) -> Vec<[[f64; 2]; 2]> {
    let mut out = family(w, h, right_deg.clamp(1.0, 89.0), step, offset[1]);
    out.extend(family(w, h, -left_deg.clamp(1.0, 89.0), step, offset[1]));
    if vertical {
        out.extend(verticals(w, h, step, offset[0]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_cover_the_document_and_clip_to_it() {
        let f = family(100.0, 100.0, 30.0, 20.0, 0.0);
        assert!(!f.is_empty());
        for [a, b] in &f {
            for p in [a, b] {
                assert!(p[0] >= -1e-6 && p[0] <= 100.0 + 1e-6 && p[1] >= -1e-6 && p[1] <= 100.0 + 1e-6, "{p:?}");
            }
            // Rising to the right: the right end is higher (smaller y).
            let (l, r) = if a[0] <= b[0] { (a, b) } else { (b, a) };
            assert!(r[1] <= l[1] + 1e-6);
        }
        // Perpendicular spacing of 20 px between neighbours.
        let n = [30f64.to_radians().sin(), 30f64.to_radians().cos()];
        let mut cs: Vec<f64> = f.iter().map(|[a, _]| a[0] * n[0] + a[1] * n[1]).collect();
        cs.sort_by(f64::total_cmp);
        assert!(cs.windows(2).all(|w| (w[1] - w[0] - 20.0).abs() < 1e-6), "{cs:?}");
        let v = verticals(100.0, 50.0, 25.0, 0.0);
        assert_eq!(v.len(), 3);
        assert_eq!(v[0], [[25.0, 0.0], [25.0, 50.0]]);
        let all = lines(100.0, 100.0, 20.0, 30.0, 30.0, true, [0.0, 0.0]);
        assert!(all.len() > f.len() + 3);
        assert!(family(100.0, 100.0, 30.0, 0.0, 0.0).is_empty());
    }
}
