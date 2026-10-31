//! Maps: BSP to `.rmap`, converting every material, texture and model they use.

use std::collections::HashMap;

use anyhow::Context;
use glam::{Mat3, Vec3};
use refract_asset::entities::{self, Entity};
use refract_asset::map::{MapData, MapFile, MapInfo, Prop, SkyCamera, chunk_flags, prop_flags};
use refract_asset::{MappedFile, ModelFile};
use refract_core::Angles;
use refract_formats::bsp::{self, Map, MeshBatch};
use refract_formats::material::{self as source_material, mapping_size};

use refract_render::Sky;

use crate::{CONVERTER_VERSION, Converter, fog, geometry, lights, parallel_map};

pub enum MapOutcome {
    Converted,
    UpToDate,
}

impl Converter {
    /// Converts `maps/<name>.bsp` unless its converted form is up to date.
    pub fn map(&mut self, name: &str) -> anyhow::Result<MapOutcome> {
        let path = format!("maps/{}.rmap", name.to_ascii_lowercase());
        let fingerprint = bsp::map_fingerprint(&self.fs, name).with_context(|| format!("maps/{name}.bsp not found"))?;
        let source = format!("bsp {fingerprint:016x} converter {CONVERTER_VERSION}");
        if !self.force && self.converted_from(&path).as_deref() == Some(source.as_str()) {
            return Ok(MapOutcome::UpToDate);
        }

        let map = bsp::load_map(&mut self.fs, name)?;
        let mut data = MapData {
            info: MapInfo {
                name: name.to_owned(),
                source,
                sun: map.find_entities("light_environment").next().map(lights::sun),
                ambient: lights::ambient(map.find_entities("light_environment").next()),
                sky: None,
                reflections: None,
                sky_camera: map.sky_camera.map(|c| SkyCamera { origin: c.origin, scale: c.scale }),
                fog: fog::fog(&map),
            },
            ..Default::default()
        };

        let sky = self.sky(&map);
        data.info.sky = sky.as_ref().map(|(path, _)| path.clone());
        self.add_geometry(&map, &mut data);
        let boxes = self.add_props(&map, &mut data);
        data.lights = map.entities.iter().filter(|e| e.classname == "light" || e.classname == "light_spot").filter_map(lights::light).collect();
        lights::fit_fixtures(&mut data.lights, &boxes);
        let light = self.light(&map, &data, sky.as_ref().map_or(Sky::CLEAR_DAY, |(_, light)| *light), data.info.sky.as_deref());
        data.probes = light.probes;
        if let Some((positions, path)) = light.reflections {
            data.reflection_positions = positions;
            data.info.reflections = Some(path);
        }
        let entities: Vec<Entity> = map.entities.iter().map(|e| Entity { pairs: e.keyvalues.clone() }).collect();
        data.entities = entities::write(&entities);

        self.write(&path, &data.write())?;
        log::info!(
            "{name}: {} chunks, {} materials, {} props of {} models, {} lights",
            data.geometry.submeshes.len(),
            data.geometry.materials.len(),
            data.props.len(),
            data.models.len(),
            data.lights.len()
        );
        Ok(MapOutcome::Converted)
    }

    /// The `source` of the converted map at `path`, if it reads.
    fn converted_from(&self, path: &str) -> Option<String> {
        let file = MappedFile::open(&self.out.join(path)).ok()?;
        MapFile::read(file.bytes()).ok().map(|map| map.info.source)
    }

    fn add_geometry(&self, map: &Map, data: &mut MapData) {
        let groups: [(&[MeshBatch], u32); 3] = [(&map.batches, 0), (&map.sky_batches, chunk_flags::SKYBOX), (&map.overlay_batches, chunk_flags::OVERLAY)];
        let batches: Vec<(&MeshBatch, u32)> = groups.iter().flat_map(|(batches, flags)| batches.iter().map(move |b| (b, *flags))).collect();

        // Materials (and their textures) in parallel; geometry in order afterwards.
        let mut names: Vec<&str> = batches.iter().map(|(b, _)| b.material.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        let converted: Vec<(&str, String, [f32; 2])> = parallel_map(&names, |name| {
            let scale = source_material::load_material(&self.fs, name).map_or([32, 32], |info| mapping_size(&self.fs, &info));
            (*name, self.material(name), scale.map(|s| s as f32))
        });
        for (batch, flags) in batches {
            let (_, material, mapping) = converted.iter().find(|(name, ..)| *name == batch.material).expect("converted above");
            let uv_scale = if batch.texel_uvs { *mapping } else { [1.0, 1.0] };
            geometry::append(&mut data.geometry, batch, material, uv_scale, flags);
        }
        geometry::append(&mut data.geometry, &map.shadow_batch, "", [1.0, 1.0], chunk_flags::SHADOW_ONLY);
    }

    /// Adds the map's props; returns where each one's box lies.
    fn add_props(&self, map: &Map, data: &mut MapData) -> Vec<lights::PlacedBox> {
        let props: Vec<_> = map.static_props.iter().chain(&map.entity_props).collect();
        let mut models: Vec<(&str, usize)> = props.iter().map(|p| (p.model.as_str(), p.skin)).collect();
        models.sort_unstable();
        models.dedup();
        let converted: Vec<((&str, usize), Option<String>)> = parallel_map(&models, |&(model, skin)| ((model, skin), self.model(model, skin)));
        let bounds: HashMap<&str, (Vec3, Vec3)> = converted.iter().filter_map(|(_, path)| Some((path.as_deref()?, self.model_bounds(path.as_deref()?)?))).collect();
        let mut boxes = Vec::new();

        for prop in props {
            let Some((_, Some(path))) = converted.iter().find(|(key, _)| *key == (prop.model.as_str(), prop.skin)) else { continue };
            let model = data.model_index(path);
            if !prop.in_skybox
                && let Some(&(mins, maxs)) = bounds.get(path.as_str())
            {
                let [pitch, yaw, roll] = prop.angles;
                let rotation = Mat3::from_quat(Angles::new(pitch, yaw, roll).to_quat());
                boxes.push(lights::PlacedBox { rotation, origin: Vec3::from(prop.origin), scale: prop.scale, mins, maxs });
            }
            data.props.push(Prop {
                model,
                flags: if prop.in_skybox { prop_flags::SKYBOX } else { 0 },
                scale: prop.scale,
                fade_start: prop.fade_min,
                fade_end: prop.fade_max,
                origin: prop.origin,
                angles: prop.angles,
                color: prop.color,
                _pad: 0,
            });
        }
        boxes
    }

    /// The box around a converted model's vertices.
    fn model_bounds(&self, path: &str) -> Option<(Vec3, Vec3)> {
        let file = MappedFile::open(&self.out.join(path)).ok()?;
        let model = ModelFile::read(file.bytes()).ok()?;
        let positions = model.geometry.positions.iter().map(|p| Vec3::from(*p));
        positions.fold(None, |bounds: Option<(Vec3, Vec3)>, p| Some(bounds.map_or((p, p), |(min, max)| (min.min(p), max.max(p)))))
    }
}
