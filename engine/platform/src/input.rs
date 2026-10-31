//! Keyboard and mouse state, gathered between two frames.

use std::collections::HashSet;

use glam::Vec2;
use winit::event::{ElementState, MouseButton};
use winit::keyboard::KeyCode;

#[derive(Default)]
pub struct Input {
    keys_down: HashSet<KeyCode>,
    keys_pressed: HashSet<KeyCode>,
    buttons_down: HashSet<MouseButton>,
    mouse_delta: Vec2,
}

impl Input {
    /// Whether the key is held.
    pub fn key_down(&self, key: KeyCode) -> bool {
        self.keys_down.contains(&key)
    }

    /// Whether the key went down since the last frame.
    pub fn key_pressed(&self, key: KeyCode) -> bool {
        self.keys_pressed.contains(&key)
    }

    pub fn button_down(&self, button: MouseButton) -> bool {
        self.buttons_down.contains(&button)
    }

    /// Raw mouse motion since the last frame, in device units (+y is down).
    pub fn mouse_delta(&self) -> Vec2 {
        self.mouse_delta
    }

    pub(crate) fn on_key(&mut self, key: KeyCode, state: ElementState, repeat: bool) {
        match state {
            ElementState::Pressed => {
                if !repeat {
                    self.keys_pressed.insert(key);
                }
                self.keys_down.insert(key);
            }
            ElementState::Released => {
                self.keys_down.remove(&key);
            }
        }
    }

    pub(crate) fn on_button(&mut self, button: MouseButton, state: ElementState) {
        match state {
            ElementState::Pressed => self.buttons_down.insert(button),
            ElementState::Released => self.buttons_down.remove(&button),
        };
    }

    pub(crate) fn on_mouse_motion(&mut self, delta: Vec2) {
        self.mouse_delta += delta;
    }

    /// Forgets held keys, e.g. when the window loses focus.
    pub(crate) fn release_all(&mut self) {
        self.keys_down.clear();
        self.buttons_down.clear();
    }

    /// Clears what only lasts one frame.
    pub(crate) fn end_frame(&mut self) {
        self.keys_pressed.clear();
        self.mouse_delta = Vec2::ZERO;
    }
}
