//! The local player: input becomes one command per tick, the simulation
//! moves the player, and the camera follows its eyes, interpolated between
//! ticks.
//!
//! Controls: mouse to look, WASD to move, Space or the mouse wheel to jump,
//! Ctrl to crouch, Shift to run, Alt to walk slowly, V for noclip, R to go
//! back to the spawn.

use glam::Vec3;
use refract_core::Angles;
use refract_core::axes::UP;
use refract_platform::{Input, KeyCode};
use refract_render::Camera;
use refract_world::{Collision, MoveSettings, Player, PlayerCommand, TICK, TickClock};

/// Degrees per mouse count.
const SENSITIVITY: f32 = 0.022 * 3.0;
/// Seconds for the view to catch up after a step up or down.
const STEP_SMOOTHING: f32 = 0.06;

pub struct LocalPlayer {
    world: Collision,
    settings: MoveSettings,
    spawn: (Vec3, f32),
    player: Player,
    /// The state one tick earlier, for interpolation.
    previous: Player,
    clock: TickClock,
    angles: Angles,
    /// A jump asked for (key press, wheel) and not yet seen by a tick.
    jump_queued: bool,
    /// Vertical offset of the view, easing out the jumps of the eyes when
    /// climbing or walking down steps.
    step_offset: f32,
}

impl LocalPlayer {
    /// The player stands on the ground under `origin`, or on whatever is
    /// there up to a step above it.
    pub fn new(world: Collision, origin: Vec3, yaw: f32) -> Self {
        let settings = MoveSettings::default();
        let mut player = Player::new(origin);
        let hull = player.hull(&settings);
        player.origin = world.trace(origin + UP * settings.step_height, origin - UP * 256.0, hull).end;
        Self {
            world,
            settings,
            spawn: (origin, yaw),
            player,
            previous: player,
            clock: TickClock::default(),
            angles: Angles::new(0.0, yaw, 0.0),
            jump_queued: false,
            step_offset: 0.0,
        }
    }

    /// Reads input (mouse look only when `looking`) and runs the ticks due.
    pub fn update(&mut self, input: &Input, looking: bool, dt: f32) {
        if looking {
            let delta = input.mouse_delta() * SENSITIVITY;
            self.angles.yaw = (self.angles.yaw - delta.x).rem_euclid(360.0);
            self.angles.pitch = (self.angles.pitch + delta.y).clamp(-89.0, 89.0);
        }
        if input.key_pressed(KeyCode::KeyV) {
            // The velocity carries over both ways.
            self.player.noclip = !self.player.noclip;
        }
        if input.key_pressed(KeyCode::KeyR) {
            let (origin, yaw) = self.spawn;
            *self = Self::new(std::mem::take(&mut self.world), origin, yaw);
            return;
        }
        self.jump_queued |= input.key_pressed(KeyCode::Space) || input.wheel_delta() != 0.0;

        let axis = |positive: KeyCode, negative: KeyCode| input.key_down(positive) as i32 as f32 - input.key_down(negative) as i32 as f32;
        let mut command = PlayerCommand {
            angles: self.angles,
            forward: axis(KeyCode::KeyW, KeyCode::KeyS),
            side: axis(KeyCode::KeyD, KeyCode::KeyA),
            jump: input.key_down(KeyCode::Space),
            crouch: input.key_down(KeyCode::ControlLeft) || input.key_down(KeyCode::ControlRight),
            sprint: input.key_down(KeyCode::ShiftLeft),
            slow_walk: input.key_down(KeyCode::AltLeft),
        };
        for _ in 0..self.clock.advance(dt) {
            // A queued press counts once, then is released for a tick so
            // that the next one is a new press.
            if self.jump_queued && !command.jump {
                command.jump = true;
            }
            self.previous = self.player;
            self.player.step(&command, &self.world, &self.settings, TICK);
            if self.jump_queued {
                self.jump_queued = false;
                command.jump = input.key_down(KeyCode::Space);
            }
            self.smooth_steps();
        }
        self.step_offset *= (-dt / STEP_SMOOTHING).exp();
    }

    /// Hides the sudden height changes of walking up or down a step: the
    /// view starts where it was and eases to the new height.
    fn smooth_steps(&mut self) {
        let dz = self.player.origin.z - self.previous.origin.z;
        let walking = self.previous.on_ground && self.player.on_ground && !self.player.noclip;
        if walking && dz != 0.0 && dz.abs() <= self.settings.step_height + 0.5 {
            self.step_offset = (self.step_offset - dz).clamp(-self.settings.step_height, self.settings.step_height);
            // Interpolate from the new height: the offset does the easing.
            self.previous.origin.z = self.player.origin.z;
        }
    }

    pub fn camera(&self) -> Camera {
        let alpha = self.clock.alpha();
        let eye = self.previous.eye(&self.settings).lerp(self.player.eye(&self.settings), alpha) + UP * self.step_offset;
        Camera { position: eye, angles: self.angles, ..Default::default() }
    }

    /// Ground speed, for the title bar.
    pub fn speed(&self) -> f32 {
        refract_world::movement::ground_speed(self.player.velocity)
    }
}
