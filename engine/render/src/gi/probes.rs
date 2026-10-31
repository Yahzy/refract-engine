//! Helpers around baked light: sizing a probe grid, filling probes inside
//! walls, and packing reflection cubemaps.

use std::borrow::Cow;

use glam::{UVec3, Vec3};
use refract_asset::encoding::{f16_bits, rgb9e5, rgb9e5_decode};
use refract_asset::map::{Probe, ProbeGrid, probe_flags};
use refract_asset::{TextureData, TextureFormat};

/// Light arriving from +X, -X, +Y, -Y, +Z and -Z, linear RGB.
pub type LightCube = [[f32; 3]; 6];

/// Passes spreading open cells into closed ones.
const DILATE_PASSES: usize = 3;

/// A grid covering `min..max` with cells of at least `min_cell`, and at most
/// about `max_probes` probes.
pub fn grid_around(min: Vec3, max: Vec3, min_cell: f32, max_probes: f32) -> ProbeGrid {
    let size = (max - min).max(Vec3::splat(1.0));
    let cell = (size.x * size.y * size.z / max_probes).cbrt().max(min_cell);
    let resolution = (size / cell).ceil().as_uvec3().max(UVec3::ONE);
    let origin = (min + max) * 0.5 - resolution.as_vec3() * cell * 0.5;
    ProbeGrid { origin: origin.to_array(), cell_size: cell, resolution: resolution.to_array(), _pad: 0 }
}

/// Gives probes inside walls the light of their open neighbours. They stay
/// marked closed: surfaces only use them where no open probe is near (thin
/// gaps, small rooms), which would otherwise have no light at all.
pub fn fill_closed(probes: &mut [Probe], resolution: [u32; 3]) {
    let cells = probes.iter().map(|p| (p.flags & probe_flags::VALID != 0).then(|| p.light.map(rgb9e5_decode))).collect();
    for (probe, cube) in probes.iter_mut().zip(dilate(cells, resolution)) {
        if let Some(cube) = cube
            && probe.flags & probe_flags::VALID == 0
        {
            probe.light = cube.map(rgb9e5);
        }
    }
}

/// Fills empty cells with the average of their filled neighbours, a few times over.
pub fn dilate(mut cells: Vec<Option<LightCube>>, resolution: [u32; 3]) -> Vec<Option<LightCube>> {
    let r = UVec3::from(resolution).as_ivec3();
    let index = |x: i32, y: i32, z: i32| (x + r.x * (y + r.y * z)) as usize;
    for _ in 0..DILATE_PASSES {
        let previous = cells.clone();
        for z in 0..r.z {
            for y in 0..r.y {
                for x in 0..r.x {
                    if previous[index(x, y, z)].is_some() {
                        continue;
                    }
                    let mut sum = [[0.0f32; 3]; 6];
                    let mut count = 0.0;
                    for (dx, dy, dz) in [(1, 0, 0), (-1, 0, 0), (0, 1, 0), (0, -1, 0), (0, 0, 1), (0, 0, -1)] {
                        let (nx, ny, nz) = (x + dx, y + dy, z + dz);
                        if nx < 0 || ny < 0 || nz < 0 || nx >= r.x || ny >= r.y || nz >= r.z {
                            continue;
                        }
                        if let Some(cube) = previous[index(nx, ny, nz)] {
                            for face in 0..6 {
                                for k in 0..3 {
                                    sum[face][k] += cube[face][k];
                                }
                            }
                            count += 1.0;
                        }
                    }
                    if count > 0.0 {
                        cells[index(x, y, z)] = Some(sum.map(|face| face.map(|c| c / count)));
                    }
                }
            }
        }
    }
    cells
}

/// Baked reflection texels (as [`super::bake::Baked::reflections`]) as a
/// cube array of `count` cubemaps of `size`² faces, with every mip (each a
/// 2x2 box filter of the one above), half floats.
pub fn reflection_cubemaps(texels: &[[f32; 4]], count: u32, size: u32) -> TextureData<'static> {
    let mut level: Vec<[f32; 4]> = texels.to_vec();
    let mut level_size = size;
    let mut mips = Vec::new();
    loop {
        mips.push(Cow::Owned(level.iter().flat_map(|t| t.map(|c| f16_bits(c).to_le_bytes())).flatten().collect::<Vec<u8>>()));
        if level_size == 1 {
            break;
        }
        let half = level_size / 2;
        let mut next = Vec::with_capacity((count * 6 * half * half) as usize);
        for image in level.chunks_exact((level_size * level_size) as usize) {
            for y in 0..half {
                for x in 0..half {
                    let at = |dx: u32, dy: u32| image[((y * 2 + dy) * level_size + x * 2 + dx) as usize];
                    let (a, b, c, d) = (at(0, 0), at(1, 0), at(0, 1), at(1, 1));
                    next.push(std::array::from_fn(|k| (a[k] + b[k] + c[k] + d[k]) * 0.25));
                }
            }
        }
        level = next;
        level_size = half;
    }
    TextureData { format: TextureFormat::Rgba16Float, width: size, height: size, layers: count, faces: 6, mips }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dilation_fills_neighbours_only() {
        let lit: LightCube = [[1.0; 3]; 6];
        let cells = vec![Some(lit), None, None, None, None];
        let filled = dilate(cells, [5, 1, 1]);
        assert!(filled[1].is_some() && filled[3].is_some());
        assert!(filled[4].is_none(), "beyond {DILATE_PASSES} passes");
    }

    #[test]
    fn half_floats() {
        assert_eq!(f16_bits(2048.0), 0x6800);
        assert_eq!(f16_bits(1.0), 0x3c00);
    }

    #[test]
    fn grid_covers_the_bounds() {
        let grid = grid_around(Vec3::ZERO, Vec3::new(1000.0, 500.0, 100.0), 64.0, 1e6);
        assert_eq!(grid.cell_size, 64.0);
        assert_eq!(grid.resolution, [16, 8, 2]);
    }
}
