//! Drawing the world's props: one renderer instance per prop, moved each
//! frame to where the prop is between the last two ticks.

use std::collections::HashMap;

use glam::{Mat4, Vec3};
use refract_render::{InstanceId, MaterialId, MeshId, Renderer, shapes};
use refract_world::{EntityId, PropDesc, PropShape, World};

#[derive(Default)]
pub struct PropView {
    /// The instance drawing each prop, and the mesh it uses.
    drawn: HashMap<EntityId, (InstanceId, MeshKey)>,
    /// What each prop is drawn with, chosen when spawned.
    materials: HashMap<EntityId, MaterialId>,
    meshes: HashMap<MeshKey, MeshId>,
    /// Hidden instances of removed props, reused for new ones of the same mesh and material.
    unused: HashMap<(MeshKey, MaterialId), Vec<InstanceId>>,
}

/// Props of the same shape and size share a mesh.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum MeshKey {
    Box([u32; 3]),
    Sphere(u32),
}

impl MeshKey {
    fn of(shape: PropShape) -> Self {
        match shape {
            PropShape::Box { half_extents } => Self::Box(half_extents.to_array().map(f32::to_bits)),
            PropShape::Sphere { radius } => Self::Sphere(radius.to_bits()),
        }
    }
}

impl PropView {
    pub fn spawn(&mut self, world: &mut World, desc: PropDesc, material: MaterialId) -> EntityId {
        let id = world.spawn_prop(desc);
        self.materials.insert(id, material);
        id
    }

    /// Places every prop where it is drawn this frame: `alpha` of the way
    /// from the previous tick to the last.
    pub fn update(&mut self, world: &World, alpha: f32, renderer: &mut Renderer) {
        let gone: Vec<EntityId> = self.drawn.keys().copied().filter(|id| world.prop(*id).is_none()).collect();
        for id in gone {
            let (instance, key) = self.drawn.remove(&id).unwrap();
            let material = self.materials.remove(&id).unwrap();
            renderer.set_visible(instance, false);
            self.unused.entry((key, material)).or_default().push(instance);
        }
        for (id, prop) in world.props() {
            let (position, rotation) = prop.interpolated(alpha);
            let transform = Mat4::from_rotation_translation(rotation, position);
            if let Some((instance, _)) = self.drawn.get(&id) {
                renderer.set_transform(*instance, transform);
                continue;
            }
            let Some(&material) = self.materials.get(&id) else { continue };
            let key = MeshKey::of(prop.shape);
            let instance = match self.unused.get_mut(&(key, material)).and_then(Vec::pop) {
                Some(instance) => {
                    renderer.set_transform(instance, transform);
                    renderer.set_visible(instance, true);
                    instance
                }
                None => {
                    let mesh = *self.meshes.entry(key).or_insert_with(|| renderer.add_mesh(&mesh(prop.shape)));
                    renderer.add_instance(mesh, material, transform)
                }
            };
            self.drawn.insert(id, (instance, key));
        }
    }
}

fn mesh(shape: PropShape) -> refract_render::MeshData {
    match shape {
        PropShape::Box { half_extents } => shapes::cuboid(half_extents),
        PropShape::Sphere { radius } => {
            // Finer for bigger spheres.
            let segments = (radius * 2.0).clamp(16.0, 64.0) as u32;
            shapes::sphere(radius, segments, segments / 2)
        }
    }
}

/// A box of `mass` kilograms.
pub fn crate_desc(position: Vec3, half_extents: Vec3, mass: f32) -> PropDesc {
    let shape = PropShape::Box { half_extents };
    PropDesc { shape, material: refract_world::Material { density: mass / shape.volume(), friction: 0.6, restitution: 0.0 }, position, rotation: glam::Quat::IDENTITY, velocity: Vec3::ZERO }
}

/// A bouncy ball of `mass` kilograms.
pub fn ball_desc(position: Vec3, radius: f32, mass: f32) -> PropDesc {
    let shape = PropShape::Sphere { radius };
    PropDesc { shape, material: refract_world::Material { density: mass / shape.volume(), friction: 0.5, restitution: 0.6 }, position, rotation: glam::Quat::IDENTITY, velocity: Vec3::ZERO }
}
