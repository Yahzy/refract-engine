//! Orientation as `pitch yaw roll` in degrees, the form maps and players use.

use glam::{Quat, Vec3};

use crate::axes::FORWARD;

/// Pitch is positive looking down, yaw turns counter-clockwise seen from
/// above (0 faces +X, 90 faces +Y), roll is positive tilting right.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Angles {
    pub pitch: f32,
    pub yaw: f32,
    pub roll: f32,
}

impl Angles {
    pub const fn new(pitch: f32, yaw: f32, roll: f32) -> Self {
        Self { pitch, yaw, roll }
    }

    pub fn to_quat(self) -> Quat {
        Quat::from_rotation_z(self.yaw.to_radians())
            * Quat::from_rotation_y(self.pitch.to_radians())
            * Quat::from_rotation_x(self.roll.to_radians())
    }

    pub fn forward(self) -> Vec3 {
        self.to_quat() * FORWARD
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaw_turns_towards_left() {
        let forward = Angles::new(0.0, 90.0, 0.0).forward();
        assert!(forward.abs_diff_eq(Vec3::Y, 1e-6));
    }

    #[test]
    fn positive_pitch_looks_down() {
        let forward = Angles::new(45.0, 0.0, 0.0).forward();
        assert!(forward.z < 0.0 && forward.x > 0.0);
    }

    #[test]
    fn positive_roll_tilts_right() {
        let up = Angles::new(0.0, 0.0, 30.0).to_quat() * Vec3::Z;
        assert!(up.y < 0.0);
    }
}
