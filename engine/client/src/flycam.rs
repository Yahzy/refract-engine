//! Free-flying camera: hold the right mouse button to look around, WASD to
//! move, E/Q to go up/down, Shift to go faster.

use glam::Vec3;
use refract_core::axes::{LEFT, UP};
use refract_platform::{Input, KeyCode, MouseButton};
use refract_render::Camera;

const SPEED: f32 = 400.0;
const FAST: f32 = 4.0;
/// Degrees per mouse count.
const SENSITIVITY: f32 = 0.1;

/// Whether the camera wants the mouse captured this frame.
pub fn looking(input: &Input) -> bool {
    input.button_down(MouseButton::Right)
}

pub fn update(camera: &mut Camera, input: &Input, dt: f32) {
    if looking(input) {
        let delta = input.mouse_delta() * SENSITIVITY;
        camera.angles.yaw = (camera.angles.yaw - delta.x).rem_euclid(360.0);
        camera.angles.pitch = (camera.angles.pitch + delta.y).clamp(-89.0, 89.0);
    }

    let rotation = camera.angles.to_quat();
    let mut direction = Vec3::ZERO;
    for (key, axis) in [
        (KeyCode::KeyW, camera.angles.forward()),
        (KeyCode::KeyS, -camera.angles.forward()),
        (KeyCode::KeyA, rotation * LEFT),
        (KeyCode::KeyD, -(rotation * LEFT)),
        (KeyCode::KeyE, UP),
        (KeyCode::KeyQ, -UP),
    ] {
        if input.key_down(key) {
            direction += axis;
        }
    }
    let speed = if input.key_down(KeyCode::ShiftLeft) { SPEED * FAST } else { SPEED };
    camera.position += direction.normalize_or_zero() * speed * dt;
}
