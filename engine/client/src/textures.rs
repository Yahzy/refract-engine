//! Textures generated in code, for levels built without content: a grid
//! that makes distances and speed readable, and the matching tile relief.

use glam::{Vec2, Vec3};
use refract_asset::{TextureData, TextureFormat};

/// Texels per side.
const SIZE: usize = 512;
/// The grid has `TILES` tiles per side, each `LINES` cells wide.
const TILES: usize = 4;
const LINES: usize = 4;

/// Light grey with fine lines between cells and darker ones between tiles;
/// tint it with the material's base colour. sRGB.
pub fn grid() -> TextureData<'static> {
    let tile = SIZE / TILES;
    let cell = tile / LINES;
    let mut pixels = Vec::with_capacity(SIZE * SIZE * 4);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let edge = |v: usize, period: usize| {
                let d = v % period;
                d.min(period - 1 - d)
            };
            let mut value = 0.82f32;
            if edge(x, cell) == 0 || edge(y, cell) == 0 {
                value = 0.7;
            }
            if edge(x, tile) <= 1 || edge(y, tile) <= 1 {
                value = 0.45;
            }
            // A faint checker of tiles, so that motion reads even far away.
            if ((x / tile) + (y / tile)) % 2 == 1 {
                value *= 0.93;
            }
            let byte = (value * 255.0).round() as u8;
            pixels.extend_from_slice(&[byte, byte, byte, 255]);
        }
    }
    with_mips(TextureFormat::Rgba8Srgb, pixels, |texels| {
        let mut sum = [0u32; 4];
        for t in texels {
            for k in 0..4 {
                sum[k] += t[k] as u32;
            }
        }
        sum.map(|c| ((c + 2) / 4) as u8)
    })
}

/// Tangent-space normals (green along +V) of tiles separated by bevelled
/// grooves, aligned with [`grid`]'s tiles.
pub fn tile_normals() -> TextureData<'static> {
    let tile = (SIZE / TILES) as f32;
    // Bevel width and depth, in texels.
    const BEVEL: f32 = 3.0;
    const DEPTH: f32 = 2.0;
    let height = |x: f32, y: f32| {
        let edge = |v: f32| {
            let d = v.rem_euclid(tile);
            d.min(tile - d)
        };
        (edge(x).min(edge(y)) / BEVEL).min(1.0) * DEPTH
    };
    let mut pixels = Vec::with_capacity(SIZE * SIZE * 4);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let du = (height(fx + 1.0, fy) - height(fx - 1.0, fy)) * 0.5;
            let dv = (height(fx, fy + 1.0) - height(fx, fy - 1.0)) * 0.5;
            pixels.extend_from_slice(&encode_normal(Vec3::new(-du, -dv, 1.0).normalize()));
        }
    }
    with_mips(TextureFormat::Rgba8, pixels, |texels| {
        let sum: Vec3 = texels.iter().map(|t| decode_normal(*t)).sum();
        encode_normal(sum.normalize_or(Vec3::Z))
    })
}

fn encode_normal(n: Vec3) -> [u8; 4] {
    let c = (n * 0.5 + 0.5) * 255.0;
    [c.x.round() as u8, c.y.round() as u8, c.z.round() as u8, 255]
}

fn decode_normal(t: [u8; 4]) -> Vec3 {
    Vec3::new(t[0] as f32, t[1] as f32, t[2] as f32) / 255.0 * 2.0 - 1.0
}

/// The full mip chain of a square RGBA8 image, each level made by `reduce`
/// from 2x2 texels of the one above.
fn with_mips(format: TextureFormat, pixels: Vec<u8>, reduce: impl Fn([[u8; 4]; 4]) -> [u8; 4]) -> TextureData<'static> {
    let mut mips = vec![pixels];
    let mut size = SIZE;
    while size > 1 {
        let above = mips.last().unwrap();
        let half = size / 2;
        let mut level = Vec::with_capacity(half * half * 4);
        for y in 0..half {
            for x in 0..half {
                let at = |p: Vec2| {
                    let i = ((p.y as usize) * size + p.x as usize) * 4;
                    [above[i], above[i + 1], above[i + 2], above[i + 3]]
                };
                let (x2, y2) = ((x * 2) as f32, (y * 2) as f32);
                level.extend_from_slice(&reduce([at(Vec2::new(x2, y2)), at(Vec2::new(x2 + 1.0, y2)), at(Vec2::new(x2, y2 + 1.0)), at(Vec2::new(x2 + 1.0, y2 + 1.0))]));
            }
        }
        mips.push(level);
        size = half;
    }
    TextureData { format, width: SIZE as u32, height: SIZE as u32, layers: 1, faces: 1, mips: mips.into_iter().map(Into::into).collect() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_mip_chains() {
        for texture in [grid(), tile_normals()] {
            assert_eq!(texture.mips.len(), 10);
            for (level, mip) in texture.mips.iter().enumerate() {
                let (w, h) = texture.mip_size(level);
                assert_eq!(mip.len(), (w * h * 4) as usize);
            }
        }
    }

    #[test]
    fn flat_tiles_face_straight_out() {
        let normals = tile_normals();
        let center = (SIZE / TILES / 2) * (SIZE + 1) * 4;
        let n = decode_normal([normals.mips[0][center], normals.mips[0][center + 1], normals.mips[0][center + 2], 255]);
        assert!(n.abs_diff_eq(Vec3::Z, 0.01), "{n}");
    }
}
