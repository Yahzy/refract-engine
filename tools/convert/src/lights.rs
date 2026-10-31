//! Light entities to the engine's lights.
//!
//! Brightness units: the engine shows a white surface lit with intensity 1 as
//! white. The `_light` brightness is scaled so that maps look as bright
//! as they were built to.

use glam::{Mat3, Vec3};
use refract_asset::map::{LIGHT_POINT, LIGHT_SPOT, Light, Sun};
use refract_core::Angles;
use refract_formats::bsp::MapEntity;

/// Sun intensity per unit of `_light` brightness.
const SUN_PER_BRIGHTNESS: f32 = 0.018;
/// Point and spot intensity per unit of `_light` brightness.
const LIGHT_PER_BRIGHTNESS: f32 = 0.2;
/// Shadow rays always ignore this much around a light (its bulb).
pub const MIN_CLEARANCE: f32 = 32.0;
/// Props larger than this are not fixtures (buildings, vehicles): a light
/// inside them keeps its shadows.
const MAX_FIXTURE_SIZE: f32 = 192.0;

/// A placed model's box: `rotation` and `origin` take model space to the
/// world, `scale` included.
pub struct PlacedBox {
    pub rotation: Mat3,
    pub origin: Vec3,
    pub scale: f32,
    pub mins: Vec3,
    pub maxs: Vec3,
}

/// Gives each light the room its fixture takes: maps put lights inside lamp
/// models, which must not swallow their light. A small prop whose box holds
/// the light is its fixture; the clearance reaches its farthest corner.
pub fn fit_fixtures(lights: &mut [Light], props: &[PlacedBox]) {
    for light in lights {
        let position = Vec3::from(light.position);
        let mut clearance = MIN_CLEARANCE;
        for prop in props {
            let size = (prop.maxs - prop.mins) * prop.scale;
            if size.max_element() > MAX_FIXTURE_SIZE {
                continue;
            }
            let local = prop.rotation.transpose() * (position - prop.origin) / prop.scale;
            let margin = Vec3::splat(2.0);
            if local.cmplt(prop.mins - margin).any() || local.cmpgt(prop.maxs + margin).any() {
                continue;
            }
            let far = (local - prop.mins).abs().max((prop.maxs - local).abs()) * prop.scale;
            clearance = clearance.max(far.length());
        }
        light.clearance = clearance;
    }
}

/// Flat ambient per unit of `_ambient` brightness.
const AMBIENT_PER_BRIGHTNESS: f32 = 0.0012;
/// `light` spawnflag: starts switched off.
const SF_INITIALLY_DARK: u32 = 1;

pub fn sun(environment: &MapEntity) -> Sun {
    let (color, brightness) = light_color(environment.get("_light"), [255.0, 245.0, 230.0, 400.0]);
    Sun { direction: light_direction(environment).to_array(), color, intensity: brightness * SUN_PER_BRIGHTNESS }
}

/// The flat ambient light of `light_environment`, linear RGB.
pub fn ambient(environment: Option<&MapEntity>) -> [f32; 3] {
    let (color, brightness) = light_color(environment.and_then(|e| e.get("_ambient")), [190.0, 200.0, 210.0, 300.0]);
    color.map(|c| c * brightness * AMBIENT_PER_BRIGHTNESS)
}

/// A `light` or `light_spot`, unless it starts switched off.
pub fn light(entity: &MapEntity) -> Option<Light> {
    let spawnflags: u32 = entity.get("spawnflags").and_then(|v| v.parse().ok()).unwrap_or(0);
    if spawnflags & SF_INITIALLY_DARK != 0 {
        return None;
    }
    let position = entity.vector("origin")?;
    let (color, brightness) = light_color(entity.get("_light"), [255.0, 255.0, 255.0, 200.0]);
    let mut light = Light {
        kind: LIGHT_POINT,
        position,
        range: range(entity, brightness),
        intensity: brightness * LIGHT_PER_BRIGHTNESS,
        color,
        ..Default::default()
    };
    if entity.classname == "light_spot" {
        let number = |key: &str, default: f32| entity.get(key).and_then(|v| v.parse::<f32>().ok()).unwrap_or(default);
        light.kind = LIGHT_SPOT;
        light.direction = light_direction(entity).to_array();
        light.outer_angle = number("_cone", 45.0).min(89.0);
        light.inner_angle = number("_inner_cone", 30.0).min(light.outer_angle);
    }
    Some(light)
}

/// Where a light points. Unlike view angles, a light's pitch is positive
/// pointing up, whether it comes from the `pitch` key or from `angles`. The
/// `angle` key, when set, overrides the yaw; -1 means straight up, -2 down.
fn light_direction(entity: &MapEntity) -> Vec3 {
    let number = |key: &str| entity.get(key).and_then(|v| v.parse::<f32>().ok()).filter(|v| *v != 0.0);
    let angles = entity.vector("angles").unwrap_or([0.0; 3]);
    match number("angle") {
        Some(-1.0) => return Vec3::Z,
        Some(-2.0) => return -Vec3::Z,
        _ => {}
    }
    let yaw = number("angle").unwrap_or(angles[1]);
    let pitch = number("pitch").unwrap_or(angles[0]);
    Angles::new(-pitch, yaw, 0.0).forward()
}

/// `"r g b brightness"` (sRGB bytes) as linear RGB and brightness. Without a
/// brightness (`"r g b"`), it is 200.
fn light_color(value: Option<&str>, default: [f32; 4]) -> ([f32; 3], f32) {
    let parts: Vec<f32> = value.unwrap_or("").split_whitespace().filter_map(|s| s.parse().ok()).collect();
    let [r, g, b, brightness] = match parts.as_slice() {
        [r, g, b, brightness, ..] => [*r, *g, *b, *brightness],
        [r, g, b] => [*r, *g, *b, 200.0],
        _ => default,
    };
    ([r, g, b].map(|c| srgb_to_linear(c / 255.0)), brightness.max(0.0))
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// Light radius in world units: `_zero_percent_distance` or `_distance` if
/// set, otherwise growing with brightness.
fn range(entity: &MapEntity, brightness: f32) -> f32 {
    ["_zero_percent_distance", "_distance"]
        .iter()
        .filter_map(|k| entity.get(k)?.parse::<f32>().ok())
        .find(|d| *d > 0.0)
        .unwrap_or_else(|| (brightness.sqrt() * 40.0).clamp(256.0, 2048.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(classname: &str, pairs: &[(&str, &str)]) -> MapEntity {
        MapEntity { classname: classname.into(), keyvalues: pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
    }

    #[test]
    fn sun_points_down() {
        let sun = sun(&entity("light_environment", &[("pitch", "-60"), ("angles", "0 90 0"), ("_light", "255 255 255 500")]));
        assert!(sun.direction[2] < -0.8 && sun.direction[1] > 0.4);
        assert!((sun.intensity - 9.0).abs() < 1e-4);
    }

    #[test]
    fn light_pitch_points_up_when_positive() {
        let up = light(&entity("light_spot", &[("origin", "0 0 0"), ("angles", "60 0 0")])).unwrap();
        assert!(up.direction[2] > 0.8, "angles pitch 60 points up: {:?}", up.direction);
        let down = light(&entity("light_spot", &[("origin", "0 0 0"), ("angles", "0 0 0"), ("angle", "-2")])).unwrap();
        assert_eq!(down.direction, [0.0, 0.0, -1.0]);
    }

    #[test]
    fn spot_and_switched_off_lights() {
        let spot = light(&entity("light_spot", &[("origin", "1 2 3"), ("_light", "255 0 0"), ("_cone", "40"), ("_inner_cone", "60")])).unwrap();
        assert_eq!(spot.kind, LIGHT_SPOT);
        assert_eq!((spot.inner_angle, spot.outer_angle), (40.0, 40.0));
        assert_eq!(spot.color, [1.0, 0.0, 0.0]);
        assert!(light(&entity("light", &[("origin", "0 0 0"), ("spawnflags", "1")])).is_none());
    }
}
