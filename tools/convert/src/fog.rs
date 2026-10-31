//! The map's fog, turned from a linear blend between two distances into
//! height fog: exponential with distance, thinning with height.

use refract_asset::map::Fog;
use refract_formats::bsp::{Map, MapEntity};

use crate::lights::srgb_to_linear;

/// Height over which the fog thins by a factor e (about 100 m).
const FALLOFF: f32 = 4096.0;
/// The fog's colour is shown this bright, like the sky.
const BRIGHTNESS: f32 = 1.5;
/// Below this, the distance where a fog is half opaque is taken as this.
const MIN_HALF_DISTANCE: f32 = 64.0;

/// The fog of the map's `env_fog_controller`, if it is on. The fog is half
/// opaque where the map's linear fog was halfway (between its start and end
/// distances), then keeps thickening smoothly instead of hitting a wall at
/// the end distance. The 3D skybox's fog (set on `sky_camera`, in skybox
/// units) gives the density of the far scenery and the sky.
pub fn fog(map: &Map) -> Option<Fog> {
    let controller = map.find_entities("env_fog_controller").find(|e| enabled(e))?;
    let world = density(controller, 1.0);
    let sky_density = match (map.find_entities("sky_camera").find(|e| enabled(e)), &map.sky_camera) {
        (Some(camera), Some(sky)) => density(camera, sky.scale),
        _ => world,
    };
    let color = number_list(controller.get("fogcolor")).map_or([0.5; 3], |[r, g, b]| [r, g, b].map(|c| srgb_to_linear(c / 255.0) * BRIGHTNESS));
    let max_opacity = controller.get("fogmaxdensity").and_then(|v| v.parse().ok()).unwrap_or(1.0f32).clamp(0.0, 1.0);
    let height = map.find_entities("info_player_start").find_map(|e| number_list::<3>(e.get("origin"))).map_or(0.0, |o| o[2]);
    Some(Fog { color, density: world, height, falloff: FALLOFF, max_opacity, sky_density })
}

fn enabled(entity: &MapEntity) -> bool {
    entity.get("fogenable").is_some_and(|v| v.trim() != "0")
}

/// Extinction per world unit for the fog distances of `entity`, which are
/// `scale` world units each.
fn density(entity: &MapEntity, scale: f32) -> f32 {
    let distance = |key: &str| entity.get(key).and_then(|v| v.parse::<f32>().ok());
    let start = distance("fogstart").unwrap_or(0.0).max(0.0);
    let end = distance("fogend").unwrap_or(10000.0).max(start);
    let half = ((start + end) * 0.5 * scale).max(MIN_HALF_DISTANCE);
    std::f32::consts::LN_2 / half
}

fn number_list<const N: usize>(value: Option<&str>) -> Option<[f32; N]> {
    let numbers: Vec<f32> = value?.split_whitespace().filter_map(|s| s.parse().ok()).collect();
    numbers.get(..N)?.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_opaque_halfway() {
        let entity = MapEntity { classname: "env_fog_controller".into(), keyvalues: vec![("fogstart".into(), "1000".into()), ("fogend".into(), "3000".into())] };
        let density = density(&entity, 1.0);
        assert!(((-density * 2000.0).exp() - 0.5).abs() < 1e-5);
        assert!((super::density(&entity, 16.0) * 16.0 - density).abs() < 1e-9);
    }
}
