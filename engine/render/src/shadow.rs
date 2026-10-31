//! Shadow views: where each shadow map looks from.
//!
//! The sun uses cascaded shadow maps: the view frustum is cut into slices,
//! each covered by its own orthographic map, finer near the camera. Each
//! slice's map is sized by its bounding sphere and snapped to whole texels, so
//! shadows do not shimmer when the camera moves or turns.
//!
//! Local lights share one array of smaller maps: a spot light takes one layer,
//! a point light six (one per cube face). When there are more shadowed lights
//! than layers, the ones nearest the camera get them.

use glam::{Mat4, Vec3, Vec4};

use crate::Camera;
use crate::culling::{Frustum, Sphere};
use crate::gpu_types::{GpuLight, LIGHT_POINT};

pub const CASCADES: usize = 4;
pub const SUN_RESOLUTION: u32 = 2048;
pub const LOCAL_RESOLUTION: u32 = 1024;
pub const LOCAL_LAYERS: u32 = 16;
/// How far from the camera the sun's shadows reach, in world units.
pub const SUN_DISTANCE: f32 = 6000.0;
/// Field of view of each cube face of a point light, a bit over 90° so that
/// filtering near a face's edge stays inside the map.
pub const POINT_FACE_FOV: f32 = 92.0;
/// Casters this far behind a cascade (towards the sun) still cast into it.
const CASTER_REACH: f32 = 4096.0;
/// Weight of the logarithmic split against the uniform one.
const SPLIT_LAMBDA: f32 = 0.8;

pub struct Cascade {
    pub view_proj: Mat4,
    /// View depth where this cascade ends.
    pub far: f32,
    /// Size of one shadow map texel, in world units.
    pub texel_size: f32,
}

/// The sun's cascades for `camera`, with `towards_sun` normalized.
pub fn sun_cascades(camera: &Camera, aspect: f32, towards_sun: Vec3) -> [Cascade; CASCADES] {
    let (near, far) = (camera.near, SUN_DISTANCE);
    let split = |i: usize| {
        let t = i as f32 / CASCADES as f32;
        SPLIT_LAMBDA * near * (far / near).powf(t) + (1.0 - SPLIT_LAMBDA) * (near + (far - near) * t)
    };
    std::array::from_fn(|i| {
        let (slice_near, slice_far) = (split(i), split(i + 1));
        let bounds = slice_sphere(camera, aspect, slice_near, slice_far);
        let radius = bounds.radius;
        let view = Mat4::look_to_rh(bounds.center + towards_sun * (radius + CASTER_REACH), -towards_sun, up_for(towards_sun));
        let projection = Mat4::orthographic_rh(-radius, radius, -radius, radius, 0.0, 2.0 * radius + CASTER_REACH);
        Cascade { view_proj: snap_to_texels(projection * view, SUN_RESOLUTION), far: slice_far, texel_size: 2.0 * radius / SUN_RESOLUTION as f32 }
    })
}

/// A spot light's view, covering its cone out to `range`.
pub fn spot_view(position: Vec3, direction: Vec3, outer_angle: f32, range: f32) -> Mat4 {
    let fov = (outer_angle * 2.0 + 2.0).min(170.0).to_radians();
    Mat4::perspective_rh(fov, 1.0, local_near(range), range) * Mat4::look_to_rh(position, direction, up_for(direction))
}

/// The six cube face views of a point light, in the order +X, -X, +Y, -Y, +Z, -Z
/// (the shader picks a face by the major axis of the light-to-surface vector).
pub fn point_views(position: Vec3, range: f32) -> [Mat4; 6] {
    let projection = Mat4::perspective_rh(POINT_FACE_FOV.to_radians(), 1.0, local_near(range), range);
    [Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z].map(|dir| projection * Mat4::look_to_rh(position, dir, up_for(dir)))
}

/// Gives shadow map layers to the lights that cast shadows (`casts[i]`), the
/// ones nearest the camera first, skipping lights out of view. Sets each
/// light's `shadow` to its first layer; returns the view of every used layer.
pub fn allocate_local_shadows(lights: &mut [GpuLight], casts: &[bool], camera: Vec3, view: &Frustum) -> Vec<Mat4> {
    let mut candidates: Vec<usize> = (0..lights.len())
        .filter(|&i| casts[i] && view.intersects(&Sphere { center: Vec3::from(lights[i].position), radius: lights[i].range }))
        .collect();
    let gap = |light: &GpuLight| (camera.distance(Vec3::from(light.position)) - light.range).max(0.0);
    candidates.sort_by(|&a, &b| gap(&lights[a]).total_cmp(&gap(&lights[b])));

    let mut views = Vec::new();
    for i in candidates {
        let light = &mut lights[i];
        let (position, range) = (Vec3::from(light.position), light.range);
        let layers = if light.kind == LIGHT_POINT { 6 } else { 1 };
        if views.len() + layers > LOCAL_LAYERS as usize {
            continue;
        }
        light.shadow = views.len() as u32;
        if light.kind == LIGHT_POINT {
            views.extend(point_views(position, range));
        } else {
            views.push(spot_view(position, Vec3::from(light.direction), light.cos_outer.acos().to_degrees(), range));
        }
    }
    views
}

fn local_near(range: f32) -> f32 {
    (range * 0.005).clamp(1.0, 8.0)
}

/// An up vector not parallel to `dir`.
fn up_for(dir: Vec3) -> Vec3 {
    if dir.z.abs() > 0.99 { Vec3::X } else { Vec3::Z }
}

/// The smallest sphere around the camera frustum between two view depths. Its
/// radius depends only on the lens, so the map's scale stays the same as the
/// camera turns.
fn slice_sphere(camera: &Camera, aspect: f32, near: f32, far: f32) -> Sphere {
    // A corner at view depth d lies k * d away from the view axis.
    let k2 = (camera.fov_y.to_radians() * 0.5).tan().powi(2) * (1.0 + aspect * aspect);
    // Depth of the point on the axis equally far from the near and far corners.
    let depth = ((far + near) * (1.0 + k2) * 0.5).min(far);
    let radius = ((far - depth).powi(2) + k2 * far * far).sqrt();
    Sphere { center: camera.position + camera.angles.forward() * depth, radius }
}

/// The 8 corners of the camera frustum between two view depths.
#[cfg(test)]
fn slice_corners(camera: &Camera, aspect: f32, near: f32, far: f32) -> [Vec3; 8] {
    let rotation = camera.angles.to_quat();
    let (forward, left, up) = (rotation * Vec3::X, rotation * Vec3::Y, rotation * Vec3::Z);
    let tan_half = (camera.fov_y.to_radians() * 0.5).tan();
    let mut corners = [Vec3::ZERO; 8];
    for (i, depth) in [near, far].into_iter().enumerate() {
        let (half_h, half_w) = (depth * tan_half, depth * tan_half * aspect);
        let center = camera.position + forward * depth;
        for (j, (sx, sy)) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].into_iter().enumerate() {
            corners[i * 4 + j] = center + left * (half_w * sx) + up * (half_h * sy);
        }
    }
    corners
}

/// Shifts the projection so that the world origin falls on a texel corner:
/// a moving camera then moves the map by whole texels only.
fn snap_to_texels(view_proj: Mat4, resolution: u32) -> Mat4 {
    let half = resolution as f32 * 0.5;
    let origin = view_proj * Vec4::W;
    let texels = origin.truncate().truncate() * half;
    let offset = (texels.round() - texels) / half;
    Mat4::from_translation(offset.extend(0.0)) * view_proj
}

#[cfg(test)]
mod tests {
    use super::*;
    use refract_core::Angles;

    fn camera() -> Camera {
        Camera { position: Vec3::new(100.0, -50.0, 64.0), angles: Angles::new(20.0, 30.0, 0.0), ..Default::default() }
    }

    #[test]
    fn cascades_cover_their_slices() {
        let camera = camera();
        let sun = Vec3::new(0.3, -0.4, 0.8).normalize();
        let cascades = sun_cascades(&camera, 16.0 / 9.0, sun);
        let mut near = camera.near;
        for cascade in &cascades {
            for corner in slice_corners(&camera, 16.0 / 9.0, near, cascade.far) {
                let clip = cascade.view_proj * corner.extend(1.0);
                assert!(clip.x.abs() <= 1.0 && clip.y.abs() <= 1.0, "corner outside the map: {clip}");
                assert!((0.0..=1.0).contains(&clip.z), "corner outside the depth range: {clip}");
            }
            near = cascade.far;
        }
        assert!((cascades[CASCADES - 1].far - SUN_DISTANCE).abs() < 1e-2);
    }

    #[test]
    fn moving_the_camera_moves_the_map_by_whole_texels() {
        let sun = Vec3::new(0.3, -0.4, 0.8).normalize();
        let mut camera = camera();
        let a = sun_cascades(&camera, 1.0, sun)[0].view_proj * Vec4::W;
        camera.position += Vec3::new(3.3, 1.7, 0.0);
        let b = sun_cascades(&camera, 1.0, sun)[0].view_proj * Vec4::W;
        let texels = (b - a).truncate().truncate() * SUN_RESOLUTION as f32 * 0.5;
        assert!((texels - texels.round()).abs().max_element() < 1e-2, "moved by {texels} texels");
    }

    #[test]
    fn nearest_lights_get_shadows_first() {
        use crate::{PointLight, SpotLight};
        let point = |x: f32| GpuLight::from(&PointLight { position: Vec3::new(x, 0.0, 0.0), color: Vec3::ONE, intensity: 1.0, range: 100.0, casts_shadows: true, clearance: 0.0 });
        let spot = GpuLight::from(&SpotLight {
            position: Vec3::new(300.0, 0.0, 0.0),
            direction: Vec3::X,
            color: Vec3::ONE,
            intensity: 1.0,
            range: 100.0,
            inner_angle: 20.0,
            outer_angle: 30.0,
            casts_shadows: true,
            clearance: 0.0,
        });
        // Far point light, near point light, a spot, and a light that casts no shadow.
        let mut lights = vec![point(2000.0), point(200.0), spot, point(250.0)];
        let casts = [true, true, true, false];
        let everything = Frustum::from_matrix(&Mat4::orthographic_rh(-1e5, 1e5, -1e5, 1e5, -1e5, 1e5));
        let views = allocate_local_shadows(&mut lights, &casts, Vec3::ZERO, &everything);
        assert_eq!(lights[1].shadow, 0);
        assert_eq!(lights[2].shadow, 6);
        // The far point light still fits: 6 + 1 + 6 layers of 16.
        assert_eq!(lights[0].shadow, 7);
        assert_eq!(lights[3].shadow, crate::gpu_types::NO_SHADOW);
        assert_eq!(views.len(), 13);
    }

    #[test]
    fn point_faces_see_their_axis() {
        let views = point_views(Vec3::ZERO, 500.0);
        for (view, dir) in views.iter().zip([Vec3::X, -Vec3::X, Vec3::Y, -Vec3::Y, Vec3::Z, -Vec3::Z]) {
            let clip = *view * (dir * 100.0).extend(1.0);
            let ndc = clip.truncate() / clip.w;
            assert!(ndc.x.abs() < 1e-4 && ndc.y.abs() < 1e-4 && (0.0..1.0).contains(&ndc.z));
        }
    }
}
