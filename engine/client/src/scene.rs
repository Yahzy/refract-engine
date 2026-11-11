//! What the client shows: the lighting of the loaded level, its solid
//! geometry when it has some, and where the player starts. Geometry and
//! materials live in the renderer.

use std::f32::consts::TAU;

use glam::{Mat4, Vec3};
use refract_render::{Camera, Fog, Frame, InstanceId, MaterialId, PointLight, Renderer, Sky, SpotLight, Sun};
use refract_world::{Collision, PropDesc};

pub struct Scene {
    pub spawn: Camera,
    pub sun: Sun,
    pub sky: Sky,
    pub ambient: Vec3,
    pub exposure: f32,
    /// Lights that stay in place.
    pub point_lights: Vec<PointLight>,
    pub spot_lights: Vec<SpotLight>,
    /// Lights circling around, added to `point_lights` each frame.
    pub orbits: Vec<Orbit>,
    pub fog: Option<Fog>,
    /// What players collide with; none for levels only flown through.
    pub collision: Option<Collision>,
    /// Physics objects placed in the level, and what they are drawn with.
    pub props: Vec<(PropDesc, MaterialId)>,
    /// `point_lights` and the orbiting lights at the current time.
    lights: Vec<PointLight>,
}

/// A light circling around a point, with the emissive marker showing where it is.
pub struct Orbit {
    pub center: Vec3,
    pub radius: f32,
    /// Turns per second.
    pub speed: f32,
    pub phase: f32,
    pub color: Vec3,
    pub intensity: f32,
    pub range: f32,
    pub marker: InstanceId,
}

impl Scene {
    pub fn new(spawn: Camera, sun: Sun) -> Self {
        Self {
            spawn,
            sun,
            sky: default_sky(),
            ambient: Vec3::ZERO,
            exposure: 1.0,
            point_lights: Vec::new(),
            spot_lights: Vec::new(),
            orbits: Vec::new(),
            fog: None,
            collision: None,
            props: Vec::new(),
            lights: Vec::new(),
        }
    }

    /// Moves the orbiting lights and their markers to where they are at `time`.
    pub fn update(&mut self, renderer: &mut Renderer, time: f32) {
        self.lights.clear();
        self.lights.extend_from_slice(&self.point_lights);
        for orbit in &self.orbits {
            let angle = (orbit.phase + time * orbit.speed) * TAU;
            let position = orbit.center + Vec3::new(angle.cos(), angle.sin(), 0.0) * orbit.radius;
            self.lights.push(PointLight { position, color: orbit.color, intensity: orbit.intensity, range: orbit.range, casts_shadows: true, clearance: 0.0 });
            renderer.set_transform(orbit.marker, Mat4::from_translation(position));
        }
    }

    /// Call [`Scene::update`] first.
    pub fn frame(&self, camera: Camera, time: f32) -> Frame<'_> {
        Frame {
            camera,
            sun: self.sun,
            sky: self.sky,
            point_lights: &self.lights,
            spot_lights: &self.spot_lights,
            ambient: self.ambient,
            exposure: self.exposure,
            time,
            fog: self.fog,
        }
    }
}

/// A clear day.
pub fn default_sky() -> Sky {
    Sky::CLEAR_DAY
}
