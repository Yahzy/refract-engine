//! The map's indirect light: a grid of probes over the playable world, and
//! reflection cubemaps where the map placed `env_cubemap`s.
//!
//! Baked on the GPU when it can trace rays: the world and its opaque props,
//! lit by the sun, the sky and every light, with a few bounces. Otherwise the
//! map's own per-leaf ambient cubes are resampled into the grid, and there
//! are no reflections.

use glam::{Mat4, UVec3, Vec3};
use refract_asset::encoding::rgb9e5;
use refract_asset::map::{MapData, Probe, ProbeGrid, chunk_flags, prop_flags, probe_flags};
use refract_asset::material::{Material, MaterialKind};
use refract_asset::{MappedFile, ModelFile, TextureData, TextureFormat};
use refract_core::Angles;
use refract_render::Sky;
use refract_render::gi::bake::{self, BakeInput};
use refract_formats::ambient::AmbientCube;
use refract_formats::bsp::Map;

use crate::{Converter, parallel_map};

const MIN_CELL: f32 = 64.0;
const MAX_PROBES: f32 = 2_000_000.0;
const BOUNCES: u32 = 3;
/// Albedo of hidden brush faces, which only block light.
const HIDDEN_ALBEDO: [f32; 3] = [0.3; 3];
/// Distance recorded for every direction of probes without visibility data.
const FAR: f32 = 2048.0;
/// Passes spreading open cells into closed ones (resampled ambient cubes).
const DILATE_PASSES: usize = 3;
/// Texels per side of a reflection cubemap face.
const REFLECTION_SIZE: u32 = 64;

/// The light baked for a map.
#[derive(Default)]
pub struct BakedLight {
    pub probes: Option<(ProbeGrid, Vec<Probe>)>,
    /// Positions of the reflection cubemaps, and their native path.
    pub reflections: Option<(Vec<[f32; 4]>, String)>,
}

impl Converter {
    /// `sky` is the light the sky casts; `sky_cubemap` the sky's native path.
    pub(crate) fn light(&self, map: &Map, data: &MapData, sky: Sky, sky_cubemap: Option<&str>) -> BakedLight {
        let Some(grid) = grid_over(data) else { return BakedLight::default() };
        if bake::available() {
            match self.bake(map, data, grid, sky, sky_cubemap) {
                Ok(light) => return light,
                Err(err) => log::warn!("baking light failed, using the map's ambient cubes: {err:#}"),
            }
        } else {
            log::warn!("no GPU able to trace rays: using the map's ambient cubes");
        }
        let probes = map.ambient.as_ref().map(|_| (grid, resample(map, grid)));
        BakedLight { probes, reflections: None }
    }

    fn bake(&self, map: &Map, data: &MapData, grid: ProbeGrid, sky: Sky, sky_cubemap: Option<&str>) -> anyhow::Result<BakedLight> {
        let start = std::time::Instant::now();
        let mut scene = BakeScene::default();
        let geometry = &data.geometry;
        for submesh in &geometry.submeshes {
            if submesh.flags & (chunk_flags::SKYBOX | chunk_flags::OVERLAY) != 0 {
                continue;
            }
            let albedo = match geometry.materials[submesh.material as usize].as_str() {
                "" => HIDDEN_ALBEDO,
                path => match self.bake_albedo(path) {
                    Some(albedo) => albedo,
                    None => continue,
                },
            };
            let range = submesh.first_index as usize..(submesh.first_index + submesh.index_count) as usize;
            scene.add(&geometry.positions, &geometry.indices[range], Mat4::IDENTITY, albedo);
        }
        for prop in data.props.iter().filter(|p| p.flags & prop_flags::SKYBOX == 0) {
            let path = &data.models[prop.model as usize];
            let Ok(file) = MappedFile::open(&self.out.join(path)) else { continue };
            let Ok(model) = ModelFile::read(file.bytes()) else { continue };
            let model = &model.geometry;
            let materials: Vec<&str> = model.materials().collect();
            let [pitch, yaw, roll] = prop.angles;
            let placement = Mat4::from_scale_rotation_translation(Vec3::splat(prop.scale), Angles::new(pitch, yaw, roll).to_quat(), prop.origin.into());
            for submesh in model.submeshes {
                if let Some(albedo) = self.bake_albedo(materials[submesh.material as usize]) {
                    let range = submesh.first_index as usize..(submesh.first_index + submesh.index_count) as usize;
                    scene.add(model.positions, &model.indices[range], placement, albedo);
                }
            }
        }

        let sun = data.info.sun.as_ref().map(|s| (Vec3::from(s.direction), Vec3::from(s.color) * s.intensity));
        let reflection_positions: Vec<Vec3> = map.cubemaps.iter().map(|c| Vec3::from(c.origin.map(|v| v as f32))).collect();
        let sky_file = sky_cubemap.and_then(|path| MappedFile::open(&self.out.join(path)).ok());
        let sky_texture = sky_file.as_ref().and_then(|file| TextureData::read(file.bytes()).ok());
        let input = BakeInput {
            positions: &scene.positions,
            indices: &scene.indices,
            albedo: &scene.albedo,
            sun,
            sky,
            lights: &data.lights,
            grid,
            bounces: BOUNCES,
            reflections: &reflection_positions,
            reflection_size: REFLECTION_SIZE,
            sky_cubemap: sky_texture.as_ref(),
        };
        let baked = bake::bake(&input)?;
        let valid = baked.probes.iter().filter(|p| p.flags & probe_flags::VALID != 0).count();
        log::info!(
            "baked {} probes ({valid} in open space) and {} reflections over {} triangles in {:.1} s",
            baked.probes.len(),
            reflection_positions.len(),
            scene.albedo.len(),
            start.elapsed().as_secs_f32()
        );

        let reflections = if reflection_positions.is_empty() {
            None
        } else {
            let path = format!("maps/{}.reflections.ktx2", data.info.name.to_ascii_lowercase());
            self.write(&path, &cube_array(&baked.reflections, reflection_positions.len() as u32).write())?;
            Some((reflection_positions.iter().map(|p| p.extend(0.0).to_array()).collect(), path))
        };
        Ok(BakedLight { probes: Some((grid, baked.probes)), reflections })
    }

    /// What a material reflects, for the bake; `None` for materials light goes through.
    fn bake_albedo(&self, path: &str) -> Option<[f32; 3]> {
        let text = std::fs::read_to_string(self.out.join(path)).ok()?;
        let material = Material::parse(&text).ok()?;
        matches!(material.kind, MaterialKind::Opaque | MaterialKind::AlphaTest).then(|| material.albedo.unwrap_or([0.5; 3]))
    }
}

/// Baked reflection texels as a cube array with every mip (each a 2x2 box
/// filter of the one above), half floats.
fn cube_array(texels: &[[f32; 4]], count: u32) -> TextureData<'static> {
    let mut level: Vec<[f32; 4]> = texels.to_vec();
    let mut size = REFLECTION_SIZE;
    let mut mips = Vec::new();
    loop {
        mips.push(std::borrow::Cow::Owned(level.iter().flat_map(|t| t.map(|c| f16_bits(c).to_le_bytes())).flatten().collect::<Vec<u8>>()));
        if size == 1 {
            break;
        }
        let half = size / 2;
        let mut next = Vec::with_capacity((count * 6 * half * half) as usize);
        for image in level.chunks_exact((size * size) as usize) {
            for y in 0..half {
                for x in 0..half {
                    let at = |dx: u32, dy: u32| image[((y * 2 + dy) * size + x * 2 + dx) as usize];
                    let (a, b, c, d) = (at(0, 0), at(1, 0), at(0, 1), at(1, 1));
                    next.push(std::array::from_fn(|k| (a[k] + b[k] + c[k] + d[k]) * 0.25));
                }
            }
        }
        level = next;
        size = half;
    }
    TextureData { format: TextureFormat::Rgba16Float, width: REFLECTION_SIZE, height: REFLECTION_SIZE, layers: count, faces: 6, mips }
}

/// Triangles for the bake, in world space, with one albedo each.
#[derive(Default)]
struct BakeScene {
    positions: Vec<[f32; 3]>,
    indices: Vec<u32>,
    albedo: Vec<[f32; 3]>,
}

impl BakeScene {
    fn add(&mut self, positions: &[[f32; 3]], indices: &[u32], transform: Mat4, albedo: [f32; 3]) {
        // Only the vertices these triangles use.
        let mut remap = std::collections::HashMap::new();
        for &index in indices {
            let next = self.positions.len() as u32;
            let new = *remap.entry(index).or_insert_with(|| {
                self.positions.push(transform.transform_point3(Vec3::from(positions[index as usize])).to_array());
                next
            });
            self.indices.push(new);
        }
        self.albedo.extend(std::iter::repeat_n(albedo, indices.len() / 3));
    }
}

/// A grid over the playable world's geometry, as fine as the probe budget allows.
fn grid_over(data: &MapData) -> Option<ProbeGrid> {
    let geometry = &data.geometry;
    let mut points = geometry
        .submeshes
        .iter()
        .filter(|s| s.flags & (chunk_flags::SKYBOX | chunk_flags::OVERLAY) == 0)
        .flat_map(|s| geometry.indices[s.first_index as usize..(s.first_index + s.index_count) as usize].iter())
        .map(|&i| Vec3::from(geometry.positions[i as usize]));
    let first = points.next()?;
    let (min, max) = points.fold((first, first), |(min, max), p| (min.min(p), max.max(p)));
    let size = (max - min).max(Vec3::splat(1.0));
    let cell = (size.x * size.y * size.z / MAX_PROBES).cbrt().max(MIN_CELL);
    let resolution = (size / cell).ceil().as_uvec3().max(UVec3::ONE);
    let origin = (min + max) * 0.5 - resolution.as_vec3() * cell * 0.5;
    Some(ProbeGrid { origin: origin.to_array(), cell_size: cell, resolution: resolution.to_array(), _pad: 0 })
}

/// Probes from the map's own ambient cubes, without visibility data. Cells
/// inside walls take the light of their open neighbours.
fn resample(map: &Map, grid: ProbeGrid) -> Vec<Probe> {
    let resolution = UVec3::from(grid.resolution);
    let slices: Vec<u32> = (0..resolution.z).collect();
    let cells: Vec<Option<AmbientCube>> = parallel_map(&slices, |&z| {
        let mut slice = Vec::with_capacity((resolution.x * resolution.y) as usize);
        for y in 0..resolution.y {
            for x in 0..resolution.x {
                let center = Vec3::from(grid.origin) + (UVec3::new(x, y, z).as_vec3() + 0.5) * grid.cell_size;
                slice.push(map.ambient_at(center.to_array()));
            }
        }
        slice
    })
    .into_iter()
    .flatten()
    .collect();
    let far = f16_bits(FAR);
    dilate(cells, resolution)
        .into_iter()
        .map(|cube| match cube {
            Some(cube) => Probe { light: cube.map(rgb9e5), distance: [far; 6], flags: probe_flags::VALID },
            None => Probe::default(),
        })
        .collect()
}

/// `value` as half-float bits (positive, finite values).
fn f16_bits(value: f32) -> u16 {
    let bits = value.to_bits();
    let exponent = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    if exponent <= 0 {
        return 0;
    }
    if exponent >= 31 {
        return 0x7bff;
    }
    ((exponent as u16) << 10) | ((bits >> 13) & 0x3ff) as u16
}

/// Fills empty cells with the average of their filled neighbours, a few times over.
fn dilate(mut cells: Vec<Option<AmbientCube>>, resolution: UVec3) -> Vec<Option<AmbientCube>> {
    let r = resolution.as_ivec3();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dilation_fills_neighbours_only() {
        let lit: AmbientCube = [[1.0; 3]; 6];
        let cells = vec![Some(lit), None, None, None, None];
        let filled = dilate(cells, UVec3::new(5, 1, 1));
        assert!(filled[1].is_some() && filled[3].is_some());
        assert!(filled[4].is_none(), "beyond {DILATE_PASSES} passes");
    }

    #[test]
    fn half_floats() {
        assert_eq!(f16_bits(2048.0), 0x6800);
        assert_eq!(f16_bits(1.0), 0x3c00);
    }
}
