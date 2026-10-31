//! Geometry. Every mesh lives in shared buffers (positions, other vertex
//! attributes, indices), so drawing never rebinds buffers. Positions are a
//! stream of their own: depth and shadow passes read nothing else.

use glam::Vec3;
pub use refract_asset::VertexAttributes;

use crate::buffer::GrowableBuffer;
use crate::culling::Sphere;

pub const POSITION_LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
    array_stride: 12,
    step_mode: wgpu::VertexStepMode::Vertex,
    attributes: &wgpu::vertex_attr_array![0 => Float32x3],
};

pub const ATTRIBUTE_LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
    array_stride: size_of::<VertexAttributes>() as u64,
    step_mode: wgpu::VertexStepMode::Vertex,
    attributes: &wgpu::vertex_attr_array![1 => Snorm16x2, 2 => Snorm16x4, 3 => Float32x2, 4 => Unorm8x4],
};

/// Triangles, counter-clockwise when seen from the front.
#[derive(Clone, Debug, Default)]
pub struct MeshData {
    pub positions: Vec<[f32; 3]>,
    pub attributes: Vec<VertexAttributes>,
    pub indices: Vec<u32>,
}

/// A range of a geometry's indices, drawn as one mesh.
#[derive(Clone, Copy, Debug)]
pub struct MeshPart {
    pub first_index: u32,
    pub index_count: u32,
    pub bounds: Sphere,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MeshId(pub(crate) u32);

/// Where a mesh sits in the shared buffers.
#[derive(Clone, Copy)]
pub struct MeshRange {
    pub first_index: u32,
    pub index_count: u32,
    pub base_vertex: i32,
    /// Vertices from `base_vertex` the mesh's indices may refer to.
    pub vertex_count: u32,
    pub bounds: Sphere,
}

pub struct MeshArena {
    pub positions: GrowableBuffer,
    pub attributes: GrowableBuffer,
    pub indices: GrowableBuffer,
    ranges: Vec<MeshRange>,
}

impl MeshArena {
    /// With `ray_tracing`, positions and indices also feed acceleration structures.
    pub fn new(device: &wgpu::Device, ray_tracing: bool) -> Self {
        let blas = if ray_tracing { wgpu::BufferUsages::BLAS_INPUT } else { wgpu::BufferUsages::empty() };
        Self {
            positions: GrowableBuffer::new(device, "positions", wgpu::BufferUsages::VERTEX | blas, 1 << 20),
            attributes: GrowableBuffer::new(device, "attributes", wgpu::BufferUsages::VERTEX, 1 << 21),
            indices: GrowableBuffer::new(device, "indices", wgpu::BufferUsages::INDEX | blas, 1 << 20),
            ranges: Vec::new(),
        }
    }

    /// Uploads one geometry and returns a mesh for each part.
    pub fn add(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, positions: &[[f32; 3]], attributes: &[VertexAttributes], indices: &[u32], parts: &[MeshPart]) -> Vec<MeshId> {
        assert_eq!(positions.len(), attributes.len(), "vertex streams differ in length");
        let base_vertex = (self.positions.append(device, queue, bytemuck::cast_slice(positions)) / 12) as i32;
        self.attributes.append(device, queue, bytemuck::cast_slice(attributes));
        let first_index = (self.indices.append(device, queue, bytemuck::cast_slice(indices)) / 4) as u32;
        parts
            .iter()
            .map(|part| {
                self.ranges.push(MeshRange { first_index: first_index + part.first_index, index_count: part.index_count, base_vertex, vertex_count: positions.len() as u32, bounds: part.bounds });
                MeshId(self.ranges.len() as u32 - 1)
            })
            .collect()
    }

    pub fn range(&self, id: MeshId) -> MeshRange {
        self.ranges[id.0 as usize]
    }

    pub fn bind<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>, with_attributes: bool) {
        pass.set_vertex_buffer(0, self.positions.buffer().slice(..));
        if with_attributes {
            pass.set_vertex_buffer(1, self.attributes.buffer().slice(..));
        }
        pass.set_index_buffer(self.indices.buffer().slice(..), wgpu::IndexFormat::Uint32);
    }
}

impl MeshData {
    /// The whole mesh as one part.
    pub fn whole(&self) -> MeshPart {
        MeshPart { first_index: 0, index_count: self.indices.len() as u32, bounds: Sphere::around(self.positions.iter().map(|p| Vec3::from(*p))) }
    }
}
