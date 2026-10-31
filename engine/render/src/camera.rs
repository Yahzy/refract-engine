//! The point of view: position, orientation and lens.

use glam::{Mat4, Vec3};
use refract_core::Angles;
use refract_core::axes::UP;

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub position: Vec3,
    pub angles: Angles,
    /// Vertical field of view, in degrees.
    pub fov_y: f32,
    /// Distance to the near plane, in world units. There is no far plane.
    pub near: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self { position: Vec3::ZERO, angles: Angles::default(), fov_y: 75.0, near: 4.0 }
    }
}

impl Camera {
    pub fn view(&self) -> Mat4 {
        Mat4::look_to_rh(self.position, self.angles.forward(), UP)
    }

    /// Reversed depth (1 at the near plane, 0 at infinity) for even precision
    /// across the whole view distance.
    pub fn projection(&self, aspect: f32) -> Mat4 {
        Mat4::perspective_infinite_reverse_rh(self.fov_y.to_radians(), aspect, self.near)
    }
}
