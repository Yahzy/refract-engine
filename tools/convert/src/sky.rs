//! The 2D sky as an HDR cubemap in the engine's convention: sampled with a
//! world direction (X forward, Y left, Z up), standard cube face layout.
//!
//! Either the map's six skybox textures, resampled, or the painted sky
//! (`skyname painted`, set by `env_skypaint`) evaluated for every texel.

use std::borrow::Cow;

use glam::{Vec2, Vec3};
use refract_asset::{TextureData, TextureFormat};
use refract_render::Sky;
use refract_formats::bsp::{Map, MapEntity};
use refract_formats::material::{Texture, load_material, load_skybox, load_texture};

use crate::Converter;

const FACE_SIZE: u32 = 256;
/// The sky is shown this bright (a white texel gives 1.5).
const BRIGHTNESS: f32 = 1.5;
/// How much of the map's ambient colour the sky's light takes (0 grey, 1 all).
const SKY_TINT: f32 = 0.5;

impl Converter {
    /// Converts the map's sky; returns its native path, and the light it
    /// casts as a gradient (for baking).
    pub(crate) fn sky(&self, map: &Map) -> Option<(String, Sky)> {
        let skyname = map.worldspawn().and_then(|w| w.get("skyname")).unwrap_or("").to_ascii_lowercase();
        let painted = skyname == "painted" || map.find_entities("env_skypaint").next().is_some();
        let (path, color): (String, Box<dyn Fn(Vec3) -> Vec3>) = if painted {
            let paint = SkyPaint::from_map(self, map);
            (format!("maps/{}.sky.ktx2", map.name.to_ascii_lowercase()), Box::new(move |dir| paint.color(dir).max(Vec3::ZERO).powf(2.2)))
        } else {
            if skyname.is_empty() {
                return None;
            }
            let faces = load_skybox(&self.fs, &skyname)?;
            (format!("textures/skybox/{skyname}.ktx2"), Box::new(move |dir| sample_skybox(&faces, dir)))
        };
        let light = |dir: Vec3| color(dir.normalize()) * BRIGHTNESS;
        // How much light comes from the sky; its colour is the one the map
        // gives its ambient light, halfway to grey, as the textures' blue is
        // far too strong and shadows lit by the sky alone would look blue.
        let mut gradient = gradient(&light);
        if let Some(environment) = map.find_entities("light_environment").next() {
            let tint = Vec3::from(crate::lights::ambient(Some(environment)));
            if luminance(tint) > 1e-6 {
                let tint = Vec3::ONE.lerp(tint / luminance(tint), SKY_TINT);
                for c in [&mut gradient.zenith, &mut gradient.horizon, &mut gradient.ground] {
                    *c = tint * luminance(*c);
                }
            }
        }
        log::debug!("sky light: zenith {} horizon {} ground {}", gradient.zenith, gradient.horizon, gradient.ground);
        if !self.claim(&path) && !painted {
            return Some((path, gradient));
        }
        let mut pixels = Vec::with_capacity((FACE_SIZE * FACE_SIZE * 6 * 8) as usize);
        for face in 0..6 {
            for y in 0..FACE_SIZE {
                for x in 0..FACE_SIZE {
                    let uv = (Vec2::new(x as f32, y as f32) + 0.5) / FACE_SIZE as f32;
                    let c = color(cube_direction(face, uv).normalize()) * BRIGHTNESS;
                    for v in [c.x, c.y, c.z, 1.0] {
                        pixels.extend_from_slice(&f16_bits(v).to_le_bytes());
                    }
                }
            }
        }
        let texture = TextureData { format: TextureFormat::Rgba16Float, width: FACE_SIZE, height: FACE_SIZE, layers: 1, faces: 6, mips: vec![Cow::Owned(pixels)] };
        match self.write(&path, &texture.write()) {
            Ok(()) => Some((path, gradient)),
            Err(err) => {
                log::warn!("{err:#}");
                None
            }
        }
    }
}

fn luminance(c: Vec3) -> f32 {
    c.dot(Vec3::new(0.2126, 0.7152, 0.0722))
}

/// The sky's light as a zenith / horizon / ground gradient. Each is the light
/// a surface facing that way receives, averaged over the half of the sky it
/// sees: a single texel straight up would give the bluest spot of the sky,
/// and leave out the clouds and the paler horizon.
fn gradient(light: &dyn Fn(Vec3) -> Vec3) -> Sky {
    const SAMPLES: u32 = 4096;
    let golden = std::f32::consts::PI * (3.0 - 5f32.sqrt());
    let directions: Vec<Vec3> = (0..SAMPLES)
        .map(|i| {
            let z = 1.0 - 2.0 * (i as f32 + 0.5) / SAMPLES as f32;
            let r = (1.0 - z * z).sqrt();
            let (s, c) = (golden * i as f32).sin_cos();
            Vec3::new(r * c, r * s, z)
        })
        .collect();
    let lights: Vec<Vec3> = directions.iter().map(|&d| light(d)).collect();
    let facing = |normal: Vec3| {
        let (sum, weight) = directions.iter().zip(&lights).fold((Vec3::ZERO, 0.0), |(sum, weight), (d, l)| {
            let w = d.dot(normal).max(0.0);
            (sum + *l * w, weight + w)
        });
        sum / weight
    };
    let horizon = (0..8).map(|i| (i as f32 * std::f32::consts::TAU / 8.0).sin_cos()).map(|(s, c)| facing(Vec3::new(c, s, 0.0))).sum::<Vec3>() / 8.0;
    Sky { zenith: facing(Vec3::Z), horizon, ground: facing(-Vec3::Z) }
}

/// The direction through texel `uv` of cube face `face` (+X, -X, +Y, -Y, +Z, -Z).
fn cube_direction(face: u32, uv: Vec2) -> Vec3 {
    let (s, t) = (uv.x * 2.0 - 1.0, uv.y * 2.0 - 1.0);
    match face {
        0 => Vec3::new(1.0, -t, -s),
        1 => Vec3::new(-1.0, -t, s),
        2 => Vec3::new(s, 1.0, t),
        3 => Vec3::new(s, -1.0, -t),
        4 => Vec3::new(s, -t, 1.0),
        _ => Vec3::new(-s, -t, -1.0),
    }
}

/// The face and texel coordinates a direction looks up (inverse of [`cube_direction`]).
fn cube_lookup(d: Vec3) -> (usize, Vec2) {
    let a = d.abs();
    let (face, sc, tc, ma) = if a.x >= a.y && a.x >= a.z {
        if d.x > 0.0 { (0, -d.z, -d.y, a.x) } else { (1, d.z, -d.y, a.x) }
    } else if a.y >= a.z {
        if d.y > 0.0 { (2, d.x, d.z, a.y) } else { (3, d.x, -d.z, a.y) }
    } else if d.z > 0.0 {
        (4, d.x, -d.y, a.z)
    } else {
        (5, -d.x, -d.y, a.z)
    };
    (face, Vec2::new(sc / ma + 1.0, tc / ma + 1.0) * 0.5)
}

/// The skybox textures as loaded are laid out for a Y-up cube sampled with
/// (-y, z, x) of a world direction.
/// The top face is turned a quarter: its top edge meets `lf`, its right edge `ft`.
fn sample_skybox(faces: &[Texture; 6], dir: Vec3) -> Vec3 {
    let (face, uv) = cube_lookup(Vec3::new(-dir.y, dir.z, dir.x));
    let uv = if face == 2 { Vec2::new(1.0 - uv.y, uv.x) } else { uv };
    let texture = &faces[face];
    let x = ((uv.x * texture.width as f32) as u32).min(texture.width - 1);
    let y = ((uv.y * texture.height as f32) as u32).min(texture.height - 1);
    let i = ((y * texture.width + x) * 4) as usize;
    let p = &texture.mips[0][i..i + 3];
    Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32).map(|c| srgb_to_linear(c / 255.0))
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

/// `value` as half-float bits.
fn f16_bits(value: f32) -> u16 {
    let bits = value.max(0.0).to_bits();
    let exponent = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    if exponent <= 0 {
        return 0;
    }
    if exponent >= 31 {
        return 0x7bff;
    }
    ((exponent as u16) << 10) | ((bits >> 13) & 0x3ff) as u16
}

/// `env_skypaint` settings, with the entity's default values.
struct SkyPaint {
    top: Vec3,
    bottom: Vec3,
    fade_bias: f32,
    /// Towards the sun.
    sun_normal: Vec3,
    sun_color: Vec3,
    sun_size: f32,
    dusk_color: Vec3,
    dusk_scale: f32,
    dusk_intensity: f32,
    stars: Option<StarLayer>,
}

/// The texture layer (`drawstars`): stars or clouds, drawn at rest.
struct StarLayer {
    texture: Texture,
    layers: u32,
    scale: f32,
    fade: f32,
}

impl SkyPaint {
    fn from_map(converter: &Converter, map: &Map) -> SkyPaint {
        let fs = &converter.fs;
        let paint = map.find_entities("env_skypaint").next();
        let number = |key: &str, default: f32| paint.and_then(|e| e.get(key)).and_then(|v| v.trim().parse().ok()).unwrap_or(default);
        let vector = |key: &str, default: [f32; 3]| Vec3::from(paint.and_then(|e| e.vector(key)).unwrap_or(default));
        let sun_normal = map
            .find_entities("env_sun")
            .next()
            .filter(|_| number("sunposmethod", 0.0) != 0.0)
            .map(sun_direction)
            .unwrap_or_else(|| vector("sunnormal", [0.4, 0.0, 0.01]));
        let stars = (number("drawstars", 0.0) != 0.0)
            .then(|| {
                let name = paint.and_then(|e| e.get("startexture")).unwrap_or("skybox/starfield");
                // The key names a material, sometimes a bare texture.
                let texture = load_material(fs, name).and_then(|m| m.base_texture).and_then(|t| load_texture(fs, &t)).or_else(|| load_texture(fs, name))?;
                Some(StarLayer {
                    texture,
                    layers: number("starlayers", 1.0).clamp(1.0, 3.0) as u32,
                    scale: 1.0 / number("starscale", 0.5).max(0.01),
                    fade: number("starfade", 1.5).max(0.01),
                })
            })
            .flatten();
        SkyPaint {
            top: vector("topcolor", [0.2, 0.5, 1.0]),
            bottom: vector("bottomcolor", [0.8, 1.0, 1.0]),
            fade_bias: number("fadebias", 1.0),
            sun_normal,
            sun_color: vector("suncolor", [0.2, 0.1, 0.0]),
            sun_size: number("sunsize", 2.0),
            dusk_color: vector("duskcolor", [1.0, 0.2, 0.0]),
            dusk_scale: number("duskscale", 0.5),
            dusk_intensity: number("duskintensity", 1.0),
            stars,
        }
    }

    /// The sky's colour in direction `dir`, gamma space (as displayed).
    fn color(&self, dir: Vec3) -> Vec3 {
        let horizon = 1.0 - dir.z.abs();
        let sun_dot = if self.sun_normal.length() >= 0.001 { self.sun_normal.normalize().dot(dir) } else { 0.0 };
        let dusk = (horizon * 8.0 + sun_dot) / 9.0;
        let dusk_amount = dusk.abs().powf(self.dusk_scale * -5.0 + 10.0) * self.dusk_intensity;

        let mut sky = self.bottom.lerp(self.top, dir.z.abs().powf(self.fade_bias));
        if dusk > 0.0 {
            sky = sky - sky * dusk_amount * 0.5 + self.dusk_color * dusk_amount;
        }
        if sun_dot > 0.0 && self.sun_size > 0.0 {
            sky += self.sun_color * sun_dot.powf(1.0 / (self.sun_size * 0.05));
        }
        if let Some(layer) = &self.stars
            && dir.z > 0.0
        {
            let amount = dir.z.powf(1.0 / layer.fade);
            let star_coord = |d: f32| d.clamp(-1.0, 1.0).asin() + d.sin() * horizon;
            // The layer lies a quarter turn from the world axes.
            let mut coord = Vec2::new(star_coord(dir.y), star_coord(-dir.x));
            let mut stars = sample_wrapped(&layer.texture, coord * layer.scale) * amount;
            for (index, weight) in [(1, 0.5), (2, 0.75)] {
                if layer.layers > index {
                    coord.x -= 1.0;
                    stars += sample_wrapped(&layer.texture, coord * layer.scale) * amount * weight;
                }
            }
            sky += stars;
        }
        sky
    }
}

/// `env_sun`'s direction towards the sun: its yaw, and `pitch` (negative is up).
fn sun_direction(sun: &MapEntity) -> Vec3 {
    let angles = sun.vector("angles").unwrap_or([0.0; 3]);
    let pitch = sun.get("pitch").and_then(|v| v.trim().parse::<f32>().ok()).unwrap_or(angles[0]).to_radians();
    let yaw = angles[1].to_radians();
    Vec3::new(pitch.cos() * yaw.cos(), pitch.cos() * yaw.sin(), -pitch.sin())
}

/// Bilinear, wrapping lookup of the top mip; RGB 0..1 as stored.
fn sample_wrapped(texture: &Texture, uv: Vec2) -> Vec3 {
    let (w, h) = (texture.width as i64, texture.height as i64);
    let (x, y) = (uv.x * w as f32 - 0.5, uv.y * h as f32 - 0.5);
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (x - x0, y - y0);
    let texel = |tx: i64, ty: i64| {
        let i = ((ty.rem_euclid(h) * w + tx.rem_euclid(w)) * 4) as usize;
        let p = &texture.mips[0][i..i + 3];
        Vec3::new(p[0] as f32, p[1] as f32, p[2] as f32) / 255.0
    };
    let (x0, y0) = (x0 as i64, y0 as i64);
    texel(x0, y0).lerp(texel(x0 + 1, y0), fx).lerp(texel(x0, y0 + 1).lerp(texel(x0 + 1, y0 + 1), fx), fy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_lookup_inverts_cube_direction() {
        for face in 0..6 {
            for uv in [Vec2::new(0.25, 0.75), Vec2::new(0.5, 0.5), Vec2::new(0.9, 0.1)] {
                let (back_face, back_uv) = cube_lookup(cube_direction(face, uv));
                assert_eq!(back_face, face as usize);
                assert!(back_uv.abs_diff_eq(uv, 1e-5), "face {face}: {uv} -> {back_uv}");
            }
        }
    }
}
