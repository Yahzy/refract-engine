//! Dynamic lights and the sky. Colours are linear RGB; intensities are relative
//! brightness: a point light of intensity 1 lights a surface 1 meter away like
//! a sun of intensity 1.

use glam::Vec3;

use crate::gpu_types::{GpuLight, LIGHT_POINT, LIGHT_SPOT, NO_SHADOW};

#[derive(Clone, Copy, Debug)]
pub struct Sun {
    /// Direction the sunlight travels.
    pub direction: Vec3,
    pub color: Vec3,
    pub intensity: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Sky {
    pub zenith: Vec3,
    pub horizon: Vec3,
    pub ground: Vec3,
}

impl Sky {
    /// A clear day.
    pub const CLEAR_DAY: Sky = Sky { zenith: Vec3::new(0.18, 0.32, 0.65), horizon: Vec3::new(0.55, 0.62, 0.72), ground: Vec3::new(0.12, 0.11, 0.10) };
}

#[derive(Clone, Copy, Debug)]
pub struct PointLight {
    pub position: Vec3,
    pub color: Vec3,
    pub intensity: f32,
    /// Distance at which the light fades out completely, in world units.
    pub range: f32,
    pub casts_shadows: bool,
    /// Room around the light that ray-traced shadows ignore: the fixture
    /// (globe, shade, housing) the light sits in.
    pub clearance: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct SpotLight {
    pub position: Vec3,
    /// Direction the light shines.
    pub direction: Vec3,
    pub color: Vec3,
    pub intensity: f32,
    pub range: f32,
    /// Half-angles of the full-brightness cone and of the outer edge, in degrees.
    pub inner_angle: f32,
    pub outer_angle: f32,
    pub casts_shadows: bool,
    /// See [`PointLight::clearance`].
    pub clearance: f32,
}

impl From<&PointLight> for GpuLight {
    fn from(light: &PointLight) -> Self {
        GpuLight {
            position: light.position.into(),
            range: light.range,
            color: (light.color * light.intensity).into(),
            kind: LIGHT_POINT,
            direction: [0.0; 3],
            cos_outer: -1.0,
            cos_inner: -1.0,
            shadow: NO_SHADOW,
            clearance: light.clearance,
            _pad: 0.0,
        }
    }
}

impl From<&SpotLight> for GpuLight {
    fn from(light: &SpotLight) -> Self {
        GpuLight {
            position: light.position.into(),
            range: light.range,
            color: (light.color * light.intensity).into(),
            kind: LIGHT_SPOT,
            direction: light.direction.normalize().into(),
            cos_outer: light.outer_angle.to_radians().cos(),
            cos_inner: light.inner_angle.to_radians().cos(),
            shadow: NO_SHADOW,
            clearance: light.clearance,
            _pad: 0.0,
        }
    }
}
