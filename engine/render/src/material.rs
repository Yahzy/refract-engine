//! Surface materials, metallic-roughness model.

use glam::{Vec3, Vec4};

use crate::TextureId;
use crate::gpu_types::GpuMaterial;

/// How a surface covers what is behind it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BlendMode {
    #[default]
    Opaque,
    /// Opaque where the base colour's alpha reaches the cutoff, holes elsewhere.
    AlphaTest,
    /// Blended over what is behind by its alpha.
    Translucent,
    /// Added to what is behind.
    Additive,
}

#[derive(Clone, Copy, Debug)]
pub struct Material {
    /// Linear RGB albedo (or specular colour of metals), and opacity;
    /// multiplies the base texture.
    pub base_color: Vec4,
    pub base_texture: Option<TextureId>,
    pub metallic: f32,
    pub roughness: f32,
    /// How much a non-metal reflects head-on (F0); 0.04 for most materials.
    pub reflectance: f32,
    /// Emitted light, linear RGB, added on top of the lit surface.
    pub emissive: Vec3,
    pub blend: BlendMode,
    /// Alpha below which an alpha-tested surface has a hole.
    pub alpha_cutoff: f32,
    /// Drawn from both sides.
    pub two_sided: bool,
}

impl Default for Material {
    fn default() -> Self {
        Self {
            base_color: Vec4::ONE,
            base_texture: None,
            metallic: 0.0,
            roughness: 0.5,
            reflectance: 0.04,
            emissive: Vec3::ZERO,
            blend: BlendMode::Opaque,
            alpha_cutoff: 0.5,
            two_sided: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MaterialId(pub(crate) u32);

pub const FLAG_ALPHA_TEST: u32 = 1;

impl From<&Material> for GpuMaterial {
    fn from(material: &Material) -> Self {
        GpuMaterial {
            base_color: material.base_color.into(),
            emissive: material.emissive.into(),
            metallic: material.metallic,
            roughness: material.roughness.clamp(0.0, 1.0),
            base_texture: material.base_texture.unwrap_or(TextureId::WHITE).0,
            alpha_cutoff: material.alpha_cutoff,
            flags: if material.blend == BlendMode::AlphaTest { FLAG_ALPHA_TEST } else { 0 },
            reflectance: material.reflectance,
            _pad: [0.0; 3],
        }
    }
}
