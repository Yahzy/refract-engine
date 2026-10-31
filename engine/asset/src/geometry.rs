//! Triangle geometry shared by models and maps: two vertex streams and one
//! index buffer, split into submeshes that each use one material.
//!
//! Positions are a stream of their own because the depth and shadow passes
//! read nothing else.

use bytemuck::{Pod, Zeroable};

use crate::container::{Container, ContainerWriter};
use crate::AssetError;

/// Everything about a vertex but its position. 24 bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct VertexAttributes {
    /// Octahedral unit normal ([`crate::encoding::oct_encode`]).
    pub normal: [i16; 2],
    /// Unit tangent (snorm16 xyz) and the bitangent's sign in w (±32767).
    pub tangent: [i16; 4],
    pub uv: [f32; 2],
    /// Linear RGB tint, and in alpha the blend from a material's first layer
    /// (0) to its second (255).
    pub color: [u8; 4],
}

/// A range of triangles drawn with one material, with its bounding sphere.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct Submesh {
    pub first_index: u32,
    pub index_count: u32,
    /// Index into the file's material list.
    pub material: u32,
    /// Meaning depends on the file; see [`crate::map::chunk_flags`].
    pub flags: u32,
    pub center: [f32; 3],
    pub radius: f32,
}

/// Geometry read in place.
pub struct Geometry<'a> {
    pub positions: &'a [[f32; 3]],
    pub attributes: &'a [VertexAttributes],
    pub indices: &'a [u32],
    pub submeshes: &'a [Submesh],
    /// Material paths, one per line.
    materials: &'a str,
}

impl<'a> Geometry<'a> {
    pub(crate) fn read(container: &Container<'a>) -> Result<Self, AssetError> {
        let geometry = Self {
            positions: container.array(*b"POSI", "positions")?,
            attributes: container.array(*b"ATTR", "attributes")?,
            indices: container.array(*b"INDX", "indices")?,
            submeshes: container.array(*b"SUBM", "submeshes")?,
            materials: container.text(*b"MATS", "materials")?,
        };
        if geometry.positions.len() != geometry.attributes.len() {
            return Err(AssetError::Corrupt("vertex streams differ in length"));
        }
        let vertex_count = geometry.positions.len() as u32;
        let index_count = geometry.indices.len() as u64;
        if geometry.indices.iter().any(|&i| i >= vertex_count) || geometry.submeshes.iter().any(|s| s.first_index as u64 + s.index_count as u64 > index_count) {
            return Err(AssetError::Corrupt("index out of range"));
        }
        Ok(geometry)
    }

    pub fn materials(&self) -> impl Iterator<Item = &'a str> {
        self.materials.lines()
    }
}

/// Geometry being built, owned.
#[derive(Clone, Debug, Default)]
pub struct GeometryData {
    pub positions: Vec<[f32; 3]>,
    pub attributes: Vec<VertexAttributes>,
    pub indices: Vec<u32>,
    pub submeshes: Vec<Submesh>,
    pub materials: Vec<String>,
}

impl GeometryData {
    pub(crate) fn write(&self, writer: &mut ContainerWriter) {
        writer
            .array(*b"POSI", &self.positions)
            .array(*b"ATTR", &self.attributes)
            .array(*b"INDX", &self.indices)
            .array(*b"SUBM", &self.submeshes)
            .text(*b"MATS", &self.materials.join("\n"));
    }

    /// The index of `material` in the list, added if new.
    pub fn material_index(&mut self, material: &str) -> u32 {
        match self.materials.iter().position(|m| m == material) {
            Some(i) => i as u32,
            None => {
                self.materials.push(material.to_owned());
                self.materials.len() as u32 - 1
            }
        }
    }
}
