//! Models (`.rmdl`): geometry whose submeshes each use one material.

use crate::container::{Container, ContainerWriter};
use crate::geometry::{Geometry, GeometryData};
use crate::AssetError;

const KIND: [u8; 4] = *b"MODL";
pub const VERSION: u32 = 1;

pub struct ModelFile<'a> {
    pub geometry: Geometry<'a>,
}

impl<'a> ModelFile<'a> {
    pub fn read(bytes: &'a [u8]) -> Result<Self, AssetError> {
        let container = Container::parse(bytes, KIND, VERSION, "model")?;
        Ok(Self { geometry: Geometry::read(&container)? })
    }
}

pub fn write(geometry: &GeometryData) -> Vec<u8> {
    let mut writer = ContainerWriter::new(KIND, VERSION);
    geometry.write(&mut writer);
    writer.finish()
}
