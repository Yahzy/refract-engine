//! Maps (`.rmap`).
//!
//! Sections: `INFO` (TOML: sun, sky, fog...), the world's geometry cut into
//! chunks (submeshes, see [`chunk_flags`]), placed models (`PROP`, `MODL`),
//! dynamic lights (`LITE`), baked indirect light as a grid of probes
//! (`PRBG`, `PROB`) and the entities (`ENTS`, see [`crate::entities`]).

use bytemuck::{Pod, Zeroable};
use serde::{Deserialize, Serialize};

use crate::container::{Container, ContainerWriter};
use crate::geometry::{Geometry, GeometryData};
use crate::AssetError;

const KIND: [u8; 4] = *b"MAP ";
pub const VERSION: u32 = 2;

/// [`crate::Submesh::flags`] of map chunks.
pub mod chunk_flags {
    /// Part of the 3D skybox, drawn around [`super::SkyCamera`].
    pub const SKYBOX: u32 = 1;
    /// A decal laid on a surface: drawn just in front of it.
    pub const OVERLAY: u32 = 2;
    /// Casts shadows but is never drawn (hidden faces of solid brushes).
    pub const SHADOW_ONLY: u32 = 4;
}

/// [`Prop::flags`].
pub mod prop_flags {
    /// Inside the 3D skybox.
    pub const SKYBOX: u32 = 1;
    pub const NO_SHADOW: u32 = 2;
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MapInfo {
    pub name: String,
    /// Identifies the source the map was converted from; the converter
    /// redoes the conversion when it changes.
    pub source: String,
    pub sun: Option<Sun>,
    /// Light reaching everything from everywhere, linear RGB, where the
    /// ambient grid has nothing.
    #[serde(default)]
    pub ambient: [f32; 3],
    /// Sky texture: a cubemap, or none for the procedural sky.
    pub sky: Option<String>,
    /// Reflection cubemaps (a cube array, one cubemap per position of `REFL`).
    #[serde(default)]
    pub reflections: Option<String>,
    pub sky_camera: Option<SkyCamera>,
    pub fog: Option<Fog>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Sun {
    /// Direction the sunlight travels (unit vector).
    pub direction: [f32; 3],
    /// Linear RGB.
    pub color: [f32; 3],
    pub intensity: f32,
}

/// The 3D skybox is a small room drawn as seen from `origin`, with the
/// viewer's position scaled down by `scale`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SkyCamera {
    pub origin: [f32; 3],
    pub scale: f32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Fog {
    pub color: [f32; 3],
    pub start: f32,
    pub end: f32,
    pub max_density: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Prop {
    /// Index into the model list.
    pub model: u32,
    pub flags: u32,
    pub scale: f32,
    /// Distance (world units) where it starts fading out, and where it is
    /// gone; `fade_end` is 0 when it never fades.
    pub fade_start: f32,
    pub fade_end: f32,
    pub origin: [f32; 3],
    /// `pitch yaw roll`, degrees.
    pub angles: [f32; 3],
    /// sRGB tint and opacity.
    pub color: [f32; 4],
    pub _pad: u32,
}

pub const LIGHT_POINT: u32 = 0;
pub const LIGHT_SPOT: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Light {
    pub kind: u32,
    pub position: [f32; 3],
    pub range: f32,
    /// Direction the light shines (spots).
    pub direction: [f32; 3],
    pub intensity: f32,
    /// Linear RGB.
    pub color: [f32; 3],
    /// Half-angles of the full-brightness cone and of its edge, degrees.
    pub inner_angle: f32,
    pub outer_angle: f32,
    pub flags: u32,
    pub _pad: u32,
}

/// A regular grid of light probes, one at each cell's center.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct ProbeGrid {
    /// Corner of the first cell.
    pub origin: [f32; 3],
    pub cell_size: f32,
    pub resolution: [u32; 3],
    pub _pad: u32,
}

/// The light around one point: an ambient cube (light arriving from +X, -X,
/// +Y, -Y, +Z and -Z), and how far one can see in each of those directions,
/// so that probes behind a wall are not used to light the other side.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Probe {
    /// RGB9E5 ([`crate::encoding::rgb9e5`]); a surface facing that axis
    /// receives `colour * albedo`.
    pub light: [u32; 6],
    /// Mean free distance towards each axis, world units, as f16 bits.
    pub distance: [u16; 6],
    pub flags: u32,
}

/// [`Probe::flags`].
pub mod probe_flags {
    /// In open space (not inside walls): usable for lighting.
    pub const VALID: u32 = 1;
}

pub struct MapFile<'a> {
    pub info: MapInfo,
    pub geometry: Geometry<'a>,
    pub props: &'a [Prop],
    models: &'a str,
    pub lights: &'a [Light],
    pub probes: Option<(ProbeGrid, &'a [Probe])>,
    /// Where each reflection cubemap was taken (xyz; w unused).
    pub reflection_positions: &'a [[f32; 4]],
    pub entities: &'a str,
}

impl<'a> MapFile<'a> {
    pub fn read(bytes: &'a [u8]) -> Result<Self, AssetError> {
        let container = Container::parse(bytes, KIND, VERSION, "map")?;
        let probes = match container.raw(*b"PRBG") {
            Some(_) => {
                let grid: ProbeGrid = *container.single(*b"PRBG", "probe grid")?;
                let probes = container.array(*b"PROB", "probes")?;
                if probes.len() != grid.resolution.iter().map(|&r| r as usize).product::<usize>() {
                    return Err(AssetError::Corrupt("probe count"));
                }
                Some((grid, probes))
            }
            None => None,
        };
        let props: &[Prop] = container.array(*b"PROP", "props")?;
        let models = container.text(*b"MODL", "models")?;
        if props.iter().any(|p| p.model as usize >= models.lines().count()) {
            return Err(AssetError::Corrupt("prop model index"));
        }
        Ok(Self {
            info: toml::from_str(container.text(*b"INFO", "info")?)?,
            geometry: Geometry::read(&container)?,
            props,
            models,
            lights: container.array(*b"LITE", "lights")?,
            probes,
            reflection_positions: container.raw(*b"REFL").map_or(Ok(&[][..]), |_| container.array(*b"REFL", "reflection positions"))?,
            entities: container.text(*b"ENTS", "entities")?,
        })
    }

    pub fn models(&self) -> impl Iterator<Item = &'a str> {
        self.models.lines()
    }
}

/// A map being built by the converter.
#[derive(Clone, Debug, Default)]
pub struct MapData {
    pub info: MapInfo,
    pub geometry: GeometryData,
    pub props: Vec<Prop>,
    pub models: Vec<String>,
    pub lights: Vec<Light>,
    /// The grid and its probes, x fastest, then y, then z.
    pub probes: Option<(ProbeGrid, Vec<Probe>)>,
    pub reflection_positions: Vec<[f32; 4]>,
    pub entities: String,
}

impl MapData {
    pub fn write(&self) -> Vec<u8> {
        let mut writer = ContainerWriter::new(KIND, VERSION);
        writer.text(*b"INFO", &toml::to_string(&self.info).expect("map info serializes"));
        self.geometry.write(&mut writer);
        writer.array(*b"PROP", &self.props).text(*b"MODL", &self.models.join("\n")).array(*b"LITE", &self.lights);
        if let Some((grid, probes)) = &self.probes {
            writer.array(*b"PRBG", &[*grid]).array(*b"PROB", probes);
        }
        if !self.reflection_positions.is_empty() {
            writer.array(*b"REFL", &self.reflection_positions);
        }
        writer.text(*b"ENTS", &self.entities);
        writer.finish()
    }

    /// The index of `model` in the list, added if new.
    pub fn model_index(&mut self, model: &str) -> u32 {
        match self.models.iter().position(|m| m == model) {
            Some(i) => i as u32,
            None => {
                self.models.push(model.to_owned());
                self.models.len() as u32 - 1
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::{Submesh, VertexAttributes};

    #[test]
    fn round_trip() {
        let mut map = MapData::default();
        map.info.name = "test".into();
        map.info.sun = Some(Sun { direction: [0.0, 0.0, -1.0], color: [1.0; 3], intensity: 3.0 });
        map.geometry.positions = vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        map.geometry.attributes = vec![VertexAttributes::default(); 3];
        map.geometry.indices = vec![0, 1, 2];
        let material = map.geometry.material_index("materials/a.rmat");
        map.geometry.submeshes = vec![Submesh { first_index: 0, index_count: 3, material, flags: chunk_flags::SKYBOX, center: [0.5; 3], radius: 1.0 }];
        let model = map.model_index("models/b.rmdl");
        map.props = vec![Prop { model, scale: 1.0, ..Default::default() }];
        map.lights = vec![Light { kind: LIGHT_SPOT, range: 100.0, ..Default::default() }];
        map.probes = Some((ProbeGrid { resolution: [1, 1, 1], cell_size: 64.0, ..Default::default() }, vec![Probe { flags: probe_flags::VALID, ..Default::default() }]));
        map.entities = "classname\tworldspawn\n".into();

        let bytes = map.write();
        let aligned: Vec<u128> = bytemuck::pod_collect_to_vec(&bytes);
        let file = MapFile::read(&bytemuck::cast_slice::<u128, u8>(&aligned)[..bytes.len()]).unwrap();
        assert_eq!(file.info.name, "test");
        assert_eq!(file.info.sun.as_ref().unwrap().intensity, 3.0);
        assert_eq!(file.geometry.indices, &[0, 1, 2]);
        assert_eq!(file.geometry.materials().collect::<Vec<_>>(), ["materials/a.rmat"]);
        assert_eq!(file.geometry.submeshes[0].flags, chunk_flags::SKYBOX);
        assert_eq!(file.models().collect::<Vec<_>>(), ["models/b.rmdl"]);
        assert_eq!(file.props[0].scale, 1.0);
        assert_eq!(file.lights[0].kind, LIGHT_SPOT);
        assert_eq!(file.probes.unwrap().1[0].flags, probe_flags::VALID);
        assert_eq!(file.entities, "classname\tworldspawn\n");
    }
}
