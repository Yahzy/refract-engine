//! Player movement: a box that walks, runs, crouches, jumps, climbs steps,
//! slides along walls and slopes, and keeps its speed in the air, where
//! strafing while turning gains speed.
//!
//! Each tick turns one [`PlayerCommand`] into motion. The player is plain
//! data, so a client can predict it and a server replay it.

use glam::{Vec3, Vec3Swizzles};
use refract_core::Angles;
use refract_core::axes::UP;

use crate::collision::{DIST_EPSILON, Hull, Solid, Trace, horizontal};
use crate::entity::EntityId;

/// Tunable movement. Speeds in units per second.
#[derive(Clone, Copy, Debug)]
pub struct MoveSettings {
    pub gravity: f32,
    /// How fast speed on the ground fades without input.
    pub friction: f32,
    /// Below this speed, friction acts as if moving at it: slow players stop quickly.
    pub stop_speed: f32,
    /// Ground acceleration, in multiples of the wished speed per second.
    pub accelerate: f32,
    pub air_accelerate: f32,
    /// The most speed air control adds along the wished direction. Low, so
    /// air control steers rather than propels; turning while strafing keeps
    /// finding new directions to add speed in.
    pub air_speed_cap: f32,
    pub walk_speed: f32,
    pub run_speed: f32,
    pub slow_walk_speed: f32,
    /// Multiplies the speed while crouched.
    pub crouch_speed_factor: f32,
    /// Upward speed of a jump.
    pub jump_speed: f32,
    /// Holding jump jumps again on landing; otherwise each jump needs a new press.
    pub auto_jump: bool,
    /// Tallest step climbed without jumping.
    pub step_height: f32,
    /// Ground is a surface whose normal's height is at least this (0.7: about 45°); steeper slopes slide.
    pub min_ground_normal: f32,
    /// Moving up faster than this leaves the ground (jumping, pushed up a ramp).
    pub leave_ground_speed: f32,
    pub max_speed: f32,
    pub half_width: f32,
    pub stand_height: f32,
    pub crouch_height: f32,
    pub stand_eye: f32,
    pub crouch_eye: f32,
    /// Seconds to crouch on the ground.
    pub crouch_time: f32,
    /// Noclip flight speed; there is no acceleration: releasing the keys stops at once.
    pub noclip_speed: f32,
    /// Multiplies the noclip speed while running, and while crouching.
    pub noclip_run_factor: f32,
    pub noclip_crouch_factor: f32,
    /// Multiplies acceleration and air control when pushed up along a slope
    /// too steep to stand on.
    pub steep_slope_friction: f32,
    /// Kilograms: how hard the player pushes objects and weighs on them.
    pub mass: f32,
}

impl Default for MoveSettings {
    fn default() -> Self {
        Self {
            gravity: 600.0,
            friction: 8.0,
            stop_speed: 10.0,
            accelerate: 10.0,
            air_accelerate: 10.0,
            air_speed_cap: 30.0,
            walk_speed: 200.0,
            run_speed: 400.0,
            slow_walk_speed: 100.0,
            crouch_speed_factor: 0.3,
            jump_speed: 200.0,
            auto_jump: false,
            step_height: 18.0,
            min_ground_normal: 0.7,
            leave_ground_speed: 140.0,
            max_speed: 3500.0,
            half_width: 16.0,
            stand_height: 72.0,
            crouch_height: 36.0,
            stand_eye: 64.0,
            crouch_eye: 28.0,
            crouch_time: 0.2,
            noclip_speed: 500.0,
            noclip_run_factor: 3.0,
            noclip_crouch_factor: 0.1,
            steep_slope_friction: 0.25,
            mass: 85.0,
        }
    }
}

/// What a player asks for during one tick.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerCommand {
    /// Where the player looks; movement follows the yaw.
    pub angles: Angles,
    /// -1 (back) to 1 (forward).
    pub forward: f32,
    /// -1 (left) to 1 (right).
    pub side: f32,
    /// Also rises in noclip.
    pub jump: bool,
    pub crouch: bool,
    pub sprint: bool,
    pub slow_walk: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Player {
    /// The bottom center of its box.
    pub origin: Vec3,
    pub velocity: Vec3,
    pub on_ground: bool,
    /// The entity stood on, if the ground is one.
    pub ground_entity: Option<EntityId>,
    /// The box is crouched (half height).
    pub crouched: bool,
    /// How far the view has lowered towards crouching, 0 to 1.
    pub crouch_amount: f32,
    pub noclip: bool,
    /// Jump was held during the previous tick.
    jump_held: bool,
    /// Scales acceleration and friction: 1, less when pushed up a steep slope.
    surface_friction: f32,
}

impl Player {
    pub fn new(origin: Vec3) -> Self {
        Self { origin, velocity: Vec3::ZERO, on_ground: false, ground_entity: None, crouched: false, crouch_amount: 0.0, noclip: false, jump_held: false, surface_friction: 1.0 }
    }

    pub fn hull(&self, settings: &MoveSettings) -> Hull {
        hull(settings, self.crouched)
    }

    /// Height of the eyes above the origin.
    pub fn eye_height(&self, settings: &MoveSettings) -> f32 {
        if self.crouched { settings.crouch_eye } else { settings.stand_eye + (settings.crouch_eye - settings.stand_eye) * self.crouch_amount }
    }

    pub fn eye(&self, settings: &MoveSettings) -> Vec3 {
        self.origin + UP * self.eye_height(settings)
    }

    /// Advances the player by one tick of `dt` seconds, and returns the
    /// entities it ran into.
    pub fn step(&mut self, command: &PlayerCommand, world: &dyn Solid, settings: &MoveSettings, dt: f32) -> Vec<Touch> {
        let mut mover = Mover { player: self, world, settings, dt, touches: Vec::new() };
        if mover.player.noclip {
            mover.fly(command);
        } else {
            mover.walk(command);
        }
        let touches = mover.touches;
        self.jump_held = command.jump;
        touches
    }
}

/// An entity the player ran into during a tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Touch {
    pub entity: EntityId,
    /// The entity's surface, facing the player.
    pub normal: Vec3,
    /// The player's velocity when it hit.
    pub velocity: Vec3,
}

fn hull(settings: &MoveSettings, crouched: bool) -> Hull {
    let height = if crouched { settings.crouch_height } else { settings.stand_height };
    let w = settings.half_width;
    Hull { min: Vec3::new(-w, -w, 0.0), max: Vec3::new(w, w, height) }
}

/// Most surfaces one move slides along before giving up.
const MAX_CLIP_PLANES: usize = 5;
const MAX_BUMPS: usize = 4;

struct Mover<'a> {
    player: &'a mut Player,
    world: &'a dyn Solid,
    settings: &'a MoveSettings,
    dt: f32,
    touches: Vec<Touch>,
}

impl Mover<'_> {
    fn trace(&self, start: Vec3, end: Vec3) -> Trace {
        self.world.trace(start, end, self.player.hull(self.settings))
    }

    fn walk(&mut self, command: &PlayerCommand) {
        let s = *self.settings;
        let dt = self.dt;
        self.crouch(command.crouch);

        // Gravity is applied in two halves around the move, which integrates
        // a constant acceleration exactly.
        self.player.velocity.z -= s.gravity * 0.5 * dt;

        let wants_jump = command.jump && (s.auto_jump || !self.player.jump_held);
        if wants_jump && self.player.on_ground {
            // Jumping skips this tick's friction: landing and jumping again
            // at once keeps all the speed. Crouched, the jump replaces the
            // vertical speed; standing, it adds to it. The jump's gravity is
            // finished at once.
            if self.player.crouched || self.player.crouch_amount > 0.0 {
                self.player.velocity.z = s.jump_speed;
            } else {
                self.player.velocity.z += s.jump_speed;
            }
            self.player.velocity.z -= s.gravity * 0.5 * dt;
            self.player.on_ground = false;
        }

        if self.player.on_ground {
            self.player.velocity.z = 0.0;
            self.friction(s.friction);
        }

        let (wish_dir, wish_speed) = self.wish(command);
        if self.player.on_ground {
            self.accelerate(wish_dir, wish_speed, s.accelerate);
            self.ground_move();
        } else {
            self.air_accelerate(wish_dir, wish_speed);
            self.slide();
        }

        self.categorize();
        if !self.player.on_ground {
            self.player.velocity.z -= s.gravity * 0.5 * dt;
        }
        self.player.velocity = self.player.velocity.clamp(Vec3::splat(-s.max_speed), Vec3::splat(s.max_speed));
    }

    /// The wished direction and speed, along the ground: movement follows
    /// the view's yaw only.
    fn wish(&self, command: &PlayerCommand) -> (Vec3, f32) {
        let s = self.settings;
        let yaw = command.angles.yaw.to_radians();
        let (forward, right) = (Vec3::new(yaw.cos(), yaw.sin(), 0.0), Vec3::new(yaw.sin(), -yaw.cos(), 0.0));
        let mut speed = if command.sprint {
            s.run_speed
        } else if command.slow_walk {
            s.slow_walk_speed
        } else {
            s.walk_speed
        };
        // Crouching slows down on the ground only.
        if self.player.crouched && self.player.on_ground {
            speed *= s.crouch_speed_factor;
        }
        let wish = (forward * command.forward + right * command.side) * speed;
        let wish_speed = wish.length().min(speed);
        (wish.normalize_or_zero(), wish_speed)
    }

    fn friction(&mut self, friction: f32) {
        let velocity = &mut self.player.velocity;
        let speed = velocity.length();
        if speed < 0.1 {
            return;
        }
        let control = speed.max(self.settings.stop_speed);
        let new_speed = (speed - control * friction * self.player.surface_friction * self.dt).max(0.0);
        *velocity *= new_speed / speed;
    }

    /// Adds speed along `wish_dir` until the speed along it reaches `wish_speed`.
    fn accelerate(&mut self, wish_dir: Vec3, wish_speed: f32, accelerate: f32) {
        let current = self.player.velocity.dot(wish_dir);
        let add = wish_speed - current;
        if add <= 0.0 {
            return;
        }
        let gain = (accelerate * wish_speed * self.dt * self.player.surface_friction).min(add);
        self.player.velocity += wish_dir * gain;
    }

    /// Like `accelerate`, but only up to the air speed cap along the wished
    /// direction, with the full wished speed setting the rate.
    fn air_accelerate(&mut self, wish_dir: Vec3, wish_speed: f32) {
        let s = self.settings;
        let current = self.player.velocity.dot(wish_dir);
        let add = wish_speed.min(s.air_speed_cap) - current;
        if add <= 0.0 {
            return;
        }
        let gain = (s.air_accelerate * wish_speed * self.dt * self.player.surface_friction).min(add);
        self.player.velocity += wish_dir * gain;
    }

    /// Moves along the ground, climbing steps, then sticks to the ground
    /// when walking down steps or slopes.
    fn ground_move(&mut self) {
        self.player.velocity.z = 0.0;
        if self.player.velocity.length_squared() < 1.0 {
            self.player.velocity = Vec3::ZERO;
            return;
        }
        let origin = self.player.origin;
        let destination = origin + self.player.velocity * self.dt;
        let trace = self.trace(origin, destination);
        if trace.fraction == 1.0 {
            self.player.origin = trace.end;
        } else {
            self.step_move();
        }
        self.stay_on_ground();
    }

    /// Moves both as is and lifted by a step's height, and keeps whichever
    /// went further.
    fn step_move(&mut self) {
        let s = *self.settings;
        let (start, start_velocity) = (self.player.origin, self.player.velocity);

        self.slide();
        let (flat, flat_velocity) = (self.player.origin, self.player.velocity);

        self.player.origin = start;
        self.player.velocity = start_velocity;
        let up = self.trace(start, start + UP * s.step_height);
        if !up.all_solid {
            self.player.origin = up.end;
        }
        self.slide();
        let down = self.trace(self.player.origin, self.player.origin - UP * s.step_height);
        // Stepped onto something too steep to stand on: keep the flat move.
        if down.normal.is_some_and(|n| n.z < s.min_ground_normal) {
            self.player.origin = flat;
            self.player.velocity = flat_velocity;
            return;
        }
        if !down.all_solid {
            self.player.origin = down.end;
        }
        let stepped = self.player.origin;
        if (flat - start).xy().length_squared() > (stepped - start).xy().length_squared() {
            self.player.origin = flat;
            self.player.velocity = flat_velocity;
        } else {
            self.player.velocity.z = flat_velocity.z;
        }
    }

    /// Follows the ground down when it drops by less than a step.
    fn stay_on_ground(&mut self) {
        let s = self.settings;
        let origin = self.player.origin;
        let start = self.trace(origin, origin + UP * 2.0).end;
        let down = self.trace(start, origin - UP * s.step_height);
        if down.fraction > 0.0 && down.fraction < 1.0 && !down.start_solid && down.normal.is_some_and(|n| n.z >= s.min_ground_normal) {
            self.player.origin = down.end;
        }
    }

    /// Moves by the velocity for the tick, sliding along whatever is hit.
    fn slide(&mut self) {
        let primal = self.player.velocity;
        let mut original = primal;
        let mut planes = [Vec3::ZERO; MAX_CLIP_PLANES];
        let mut plane_count = 0;
        let mut time_left = self.dt;
        let mut moved = 0.0;
        for _ in 0..MAX_BUMPS {
            let velocity = self.player.velocity;
            if velocity.length_squared() == 0.0 {
                break;
            }
            let trace = self.trace(self.player.origin, self.player.origin + velocity * time_left);
            moved += trace.fraction;
            if trace.all_solid {
                self.player.velocity = Vec3::ZERO;
                return;
            }
            if trace.fraction > 0.0 {
                self.player.origin = trace.end;
                original = velocity;
                plane_count = 0;
            }
            let Some(normal) = trace.normal else { break };
            if let Some(entity) = trace.entity {
                self.touch(Touch { entity, normal, velocity });
            }
            if trace.fraction == 0.0 {
                // Stopped before moving: sliding along this surface, rounding
                // left the box nearer to it than the sweeps keep. Step back
                // out, or every later sweep along it stops at once too (and
                // the player sticks to slopes it should slide along).
                let away = self.player.origin + normal * DIST_EPSILON;
                if !self.world.is_solid(away, self.player.hull(self.settings)) {
                    self.player.origin = away;
                }
            }
            time_left -= time_left * trace.fraction;
            if plane_count >= MAX_CLIP_PLANES {
                self.player.velocity = Vec3::ZERO;
                break;
            }
            planes[plane_count] = normal;
            plane_count += 1;

            if plane_count == 1 && !self.player.on_ground {
                // In the air against one surface: slide along it.
                self.player.velocity = clip_velocity(original, normal);
                original = self.player.velocity;
                continue;
            }
            // Find a surface to slide along that does not push into the others.
            let slid = (0..plane_count).find_map(|i| {
                let v = clip_velocity(original, planes[i]);
                (0..plane_count).all(|j| j == i || v.dot(planes[j]) >= 0.0).then_some(v)
            });
            match slid {
                Some(v) => self.player.velocity = v,
                None if plane_count == 2 => {
                    // Into a crease: move along it.
                    let crease = planes[0].cross(planes[1]).normalize_or_zero();
                    self.player.velocity = crease * crease.dot(self.player.velocity);
                }
                None => {
                    self.player.velocity = Vec3::ZERO;
                    break;
                }
            }
            // Turned back against the original direction: stop, or it jitters in corners.
            if self.player.velocity.dot(primal) <= 0.0 {
                self.player.velocity = Vec3::ZERO;
                break;
            }
        }
        if moved == 0.0 {
            self.player.velocity = Vec3::ZERO;
        }
    }

    /// Keeps the hardest hit on each entity.
    fn touch(&mut self, touch: Touch) {
        let into = |t: &Touch| -t.velocity.dot(t.normal);
        match self.touches.iter_mut().find(|t| t.entity == touch.entity) {
            Some(t) if into(t) < into(&touch) => *t = touch,
            Some(_) => {}
            None => self.touches.push(touch),
        }
    }

    /// Whether the player stands on ground: a surface flat enough within 2
    /// units below. Pushed up along a steeper one, acceleration weakens.
    fn categorize(&mut self) {
        let s = self.settings;
        self.player.surface_friction = 1.0;
        self.player.ground_entity = None;
        if self.player.velocity.z > s.leave_ground_speed {
            self.player.on_ground = false;
            return;
        }
        let origin = self.player.origin;
        let trace = self.trace(origin, origin - UP * 2.0);
        self.player.on_ground = trace.normal.is_some_and(|n| n.z >= s.min_ground_normal);
        if self.player.on_ground {
            self.player.ground_entity = trace.entity;
        }
        if self.player.on_ground {
            self.player.velocity.z = 0.0;
        } else if trace.normal.is_some() && self.player.velocity.z > 0.0 {
            self.player.surface_friction = s.steep_slope_friction;
        }
    }

    /// Crouching: on the ground the view lowers over `crouch_time`, then the
    /// box shrinks from the top; in the air the box shrinks at once from the
    /// bottom, lifting the feet (a crouch jump reaches higher). Standing up
    /// needs room.
    fn crouch(&mut self, wants: bool) {
        let s = *self.settings;
        let rate = self.dt / s.crouch_time;
        let p = &mut *self.player;
        let lift = UP * (s.stand_height - s.crouch_height);
        if wants {
            if p.crouched {
                return;
            }
            if p.on_ground {
                p.crouch_amount = (p.crouch_amount + rate).min(1.0);
                p.crouched = p.crouch_amount >= 1.0;
            } else {
                p.origin += lift;
                p.crouched = true;
                p.crouch_amount = 1.0;
            }
            return;
        }
        if !p.crouched {
            p.crouch_amount = (p.crouch_amount - rate).max(0.0);
            return;
        }
        let standing = hull(&s, false);
        if !p.on_ground && !self.world.is_solid(p.origin - lift, standing) {
            // In the air, the feet come back down.
            p.origin -= lift;
            p.crouched = false;
            p.crouch_amount = 0.0;
        } else if !self.world.is_solid(p.origin, standing) {
            // On the ground (or with no room below), stand up and let the view rise.
            p.crouched = false;
            p.crouch_amount = 1.0 - rate;
        }
    }

    /// Flying through everything at a fixed speed along the view: no
    /// acceleration, no friction. The velocity stays as it was last set, so
    /// leaving noclip keeps it.
    fn fly(&mut self, command: &PlayerCommand) {
        let s = self.settings;
        let forward = command.angles.forward() * command.forward.signum() * (command.forward != 0.0) as i32 as f32;
        let right = command.angles.to_quat() * -refract_core::axes::LEFT * command.side.signum() * (command.side != 0.0) as i32 as f32;
        let up = if command.jump { UP } else { Vec3::ZERO };
        let mut velocity = (forward + right + up).normalize_or_zero() * s.noclip_speed;
        if command.sprint {
            velocity *= s.noclip_run_factor;
        }
        if command.crouch {
            velocity *= s.noclip_crouch_factor;
        }
        self.player.velocity = velocity;
        self.player.origin += velocity * self.dt;
        self.player.on_ground = false;
        self.player.ground_entity = None;
    }
}

/// The velocity with its part into the surface removed.
fn clip_velocity(velocity: Vec3, normal: Vec3) -> Vec3 {
    let mut out = velocity - normal * velocity.dot(normal);
    // Rounding can leave it slightly into the surface.
    let into = out.dot(normal);
    if into < 0.0 {
        out -= normal * into;
    }
    out
}

/// Speed along the ground.
pub fn ground_speed(velocity: Vec3) -> f32 {
    horizontal(velocity).length()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collision::{Brush, Collision, Plane};
    use crate::TICK;

    fn level(extra: Vec<Brush>) -> Collision {
        let mut brushes = vec![Brush::cuboid(Vec3::new(-4096.0, -4096.0, -64.0), Vec3::new(4096.0, 4096.0, 0.0))];
        brushes.extend(extra);
        Collision::new(brushes)
    }

    fn run(player: &mut Player, world: &Collision, command: PlayerCommand, ticks: usize) {
        let settings = MoveSettings::default();
        for _ in 0..ticks {
            player.step(&command, world, &settings, TICK);
        }
    }

    fn forward() -> PlayerCommand {
        PlayerCommand { forward: 1.0, ..Default::default() }
    }

    fn settled(world: &Collision, at: Vec3) -> Player {
        let mut player = Player::new(at + UP);
        run(&mut player, world, PlayerCommand::default(), 10);
        assert!(player.on_ground);
        player
    }

    #[test]
    fn walking_and_running_reach_their_speeds() {
        let world = level(vec![]);
        let mut player = settled(&world, Vec3::ZERO);
        run(&mut player, &world, forward(), 66);
        assert!((ground_speed(player.velocity) - 200.0).abs() < 1.0, "{}", player.velocity);
        run(&mut player, &world, PlayerCommand { sprint: true, ..forward() }, 66);
        assert!((ground_speed(player.velocity) - 400.0).abs() < 1.0, "{}", player.velocity);
        assert!(player.origin.z.abs() < 0.1);
        // Friction stops the player within a fraction of a second.
        run(&mut player, &world, PlayerCommand::default(), 40);
        assert_eq!(player.velocity, Vec3::ZERO);
    }

    #[test]
    fn jump_reaches_its_height_and_lands() {
        let world = level(vec![]);
        let mut player = settled(&world, Vec3::ZERO);
        let settings = MoveSettings::default();
        let mut top: f32 = 0.0;
        player.step(&PlayerCommand { jump: true, ..Default::default() }, &world, &settings, TICK);
        assert!(!player.on_ground);
        for _ in 0..100 {
            player.step(&PlayerCommand { jump: true, ..Default::default() }, &world, &settings, TICK);
            top = top.max(player.origin.z);
        }
        let expected = settings.jump_speed * settings.jump_speed / (2.0 * settings.gravity);
        assert!((top - expected).abs() < 2.0, "apex {top}, expected {expected}");
        assert!(player.on_ground, "landed");
        // Still holding jump: no new jump.
        player.step(&PlayerCommand { jump: true, ..Default::default() }, &world, &settings, TICK);
        assert!(player.on_ground);
    }

    #[test]
    fn steps_up_to_the_step_height_only() {
        for (height, climbs) in [(16.0, true), (18.0, true), (24.0, false)] {
            let world = level(vec![Brush::cuboid(Vec3::new(64.0, -256.0, 0.0), Vec3::new(512.0, 256.0, height))]);
            let mut player = settled(&world, Vec3::ZERO);
            run(&mut player, &world, forward(), 120);
            assert_eq!(player.origin.x > 100.0, climbs, "step {height}: {}", player.origin);
            if climbs {
                assert!((player.origin.z - height).abs() < 0.1 && player.on_ground);
            }
        }
    }

    #[test]
    fn walks_down_stairs_without_leaving_the_ground() {
        let stairs = (0..8).map(|i| Brush::cuboid(Vec3::new(i as f32 * 32.0, -128.0, 0.0), Vec3::new(i as f32 * 32.0 + 32.0, 128.0, 128.0 - i as f32 * 16.0))).collect();
        let world = level(stairs);
        let mut player = settled(&world, Vec3::new(16.0, 0.0, 128.0));
        let settings = MoveSettings::default();
        for _ in 0..80 {
            player.step(&forward(), &world, &settings, TICK);
            if player.origin.x < 250.0 {
                assert!(player.on_ground, "airborne at {}", player.origin);
            }
        }
    }

    #[test]
    fn walks_up_gentle_slopes_and_slides_off_steep_ones() {
        // 30°: walkable.
        let gentle = level(vec![Brush::ramp(Vec3::new(64.0, -256.0, 0.0), Vec3::new(64.0 + 256.0 * 3f32.sqrt(), 256.0, 256.0), 0.0)]);
        let mut player = Player::new(Vec3::new(300.0, 0.0, 300.0));
        run(&mut player, &gentle, PlayerCommand::default(), 200);
        assert!(player.on_ground && player.velocity.length() < 1.0, "rests on a 30° slope: {:?}", player);

        // 60°: slides down.
        let steep = level(vec![Brush::ramp(Vec3::new(0.0, -256.0, 0.0), Vec3::new(256.0 / 3f32.sqrt(), 256.0, 256.0), 0.0)]);
        let mut player = Player::new(Vec3::new(20.0, 0.0, 300.0));
        run(&mut player, &steep, PlayerCommand::default(), 30);
        assert!(!player.on_ground && player.velocity.x > 50.0, "slides off a 60° slope: {:?}", player);
    }

    #[test]
    fn strafing_in_the_air_gains_speed() {
        let world = Collision::default();
        let settings = MoveSettings::default();
        let mut player = Player::new(Vec3::new(0.0, 0.0, 100_000.0));
        player.velocity = Vec3::new(300.0, 0.0, 0.0);
        for _ in 0..66 {
            // Look along the motion and strafe right: the turn follows the speed gained.
            let yaw = player.velocity.y.atan2(player.velocity.x).to_degrees();
            let command = PlayerCommand { angles: Angles::new(0.0, yaw, 0.0), side: 1.0, ..Default::default() };
            player.step(&command, &world, &settings, TICK);
        }
        // Each tick adds the air speed cap at a right angle: v² grows by 30² per tick.
        assert!(ground_speed(player.velocity) > 380.0, "{}", ground_speed(player.velocity));
        // Holding forward in the air adds almost nothing beyond the cap.
        let mut player = Player::new(Vec3::new(0.0, 0.0, 100_000.0));
        player.velocity = Vec3::new(300.0, 0.0, 0.0);
        run(&mut player, &world, forward(), 66);
        assert!((ground_speed(player.velocity) - 300.0).abs() < 1.0);
    }

    #[test]
    fn crouching_fits_under_low_ceilings_and_stands_up_after() {
        // A tunnel 48 high from x = 64 to 256.
        let world = level(vec![Brush::cuboid(Vec3::new(64.0, -128.0, 48.0), Vec3::new(256.0, 128.0, 128.0))]);
        let mut player = settled(&world, Vec3::ZERO);
        run(&mut player, &world, forward(), 66);
        assert!(player.origin.x < 64.0, "blocked standing");
        run(&mut player, &world, PlayerCommand { crouch: true, ..forward() }, 300);
        assert!(player.origin.x > 64.0 && player.crouched);
        // Under the ceiling, letting go of crouch keeps the player crouched.
        let mut inside = player;
        inside.origin.x = 150.0;
        run(&mut inside, &world, PlayerCommand::default(), 5);
        assert!(inside.crouched);
        // Past the tunnel it stands up.
        let mut out = player;
        out.origin.x = 400.0;
        run(&mut out, &world, PlayerCommand::default(), 30);
        assert!(!out.crouched && out.crouch_amount == 0.0);
    }

    #[test]
    fn crouch_jump_climbs_higher_than_a_jump() {
        let ledge = |height: f32| level(vec![Brush::cuboid(Vec3::new(120.0, -128.0, 0.0), Vec3::new(256.0, 128.0, height))]);
        let try_climb = |height: f32, crouch: bool| {
            let world = ledge(height);
            let mut player = settled(&world, Vec3::ZERO);
            // Run up, then jump.
            run(&mut player, &world, forward(), 20);
            run(&mut player, &world, PlayerCommand { jump: true, crouch, ..forward() }, 60);
            player.origin.z > height - 1.0
        };
        assert!(try_climb(30.0, false));
        assert!(!try_climb(50.0, false));
        assert!(try_climb(50.0, true));
    }

    /// A ridge with 60° sides along X, like a surf ramp.
    fn surf_ramp() -> Collision {
        let (center, height) = (950.0f32, 320.0f32);
        let half_width = height / 60f32.to_radians().tan();
        let planes = [
            Plane::new(-Vec3::Z, Vec3::ZERO),
            Plane::new(Vec3::X, Vec3::new(3000.0, 0.0, 0.0)),
            Plane::new(-Vec3::X, Vec3::new(200.0, 0.0, 0.0)),
            Plane::new(Vec3::new(0.0, -height, half_width), Vec3::new(0.0, center - half_width, 0.0)),
            Plane::new(Vec3::new(0.0, height, half_width), Vec3::new(0.0, center + half_width, 0.0)),
        ];
        level(vec![Brush::new(&planes).unwrap()])
    }

    #[test]
    fn surfing_slides_along_the_ramp_without_sticking() {
        let world = surf_ramp();
        let settings = MoveSettings::default();
        let mut player = Player::new(Vec3::new(250.0, 890.0, 380.0));
        player.velocity = Vec3::new(600.0, 0.0, 0.0);
        // Hold towards the ramp (left, +Y) for three seconds.
        let command = PlayerCommand { side: -1.0, ..Default::default() };
        for tick in 0..200 {
            player.step(&command, &world, &settings, TICK);
            assert!(player.velocity.x > 550.0, "stopped at tick {tick}: {:?}", player);
        }
        assert!(!player.on_ground && player.origin.z > 100.0, "still on the ramp: {:?}", player);
    }

    #[test]
    fn noclip_stops_on_release_and_keeps_its_speed_when_left() {
        let world = level(vec![]);
        let settings = MoveSettings::default();
        let mut player = Player::new(Vec3::new(0.0, 0.0, 500.0));
        player.noclip = true;
        player.step(&forward(), &world, &settings, TICK);
        assert!((player.velocity.length() - settings.noclip_speed).abs() < 1e-3);
        player.step(&PlayerCommand::default(), &world, &settings, TICK);
        assert_eq!(player.velocity, Vec3::ZERO);

        player.step(&PlayerCommand { sprint: true, ..forward() }, &world, &settings, TICK);
        player.noclip = false;
        player.step(&PlayerCommand::default(), &world, &settings, TICK);
        assert!(player.velocity.x > 1400.0, "{}", player.velocity);
    }
}
