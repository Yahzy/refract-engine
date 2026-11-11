//! The C interface of `shim/physics.c`. The structs mirror the C ones
//! exactly; `tests::sizes_match_the_c_side` checks them.

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quat {
    pub v: Vec3,
    pub s: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub p: Vec3,
    pub q: Quat,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WorldId {
    pub index1: u16,
    pub generation: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BodyId {
    pub index1: i32,
    pub world0: u16,
    pub generation: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Material {
    pub density: f32,
    pub friction: f32,
    pub restitution: f32,
    pub category: u64,
    pub mask: u64,
}

unsafe extern "C" {
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn rf_sizes(out: *mut i32);
    pub fn rf_set_length_units(units_per_meter: f32);
    pub fn rf_create_world(gravity: Vec3) -> WorldId;
    pub fn rf_destroy_world(world: WorldId);
    pub fn rf_step(world: WorldId, dt: f32, substeps: i32);
    pub fn rf_create_body(world: WorldId, kind: i32, position: Vec3, rotation: Quat, user: u64) -> BodyId;
    pub fn rf_destroy_body(body: BodyId);
    pub fn rf_add_box(body: BodyId, half_extents: Vec3, offset: Vec3, material: *const Material) -> bool;
    pub fn rf_add_sphere(body: BodyId, center: Vec3, radius: f32, material: *const Material) -> bool;
    pub fn rf_add_hull(body: BodyId, points: *const Vec3, count: i32, material: *const Material) -> bool;
    pub fn rf_body_valid(body: BodyId) -> bool;
    pub fn rf_body_transform(body: BodyId) -> Transform;
    pub fn rf_body_velocity(body: BodyId) -> Vec3;
    pub fn rf_body_angular_velocity(body: BodyId) -> Vec3;
    pub fn rf_body_mass(body: BodyId) -> f32;
    pub fn rf_body_awake(body: BodyId) -> bool;
    pub fn rf_body_user(body: BodyId) -> u64;
    pub fn rf_body_set_user(body: BodyId, user: u64);
    pub fn rf_body_set_transform(body: BodyId, position: Vec3, rotation: Quat);
    pub fn rf_body_set_velocity(body: BodyId, linear: Vec3, angular: Vec3);
    pub fn rf_body_apply_impulse(body: BodyId, impulse: Vec3, point: Vec3);
    pub fn rf_query_box(world: WorldId, min: Vec3, max: Vec3, mask: u64, bodies: *mut BodyId, capacity: i32) -> i32;
}

impl From<glam::Vec3> for Vec3 {
    fn from(v: glam::Vec3) -> Self {
        Self { x: v.x, y: v.y, z: v.z }
    }
}

impl From<Vec3> for glam::Vec3 {
    fn from(v: Vec3) -> Self {
        Self::new(v.x, v.y, v.z)
    }
}

impl From<glam::Quat> for Quat {
    fn from(q: glam::Quat) -> Self {
        Self { v: Vec3 { x: q.x, y: q.y, z: q.z }, s: q.w }
    }
}

impl From<Quat> for glam::Quat {
    fn from(q: Quat) -> Self {
        Self::from_xyzw(q.v.x, q.v.y, q.v.z, q.s)
    }
}
