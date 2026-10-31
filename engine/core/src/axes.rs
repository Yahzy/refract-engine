//! World axes and units.

use glam::Vec3;

pub const FORWARD: Vec3 = Vec3::X;
pub const LEFT: Vec3 = Vec3::Y;
pub const UP: Vec3 = Vec3::Z;

/// Meters per world unit.
pub const METERS_PER_UNIT: f32 = 0.0254;
