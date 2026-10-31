//! A scene built in code to exercise the renderer without content: a grid of
//! spheres sweeping roughness and metalness, a few blocks, a sun and moving
//! coloured lights.

use std::f32::consts::TAU;

use glam::{Mat4, Vec3, Vec4};
use refract_core::Angles;
use refract_render::{Camera, InstanceId, Material, PointLight, Renderer, SpotLight, Sun, shapes};

use crate::scene::{Scene, default_sky};

const SPHERE_RADIUS: f32 = 24.0;
const SPACING: f32 = 64.0;
const COLUMNS: usize = 7;

/// A light circling around a point.
struct Orbit {
    center: Vec3,
    radius: f32,
    /// Turns per second.
    speed: f32,
    phase: f32,
    color: Vec3,
    /// The emissive sphere showing where the light is.
    marker: InstanceId,
}

pub struct TestScene {
    orbits: Vec<Orbit>,
}

impl TestScene {
    pub fn build(renderer: &mut Renderer) -> Scene {
        let ground = renderer.add_mesh(&shapes::plane(4096.0, 64.0));
        let sphere = renderer.add_mesh(&shapes::sphere(SPHERE_RADIUS, 48, 24));
        let small_sphere = renderer.add_mesh(&shapes::sphere(4.0, 16, 8));
        let block = renderer.add_mesh(&shapes::cuboid(Vec3::new(32.0, 128.0, 64.0)));
        let pillar = renderer.add_mesh(&shapes::cuboid(Vec3::new(16.0, 16.0, 96.0)));

        let concrete = renderer.add_material(&Material { base_color: Vec4::new(0.45, 0.45, 0.43, 1.0), roughness: 0.85, ..Default::default() });
        renderer.add_instance(ground, concrete, Mat4::IDENTITY);

        // Rows: red plastic, then gold; columns: roughness from 0 to 1.
        let rows = [(Vec3::new(0.8, 0.05, 0.04), 0.0), (Vec3::new(1.0, 0.77, 0.34), 1.0)];
        for (row, (color, metallic)) in rows.into_iter().enumerate() {
            for column in 0..COLUMNS {
                let roughness = column as f32 / (COLUMNS - 1) as f32;
                let material = renderer.add_material(&Material { base_color: color.extend(1.0), metallic, roughness, ..Default::default() });
                let y = (column as f32 - (COLUMNS - 1) as f32 / 2.0) * -SPACING;
                let position = Vec3::new(row as f32 * SPACING, y, SPHERE_RADIUS);
                renderer.add_instance(sphere, material, Mat4::from_translation(position));
            }
        }

        let white = renderer.add_material(&Material { base_color: Vec4::new(0.8, 0.8, 0.8, 1.0), roughness: 0.6, ..Default::default() });
        let steel = renderer.add_material(&Material { base_color: Vec4::new(0.6, 0.6, 0.62, 1.0), metallic: 1.0, roughness: 0.3, ..Default::default() });
        renderer.add_instance(block, white, Mat4::from_translation(Vec3::new(260.0, -200.0, 64.0)));
        renderer.add_instance(block, white, Mat4::from_rotation_translation(glam::Quat::from_rotation_z(0.6), Vec3::new(240.0, 240.0, 64.0)));
        renderer.add_instance(pillar, steel, Mat4::from_translation(Vec3::new(-60.0, -260.0, 96.0)));
        renderer.add_instance(pillar, white, Mat4::from_translation(Vec3::new(380.0, 0.0, 96.0)));

        // (center, radius, turns per second, phase, colour)
        let orbit_specs = [
            (Vec3::new(32.0, 0.0, 70.0), 220.0, 0.08, 0.0, Vec3::new(1.0, 0.35, 0.1)),
            (Vec3::new(32.0, 0.0, 50.0), 160.0, -0.11, 0.33, Vec3::new(0.15, 0.45, 1.0)),
            (Vec3::new(32.0, 0.0, 90.0), 280.0, 0.05, 0.66, Vec3::new(0.3, 1.0, 0.4)),
        ];
        let orbits = orbit_specs
            .into_iter()
            .map(|(center, radius, speed, phase, color)| {
                let bulb = renderer.add_material(&Material { base_color: Vec4::new(0.0, 0.0, 0.0, 1.0), emissive: color * 40.0, ..Default::default() });
                let marker = renderer.add_instance(small_sphere, bulb, Mat4::IDENTITY);
                renderer.set_casts_shadows(marker, false);
                Orbit { center, radius, speed, phase, color, marker }
            })
            .collect();

        let spot_lights = vec![SpotLight {
            position: Vec3::new(380.0, 0.0, 300.0),
            direction: Angles::new(70.0, 180.0, 0.0).forward(),
            color: Vec3::new(1.0, 0.95, 0.85),
            intensity: 300.0,
            range: 900.0,
            inner_angle: 15.0,
            outer_angle: 25.0,
            casts_shadows: true,
            clearance: 0.0,
        }];

        let mut test = Self { orbits };
        let mut point_lights = Vec::new();
        test.update(renderer, 0.0, &mut point_lights);
        Scene {
            spawn: Camera { position: Vec3::new(-420.0, 0.0, 160.0), angles: Angles::new(14.0, 0.0, 0.0), ..Default::default() },
            sun: Sun { direction: Angles::new(50.0, 135.0, 0.0).forward(), color: Vec3::new(1.0, 0.95, 0.88), intensity: 8.0 },
            sky: default_sky(),
            ambient: Vec3::ZERO,
            exposure: 0.6,
            point_lights,
            spot_lights,
            test: Some(test),
        }
    }

    /// Moves the lights and their markers to where they are at `time`.
    pub fn update(&mut self, renderer: &mut Renderer, time: f32, lights: &mut Vec<PointLight>) {
        lights.clear();
        for orbit in &self.orbits {
            let angle = (orbit.phase + time * orbit.speed) * TAU;
            let position = orbit.center + Vec3::new(angle.cos(), angle.sin(), 0.0) * orbit.radius;
            lights.push(PointLight { position, color: orbit.color, intensity: 30.0, range: 600.0, casts_shadows: true, clearance: 0.0 });
            renderer.set_transform(orbit.marker, Mat4::from_translation(position));
        }
    }
}
