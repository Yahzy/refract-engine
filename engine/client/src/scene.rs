//! What the client shows: the lighting of the loaded map (or test scene) and
//! where the player starts. Geometry and materials live in the renderer.

use glam::Vec3;
use refract_render::{Camera, Frame, PointLight, Renderer, Sky, SpotLight, Sun};

use crate::test_scene::TestScene;

pub struct Scene {
    pub spawn: Camera,
    pub sun: Sun,
    pub sky: Sky,
    pub ambient: Vec3,
    pub exposure: f32,
    pub point_lights: Vec<PointLight>,
    pub spot_lights: Vec<SpotLight>,
    /// Animates the test scene's lights.
    pub test: Option<TestScene>,
}

impl Scene {
    pub fn update(&mut self, renderer: &mut Renderer, time: f32) {
        if let Some(test) = &mut self.test {
            test.update(renderer, time, &mut self.point_lights);
        }
    }

    pub fn frame(&self, camera: Camera, time: f32) -> Frame<'_> {
        Frame {
            camera,
            sun: self.sun,
            sky: self.sky,
            point_lights: &self.point_lights,
            spot_lights: &self.spot_lights,
            ambient: self.ambient,
            exposure: self.exposure,
            time,
        }
    }
}

/// A clear day.
pub fn default_sky() -> Sky {
    Sky::CLEAR_DAY
}
