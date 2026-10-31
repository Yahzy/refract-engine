//! Materials (`.rmat`), as TOML:
//!
//! ```toml
//! kind = "alpha_test"
//! base_color = "textures/foliage/leaves01.ktx2"
//! alpha_cutoff = 0.5
//! roughness = 0.9
//! two_sided = true
//! ```
//!
//! Every field but `kind` may be left out.

use serde::{Deserialize, Serialize};

use crate::AssetError;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterialKind {
    #[default]
    Opaque,
    /// Opaque where the base colour's alpha reaches `alpha_cutoff`, holes elsewhere.
    AlphaTest,
    /// Blended over what is behind by its alpha.
    Translucent,
    /// Added to what is behind (glows, beams).
    Additive,
    Water,
    /// Shows the sky: the surface is not drawn, the sky behind it is.
    Sky,
    /// Never drawn (tool textures, triggers...).
    Invisible,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Material {
    pub kind: MaterialKind,
    /// Texture paths.
    pub base_color: Option<String>,
    pub normal: Option<String>,
    /// Linear RGB tint and opacity, multiplied with the base colour.
    pub color: [f32; 4],
    /// Average linear colour of the base texture times `color`: what the
    /// surface reflects, as light baking sees it.
    pub albedo: Option<[f32; 3]>,
    pub roughness: f32,
    pub metallic: f32,
    /// How much a non-metal reflects head-on (F0); 0.04 for most materials.
    pub reflectance: f32,
    /// Where the reflection cubemap this surface mirrors was taken, when the
    /// map chose one; otherwise the one nearest to the surface is used.
    pub reflection_at: Option<[f32; 3]>,
    pub alpha_cutoff: f32,
    pub two_sided: bool,
    /// Second layer, blended in by the vertex colour's alpha (terrain blends).
    pub layer2: Option<Layer>,
    /// A finer texture repeated over the base colour.
    pub detail: Option<Detail>,
    /// Texture scrolling, in UV units per second.
    pub scroll: Option<[f32; 2]>,
    /// How a `water` surface looks; its ripples are the `normal` map.
    pub water: Option<Water>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Layer {
    pub base_color: Option<String>,
    pub normal: Option<String>,
    /// Sharpens the blend: green is where the layers meet, red how soft
    /// the transition is.
    pub blend_modulate: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Detail {
    pub texture: String,
    /// How many times the detail repeats per repeat of the base texture.
    pub scale: f32,
    pub blend: DetailBlend,
    /// How strongly the detail applies, 0 to 1.
    pub amount: f32,
    /// Linear RGB, multiplied with the detail texture.
    pub tint: [f32; 3],
}

impl Default for Detail {
    fn default() -> Self {
        Self { texture: String::new(), scale: 4.0, blend: DetailBlend::Modulate2x, amount: 1.0, tint: [1.0; 3] }
    }
}

/// A water surface: what is behind it, seen through and fogged by the depth
/// of water crossed, mixed by Fresnel with what it reflects; both distorted
/// by the animated normal map.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Water {
    /// Linear RGB of the water's depths.
    pub fog_color: [f32; 3],
    /// Distances through the water (world units) where its colour starts
    /// hiding what is behind, and where it hides it fully.
    pub fog_start: f32,
    pub fog_end: f32,
    /// Linear RGB, multiplied with what the surface reflects.
    pub reflect_tint: [f32; 3],
    /// Linear RGB, multiplied with what is seen through the surface.
    pub refract_tint: [f32; 3],
    /// How strongly the ripples distort the reflection and what is seen through.
    pub reflect_amount: f32,
    pub refract_amount: f32,
    /// Normal map scrolling, UV units per second.
    pub scroll: [f32; 2],
    /// Two more layers of the normal map over the first, scrolling at these
    /// speeds (UV units per second).
    pub layers: Option<[[f32; 2]; 2]>,
    /// Frames per second through the normal map's array layers; 0 keeps the first.
    pub frame_rate: f32,
    /// The surface's underside, seen from under the water: no reflection.
    pub below: bool,
}

impl Default for Water {
    fn default() -> Self {
        Self {
            fog_color: [0.03, 0.16, 0.2],
            fog_start: 0.0,
            fog_end: 1024.0,
            reflect_tint: [1.0; 3],
            refract_tint: [1.0; 3],
            reflect_amount: 0.8,
            refract_amount: 0.0,
            scroll: [0.0; 2],
            layers: None,
            frame_rate: 0.0,
            below: false,
        }
    }
}

/// How the detail texture combines with the base colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetailBlend {
    /// Base × 2 × detail: mid-grey leaves the base unchanged.
    #[default]
    Modulate2x,
    /// Base + detail.
    Add,
    /// The detail over the base, by the detail's alpha.
    Over,
    /// The detail under the base, where the base's alpha is low.
    Under,
    /// Base × detail.
    Multiply,
    /// Light the detail gives off, added after lighting.
    Glow,
}

impl Default for Material {
    fn default() -> Self {
        Self {
            kind: MaterialKind::Opaque,
            base_color: None,
            normal: None,
            color: [1.0; 4],
            albedo: None,
            roughness: 0.8,
            metallic: 0.0,
            reflectance: 0.04,
            reflection_at: None,
            alpha_cutoff: 0.5,
            two_sided: false,
            layer2: None,
            detail: None,
            scroll: None,
            water: None,
        }
    }
}

impl Material {
    pub fn parse(text: &str) -> Result<Self, AssetError> {
        Ok(toml::from_str(text)?)
    }

    pub fn to_text(&self) -> String {
        toml::to_string(self).expect("materials serialize")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_defaults() {
        let material = Material { kind: MaterialKind::AlphaTest, base_color: Some("textures/a.ktx2".into()), two_sided: true, ..Default::default() };
        assert_eq!(Material::parse(&material.to_text()).unwrap(), material);

        let minimal = Material::parse("kind = \"water\"").unwrap();
        assert_eq!(minimal.kind, MaterialKind::Water);
        assert_eq!(minimal.color, [1.0; 4]);
        assert!(Material::parse("kind = \"opaque\"\ntypo = 1").is_err());
    }
}
