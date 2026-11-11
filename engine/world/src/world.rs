//! The world: the level's solid geometry, the physics objects (props) in
//! it, and how players move among them.
//!
//! Props are simulated by the physics engine. Players are not: they keep
//! their own movement, sweeping their box through the level and the props
//! near them as through brushes, and push props they run into and weigh on
//! those they stand on. Each player also has a kinematic box in the
//! physics world that follows it, so that props fall onto and rest on
//! players instead of going through them.

use glam::{Quat, Vec3};
use refract_core::axes::{METERS_PER_UNIT, UP};
use refract_physics::{BodyId, BodyKind, Filter, Material, Physics, Shape};

use crate::collision::{Brush, Collision, Hull, Plane, Solid, Sweep, Trace};
use crate::entity::{Entities, EntityId};
use crate::movement::{MoveSettings, Player, PlayerCommand};

/// Collision categories of physics shapes.
const LEVEL: u64 = 1;
const PROPS: u64 = 2;
const PLAYERS: u64 = 4;

/// Physics substeps per tick.
const SUBSTEPS: u32 = 4;
/// Props falling below this are removed.
const KILL_HEIGHT: f32 = -16384.0;
/// Held props move towards where they are held at most this fast (units/s).
const HOLD_SPEED: f32 = 1500.0;
/// The share of the way to where they are held that held props cover each
/// tick: below one, they ease in instead of overshooting.
const HOLD_STIFFNESS: f32 = 0.5;
/// Margin around a player's move within which props are swept against.
const NEARBY: f32 = 32.0;
/// The player's physics box is this much smaller than its movement box on
/// each side, and more below, so that it holds props off without rubbing
/// on what the player walks along or stands on.
const BODY_INSET: f32 = 0.5;
const BODY_INSET_BELOW: f32 = 2.0;

/// The shape of a prop, centered on its origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PropShape {
    Box { half_extents: Vec3 },
    Sphere { radius: f32 },
}

impl PropShape {
    /// Cubic meters, to turn a mass into a density.
    pub fn volume(&self) -> f32 {
        match *self {
            Self::Box { half_extents } => (half_extents * 2.0 * METERS_PER_UNIT).element_product(),
            Self::Sphere { radius } => 4.0 / 3.0 * std::f32::consts::PI * (radius * METERS_PER_UNIT).powi(3),
        }
    }
}

/// What to spawn.
#[derive(Clone, Copy, Debug)]
pub struct PropDesc {
    pub shape: PropShape,
    pub material: Material,
    pub position: Vec3,
    pub rotation: Quat,
    pub velocity: Vec3,
}

/// A physics object.
#[derive(Debug)]
pub struct Prop {
    pub shape: PropShape,
    pub position: Vec3,
    pub rotation: Quat,
    /// Position and rotation one tick earlier, to draw between ticks.
    pub previous: (Vec3, Quat),
    body: BodyId,
    /// What players collide with, in the prop's frame.
    brush: Brush,
    /// Held by a player: players go through it, so that it cannot carry
    /// the one holding it.
    held: bool,
}

impl Prop {
    /// Position and rotation `alpha` of the way from the previous tick to the last.
    pub fn interpolated(&self, alpha: f32) -> (Vec3, Quat) {
        (self.previous.0.lerp(self.position, alpha), self.previous.1.slerp(self.rotation, alpha))
    }
}

/// The player's box in the physics world.
struct PlayerBody {
    body: BodyId,
    crouched: bool,
}

pub struct World {
    collision: Collision,
    physics: Physics,
    props: Entities<Prop>,
    player_body: Option<PlayerBody>,
    gravity: f32,
    /// Reused query results.
    found: Vec<BodyId>,
}

impl World {
    /// A world around a level's brushes, with gravity in units/s² (downwards).
    pub fn new(collision: Collision, gravity: f32) -> Self {
        let mut physics = Physics::new(METERS_PER_UNIT, -UP * gravity);
        let level = physics.create_body(BodyKind::Static, Vec3::ZERO, Quat::IDENTITY, u64::MAX);
        let filter = Filter { category: LEVEL, mask: u64::MAX };
        for brush in collision.brushes() {
            if !physics.add_shape(level, Shape::Hull(&brush.vertices()), Material::default(), filter) {
                log::warn!("a brush at {:?} is too thin for physics", brush.bounds().min);
            }
        }
        Self { collision, physics, props: Entities::default(), player_body: None, gravity, found: Vec::new() }
    }

    /// The level's solid geometry.
    pub fn collision(&self) -> &Collision {
        &self.collision
    }

    pub fn spawn_prop(&mut self, desc: PropDesc) -> EntityId {
        let rotation = desc.rotation.normalize();
        let brush = match desc.shape {
            PropShape::Box { half_extents } => Brush::cuboid(-half_extents, half_extents),
            PropShape::Sphere { radius } => sphere_brush(radius),
        };
        let body = self.physics.create_body(BodyKind::Dynamic, desc.position, rotation, 0);
        let id = self.props.insert(Prop { shape: desc.shape, position: desc.position, rotation, previous: (desc.position, rotation), body, brush, held: false });
        self.physics.set_user(body, id.to_bits());
        let shape = match desc.shape {
            PropShape::Box { half_extents } => Shape::Box { half_extents, offset: Vec3::ZERO },
            PropShape::Sphere { radius } => Shape::Sphere { center: Vec3::ZERO, radius },
        };
        self.physics.add_shape(body, shape, desc.material, Filter { category: PROPS, mask: u64::MAX });
        self.physics.set_velocity(body, desc.velocity, Vec3::ZERO);
        id
    }

    pub fn remove_prop(&mut self, id: EntityId) -> bool {
        let Some(prop) = self.props.remove(id) else { return false };
        self.physics.destroy_body(prop.body);
        true
    }

    pub fn props(&self) -> impl Iterator<Item = (EntityId, &Prop)> {
        self.props.iter()
    }

    pub fn prop(&self, id: EntityId) -> Option<&Prop> {
        self.props.get(id)
    }

    /// Kilograms.
    pub fn prop_mass(&self, id: EntityId) -> Option<f32> {
        self.props.get(id).map(|prop| self.physics.mass(prop.body))
    }

    pub fn prop_velocity(&self, id: EntityId) -> Option<Vec3> {
        self.props.get(id).map(|prop| self.physics.velocity(prop.body))
    }

    /// Moves a held prop towards `position` and `rotation` over the next
    /// tick, against gravity. False if there is no such prop.
    pub fn hold_prop(&mut self, id: EntityId, position: Vec3, rotation: Quat, dt: f32) -> bool {
        let Some(prop) = self.props.get_mut(id) else { return false };
        prop.held = true;
        let linear = ((position - prop.position) * HOLD_STIFFNESS / dt).clamp_length_max(HOLD_SPEED) + UP * self.gravity * dt;
        let (axis, angle) = (rotation * prop.rotation.inverse()).normalize().to_axis_angle();
        // The shorter way round.
        let angle = if angle > std::f32::consts::PI { angle - std::f32::consts::TAU } else { angle };
        self.physics.set_velocity(prop.body, linear, axis * angle * HOLD_STIFFNESS / dt);
        true
    }

    /// Lets go of a held prop, sent off at `velocity` if given.
    pub fn release_prop(&mut self, id: EntityId, velocity: Option<Vec3>) {
        let Some(prop) = self.props.get_mut(id) else { return };
        prop.held = false;
        if let Some(velocity) = velocity {
            let angular = self.physics.angular_velocity(prop.body);
            self.physics.set_velocity(prop.body, velocity, angular);
        }
    }

    /// Sweeps a box through the level and the props.
    pub fn trace(&mut self, start: Vec3, end: Vec3, hull: Hull) -> Trace {
        let nearby = self.nearby(start.min(end) + hull.min, start.max(end) + hull.max);
        Nearby { level: &self.collision, props: &nearby }.trace(start, end, hull)
    }

    /// Moves a player by one tick among the level and the props, pushes
    /// the props it runs into and weighs on the one it stands on.
    pub fn move_player(&mut self, player: &mut Player, command: &PlayerCommand, settings: &MoveSettings, dt: f32) {
        let hull = player.hull(settings);
        let reach = player.velocity.abs() * dt + Vec3::splat(settings.step_height + NEARBY);
        let mut nearby = self.nearby(player.origin + hull.min - reach, player.origin + hull.max + reach);
        nearby.retain(|(id, _)| !self.props.get(*id).is_some_and(|prop| prop.held));
        let touches = player.step(command, &Nearby { level: &self.collision, props: &nearby }, settings, dt);

        for touch in touches {
            // Standing on it is weight, below; only sideways bumps push.
            if touch.normal.z >= settings.min_ground_normal {
                continue;
            }
            let Some(prop) = self.props.get(touch.entity) else { continue };
            let body = prop.body;
            let push = -touch.normal;
            let closing = touch.velocity.dot(push) - self.physics.velocity(body).dot(push);
            if closing <= 0.0 {
                continue;
            }
            // A collision without bounce between the player and the prop,
            // pushing at most with the player's weight: light props take
            // the player's speed, heavy ones creep or stay put.
            let mass = self.physics.mass(body);
            let reduced = mass * settings.mass / (mass + settings.mass);
            let impulse = (closing * reduced).min(settings.mass * self.gravity * dt);
            let center = player.origin + (hull.min + hull.max) * 0.5;
            let bounds = nearby.iter().find(|(id, _)| *id == touch.entity).map_or(prop.brush.bounds(), |(_, brush)| brush.bounds());
            let point = center.clamp(bounds.min, bounds.max);
            self.physics.apply_impulse(body, push * impulse, point);
            // The player was stopped against the prop; it follows at the
            // speed the push gave it, so pushing keeps up with walking.
            let follow = self.physics.velocity(body).dot(push).min(touch.velocity.dot(push));
            let current = player.velocity.dot(push);
            if follow > current {
                player.velocity += push * (follow - current);
            }
        }
        if let Some(prop) = player.ground_entity.and_then(|id| self.props.get(id)) {
            self.physics.apply_impulse(prop.body, -UP * settings.mass * self.gravity * dt, player.origin);
        }
        self.follow_player(player, settings, dt);
    }

    /// Advances the props by one tick.
    pub fn step(&mut self, dt: f32) {
        self.physics.step(dt, SUBSTEPS);
        let mut fallen = Vec::new();
        for (id, prop) in self.props.iter_mut() {
            prop.previous = (prop.position, prop.rotation);
            (prop.position, prop.rotation) = self.physics.transform(prop.body);
            if prop.position.z < KILL_HEIGHT {
                fallen.push(id);
            }
        }
        for id in fallen {
            self.remove_prop(id);
        }
    }

    /// The props whose bounds overlap `min..max`, as brushes where they are.
    fn nearby(&mut self, min: Vec3, max: Vec3) -> Vec<(EntityId, Brush)> {
        self.physics.bodies_in_box(min, max, PROPS, &mut self.found);
        self.found
            .iter()
            .filter_map(|body| {
                let id = EntityId::from_bits(self.physics.user(*body));
                let prop = self.props.get(id)?;
                Some((id, prop.brush.transformed(prop.rotation, prop.position)))
            })
            .collect()
    }

    /// Keeps the player's physics box on the player: moved by velocity, so
    /// that it pushes what it sweeps into, or put back in place after a
    /// jump (respawn). None while flying through everything.
    fn follow_player(&mut self, player: &Player, settings: &MoveSettings, dt: f32) {
        if player.noclip {
            if let Some(old) = self.player_body.take() {
                self.physics.destroy_body(old.body);
            }
            return;
        }
        if self.player_body.as_ref().is_some_and(|b| b.crouched != player.crouched) {
            let old = self.player_body.take().unwrap();
            self.physics.destroy_body(old.body);
        }
        let body = match &self.player_body {
            Some(b) => b.body,
            None => {
                let body = self.physics.create_body(BodyKind::Kinematic, player.origin, Quat::IDENTITY, u64::MAX);
                let hull = player.hull(settings);
                let min = hull.min + Vec3::new(BODY_INSET, BODY_INSET, BODY_INSET_BELOW);
                let max = hull.max - Vec3::splat(BODY_INSET);
                let shape = Shape::Box { half_extents: (max - min) * 0.5, offset: (max + min) * 0.5 };
                self.physics.add_shape(body, shape, Material::default(), Filter { category: PLAYERS, mask: PROPS });
                self.player_body = Some(PlayerBody { body, crouched: player.crouched });
                body
            }
        };
        let (position, _) = self.physics.transform(body);
        let offset = player.origin - position;
        if offset.length() > settings.max_speed * dt {
            self.physics.set_transform(body, player.origin, Quat::IDENTITY);
            self.physics.set_velocity(body, Vec3::ZERO, Vec3::ZERO);
        } else {
            self.physics.set_velocity(body, offset / dt, Vec3::ZERO);
        }
    }
}

/// The level and the props near a move.
struct Nearby<'a> {
    level: &'a Collision,
    props: &'a [(EntityId, Brush)],
}

impl Solid for Nearby<'_> {
    fn trace(&self, start: Vec3, end: Vec3, hull: Hull) -> Trace {
        let mut sweep = Sweep::new(start, end, hull);
        self.level.clip(&mut sweep);
        for (id, brush) in self.props {
            sweep.clip(brush, Some(*id));
        }
        sweep.finish()
    }
}

/// A convex solid around a sphere for players to collide with: the 26
/// planes facing the box's faces, edges and corners, touching the sphere.
fn sphere_brush(radius: f32) -> Brush {
    let mut planes = Vec::new();
    for x in -1..=1 {
        for y in -1..=1 {
            for z in -1..=1 {
                if (x, y, z) != (0, 0, 0) {
                    let normal = Vec3::new(x as f32, y as f32, z as f32).normalize();
                    planes.push(Plane { normal, dist: radius });
                }
            }
        }
    }
    Brush::new(&planes).expect("a sphere with positive radius")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TICK;

    const WOOD: Material = Material { density: 600.0, friction: 0.6, restitution: 0.0 };

    fn level() -> World {
        World::new(Collision::new(vec![Brush::cuboid(Vec3::new(-2048.0, -2048.0, -64.0), Vec3::new(2048.0, 2048.0, 0.0))]), 600.0)
    }

    /// A cube of `mass` kilograms.
    fn crate_desc(position: Vec3, half: f32, mass: f32) -> PropDesc {
        let shape = PropShape::Box { half_extents: Vec3::splat(half) };
        PropDesc { shape, material: Material { density: mass / shape.volume(), ..WOOD }, position, rotation: Quat::IDENTITY, velocity: Vec3::ZERO }
    }

    /// Runs ticks the way a game does: players move, then the props.
    fn run(world: &mut World, player: &mut Player, command: PlayerCommand, ticks: usize) {
        let settings = MoveSettings::default();
        for _ in 0..ticks {
            world.move_player(player, &command, &settings, TICK);
            world.step(TICK);
        }
    }

    fn forward() -> PlayerCommand {
        PlayerCommand { forward: 1.0, ..Default::default() }
    }

    #[test]
    fn players_stand_on_props() {
        let mut world = level();
        let id = world.spawn_prop(crate_desc(Vec3::new(0.0, 0.0, 16.0), 16.0, 50.0));
        let mut player = Player::new(Vec3::new(0.0, 0.0, 100.0));
        run(&mut world, &mut player, PlayerCommand::default(), 120);
        assert!(player.on_ground);
        assert_eq!(player.ground_entity, Some(id));
        assert!((player.origin.z - 32.0).abs() < 0.5, "{}", player.origin);
        // The crate holds the player's weight without sinking.
        assert!((world.prop(id).unwrap().position.z - 16.0).abs() < 0.5);
    }

    #[test]
    fn players_climb_low_props_like_steps() {
        let mut world = level();
        let id = world.spawn_prop(crate_desc(Vec3::new(100.0, 0.0, 8.0), 8.0, 50.0));
        let mut player = Player::new(Vec3::new(0.0, 0.0, 1.0));
        run(&mut world, &mut player, forward(), 40);
        assert_eq!(player.ground_entity, Some(id), "{}", player.origin);
    }

    #[test]
    fn light_props_get_pushed_and_heavy_ones_barely() {
        let distance = |mass: f32| {
            let mut world = level();
            // A crate 48 units high: too tall to step onto.
            let id = world.spawn_prop(crate_desc(Vec3::new(80.0, 0.0, 24.0), 24.0, mass));
            let mut player = Player::new(Vec3::new(0.0, 0.0, 1.0));
            run(&mut world, &mut player, forward(), 120);
            world.prop(id).unwrap().position.x - 80.0
        };
        let light = distance(10.0);
        let medium = distance(100.0);
        let heavy = distance(1000.0);
        // Walking at 200 u/s for 1.8 s, less the run-up.
        assert!(light > 200.0, "light crate moved {light}");
        assert!(medium > 10.0 && medium < light * 0.6, "medium crate moved {medium}, light {light}");
        assert!(heavy < 5.0, "heavy crate moved {heavy}");
    }

    #[test]
    fn props_rest_on_players() {
        let mut world = level();
        let mut player = Player::new(Vec3::new(0.0, 0.0, 1.0));
        run(&mut world, &mut player, PlayerCommand::default(), 5);
        let id = world.spawn_prop(crate_desc(Vec3::new(0.0, 0.0, 200.0), 8.0, 20.0));
        run(&mut world, &mut player, PlayerCommand::default(), 200);
        let z = world.prop(id).unwrap().position.z;
        assert!(z > 72.0 + 8.0 - 1.0, "the crate went into the player: {z}");
    }

    #[test]
    fn traces_report_the_prop_hit() {
        let mut world = level();
        let id = world.spawn_prop(crate_desc(Vec3::new(100.0, 0.0, 16.0), 16.0, 50.0));
        let hull = Hull { min: Vec3::splat(-4.0), max: Vec3::splat(4.0) };
        let trace = world.trace(Vec3::new(0.0, 0.0, 16.0), Vec3::new(200.0, 0.0, 16.0), hull);
        assert_eq!(trace.entity, Some(id));
        assert!((trace.end.x - (84.0 - 4.0)).abs() < 0.1, "{}", trace.end);
        let below = world.trace(Vec3::new(0.0, 0.0, 16.0), Vec3::new(0.0, 0.0, -16.0), hull);
        assert_eq!(below.entity, None);
        assert!(below.normal.is_some());
    }

    #[test]
    fn spheres_roll_and_fallen_props_are_removed() {
        let mut world = level();
        let ball = world.spawn_prop(PropDesc { shape: PropShape::Sphere { radius: 8.0 }, material: WOOD, position: Vec3::new(0.0, 0.0, 8.0), rotation: Quat::IDENTITY, velocity: Vec3::X * 200.0 });
        let lost = world.spawn_prop(crate_desc(Vec3::new(4000.0, 0.0, 100.0), 8.0, 20.0));
        for _ in 0..600 {
            world.step(TICK);
        }
        let prop = world.prop(ball).unwrap();
        assert!(prop.position.x > 300.0 && (prop.position.z - 8.0).abs() < 0.5, "{}", prop.position);
        assert!(prop.rotation.angle_between(Quat::IDENTITY) > 0.1, "it slid instead of rolling");
        assert!(world.prop(lost).is_none());
        assert_eq!(world.props().count(), 1);
    }

    #[test]
    fn held_props_go_where_they_are_held_and_let_players_through() {
        let mut world = level();
        let id = world.spawn_prop(crate_desc(Vec3::new(100.0, 0.0, 16.0), 16.0, 30.0));
        let target = Vec3::new(60.0, 40.0, 80.0);
        let turned = Quat::from_rotation_z(1.0);
        for _ in 0..60 {
            assert!(world.hold_prop(id, target, turned, TICK));
            world.step(TICK);
        }
        let prop = world.prop(id).unwrap();
        assert!(prop.position.distance(target) < 1.0, "{}", prop.position);
        assert!(prop.rotation.angle_between(turned) < 0.02);

        // Walking into it does not push it, and the player is not stopped.
        let mut player = Player::new(Vec3::new(0.0, 40.0, 1.0));
        let settings = MoveSettings::default();
        let command = PlayerCommand { forward: 1.0, ..Default::default() };
        for _ in 0..60 {
            world.hold_prop(id, target, turned, TICK);
            world.move_player(&mut player, &command, &settings, TICK);
            world.step(TICK);
        }
        assert!(player.origin.x > 100.0, "{}", player.origin);

        world.release_prop(id, Some(Vec3::X * 500.0));
        world.step(TICK);
        assert!(world.prop_velocity(id).unwrap().x > 400.0);
    }
}
