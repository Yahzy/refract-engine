//! The playground: a level built in code to move around in and to show
//! what the renderer does. A walled square with, around the spawn:
//!
//! - north-west, movement: stairs, steps too tall to walk up, gentle and
//!   steep ramps, boxes of growing height (jump, crouch jump);
//! - north-east, a surf ramp below a high platform reached by a long ramp;
//! - east, platforms to jump between;
//! - south-west, a hall with coloured lights, crates and a crouch tunnel;
//! - south-east, materials: spheres sweeping roughness and metalness,
//!   metal pillars, a glossy floor, lights circling around.

use glam::{Mat4, Vec3, Vec4};
use refract_core::Angles;
use refract_render::{Camera, Material, PointLight, Renderer, SpotLight, Sun, TextureId, shapes};
use refract_world::{Brush, MoveSettings, Plane};

use crate::level::{LevelBuilder, Surface};
use crate::scene::{Orbit, Scene};
use crate::textures;

/// Half the size of the walled square.
const HALF: f32 = 1536.0;
const WALL_HEIGHT: f32 = 512.0;
/// Average of the grid texture, linear.
const GRID_ALBEDO: f32 = 0.6;

pub const SPAWN: Vec3 = Vec3::new(-1300.0, 0.0, 0.0);
pub const SPAWN_YAW: f32 = 0.0;

struct Surfaces {
    floor: Surface,
    wall: Surface,
    orange: Surface,
    blue: Surface,
    green: Surface,
    concrete: Surface,
    steel: Surface,
    gold: Surface,
    copper: Surface,
    glossy: Surface,
    hazard: Surface,
    light_strip: Surface,
}

impl Surfaces {
    fn new(level: &mut LevelBuilder) -> Self {
        let grid = level.renderer.add_texture(&textures::grid(), "grid");
        let tiles = level.renderer.add_texture(&textures::tile_normals(), "tile normals");
        let gridded = |level: &mut LevelBuilder, color: Vec3, roughness: f32| {
            level.surface(&Material { base_color: color.extend(1.0), base_texture: Some(grid), normal_texture: Some(tiles), roughness, ..Default::default() }, GRID_ALBEDO)
        };
        let floor = gridded(level, Vec3::new(0.4, 0.4, 0.39), 0.6);
        let wall = gridded(level, Vec3::new(0.72, 0.7, 0.66), 0.8);
        let orange = gridded(level, Vec3::new(0.95, 0.42, 0.1), 0.55);
        let blue = gridded(level, Vec3::new(0.2, 0.42, 0.95), 0.35);
        let green = gridded(level, Vec3::new(0.3, 0.75, 0.25), 0.6);
        let plain = |level: &mut LevelBuilder, color: Vec3, metallic: f32, roughness: f32, normal: Option<TextureId>| {
            level.surface(&Material { base_color: color.extend(1.0), metallic, roughness, normal_texture: normal, ..Default::default() }, 1.0)
        };
        Self {
            floor,
            wall,
            orange,
            blue,
            green,
            concrete: plain(level, Vec3::new(0.4, 0.4, 0.38), 0.0, 0.9, Some(tiles)),
            steel: plain(level, Vec3::new(0.6, 0.6, 0.62), 1.0, 0.25, None),
            gold: plain(level, Vec3::new(1.0, 0.77, 0.34), 1.0, 0.2, None),
            copper: plain(level, Vec3::new(0.95, 0.64, 0.54), 1.0, 0.4, Some(tiles)),
            glossy: plain(level, Vec3::new(0.03, 0.03, 0.035), 0.0, 0.06, Some(tiles)),
            hazard: level.surface(&Material { base_color: Vec4::new(0.05, 0.12, 0.03, 1.0), emissive: Vec3::new(0.06, 0.25, 0.02), roughness: 0.3, ..Default::default() }, 1.0),
            light_strip: level.surface(&Material { base_color: Vec4::new(0.0, 0.0, 0.0, 1.0), emissive: Vec3::new(1.0, 0.92, 0.8) * 20.0, ..Default::default() }, 1.0),
        }
    }
}

pub fn build(renderer: &mut Renderer) -> Scene {
    let settings = MoveSettings::default();
    let spawn = Camera { position: SPAWN + Vec3::Z * settings.stand_eye, angles: Angles::new(0.0, SPAWN_YAW, 0.0), ..Default::default() };
    let sun = Sun { direction: Angles::new(40.0, 120.0, 0.0).forward(), color: Vec3::new(1.0, 0.93, 0.84), intensity: 6.0 };
    let mut scene = Scene::new(spawn, sun);
    scene.exposure = 0.8;

    let mut level = LevelBuilder::new(renderer);
    let s = Surfaces::new(&mut level);
    enclosure(&mut level, &s);
    movement(&mut level, &s);
    surf(&mut level, &s);
    jumps(&mut level, &s);
    hall(&mut level, &s);
    let orbits = showcase(&mut level, &s);
    for position in [Vec3::new(-1200.0, 0.0, 96.0), Vec3::new(-700.0, -1000.0, 96.0), Vec3::new(800.0, -1000.0, 96.0), Vec3::new(800.0, 0.0, 128.0), Vec3::new(-700.0, 900.0, 200.0), Vec3::new(700.0, 950.0, 400.0)] {
        level.reflection(position);
    }
    level.finish(&mut scene);
    scene.orbits = orbits;
    scene
}

/// The ground and the walls around it, with light strips along their top.
fn enclosure(level: &mut LevelBuilder, s: &Surfaces) {
    level.boxed(Vec3::new(-HALF, -HALF, -64.0), Vec3::new(HALF, HALF, 0.0), s.floor);
    let t = 64.0;
    level.boxed(Vec3::new(-HALF - t, -HALF - t, -64.0), Vec3::new(-HALF, HALF + t, WALL_HEIGHT), s.wall);
    level.boxed(Vec3::new(HALF, -HALF - t, -64.0), Vec3::new(HALF + t, HALF + t, WALL_HEIGHT), s.wall);
    level.boxed(Vec3::new(-HALF, -HALF - t, -64.0), Vec3::new(HALF, -HALF, WALL_HEIGHT), s.wall);
    level.boxed(Vec3::new(-HALF, HALF, -64.0), Vec3::new(HALF, HALF + t, WALL_HEIGHT), s.wall);
    // Spawn marker: a slab of concrete.
    level.boxed(SPAWN - Vec3::new(96.0, 96.0, 0.0), SPAWN + Vec3::new(96.0, 96.0, 2.0), s.concrete);
}

/// North-west: walking up, jumping up, sliding down.
fn movement(level: &mut LevelBuilder, s: &Surfaces) {
    // Eight steps of 16 up to a platform 128 high.
    for i in 0..8 {
        let x = -1100.0 + i as f32 * 32.0;
        level.boxed(Vec3::new(x, 600.0, 0.0), Vec3::new(x + 32.0, 792.0, 16.0 * (i + 1) as f32), s.orange);
    }
    // Next to them, steps of 24: too tall to walk, each needs a jump.
    for i in 0..5 {
        let x = -1100.0 + i as f32 * 48.0;
        level.boxed(Vec3::new(x, 808.0, 0.0), Vec3::new(x + 48.0, 1000.0, 24.0 * (i + 1) as f32), s.green);
    }
    level.boxed(Vec3::new(-860.0, 600.0, 0.0), Vec3::new(-500.0, 1000.0, 128.0), s.wall);
    // Down the other side: 30° (walkable) and 55° (slides).
    let gentle = 128.0 * 3f32.sqrt();
    level.brush(Brush::ramp(Vec3::new(-500.0, 600.0, 0.0), Vec3::new(-500.0 + gentle, 800.0, 128.0), 0.0), s.orange);
    let steep = 128.0 / 55f32.to_radians().tan();
    level.brush(Brush::ramp(Vec3::new(-500.0, 800.0, 0.0), Vec3::new(-500.0 + steep, 1000.0, 128.0), 0.0), s.blue);
    // Boxes 32, 48, 64 and 80 high: jump, crouch jump, barely, never.
    for (i, height) in [32.0, 48.0, 64.0, 80.0].into_iter().enumerate() {
        let x = -1100.0 + i as f32 * 160.0;
        level.boxed(Vec3::new(x, 1150.0, 0.0), Vec3::new(x + 96.0, 1246.0, height), s.orange);
    }
}

/// North-east: a long ramp up to a high platform, and a surf ramp below it.
fn surf(level: &mut LevelBuilder, s: &Surfaces) {
    let top = 384.0;
    // The platform, and the 30° ramp along the north wall leading to it.
    level.boxed(Vec3::new(-100.0, 850.0, 0.0), Vec3::new(200.0, 1400.0, top), s.wall);
    level.brush(Brush::ramp(Vec3::new(200.0, 1250.0, 0.0), Vec3::new(200.0 + top * 3f32.sqrt(), 1400.0, top), 0.0), s.orange);
    // The surf ramp: a ridge with 60° sides, running east from the platform.
    let (center, height) = (950.0, 320.0);
    let half_width = height / 60f32.to_radians().tan();
    let (x0, x1) = (200.0, 1400.0);
    let planes = [
        Plane::new(-Vec3::Z, Vec3::ZERO),
        Plane::new(Vec3::X, Vec3::new(x1, 0.0, 0.0)),
        Plane::new(-Vec3::X, Vec3::new(x0, 0.0, 0.0)),
        Plane::new(Vec3::new(0.0, -height, half_width), Vec3::new(0.0, center - half_width, 0.0)),
        Plane::new(Vec3::new(0.0, height, half_width), Vec3::new(0.0, center + half_width, 0.0)),
    ];
    level.brush(Brush::new(&planes).expect("surf ramp"), s.blue);
    level.point_light(PointLight { position: Vec3::new(50.0, 1125.0, top + 120.0), color: Vec3::new(0.6, 0.8, 1.0), intensity: 30.0, range: 900.0, casts_shadows: false, clearance: 0.0 });
}

/// East: platforms over a glowing floor, the gaps widening.
fn jumps(level: &mut LevelBuilder, s: &Surfaces) {
    level.boxed(Vec3::new(240.0, -300.0, 0.0), Vec3::new(1400.0, 300.0, 2.0), s.hazard);
    let mut x = 240.0;
    for gap in [0.0, 96.0, 128.0, 160.0, 192.0, 224.0] {
        x += gap;
        level.boxed(Vec3::new(x, -64.0, 0.0), Vec3::new(x + 96.0, 64.0, 64.0), if gap == 0.0 { s.orange } else { s.concrete });
        x += 96.0;
    }
    for x in [500.0, 1000.0] {
        level.point_light(PointLight { position: Vec3::new(x, 0.0, 220.0), color: Vec3::new(1.0, 0.85, 0.6), intensity: 25.0, range: 700.0, casts_shadows: false, clearance: 0.0 });
    }
}

/// South-west: a roofed hall, lit inside.
fn hall(level: &mut LevelBuilder, s: &Surfaces) {
    let (x0, x1, y0, y1, roof) = (-1100.0, -300.0, -1350.0, -650.0, 256.0);
    level.boxed(Vec3::new(x0, y0, roof), Vec3::new(x1, y1, roof + 32.0), s.concrete);
    // Back and west walls; the other sides stand on pillars.
    level.boxed(Vec3::new(x0, y0 - 32.0, 0.0), Vec3::new(x1, y0, roof), s.wall);
    level.boxed(Vec3::new(x0 - 32.0, y0 - 32.0, 0.0), Vec3::new(x0, y1, roof), s.wall);
    for (x, y) in [(x1 - 32.0, y1 - 32.0), (x1 - 32.0, -1000.0), (-700.0, y1 - 32.0)] {
        level.boxed(Vec3::new(x, y, 0.0), Vec3::new(x + 32.0, y + 32.0, roof), s.steel);
    }
    // Light strips under the roof.
    for y in [-1200.0, -1000.0, -800.0] {
        level.boxed(Vec3::new(x0 + 64.0, y - 4.0, roof - 4.0), Vec3::new(x1 - 64.0, y + 4.0, roof), s.light_strip);
    }
    // A crouch tunnel, 48 high.
    level.boxed(Vec3::new(-1000.0, -1250.0, 0.0), Vec3::new(-760.0, -1218.0, 96.0), s.wall);
    level.boxed(Vec3::new(-1000.0, -1150.0, 0.0), Vec3::new(-760.0, -1118.0, 96.0), s.wall);
    level.boxed(Vec3::new(-1000.0, -1250.0, 48.0), Vec3::new(-760.0, -1118.0, 96.0), s.orange);
    // Crates.
    for (x, y) in [(-600.0, -1250.0), (-536.0, -1250.0), (-568.0, -1186.0)] {
        level.boxed(Vec3::new(x, y, 0.0), Vec3::new(x + 64.0, y + 64.0, 64.0), s.copper);
    }
    level.boxed(Vec3::new(-584.0, -1234.0, 64.0), Vec3::new(-520.0, -1170.0, 128.0), s.copper);

    for (position, color) in [
        (Vec3::new(-950.0, -900.0, 120.0), Vec3::new(1.0, 0.35, 0.1)),
        (Vec3::new(-650.0, -1150.0, 120.0), Vec3::new(0.15, 0.5, 1.0)),
        (Vec3::new(-450.0, -850.0, 120.0), Vec3::new(1.0, 0.2, 0.6)),
    ] {
        level.point_light(PointLight { position, color, intensity: 20.0, range: 500.0, casts_shadows: false, clearance: 0.0 });
    }
    level.spot_light(SpotLight {
        position: Vec3::new(-700.0, -1000.0, roof - 8.0),
        direction: -Vec3::Z,
        color: Vec3::new(1.0, 0.95, 0.85),
        intensity: 60.0,
        range: 600.0,
        inner_angle: 20.0,
        outer_angle: 35.0,
        casts_shadows: false,
        clearance: 0.0,
    });
}

/// South-east: materials under moving lights.
fn showcase(level: &mut LevelBuilder, s: &Surfaces) -> Vec<Orbit> {
    let center = Vec3::new(800.0, -1000.0, 0.0);
    level.boxed(Vec3::new(240.0, -1400.0, 0.0), Vec3::new(1400.0, -600.0, 2.0), s.glossy);
    // Spheres on a plinth: red plastic and gold, roughness from 0 to 1.
    level.boxed(Vec3::new(560.0, -1100.0, 2.0), Vec3::new(1040.0, -900.0, 18.0), s.concrete);
    let radius = 24.0;
    let sphere = shapes::sphere(radius, 48, 24);
    for (row, (color, metallic)) in [(Vec3::new(0.8, 0.05, 0.04), 0.0), (Vec3::new(1.0, 0.77, 0.34), 1.0)].into_iter().enumerate() {
        for column in 0..7 {
            let roughness = column as f32 / 6.0;
            let surface = level.surface(&Material { base_color: color.extend(1.0), metallic, roughness, ..Default::default() }, 1.0);
            let position = Vec3::new(608.0 + column as f32 * 64.0, -1050.0 + row as f32 * 100.0, 18.0 + radius);
            level.mesh(sphere.clone(), surface, Mat4::from_translation(position));
            // Solid as a box.
            level.solid(Brush::cuboid(position - Vec3::splat(radius * 0.8), position + Vec3::splat(radius * 0.8)));
        }
    }
    for (i, surface) in [s.steel, s.gold, s.copper, s.steel].into_iter().enumerate() {
        let x = 400.0 + i as f32 * 280.0;
        level.boxed(Vec3::new(x, -1300.0, 2.0), Vec3::new(x + 40.0, -1260.0, 200.0), surface);
    }
    level.boxed(Vec3::new(1200.0, -800.0, 2.0), Vec3::new(1330.0, -670.0, 130.0), s.glossy);

    let small = level.renderer.add_mesh(&shapes::sphere(4.0, 16, 8));
    [(Vec3::new(1.0, 0.35, 0.1), 220.0, 0.08, 0.0, 70.0), (Vec3::new(0.15, 0.45, 1.0), 160.0, -0.11, 0.33, 50.0), (Vec3::new(0.3, 1.0, 0.4), 300.0, 0.05, 0.66, 90.0)]
        .into_iter()
        .map(|(color, radius, speed, phase, height)| {
            let bulb = level.renderer.add_material(&Material { base_color: Vec4::new(0.0, 0.0, 0.0, 1.0), emissive: color * 40.0, ..Default::default() });
            let marker = level.renderer.add_instance(small, bulb, Mat4::IDENTITY);
            level.renderer.set_casts_shadows(marker, false);
            Orbit { center: center + Vec3::Z * height, radius, speed, phase, color, intensity: 30.0, range: 600.0, marker }
        })
        .collect()
}
