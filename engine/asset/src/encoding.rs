//! Compact encodings of vectors and colours used in the formats.

use glam::{Vec2, Vec3};

/// A unit vector folded onto an octahedron and stored as two snorm16 values.
pub fn oct_encode(n: Vec3) -> [i16; 2] {
    let n = n / (n.x.abs() + n.y.abs() + n.z.abs()).max(1e-20);
    let mut p = Vec2::new(n.x, n.y);
    if n.z < 0.0 {
        p = (1.0 - Vec2::new(p.y.abs(), p.x.abs())) * Vec2::new(p.x.signum(), p.y.signum());
    }
    p.to_array().map(snorm16)
}

pub fn oct_decode(e: [i16; 2]) -> Vec3 {
    let p = Vec2::new(e[0] as f32, e[1] as f32) / 32767.0;
    let z = 1.0 - p.x.abs() - p.y.abs();
    let t = (-z).max(0.0);
    let xy = p - Vec2::new(p.x.signum(), p.y.signum()) * t;
    Vec3::new(xy.x, xy.y, z).normalize()
}

pub fn snorm16(v: f32) -> i16 {
    (v.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

/// RGB with a shared exponent in 32 bits (the GPU's `RGB9E5` format): HDR
/// colours at a quarter of the size of three floats.
pub fn rgb9e5(color: [f32; 3]) -> u32 {
    const MANTISSA: i32 = 9;
    const BIAS: i32 = 15;
    const MAX: f32 = 65408.0;
    let c = color.map(|v| v.clamp(0.0, MAX));
    let max = c[0].max(c[1]).max(c[2]);
    if max <= 0.0 {
        return 0;
    }
    let mut exponent = (max.log2().floor() as i32).max(-BIAS - 1) + 1 + BIAS;
    let mut scale = 2f32.powi(exponent - BIAS - MANTISSA);
    if (max / scale).round() as i32 == 1 << MANTISSA {
        exponent += 1;
        scale *= 2.0;
    }
    let [r, g, b] = c.map(|v| ((v / scale).round() as u32).min(511));
    r | (g << 9) | (b << 18) | ((exponent as u32) << 27)
}

/// The colour of [`rgb9e5`] bits.
pub fn rgb9e5_decode(v: u32) -> [f32; 3] {
    let scale = 2f32.powi((v >> 27) as i32 - 24);
    [v & 511, (v >> 9) & 511, (v >> 18) & 511].map(|m| m as f32 * scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn octahedral_round_trip() {
        for n in [Vec3::X, -Vec3::Y, Vec3::Z, -Vec3::Z, Vec3::new(0.3, -0.5, -0.8).normalize(), Vec3::new(-0.7, 0.1, 0.2).normalize()] {
            let decoded = oct_decode(oct_encode(n));
            assert!(decoded.abs_diff_eq(n, 1e-3), "{n} -> {decoded}");
        }
    }

    #[test]
    fn rgb9e5_round_trip() {
        for color in [[0.0; 3], [1.0, 0.5, 0.25], [12.0, 0.01, 3.0], [0.002, 0.002, 0.004]] {
            let back = rgb9e5_decode(rgb9e5(color));
            for k in 0..3 {
                let max = color.iter().copied().fold(0.0, f32::max);
                assert!((back[k] - color[k]).abs() <= max / 256.0 + 1e-6, "{color:?} -> {back:?}");
            }
        }
    }
}
