//! Materials: VMT to `.rmat`, converting the textures they use.

use refract_asset::material::{Detail, DetailBlend, Layer, Material, MaterialKind, Water};
use refract_formats::material::{self, MaterialInfo, parse_color, parse_number};

use crate::texture::Role;
use crate::{Converter, material_path};

/// Matte, as surfaces without `$phong` or `$envmap` look.
const MATTE: f32 = 0.85;
/// Surfaces that reflect their surroundings (`$envmap`).
const ENVMAP_ROUGHNESS: f32 = 0.2;

impl Converter {
    /// Converts `materials/<name>.vmt` and its textures; returns its native
    /// path. A missing material still gets a file (plain white), so that
    /// maps referring to it stay drawable.
    pub(crate) fn material(&self, name: &str) -> String {
        let path = material_path(name);
        if !self.claim(&path) {
            return path;
        }
        let material = match material::load_material(&self.fs, name) {
            Some(info) => self.convert_material(&info),
            None => {
                log::warn!("missing material {name}");
                Material::default()
            }
        };
        if let Err(err) = self.write(&path, material.to_text().as_bytes()) {
            log::warn!("{err:#}");
        }
        path
    }

    fn convert_material(&self, info: &MaterialInfo) -> Material {
        let var = |key: &str| info.vars.get(key).map(String::as_str);
        let set = |key: &str| var(key).and_then(parse_number).is_some_and(|v| v != 0.0);
        let tint = var("$color").or(var("$color2")).and_then(parse_color).unwrap_or([1.0; 3]);
        let alpha = var("$alpha").and_then(parse_number).unwrap_or(1.0);

        let kind = if info.shader == "water" {
            MaterialKind::Water
        } else if is_invisible(info) {
            MaterialKind::Invisible
        } else if info.additive {
            MaterialKind::Additive
        } else if info.alpha_test.is_some() {
            MaterialKind::AlphaTest
        } else if info.translucent || alpha < 1.0 {
            MaterialKind::Translucent
        } else {
            MaterialKind::Opaque
        };
        let envmap_tint = var("$envmaptint").and_then(parse_color).unwrap_or([1.0; 3]);
        let envmap_strength = (envmap_tint[0] * 0.2126 + envmap_tint[1] * 0.7152 + envmap_tint[2] * 0.0722).max(0.0);
        let roughness = if info.envmap {
            // Reflects its surroundings: glossy.
            ENVMAP_ROUGHNESS
        } else if set("$phong") {
            // Blinn-Phong exponent to GGX: alpha = sqrt(2 / (n + 2)), perceptual roughness = sqrt(alpha).
            let exponent = var("$phongexponent").and_then(parse_number).unwrap_or(5.0).max(1.0);
            (2.0 / (exponent + 2.0)).sqrt().sqrt().max(0.3)
        } else {
            MATTE
        };

        let texture = |name: &Option<String>, role| name.as_deref().and_then(|n| self.texture(n, role));
        let bump = if info.ssbump { Role::SelfShadowedBump } else { Role::Data };
        let layer2 = info.base_texture2.as_ref().map(|_| Layer {
            base_color: texture(&info.base_texture2, Role::Color),
            normal: texture(&info.bump_map2, bump),
            blend_modulate: texture(&info.blend_modulate, Role::Data),
        });
        let detail = info.detail.as_ref().and_then(|detail| {
            let blend = match detail.blend_mode {
                0 | 7 => DetailBlend::Modulate2x,
                1 => DetailBlend::Add,
                2 | 3 => DetailBlend::Over,
                4 => DetailBlend::Under,
                5 | 6 => DetailBlend::Glow,
                8 => DetailBlend::Multiply,
                _ => return None,
            };
            // Modulating details are factors around mid-grey, not colours.
            let role = if blend == DetailBlend::Modulate2x { Role::Data } else { Role::Color };
            Some(Detail {
                texture: self.texture(&detail.texture, role)?,
                scale: detail.scale,
                blend,
                amount: detail.blend_factor.clamp(0.0, 1.0),
                tint: var("$detailtint").and_then(parse_color).unwrap_or([1.0; 3]),
            })
        });
        Material {
            kind,
            base_color: texture(&info.base_texture, Role::Color),
            normal: texture(&info.bump_map, bump),
            color: [tint[0], tint[1], tint[2], alpha],
            albedo: Some(self.average_color(info.base_texture.as_deref()).map_or(tint, |c| [c[0] * tint[0], c[1] * tint[1], c[2] * tint[2]])),
            roughness,
            metallic: 0.0,
            reflectance: if info.envmap { (envmap_strength * 0.25).clamp(0.04, 0.25) } else { 0.04 },
            reflection_at: var("$envmap").and_then(cubemap_position),
            alpha_cutoff: info.alpha_test.unwrap_or(0.5),
            two_sided: info.no_cull,
            layer2,
            detail,
            scroll: None,
            water: (kind == MaterialKind::Water).then(|| water(info)),
        }
    }
}

/// The `water` shader's parameters, and the proxies that animate its normal map.
fn water(info: &MaterialInfo) -> Water {
    let var = |key: &str| info.vars.get(key).map(String::as_str);
    let number = |key: &str, default: f32| var(key).and_then(parse_number).unwrap_or(default);
    let defaults = Water::default();
    let fog_start = number("$fogstart", defaults.fog_start);
    let mut water = Water {
        fog_color: info.fog_color.unwrap_or(defaults.fog_color),
        fog_start,
        fog_end: number("$fogend", defaults.fog_end).max(fog_start + 1.0),
        reflect_tint: var("$reflecttint").and_then(parse_color).unwrap_or([1.0; 3]),
        refract_tint: var("$refracttint").and_then(parse_color).unwrap_or([1.0; 3]),
        reflect_amount: number("$reflectamount", defaults.reflect_amount),
        refract_amount: number("$refractamount", defaults.refract_amount),
        below: number("$abovewater", 1.0) == 0.0,
        ..defaults
    };
    // `$scroll1` with a speed along U adds two layers, scrolling at `$scroll1` and `$scroll2`.
    let scroll1 = var("$scroll1").and_then(parse_vector2).unwrap_or([0.0; 2]);
    if scroll1[0] != 0.0 {
        water.layers = Some([scroll1, var("$scroll2").and_then(parse_vector2).unwrap_or([0.0; 2])]);
    }
    for proxy in &info.proxies {
        let param = |key: &str| proxy.params.get(key).map(String::as_str);
        let targets_normals = |key: &str| param(key).is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "$bumptransform" | "$normalmap" | "$bumpmap" | "$bumpframe"));
        match proxy.name.as_str() {
            "texturescroll" if targets_normals("texturescrollvar") => {
                let rate = param("texturescrollrate").and_then(parse_number).unwrap_or(1.0);
                let angle = param("texturescrollangle").and_then(parse_number).unwrap_or(0.0).to_radians();
                water.scroll = [angle.cos() * rate, angle.sin() * rate];
            }
            "animatedtexture" if targets_normals("animatedtexturevar") || targets_normals("animatedtextureframenumvar") => {
                water.frame_rate = param("animatedtextureframerate").and_then(parse_number).unwrap_or(15.0).max(0.0);
            }
            _ => {}
        }
    }
    water
}

/// The first two numbers of `[x y ...]`.
fn parse_vector2(value: &str) -> Option<[f32; 2]> {
    let mut numbers = value.trim().trim_matches(|c| c == '[' || c == ']').split_whitespace().map(|n| n.parse::<f32>().ok());
    Some([numbers.next()??, numbers.next()??])
}

impl Converter {
    /// Average linear colour of a texture, from one of its small mips.
    fn average_color(&self, texture: Option<&str>) -> Option<[f32; 3]> {
        let texture = material::load_texture(&self.fs, texture?)?;
        let level = (0..texture.mips.len()).find(|&l| (texture.width >> l).max(1) * (texture.height >> l).max(1) <= 256).unwrap_or(texture.mips.len() - 1);
        let pixels = &texture.mips[level];
        let count = (pixels.len() / 4).max(1) as f32;
        let mut sum = [0.0f32; 3];
        for p in pixels.chunks_exact(4) {
            for k in 0..3 {
                sum[k] += srgb_to_linear(p[k] as f32 / 255.0);
            }
        }
        Some(sum.map(|s| s / count))
    }
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// The position of a map's own cubemap, from its texture name
/// (`maps/<map>/c<x>_<y>_<z>`), which compiled maps give each face's material.
fn cubemap_position(envmap: &str) -> Option<[f32; 3]> {
    let name = envmap.rsplit(['/', '\\']).next()?.strip_prefix('c')?;
    let name = name.strip_suffix(".hdr").unwrap_or(name);
    let mut parts = name.split('_').map(|p| p.parse::<f32>().ok());
    let position = [parts.next()??, parts.next()??, parts.next()??];
    parts.next().is_none().then_some(position)
}

/// Tool materials that only shape the map (clips, triggers, hints...).
fn is_invisible(info: &MaterialInfo) -> bool {
    let tool = |key: &str| info.vars.get(key).and_then(|v| parse_number(v)).is_some_and(|v| v != 0.0);
    tool("%compilenodraw") || tool("%compileclip") || tool("%compiletrigger") || tool("%playerclip") || tool("%compileinvisible")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vectors() {
        assert_eq!(parse_vector2("[.01 .01 .05]"), Some([0.01, 0.01]));
        assert_eq!(parse_vector2("[1]"), None);
    }

    #[test]
    fn cubemap_positions_from_names() {
        assert_eq!(cubemap_position("maps/test_map/c-3072_-1472_-96"), Some([-3072.0, -1472.0, -96.0]));
        assert_eq!(cubemap_position("maps/test_map/c87_-4109_63.hdr"), Some([87.0, -4109.0, 63.0]));
        assert_eq!(cubemap_position("env_cubemap"), None);
        assert_eq!(cubemap_position("maps/x/c1_2"), None);
    }
}
