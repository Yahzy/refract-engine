//! Rigid body physics, in world units: bodies made of convex shapes that
//! fall, collide, rest and sleep. A safe interface over the Box3D library,
//! compiled from the `box3d/` submodule with a small C layer
//! (`shim/physics.c`) so that only plain values cross the boundary.
//!
//! The library works in any length unit once told how many make a meter;
//! [`Physics::new`] sets it, so positions, speeds and gravity are given in
//! world units. Densities are in kg/m³ and masses in kg.

mod ffi;

use std::cell::Cell;
use std::marker::PhantomData;
use std::sync::{Mutex, OnceLock};

use glam::{Quat, Vec3};

/// How a body moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyKind {
    /// Never moves (level geometry).
    Static,
    /// Moved by setting its velocity; pushes dynamic bodies, nothing pushes it.
    Kinematic,
    /// Moved by forces and collisions.
    Dynamic,
}

/// A convex part of a body, placed in the body's frame.
#[derive(Clone, Copy, Debug)]
pub enum Shape<'a> {
    Box { half_extents: Vec3, offset: Vec3 },
    Sphere { center: Vec3, radius: f32 },
    /// The convex hull of the points (at most 128 kept).
    Hull(&'a [Vec3]),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    /// kg/m³: water is 1000, wood about 600, steel 7800.
    pub density: f32,
    /// Coulomb friction, usually 0 to 1.
    pub friction: f32,
    /// Bounciness, 0 to 1.
    pub restitution: f32,
}

impl Default for Material {
    fn default() -> Self {
        Self { density: 1000.0, friction: 0.6, restitution: 0.0 }
    }
}

/// Which shapes collide: two shapes touch when each one's category is in
/// the other's mask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Filter {
    pub category: u64,
    pub mask: u64,
}

impl Default for Filter {
    fn default() -> Self {
        Self { category: 1, mask: u64::MAX }
    }
}

/// A body of a [`Physics`] world. Using it after it is destroyed panics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BodyId(ffi::BodyId);

/// A simulated world.
pub struct Physics {
    world: ffi::WorldId,
    /// m³ per cubic world unit, to turn densities per m³ into densities per unit³.
    unit_volume: f32,
    /// The library keeps per-world state that is not safe to share between threads.
    _not_sync: PhantomData<Cell<()>>,
}

/// World units per meter, set once for the process.
static UNITS_PER_METER: OnceLock<f32> = OnceLock::new();
/// Creating and destroying worlds touches the library's global table of worlds.
static WORLDS: Mutex<()> = Mutex::new(());

impl Physics {
    /// A world where one unit is `meters_per_unit` meters, with gravity in
    /// units/s². All worlds of a process share the same unit.
    pub fn new(meters_per_unit: f32, gravity: Vec3) -> Self {
        let units = 1.0 / meters_per_unit;
        let set = *UNITS_PER_METER.get_or_init(|| {
            // SAFETY: plain value, set before any world exists.
            unsafe { ffi::rf_set_length_units(units) };
            units
        });
        assert_eq!(set, units, "every physics world of a process uses the same unit");
        let _lock = WORLDS.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: plain values; the library copies what it needs.
        let world = unsafe { ffi::rf_create_world(gravity.into()) };
        Self { world, unit_volume: meters_per_unit.powi(3), _not_sync: PhantomData }
    }

    /// Advances by `dt` seconds, solved in `substeps` smaller steps (4 is good).
    pub fn step(&mut self, dt: f32, substeps: u32) {
        // SAFETY: the world is alive while `self` is.
        unsafe { ffi::rf_step(self.world, dt, substeps as i32) };
    }

    /// A body without shapes yet; `user` is any value to find what it belongs to.
    pub fn create_body(&mut self, kind: BodyKind, position: Vec3, rotation: Quat, user: u64) -> BodyId {
        let kind = match kind {
            BodyKind::Static => 0,
            BodyKind::Kinematic => 1,
            BodyKind::Dynamic => 2,
        };
        // SAFETY: plain values.
        BodyId(unsafe { ffi::rf_create_body(self.world, kind, position.into(), rotation.normalize().into(), user) })
    }

    pub fn destroy_body(&mut self, body: BodyId) {
        let body = self.valid(body);
        // SAFETY: a valid body.
        unsafe { ffi::rf_destroy_body(body) };
    }

    /// Adds a shape; a dynamic body's mass follows. False when the shape
    /// is degenerate (flat or empty hull) and was left out.
    pub fn add_shape(&mut self, body: BodyId, shape: Shape, material: Material, filter: Filter) -> bool {
        let body = self.valid(body);
        let material = ffi::Material {
            density: material.density * self.unit_volume,
            friction: material.friction,
            restitution: material.restitution,
            category: filter.category,
            mask: filter.mask,
        };
        // SAFETY: a valid body; the library copies the material and geometry.
        unsafe {
            match shape {
                Shape::Box { half_extents, offset } => ffi::rf_add_box(body, half_extents.into(), offset.into(), &material),
                Shape::Sphere { center, radius } => ffi::rf_add_sphere(body, center.into(), radius, &material),
                Shape::Hull(points) => {
                    let points: Vec<ffi::Vec3> = points.iter().map(|p| (*p).into()).collect();
                    points.len() >= 4 && ffi::rf_add_hull(body, points.as_ptr(), points.len() as i32, &material)
                }
            }
        }
    }

    /// Position (of the body origin) and rotation.
    pub fn transform(&self, body: BodyId) -> (Vec3, Quat) {
        // SAFETY: a valid body.
        let t = unsafe { ffi::rf_body_transform(self.valid(body)) };
        (t.p.into(), t.q.into())
    }

    pub fn set_transform(&mut self, body: BodyId, position: Vec3, rotation: Quat) {
        // SAFETY: a valid body.
        unsafe { ffi::rf_body_set_transform(self.valid(body), position.into(), rotation.normalize().into()) };
    }

    pub fn velocity(&self, body: BodyId) -> Vec3 {
        // SAFETY: a valid body.
        unsafe { ffi::rf_body_velocity(self.valid(body)) }.into()
    }

    /// Radians per second around each axis.
    pub fn angular_velocity(&self, body: BodyId) -> Vec3 {
        // SAFETY: a valid body.
        unsafe { ffi::rf_body_angular_velocity(self.valid(body)) }.into()
    }

    pub fn set_velocity(&mut self, body: BodyId, linear: Vec3, angular: Vec3) {
        // SAFETY: a valid body.
        unsafe { ffi::rf_body_set_velocity(self.valid(body), linear.into(), angular.into()) };
    }

    /// Kilograms; zero for static and kinematic bodies.
    pub fn mass(&self, body: BodyId) -> f32 {
        // SAFETY: a valid body.
        unsafe { ffi::rf_body_mass(self.valid(body)) }
    }

    /// Sleeping bodies cost nothing until something touches them.
    pub fn is_awake(&self, body: BodyId) -> bool {
        // SAFETY: a valid body.
        unsafe { ffi::rf_body_awake(self.valid(body)) }
    }

    /// The value given at creation.
    pub fn user(&self, body: BodyId) -> u64 {
        // SAFETY: a valid body.
        unsafe { ffi::rf_body_user(self.valid(body)) }
    }

    pub fn set_user(&mut self, body: BodyId, user: u64) {
        // SAFETY: a valid body.
        unsafe { ffi::rf_body_set_user(self.valid(body), user) };
    }

    /// An impulse (kg·units/s) at a world point; wakes the body.
    pub fn apply_impulse(&mut self, body: BodyId, impulse: Vec3, point: Vec3) {
        // SAFETY: a valid body.
        unsafe { ffi::rf_body_apply_impulse(self.valid(body), impulse.into(), point.into()) };
    }

    /// Replaces `bodies` with those having a shape whose bounds overlap
    /// `min..max` and whose category is in `mask`.
    pub fn bodies_in_box(&self, min: Vec3, max: Vec3, mask: u64, bodies: &mut Vec<BodyId>) {
        let mut found: Vec<ffi::BodyId> = Vec::with_capacity(bodies.capacity().max(16));
        loop {
            let capacity = found.capacity();
            // SAFETY: the library writes at most `capacity` ids.
            let count = unsafe { ffi::rf_query_box(self.world, min.into(), max.into(), mask, found.as_mut_ptr(), capacity as i32) } as usize;
            if count <= capacity {
                // SAFETY: the first `count` ids were written.
                unsafe { found.set_len(count) };
                break;
            }
            found.reserve(count);
        }
        bodies.clear();
        bodies.extend(found.into_iter().map(BodyId));
    }

    fn valid(&self, body: BodyId) -> ffi::BodyId {
        // SAFETY: checking takes any id.
        assert!(unsafe { ffi::rf_body_valid(body.0) }, "physics body used after it was destroyed");
        body.0
    }
}

impl Drop for Physics {
    fn drop(&mut self) {
        let _lock = WORLDS.lock().unwrap_or_else(|e| e.into_inner());
        // SAFETY: the world is destroyed once, and its ids are useless after.
        unsafe { ffi::rf_destroy_world(self.world) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const METERS_PER_UNIT: f32 = 0.0254;
    const GRAVITY: Vec3 = Vec3::new(0.0, 0.0, -600.0);
    const TICK: f32 = 0.015;

    fn ground(physics: &mut Physics) {
        let body = physics.create_body(BodyKind::Static, Vec3::ZERO, Quat::IDENTITY, 0);
        assert!(physics.add_shape(body, Shape::Box { half_extents: Vec3::new(512.0, 512.0, 16.0), offset: Vec3::new(0.0, 0.0, -16.0) }, Material::default(), Filter::default()));
    }

    fn crate_at(physics: &mut Physics, position: Vec3) -> BodyId {
        let body = physics.create_body(BodyKind::Dynamic, position, Quat::IDENTITY, 7);
        assert!(physics.add_shape(body, Shape::Box { half_extents: Vec3::splat(16.0), offset: Vec3::ZERO }, Material { density: 600.0, ..Default::default() }, Filter::default()));
        body
    }

    #[test]
    fn sizes_match_the_c_side() {
        let mut sizes = [0i32; 7];
        // SAFETY: writes seven values.
        unsafe { ffi::rf_sizes(sizes.as_mut_ptr()) };
        use std::mem::size_of;
        let expected = [size_of::<ffi::Vec3>(), size_of::<ffi::Quat>(), size_of::<ffi::Transform>(), size_of::<ffi::WorldId>(), size_of::<ffi::BodyId>(), size_of::<ffi::Material>(), size_of::<ffi::Transform>()];
        assert_eq!(sizes.map(|s| s as usize), expected);
    }

    #[test]
    fn a_crate_falls_and_rests_on_the_ground() {
        let mut physics = Physics::new(METERS_PER_UNIT, GRAVITY);
        ground(&mut physics);
        let body = crate_at(&mut physics, Vec3::new(0.0, 0.0, 200.0));
        // 32 units = 0.8128 m a side, wood: ~322 kg.
        let mass = physics.mass(body);
        assert!((mass - 0.8128f32.powi(3) * 600.0).abs() < 1.0, "{mass}");
        assert_eq!(physics.user(body), 7);
        for _ in 0..300 {
            physics.step(TICK, 4);
        }
        let (position, _) = physics.transform(body);
        assert!((position.z - 16.0).abs() < 0.5, "{position}");
        assert!(!physics.is_awake(body));
    }

    #[test]
    fn a_stack_stays_up() {
        let mut physics = Physics::new(METERS_PER_UNIT, GRAVITY);
        ground(&mut physics);
        let crates: Vec<_> = (0..5).map(|i| crate_at(&mut physics, Vec3::new(0.0, 0.0, 16.5 + 33.0 * i as f32))).collect();
        for _ in 0..600 {
            physics.step(TICK, 4);
        }
        let (top, _) = physics.transform(*crates.last().unwrap());
        assert!(top.truncate().length() < 1.0 && (top.z - (16.0 + 32.0 * 4.0)).abs() < 2.0, "{top}");
    }

    #[test]
    fn impulses_push_and_queries_find() {
        let mut physics = Physics::new(METERS_PER_UNIT, GRAVITY);
        ground(&mut physics);
        let body = crate_at(&mut physics, Vec3::new(0.0, 0.0, 16.0));
        let mass = physics.mass(body);
        physics.apply_impulse(body, Vec3::X * mass * 100.0, Vec3::new(0.0, 0.0, 16.0));
        assert!((physics.velocity(body).x - 100.0).abs() < 1.0);
        let mut found = Vec::new();
        physics.bodies_in_box(Vec3::new(-20.0, -20.0, 10.0), Vec3::new(20.0, 20.0, 20.0), u64::MAX, &mut found);
        assert!(found.contains(&body));
        physics.bodies_in_box(Vec3::new(100.0, 100.0, 100.0), Vec3::new(120.0, 120.0, 120.0), u64::MAX, &mut found);
        assert!(found.is_empty());
    }

    #[test]
    fn hulls_need_volume() {
        let mut physics = Physics::new(METERS_PER_UNIT, GRAVITY);
        let body = physics.create_body(BodyKind::Static, Vec3::ZERO, Quat::IDENTITY, 0);
        let flat = [Vec3::ZERO, Vec3::X * 10.0, Vec3::Y * 10.0, Vec3::new(10.0, 10.0, 0.0)];
        assert!(!physics.add_shape(body, Shape::Hull(&flat), Material::default(), Filter::default()));
        let tetra = [Vec3::ZERO, Vec3::X * 10.0, Vec3::Y * 10.0, Vec3::Z * 10.0];
        assert!(physics.add_shape(body, Shape::Hull(&tetra), Material::default(), Filter::default()));
    }
}
