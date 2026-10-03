// SPDX-License-Identifier: GPL-3.0-or-later

//! Colour management: the working-space conversions a colour-managed editor needs, re-derived from
//! the maths in Krita's pigment and GIMP's babl (behaviour studied, no code copied). Full ICC profile
//! parsing and a general CMM are a later pass; this is the matrix/transfer core they are built on:
//! sRGB <-> linear, linear sRGB <-> CIE XYZ (D65), XYZ <-> CIE Lab, and Bradford chromatic adaptation
//! between white points. With these a document can convert between the standard RGB working spaces
//! and do a perceptual Lab round-trip, which is what the colour tools actually call.

/// The sRGB electro-optical transfer: encoded 0..1 -> linear 0..1.
pub fn srgb_to_linear(c: f64) -> f64 {
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// The inverse: linear 0..1 -> sRGB-encoded 0..1.
pub fn linear_to_srgb(c: f64) -> f64 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

/// Linear sRGB (D65) to CIE XYZ. Row-major 3x3, the standard sRGB primaries.
pub fn linear_srgb_to_xyz(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    (
        0.412_456_4 * r + 0.357_576_1 * g + 0.180_437_5 * b,
        0.212_672_9 * r + 0.715_152_2 * g + 0.072_175_0 * b,
        0.019_333_9 * r + 0.119_192_0 * g + 0.950_304_1 * b,
    )
}

/// CIE XYZ to linear sRGB (D65), the inverse of [`linear_srgb_to_xyz`].
pub fn xyz_to_linear_srgb(x: f64, y: f64, z: f64) -> (f64, f64, f64) {
    (
        3.240_454_2 * x - 1.537_138_5 * y - 0.498_531_4 * z,
        -0.969_266_0 * x + 1.876_010_8 * y + 0.041_556_0 * z,
        0.055_643_4 * x - 0.204_025_9 * y + 1.057_225_2 * z,
    )
}

/// The D65 reference white in XYZ (Y normalised to 1).
pub const D65: (f64, f64, f64) = (0.950_47, 1.0, 1.088_83);
/// The D50 reference white (the ICC connection-space white).
pub const D50: (f64, f64, f64) = (0.964_22, 1.0, 0.825_21);

fn lab_f(t: f64) -> f64 {
    const DELTA: f64 = 6.0 / 29.0;
    if t > DELTA * DELTA * DELTA {
        t.cbrt()
    } else {
        t / (3.0 * DELTA * DELTA) + 4.0 / 29.0
    }
}

fn lab_f_inv(t: f64) -> f64 {
    const DELTA: f64 = 6.0 / 29.0;
    if t > DELTA {
        t * t * t
    } else {
        3.0 * DELTA * DELTA * (t - 4.0 / 29.0)
    }
}

/// CIE XYZ to CIE Lab under reference white `white`.
pub fn xyz_to_lab(x: f64, y: f64, z: f64, white: (f64, f64, f64)) -> (f64, f64, f64) {
    let fx = lab_f(x / white.0);
    let fy = lab_f(y / white.1);
    let fz = lab_f(z / white.2);
    (116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz))
}

/// CIE Lab back to XYZ under reference white `white`.
pub fn lab_to_xyz(l: f64, a: f64, b: f64, white: (f64, f64, f64)) -> (f64, f64, f64) {
    let fy = (l + 16.0) / 116.0;
    let fx = fy + a / 500.0;
    let fz = fy - b / 200.0;
    (
        white.0 * lab_f_inv(fx),
        white.1 * lab_f_inv(fy),
        white.2 * lab_f_inv(fz),
    )
}

/// Bradford chromatic adaptation: adapt an XYZ colour measured under `src_white` to appear correct
/// under `dst_white`. This is the white-point transform ICC uses to convert between D50 and D65
/// working spaces. Returns the 3x3 adaptation matrix (row-major) to multiply an XYZ vector by.
pub fn bradford_adaptation(src_white: (f64, f64, f64), dst_white: (f64, f64, f64)) -> [f64; 9] {
    // The Bradford cone-response matrix and its inverse.
    const M: [f64; 9] = [
        0.895_1, 0.266_4, -0.161_4, -0.750_2, 1.713_5, 0.036_7, 0.038_9, -0.068_5, 1.029_6,
    ];
    const M_INV: [f64; 9] = [
        0.986_993, -0.147_054, 0.159_963, 0.432_305, 0.518_360, 0.049_291, -0.008_529, 0.040_043,
        0.968_487,
    ];
    let mul = |m: &[f64; 9], v: (f64, f64, f64)| {
        (
            m[0] * v.0 + m[1] * v.1 + m[2] * v.2,
            m[3] * v.0 + m[4] * v.1 + m[5] * v.2,
            m[6] * v.0 + m[7] * v.1 + m[8] * v.2,
        )
    };
    let s = mul(&M, src_white);
    let d = mul(&M, dst_white);
    // Diagonal scaling in cone space.
    let diag = [d.0 / s.0, d.1 / s.1, d.2 / s.2];
    // Result = M_INV * diag(d/s) * M.
    let scaled = [
        diag[0] * M[0],
        diag[0] * M[1],
        diag[0] * M[2],
        diag[1] * M[3],
        diag[1] * M[4],
        diag[1] * M[5],
        diag[2] * M[6],
        diag[2] * M[7],
        diag[2] * M[8],
    ];
    // M_INV (3x3) * scaled (3x3).
    let mut out = [0.0_f64; 9];
    for row in 0..3 {
        for col in 0..3 {
            out[row * 3 + col] = M_INV[row * 3] * scaled[col]
                + M_INV[row * 3 + 1] * scaled[3 + col]
                + M_INV[row * 3 + 2] * scaled[6 + col];
        }
    }
    out
}

/// Convenience: an 8-bit sRGB pixel to CIE Lab (D65), for perceptual colour tools.
pub fn srgb8_to_lab(r: u8, g: u8, b: u8) -> (f64, f64, f64) {
    let lr = srgb_to_linear(f64::from(r) / 255.0);
    let lg = srgb_to_linear(f64::from(g) / 255.0);
    let lb = srgb_to_linear(f64::from(b) / 255.0);
    let (x, y, z) = linear_srgb_to_xyz(lr, lg, lb);
    xyz_to_lab(x, y, z, D65)
}

/// Convenience: CIE Lab (D65) back to an 8-bit sRGB pixel, clamped to the gamut.
pub fn lab_to_srgb8(l: f64, a: f64, b: f64) -> (u8, u8, u8) {
    let (x, y, z) = lab_to_xyz(l, a, b, D65);
    let (lr, lg, lb) = xyz_to_linear_srgb(x, y, z);
    let enc = |c: f64| {
        (linear_to_srgb(c.clamp(0.0, 1.0)) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    (enc(lr), enc(lg), enc(lb))
}
