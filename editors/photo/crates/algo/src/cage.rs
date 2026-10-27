//! Cage Transform: a closed polygon (the cage) drawn around part of the image, whose vertices are
//! then moved; the pixels inside follow by mean value coordinates (M. S. Floater, "Mean value
//! coordinates", CAGD 2003; K. Hormann and M. S. Floater, "Mean value coordinates for arbitrary
//! planar polygons", ACM TOG 2006), so the interior deforms smoothly and the cage edges map onto
//! the moved edges. Pixels outside the cage stay where they are.

/// A cage: its vertices before (`src`) and after (`dst`) the edit, same count and order.
#[derive(Clone, Debug, PartialEq)]
pub struct Cage {
    pub src: Vec<[f64; 2]>,
    pub dst: Vec<[f64; 2]>,
}

/// Mean value coordinates of `p` with respect to the closed polygon `poly` (Hormann & Floater's
/// robust form). On a vertex the weight is 1 there; on an edge the two end weights interpolate.
pub fn mean_value_coordinates(poly: &[[f64; 2]], p: [f64; 2]) -> Vec<f64> {
    let n = poly.len();
    let mut w = vec![0.0; n];
    if n == 0 {
        return w;
    }
    let s: Vec<[f64; 2]> = poly.iter().map(|v| [v[0] - p[0], v[1] - p[1]]).collect();
    let r: Vec<f64> = s.iter().map(|v| (v[0] * v[0] + v[1] * v[1]).sqrt()).collect();
    const EPS: f64 = 1e-9;
    for i in 0..n {
        if r[i] < EPS {
            w[i] = 1.0;
            return w;
        }
    }
    let mut a = vec![0.0; n];
    let mut d = vec![0.0; n];
    for i in 0..n {
        let j = (i + 1) % n;
        a[i] = 0.5 * (s[i][0] * s[j][1] - s[j][0] * s[i][1]);
        d[i] = s[i][0] * s[j][0] + s[i][1] * s[j][1];
    }
    for i in 0..n {
        let j = (i + 1) % n;
        if a[i].abs() < EPS && d[i] < 0.0 {
            // On the edge i–j: linear between its ends.
            let t = r[i] / (r[i] + r[j]);
            w[i] = 1.0 - t;
            w[j] = t;
            return w;
        }
    }
    let mut total = 0.0;
    for i in 0..n {
        let prev = (i + n - 1) % n;
        let mut wi = 0.0;
        if a[prev].abs() > EPS {
            wi += (r[prev] - d[prev] / r[i]) / a[prev];
        }
        if a[i].abs() > EPS {
            wi += (r[(i + 1) % n] - d[i] / r[i]) / a[i];
        }
        w[i] = wi;
        total += wi;
    }
    if total.abs() > EPS {
        for v in &mut w {
            *v /= total;
        }
    }
    w
}

/// Even-odd point-in-polygon test.
fn inside(poly: &[[f64; 2]], p: [f64; 2]) -> bool {
    let n = poly.len();
    let mut inside = false;
    let mut j = n.saturating_sub(1);
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
        j = i;
    }
    inside
}

impl Cage {
    /// A cage of at least three vertices whose two rings match and are finite.
    pub fn new(src: Vec<[f64; 2]>, dst: Vec<[f64; 2]>) -> Option<Cage> {
        if src.len() < 3 || src.len() != dst.len() || src.iter().chain(&dst).flatten().any(|v| !v.is_finite()) {
            return None;
        }
        Some(Cage { src, dst })
    }

    /// Nothing moves.
    pub fn is_identity(&self) -> bool {
        self.src.iter().zip(&self.dst).all(|(a, b)| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9)
    }

    /// Forward map: where the source document point `(x, y)` lands. Points outside the cage stay.
    pub fn map(&self, x: f64, y: f64) -> (f64, f64) {
        let p = [x, y];
        if !inside(&self.src, p) {
            return (x, y);
        }
        let w = mean_value_coordinates(&self.src, p);
        let mut out = [0.0, 0.0];
        for (wi, d) in w.iter().zip(&self.dst) {
            out[0] += wi * d[0];
            out[1] += wi * d[1];
        }
        if out.iter().all(|v| v.is_finite()) { (out[0], out[1]) } else { (x, y) }
    }

    /// Bounds (x0, y0, x1, y1) of the moved cage, for the warped area.
    pub fn dst_bounds(&self) -> [f64; 4] {
        let mut b = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
        for v in &self.dst {
            b[0] = b[0].min(v[0]);
            b[1] = b[1].min(v[1]);
            b[2] = b[2].max(v[0]);
            b[3] = b[3].max(v[1]);
        }
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinates_partition_unity_and_reproduce_vertices() {
        let sq = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        for p in [[5.0, 5.0], [1.0, 2.0], [9.5, 0.5]] {
            let w = mean_value_coordinates(&sq, p);
            assert!((w.iter().sum::<f64>() - 1.0).abs() < 1e-9, "{w:?}");
            let (mut x, mut y) = (0.0, 0.0);
            for (wi, v) in w.iter().zip(&sq) {
                x += wi * v[0];
                y += wi * v[1];
            }
            assert!((x - p[0]).abs() < 1e-6 && (y - p[1]).abs() < 1e-6, "{p:?} -> {x} {y}");
        }
        let w = mean_value_coordinates(&sq, [10.0, 10.0]);
        assert_eq!(w, vec![0.0, 0.0, 1.0, 0.0]);
        let w = mean_value_coordinates(&sq, [5.0, 0.0]);
        assert!((w[0] - 0.5).abs() < 1e-9 && (w[1] - 0.5).abs() < 1e-9 && w[2] == 0.0 && w[3] == 0.0, "{w:?}");
    }

    #[test]
    fn identity_cage_keeps_points_and_a_moved_vertex_pulls_the_inside() {
        let sq = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let c = Cage::new(sq.clone(), sq.clone()).unwrap();
        assert!(c.is_identity());
        assert_eq!(c.map(3.0, 4.0), (3.0, 4.0));
        let mut dst = sq.clone();
        dst[2] = [14.0, 14.0];
        let c = Cage::new(sq, dst).unwrap();
        assert!(!c.is_identity());
        let (x, y) = c.map(9.0, 9.0);
        assert!(x > 9.0 && y > 9.0, "{x} {y}");
        // Outside the cage nothing moves.
        assert_eq!(c.map(-1.0, 5.0), (-1.0, 5.0));
        assert!(Cage::new(vec![[0.0, 0.0], [1.0, 0.0]], vec![[0.0, 0.0], [1.0, 0.0]]).is_none());
    }
}
