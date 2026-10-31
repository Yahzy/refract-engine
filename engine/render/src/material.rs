//! Surface materials, metallic-roughness model.

use glam::{Vec3, Vec4};
pub use refract_asset::material::DetailBlend;

use crate::TextureId;
use crate::gpu_types::{GpuMaterial, NO_TEXTURE};

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
    /// Tangent-space normal map, green along increasing V.
    pub normal_texture: Option<TextureId>,
    /// Blended in by the vertex colour's alpha.
    pub layer2: Option<Layer>,
    pub detail: Option<Detail>,
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
            normal_texture: None,
            layer2: None,
            detail: None,
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

/// A second layer of base colour and normals (terrain blends).
#[derive(Clone, Copy, Debug)]
pub struct Layer {
    pub base_texture: TextureId,
    pub normal_texture: Option<TextureId>,
    /// Sharpens the blend: green is where the layers meet, red how softly.
    pub blend_modulate: Option<TextureId>,
}

/// A finer texture repeated over the base colour.
#[derive(Clone, Copy, Debug)]
pub struct Detail {
    pub texture: TextureId,
    /// Repeats per repeat of the base texture.
    pub scale: f32,
    pub blend: DetailBlend,
    /// 0 to 1.
    pub amount: f32,
    /// Linear RGB, multiplies the detail texture.
    pub tint: Vec3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MaterialId(pub(crate) u32);

pub const FLAG_ALPHA_TEST: u32 = 1;
pub const FLAG_ADDITIVE: u32 = 2;

impl From<&Material> for GpuMaterial {
    fn from(material: &Material) -> Self {
        let index = |texture: Option<TextureId>| texture.map_or(NO_TEXTURE, |t| t.0);
        let layer2 = material.layer2.as_ref();
        let detail = material.detail.as_ref();
        GpuMaterial {
            base_color: material.base_color.into(),
            emissive: material.emissive.into(),
            metallic: material.metallic,
            roughness: material.roughness.clamp(0.0, 1.0),
            base_texture: material.base_texture.unwrap_or(TextureId::WHITE).0,
            alpha_cutoff: material.alpha_cutoff,
            flags: match material.blend {
                BlendMode::AlphaTest => FLAG_ALPHA_TEST,
                BlendMode::Additive => FLAG_ADDITIVE,
                _ => 0,
            },
            reflectance: material.reflectance,
            normal_texture: index(material.normal_texture),
            base_texture2: index(layer2.map(|l| l.base_texture)),
            normal_texture2: index(layer2.and_then(|l| l.normal_texture)),
            blend_modulate: index(layer2.and_then(|l| l.blend_modulate)),
            detail_texture: index(detail.map(|d| d.texture)),
            detail_scale: detail.map_or(1.0, |d| d.scale),
            detail_amount: detail.map_or(0.0, |d| d.amount.clamp(0.0, 1.0)),
            detail_tint: detail.map_or([1.0; 3], |d| d.tint.into()),
            detail_blend: detail.map_or(0, |d| match d.blend {
                DetailBlend::Modulate2x => 0,
                DetailBlend::Add => 1,
                DetailBlend::Over => 2,
                DetailBlend::Under => 3,
                DetailBlend::Multiply => 4,
                DetailBlend::Glow => 5,
            }),
        }
    }
}
