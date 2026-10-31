//! Levels built in code from brushes: the same convex solids make the
//! collision, the drawn geometry (textured by world-aligned projection) and
//! what the light baker traces.

use glam::{Mat4, Vec3};
use refract_asset::encoding::{oct_encode, snorm16};
use refract_asset::map::{LIGHT_POINT, LIGHT_SPOT, Light};
use refract_render::gi::{bake, probes};
use refract_render::{Material, MaterialId, MeshData, MeshPart, PointLight, ProbeGrid, Renderer, Sphere, SpotLight, VertexAttributes};
use refract_world::{Brush, Collision};

use crate::scene::Scene;

/// World units covered by one repeat of a texture.
const TEXTURE_UNITS: f32 = 256.0;
/// Finest probe spacing, and the most probes, of the baked light.
const PROBE_CELL: f32 = 32.0;
const MAX_PROBES: f32 = 1_000_000.0;
const BOUNCES: u32 = 3;
const REFLECTION_SIZE: u32 = 64;

/// A material and the colour it bounces light with.
#[derive(Clone, Copy)]
pub struct Surface {
    pub material: MaterialId,
    /// Average linear albedo, texture included.
    pub albedo: Vec3,
}

pub struct LevelBuilder<'a> {
    pub renderer: &'a mut Renderer,
    brushes: Vec<(Brush, Surface)>,
    /// Solid but not drawn.
    solids: Vec<Brush>,
    /// Drawn, not solid (decoration): mesh, surface, placement.
    meshes: Vec<(MeshData, Surface, Mat4)>,
    point_lights: Vec<PointLight>,
    spot_lights: Vec<SpotLight>,
    reflections: Vec<Vec3>,
}

impl<'a> LevelBuilder<'a> {
    pub fn new(renderer: &'a mut Renderer) -> Self {
        Self { renderer, brushes: Vec::new(), solids: Vec::new(), meshes: Vec::new(), point_lights: Vec::new(), spot_lights: Vec::new(), reflections: Vec::new() }
    }

    /// `texture_albedo`: the average colour of its base texture (white without one).
    pub fn surface(&mut self, material: &Material, texture_albedo: f32) -> Surface {
        Surface { material: self.renderer.add_material(material), albedo: material.base_color.truncate() * texture_albedo }
    }

    pub fn brush(&mut self, brush: Brush, surface: Surface) {
        self.brushes.push((brush, surface));
    }

    pub fn boxed(&mut self, min: Vec3, max: Vec3, surface: Surface) {
        self.brush(Brush::cuboid(min, max), surface);
    }

    /// Solid but not drawn: a simpler shape for something drawn with [`LevelBuilder::mesh`].
    pub fn solid(&mut self, brush: Brush) {
        self.solids.push(brush);
    }

    /// Drawn and lit, but not solid.
    pub fn mesh(&mut self, mesh: MeshData, surface: Surface, placement: Mat4) {
        self.meshes.push((mesh, surface, placement));
    }

    /// A light that stays in place: it is baked into the indirect light.
    pub fn point_light(&mut self, light: PointLight) {
        self.point_lights.push(light);
    }

    pub fn spot_light(&mut self, light: SpotLight) {
        self.spot_lights.push(light);
    }

    /// Where to bake a reflection cubemap; surfaces reflect the nearest.
    pub fn reflection(&mut self, position: Vec3) {
        self.reflections.push(position);
    }

    /// Uploads the level, bakes its light when the GPU can, and fills the
    /// scene's lights and collision.
    pub fn finish(mut self, scene: &mut Scene) {
        let mut geometry = MeshData::default();
        let mut parts = Vec::new();
        for (brush, _) in &self.brushes {
            let first = geometry.indices.len();
            add_brush(&mut geometry, brush);
            let range = &geometry.indices[first..];
            parts.push(MeshPart { first_index: first as u32, index_count: range.len() as u32, bounds: Sphere::around(range.iter().map(|&i| Vec3::from(geometry.positions[i as usize]))) });
        }
        let meshes = self.renderer.add_geometry(&geometry.positions, &geometry.attributes, &geometry.indices, &parts);
        for ((_, surface), mesh) in self.brushes.iter().zip(meshes) {
            self.renderer.add_instance(mesh, surface.material, Mat4::IDENTITY);
        }
        for (mesh, surface, placement) in &self.meshes {
            let id = self.renderer.add_mesh(mesh);
            self.renderer.add_instance(id, surface.material, *placement);
        }

        self.bake(&geometry, &parts, scene);
        scene.point_lights.extend_from_slice(&self.point_lights);
        scene.spot_lights.extend_from_slice(&self.spot_lights);
        scene.collision = Some(Collision::new(self.brushes.into_iter().map(|(brush, _)| brush).chain(self.solids).collect()));
    }

    fn bake(&mut self, geometry: &MeshData, parts: &[MeshPart], scene: &Scene) {
        if !bake::available() {
            log::warn!("no GPU able to trace rays: the level is lit by the sky alone");
            return;
        }
        let start = std::time::Instant::now();
        // World-space triangles, one albedo each.
        let mut positions = geometry.positions.clone();
        let mut indices = geometry.indices.clone();
        let mut albedo = Vec::new();
        for (part, (_, surface)) in parts.iter().zip(&self.brushes) {
            albedo.extend(std::iter::repeat_n(surface.albedo.extend(0.0).to_array(), part.index_count as usize / 3));
        }
        for (mesh, surface, placement) in &self.meshes {
            let base = positions.len() as u32;
            positions.extend(mesh.positions.iter().map(|p| placement.transform_point3(Vec3::from(*p)).to_array()));
            indices.extend(mesh.indices.iter().map(|i| base + i));
            albedo.extend(std::iter::repeat_n(surface.albedo.extend(0.0).to_array(), mesh.indices.len() / 3));
        }
        let (min, max) = positions.iter().fold((Vec3::MAX, Vec3::MIN), |(min, max), p| (min.min(Vec3::from(*p)), max.max(Vec3::from(*p))));
        let grid = probes::grid_around(min, max, PROBE_CELL, MAX_PROBES);
        let lights: Vec<Light> = self.point_lights.iter().map(point_light).chain(self.spot_lights.iter().map(spot_light)).collect();
        let input = bake::BakeInput {
            positions: &positions,
            indices: &indices,
            albedo: &albedo,
            sun: Some((scene.sun.direction, scene.sun.color * scene.sun.intensity)),
            sky: scene.sky,
            lights: &lights,
            grid,
            bounces: BOUNCES,
            reflections: &self.reflections,
            reflection_size: REFLECTION_SIZE,
            sky_cubemap: None,
        };
        let mut baked = match bake::bake(&input) {
            Ok(baked) => baked,
            Err(err) => {
                log::warn!("baking light failed, the level is lit by the sky alone: {err}");
                return;
            }
        };
        probes::fill_closed(&mut baked.probes, grid.resolution);
        self.renderer.set_probes(Some(&ProbeGrid { origin: grid.origin.into(), cell_size: grid.cell_size, resolution: grid.resolution, probes: &baked.probes }));
        if !self.reflections.is_empty() {
            let cubemaps = probes::reflection_cubemaps(&baked.reflections, self.reflections.len() as u32, REFLECTION_SIZE);
            self.renderer.set_reflections(&cubemaps, &self.reflections);
        }
        log::info!("baked {} probes and {} reflections in {:.2} s", baked.probes.len(), self.reflections.len(), start.elapsed().as_secs_f32());
    }
}

/// Appends a brush's faces, textured by projection along the axis each
/// face is most turned to.
fn add_brush(mesh: &mut MeshData, brush: &Brush) {
    for (plane, polygon) in brush.faces() {
        let n = plane.normal;
        let a = n.abs();
        let (u, v) = if a.z >= a.x && a.z >= a.y {
            (Vec3::X, Vec3::Y)
        } else if a.x >= a.y {
            (Vec3::Y, -Vec3::Z)
        } else {
            (Vec3::X, -Vec3::Z)
        };
        let sign = if n.cross(u).dot(v) >= 0.0 { i16::MAX } else { -i16::MAX };
        let first = mesh.positions.len() as u32;
        for p in polygon {
            mesh.positions.push(p.to_array());
            mesh.attributes.push(VertexAttributes {
                normal: oct_encode(n),
                tangent: [snorm16(u.x), snorm16(u.y), snorm16(u.z), sign],
                uv: [p.dot(u) / TEXTURE_UNITS, p.dot(v) / TEXTURE_UNITS],
                color: [255; 4],
            });
        }
        for i in 1..polygon.len() as u32 - 1 {
            mesh.indices.extend_from_slice(&[first, first + i, first + i + 1]);
        }
    }
}

fn point_light(light: &PointLight) -> Light {
    Light {
        kind: LIGHT_POINT,
        position: light.position.to_array(),
        range: light.range,
        direction: [0.0, 0.0, -1.0],
        intensity: light.intensity,
        color: light.color.to_array(),
        inner_angle: 0.0,
        outer_angle: 0.0,
        flags: 0,
        clearance: light.clearance,
    }
}

fn spot_light(light: &SpotLight) -> Light {
    Light {
        kind: LIGHT_SPOT,
        position: light.position.to_array(),
        range: light.range,
        direction: light.direction.to_array(),
        intensity: light.intensity,
        color: light.color.to_array(),
        inner_angle: light.inner_angle,
        outer_angle: light.outer_angle,
        flags: 0,
        clearance: light.clearance,
    }
}
