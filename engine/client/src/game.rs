//! Playing a level on foot: the world (level and props) and the local
//! player, advanced together tick by tick, and drawn between ticks.
//!
//! Besides the player's controls (see `player.rs`): E picks up the prop
//! looked at and puts it down, a left click throws it; F throws a crate, G
//! a ball, H a big light crate, and Backspace removes what was thrown.

use glam::{Quat, Vec3, Vec4};
use refract_platform::{Input, KeyCode, MouseButton};
use refract_render::{Camera, Material, MaterialId, Renderer};
use refract_world::{Collision, EntityId, Hull, MoveSettings, PlayerCommand, PropDesc, PropShape, TICK, TickClock, World};

use crate::player::LocalPlayer;
use crate::props::{PropView, ball_desc, crate_desc};

/// Speed props are thrown at, on top of the player's.
const THROW_SPEED: f32 = 300.0;
/// How far ahead of the eyes thrown props appear, at most.
const THROW_DISTANCE: f32 = 64.0;
/// How far props can be picked up from (from the eyes).
const REACH: f32 = 160.0;
/// Heaviest prop that can be picked up (kg).
const MAX_HOLD_MASS: f32 = 250.0;
/// Space left between the eyes and a held prop.
const HOLD_GAP: f32 = 24.0;
/// A held prop stuck this far from where it is held is dropped.
const HOLD_BREAK: f32 = 128.0;
/// Momentum of a throw (kg·units/s): light props fly fast, heavy ones not
/// so far, between the two speeds below.
const HOLD_THROW: f32 = 15000.0;
const HOLD_THROW_SPEEDS: (f32, f32) = (150.0, 1200.0);

/// A prop the player carries in front of its eyes.
struct Held {
    id: EntityId,
    /// From the eyes to the prop's center.
    distance: f32,
    /// The prop's rotation relative to where the player faces, kept while held.
    rotation: Quat,
}

pub struct Game {
    world: World,
    player: LocalPlayer,
    props: PropView,
    clock: TickClock,
    /// Materials of thrown props.
    looks: [MaterialId; 3],
    thrown: Vec<EntityId>,
    held: Option<Held>,
}

impl Game {
    /// Starts at `origin` (feet), with the level's own props.
    pub fn new(renderer: &mut Renderer, collision: Collision, props: Vec<(PropDesc, MaterialId)>, origin: Vec3, yaw: f32) -> Self {
        let mut world = World::new(collision, MoveSettings::default().gravity);
        let player = LocalPlayer::new(&world, origin, yaw);
        let mut view = PropView::default();
        for (desc, material) in props {
            view.spawn(&mut world, desc, material);
        }
        let looks = [
            renderer.add_material(&Material { base_color: Vec4::new(0.55, 0.36, 0.2, 1.0), roughness: 0.7, ..Default::default() }),
            renderer.add_material(&Material { base_color: Vec4::new(0.85, 0.12, 0.08, 1.0), roughness: 0.25, ..Default::default() }),
            renderer.add_material(&Material { base_color: Vec4::new(0.2, 0.5, 0.95, 1.0), roughness: 0.5, ..Default::default() }),
        ];
        Self { world, player, props: view, clock: TickClock::default(), looks, thrown: Vec::new(), held: None }
    }

    /// Reads input, runs the ticks due and places what is drawn.
    pub fn update(&mut self, input: &Input, looking: bool, dt: f32, renderer: &mut Renderer) {
        let command = self.player.command(input, looking, &self.world);
        self.throw(input);
        self.carry(input, looking);
        self.advance(command, input.key_down(KeyCode::Space), dt, renderer);
    }

    /// Runs the ticks due in `dt` seconds with the player doing `command`.
    pub fn advance(&mut self, mut command: PlayerCommand, jump_down: bool, dt: f32, renderer: &mut Renderer) {
        for _ in 0..self.clock.advance(dt) {
            self.player.tick(&mut self.world, &mut command, jump_down, TICK);
            self.hold();
            self.world.step(TICK);
        }
        self.player.ease(dt);
        self.props.update(&self.world, self.clock.alpha(), renderer);
    }

    fn throw(&mut self, input: &Input) {
        if input.key_pressed(KeyCode::Backspace) {
            for id in self.thrown.drain(..) {
                self.world.remove_prop(id);
            }
        }
        let (desc, look) = if input.key_pressed(KeyCode::KeyF) {
            (crate_desc(Vec3::ZERO, Vec3::splat(12.0), 15.0), 0)
        } else if input.key_pressed(KeyCode::KeyG) {
            (ball_desc(Vec3::ZERO, 10.0, 5.0), 1)
        } else if input.key_pressed(KeyCode::KeyH) {
            (crate_desc(Vec3::ZERO, Vec3::new(32.0, 24.0, 24.0), 60.0), 2)
        } else {
            return;
        };
        let forward = self.player.angles.forward();
        let eye = self.player.eye(self.clock.alpha());
        let half = match desc.shape {
            PropShape::Box { half_extents } => half_extents.max_element(),
            PropShape::Sphere { radius } => radius,
        };
        // As far ahead as there is room for it.
        let hull = Hull { min: Vec3::splat(-half), max: Vec3::splat(half) };
        let position = self.world.trace(eye, eye + forward * (THROW_DISTANCE + half), hull).end;
        let rotation = Quat::from_rotation_z(self.player.angles.yaw.to_radians());
        let velocity = forward * THROW_SPEED + self.player.player.velocity;
        let id = self.props.spawn(&mut self.world, PropDesc { position, rotation, velocity, ..desc }, self.looks[look]);
        self.thrown.push(id);
    }

    /// E picks up or puts down, a left click throws what is held.
    fn carry(&mut self, input: &Input, looking: bool) {
        let forward = self.player.angles.forward();
        if let Some(held) = &self.held {
            let id = held.id;
            if input.key_pressed(KeyCode::KeyE) {
                self.world.release_prop(id, None);
                self.held = None;
            } else if looking && input.button_pressed(MouseButton::Left) {
                let mass = self.world.prop_mass(id).unwrap_or(1.0);
                let speed = (HOLD_THROW / mass).clamp(HOLD_THROW_SPEEDS.0, HOLD_THROW_SPEEDS.1);
                self.world.release_prop(id, Some(forward * speed + self.player.player.velocity));
                self.held = None;
            }
            return;
        }
        if !input.key_pressed(KeyCode::KeyE) {
            return;
        }
        let eye = self.player.player.eye(&self.player.settings);
        let trace = self.world.trace(eye, eye + forward * REACH, Hull { min: Vec3::ZERO, max: Vec3::ZERO });
        let Some(id) = trace.entity else { return };
        let (Some(prop), Some(mass)) = (self.world.prop(id), self.world.prop_mass(id)) else { return };
        if mass > MAX_HOLD_MASS {
            return;
        }
        let size = match prop.shape {
            PropShape::Box { half_extents } => half_extents.length(),
            PropShape::Sphere { radius } => radius,
        };
        let distance = eye.distance(prop.position).max(size + HOLD_GAP);
        let rotation = self.facing().inverse() * prop.rotation;
        self.held = Some(Held { id, distance, rotation });
    }

    /// Moves what is held to where it is held; once per tick.
    fn hold(&mut self) {
        let Some(held) = &self.held else { return };
        let target = self.player.player.eye(&self.player.settings) + self.player.angles.forward() * held.distance;
        let rotation = self.facing() * held.rotation;
        let stuck = self.world.prop(held.id).is_some_and(|prop| prop.position.distance(target) > HOLD_BREAK);
        if stuck {
            self.world.release_prop(held.id, None);
        }
        if stuck || !self.world.hold_prop(held.id, target, rotation, TICK) {
            self.held = None;
        }
    }

    /// Where the player faces, turning only around the vertical.
    fn facing(&self) -> Quat {
        Quat::from_rotation_z(self.player.angles.yaw.to_radians())
    }

    pub fn camera(&self) -> Camera {
        self.player.camera(self.clock.alpha())
    }

    /// Ground speed, for the title bar.
    pub fn speed(&self) -> f32 {
        self.player.speed()
    }
}
