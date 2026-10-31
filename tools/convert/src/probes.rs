//! The map's indirect light: a grid of probes over the playable world, and
//! reflection cubemaps where the map placed `env_cubemap`s.
//!
//! Baked on the GPU when it can trace rays: the world and its opaque props,
//! lit by the sun, the sky and every light, with a few bounces. Otherwise the
//! map's own per-leaf ambient cubes are resampled into the grid, and there
//! are no reflections.

use glam::{Mat4, UVec3, Vec3};
use refract_asset::encoding::{f16_bits, rgb9e5};
use refract_asset::map::{MapData, Probe, ProbeGrid, chunk_flags, prop_flags, probe_flags};
use refract_asset::material::{Material, MaterialKind};
use refract_asset::{MappedFile, ModelFile, TextureData};
use refract_core::Angles;
use refract_render::Sky;
use refract_render::gi::bake::{self, BakeInput};
use refract_render::gi::probes;
use refract_formats::ambient::AmbientCube;
use refract_formats::bsp::Map;

use crate::{Converter, parallel_map};

const MIN_CELL: f32 = 64.0;
const MAX_PROBES: f32 = 2_000_000.0;
const BOUNCES: u32 = 3;
/// Albedo of hidden brush faces, which only block light.
const HIDDEN_ALBEDO: [f32; 4] = [0.3, 0.3, 0.3, 0.0];
/// Distance recorded for every direction of probes without visibility data.
const FAR: f32 = 2048.0;
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
        // The 3D skybox, placed around the world like the client draws it:
        // reflections and the sky's light see it.
        let skybox = data.info.sky_camera.as_ref().map(|c| Mat4::from_scale(Vec3::splat(c.scale)) * Mat4::from_translation(-Vec3::from(c.origin)));
        for submesh in &geometry.submeshes {
            let transform = match (submesh.flags & chunk_flags::SKYBOX != 0, skybox) {
                _ if submesh.flags & chunk_flags::OVERLAY != 0 => continue,
                (false, _) => Mat4::IDENTITY,
                (true, Some(sky)) => sky,
                (true, None) => continue,
            };
            let albedo = match geometry.materials[submesh.material as usize].as_str() {
                "" => HIDDEN_ALBEDO,
                path => match self.bake_albedo(path) {
                    Some(albedo) => albedo,
                    None => continue,
                },
            };
            let range = submesh.first_index as usize..(submesh.first_index + submesh.index_count) as usize;
            scene.add(&geometry.positions, &geometry.indices[range], transform, albedo);
        }
        for prop in &data.props {
            let in_skybox = prop.flags & prop_flags::SKYBOX != 0;
            if in_skybox && skybox.is_none() {
                continue;
            }
            let path = &data.models[prop.model as usize];
            let Ok(file) = MappedFile::open(&self.out.join(path)) else { continue };
            let Ok(model) = ModelFile::read(file.bytes()) else { continue };
            let model = &model.geometry;
            let materials: Vec<&str> = model.materials().collect();
            let [pitch, yaw, roll] = prop.angles;
            let mut placement = Mat4::from_scale_rotation_translation(Vec3::splat(prop.scale), Angles::new(pitch, yaw, roll).to_quat(), prop.origin.into());
            if in_skybox && let Some(sky) = skybox {
                placement = sky * placement;
            }
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
        let mut baked = bake::bake(&input)?;
        probes::fill_closed(&mut baked.probes, grid.resolution);
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
            self.write(&path, &probes::reflection_cubemaps(&baked.reflections, reflection_positions.len() as u32, REFLECTION_SIZE).write())?;
            Some((reflection_positions.iter().map(|p| p.extend(0.0).to_array()).collect(), path))
        };
        Ok(BakedLight { probes: Some((grid, baked.probes)), reflections })
    }

    /// What a material reflects, for the bake (alpha 1 for thin surfaces:
    /// two-sided or cut out, like leaves); `None` for materials light goes
    /// through.
    fn bake_albedo(&self, path: &str) -> Option<[f32; 4]> {
        let text = std::fs::read_to_string(self.out.join(path)).ok()?;
        let material = Material::parse(&text).ok()?;
        if !matches!(material.kind, MaterialKind::Opaque | MaterialKind::AlphaTest) {
            return None;
        }
        let [r, g, b] = material.albedo.unwrap_or([0.5; 3]);
        let thin = material.two_sided || material.kind == MaterialKind::AlphaTest;
        Some([r, g, b, if thin { 1.0 } else { 0.0 }])
    }
}

/// Triangles for the bake, in world space, with one albedo each.
#[derive(Default)]
struct BakeScene {
    positions: Vec<[f32; 3]>,
    indices: Vec<u32>,
    albedo: Vec<[f32; 4]>,
}

impl BakeScene {
    fn add(&mut self, positions: &[[f32; 3]], indices: &[u32], transform: Mat4, albedo: [f32; 4]) {
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
    Some(probes::grid_around(min, max, MIN_CELL, MAX_PROBES))
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
    probes::dilate(cells, grid.resolution)
        .into_iter()
        .map(|cube| match cube {
            Some(cube) => Probe { light: cube.map(rgb9e5), distance: [far; 6], flags: probe_flags::VALID },
            None => Probe::default(),
        })
        .collect()
}
