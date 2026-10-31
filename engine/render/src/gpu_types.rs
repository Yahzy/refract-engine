//! Data as the shaders read it. Layouts must match `shaders/lib/frame.slang`
//! (std140 for the uniform buffer, std430 for storage buffers).

use bytemuck::{Pod, Zeroable};

pub const NO_SHADOW: u32 = u32::MAX;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct GpuView {
    pub view_proj: [[f32; 4]; 4],
    pub inv_view_proj: [[f32; 4]; 4],
    pub cascades: [[[f32; 4]; 4]; 4],
    pub cascade_far: [f32; 4],
    pub cascade_texel: [f32; 4],
    pub position: [f32; 3],
    pub exposure: f32,
    pub forward: [f32; 3],
    pub light_count: u32,
    pub sun_direction: [f32; 3],
    pub local_shadow_texel: f32,
    pub sun_color: [f32; 3],
    pub time: f32,
    pub sky_color: [f32; 3],
    pub sun_shadow_texel: f32,
    pub horizon_color: [f32; 3],
    pub _pad0: f32,
    pub ground_color: [f32; 3],
    pub _pad1: f32,
    pub probe_origin: [f32; 3],
    /// 1 when the probe grid lights the scene, 0 for the sky's light.
    pub has_probes: u32,
    pub probe_resolution: [u32; 3],
    pub probe_cell: f32,
    pub flat_ambient: [f32; 3],
    /// 1 when the sky is a cubemap, 0 for the procedural sky.
    pub has_sky: u32,
    pub screen_size: [f32; 2],
    pub cluster_near: f32,
    pub cluster_far: f32,
    pub fog_color: [f32; 3],
    pub fog_density: f32,
    pub fog_height: f32,
    pub fog_falloff: f32,
    /// 0 without fog.
    pub fog_max_opacity: f32,
    pub fog_sky_density: f32,
}

/// Must match `shaders/lib/frame.slang`.
pub const CLUSTERS: u32 = 16 * 9 * 24;
pub const CLUSTER_MAX_LIGHTS: u32 = 128;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct GpuInstance {
    pub model: [[f32; 4]; 4],
    pub material: u32,
    /// Index of the nearest reflection cubemap, or [`NO_REFLECTION`].
    pub reflection: u32,
    pub flags: u32,
    pub _pad: u32,
}

/// Instance flag: distant scenery, fogged like the sky.
pub const INSTANCE_DISTANT: u32 = 1;

pub const NO_REFLECTION: u32 = u32::MAX;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct GpuMaterial {
    pub base_color: [f32; 4],
    pub emissive: [f32; 3],
    pub metallic: f32,
    pub roughness: f32,
    pub base_texture: u32,
    pub alpha_cutoff: f32,
    pub flags: u32,
    pub reflectance: f32,
    /// Texture indices; [`NO_TEXTURE`] when the material has none.
    pub normal_texture: u32,
    pub base_texture2: u32,
    pub normal_texture2: u32,
    pub blend_modulate: u32,
    pub detail_texture: u32,
    pub detail_scale: f32,
    pub detail_amount: f32,
    pub detail_tint: [f32; 3],
    pub detail_blend: u32,
}

pub const NO_TEXTURE: u32 = u32::MAX;

pub const LIGHT_POINT: u32 = 0;
pub const LIGHT_SPOT: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct GpuLight {
    pub position: [f32; 3],
    pub range: f32,
    pub color: [f32; 3],
    pub kind: u32,
    pub direction: [f32; 3],
    pub cos_outer: f32,
    pub cos_inner: f32,
    pub shadow: u32,
    /// Room around the light that shadow rays ignore (its fixture).
    pub clearance: f32,
    pub _pad: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_match_the_shaders() {
        assert_eq!(size_of::<GpuView>(), 624);
        assert_eq!(size_of::<GpuInstance>(), 80);
        assert_eq!(size_of::<GpuMaterial>(), 96);
        assert_eq!(size_of::<GpuLight>(), 64);
    }
}
