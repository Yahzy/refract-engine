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
    /// Texture scrolling, in UV units per second.
    pub scroll: Option<[f32; 2]>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Layer {
    pub base_color: Option<String>,
    pub normal: Option<String>,
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
            scroll: None,
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
