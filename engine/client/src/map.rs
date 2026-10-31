//! Loading a converted map (`.rmap`) and everything it uses into the renderer.
//!
//! Maps are converted on demand: before loading, the client runs
//! `refract-convert`, which returns at once when the map is up to date.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, bail};
use glam::{Mat4, Vec3, Vec4};
use refract_asset::map::{LIGHT_SPOT, MapFile, chunk_flags, prop_flags};
use refract_asset::material::MaterialKind;
use refract_asset::{MappedFile, ModelFile, TextureData, entities};
use refract_core::Angles;
use refract_render::{BlendMode, Camera, Detail, Fog, Layer, Material, MaterialId, MeshId, MeshPart, PointLight, ProbeGrid, Renderer, Sphere, SpotLight, Sun, TextureId};

use crate::scene::{Scene, default_sky};

/// Share of the flat ambient light kept where baked probes light the map,
/// grey: it is all that lights rooms without lamps, which the map's ambient
/// colour (meant for the sky) would tint.
const BAKED_AMBIENT_FLOOR: f32 = 0.1;
/// Height of the eyes above the player's origin.
const EYE_HEIGHT: f32 = 64.0;

/// Converts `name` if needed, then loads it.
pub fn load(renderer: &mut Renderer, game: &Path, name: &str) -> anyhow::Result<Scene> {
    let content = game.join("cache").join("content");
    convert(game, &content, name)?;
    let file = MappedFile::open(&content.join(format!("maps/{}.rmap", name.to_ascii_lowercase())))?;
    let map = MapFile::read(file.bytes())?;
    let reflection_positions = map.reflection_positions.iter().map(|p| Vec3::new(p[0], p[1], p[2])).collect();
    let mut loader = Loader { renderer, content, textures: HashMap::new(), materials: HashMap::new(), models: HashMap::new(), reflection_positions };
    loader.load(&map)
}

fn convert(game: &Path, content: &Path, name: &str) -> anyhow::Result<()> {
    let converter = std::env::current_exe()?.with_file_name(format!("refract-convert{}", std::env::consts::EXE_SUFFIX));
    if !converter.exists() {
        log::warn!("{} not found: using the content already converted", converter.display());
        return Ok(());
    }
    let status = Command::new(&converter).args(["map", name, "--game"]).arg(game).arg("--out").arg(content).status().context("running refract-convert")?;
    if !status.success() {
        bail!("refract-convert failed on {name}");
    }
    Ok(())
}

/// A material as drawn: its id and the reflection cubemap the map chose for
/// it; `None` for materials that are never drawn.
type Drawn = Option<(MaterialId, Option<u32>)>;

struct Loader<'a> {
    renderer: &'a mut Renderer,
    content: PathBuf,
    textures: HashMap<String, TextureId>,
    materials: HashMap<String, Drawn>,
    reflection_positions: Vec<Vec3>,
    models: HashMap<String, Vec<(MeshId, Drawn)>>,
}

impl Loader<'_> {
    fn load(&mut self, map: &MapFile) -> anyhow::Result<Scene> {
        // 3D skybox: scaled up about the sky camera, so that it surrounds the world.
        let sky = map.info.sky_camera.as_ref().map_or(Mat4::IDENTITY, |c| Mat4::from_scale(Vec3::splat(c.scale)) * Mat4::from_translation(-Vec3::from(c.origin)));

        // Reflections first: instances pick theirs as they are added.
        if let Some(path) = map.info.reflections.as_deref() {
            let positions: Vec<Vec3> = map.reflection_positions.iter().map(|p| Vec3::new(p[0], p[1], p[2])).collect();
            match MappedFile::open(&self.content.join(path)).map_err(anyhow::Error::from).and_then(|file| {
                let cubemaps = TextureData::read(file.bytes())?;
                self.renderer.set_reflections(&cubemaps, &positions);
                Ok(())
            }) {
                Ok(()) => {}
                Err(err) => log::warn!("{path}: {err:#}"),
            }
        }
        let geometry = &map.geometry;
        let parts: Vec<MeshPart> = geometry.submeshes.iter().map(|s| MeshPart { first_index: s.first_index, index_count: s.index_count, bounds: Sphere { center: s.center.into(), radius: s.radius } }).collect();
        let meshes = self.renderer.add_geometry(geometry.positions, geometry.attributes, geometry.indices, &parts);
        let materials: Vec<&str> = geometry.materials().collect();
        let shadow_only = self.renderer.add_material(&Material::default());
        for (submesh, mesh) in geometry.submeshes.iter().zip(meshes) {
            let transform = if submesh.flags & chunk_flags::SKYBOX != 0 { sky } else { Mat4::IDENTITY };
            if submesh.flags & chunk_flags::SHADOW_ONLY != 0 {
                let instance = self.renderer.add_instance(mesh, shadow_only, transform);
                self.renderer.set_visible(instance, false);
            } else if let Some(material) = self.material(materials[submesh.material as usize]) {
                self.add_instance(mesh, material, transform, submesh.flags & chunk_flags::SKYBOX != 0);
            }
        }

        let models: Vec<&str> = map.models().collect();
        for prop in map.props {
            let parts = self.model(models[prop.model as usize]);
            let placement = Mat4::from_scale_rotation_translation(Vec3::splat(prop.scale), angles(prop.angles).to_quat(), prop.origin.into());
            let distant = prop.flags & prop_flags::SKYBOX != 0;
            let transform = if distant { sky * placement } else { placement };
            for &(mesh, material) in &parts {
                if let Some(material) = material {
                    self.add_instance(mesh, material, transform, distant);
                }
            }
        }

        let grid = map.probes.map(|(grid, probes)| ProbeGrid { origin: grid.origin.into(), cell_size: grid.cell_size, resolution: grid.resolution, probes });
        self.renderer.set_probes(grid.as_ref());
        let sky = map.info.sky.as_deref().and_then(|path| {
            let file = MappedFile::open(&self.content.join(path)).map_err(|err| log::warn!("{path}: {err}")).ok()?;
            let texture = TextureData::read(file.bytes()).map_err(|err| log::warn!("{path}: {err}")).ok()?;
            self.renderer.set_sky(Some(&texture));
            Some(())
        });
        if sky.is_none() {
            self.renderer.set_sky(None);
        }

        let mut scene = Scene {
            spawn: spawn(map.entities),
            sun: map.info.sun.as_ref().map_or(Sun { direction: -Vec3::Z, color: Vec3::ONE, intensity: 0.0 }, |s| Sun { direction: s.direction.into(), color: s.color.into(), intensity: s.intensity }),
            sky: default_sky(),
            ambient: match grid {
                Some(_) => Vec3::splat(Vec3::from(map.info.ambient).dot(Vec3::new(0.2126, 0.7152, 0.0722)) * BAKED_AMBIENT_FLOOR),
                None => Vec3::from(map.info.ambient),
            },
            exposure: 1.0,
            point_lights: Vec::new(),
            spot_lights: Vec::new(),
            fog: map.info.fog.as_ref().map(|f| Fog {
                color: f.color.into(),
                density: f.density,
                height: f.height,
                falloff: f.falloff,
                max_opacity: f.max_opacity,
                sky_density: f.sky_density,
            }),
            test: None,
        };
        for light in map.lights {
            let (position, color) = (Vec3::from(light.position), Vec3::from(light.color));
            if light.kind == LIGHT_SPOT {
                scene.spot_lights.push(SpotLight {
                    position,
                    direction: light.direction.into(),
                    color,
                    intensity: light.intensity,
                    range: light.range,
                    inner_angle: light.inner_angle,
                    outer_angle: light.outer_angle,
                    casts_shadows: false,
                    clearance: light.clearance,
                });
            } else {
                scene.point_lights.push(PointLight { position, color, intensity: light.intensity, range: light.range, casts_shadows: false, clearance: light.clearance });
            }
        }
        log::info!("loaded {}: {} textures, {} materials, {} models, {} lights", map.info.name, self.textures.len(), self.materials.len(), self.models.len(), map.lights.len());
        Ok(scene)
    }

    /// `distant`: in the 3D skybox.
    fn add_instance(&mut self, mesh: MeshId, (material, reflection): (MaterialId, Option<u32>), transform: Mat4, distant: bool) {
        let instance = self.renderer.add_instance(mesh, material, transform);
        if let Some(cubemap) = reflection {
            self.renderer.set_reflection(instance, cubemap);
        }
        if distant {
            self.renderer.set_distant(instance, true);
        }
    }

    fn material(&mut self, path: &str) -> Drawn {
        if path.is_empty() {
            return None;
        }
        if let Some(material) = self.materials.get(path) {
            return *material;
        }
        let source = match std::fs::read_to_string(self.content.join(path)).map_err(anyhow::Error::from).and_then(|text| Ok(refract_asset::Material::parse(&text)?)) {
            Ok(material) => material,
            Err(err) => {
                log::warn!("{path}: {err:#}");
                refract_asset::Material::default()
            }
        };
        let blend = match source.kind {
            MaterialKind::Opaque => BlendMode::Opaque,
            MaterialKind::AlphaTest => BlendMode::AlphaTest,
            MaterialKind::Translucent | MaterialKind::Water => BlendMode::Translucent,
            MaterialKind::Additive => BlendMode::Additive,
            MaterialKind::Sky | MaterialKind::Invisible => {
                self.materials.insert(path.to_owned(), None);
                return None;
            }
        };
        let mut material = Material {
            base_color: Vec4::from(source.color),
            base_texture: source.base_color.as_deref().map(|t| self.texture(t)),
            normal_texture: source.normal.as_deref().map(|t| self.texture(t)),
            layer2: source.layer2.as_ref().and_then(|layer| {
                Some(Layer {
                    base_texture: self.texture(layer.base_color.as_deref()?),
                    normal_texture: layer.normal.as_deref().map(|t| self.texture(t)),
                    blend_modulate: layer.blend_modulate.as_deref().map(|t| self.texture(t)),
                })
            }),
            detail: source.detail.as_ref().map(|detail| Detail {
                texture: self.texture(&detail.texture),
                scale: detail.scale,
                blend: detail.blend,
                amount: detail.amount,
                tint: Vec3::from(detail.tint),
            }),
            metallic: source.metallic,
            roughness: source.roughness,
            reflectance: source.reflectance,
            blend,
            alpha_cutoff: source.alpha_cutoff,
            two_sided: source.two_sided,
            ..Default::default()
        };
        if source.kind == MaterialKind::Water {
            // Until water has its own shading: a dark, glossy, see-through surface.
            material = Material { base_color: Vec4::new(0.05, 0.09, 0.1, 0.7), base_texture: None, normal_texture: None, detail: None, roughness: 0.05, two_sided: true, ..material };
        }
        let id = self.renderer.add_material(&material);
        // The map's chosen cubemap, by its position.
        let reflection = source.reflection_at.and_then(|at| {
            let at = Vec3::from(at);
            self.reflection_positions.iter().position(|p| p.distance_squared(at) < 4.0).map(|i| i as u32)
        });
        self.materials.insert(path.to_owned(), Some((id, reflection)));
        Some((id, reflection))
    }

    fn texture(&mut self, path: &str) -> TextureId {
        if let Some(id) = self.textures.get(path) {
            return *id;
        }
        let id = match MappedFile::open(&self.content.join(path)).map_err(anyhow::Error::from).and_then(|file| {
            let texture = TextureData::read(file.bytes())?;
            Ok(self.renderer.add_texture(&texture, path))
        }) {
            Ok(id) => id,
            Err(err) => {
                log::warn!("{path}: {err:#}");
                TextureId::WHITE
            }
        };
        self.textures.insert(path.to_owned(), id);
        id
    }

    fn model(&mut self, path: &str) -> Vec<(MeshId, Drawn)> {
        if let Some(parts) = self.models.get(path) {
            return parts.clone();
        }
        let parts = match MappedFile::open(&self.content.join(path)).map_err(anyhow::Error::from).and_then(|file| {
            let model = ModelFile::read(file.bytes())?;
            let geometry = &model.geometry;
            let parts: Vec<MeshPart> = geometry.submeshes.iter().map(|s| MeshPart { first_index: s.first_index, index_count: s.index_count, bounds: Sphere { center: s.center.into(), radius: s.radius } }).collect();
            let meshes = self.renderer.add_geometry(geometry.positions, geometry.attributes, geometry.indices, &parts);
            let materials: Vec<&str> = geometry.materials().collect();
            Ok(geometry.submeshes.iter().zip(meshes).map(|(s, mesh)| (mesh, self.material(materials[s.material as usize]))).collect())
        }) {
            Ok(parts) => parts,
            Err(err) => {
                log::warn!("{path}: {err:#}");
                Vec::new()
            }
        };
        self.models.insert(path.to_owned(), parts.clone());
        parts
    }
}

fn angles([pitch, yaw, roll]: [f32; 3]) -> Angles {
    Angles::new(pitch, yaw, roll)
}

/// The first player start, at eye height.
fn spawn(entities_text: &str) -> Camera {
    let entities = entities::parse(entities_text);
    let start = entities.iter().find(|e| e.classname() == "info_player_start").or_else(|| entities.iter().find(|e| e.classname().starts_with("info_player")));
    let vector = |e: &entities::Entity, key: &str| -> Option<Vec3> {
        let mut parts = e.get(key)?.split_whitespace().map(|v| v.parse::<f32>().ok());
        Some(Vec3::new(parts.next()??, parts.next()??, parts.next()??))
    };
    let position = start.and_then(|e| vector(e, "origin")).unwrap_or(Vec3::ZERO) + Vec3::Z * EYE_HEIGHT;
    let yaw = start.and_then(|e| vector(e, "angles")).map_or(0.0, |a| a.y);
    Camera { position, angles: Angles::new(0.0, yaw, 0.0), ..Default::default() }
}
